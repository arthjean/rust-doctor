//! Functions too tangled to review, and units too large to keep growing.
//!
//! Both detectors answer the reviewer's question rather than the compiler's:
//! not "is this call site wrong?" but "how much of this can one reading hold?".
//! A complexity figure is computed per function, a size figure per unit, and a
//! unit crosses a threshold once: the finding is the unit, never the threshold,
//! so a function over two bounds still costs the report one line.
//!
//! Cyclomatic complexity counts the paths: one, plus one per branch point,
//! where a branch point is an `if`, a `while`, a `for`, a `loop`, a match arm,
//! a lazy boolean operator or a `?`. Cognitive complexity follows SonarSource's
//! nesting-weighted definition: a branch costs one plus its nesting depth, an
//! `else` and a chained `else if` cost one flat, a run of one logical operator
//! costs one however long it is, and a labeled jump costs one. Two functions
//! with the same branch count therefore separate exactly when one buries its
//! branches deeper, which is the property the cyclomatic figure cannot see.
//!
//! The two detectors share one walk of the tree. The structural pass already
//! walks every unit for the duplication rules, and a wall-clock budget bounds
//! the whole pass, so a detector that can piggyback on a traversal must not
//! start its own.

use ra_ap_syntax::ast::{self, HasName};
use ra_ap_syntax::{AstNode, SyntaxKind, SyntaxNode};

use super::{Observation, StructureSettings, Unit, test_context};
use crate::policy::{
    PolicyPlan, RuleDefinition, STRUCTURE_COMPLEX_FUNCTION, STRUCTURE_OVERSIZED_UNIT,
};
use crate::report::ComplexityFigures;
use crate::source_kernel::compact;

/// Lines a file may reach before it is named.
///
/// Measured on this repository on 2026-08-08: six of its source files sit
/// above a thousand lines, `src/report.rs` and `src/audit.rs` among them, and
/// each is a file its own history routes around rather than reads. Naming them
/// on a self-scan is the admission the PRD asks the tool to make about its own
/// code.
pub(super) const FILE_LINES: usize = 1_000;

/// Lines an inline `mod` block may reach before it is named.
pub(super) const MODULE_LINES: usize = 500;

/// Lines an `impl` block may reach before it is named.
pub(super) const IMPL_LINES: usize = 500;

/// Lines one function may reach before it is named. Clippy's `too_many_lines`
/// draws this line at 100; a hotspot detector reports the tail, not the norm,
/// so it sits half again higher.
pub(super) const FUNCTION_LINES: usize = 150;

/// Which halves of the hotspot walk the policy leaves on.
#[derive(Debug, Clone, Copy)]
pub(super) struct Active {
    pub(super) complexity: bool,
    pub(super) size: bool,
}

impl Active {
    pub(super) fn of(plan: &PolicyPlan) -> Self {
        Self {
            complexity: plan.is_active(STRUCTURE_COMPLEX_FUNCTION.id),
            size: plan.is_active(STRUCTURE_OVERSIZED_UNIT.id),
        }
    }

    pub(super) const fn any(self) -> bool {
        self.complexity || self.size
    }
}

/// Cyclomatic and cognitive complexity of one function, measured together in
/// one walk of its body.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct Metrics {
    pub(super) cyclomatic: u32,
    pub(super) cognitive: u32,
}

