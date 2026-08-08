//! Structural pass over the enumerated source units.
//!
//! The seven native detectors answer "is this call site wrong?". A structural
//! detector answers a question no per-site lint can reach: how the codebase is
//! shaped, how often the same thing appears, how much of it a reviewer has to
//! hold at once. It reads the units the source kernel already walked and parsed,
//! so the workspace is enumerated once whatever the number of producers.
//!
//! A structural finding is a family, not a site. Observations sharing a
//! normalized key become one diagnostic whose `related` array names every other
//! member, so a pattern repeated forty times costs the report one line instead
//! of forty. That key is also the identity of the finding: it carries no source
//! position, which is what lets a baseline comparison survive an insertion
//! above it.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use ra_ap_syntax::ast::{self, HasAttrs};
use ra_ap_syntax::{AstNode, SyntaxNode};

use crate::policy::{PolicyPlan, RuleDefinition, STRUCTURE_UNREASONED_ALLOW};
use crate::report::DiagnosticContext;
use crate::source_kernel::{Enumeration, SourceSpan, compact, line_starts, source_span};

mod duplication;
mod normalize;

const FINGERPRINT_DOMAIN: &str = "rust-doctor-structure-fingerprint-v1";

/// Wall clock the whole pass is allowed before it stops and says so.
///
/// The budget the NFR sets is 2 seconds on a workspace of a thousand files, and
/// this is five times that: it is not a target, it is the point past which a
/// partial answer beats a scan that never returns. Crossing it records an error
/// under the `structure` stage, which is what makes the score stop calling
/// itself authoritative.
const TIME_BUDGET: Duration = Duration::from_secs(10);

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct StructureScan {
    pub(crate) findings: Vec<StructureFinding>,
    pub(crate) errors: Vec<StructureError>,
    #[allow(
        dead_code,
        reason = "read only by the tests that assert the pass held its budget"
    )]
    pub(crate) counters: StructureCounters,
}

/// What the pass did, for the tests that bound what it is allowed to cost.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct StructureCounters {
    /// Functions the duplication pass kept, above the node floor.
    pub(crate) functions: usize,
    /// Distinct canonical forms among them. This is what the near-duplicate
    /// pass compares, and it is the number a workload has to present: a
    /// benchmark of ten thousand functions sharing five forms measures the
    /// walk and nothing else.
    pub(crate) shapes: usize,
    /// Pairs of shapes the near-duplicate pass scored. This is what the NFR
    /// bounds: a wall clock measures the machine, but a pass that scores every
    /// pair of a workspace is quadratic wherever it runs.
    pub(crate) comparisons: usize,
    /// Bytes the pass holds for those functions at its peak: one canonical
    /// digest and one subtree hash per node, never the tree they were read
    /// from. This is the quantity the memory bound is written against, and it
    /// grows with the code, not with the number of comparisons.
    pub(crate) retained_bytes: usize,
}

