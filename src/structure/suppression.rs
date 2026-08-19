//! Lints switched off rather than answered.
//!
//! Three rules over one kind of node, the attribute: an `#[allow]` with no
//! stated reason, an inner `#![allow]` whose scope is a whole file or a whole
//! inline module, and an item carrying several suppressions at once. They read
//! the attributes the unit's single traversal collected, all three from the same
//! list, because three walks of one tree would spend the pass's wall clock three
//! times to find the same nodes.
//!
//! `#[expect]` is left alone throughout: it fails once the lint stops firing, so
//! it expires by itself and needs no census. `#[cfg_attr(test, allow(...))]` is
//! out of reach throughout too, because the attribute it produces does not exist
//! in the syntax tree, and every rule's help states the limitation.

use std::collections::BTreeMap;

use ra_ap_syntax::ast::{self, HasName};
use ra_ap_syntax::{AstNode, SyntaxNode};

use super::{Observation, Unit, single_name, test_context};
use crate::policy::{ActiveRules, 
    RuleDefinition, STRUCTURE_CRATE_LEVEL_ALLOW, STRUCTURE_STACKED_ALLOW,
    STRUCTURE_UNREASONED_ALLOW,
};
use crate::report::DiagnosticContext;
use crate::source_text::SourceSpan;

/// The rules this half of the pass produces.
pub(super) const RULES: [&RuleDefinition; 3] = [
    &STRUCTURE_CRATE_LEVEL_ALLOW,
    &STRUCTURE_STACKED_ALLOW,
    &STRUCTURE_UNREASONED_ALLOW,
];

/// Every suppression finding of one unit.
pub(super) fn observe(
    unit: &Unit<'_>,
    active: &ActiveRules,
) -> Vec<(&'static RuleDefinition, Observation)> {
    let mut observations = Vec::new();
    if active.on(&STRUCTURE_UNREASONED_ALLOW) {
        observations.extend(
            unreasoned(unit)
                .into_iter()
                .map(|observation| (&STRUCTURE_UNREASONED_ALLOW, observation)),
        );
    }
    if active.on(&STRUCTURE_CRATE_LEVEL_ALLOW) {
        observations.extend(
            crate_level(unit)
                .into_iter()
                .map(|observation| (&STRUCTURE_CRATE_LEVEL_ALLOW, observation)),
        );
    }
    if active.on(&STRUCTURE_STACKED_ALLOW) {
        observations.extend(
            stacked(unit)
                .into_iter()
                .map(|observation| (&STRUCTURE_STACKED_ALLOW, observation)),
        );
    }
    observations
}

/// Every `#[allow]` that switches a lint off without saying why.
fn unreasoned(unit: &Unit<'_>) -> Vec<Observation> {
    let mut observations = Vec::new();
    for attribute in &unit.inventory.attributes {
        let Some(reading) = allow_arguments(attribute) else {
            continue;
        };
        if reading.has_reason || reading.lints.is_empty() {
            continue;
        }

        let inner = attribute.excl_token().is_some();
        let written = format!(
            "#{}[allow({})]",
            if inner { "!" } else { "" },
            reading.lints.join(", ")
        );
        observations.push(Observation {
            key: format!(
                "{}|{}",
                if inner { "inner" } else { "outer" },
                reading.lints.join(",")
            ),
            subject: format!("{written} switches a lint off without a stated reason."),
            span: unit.span(attribute.syntax()),
            context: test_context(attribute.syntax()).or(unit.context),
            complexity: None,
        });
    }
    observations
}

/// Every inner `#![allow(...)]` whose scope is a whole file or a whole inline
/// module.
///
/// The scope, not the justification, is what this rule judges: an attribute
/// carrying `reason = "..."` is reported all the same, and the help says so.
/// An outer `#[allow]` on a single item belongs to the stacked and unreasoned
/// detectors, and an inner attribute in any narrower position, a function body
/// for instance, exempts one item and is left to them too.
fn crate_level(unit: &Unit<'_>) -> Vec<Observation> {
    let mut observations = Vec::new();
    for attribute in &unit.inventory.attributes {
        if attribute.excl_token().is_none() {
            continue;
        }
        let Some(reading) = allow_arguments(attribute) else {
            continue;
        };
        if reading.lints.is_empty() {
            continue;
        }
        let Some(parent) = attribute.syntax().parent() else {
            continue;
        };

        let lints = reading.lints.join(", ");
        let (scope, subject) = if ast::SourceFile::cast(parent.clone()).is_some() {
            (
                "file".to_owned(),
                format!("#![allow({lints})] covers this whole file."),
            )
        } else if let Some(module) = ast::ItemList::cast(parent.clone())
            .and_then(|list| list.syntax().parent())
            .and_then(ast::Module::cast)
        {
            let name = module
                .name()
                .map(|name| name.text().to_string())
                .unwrap_or_default();
            (
                format!("module:{name}"),
                format!("#![allow({lints})] covers the whole module \"{name}\"."),
            )
        } else {
            continue;
        };

        observations.push(Observation {
            key: format!("{scope}|{}", reading.lints.join(",")),
            subject,
            span: unit.span(attribute.syntax()),
            context: test_context(attribute.syntax()).or(unit.context),
            complexity: None,
        });
    }
    observations
}