/// Every hotspot of one unit, in one traversal: the functions whose complexity
/// crosses a threshold, and the units grown past the line bound of their kind.
///
/// The keys carry the qualified name under the file, never a position, so a
/// finding keeps its identity while its unit stays where it is, however the
/// code above it moves.
pub(super) fn observe(
    unit: &Unit<'_>,
    settings: &StructureSettings,
    active: Active,
) -> Vec<(&'static RuleDefinition, Observation)> {
    let mut observations = Vec::new();
    if active.size {
        let file_lines = lines_of(unit, unit.tree.syntax());
        if file_lines >= FILE_LINES {
            observations.push((
                &STRUCTURE_OVERSIZED_UNIT,
                Observation {
                    key: format!("file|{}", unit.path),
                    subject: format!("{} is {file_lines} lines long.", unit.path),
                    span: unit.span(unit.tree.syntax()),
                    context: unit.context,
                    complexity: None,
                },
            ));
        }
    }

    for node in unit.tree.syntax().descendants() {
        match node.kind() {
            SyntaxKind::FN => {
                let Some(function) = ast::Fn::cast(node.clone()) else {
                    continue;
                };
                let Some(body) = function.body() else {
                    continue;
                };
                if active.size {
                    let lines = lines_of(unit, &node);
                    if lines >= FUNCTION_LINES
                        && let Some(name) = qualified_name(&function)
                    {
                        observations.push((
                            &STRUCTURE_OVERSIZED_UNIT,
                            sized(unit, &node, format!("Function {name}"), "fn", lines),
                        ));
                    }
                }
                if active.complexity {
                    let mut metrics = Metrics::default();
                    measure(body.syntax(), 0, &mut metrics);
                    metrics.cyclomatic += 1;
                    if metrics.cyclomatic >= settings.cyclomatic_threshold
                        || metrics.cognitive >= settings.cognitive_threshold
                    {
                        let Some(name) = qualified_name(&function) else {
                            continue;
                        };
                        observations.push((
                            &STRUCTURE_COMPLEX_FUNCTION,
                            Observation {
                                key: format!("fn|{}|{name}", unit.path),
                                subject: format!(
                                    "Function {name} reaches cyclomatic complexity {} and cognitive complexity {}.",
                                    metrics.cyclomatic, metrics.cognitive
                                ),
                                span: unit.span(&node),
                                context: test_context(&node).or(unit.context),
                                complexity: Some(ComplexityFigures {
                                    cyclomatic: metrics.cyclomatic,
                                    cognitive: metrics.cognitive,
                                }),
                            },
                        ));
                    }
                }
            }
            SyntaxKind::IMPL if active.size => {
                let lines = lines_of(unit, &node);
                if lines >= IMPL_LINES
                    && let Some(block) = ast::Impl::cast(node.clone())
                {
                    let label = format!("Impl block {}", impl_label(&block));
                    observations.push((
                        &STRUCTURE_OVERSIZED_UNIT,
                        sized(unit, &node, label, "impl", lines),
                    ));
                }
            }
            SyntaxKind::MODULE if active.size => {
                let lines = lines_of(unit, &node);
                if lines >= MODULE_LINES
                    && let Some(name) = ast::Module::cast(node.clone())
                        .filter(|module| module.item_list().is_some())
                        .and_then(|module| module.name())
                {
                    observations.push((
                        &STRUCTURE_OVERSIZED_UNIT,
                        sized(unit, &node, format!("Module {name}"), "mod", lines),
                    ));
                }
            }
            _ => {}
        }
    }
    observations
}

fn sized(
    unit: &Unit<'_>,
    node: &SyntaxNode,
    label: String,
    kind: &str,
    lines: usize,
) -> Observation {
    Observation {
        key: format!("{kind}|{}|{label}", unit.path),
        subject: format!("{label} spans {lines} lines."),
        span: unit.span(node),
        context: test_context(node).or(unit.context),
        complexity: None,
    }
}

/// Lines a node covers, from the line index alone. The full span, with its
/// column arithmetic, is only computed for the units actually reported.
///
/// The end is read at the last character of the node, never at the position
/// after it. A file ends on its final newline, and that position sits on the
/// empty line past it: counted, every file ending the way files end would be
/// reported one line longer than it is.
fn lines_of(unit: &Unit<'_>, node: &SyntaxNode) -> usize {
    let range = node.text_range();
    let first = usize::from(range.start());
    let last = usize::from(range.end()).saturating_sub(1).max(first);
    let start = unit.line_starts.partition_point(|offset| *offset <= first);
    let end = unit.line_starts.partition_point(|offset| *offset <= last);
    end.saturating_sub(start).saturating_add(1)
}

