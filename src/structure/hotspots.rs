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
//! Neither detector walks the tree. Both read the inventory the unit's single
//! traversal collected, because the pass runs under a wall clock and a
//! traversal per detector spends it once per detector over the same nodes.

use ra_ap_syntax::ast::{self, HasName};
use ra_ap_syntax::{AstNode, SyntaxKind, SyntaxNode};

use super::{Active, Observation, StructureSettings, Unit, test_context};
use crate::policy::{RuleDefinition, STRUCTURE_COMPLEX_FUNCTION, STRUCTURE_OVERSIZED_UNIT};
use crate::report::ComplexityFigures;
use crate::source_text::compact;

/// The rules this half of the pass produces.
pub(super) const RULES: [&RuleDefinition; 2] =
    [&STRUCTURE_COMPLEX_FUNCTION, &STRUCTURE_OVERSIZED_UNIT];

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

/// Cyclomatic and cognitive complexity of one function, measured together in
/// one walk of its body.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct Metrics {
    pub(super) cyclomatic: u32,
    pub(super) cognitive: u32,
}

/// Every hotspot of one unit: the functions whose complexity crosses a
/// threshold, and the units grown past the line bound of their kind.
///
/// The keys carry the qualified name under the file, never a position, so a
/// finding keeps its identity while its unit stays where it is, however the
/// code above it moves.
pub(super) fn observe(
    unit: &Unit<'_>,
    settings: &StructureSettings,
    active: &Active,
) -> Vec<(&'static RuleDefinition, Observation)> {
    let mut observations = Vec::new();
    if active.on(&STRUCTURE_OVERSIZED_UNIT) {
        // The file is a unit like the three kinds inside it, so it goes through
        // the same bound table rather than through a prologue of its own.
        let units = std::iter::once(unit.tree.syntax())
            .chain(unit.inventory.functions.iter().map(AstNode::syntax))
            .chain(unit.inventory.implementations.iter().map(AstNode::syntax))
            .chain(unit.inventory.modules.iter().map(AstNode::syntax));
        for node in units {
            observations.extend(
                oversized(unit, node).map(|observation| (&STRUCTURE_OVERSIZED_UNIT, observation)),
            );
        }
    }
    if active.on(&STRUCTURE_COMPLEX_FUNCTION) {
        for function in &unit.inventory.functions {
            observations.extend(
                complex(unit, function, settings)
                    .map(|observation| (&STRUCTURE_COMPLEX_FUNCTION, observation)),
            );
        }
    }
    observations
}

/// Tag and line bound of the kind of unit this node is, or nothing when the
/// size rule does not judge its kind.
const fn bound(kind: SyntaxKind) -> Option<(&'static str, usize)> {
    match kind {
        SyntaxKind::SOURCE_FILE => Some(("file", FILE_LINES)),
        SyntaxKind::FN => Some(("fn", FUNCTION_LINES)),
        SyntaxKind::IMPL => Some(("impl", IMPL_LINES)),
        SyntaxKind::MODULE => Some(("mod", MODULE_LINES)),
        _ => None,
    }
}

/// The oversized-unit observation this node carries, if it carries one.
fn oversized(unit: &Unit<'_>, node: &SyntaxNode) -> Option<Observation> {
    let (tag, limit) = bound(node.kind())?;
    let lines = lines_of(unit, node);
    if lines < limit {
        return None;
    }
    // Naming a function means walking its ancestors, so the label is built
    // after the bound is crossed rather than for every unit of the file.
    let label = label(unit, node)?;
    let subject = match node.kind() {
        SyntaxKind::SOURCE_FILE => format!("{label} is {lines} lines long."),
        _ => format!("{label} spans {lines} lines."),
    };
    Some(Observation {
        key: format!("{tag}|{}|{label}", unit.path),
        subject,
        span: unit.span(node),
        context: test_context(node).or(unit.context),
        complexity: None,
    })
}

/// What the finding calls this unit, or nothing when the node is not a unit
/// after all: a declaration with no body and an out-of-line module both name
/// code that lives somewhere else, and there is nothing in them to divide.
///
/// A file is called by its path, which is why its key repeats it: one shape of
/// key for the four kinds is worth more than a shorter string for one of them.
fn label(unit: &Unit<'_>, node: &SyntaxNode) -> Option<String> {
    match node.kind() {
        SyntaxKind::SOURCE_FILE => Some(unit.path.to_owned()),
        SyntaxKind::FN => {
            let function = ast::Fn::cast(node.clone())?;
            function.body()?;
            Some(format!("Function {}", qualified_name(&function)?))
        }
        SyntaxKind::IMPL => Some(format!(
            "Impl block {}",
            impl_label(&ast::Impl::cast(node.clone())?)
        )),
        SyntaxKind::MODULE => {
            let module = ast::Module::cast(node.clone())?;
            module.item_list()?;
            Some(format!("Module {}", module.name()?))
        }
        _ => None,
    }
}

