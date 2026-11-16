use std::collections::{BTreeMap, BTreeSet};

use ra_ap_syntax::ast::{self, HasName};
use ra_ap_syntax::{AstNode, Edition, NodeOrToken, SourceFile, SyntaxElement, SyntaxKind, SyntaxNode};

#[derive(Debug, Default)]
pub(super) struct DeclaringMacros {
    templates: BTreeMap<String, BTreeMap<String, BTreeSet<String>>>,
}

impl DeclaringMacros {
    pub(super) fn learn(&mut self, package_id: &str, tree: &SourceFile) -> Vec<String> {
        let mut learned = Vec::new();
        for (name, attributes) in definitions(tree) {
            let known = self
                .templates
                .entry(package_id.to_owned())
                .or_default()
                .entry(name.clone())
                .or_default();
            let before = known.len();
            known.extend(attributes);
            if known.len() > before {
                learned.push(name);
            }
        }
        learned
    }

    pub(super) fn declaration(&self, package_id: &str, macro_name: &str, module: &str) -> Option<ast::Module> {
        let templates = self.templates.get(package_id)?.get(macro_name)?;
        let attributes = match templates.len() {
            0 => return None,
            1 => templates.first().map(String::as_str).unwrap_or_default(),
            _ => "",
        };
        synthetic_module(attributes, module)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Invocation {
    pub(super) macro_name: String,
    pub(super) arguments: Vec<String>,
    pub(super) inline_modules: Vec<String>,
}

pub(super) fn invocations(tree: &SourceFile, error_ranges: &[ra_ap_syntax::TextRange]) -> Vec<Invocation> {
    tree.syntax()
        .descendants()
        .filter_map(ast::MacroCall::cast)
        .filter(|call| !crate::source_text::intersects_errors(call.syntax().text_range(), error_ranges))
        .filter_map(|call| {
            let macro_name = local_macro_name(&call.path()?)?;
            let arguments = argument_identifiers(call.token_tree()?.syntax());
            Some(Invocation {
                macro_name,
                arguments,
                inline_modules: inline_modules(call.syntax()),
            })
        })
        .collect()
}

pub(super) fn inline_modules(node: &SyntaxNode) -> Vec<String> {
    let mut inline: Vec<String> = node
        .ancestors()
        .skip(1)
        .filter_map(ast::Module::cast)
        .filter(|ancestor| ancestor.item_list().is_some())
        .filter_map(|ancestor| ancestor.name().map(|name| name.text().to_string()))
        .collect();
    inline.reverse();
    inline
}

fn local_macro_name(path: &ast::Path) -> Option<String> {
    let text: String = path
        .syntax()
        .text()
        .to_string()
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect();
    let name = text
        .strip_prefix("crate::")
        .or_else(|| text.strip_prefix("self::"))
        .unwrap_or(&text);
    (!name.is_empty() && !name.contains("::")).then(|| name.to_owned())
}

fn argument_identifiers(arguments: &SyntaxNode) -> Vec<String> {
    let mut identifiers = Vec::new();
    let mut seen = BTreeSet::new();
    let mut pending = vec![arguments.clone()];
    while let Some(tree) = pending.pop() {
        let elements: Vec<SyntaxElement> = significant_children(&tree);
        for (index, element) in elements.iter().enumerate() {
            match element {
                NodeOrToken::Node(child) => {
                    let attribute = index
                        .checked_sub(1)
                        .and_then(|previous| elements.get(previous))
                        .is_some_and(|previous| previous.kind() == SyntaxKind::POUND);
                    if !attribute {
                        pending.push(child.clone());
                    }
                }
                NodeOrToken::Token(token) if token.kind() == SyntaxKind::IDENT => {
                    let name = token.text().to_owned();
                    if seen.insert(name.clone()) {
                        identifiers.push(name);
                    }
                }
                NodeOrToken::Token(_) => {}
            }
        }
    }
    identifiers
}

fn significant_children(tree: &SyntaxNode) -> Vec<SyntaxElement> {
    tree.children_with_tokens()
        .filter(|element| !element.kind().is_trivia())
        .collect()
}

fn definitions(tree: &SourceFile) -> Vec<(String, BTreeSet<String>)> {
    tree.syntax()
        .descendants()
        .filter_map(ast::MacroRules::cast)
        .filter_map(|rules| {
            let name = rules.name()?.text().to_string();
            let body = rules.token_tree()?;
            let attributes = templates(body.syntax());
            (!attributes.is_empty()).then_some((name, attributes))
        })
        .collect()
}

fn templates(body: &SyntaxNode) -> BTreeSet<String> {
    let mut attributes = BTreeSet::new();
    for tree in body.descendants().filter(|node| node.kind() == SyntaxKind::TOKEN_TREE) {
        let elements = significant_children(&tree);
        for (index, element) in elements.iter().enumerate() {
            if element.kind() != SyntaxKind::MOD_KW {
                continue;
            }
            let following: Vec<SyntaxKind> = elements
                .iter()
                .skip(index + 1)
                .take(3)
                .map(SyntaxElement::kind)
                .collect();
            if following == [SyntaxKind::DOLLAR, SyntaxKind::IDENT, SyntaxKind::SEMICOLON] {
                attributes.insert(preceding_attributes(&elements[..index]));
            }
        }
    }
    attributes
}

fn preceding_attributes(before: &[SyntaxElement]) -> String {
    let mut attributes = Vec::new();
    let mut index = before.len();
    while index > 0 {
        let previous = &before[index - 1];
        let earlier = index.checked_sub(2).and_then(|earlier| before.get(earlier));
        match previous {
            NodeOrToken::Token(token) if token.kind() == SyntaxKind::PUB_KW => index -= 1,
            NodeOrToken::Token(token)
                if token.kind() == SyntaxKind::IDENT
                    && earlier.is_some_and(|earlier| earlier.kind() == SyntaxKind::DOLLAR) =>
            {
                index -= 2;
            }
            NodeOrToken::Node(group) if earlier.is_some_and(|earlier| earlier.kind() == SyntaxKind::PUB_KW) && group.text().to_string().starts_with('(') => {
                index -= 1;
            }
            NodeOrToken::Node(group)
                if earlier.is_some_and(|earlier| earlier.kind() == SyntaxKind::POUND)
                    && group.text().to_string().starts_with('[') =>
            {
                let text = group.text().to_string();
                if !text.contains('$') {
                    attributes.push(format!("#{text}"));
                }
                index -= 2;
            }
            _ => break,
        }
    }
    attributes.reverse();
    attributes.join("\n")
}

fn synthetic_module(attributes: &str, module: &str) -> Option<ast::Module> {
    let source = format!("{attributes}\nmod {module};\n");
    let parse = SourceFile::parse(&source, Edition::Edition2024);
    if !parse.errors().is_empty() {
        return None;
    }
    parse.tree().syntax().descendants().find_map(ast::Module::cast)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(source: &str) -> SourceFile {
        SourceFile::parse(source, Edition::Edition2024).tree()
    }

    #[test]
    fn a_transcriber_writing_mod_metavariable_semicolon_declares_modules() {
        let tree = parsed(
            "macro_rules! modules {\n    ($($module:ident),* $(,)?) => { $(pub mod $module;)* };\n}\nmacro_rules! inline { ($name:ident) => { mod $name {} }; }\nmacro_rules! matcher_only { (mod $name:ident;) => {}; }\n",
        );
        let found = definitions(&tree);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].0, "modules");
        assert_eq!(found[0].1, BTreeSet::from([String::new()]));
    }