/// One structural family, published as a single diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StructureFinding {
    pub(crate) definition: &'static RuleDefinition,
    pub(crate) message: String,
    pub(crate) package: Option<String>,
    pub(crate) target: Option<String>,
    pub(crate) context: Option<DiagnosticContext>,
    pub(crate) path: String,
    pub(crate) span: SourceSpan,
    /// Every member beyond the first, in workspace-relative sorted order.
    pub(crate) related: Vec<StructureLocation>,
    pub(crate) occurrences: usize,
    /// Identity of the normalized content, as a hexadecimal blake3 digest. It
    /// contains no line and no column, so unrelated edits above the finding
    /// leave it unchanged.
    pub(crate) structure: String,
    /// How alike the members of the family are, in basis points, when the rule
    /// grouped them on a similarity rather than on an equality.
    pub(crate) similarity: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct StructureLocation {
    pub(crate) path: String,
    pub(crate) span: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct StructureError {
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

/// What a detector reads off one parsed unit.
///
/// `key` is what makes two observations the same finding. It never contains a
/// position and it never contains a path, so a family gathers what the whole
/// workspace does, across files, and keeps its identity across edits. A
/// detector that wants a per-file family says so by putting the path in its own
/// key.
struct Observation {
    key: String,
    subject: String,
    span: SourceSpan,
    context: Option<DiagnosticContext>,
}

/// A detector declares the rule it carries and the function that reads a unit.
/// The pass groups what every detector returns, so a detector never counts,
/// sorts or names a family itself.
struct Detector {
    definition: &'static RuleDefinition,
    observe: fn(&Unit<'_>) -> Vec<Observation>,
}

/// What a detector knows about the unit under analysis.
struct Unit<'a> {
    tree: ast::SourceFile,
    source: &'a str,
    line_starts: Vec<usize>,
    /// Non-production mark the Cargo targets reaching this unit agree on.
    context: Option<DiagnosticContext>,
}

impl Unit<'_> {
    fn span(&self, node: &SyntaxNode) -> SourceSpan {
        source_span(node.text_range(), &self.line_starts, self.source)
    }
}

static UNREASONED_ALLOW: Detector = Detector {
    definition: &STRUCTURE_UNREASONED_ALLOW,
    observe: unreasoned_allow,
};

static DETECTORS: [&Detector; 1] = [&UNREASONED_ALLOW];

/// Members of one family, before it becomes a diagnostic.
#[derive(Debug, Default)]
struct Family {
    members: Vec<Member>,
    subject: String,
    similarity: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Member {
    path: String,
    span: SourceSpan,
    context: Option<DiagnosticContext>,
}

/// The wall clock the pass is running against.
///
/// A workspace far larger than the benchmark must not turn a scan into a hang.
/// Crossing the budget is not a failure of the workspace, it is the pass saying
/// what it did not get to, so it stops, publishes what it collected and records
/// an error, which is what makes the score drop its authoritative flag.
struct Deadline {
    start: Instant,
    budget: Duration,
}

impl Deadline {
    fn new(budget: Duration) -> Self {
        Self {
            start: Instant::now(),
            budget,
        }
    }

    fn exceeded(&self) -> bool {
        self.start.elapsed() >= self.budget
    }
}

pub(crate) fn analyze(enumeration: &Enumeration, plan: &PolicyPlan) -> StructureScan {
    analyze_within(enumeration, plan, TIME_BUDGET)
}

fn analyze_within(
    enumeration: &Enumeration,
    plan: &PolicyPlan,
    budget: Duration,
) -> StructureScan {
    let detectors: Vec<&'static Detector> = DETECTORS
        .iter()
        .copied()
        .filter(|detector| plan.is_active(detector.definition.id))
        .collect();
    let duplication = duplication::Active::of(plan);
    if detectors.is_empty() && !duplication.any() {
        return StructureScan::default();
    }

    let deadline = Deadline::new(budget);
    let mut stopped = false;
    let mut errors = Vec::new();
    let mut families = BTreeMap::<(&'static str, String), Family>::new();
    let mut functions = Vec::new();
    for unit in enumeration.units() {
        if deadline.exceeded() {
            stopped = true;
            break;
        }
        // A unit the parser could not read is skipped whole: a normalized tree
        // built over a recovered region describes the recovery, not the code.
        if !unit.parses_cleanly() {
            errors.push(StructureError {
                code: "parse-error",
                message: format!(
                    "Source path \"{}\" was skipped: the parser could not read it.",
                    unit.relative_path()
                ),
            });
            continue;
        }
        let analysed = Unit {
            tree: unit.tree(),
            source: unit.source(),
            line_starts: line_starts(unit.source()),
            context: unit.context(enumeration.contexts()),
        };
        for detector in &detectors {
            for observation in (detector.observe)(&analysed) {
                let family = families
                    .entry((detector.definition.id, observation.key))
                    .or_default();
                family.members.push(Member {
                    path: unit.relative_path().to_owned(),
                    span: observation.span,
                    context: observation.context,
                });
                if family.subject.is_empty() {
                    family.subject = observation.subject;
                }
            }
        }
        if duplication.any() {
            // Only the canonical form of each function is retained, never the
            // tree it was read from, so the walk stays linear in memory
            // whatever the size of the workspace.
            functions.extend(duplication::observe(&analysed, unit.relative_path()));
        }
    }

    let mut counters = StructureCounters::default();
    if duplication.any() {
        counters.functions = functions.len();
        counters.shapes = duplication::shapes(&functions);
        counters.retained_bytes = functions.iter().map(duplication::retained_bytes).sum();
        let grouping = duplication::groups(functions, duplication, &deadline);
        counters.comparisons = grouping.comparisons;
        for group in grouping.groups {
            families.insert(
                (group.definition.id, group.key),
                Family {
                    members: group.members,
                    subject: group.subject,
                    similarity: group.similarity,
                },
            );
        }
    }
    if stopped || deadline.exceeded() {
        errors.push(StructureError {
            code: "time-budget",
            message: "Structural analysis stopped at the time budget; results are partial."
                .to_owned(),
        });
    }

    errors.sort();
    errors.dedup();
    StructureScan {
        findings: findings(families, enumeration),
        errors,
        counters,
    }
}

fn findings(
    families: BTreeMap<(&'static str, String), Family>,
    enumeration: &Enumeration,
) -> Vec<StructureFinding> {
    let packages: BTreeMap<&str, (Option<String>, Option<String>)> = enumeration
        .units()
        .map(|unit| (unit.relative_path(), (unit.package(), unit.target())))
        .collect();
    families
        .into_iter()
        .filter_map(|((rule, key), mut family)| {
            let definition = crate::policy::find(rule)?;
            family.members.sort();
            family.members.dedup();
            let (first, related) = family.members.split_first()?;
            let (package, target) = packages
                .get(first.path.as_str())
                .cloned()
                .unwrap_or_default();
            Some(StructureFinding {
                definition,
                message: family.subject,
                package,
                target,
                context: unanimous_context(&family.members),
                path: first.path.clone(),
                span: first.span,
                related: related
                    .iter()
                    .map(|member| StructureLocation {
                        path: member.path.clone(),
                        span: member.span,
                    })
                    .collect(),
                occurrences: family.members.len(),
                structure: structural_hash(rule, &key),
                similarity: family.similarity,
            })
        })
        .collect()
}

/// Mark of a family, when every member carries the same one.
///
/// A family straddling production and a test target stays unmarked and keeps
/// weighing on the score: the shipped half of it is still shipped.
fn unanimous_context(members: &[Member]) -> Option<DiagnosticContext> {
    let mut contexts = members.iter().map(|member| member.context);
    let first = contexts.next().flatten()?;
    contexts.all(|context| context == Some(first)).then_some(first)
}

/// Identity of a family: its rule and its normalized key, never its position.
fn structural_hash(rule: &str, key: &str) -> String {
    let mut hasher = blake3::Hasher::new();
    for field in [FINGERPRINT_DOMAIN, rule, key] {
        hasher.update(&(field.len() as u64).to_le_bytes());
        hasher.update(field.as_bytes());
    }
    hasher.finalize().to_hex().to_string()
}

/// Every `#[allow]` that switches a lint off without saying why.
///
/// `#[expect]` is left alone on purpose: it fails once the lint stops firing,
/// so it expires by itself and needs no census. `#[cfg_attr(..., allow(...))]`
/// is out of reach here, because the attribute it produces does not exist in
/// the syntax tree.
fn unreasoned_allow(unit: &Unit<'_>) -> Vec<Observation> {
    let mut observations = Vec::new();
    for attribute in unit.tree.syntax().descendants().filter_map(ast::Attr::cast) {
        // Only the `allow(...)` form is a census subject: `#[expect]` carries
        // another path, and `#[cfg_attr(test, allow(...))]` is a `CfgAttrMeta`
        // whose produced attribute does not exist in this tree.
        let Some(ast::Meta::TokenTreeMeta(meta)) = attribute.meta() else {
            continue;
        };
        if meta
            .path()
            .and_then(|path| path.as_single_name_ref())
            .map(|name| name.text().to_string())
            .as_deref()
            != Some("allow")
        {
            continue;
        }
        let Some(arguments) = meta.token_tree().map(|tree| arguments(tree.syntax())) else {
            continue;
        };
        if arguments.iter().any(|argument| is_reason(argument)) {
            continue;
        }
        let mut lints: Vec<&str> = arguments
            .iter()
            .map(String::as_str)
            .filter(|argument| !argument.is_empty())
            .collect();
        if lints.is_empty() {
            continue;
        }
        lints.sort_unstable();
        lints.dedup();

        let inner = attribute.excl_token().is_some();
        let written = format!(
            "#{}[allow({})]",
            if inner { "!" } else { "" },
            lints.join(", ")
        );
        observations.push(Observation {
            key: format!("{}|{}", if inner { "inner" } else { "outer" }, lints.join(",")),
            subject: format!("{written} switches a lint off without a stated reason."),
            span: unit.span(attribute.syntax()),
            context: test_context(attribute.syntax()).or(unit.context),
        });
    }
    observations
}

/// Is this argument the `reason = "..."` the attribute needs to be deliberate?
fn is_reason(argument: &str) -> bool {
    argument
        .split_once('=')
        .is_some_and(|(name, value)| name == "reason" && value.starts_with('"'))
}

/// Top-level arguments of a token tree, without trivia and without the
/// delimiters. A nested tree stays inside the argument that carries it, so
/// `allow(a, b(c, d))` reads as two arguments.
fn arguments(tree: &SyntaxNode) -> Vec<String> {
    let mut arguments = vec![String::new()];
    let mut depth = 0_usize;
    for element in tree.descendants_with_tokens() {
        let Some(token) = element.into_token() else {
            continue;
        };
        if token.kind().is_trivia() {
            continue;
        }
        match token.text() {
            "(" | "[" | "{" => {
                depth += 1;
                if depth > 1 {
                    push_argument(&mut arguments, token.text());
                }
            }
            ")" | "]" | "}" => {
                depth = depth.saturating_sub(1);
                if depth > 0 {
                    push_argument(&mut arguments, token.text());
                }
            }
            "," if depth == 1 => arguments.push(String::new()),
            text => push_argument(&mut arguments, text),
        }
    }
    arguments
        .into_iter()
        .filter(|argument| !argument.is_empty())
        .collect()
}

fn push_argument(arguments: &mut [String], text: &str) {
    if let Some(argument) = arguments.last_mut() {
        argument.push_str(text);
    }
}

/// Does this node sit under a `#[cfg(test)]` module or inside a `#[test]`
/// function? Those are not a Cargo target, so no target kind names them, and
/// they are exactly the non-production code a census must not charge for.
fn test_context(node: &SyntaxNode) -> Option<DiagnosticContext> {
    node.ancestors()
        .any(|ancestor| {
            let gated = ancestor
                .children()
                .filter_map(ast::Attr::cast)
                .any(|attribute| compact(attribute.syntax()) == "#[cfg(test)]");
            let test_function = ast::Fn::cast(ancestor.clone()).is_some_and(|function| {
                function
                    .attrs()
                    .any(|attribute| compact(attribute.syntax()) == "#[test]")
            });
            gated || test_function
        })
        .then_some(DiagnosticContext::Tests)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use cargo_metadata::{Metadata, MetadataCommand};
    use ra_ap_syntax::{Edition, SourceFile};

    use super::*;
    use crate::source_kernel::enumerate;

    fn metadata(relative: &str) -> Metadata {
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(relative)
            .join("Cargo.toml");
        MetadataCommand::new()
            .manifest_path(manifest)
            .no_deps()
            .other_options(["--offline".to_owned(), "--locked".to_owned()])
            .exec()
            .expect("fixture metadata should load")
    }

    /// US-007: what the bounded nomination drops, recorded rather than
    /// asserted away.
    ///
    /// The head an exact overlap bound asks for is half of a shape, which is
    /// no index at all, so the head that ships is cut to a constant and the
    /// nomination stops being exact. The corpus below is this repository,
    /// because the number worth publishing is what the step costs on real Rust
    /// and not on a generator written to flatter it.
    #[test]
    fn the_nomination_keeps_what_an_exhaustive_score_finds() {
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        let repository = MetadataCommand::new()
            .manifest_path(manifest)
            .no_deps()
            .other_options(["--offline".to_owned(), "--locked".to_owned()])
            .exec()
            .expect("this repository should describe itself");
        let enumeration = enumerate(&repository);

        let mut functions = Vec::new();
        for unit in enumeration.units().filter(|unit| unit.parses_cleanly()) {
            let analysed = Unit {
                tree: unit.tree(),
                source: unit.source(),
                line_starts: line_starts(unit.source()),
                context: unit.context(enumeration.contexts()),
            };
            functions.extend(duplication::observe(&analysed, unit.relative_path()));
        }

        let (exhaustive, kept) = duplication::nomination_recall(functions);
        assert!(
            exhaustive >= 20,
            "this repository stopped presenting pairs to recall: {exhaustive}"
        );
        assert!(
            kept * 100 >= exhaustive * 85,
            "the nomination reached {kept} of the {exhaustive} pairs an exhaustive score links"
        );
    }

    /// A workspace with no unit at all returns an empty result and no error:
    /// there is nothing to be partial about.
    #[test]
    fn an_empty_enumeration_produces_neither_finding_nor_error() {
        let scan = analyze(&Enumeration::default(), &PolicyPlan::default());
        assert_eq!(scan, StructureScan::default());
    }

    /// A unit the parser could not read is skipped and named, and the pass
    /// completes over every other unit of the same workspace.
    #[test]
    fn an_unparseable_unit_is_skipped_named_and_never_aborts_the_pass() {
        let enumeration = enumerate(&metadata("source-kernel/errors"));
        let scan = analyze(&enumeration, &PolicyPlan::default());

        let skipped: Vec<&str> = scan
            .errors
            .iter()
            .map(|error| {
                assert_eq!(error.code, "parse-error");
                error.message.as_str()
            })
            .collect();
        assert!(!skipped.is_empty(), "the fixture parses cleanly after all");
        assert!(
            skipped
                .iter()
                .all(|message| !message.contains(env!("CARGO_MANIFEST_DIR"))),
            "{skipped:?}"
        );
        let analysed = enumeration
            .units()
            .filter(|unit| unit.parses_cleanly())
            .count();
        assert!(
            analysed > 0,
            "the pass stopped at the first unreadable unit"
        );
    }

    /// US-009, FR-10: a budget the pass cannot meet stops it rather than
    /// letting it run, and the stop is published as an error under the
    /// `structure` stage, which is what takes the authoritative flag off the
    /// score.
    #[test]
    fn an_exhausted_budget_stops_the_pass_and_says_so() {
        let enumeration = enumerate(&metadata("structure/duplicate-function"));
        let stopped = analyze_within(&enumeration, &PolicyPlan::default(), Duration::ZERO);
        assert_eq!(
            stopped
                .errors
                .iter()
                .map(|error| error.code)
                .collect::<Vec<_>>(),
            ["time-budget"],
            "{:?}",
            stopped.errors
        );
        assert!(
            stopped.errors.iter().all(|error| !error
                .message
                .contains(env!("CARGO_MANIFEST_DIR"))),
            "{:?}",
            stopped.errors
        );

        // The same workspace under the shipped budget finishes, so the stop
        // above is the budget and not the workspace.
        let complete = analyze(&enumeration, &PolicyPlan::default());
        assert!(complete.errors.is_empty(), "{:?}", complete.errors);
        assert!(!complete.findings.is_empty());
    }

    /// The pass is switched off by the policy like any other producer, and
    /// costs nothing when it is.
    #[test]
    fn an_inactive_rule_leaves_the_pass_with_nothing_to_do() {
        let enumeration = enumerate(&metadata("structure/unreasoned-allow"));
        let input =
            crate::policy::PolicyInput::default().with_rule(STRUCTURE_UNREASONED_ALLOW.id, crate::policy::RuleLevel::Off);
        let plan = PolicyPlan::compile(&input).expect("policy should compile");
        assert_eq!(
            analyze(&enumeration, &plan),
            StructureScan::default(),
            "an inactive structural rule still produced a finding"
        );
        assert!(!analyze(&enumeration, &PolicyPlan::default())
            .findings
            .is_empty());
    }

    fn observe(source: &str, context: Option<DiagnosticContext>) -> Vec<Observation> {
        let unit = Unit {
            tree: SourceFile::parse(source, Edition::Edition2024).tree(),
            source,
            line_starts: line_starts(source),
            context,
        };
        unreasoned_allow(&unit)
    }

    #[test]
    fn a_reason_and_an_expectation_are_both_left_alone() {
        assert!(observe("#[allow(dead_code)]\nfn free() {}", None).len() == 1);
        for quiet in [
            "#[allow(dead_code, reason = \"the surface is public\")]\nfn free() {}",
            "#[expect(dead_code)]\nfn free() {}",
            "#[cfg_attr(test, allow(dead_code))]\nfn free() {}",
            "#[deny(dead_code)]\nfn free() {}",
            "fn free() {}",
        ] {
            assert!(observe(quiet, None).is_empty(), "{quiet}");
        }
    }

    /// The key carries the lint set and the attribute form, never a position,
    /// so the same exemption written twice forms one family.
    #[test]
    fn the_key_normalizes_order_and_ignores_position() {
        let first = observe("#[allow(b, a)]\nfn one() {}", None);
        let second = observe("\n\n\n#[allow(a, b)]\nfn two() {}", None);
        assert_eq!(first.len(), 1);
        assert_eq!(second.len(), 1);
        assert_eq!(first[0].key, second[0].key);
        assert_ne!(first[0].span, second[0].span);

        let inner = observe("#![allow(a, b)]", None);
        assert_eq!(inner.len(), 1);
        assert_ne!(inner[0].key, first[0].key);
    }

    #[test]
    fn a_cfg_test_module_marks_its_own_allows() {
        let source = "#[allow(dead_code)]\nfn shipped() {}\n\
             #[cfg(test)]\nmod tests {\n    #[allow(dead_code)]\n    fn helper() {}\n}";
        let observed = observe(source, None);
        assert_eq!(observed.len(), 2);
        assert_eq!(observed[0].context, None);
        assert_eq!(observed[1].context, Some(DiagnosticContext::Tests));
    }

    #[test]
    fn a_nested_group_stays_inside_one_argument() {
        let observed = observe("#[allow(clippy::all, other(inner, nested))]\nfn free() {}", None);
        assert_eq!(observed.len(), 1);
        assert_eq!(observed[0].key, "outer|clippy::all,other(inner,nested)");
    }

    /// The identity of a family is its rule and its key, and nothing else.
    #[test]
    fn the_structural_hash_depends_only_on_the_rule_and_the_key() {
        let hash = structural_hash("rust_doctor::structure::unreasoned_allow_attribute", "outer|a");
        assert_eq!(
            hash,
            structural_hash("rust_doctor::structure::unreasoned_allow_attribute", "outer|a")
        );
        assert_ne!(
            hash,
            structural_hash("rust_doctor::structure::unreasoned_allow_attribute", "outer|b")
        );
        assert_ne!(hash, structural_hash("rust_doctor::structure::other", "outer|a"));
        assert_eq!(hash.len(), 64);
    }

    #[test]
    fn a_family_is_marked_only_when_every_member_agrees() {
        let production = Member {
            path: "src/lib.rs".to_owned(),
            span: SourceSpan {
                line_start: 1,
                column_start: 1,
                line_end: 1,
                column_end: 2,
            },
            context: None,
        };
        let tested = Member {
            context: Some(DiagnosticContext::Tests),
            ..production.clone()
        };
        assert_eq!(
            unanimous_context(std::slice::from_ref(&tested)),
            Some(DiagnosticContext::Tests)
        );
        assert_eq!(
            unanimous_context(&[tested.clone(), tested.clone()]),
            Some(DiagnosticContext::Tests)
        );
        assert_eq!(unanimous_context(&[tested, production.clone()]), None);
        assert_eq!(unanimous_context(&[production]), None);
        assert_eq!(unanimous_context(&[]), None);
    }
}

/// US-009: what the pass is allowed to cost on a workspace of a thousand files.
///
/// The benchmark is generated rather than committed: ten thousand functions
/// are a megabyte of source that would say nothing to a reader of this
/// repository, and generating them keeps the shape of the workload written
/// down instead of frozen.
#[cfg(test)]
mod benchmark {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    use cargo_metadata::MetadataCommand;

    use super::*;
    use crate::source_kernel::enumerate;

    const FILES: usize = 1_000;
    const FUNCTIONS_PER_FILE: usize = 10;

    /// Absolute bound the NFR sets on this workload, for the binary that
    /// ships.
    const BUDGET: Duration = Duration::from_millis(2_000);

    /// A test binary is unoptimized, and the pass runs about thirteen times
    /// slower there: 21.1 s against 1.63 s, measured on 2026-08-08 on the same
    /// workspace. The NFR is written for what ships, so an unoptimized run is
    /// allowed a multiple of every bound the pass appears in, rather than held
    /// to one the profile makes unreachable.
    ///
    /// The share below needs it most: its numerator is unoptimized Rust while
    /// its denominator is a Clippy subprocess running at full speed whatever
    /// the profile, so a debug run reads the share thirteen times too high.
    const DEBUG_ALLOWANCE: u32 = 14;

    /// Multiple of every bound this profile is allowed.
    const fn allowance() -> u32 {
        if cfg!(debug_assertions) {
            DEBUG_ALLOWANCE
        } else {
            1
        }
    }

    /// What the pass measured here on 2026-08-08, in milliseconds, on the
    /// development machine. The assertion below allows a quarter more than
    /// this: a change costing more than that is a regression to explain, not a
    /// number to raise quietly.
    const RECORDED_MILLISECONDS: u128 = if cfg!(debug_assertions) { 21_100 } else { 1_650 };

    /// Share of a whole scan the structural pass may take.
    const SHARE_PERCENT: u128 = 15;

    /// Peak the pass may hold for the functions it kept.
    const MEMORY_LIMIT_BYTES: usize = 200 * 1024 * 1024;

    const OPERATORS: [&str; 6] = ["+", "-", "*", "|", "&", "^"];
    const LITERALS: [&str; 6] = ["1", "2.5", "\"text\"", "true", "'c'", "7u64"];

    /// One statement of a generated body.
    ///
    /// The canonical form keeps control flow, operators and literal types, so
    /// these are what the draw varies. Eight kinds over six operators and six
    /// literal types is a space wide enough that two bodies of a dozen
    /// statements rarely land close, which is the property the workload needs:
    /// real code is mostly shapes that are all different, with a few families
    /// inside it.
    fn statement(kind: u64, operator: &str, literal: &str, binding: usize) -> String {
        match kind % 8 {
            0 => format!("    for value in values {{ total = total {operator} *value; }}\n"),
            1 => format!(
                "    if total > limit {{ total = total {operator} limit; }} else {{ total = total {operator} 1; }}\n"
            ),
            2 => format!("    while total < limit {{ total = total {operator} 2; }}\n"),
            3 => format!(
                "    match total {{ 0 => total = total {operator} 3, 1 => total = limit, _ => total = total {operator} limit }};\n"
            ),
            4 => format!(
                "    let hold{binding} = values.iter().copied().filter(|v| *v {operator} 1 > limit).count() as u32; total = total {operator} hold{binding};\n"
            ),
            5 => format!(
                "    if let Some(v) = values.first() {{ let hold{binding} = *v {operator} limit; total = total {operator} hold{binding}; }}\n"
            ),
            6 => format!(
                "    let hold{binding} = ({literal}, total {operator} limit); total = total {operator} hold{binding}.1;\n"
            ),
            _ => format!(
                "    let hold{binding} = |x: u32| x {operator} limit; total = hold{binding}(total);\n"
            ),
        }
    }

    /// One function, drawn from a seeded sequence.
    ///
    /// What this workload has to present is ten thousand *shapes*, not ten
    /// thousand functions: a canonical form is what the pass compares, so a
    /// generator producing five forms ten thousand times collapses to five
    /// before anything expensive runs, and the scoring loop the budget is
    /// written against never sees the workload the assertion claims. It must
    /// also keep those shapes apart: a workspace where every function is a
    /// near duplicate of every other has a quadratic answer, not a quadratic
    /// pass, and measures nothing either. Every hundredth function repeats the
    /// shape of the one before it, which plants the families the pass must
    /// still find among shapes that are otherwise all different.
    fn source(position: usize) -> String {
        let seed = if position % 100 == 99 {
            position.saturating_sub(1)
        } else {
            position
        };
        let mut state = (seed as u64).wrapping_add(1).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        let mut draw = move || {
            state ^= state >> 30;
            state = state.wrapping_mul(0xbf58_476d_1ce4_e5b9);
            state ^= state >> 27;
            state
        };
        let arity = 1 + draw() as usize % 3;
        let statements = 5 + draw() as usize % 11;
        let mut body = format!(
            "    let mut total = 0;\n    let limit = {};\n",
            (0..arity)
                .map(|index| format!("limit{index}"))
                .collect::<Vec<_>>()
                .join(" + ")
        );
        for binding in 0..statements {
            let drawn = draw();
            body.push_str(&statement(
                drawn,
                OPERATORS[(drawn as usize >> 3) % OPERATORS.len()],
                LITERALS[(drawn as usize >> 6) % LITERALS.len()],
                binding,
            ));
        }
        let parameters = (0..arity)
            .map(|index| format!("limit{index}: u32"))
            .fold("values: &[u32]".to_owned(), |left, right| {
                format!("{left}, {right}")
            });
        format!("pub fn shape_{position}({parameters}) -> u32 {{\n{body}    total\n}}\n")
    }

    fn workspace() -> PathBuf {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/structure-benchmark")
            .join(std::process::id().to_string());
        let sources = root.join("src");
        fs::create_dir_all(&sources).expect("the benchmark workspace should be writable");
        let mut declarations = String::new();
        for file in 0..FILES {
            let mut unit = String::new();
            for index in 0..FUNCTIONS_PER_FILE {
                unit.push_str(&source(file * FUNCTIONS_PER_FILE + index));
                unit.push('\n');
            }
            fs::write(sources.join(format!("m{file}.rs")), unit).expect("a module should write");
            declarations.push_str(&format!("pub mod m{file};\n"));
        }
        fs::write(sources.join("lib.rs"), declarations).expect("the root should write");
        fs::write(
            root.join("Cargo.toml"),
            concat!(
                "[package]\n",
                "name = \"structure-benchmark\"\n",
                "version = \"0.1.0\"\n",
                "edition = \"2024\"\n",
                "publish = false\n\n",
                "[lib]\n",
                "path = \"src/lib.rs\"\n",
            ),
        )
        .expect("the manifest should write");
        root
    }

    #[test]
    fn the_pass_holds_its_budget_on_a_thousand_files() {
        let root = workspace();
        let metadata = MetadataCommand::new()
            .manifest_path(root.join("Cargo.toml"))
            .no_deps()
            .other_options(["--offline".to_owned()])
            .exec()
            .expect("the benchmark metadata should load");

        let enumeration = enumerate(&metadata);

        // The best of three, where three are affordable: the suite runs its
        // tests in parallel, so a single sample measures the scheduler as much
        // as it measures the pass. An unoptimized run costs twenty seconds a
        // sample and is already allowed an order of magnitude, which is wider
        // than the noise the repetition removes.
        //
        // The shipped stop-and-report budget is scaled with everything else an
        // unoptimized profile is allowed here: it is a bound on what a user
        // waits for, and holding a test binary to it would stop the pass
        // mid-workload and measure the stop instead of the pass.
        let mut pass = Duration::MAX;
        let mut scan = StructureScan::default();
        for _ in 0..if cfg!(debug_assertions) { 1 } else { 3 } {
            let started = Instant::now();
            scan = analyze_within(
                &enumeration,
                &PolicyPlan::default(),
                TIME_BUDGET * allowance(),
            );
            pass = pass.min(started.elapsed());
        }

        assert!(scan.errors.is_empty(), "{:?}", scan.errors);
        assert_eq!(
            scan.counters.functions,
            FILES * FUNCTIONS_PER_FILE,
            "the benchmark did not present the workload it claims"
        );
        // The shapes are the workload. A generator whose functions collapse
        // into a handful of canonical forms leaves the scoring loop with
        // nothing to do and reports a budget it never tested.
        assert!(
            scan.counters.shapes * 100 >= scan.counters.functions * 95,
            "the benchmark presented {} functions in only {} shapes",
            scan.counters.functions,
            scan.counters.shapes
        );
        assert!(
            !scan.findings.is_empty(),
            "the benchmark planted clone families the pass did not find"
        );
        // US-007, and the NFR behind it: the scoring must be nominated, not
        // pairwise. This is the assertion that says so, and it holds on any
        // machine, unlike the two clocks below.
        let pairwise = scan.counters.shapes * scan.counters.shapes / 2;
        assert!(
            scan.counters.comparisons * 20 <= pairwise,
            "the near-duplicate pass scored {} pairs of {} shapes, against {pairwise} for a pairwise scan",
            scan.counters.comparisons,
            scan.counters.shapes
        );

        let budget = BUDGET * allowance();
        assert!(
            pass <= budget,
            "the structural pass took {pass:?}, over its {budget:?} budget"
        );
        assert!(
            pass.as_millis() * 4 <= RECORDED_MILLISECONDS * 5,
            "the structural pass took {pass:?}, more than a quarter over the recorded {RECORDED_MILLISECONDS} ms"
        );
        assert!(
            scan.counters.retained_bytes <= MEMORY_LIMIT_BYTES,
            "the pass held {} bytes for {} functions",
            scan.counters.retained_bytes,
            scan.counters.functions
        );

        // The share is measured against a whole scan rather than against the
        // walk it shares with the source kernel, because the number the NFR
        // bounds is what the user waits for. The scan below compiles the same
        // thousand files through Clippy, which is where that wait comes from.
        //
        // Only what ships is measured here. An unoptimized numerator over a
        // Clippy denominator that runs at full speed whatever the profile
        // reads the share an order of magnitude too high, and the shipped
        // stop-and-report budget cuts a debug pass short on this workload,
        // which is the pass behaving as FR-10 asks and not a measurement. So
        // `cargo test --release` is what proves this bound.
        if !cfg!(debug_assertions) {
            let started = Instant::now();
            let report = crate::inspect(crate::InspectRequest::new(&root));
            let whole = started.elapsed();
            assert_eq!(report.status, crate::Status::Complete, "{:?}", report.errors);
            assert!(
                pass.as_millis() * 100 <= SHARE_PERCENT * whole.as_millis(),
                "the structural pass took {pass:?} of a {whole:?} scan, over its {SHARE_PERCENT}% share"
            );
        }

        let _ = fs::remove_dir_all(&root);
    }
}
