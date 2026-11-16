use std::collections::BTreeMap;

use ra_ap_syntax::ast::{self, AstToken};
use ra_ap_syntax::{AstNode, SyntaxNode};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EmbeddedSite {
    pub(crate) macro_name: &'static str,
    pub(crate) path: String,
    pub(crate) line: usize,
}

pub(super) fn embedded_variable(node: &SyntaxNode) -> Option<(&'static str, String)> {
    let call = ast::MacroCall::cast(node.clone())?;
    let macro_name = match call.path()?.segment()?.name_ref()?.text().as_str() {
        "env" => "env!",
        "option_env" => "option_env!",
        _ => return None,
    };
    let variable = call
        .token_tree()?
        .syntax()
        .children_with_tokens()
        .filter_map(|element| element.into_token())
        .find_map(ast::String::cast)?
        .value()
        .ok()?
        .into_owned();
    (!variable.is_empty()).then_some((macro_name, variable))
}

pub(super) fn record(embedded: &mut BTreeMap<String, EmbeddedSite>, variable: String, site: EmbeddedSite) {
    embedded.entry(variable).or_insert(site);
}

#[cfg(test)]
mod tests {
    use ra_ap_syntax::{Edition, SourceFile};

    use super::*;

    fn variables(source: &str) -> Vec<(&'static str, String)> {
        SourceFile::parse(source, Edition::Edition2024)
            .tree()
            .syntax()
            .descendants()
            .filter_map(|node| embedded_variable(&node))
            .collect()
    }

    #[test]
    fn a_string_literal_passed_to_env_or_option_env_names_an_embedded_variable() {
        assert_eq!(
            variables(
                "const KEY: Option<&str> = option_env!(\"POSTHOG_KEY\");\nconst HOME: &str = std::env!(\"CARGO_PKG_NAME\", \"set by cargo\");\nfn read() { let _ = std::env::var(\"RUNTIME\"); let _ = env!(concat!(\"A\", \"B\")); }\n"
            ),
            [
                ("option_env!", "POSTHOG_KEY".to_owned()),
                ("env!", "CARGO_PKG_NAME".to_owned()),
            ]
        );
    }

    #[test]
    fn the_first_site_of_a_variable_is_kept() {
        let mut embedded = BTreeMap::new();
        let site = |line| EmbeddedSite {
            macro_name: "env!",
            path: "src/lib.rs".to_owned(),
            line,
        };
        record(&mut embedded, "KEY".to_owned(), site(3));
        record(&mut embedded, "KEY".to_owned(), site(9));
        assert_eq!(embedded["KEY"].line, 3);
    }
}