    #[test]
    fn the_attributes_before_mod_travel_with_the_template() {
        let tree = parsed(
            "macro_rules! tests { ($($m:ident),*) => { $(#[cfg(test)] pub(crate) mod $m;)* }; }\nmacro_rules! vis { ($v:vis $m:ident) => { #[cfg(unix)] $v mod $m; }; }\n",
        );
        let found: BTreeMap<String, BTreeSet<String>> = definitions(&tree).into_iter().collect();
        assert_eq!(found["tests"], BTreeSet::from(["#[cfg(test)]".to_owned()]));
        assert_eq!(found["vis"], BTreeSet::from(["#[cfg(unix)]".to_owned()]));
    }

    #[test]
    fn an_invocation_by_bare_crate_or_self_path_is_local_and_a_foreign_path_is_not() {
        let tree = parsed(
            "modules!(alpha, beta);\ncrate::modules! { gamma }\nself::modules!(delta);\nother::modules!(epsilon);\nmod inner { modules!(#[cfg(test)] zeta); }\n",
        );
        let found = invocations(&tree, &[]);
        let named: Vec<(&str, Vec<&str>, Vec<&str>)> = found
            .iter()
            .map(|invocation| {
                (
                    invocation.macro_name.as_str(),
                    invocation.arguments.iter().map(String::as_str).collect(),
                    invocation.inline_modules.iter().map(String::as_str).collect(),
                )
            })
            .collect();
        assert_eq!(
            named,
            [
                ("modules", vec!["alpha", "beta"], vec![]),
                ("modules", vec!["gamma"], vec![]),
                ("modules", vec!["delta"], vec![]),
                ("modules", vec!["zeta"], vec!["inner"]),
            ]
        );
    }

    #[test]
    fn templates_that_disagree_on_attributes_declare_a_plain_module() {
        let mut macros = DeclaringMacros::default();
        let tree = parsed(
            "macro_rules! m { (test $x:ident) => { #[cfg(test)] mod $x; }; ($x:ident) => { mod $x; }; }\nmacro_rules! t { ($x:ident) => { #[cfg(test)] mod $x; }; }\n",
        );
        assert_eq!(macros.learn("p", &tree), ["m", "t"]);
        assert!(macros.learn("p", &tree).is_empty());
        let plain = macros.declaration("p", "m", "alpha").unwrap();
        assert_eq!(plain.syntax().text().to_string(), "mod alpha;");
        let gated = macros.declaration("p", "t", "alpha").unwrap();
        assert!(gated.syntax().text().to_string().starts_with("#[cfg(test)]"));
        assert!(macros.declaration("other", "t", "alpha").is_none());
    }
}