/// Both figures of one body, without the constant one cyclomatic starts from.
///
/// A nested function is skipped: it is measured on its own, and charging its
/// branches to the function that merely contains it would report the container
/// for the contained. A closure is not skipped, because it has no name to be
/// reported under: its branches belong to the function that wrote it, one
/// nesting level down.
pub(super) fn measure(node: &SyntaxNode, nesting: u32, metrics: &mut Metrics) {
    for child in node.children() {
        match child.kind() {
            SyntaxKind::FN => {}
            SyntaxKind::IF_EXPR => {
                // A chained `else if` continues the decision the first `if`
                // opened: it costs one flat, and its branches stay at the depth
                // of the chain instead of sinking one level per link.
                let chained = child
                    .parent()
                    .is_some_and(|parent| parent.kind() == SyntaxKind::IF_EXPR);
                metrics.cyclomatic += 1;
                metrics.cognitive += 1 + if chained { 0 } else { nesting };
                if ast::IfExpr::cast(child.clone())
                    .and_then(|expression| expression.else_branch())
                    .is_some_and(|branch| matches!(branch, ast::ElseBranch::Block(_)))
                {
                    metrics.cognitive += 1;
                }
                measure(&child, if chained { nesting } else { nesting + 1 }, metrics);
            }
            SyntaxKind::MATCH_EXPR => {
                metrics.cognitive += 1 + nesting;
                measure(&child, nesting + 1, metrics);
            }
            SyntaxKind::MATCH_ARM => {
                metrics.cyclomatic += 1;
                measure(&child, nesting, metrics);
            }
            SyntaxKind::FOR_EXPR | SyntaxKind::WHILE_EXPR | SyntaxKind::LOOP_EXPR => {
                metrics.cyclomatic += 1;
                metrics.cognitive += 1 + nesting;
                measure(&child, nesting + 1, metrics);
            }
            SyntaxKind::CLOSURE_EXPR => measure(&child, nesting + 1, metrics),
            SyntaxKind::BIN_EXPR => {
                if let Some(ast::BinaryOp::LogicOp(operator)) =
                    ast::BinExpr::cast(child.clone()).and_then(|expression| expression.op_kind())
                {
                    metrics.cyclomatic += 1;
                    // `a && b && c` is one thought; `a && b || c` is two. Only
                    // the first operator of a run costs.
                    if child
                        .parent()
                        .and_then(ast::BinExpr::cast)
                        .and_then(|parent| parent.op_kind())
                        != Some(ast::BinaryOp::LogicOp(operator))
                    {
                        metrics.cognitive += 1;
                    }
                }
                measure(&child, nesting, metrics);
            }
            SyntaxKind::TRY_EXPR => {
                metrics.cyclomatic += 1;
                measure(&child, nesting, metrics);
            }
            SyntaxKind::BREAK_EXPR | SyntaxKind::CONTINUE_EXPR => {
                let labeled = ast::BreakExpr::cast(child.clone())
                    .is_some_and(|jump| jump.lifetime().is_some())
                    || ast::ContinueExpr::cast(child.clone())
                        .is_some_and(|jump| jump.lifetime().is_some());
                if labeled {
                    metrics.cognitive += 1;
                }
                measure(&child, nesting, metrics);
            }
            _ => measure(&child, nesting, metrics),
        }
    }
}

/// Name of a function under everything that scopes it in its file: modules,
/// impl blocks, traits, and the function it may be nested in.
fn qualified_name(function: &ast::Fn) -> Option<String> {
    let mut segments = vec![function.name()?.text().to_string()];
    for ancestor in function.syntax().ancestors().skip(1) {
        match ancestor.kind() {
            SyntaxKind::MODULE => {
                if let Some(name) = ast::Module::cast(ancestor).and_then(|module| module.name()) {
                    segments.push(name.text().to_string());
                }
            }
            SyntaxKind::IMPL => {
                if let Some(block) = ast::Impl::cast(ancestor) {
                    segments.push(impl_label(&block));
                }
            }
            SyntaxKind::TRAIT => {
                if let Some(name) = ast::Trait::cast(ancestor).and_then(|item| item.name()) {
                    segments.push(name.text().to_string());
                }
            }
            SyntaxKind::FN => {
                if let Some(name) = ast::Fn::cast(ancestor).and_then(|outer| outer.name()) {
                    segments.push(name.text().to_string());
                }
            }
            _ => {}
        }
    }
    segments.reverse();
    Some(segments.join("::"))
}

