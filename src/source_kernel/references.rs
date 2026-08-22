//! Coarse crate-reference collector over the enumeration.
//!
//! The dependency-truth rules judge a manifest entry against evidence: the
//! set of crate names each package's sources actually reference. The
//! collection is syntactic and deliberately coarse, in the cargo-shear
//! posture: it reads the first segment of written paths, `extern crate`
//! declarations and macro invocation paths, and never resolves a name. A
//! local module shadowing a dependency name therefore credits the entry, and
//! that direction of error silences a finding instead of publishing a wrong
//! one.
//!
//! Inside a macro invocation, arguments are token trees rather than parsed
//! paths, and test code lives there almost entirely (`assert_eq!` and its
//! siblings). An identifier followed by `::` in a token tree is therefore
//! read as a crate-qualifying reference too, with the same classification as
//! a parsed path.
//!
//! What neither reading can see, the textual fallback bounds: before a rule
//! reports an entry, `mentioned` scans the package's source text for the
//! crate name between identifier boundaries, which covers doctests and
//! attribute arguments. The residual invisible class, a path constructed by
//! a macro, is named in the rule help as an abstention.

use std::collections::{BTreeMap, BTreeSet};

use ra_ap_syntax::ast::{self, HasAttrs};
use ra_ap_syntax::{AstNode, SyntaxKind, SyntaxNode, SyntaxToken, TextRange};

use crate::source_text::compact;

use super::{Enumeration, SourceUnit};

/// Where a package's sources reference one crate name.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ReferenceSites {
    /// Seen from a lib, bin or build-script target, outside any inline
    /// `#[cfg(test)]` module.
    pub(crate) production: bool,
    /// Seen from a test, bench or example target.
    pub(crate) test_target: bool,
    /// Seen inside an inline `#[cfg(test)]` module of a production target.
    pub(crate) inline_test: bool,
}

impl ReferenceSites {
    pub(crate) const fn anywhere(self) -> bool {
        self.production || self.test_target || self.inline_test
    }
}

/// References collected for every workspace package, keyed by package id.
#[derive(Debug, Default)]
pub(crate) struct CrateReferences {
    names: BTreeMap<String, BTreeMap<String, ReferenceSites>>,
    /// Packages one of whose units the parser rejected: their collection is
    /// incomplete and the dependency rules abstain for them.
    incomplete_packages: BTreeSet<String>,
    /// An enumeration failure that names no package, a module the walk could
    /// not reach for instance: no package's collection can claim completeness.
    incomplete_everywhere: bool,
}

impl CrateReferences {
    pub(crate) fn incomplete(&self, package_id: &str) -> bool {
        self.incomplete_everywhere || self.incomplete_packages.contains(package_id)
    }

    pub(crate) fn sites(&self, package_id: &str, written_name: &str) -> ReferenceSites {
        self.names
            .get(package_id)
            .and_then(|names| names.get(written_name))
            .copied()
            .unwrap_or_default()
    }
}

/// The reach a textual mention must have to count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mention {
    Anywhere,
    ProductionOnly,
}

/// Builds the per-package reference sets from the one enumeration the source
/// producers already share. No file is read and no directory is walked here:
/// the units were loaded once, and this pass only re-reads their trees.
pub(crate) fn collect(enumeration: &Enumeration) -> CrateReferences {
    let mut references = CrateReferences {
        incomplete_everywhere: enumeration
            .errors
            .iter()
            .any(|error| error.code != "parse-error"),
        ..CrateReferences::default()
    };

    for unit in enumeration.units.values() {
        let packages: Vec<String> = unit.package_ids().map(str::to_owned).collect();
        if !unit.parses_cleanly() {
            references.incomplete_packages.extend(packages);
            continue;
        }
        // A unit a production target also reaches counts as production, because
        // reporting shipped code as test-only is the expensive mistake.
        let test_unit = unit.is_test_target(enumeration.contexts());
        collect_unit(unit, &packages, test_unit, &mut references);
    }

    references
}