/// Accumulated allows on one item, before the stacked detector judges them.
#[derive(Debug)]
struct Stack {
    attributes: usize,
    /// Widest lint list a single attribute of the stack names.
    widest: usize,
    lints: Vec<String>,
    span: SourceSpan,
    context: Option<DiagnosticContext>,
}

/// An item carrying several suppressions at once: two or more separate
/// `#[allow(...)]` attributes, or one attribute naming four or more lints,
/// since one attribute listing four lints and four attributes are the same
/// act.
fn stacked(unit: &Unit<'_>) -> Vec<Observation> {
    let mut per_item = BTreeMap::<usize, Stack>::new();
    for attribute in &unit.inventory.attributes {
        if attribute.excl_token().is_some() {
            continue;
        }
        let Some(reading) = allow_arguments(attribute) else {
            continue;
        };
        if reading.lints.is_empty() {
            continue;
        }
        let Some(parent) = attribute.syntax().parent() else {
            continue;
        };

        let named = reading.lints.len();
        per_item
            .entry(usize::from(parent.text_range().start()))
            .and_modify(|stack| {
                stack.attributes += 1;
                stack.widest = stack.widest.max(named);
                stack.lints.extend(reading.lints.iter().cloned());
            })
            .or_insert_with(|| Stack {
                attributes: 1,
                widest: named,
                lints: reading.lints,
                span: unit.span(attribute.syntax()),
                context: test_context(attribute.syntax()).or(unit.context),
            });
    }

    per_item
        .into_values()
        .filter(|stack| stack.attributes >= 2 || stack.widest >= 4)
        .map(|mut stack| {
            stack.lints.sort_unstable();
            stack.lints.dedup();
            let subject = if stack.attributes >= 2 {
                format!(
                    "{} allow attributes stack {} suppressions on one item.",
                    stack.attributes,
                    stack.lints.len()
                )
            } else {
                format!(
                    "#[allow({})] stacks {} suppressions on one item.",
                    stack.lints.join(", "),
                    stack.lints.len()
                )
            };
            Observation {
                key: format!("{}|{}", stack.attributes, stack.lints.join(",")),
                subject,
                span: stack.span,
                context: stack.context,
                complexity: None,
            }
        })
        .collect()
}

/// What an `allow(...)` attribute names: its lints, sorted and deduplicated,
/// and whether a `reason = "..."` argument sits among them.
struct AllowReading {
    lints: Vec<String>,
    has_reason: bool,
}

