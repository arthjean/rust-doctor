//! Carte d'alias d'imports d'une unité source.
//!
//! Elle répond à une seule question: à ce point du fichier, quel item désigne
//! cet identifiant ? La réponse est fermée. Quand la provenance n'est pas
//! décidable, ombrage local, import glob, mot-clé de chemin relatif ou carte
//! saturée, la carte le dit et le détecteur s'abstient. Les ré-exports, les
//! méthodes de trait et `Self` restent hors de portée: ils demanderaient une
//! résolution sémantique complète, explicitement hors périmètre.

use std::collections::{BTreeMap, BTreeSet};

use ra_ap_syntax::ast::{self, HasName};
use ra_ap_syntax::{AstNode, SourceFile, SyntaxKind, SyntaxNode, TextRange};

use super::intersects_errors;

/// Nombre maximal de liaisons retenues par unité source.
///
/// Chaque entrée coûte quelques dizaines d'octets, donc la limite borne la
/// carte sous les 64 Ko par unité. Au-delà, la carte bascule en mode
/// indéterminé: elle reste interrogeable et toute résolution s'abstient.
pub(super) const BINDING_LIMIT: usize = 1_024;

/// Provenance d'un identifiant à un point donné du fichier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Provenance<'a> {
    /// L'identifiant est lié par un `use` visible et désigne cet item.
    Item(&'a [String]),
    /// L'identifiant est lié localement, provient d'un glob, ou la carte est
    /// saturée: la provenance n'est pas décidable.
    Indeterminate,
    /// Aucune liaison visible ne porte cet identifiant.
    Unbound,
}

/// Nœuds susceptibles de lier un identifiant dans leur portée.
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

/// Une portée est identifiée par les bornes de son nœud. Deux portées
/// imbriquées ont des bornes distinctes, donc la clé est stable et ordonnable
/// sans dépendre de l'identité d'un nœud rowan.
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
    globs: BTreeSet<ScopeKey>,
    bindings: usize,
    saturated: bool,
}

impl AliasMap {
    pub(super) fn build(tree: &SourceFile, error_ranges: &[TextRange], limit: usize) -> Self {
        let mut map = Self::default();
        // Le filtre porte sur le type de nœud avant toute construction typée:
        // la grande majorité des nœuds d'une unité ne lie aucun identifiant.
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

    /// Provenance de `name` au point du fichier occupé par `node`.
    pub(super) fn provenance(&self, node: &SyntaxNode, name: &str) -> Provenance<'_> {
        if self.saturated || shadowed_by_generic_parameter(node, name) {
            return Provenance::Indeterminate;
        }
        for scope in visible_scopes(node) {
            if let Some(binding) = self.scopes.get(&scope).and_then(|scope| scope.get(name)) {
                return match binding {
                    Binding::Item(path) => Provenance::Item(path),
                    Binding::Opaque => Provenance::Indeterminate,
                };
            }
            // Un glob de la portée courante peut lier le nom sans que la carte
            // sache lequel: il masque les portées englobantes plutôt que de les
            // laisser répondre à sa place.
            if self.globs.contains(&scope) {
                return Provenance::Indeterminate;
            }
        }
        Provenance::Unbound
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
            self.insert_glob(scope, limit);
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

    fn insert_glob(&mut self, scope: ScopeKey, limit: usize) {
        if self.reserve(limit) {
            self.globs.insert(scope);
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

/// Un segment porteur d'arguments génériques ou d'un ancrage de type ne nomme
/// pas seul un item: la carte refuse de le résoudre.
fn plain(segment: &ast::PathSegment) -> bool {
    let mut children = segment.syntax().children();
    children.next();
    children.next().is_none()
}

fn shadowing_name(node: &SyntaxNode) -> Option<String> {
    if let Some(item) = ast::Module::cast(node.clone()) {
        return item.name().map(|name| name.text().to_string());
    }
    if let Some(item) = ast::ExternCrate::cast(node.clone()) {
        return match item.rename() {
            Some(rename) => rename.name().map(|name| name.text().to_string()),
            None => item.name_ref().map(|name| name.text().to_string()),
        };
    }
    if let Some(item) = ast::Struct::cast(node.clone()) {
        return item.name().map(|name| name.text().to_string());
    }
    if let Some(item) = ast::Enum::cast(node.clone()) {
        return item.name().map(|name| name.text().to_string());
    }
    if let Some(item) = ast::Union::cast(node.clone()) {
        return item.name().map(|name| name.text().to_string());
    }
    if let Some(item) = ast::Trait::cast(node.clone()) {
        return item.name().map(|name| name.text().to_string());
    }
    ast::TypeAlias::cast(node.clone())
        .and_then(|item| item.name().map(|name| name.text().to_string()))
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

/// Portée dans laquelle une liaison déclarée par `node` est visible.
fn binding_scope(node: &SyntaxNode) -> Option<ScopeKey> {
    node.parent()?
        .ancestors()
        .find(is_scope)
        .map(|scope| key(&scope))
}

/// Portées visibles depuis `node`, de la plus proche jusqu'au module englobant
/// inclus. La chaîne s'arrête là: un `use` d'un module parent ne traverse pas
/// la frontière d'un module enfant.
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

    /// Résout `name` au point du premier appel de méthode du fichier, qui sert
    /// de site d'interrogation à toutes les preuves.
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
            indeterminate()
        );
        assert_eq!(
            provenance(
                "use std::process::*; fn main() { Command::new(\"sh\").arg(\"-c\"); }",
                "Absent",
            ),
            indeterminate()
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

    /// NFR: la construction reste sous 2 ms par fichier de 1 000 lignes.
    #[test]
    fn building_a_thousand_line_unit_stays_under_two_milliseconds() {
        use std::time::{Duration, Instant};

        let mut source = String::new();
        for index in 0..200 {
            source.push_str(&format!("use std::process::Command as Alias{index};\n"));
            source.push_str(&format!("pub struct Type{index};\n"));
            source.push_str(&format!(
                "pub fn call{index}(user: &str) {{\n    let _ = Command::new(\"sh\").arg(user);\n}}\n"
            ));
        }
        assert_eq!(source.lines().count(), 1_000);
        let parse = SourceFile::parse(&source, Edition::Edition2024);
        assert!(parse.errors().is_empty());

        let mut samples = Vec::new();
        for _ in 0..20 {
            let started = Instant::now();
            let map = AliasMap::build(&parse.tree(), &[], BINDING_LIMIT);
            samples.push(started.elapsed());
            assert!(!map.saturated());
        }
        samples.sort_unstable();
        let p95 = samples[18];
        // Le budget porte sur le binaire livré. Un build de test non optimisé
        // paie le coût d'accès au CST, donc la borne y est desserrée sans
        // cesser de mesurer la même construction.
        let budget = if cfg!(debug_assertions) {
            Duration::from_millis(20)
        } else {
            Duration::from_millis(2)
        };
        assert!(p95 < budget, "p95 was {p95:?}");
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