fn collect_unit(
    unit: &SourceUnit,
    packages: &[String],
    test_unit: bool,
    references: &mut CrateReferences,
) {
    // Every site of a test unit is a test site, so the gated ranges only have
    // to be found inside a production one.
    let gated = if test_unit {
        Vec::new()
    } else {
        test_gated_ranges(unit)
    };

    for node in unit.tree().syntax().descendants() {
        match node.kind() {
            // Only the outermost path of a chain carries the crate-naming
            // first segment: `a::b::c` nests three PATH nodes and the two
            // inner ones repeat what the outer one already says.
            SyntaxKind::PATH
                if node
                    .parent()
                    .is_none_or(|parent| parent.kind() != SyntaxKind::PATH) =>
            {
                let Some(name) = written_crate_name(&node) else {
                    continue;
                };
                record(
                    references,
                    packages,
                    &name,
                    site(test_unit, &gated, node.text_range()),
                );
            }
            SyntaxKind::EXTERN_CRATE => {
                let Some(name) = ast::ExternCrate::cast(node.clone())
                    .and_then(|declaration| declaration.name_ref())
                    .map(|name| name.text().to_string())
                    .filter(|name| name != "self")
                else {
                    continue;
                };
                record(
                    references,
                    packages,
                    &name,
                    site(test_unit, &gated, node.text_range()),
                );
            }
            SyntaxKind::TOKEN_TREE => {
                for token in chain_heads(&node) {
                    record(
                        references,
                        packages,
                        token.text(),
                        site(test_unit, &gated, token.text_range()),
                    );
                }
            }
            _ => {}
        }
    }
}

/// First segment of a written path, which is the only one that can name a
/// crate. A segment that is not a plain name, `crate` or `<T as Trait>` for
/// instance, names no crate.
fn written_crate_name(node: &SyntaxNode) -> Option<String> {
    match ast::Path::cast(node.clone())?.segments().next()?.kind()? {
        ast::PathSegmentKind::Name(name) => Some(name.text().to_string()),
        _ => None,
    }
}

/// Identifiers heading a written path inside a macro's token tree.
///
/// Macro arguments are tokens rather than parsed paths, and test code lives
/// there almost entirely (`assert_eq!` and its siblings). An identifier a `::`
/// follows and no `::` precedes is the head of a written chain; inside a token
/// tree that separator is two bare `:` tokens. Nested trees are their own
/// nodes, so each token is read exactly once.
fn chain_heads(tree: &SyntaxNode) -> Vec<SyntaxToken> {
    let tokens: Vec<SyntaxToken> = tree
        .children_with_tokens()
        .filter_map(|element| element.into_token())
        .filter(|token| !token.kind().is_trivia())
        .collect();
    let separator = |index: usize| {
        tokens
            .get(index)
            .zip(tokens.get(index + 1))
            .is_some_and(|(first, second)| {
                first.kind() == SyntaxKind::COLON && second.kind() == SyntaxKind::COLON
            })
    };
    tokens
        .iter()
        .enumerate()
        .filter(|(index, token)| {
            token.kind() == SyntaxKind::IDENT
                && separator(index + 1)
                && !(*index >= 2 && separator(index - 2))
        })
        .map(|(_, token)| token.clone())
        .collect()
}

fn site(test_unit: bool, test_ranges: &[TextRange], range: TextRange) -> ReferenceSites {
    if test_unit {
        ReferenceSites {
            test_target: true,
            ..ReferenceSites::default()
        }
    } else if test_ranges.iter().any(|gated| gated.contains_range(range)) {
        ReferenceSites {
            inline_test: true,
            ..ReferenceSites::default()
        }
    } else {
        ReferenceSites {
            production: true,
            ..ReferenceSites::default()
        }
    }
}

fn record(
    references: &mut CrateReferences,
    packages: &[String],
    name: &str,
    sites: ReferenceSites,
) {
    let name = name.trim_start_matches("r#");
    if name.is_empty() {
        return;
    }
    for package in packages {
        let entry = references
            .names
            .entry(package.clone())
            .or_default()
            .entry(name.to_owned())
            .or_default();
        entry.production |= sites.production;
        entry.test_target |= sites.test_target;
        entry.inline_test |= sites.inline_test;
    }
}

/// Only the exact form `#[cfg(test)]` gates a module here. A wider condition,
/// `#[cfg(any(test, feature = "..."))]` for instance, keeps its references in
/// production, because misclassifying shipped code as test-only is the
/// expensive direction.
fn is_cfg_test(attribute: &ast::Attr) -> bool {
    let Some(ast::Meta::CfgMeta(meta)) = attribute.meta() else {
        return false;
    };
    compact(meta.syntax()) == "cfg(test)"
}