/// What an `impl` block is for: `Display for Report` names the trait
/// implementation, `Report` the inherent one.
fn impl_label(block: &ast::Impl) -> String {
    let self_type = block
        .self_ty()
        .map(|type_| compact(type_.syntax()))
        .unwrap_or_default();
    match block.trait_() {
        Some(trait_) => format!("{} for {self_type}", compact(trait_.syntax())),
        None => self_type,
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use cargo_metadata::MetadataCommand;
    use ra_ap_syntax::{Edition, SourceFile};

    use super::*;
    use crate::report::DiagnosticContext;
    use crate::source_kernel::{enumerate, line_starts};
    use crate::structure::analyze;

    const COMPLEXITY_ONLY: Active = Active {
        complexity: true,
        size: false,
    };
    const SIZE_ONLY: Active = Active {
        complexity: false,
        size: true,
    };

    fn unit(source: &str) -> Unit<'_> {
        Unit {
            tree: SourceFile::parse(source, Edition::Edition2024).tree(),
            source,
            line_starts: line_starts(source),
            path: "src/lib.rs",
            context: None,
            packages: Vec::new(),
            attributes: Vec::new(),
        }
    }

    fn measured(body: &str) -> Metrics {
        let source = format!("fn probe() {{\n{body}\n}}\n");
        let parsed = unit(&source);
        let function = parsed
            .tree
            .syntax()
            .descendants()
            .find_map(ast::Fn::cast)
            .expect("the probe parses");
        let mut metrics = Metrics::default();
        measure(function.body().expect("the probe has a body").syntax(), 0, &mut metrics);
        metrics.cyclomatic += 1;
        metrics
    }

    /// US-010: cyclomatic complexity is the branch points plus one, and every
    /// counted kind counts.
    #[test]
    fn cyclomatic_complexity_is_the_branch_points_plus_one() {
        assert_eq!(measured("let value = 1;").cyclomatic, 1);
        // One if, one while, one for, one loop, two match arms, one `&&`, one
        // `?`-free body: eight branch points.
        let branched = "
            if a { b(); }
            while c { d(); }
            for item in items { e(item); }
            loop { break; }
            match f {
                0 => g(),
                _ => h(),
            }
            if i && j { k(); }
        ";
        assert_eq!(measured(branched).cyclomatic, 9);
        assert_eq!(measured("let value = fallible()?;").cyclomatic, 2);
    }

    /// US-010: two functions with equal cyclomatic complexity separate on
    /// nesting, and the deeper one reports the higher cognitive figure.
    #[test]
    fn equal_cyclomatic_complexity_still_separates_on_nesting() {
        let sequential = measured("if a { b(); }\nif c { d(); }\nif e { f(); }");
        let nested = measured("if a { if c { if e { f(); } } }");
        assert_eq!(sequential.cyclomatic, nested.cyclomatic);
        assert_eq!(sequential.cognitive, 3);
        assert_eq!(nested.cognitive, 6);
    }

    /// The SonarSource discounts: a chained `else if` costs one flat, an
    /// `else` costs one, and a run of one logical operator costs one however
    /// long it runs.
    #[test]
    fn chains_runs_and_elses_cost_one_each() {
        let chain = measured("if a { b(); } else if c { d(); } else { e(); }");
        assert_eq!(chain.cyclomatic, 3);
        assert_eq!(chain.cognitive, 3);

        assert_eq!(measured("if a && b && c { d(); }").cognitive, 2);
        assert_eq!(measured("if a && b || c { d(); }").cognitive, 3);

        let labeled = measured("'outer: for a in b { for c in d { continue 'outer; } }");
        assert_eq!(labeled.cognitive, 4);
    }

    /// A nested function is measured on its own; a closure charges the
    /// function that wrote it, one level down.
    #[test]
    fn a_nested_function_is_its_own_measure_and_a_closure_is_not() {
        let with_nested = measured("fn inner() { if a { b(); } }\nc();");
        assert_eq!(with_nested.cyclomatic, 1);
        assert_eq!(with_nested.cognitive, 0);

        let with_closure = measured("let probe = |x| if x { 1 } else { 2 };");
        assert_eq!(with_closure.cyclomatic, 2);
        assert_eq!(with_closure.cognitive, 3);
    }

    /// US-010: below both thresholds nothing is observed, above either the
    /// observation carries both figures and the qualified name.
    #[test]
    fn only_a_function_over_a_threshold_is_observed() {
        let calm = unit("fn calm() { if a { b(); } }");
        assert!(observe(&calm, &StructureSettings::default(), COMPLEXITY_ONLY).is_empty());

        let source = format!(
            "mod deep {{ impl Report {{ fn dense() {{ {} }} }} }}",
            "if a { b(); } ".repeat(25)
        );
        let dense = unit(&source);
        let observed = observe(&dense, &StructureSettings::default(), COMPLEXITY_ONLY);
        assert_eq!(observed.len(), 1);
        assert_eq!(observed[0].0.id, STRUCTURE_COMPLEX_FUNCTION.id);
        let hotspot = &observed[0].1;
        assert_eq!(hotspot.key, "fn|src/lib.rs|deep::Report::dense");
        let figures = hotspot.complexity.expect("a hotspot publishes its figures");
        assert_eq!(figures.cyclomatic, 26);
        assert_eq!(figures.cognitive, 25);
        assert!(
            hotspot.subject.contains("cyclomatic complexity 26")
                && hotspot.subject.contains("cognitive complexity 25"),
            "{}",
            hotspot.subject
        );

        let raised = StructureSettings {
            cyclomatic_threshold: 27,
            cognitive_threshold: 26,
        };
        assert!(observe(&dense, &raised, COMPLEXITY_ONLY).is_empty());
    }

    /// US-011: each unit kind is named once when it crosses its own line
    /// threshold, and the file that carries them is a unit of its own.
    #[test]
    fn every_oversized_unit_is_named_once() {
        let long_function = format!(
            "fn stretched() {{\n{}}}\n",
            "    call();\n".repeat(FUNCTION_LINES)
        );
        let long_impl = format!(
            "impl std::fmt::Debug for Wide {{\n{}}}\n",
            "fn field_0() {}\n".repeat(IMPL_LINES)
        );
        let long_module = format!(
            "mod carried {{\n{}}}\n",
            "    pub const UNIT: u8 = 0;\n".repeat(MODULE_LINES)
        );
        let source = format!("{long_function}{long_impl}{long_module}");
        let observed = observe(&unit(&source), &StructureSettings::default(), SIZE_ONLY);

        let keys: Vec<&str> = observed
            .iter()
            .map(|(definition, entry)| {
                assert_eq!(definition.id, STRUCTURE_OVERSIZED_UNIT.id);
                entry.key.as_str()
            })
            .collect();
        assert_eq!(
            keys,
            [
                "file|src/lib.rs",
                "fn|src/lib.rs|Function stretched",
                "impl|src/lib.rs|Impl block std::fmt::Debug for Wide",
                "mod|src/lib.rs|Module carried",
            ],
            "{keys:?}"
        );
        assert!(
            observed[0].1.subject.starts_with("src/lib.rs is ")
                && observed[0].1.subject.ends_with(" lines long."),
            "{}",
            observed[0].1.subject
        );
        assert!(observed[1].1.subject.contains("spans"), "{}", observed[1].1.subject);

        // One line under each threshold, nothing is observed.
        let calm = format!(
            "fn stretched() {{\n{}}}\n",
            "    call();\n".repeat(FUNCTION_LINES - 3)
        );
        assert!(observe(&unit(&calm), &StructureSettings::default(), SIZE_ONLY).is_empty());
    }

    /// US-011: the line count a file is named with is the one its reader
    /// counts. A file ends on a newline, and the empty position past it is not
    /// a line of the file.
    #[test]
    fn a_file_is_counted_at_its_last_line_and_never_past_it() {
        let terminated = unit("fn a() {}\nfn b() {}\n");
        assert_eq!(lines_of(&terminated, terminated.tree.syntax()), 2);

        let unterminated = unit("fn a() {}\nfn b() {}");
        assert_eq!(lines_of(&unterminated, unterminated.tree.syntax()), 2);

        let empty = unit("");
        assert_eq!(lines_of(&empty, empty.tree.syntax()), 1);
    }

    /// US-011: a recognized generator header excludes the file from the whole
    /// structural pass, silently.
    #[test]
    fn a_generated_file_is_excluded() {
        for header in [
            "// @generated by prost-build",
            "// DO NOT EDIT: regenerated on every build",
            "// Automatically generated by bindgen.",
        ] {
            assert!(super::super::is_generated(&format!("{header}\nfn free() {{}}\n")), "{header}");
        }
        assert!(!super::super::is_generated("fn free() {}\n"));
    }

    /// EP-003, definition of done: a scan of this repository names
    /// `src/report.rs` and `src/audit.rs` as oversized and reports at least
    /// one cognitive complexity hotspot, through the same pass `inspect` runs.
    #[test]
    fn the_self_scan_names_this_repository_s_own_hotspots() {
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        let metadata = MetadataCommand::new()
            .manifest_path(manifest)
            .no_deps()
            .other_options(["--offline".to_owned(), "--locked".to_owned()])
            .exec()
            .expect("this repository should describe itself");
        let scan = analyze(
            &metadata,
            &enumerate(&metadata),
            &crate::policy::PolicyPlan::default(),
            &StructureSettings::default(),
        );

        let oversized_files: Vec<&str> = scan
            .findings
            .iter()
            .filter(|finding| finding.definition.id == STRUCTURE_OVERSIZED_UNIT.id)
            .map(|finding| finding.path.as_str())
            .collect();
        for expected in ["src/report.rs", "src/audit.rs"] {
            assert!(
                oversized_files.contains(&expected),
                "{expected} is no longer named oversized: {oversized_files:?}"
            );
        }

        let hotspot = scan
            .findings
            .iter()
            .find(|finding| {
                finding.definition.id == STRUCTURE_COMPLEX_FUNCTION.id
                    && finding.context != Some(DiagnosticContext::Tests)
                    && finding.complexity.is_some_and(|figures| {
                        figures.cognitive >= StructureSettings::default().cognitive_threshold
                    })
            });
        assert!(
            hotspot.is_some(),
            "the self-scan reports no cognitive complexity hotspot"
        );
    }
}
