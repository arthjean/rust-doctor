use std::collections::BTreeMap;

use ra_ap_syntax::ast::{self, HasName};
use ra_ap_syntax::{AstNode, SourceFile, SyntaxKind, SyntaxNode, TextRange};

use crate::source_text::intersects_errors;

pub(super) const BINDING_LIMIT: usize = 1_024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Provenance<'a> {
    Item(&'a [String]),
    GlobMasked(Vec<Option<&'a str>>),
    Indeterminate,
    Unbound,
}

const BINDING_KINDS: [SyntaxKind; 8] = [
    SyntaxKind::USE,
    SyntaxKind::MODULE,
    SyntaxKind::EXTERN_CRATE,
    SyntaxKind::STRUCT,
    SyntaxKind::ENUM,
    SyntaxKind::UNION,
    SyntaxKind::TRAIT,
    SyntaxKind::TYPE_ALIAS,
];

type ScopeKey = (u32, u32);

#[derive(Debug, Clone, PartialEq, Eq)]
enum Binding {
    Item(Vec<String>),
    Opaque,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment {
    Name(String),
    SelfKw,
    Opaque,
}

#[derive(Debug, Default)]
pub(super) struct AliasMap {
    scopes: BTreeMap<ScopeKey, BTreeMap<String, Binding>>,
    globs: BTreeMap<ScopeKey, Vec<Option<String>>>,
    bindings: usize,
    saturated: bool,
}

impl AliasMap {
    pub(super) fn build(tree: &SourceFile, error_ranges: &[TextRange], limit: usize) -> Self {
        let mut map = Self::default();
        for node in tree
            .syntax()
            .descendants()
            .filter(|node| BINDING_KINDS.contains(&node.kind()))
        {
            if map.saturated {
                break;
            }
            let Some(scope) = binding_scope(&node) else {
                continue;
            };
            if let Some(item) = ast::Use::cast(node.clone()) {
                if let Some(tree) = item.use_tree() {
                    map.record_tree(&tree, Some(Vec::new()), scope, error_ranges, limit);
                }
                continue;
            }
            if let Some(name) = shadowing_name(&node) {
                map.insert(scope, name, Binding::Opaque, limit);
            }
        }
        map
    }

    pub(super) fn provenance(&self, node: &SyntaxNode, name: &str) -> Provenance<'_> {
        if self.saturated || shadowed_by_generic_parameter(node, name) {
            return Provenance::Indeterminate;
        }
        let mut masking = Vec::new();
        for scope in visible_scopes(node) {
            if let Some(binding) = self.scopes.get(&scope).and_then(|scope| scope.get(name)) {
                if !masking.is_empty() {
                    return Provenance::Indeterminate;
                }
                return match binding {
                    Binding::Item(path) => Provenance::Item(path),
                    Binding::Opaque => Provenance::Indeterminate,
                };
            }
            if let Some(globs) = self.globs.get(&scope) {
                masking.extend(globs.iter().map(Option::as_deref));
            }
        }
        if masking.is_empty() {
            Provenance::Unbound
        } else {
            Provenance::GlobMasked(masking)
        }
    }

    pub(super) fn globbed(&self, node: &SyntaxNode) -> bool {
        self.saturated || self.globs.contains_key(&key(node))
    }

    pub(super) const fn saturated(&self) -> bool {
        self.saturated
    }

    fn record_tree(
        &mut self,
        tree: &ast::UseTree,
        prefix: Option<Vec<String>>,
        scope: ScopeKey,
        error_ranges: &[TextRange],
        limit: usize,
    ) {
        if self.saturated || intersects_errors(tree.syntax().text_range(), error_ranges) {
            return;
        }
        let written = tree.path().map(|path| segments(&path)).unwrap_or_default();
        let relative_self = matches!(written.as_slice(), [Segment::SelfKw]);
        let resolved = if relative_self {
            prefix.clone()
        } else {
            extend(prefix.clone(), &written)
        };

        if tree.star_token().is_some() {
            let head = resolved.and_then(|path| path.first().cloned());
            self.insert_glob(scope, head, limit);
            return;
        }
        if let Some(list) = tree.use_tree_list() {
            for child in list.use_trees() {
                self.record_tree(&child, resolved.clone(), scope, error_ranges, limit);
            }
            return;
        }

        let name = match tree.rename() {
            Some(rename) => rename.name().map(|name| name.text().to_string()),
            None if relative_self => prefix.and_then(|prefix| prefix.last().cloned()),
            None => match written.last() {
                Some(Segment::Name(name)) => Some(name.clone()),
                _ => None,
            },
        };
        let Some(name) = name else {
            return;
        };
        let binding = match resolved {
            Some(path) if !path.is_empty() => Binding::Item(path),
            _ => Binding::Opaque,
        };
        self.insert(scope, name, binding, limit);
    }

    fn insert(&mut self, scope: ScopeKey, name: String, binding: Binding, limit: usize) {
        if self.reserve(limit) {
            self.scopes.entry(scope).or_default().insert(name, binding);
        }
    }

    fn insert_glob(&mut self, scope: ScopeKey, head: Option<String>, limit: usize) {
        if self.reserve(limit) {
            self.globs.entry(scope).or_default().push(head);
        }
    }

    fn reserve(&mut self, limit: usize) -> bool {
        if self.bindings >= limit {
            self.saturated = true;
            return false;
        }
        self.bindings += 1;
        true
    }
}

fn extend(prefix: Option<Vec<String>>, written: &[Segment]) -> Option<Vec<String>> {
    let mut prefix = prefix?;
    for segment in written {
        match segment {
            Segment::Name(name) => prefix.push(name.clone()),
            Segment::SelfKw | Segment::Opaque => return None,
        }
    }
    Some(prefix)
}

fn segments(path: &ast::Path) -> Vec<Segment> {
    path.segments()
        .map(|segment| match segment.kind() {
            Some(ast::PathSegmentKind::Name(name)) if plain(&segment) => {
                Segment::Name(name.text().to_string())
            }
            Some(ast::PathSegmentKind::SelfKw) => Segment::SelfKw,
            _ => Segment::Opaque,
        })
        .collect()
}

fn plain(segment: &ast::PathSegment) -> bool {
    let mut children = segment.syntax().children();
    children.next();
    children.next().is_none()
}

fn shadowing_name(node: &SyntaxNode) -> Option<String> {
    if let Some(item) = ast::ExternCrate::cast(node.clone()) {
        return match item.rename() {
            Some(rename) => rename.name().map(|name| name.text().to_string()),
            None => item.name_ref().map(|name| name.text().to_string()),
        };
    }
    ast::AnyHasName::cast(node.clone())?
        .name()
        .map(|name| name.text().to_string())
}

fn shadowed_by_generic_parameter(node: &SyntaxNode, name: &str) -> bool {
    node.ancestors().any(|ancestor| {
        ancestor
            .children()
            .filter_map(ast::GenericParamList::cast)
            .flat_map(|parameters| parameters.generic_params())
            .any(|parameter| {
                matches!(
                    parameter,
                    ast::GenericParam::TypeParam(parameter)
                        if parameter.name().is_some_and(|parameter| parameter.text() == name)
                )
            })
    })
}

fn binding_scope(node: &SyntaxNode) -> Option<ScopeKey> {
    node.parent()?
        .ancestors()
        .find(is_scope)
        .map(|scope| key(&scope))
}

fn visible_scopes(node: &SyntaxNode) -> Vec<ScopeKey> {
    let mut scopes = Vec::new();
    for ancestor in node.ancestors() {
        if ast::BlockExpr::cast(ancestor.clone()).is_some() {
            scopes.push(key(&ancestor));
            continue;
        }
        if is_module_scope(&ancestor) {
            scopes.push(key(&ancestor));
            break;
        }
    }
    scopes
}

fn is_scope(node: &SyntaxNode) -> bool {
    ast::BlockExpr::cast(node.clone()).is_some() || is_module_scope(node)
}

fn is_module_scope(node: &SyntaxNode) -> bool {
    ast::SourceFile::cast(node.clone()).is_some()
        || ast::Module::cast(node.clone()).is_some_and(|module| module.item_list().is_some())
}

fn key(node: &SyntaxNode) -> ScopeKey {
    let range = node.text_range();
    (range.start().into(), range.end().into())
}

#[cfg(test)]
mod tests {
    use ra_ap_syntax::Edition;

    use super::*;

    fn map(source: &str) -> (AliasMap, ra_ap_syntax::Parse<SourceFile>) {
        let parse = SourceFile::parse(source, Edition::Edition2024);
        let ranges: Vec<_> = parse.errors().iter().map(|error| error.range()).collect();
        let map = AliasMap::build(&parse.tree(), &ranges, BINDING_LIMIT);
        (map, parse)
    }

    fn provenance(source: &str, name: &str) -> Vec<String> {
        let (map, parse) = map(source);
        let call = parse
            .tree()
            .syntax()
            .descendants()
            .find_map(ast::MethodCallExpr::cast)
            .expect("the probe expects one method call");
        match map.provenance(call.syntax(), name) {
            Provenance::Item(path) => path.to_vec(),
            Provenance::GlobMasked(heads) => heads
                .iter()
                .map(|head| format!("<glob {}>", head.unwrap_or("relative")))
                .collect(),
            Provenance::Indeterminate => vec!["<indeterminate>".to_owned()],
            Provenance::Unbound => vec!["<unbound>".to_owned()],
        }
    }

    fn indeterminate() -> Vec<String> {
        vec!["<indeterminate>".to_owned()]
    }

    fn unbound() -> Vec<String> {
        vec!["<unbound>".to_owned()]
    }

    fn item(path: &[&str]) -> Vec<String> {
        path.iter().map(|segment| (*segment).to_owned()).collect()
    }

    #[test]
    fn imports_resolve_through_renames_nested_groups_and_relative_self() {
        assert_eq!(
            provenance(
                "use std::process::Command; fn main() { Command::new(\"sh\").arg(\"-c\"); }",
                "Command",
            ),
            item(&["std", "process", "Command"])
        );
        assert_eq!(
            provenance(
                "use std::process::Command as Cmd; fn main() { Cmd::new(\"sh\").arg(\"-c\"); }",
                "Cmd",
            ),
            item(&["std", "process", "Command"])
        );

        let nested = "use std::{process::Command, io::Write}; fn main() { Command::new(\"sh\").arg(\"-c\"); }";
        assert_eq!(
            provenance(nested, "Command"),
            item(&["std", "process", "Command"])
        );
        assert_eq!(provenance(nested, "Write"), item(&["std", "io", "Write"]));

        assert_eq!(
            provenance(
                "use std::process::{self}; fn main() { process::Command::new(\"sh\").arg(\"-c\"); }",
                "process",
            ),
            item(&["std", "process"])
        );
    }

    #[test]
    fn globs_underscores_and_relative_keywords_never_produce_an_identifier() {
        assert_eq!(
            provenance(
                "use std::process::*; fn main() { Command::new(\"sh\").arg(\"-c\"); }",
                "Command",
            ),
            vec!["<glob std>".to_owned()]
        );
        assert_eq!(
            provenance(
                "use std::process::*; fn main() { Command::new(\"sh\").arg(\"-c\"); }",
                "Absent",
            ),
            vec!["<glob std>".to_owned()]
        );
        assert_eq!(
            provenance(
                "use crate::prelude::*; use super::*; fn main() { Command::new(\"sh\").arg(\"-c\"); }",
                "Command",
            ),
            vec!["<glob relative>".to_owned(), "<glob relative>".to_owned()]
        );
        assert_eq!(
            provenance(
                "use std::process::Command as _; fn main() { local().arg(\"-c\"); }",
                "Command",
            ),
            unbound()
        );
        assert_eq!(
            provenance(
                "use crate::process::Command; fn main() { Command::new(\"sh\").arg(\"-c\"); }",
                "Command",
            ),
            indeterminate()
        );
        assert_eq!(
            provenance(
                "use super::Command; fn main() { Command::new(\"sh\").arg(\"-c\"); }",
                "Command",
            ),
            indeterminate()
        );
    }

    #[test]
    fn an_explicit_import_wins_over_a_glob_of_its_own_scope_and_loses_to_a_nearer_one() {
        assert_eq!(
            provenance(
                "use futures::prelude::*; use std::process::Command; fn main() { Command::new(\"sh\").arg(\"-c\"); }",
                "Command",
            ),
            item(&["std", "process", "Command"])
        );
        assert_eq!(
            provenance(
                "use std::process::Command; fn main() { use futures::prelude::*; Command::new(\"sh\").arg(\"-c\"); }",
                "Command",
            ),
            indeterminate()
        );
    }

    #[test]
    fn local_declarations_shadow_an_import_only_where_they_are_visible() {
        assert_eq!(
            provenance(
                "use std::process::Command; struct Command; fn main() { Command::new(\"sh\").arg(\"-c\"); }",
                "Command",
            ),
            indeterminate()
        );
        assert_eq!(
            provenance(
                "use std::process::Command; fn main() { struct Command; Command::new(\"sh\").arg(\"-c\"); }",
                "Command",
            ),
            indeterminate()
        );
        assert_eq!(
            provenance(
                "use std::process::Command; fn sibling() { struct Command; } fn main() { Command::new(\"sh\").arg(\"-c\"); }",
                "Command",
            ),
            item(&["std", "process", "Command"])
        );
        assert_eq!(
            provenance(
                "use std::process::Command; mod inner { pub fn main() { Command::new(\"sh\").arg(\"-c\"); } }",
                "Command",
            ),
            unbound()
        );
        assert_eq!(
            provenance(
                "use std::process::Command; fn main<Command>() { Command::new(\"sh\").arg(\"-c\"); }",
                "Command",
            ),
            indeterminate()
        );
        assert_eq!(
            provenance(
                "use std::process::Command; mod Command {} fn main() { Command::new(\"sh\").arg(\"-c\"); }",
                "Command",
            ),
            indeterminate()
        );
    }

    #[test]
    fn a_use_tree_covered_by_a_parse_error_is_ignored_without_failing_the_build() {
        let source = "use std::process::Command; use std::io::{Write; fn main() { Command::new(\"sh\").arg(\"-c\"); }";
        let (_, parse) = map(source);
        assert!(!parse.errors().is_empty());
        assert_eq!(
            provenance(source, "Command"),
            item(&["std", "process", "Command"])
        );
        assert_eq!(provenance(source, "Write"), unbound());
    }

    fn unit_of(items: usize) -> String {
        let mut source = String::new();
        for index in 0..items {
            source.push_str(&format!("use std::process::Command as Alias{index};\n"));
            source.push_str(&format!("pub struct Type{index};\n"));
            source.push_str(&format!(
                "pub fn call{index}(user: &str) {{\n    let _ = Command::new(\"sh\").arg(user);\n}}\n"
            ));
        }
        source
    }

    fn fastest_builds(small: &str, large: &str) -> (std::time::Duration, std::time::Duration) {
        let small = SourceFile::parse(small, Edition::Edition2024);
        let large = SourceFile::parse(large, Edition::Edition2024);
        assert!(small.errors().is_empty() && large.errors().is_empty());
        let build = |parse: &ra_ap_syntax::Parse<SourceFile>| {
            let started = std::time::Instant::now();
            let map = AliasMap::build(&parse.tree(), &[], BINDING_LIMIT);
            let elapsed = started.elapsed();
            assert!(!map.saturated());
            elapsed
        };
        let mut fastest = (std::time::Duration::MAX, std::time::Duration::MAX);
        for _ in 0..20 {
            fastest.0 = fastest.0.min(build(&small));
            fastest.1 = fastest.1.min(build(&large));
        }
        fastest
    }

    #[test]
    fn building_a_unit_costs_linearly_in_its_size() {
        let small = unit_of(20);
        let large = unit_of(200);
        assert_eq!(small.lines().count(), 100);
        assert_eq!(large.lines().count(), 1_000);

        let attempts: Vec<_> = (0..5)
            .map(|_| fastest_builds(&small, &large))
            .collect();
        assert!(
            attempts.iter().any(|(small, large)| *large <= *small * 20),
            "one thousand lines against one hundred, in every attempt: {attempts:?}"
        );
    }

    #[test]
    fn crossing_the_binding_limit_keeps_the_map_usable_in_indeterminate_mode() {
        let mut source = String::new();
        for index in 0..8 {
            source.push_str(&format!("use std::process::Command as Alias{index};\n"));
        }
        source.push_str("fn main() { Command::new(\"sh\").arg(\"-c\"); }");
        let parse = SourceFile::parse(&source, Edition::Edition2024);
        let saturated = AliasMap::build(&parse.tree(), &[], 4);
        let bounded = AliasMap::build(&parse.tree(), &[], BINDING_LIMIT);
        let call = parse
            .tree()
            .syntax()
            .descendants()
            .find_map(ast::MethodCallExpr::cast)
            .unwrap();

        assert!(saturated.saturated());
        assert_eq!(
            saturated.provenance(call.syntax(), "Alias0"),
            Provenance::Indeterminate
        );
        assert!(!bounded.saturated());
        assert!(matches!(
            bounded.provenance(call.syntax(), "Alias7"),
            Provenance::Item(_)
        ));
    }
}