/// Does any source text of the package mention the name between identifier
/// boundaries? This is the abstention check the rules run before reporting:
/// it reads what the parser cannot claim, doc comments, macro token trees and
/// attribute arguments, and one mention is enough to silence a finding.
///
/// Under `Mention::ProductionOnly`, a mention living in a test unit or inside
/// an inline `#[cfg(test)]` module does not count: that text is exactly what
/// the test-only rule reports, so reading it as production evidence would
/// silence the rule against itself.
pub(crate) fn mentioned(
    enumeration: &Enumeration,
    package_id: &str,
    name: &str,
    reach: Mention,
) -> bool {
    let underscored = name.replace('-', "_");
    let hyphenated = name.replace('_', "-");
    enumeration
        .units
        .values()
        .filter(|unit| unit.package_ids().any(|id| id == package_id))
        .filter(|unit| {
            reach == Mention::Anywhere || !unit.is_test_target(enumeration.contexts())
        })
        .any(|unit| {
            let excluded = match reach {
                Mention::Anywhere => Vec::new(),
                Mention::ProductionOnly => test_gated_ranges(unit),
            };
            contains_identifier(unit.source(), &underscored, &excluded)
                || contains_identifier(unit.source(), &hyphenated, &excluded)
        })
}

/// Ranges of the unit's inline `#[cfg(test)]` modules. One implementation, so
/// what the collector calls a test site and what the textual fallback refuses
/// to read as production evidence cannot come apart.
fn test_gated_ranges(unit: &SourceUnit) -> Vec<TextRange> {
    unit.tree()
        .syntax()
        .descendants()
        .filter_map(ast::Module::cast)
        .filter(|module| module.attrs().any(|attribute| is_cfg_test(&attribute)))
        .map(|module| module.syntax().text_range())
        .collect()
}