/// Reads the attribute as an `allow`, or refuses it.
///
/// The name is read off the meta the grammar produced rather than off a token
/// position: only the `allow(...)` token-tree form is readable here, because
/// `#[expect]` carries another path and expires by itself, and
/// `#[cfg_attr(test, allow(...))]` is a meta of its own whose produced
/// attribute never exists in the tree.
fn allow_arguments(attribute: &ast::Attr) -> Option<AllowReading> {
    let Some(ast::Meta::TokenTreeMeta(meta)) = attribute.meta() else {
        return None;
    };
    if meta.path().as_ref().and_then(single_name).as_deref() != Some("allow") {
        return None;
    }
    let arguments = meta.token_tree().map(|tree| arguments(tree.syntax()))?;
    let has_reason = arguments.iter().any(|argument| is_reason(argument));
    let mut lints: Vec<String> = arguments
        .into_iter()
        .filter(|argument| !argument.is_empty() && !is_reason(argument))
        .collect();
    lints.sort_unstable();
    lints.dedup();
    Some(AllowReading { lints, has_reason })
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

#[cfg(test)]
mod tests {
    use super::*;

    /// What one rule observes on a snippet, in a unit carrying `context`.
    fn observed(
        rule: &'static RuleDefinition,
        source: &str,
        context: Option<DiagnosticContext>,
    ) -> Vec<Observation> {
        let mut unit = Unit::probe(source, "src/lib.rs");
        unit.context = context;
        observe(&unit, &ActiveRules::from_rules([rule]))
            .into_iter()
            .map(|(_, observation)| observation)
            .collect()
    }

    fn unreasoned_on(source: &str, context: Option<DiagnosticContext>) -> Vec<Observation> {
        observed(&STRUCTURE_UNREASONED_ALLOW, source, context)
    }

    /// US-001: only the inner attribute whose scope is a whole file or a whole
    /// inline module is this rule's subject, reasoned or not.
    #[test]
    fn a_crate_level_allow_is_reported_whatever_its_reason() {
        let file = observed(
            &STRUCTURE_CRATE_LEVEL_ALLOW,
            "#![allow(clippy::unwrap_used)]\nfn free() {}",
            None,
        );
        assert_eq!(file.len(), 1);
        assert_eq!(file[0].key, "file|clippy::unwrap_used");
        assert!(file[0].subject.contains("covers this whole file"), "{}", file[0].subject);
        assert!(file[0].subject.contains("clippy::unwrap_used"), "{}", file[0].subject);

        let reasoned = observed(
            &STRUCTURE_CRATE_LEVEL_ALLOW,
            "#![allow(dead_code, reason = \"scope is what is judged\")]\n",
            None,
        );
        assert_eq!(reasoned.len(), 1, "a stated reason does not narrow the scope");
        assert_eq!(reasoned[0].key, "file|dead_code");

        for quiet in [
            "#[allow(dead_code)]\nfn free() {}",
            "#![deny(dead_code)]\n",
            "#![warn(dead_code)]\n",
            "#![doc = \"a crate\"]\n",
            "#![expect(dead_code)]\n",
            "fn free() { #![allow(dead_code)] }",
            "#![allow()]\n",
        ] {
            assert!(
                observed(&STRUCTURE_CRATE_LEVEL_ALLOW, quiet, None).is_empty(),
                "{quiet}"
            );
        }
    }

    /// US-001: an inline module carrying an inner allow is reported with the
    /// module as its subject, and a `#[cfg(test)]` module carries the mark.
    #[test]
    fn a_module_level_allow_names_its_module() {
        let observations = observed(
            &STRUCTURE_CRATE_LEVEL_ALLOW,
            "mod imports {\n    #![allow(unused_imports)]\n    pub use std::fs;\n}",
            None,
        );
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].key, "module:imports|unused_imports");
        assert!(
            observations[0].subject.contains("module \"imports\""),
            "{}",
            observations[0].subject
        );
        assert_eq!(observations[0].context, None);

        let gated = observed(
            &STRUCTURE_CRATE_LEVEL_ALLOW,
            "#[cfg(test)]\nmod tests {\n    #![allow(dead_code)]\n    fn helper() {}\n}",
            None,
        );
        assert_eq!(gated.len(), 1);
        assert_eq!(gated[0].context, Some(DiagnosticContext::Tests));
    }

    /// US-001: the key carries the scope and the lint set, never a position or
    /// a path, so an insertion above the attribute moves nothing.
    #[test]
    fn the_crate_level_key_ignores_position() {
        let first = observed(&STRUCTURE_CRATE_LEVEL_ALLOW, "#![allow(b, a)]\n", None);
        let second = observed(&STRUCTURE_CRATE_LEVEL_ALLOW, "\n\n\n#![allow(a, b)]\n", None);
        assert_eq!(first.len(), 1);
        assert_eq!(second.len(), 1);
        assert_eq!(first[0].key, second[0].key);
        assert_ne!(first[0].span, second[0].span);
    }

    /// US-002: two separate attributes and one attribute naming four lints are
    /// the same act; narrower accumulations stay quiet.
    #[test]
    fn stacked_allows_are_reported_once_per_item() {
        let separate = observed(
            &STRUCTURE_STACKED_ALLOW,
            "#[allow(dead_code)]\n#[allow(unused_variables)]\nfn stacked() {}",
            None,
        );
        assert_eq!(separate.len(), 1);
        assert_eq!(separate[0].key, "2|dead_code,unused_variables");
        assert!(
            separate[0].subject.contains("2 allow attributes"),
            "{}",
            separate[0].subject
        );

        let wide = observed(
            &STRUCTURE_STACKED_ALLOW,
            "#[allow(a, b, c, d)]\nfn wide() {}",
            None,
        );
        assert_eq!(wide.len(), 1);
        assert_eq!(wide[0].key, "1|a,b,c,d");
        assert!(wide[0].subject.contains("4 suppressions"), "{}", wide[0].subject);

        for quiet in [
            "#[allow(dead_code)]\nfn narrow() {}",
            "#[allow(a, b, c)]\nfn three() {}",
            "#[allow(a, b, c, reason = \"still three lints\")]\nfn reasoned() {}",
            "#[cfg_attr(test, allow(a, b, c, d))]\nfn gated() {}",
            "#[allow(dead_code)]\nfn one() {}\n#[allow(dead_code)]\nfn other() {}",
        ] {
            assert!(
                observed(&STRUCTURE_STACKED_ALLOW, quiet, None).is_empty(),
                "{quiet}"
            );
        }
    }

    /// US-002: an item inside a test context carries the mark, so the stack is
    /// published without weighing.
    #[test]
    fn a_stack_inside_a_test_module_is_marked() {
        let observations = observed(
            &STRUCTURE_STACKED_ALLOW,
            "#[cfg(test)]\nmod tests {\n    #[allow(dead_code)]\n    #[allow(unused_variables)]\n    fn helper() {}\n}",
            None,
        );
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].context, Some(DiagnosticContext::Tests));
    }

    #[test]
    fn a_reason_and_an_expectation_are_both_left_alone() {
        assert_eq!(unreasoned_on("#[allow(dead_code)]\nfn free() {}", None).len(), 1);
        for quiet in [
            "#[allow(dead_code, reason = \"the surface is public\")]\nfn free() {}",
            "#[expect(dead_code)]\nfn free() {}",
            "#[cfg_attr(test, allow(dead_code))]\nfn free() {}",
            "#[deny(dead_code)]\nfn free() {}",
            "fn free() {}",
        ] {
            assert!(unreasoned_on(quiet, None).is_empty(), "{quiet}");
        }
    }

    /// The key carries the lint set and the attribute form, never a position,
    /// so the same exemption written twice forms one family.
    #[test]
    fn the_key_normalizes_order_and_ignores_position() {
        let first = unreasoned_on("#[allow(b, a)]\nfn one() {}", None);
        let second = unreasoned_on("\n\n\n#[allow(a, b)]\nfn two() {}", None);
        assert_eq!(first.len(), 1);
        assert_eq!(second.len(), 1);
        assert_eq!(first[0].key, second[0].key);
        assert_ne!(first[0].span, second[0].span);

        let inner = unreasoned_on("#![allow(a, b)]", None);
        assert_eq!(inner.len(), 1);
        assert_ne!(inner[0].key, first[0].key);
    }

    #[test]
    fn a_cfg_test_module_marks_its_own_allows() {
        let source = "#[allow(dead_code)]\nfn shipped() {}\n\
             #[cfg(test)]\nmod tests {\n    #[allow(dead_code)]\n    fn helper() {}\n}";
        let observations = unreasoned_on(source, None);
        assert_eq!(observations.len(), 2);
        assert_eq!(observations[0].context, None);
        assert_eq!(observations[1].context, Some(DiagnosticContext::Tests));
    }

    #[test]
    fn a_nested_group_stays_inside_one_argument() {
        let observations =
            unreasoned_on("#[allow(clippy::all, other(inner, nested))]\nfn free() {}", None);
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].key, "outer|clippy::all,other(inner,nested)");
    }

    /// A rule the policy left off produces nothing, and the three rules read
    /// the same attributes without reading them three times over.
    #[test]
    fn each_rule_answers_only_for_itself() {
        let source = "#![allow(dead_code)]\n#[allow(a)]\n#[allow(b)]\nfn stacked() {}";
        let unit = Unit::probe(source, "src/lib.rs");
        let all = observe(&unit, &ActiveRules::from_rules(RULES));
        let mut named: Vec<&str> = all.iter().map(|(rule, _)| rule.id).collect();
        named.sort_unstable();
        named.dedup();
        assert_eq!(named.len(), 3, "{named:?}");
        assert!(observe(&unit, &ActiveRules::default()).is_empty());
    }
}
