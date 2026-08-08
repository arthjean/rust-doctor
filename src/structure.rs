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

use ra_ap_syntax::ast::{self, HasAttrs};
use ra_ap_syntax::{AstNode, SyntaxNode};

use crate::policy::{PolicyPlan, RuleDefinition, STRUCTURE_UNREASONED_ALLOW};
use crate::report::DiagnosticContext;
use crate::source_kernel::{Enumeration, SourceSpan, compact, line_starts, source_span};

const FINGERPRINT_DOMAIN: &str = "rust-doctor-structure-fingerprint-v1";

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct StructureScan {
    pub(crate) findings: Vec<StructureFinding>,
    pub(crate) errors: Vec<StructureError>,
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
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Member {
    path: String,
    span: SourceSpan,
    context: Option<DiagnosticContext>,
}

pub(crate) fn analyze(enumeration: &Enumeration, plan: &PolicyPlan) -> StructureScan {
    let detectors: Vec<&'static Detector> = DETECTORS
        .iter()
        .copied()
        .filter(|detector| plan.is_active(detector.definition.id))
        .collect();
    if detectors.is_empty() {
        return StructureScan::default();
    }

    let mut errors = Vec::new();
    let mut families = BTreeMap::<(&'static str, String), Family>::new();
    for unit in enumeration.units() {
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
    }

    errors.sort();
    errors.dedup();
    StructureScan {
        findings: findings(&detectors, families, enumeration),
        errors,
    }
}

fn findings(
    detectors: &[&'static Detector],
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
            let definition = detectors
                .iter()
                .find(|detector| detector.definition.id == rule)?
                .definition;
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
