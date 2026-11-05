use ra_ap_syntax::ast::{self, HasName};
use ra_ap_syntax::{AstNode, SyntaxKind, SyntaxNode};

use super::{Observation, StructureSettings, Unit, test_context};
use crate::policy::{ActiveRules, RuleDefinition, STRUCTURE_COMPLEX_FUNCTION, STRUCTURE_OVERSIZED_UNIT};
use crate::report::ComplexityFigures;
use crate::source_text::compact;

pub(super) const RULES: [&RuleDefinition; 2] =
    [&STRUCTURE_COMPLEX_FUNCTION, &STRUCTURE_OVERSIZED_UNIT];

pub const FILE_LINES: usize = 1_000;

pub(super) const MODULE_LINES: usize = 500;

pub(super) const IMPL_LINES: usize = 500;

pub(super) const FUNCTION_LINES: usize = 150;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct Metrics {
    pub(super) cyclomatic: u32,
    pub(super) cognitive: u32,
}

pub(super) fn observe(
    unit: &Unit<'_>,
    settings: &StructureSettings,
    active: &ActiveRules,
) -> Vec<(&'static RuleDefinition, Observation)> {
    let mut observations = Vec::new();
    if active.on(&STRUCTURE_OVERSIZED_UNIT) {
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

const fn bound(kind: SyntaxKind) -> Option<(&'static str, usize)> {
    match kind {
        SyntaxKind::SOURCE_FILE => Some(("file", FILE_LINES)),
        SyntaxKind::FN => Some(("fn", FUNCTION_LINES)),
        SyntaxKind::IMPL => Some(("impl", IMPL_LINES)),
        SyntaxKind::MODULE => Some(("mod", MODULE_LINES)),
        _ => None,
    }
}

fn oversized(unit: &Unit<'_>, node: &SyntaxNode) -> Option<Observation> {
    let (tag, limit) = bound(node.kind())?;
    let lines = lines_of(unit, node);
    if lines < limit {
        return None;
    }
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

fn lines_of(unit: &Unit<'_>, node: &SyntaxNode) -> usize {
    let range = node.text_range();
    let start = match node.kind() {
        SyntaxKind::SOURCE_FILE => range.start(),
        _ => node
            .children_with_tokens()
            .find(|child| {
                !matches!(
                    child.kind(),
                    SyntaxKind::ATTR | SyntaxKind::COMMENT | SyntaxKind::WHITESPACE
                )
            })
            .map_or(range.start(), |child| child.text_range().start()),
    };
    let first = usize::from(start);
    let last = usize::from(range.end()).saturating_sub(1).max(first);
    let start = unit.line_starts.partition_point(|offset| *offset <= first);
    let end = unit.line_starts.partition_point(|offset| *offset <= last);
    end.saturating_sub(start).saturating_add(1)
}

pub(super) fn metrics_of(body: &ast::BlockExpr) -> Metrics {
    let mut metrics = Metrics {
        cyclomatic: 1,
        cognitive: 0,
    };
    measure(body.syntax(), 0, &mut metrics);
    metrics
}

fn measure(node: &SyntaxNode, nesting: u32, metrics: &mut Metrics) {
    for child in node.children() {
        match child.kind() {
            SyntaxKind::FN => {}
            SyntaxKind::IF_EXPR => {
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

    fn complexity_only() -> ActiveRules {
        ActiveRules::from_rules([&STRUCTURE_COMPLEX_FUNCTION])
    }

    fn size_only() -> ActiveRules {
        ActiveRules::from_rules([&STRUCTURE_OVERSIZED_UNIT])
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

    #[test]
    fn cyclomatic_complexity_is_the_branch_points_plus_one() {
        assert_eq!(measured("let value = 1;").cyclomatic, 1);
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

    #[test]
    fn equal_cyclomatic_complexity_still_separates_on_nesting() {
        let sequential = measured("if a { b(); }\nif c { d(); }\nif e { f(); }");
        let nested = measured("if a { if c { if e { f(); } } }");
        assert_eq!(sequential.cyclomatic, nested.cyclomatic);
        assert_eq!(sequential.cognitive, 3);
        assert_eq!(nested.cognitive, 6);
    }

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

    #[test]
    fn a_nested_function_is_its_own_measure_and_a_closure_is_not() {
        let with_nested = measured("fn inner() { if a { b(); } }\nc();");
        assert_eq!(with_nested.cyclomatic, 1);
        assert_eq!(with_nested.cognitive, 0);

        let with_closure = measured("let probe = |x| if x { 1 } else { 2 };");
        assert_eq!(with_closure.cyclomatic, 2);
        assert_eq!(with_closure.cognitive, 3);
    }

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

        let calm = format!(
            "fn stretched() {{\n{}}}\n",
            "    call();\n".repeat(FUNCTION_LINES - 3)
        );
        assert!(observe(&unit(&calm), &StructureSettings::default(), &size_only()).is_empty());
    }

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

    #[test]
    fn a_file_is_counted_at_its_last_line_and_never_past_it() {
        let terminated = unit("fn a() {}\nfn b() {}\n");
        assert_eq!(lines_of(&terminated, terminated.tree.syntax()), 2);

        let unterminated = unit("fn a() {}\nfn b() {}");
        assert_eq!(lines_of(&unterminated, unterminated.tree.syntax()), 2);

        let empty = unit("");
        assert_eq!(lines_of(&empty, empty.tree.syntax()), 1);
    }

    #[test]
    fn a_unit_is_counted_from_its_keyword() {
        let documented = |lines: usize| {
            format!(
                "/// What it does.\n///\n/// How it does it.\n///\n/// Why.\n#[inline]\nfn stretched() {{\n{}}}\n",
                "    call();\n".repeat(lines - 2)
            )
        };
        let under = documented(FUNCTION_LINES - 1);
        let observed = observe(&unit(&under), &StructureSettings::default(), &size_only());
        let subjects: Vec<&String> = observed.iter().map(|(_, entry)| &entry.subject).collect();
        assert!(observed.is_empty(), "{subjects:?}");

        let at = documented(FUNCTION_LINES);
        let observed = observe(&unit(&at), &StructureSettings::default(), &size_only());
        assert_eq!(observed.len(), 1);
        assert_eq!(
            observed[0].1.subject,
            format!("Function stretched spans {FUNCTION_LINES} lines.")
        );
        assert_eq!(observed[0].1.span.line_start, 1, "the span still covers the doc comment");

        let file = "// line\n".repeat(FILE_LINES);
        let observed = observe(&unit(&file), &StructureSettings::default(), &size_only());
        assert_eq!(observed.len(), 1);
        assert_eq!(
            observed[0].1.subject,
            format!("src/lib.rs is {FILE_LINES} lines long.")
        );
    }

    #[test]
    fn an_inactive_half_observes_nothing() {
        let source = format!(
            "fn stretched() {{\n{}}}\n",
            "    if a { b(); }\n".repeat(FUNCTION_LINES)
        );
        let both = observe(
            &unit(&source),
            &StructureSettings::default(),
            &ActiveRules::from_rules(RULES),
        );
        assert!(both.iter().any(|(rule, _)| rule.id == STRUCTURE_OVERSIZED_UNIT.id));
        assert!(both.iter().any(|(rule, _)| rule.id == STRUCTURE_COMPLEX_FUNCTION.id));
        assert!(
            observe(&unit(&source), &StructureSettings::default(), &size_only())
                .iter()
                .all(|(rule, _)| rule.id == STRUCTURE_OVERSIZED_UNIT.id)
        );
        assert!(
            observe(&unit(&source), &StructureSettings::default(), &ActiveRules::default()).is_empty()
        );
    }
}