/// Substring match whose both neighbors are outside the identifier alphabet,
/// hyphen included, so "serde" never matches inside "serde_json" or
/// "serde-json". A match inside an excluded range does not count.
fn contains_identifier(haystack: &str, needle: &str, excluded: &[TextRange]) -> bool {
    if needle.is_empty() {
        return false;
    }
    let bytes = haystack.as_bytes();
    let boundary = |byte: Option<u8>| {
        byte.is_none_or(|byte| !(byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-'))
    };
    let mut from = 0;
    while let Some(position) = haystack[from..].find(needle) {
        let start = from + position;
        let end = start + needle.len();
        let gated = u32::try_from(start).is_ok_and(|offset| {
            excluded
                .iter()
                .any(|range| range.contains(ra_ap_syntax::TextSize::from(offset)))
        });
        if !gated
            && boundary(start.checked_sub(1).map(|index| bytes[index]))
            && boundary(bytes.get(end).copied())
        {
            return true;
        }
        from = end;
    }
    false
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use crate::report::DiagnosticContext;
    use ra_ap_syntax::{Edition, SourceFile};

    use super::super::{Identity, Reachability};
    use super::*;

    fn unit(source: &str, package: &str, path: &str) -> SourceUnit {
        let parse = SourceFile::parse(source, Edition::Edition2024);
        SourceUnit {
            source: source.to_owned(),
            error_ranges: parse.errors().iter().map(|error| error.range()).collect(),
            parse,
            edition: Edition::Edition2024,
            relative_path: path.to_owned(),
            reachability: BTreeSet::from([Reachability {
                package_id: package.to_owned(),
                package_name: package.to_owned(),
                // One synthetic target per unit: the path keeps two units of
                // the same package from sharing a context entry.
                target_key: format!("{package}:{path}"),
                target_name: package.to_owned(),
                test_gated: false,
            }]),
            traversals: BTreeSet::new(),
        }
    }

    fn enumeration_of(units: Vec<(SourceUnit, Option<DiagnosticContext>)>) -> Enumeration {
        let mut enumeration = Enumeration::default();
        for (index, (unit, context)) in units.into_iter().enumerate() {
            let key = unit
                .reachability
                .first()
                .expect("a synthetic unit names its package")
                .target_key
                .clone();
            enumeration.contexts.insert(key, context);
            enumeration.units.insert(
                Identity {
                    path: std::path::PathBuf::from(format!("/synthetic/{index}.rs")),
                    edition: Edition::Edition2024,
                },
                unit,
            );
        }
        enumeration
    }

    /// US-005: the four written reference forms are collected, and the first
    /// segment is the only one that names a crate.
    #[test]
    fn use_qualified_extern_and_macro_references_are_collected() {
        let source = "use serde::Serialize;
extern crate libc;
fn build() -> String {
    let value = serde_json::json!({});
    ::toml::to_string(&value).unwrap_or_default()
}";
        let enumeration = enumeration_of(vec![(unit(source, "alpha", "src/lib.rs"), None)]);
        let references = collect(&enumeration);

        for name in ["serde", "libc", "serde_json", "toml"] {
            let sites = references.sites("alpha", name);
            assert!(sites.production, "{name} was not collected");
            assert!(!sites.test_target && !sites.inline_test, "{name}");
        }
        // The tail segments of a chain never name a crate.
        assert!(!references.sites("alpha", "Serialize").anywhere());
        assert!(!references.sites("alpha", "to_string").anywhere());
        assert!(!references.sites("alpha", "json").anywhere());
    }

    /// `self::`, `crate::` and `super::` heads are module navigation, not
    /// crate references.
    #[test]
    fn module_navigation_heads_are_not_collected() {
        let source = "mod inner { pub fn probe() {} }
fn run() {
    self::inner::probe();
    crate::inner::probe();
}";
        let enumeration = enumeration_of(vec![(unit(source, "alpha", "src/lib.rs"), None)]);
        let references = collect(&enumeration);
        assert!(!references.sites("alpha", "self").anywhere());
        assert!(!references.sites("alpha", "crate").anywhere());
        assert!(!references.sites("alpha", "super").anywhere());
        // `inner` only ever appears as a tail segment behind a navigation
        // head, and a tail segment never names a crate.
        assert!(!references.sites("alpha", "inner").anywhere());
    }

    /// US-007's evidence: a reference inside an inline `#[cfg(test)]` module
    /// of a production unit is recorded as such, and a reference from a unit
    /// every target marks as test material lands in `test_target`.
    #[test]
    fn references_are_classified_by_target_context_and_test_gate() {
        let lib = "pub fn shipped() { probe_shipped::run(); }
#[cfg(test)]
mod tests {
    #[test]
    fn covered() { probe_inline::run(); }
}";
        let integration = "fn probe() { probe_integration::run(); }";
        let enumeration = enumeration_of(vec![
            (unit(lib, "alpha", "src/lib.rs"), None),
            (
                unit(integration, "alpha", "tests/probe.rs"),
                Some(DiagnosticContext::Tests),
            ),
        ]);
        let references = collect(&enumeration);

        let shipped = references.sites("alpha", "probe_shipped");
        assert!(shipped.production && !shipped.test_target && !shipped.inline_test);
        let inline = references.sites("alpha", "probe_inline");
        assert!(inline.inline_test && !inline.production && !inline.test_target);
        let integration = references.sites("alpha", "probe_integration");
        assert!(integration.test_target && !integration.production && !integration.inline_test);
    }

    /// A qualifying identifier inside a macro token tree is a reference, with
    /// the same classification as a parsed path, and a tail or an
    /// already-qualified identifier is not.
    #[test]
    fn qualifying_identifiers_inside_macro_arguments_are_collected() {
        let source = "pub fn shipped() { assert_eq!(probe_macro_arg::value(), 1); }
#[cfg(test)]
mod tests {
    #[test]
    fn covered() { assert_eq!(probe_macro_gated::value(), tail_module::probe::value()); }
}";
        let enumeration = enumeration_of(vec![(unit(source, "alpha", "src/lib.rs"), None)]);
        let references = collect(&enumeration);

        assert!(references.sites("alpha", "probe_macro_arg").production);
        let gated = references.sites("alpha", "probe_macro_gated");
        assert!(gated.inline_test && !gated.production);
        // `probe` sits behind `tail_module::` and is not a chain head.
        assert!(!references.sites("alpha", "probe").anywhere());
        assert!(references.sites("alpha", "tail_module").inline_test);
        // `value` heads no chain anywhere.
        assert!(!references.sites("alpha", "value").anywhere());
    }

    /// A wider cfg than the exact `#[cfg(test)]` keeps its references in
    /// production: silencing a shipped reference is the expensive direction.
    #[test]
    fn a_wider_cfg_condition_stays_production() {
        let source = "#[cfg(any(test, feature = \"probe\"))]
mod gated { pub fn run() { probe_wide::run(); } }";
        let enumeration = enumeration_of(vec![(unit(source, "alpha", "src/lib.rs"), None)]);
        let references = collect(&enumeration);
        assert!(references.sites("alpha", "probe_wide").production);
    }

    /// US-005: a unit the parser rejects contributes nothing and marks its
    /// packages incomplete, while the other packages keep their collection.
    #[test]
    fn a_parse_failure_marks_only_its_packages_incomplete() {
        let broken = "fn broken( { probe_broken::run(); }";
        let healthy = "pub fn fine() { probe_fine::run(); }";
        let enumeration = enumeration_of(vec![
            (unit(broken, "alpha", "src/lib.rs"), None),
            (unit(healthy, "beta", "src/lib.rs"), None),
        ]);
        let references = collect(&enumeration);

        assert!(references.incomplete("alpha"));
        assert!(!references.incomplete("beta"));
        assert!(references.sites("beta", "probe_fine").production);
    }

    /// An enumeration error that names no package, a module the walk never
    /// reached for instance, leaves no package able to claim a complete
    /// collection.
    #[test]
    fn an_unattributed_enumeration_error_makes_every_collection_incomplete() {
        let mut enumeration =
            enumeration_of(vec![(unit("pub fn fine() {}", "alpha", "src/lib.rs"), None)]);
        enumeration.errors.push(super::super::SourceError {
            code: "module-not-found",
            message: "Module \"probe\" declared in \"src/lib.rs\" could not be resolved.".to_owned(),
        });
        let references = collect(&enumeration);
        assert!(references.incomplete("alpha"));
        assert!(references.incomplete("beta"));
    }

    /// The textual fallback reads doc comments and macro token trees between
    /// identifier boundaries, in both the hyphenated and the underscored
    /// spelling, and never across a longer identifier.
    #[test]
    fn textual_mentions_respect_identifier_boundaries() {
        let source = "/// Compare with the `probe-doc` crate before removing.
macro_rules! wrap { () => { probe_macro::run() } }
pub fn shipped() { let serde_json = 1; let _ = serde_json; }";
        let enumeration = enumeration_of(vec![(unit(source, "alpha", "src/lib.rs"), None)]);

        assert!(mentioned(&enumeration, "alpha", "probe-doc", Mention::Anywhere));
        assert!(mentioned(&enumeration, "alpha", "probe_macro", Mention::Anywhere));
        // `serde` never matches inside `serde_json`.
        assert!(!mentioned(&enumeration, "alpha", "serde", Mention::Anywhere));
        assert!(!mentioned(&enumeration, "beta", "probe_macro", Mention::Anywhere));
    }

    /// The production-only reach ignores mentions living in test units and
    /// mentions inside an inline `#[cfg(test)]` module: that text is what the
    /// test-only rule reports, so it cannot also be the evidence that
    /// silences it.
    #[test]
    fn production_reach_ignores_test_units_and_gated_modules() {
        let lib = "pub fn shipped() {}
#[cfg(test)]
mod tests {
    #[test]
    fn covered() { probe_gated::run(); }
}";
        let enumeration = enumeration_of(vec![
            (unit(lib, "alpha", "src/lib.rs"), None),
            (
                unit("fn probe() { probe_test::run(); }", "alpha", "tests/probe.rs"),
                Some(DiagnosticContext::Tests),
            ),
        ]);
        assert!(mentioned(&enumeration, "alpha", "probe_test", Mention::Anywhere));
        assert!(!mentioned(&enumeration, "alpha", "probe_test", Mention::ProductionOnly));
        assert!(mentioned(&enumeration, "alpha", "probe_gated", Mention::Anywhere));
        assert!(!mentioned(&enumeration, "alpha", "probe_gated", Mention::ProductionOnly));
        assert!(mentioned(&enumeration, "alpha", "shipped", Mention::ProductionOnly));
    }
}