/// The complexity observation this function carries, if it crosses a threshold.
fn complex(
    unit: &Unit<'_>,
    function: &ast::Fn,
    settings: &StructureSettings,
) -> Option<Observation> {
    let metrics = metrics_of(&function.body()?);
    if metrics.cyclomatic < settings.cyclomatic_threshold
        && metrics.cognitive < settings.cognitive_threshold
    {
        return None;
    }
    let name = qualified_name(function)?;
    let node = function.syntax();
    Some(Observation {
        key: format!("fn|{}|{name}", unit.path),
        subject: format!(
            "Function {name} reaches cyclomatic complexity {} and cognitive complexity {}.",
            metrics.cyclomatic, metrics.cognitive
        ),
        span: unit.span(node),
        context: test_context(node).or(unit.context),
        complexity: Some(ComplexityFigures {
            cyclomatic: metrics.cyclomatic,
            cognitive: metrics.cognitive,
        }),
    })
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

/// Both figures of one body, in one walk of it.
///
/// Cyclomatic complexity counts the paths, and a body with no branch is one
/// path: that constant lives here, so no caller can measure a body and forget
/// it, and no test can restate it.
pub(super) fn metrics_of(body: &ast::BlockExpr) -> Metrics {
    let mut metrics = Metrics {
        cyclomatic: 1,
        cognitive: 0,
    };
    measure(body.syntax(), 0, &mut metrics);
    metrics
}

/// A nested function is skipped: it is measured on its own, and charging its
/// branches to the function that merely contains it would report the container
/// for the contained. A closure is not skipped, because it has no name to be
/// reported under: its branches belong to the function that wrote it, one
/// nesting level down.
fn measure(node: &SyntaxNode, nesting: u32, metrics: &mut Metrics) {
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
                // A jump naming a loop is the one that costs, and the label is
                // the lifetime token under it, whichever of the two jumps
                // carries it.
                if child
                    .children_with_tokens()
                    .any(|element| element.kind() == SyntaxKind::LIFETIME)
                {
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
    use super::*;

    fn complexity_only() -> Active {
        Active::of_rules([&STRUCTURE_COMPLEX_FUNCTION])
    }

    fn size_only() -> Active {
        Active::of_rules([&STRUCTURE_OVERSIZED_UNIT])
    }

    fn unit(source: &str) -> Unit<'_> {
        Unit::probe(source, "src/lib.rs")
    }

    fn measured(body: &str) -> Metrics {
        let source = format!("fn probe() {{\n{body}\n}}\n");
        let parsed = unit(&source);
        let function = parsed
            .inventory
            .functions
            .first()
            .expect("the probe parses")
            .clone();
        metrics_of(&function.body().expect("the probe has a body"))
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
        let unlabeled = measured("for a in b { for c in d { continue; } }");
        assert_eq!(unlabeled.cognitive, 3);
        let labeled_break = measured("'outer: for a in b { for c in d { break 'outer; } }");
        assert_eq!(labeled_break.cognitive, 4);
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
        assert!(observe(&calm, &StructureSettings::default(), &complexity_only()).is_empty());

        let source = format!(
            "mod deep {{ impl Report {{ fn dense() {{ {} }} }} }}",
            "if a { b(); } ".repeat(25)
        );
        let dense = unit(&source);
        let observed = observe(&dense, &StructureSettings::default(), &complexity_only());
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
        assert!(observe(&dense, &raised, &complexity_only()).is_empty());
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
        let observed = observe(&unit(&source), &StructureSettings::default(), &size_only());

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
                "file|src/lib.rs|src/lib.rs",
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
        assert!(observe(&unit(&calm), &StructureSettings::default(), &size_only()).is_empty());
    }

    /// US-011: a declaration is not a unit, however long it is written. An
    /// out-of-line module and a bodyless method both name code that lives
    /// somewhere else, and neither has anything to divide.
    #[test]
    fn a_declaration_without_a_body_is_never_a_unit() {
        let source = format!("mod carried;\n{}", "// filler\n".repeat(FILE_LINES));
        let observed = observe(&unit(&source), &StructureSettings::default(), &size_only());
        let keys: Vec<&str> = observed.iter().map(|(_, entry)| entry.key.as_str()).collect();
        assert_eq!(keys, ["file|src/lib.rs|src/lib.rs"], "{keys:?}");

        let documented = format!(
            "trait Wide {{\n{}    fn declared(&self);\n}}\n",
            "    /// One line of what it promises.\n".repeat(FUNCTION_LINES)
        );
        let observed = observe(
            &unit(&documented),
            &StructureSettings::default(),
            &size_only(),
        );
        let keys: Vec<&str> = observed.iter().map(|(_, entry)| entry.key.as_str()).collect();
        assert!(
            !keys.iter().any(|key| key.starts_with("fn|")),
            "a declaration with no body was reported as an oversized unit: {keys:?}"
        );
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

    /// A rule the policy left off costs its half of the walk nothing.
    #[test]
    fn an_inactive_half_observes_nothing() {
        let source = format!(
            "fn stretched() {{\n{}}}\n",
            "    if a { b(); }\n".repeat(FUNCTION_LINES)
        );
        let both = observe(
            &unit(&source),
            &StructureSettings::default(),
            &Active::of_rules(RULES),
        );
        assert!(both.iter().any(|(rule, _)| rule.id == STRUCTURE_OVERSIZED_UNIT.id));
        assert!(both.iter().any(|(rule, _)| rule.id == STRUCTURE_COMPLEX_FUNCTION.id));
        assert!(
            observe(&unit(&source), &StructureSettings::default(), &size_only())
                .iter()
                .all(|(rule, _)| rule.id == STRUCTURE_OVERSIZED_UNIT.id)
        );
        assert!(
            observe(&unit(&source), &StructureSettings::default(), &Active::default()).is_empty()
        );
    }
}
