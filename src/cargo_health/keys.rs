//! Where a member manifest declares each of its dependencies.
//!
//! `cargo metadata` names a dependency but not the line that declares it, so a
//! declaration finding used to publish no span, and a lines scope could not
//! tell whether the change under review added it. The member manifest is
//! re-read here under the pack's size bound, and each finding is anchored at
//! its dependency key, as react-doctor anchors a supply-chain finding at its
//! `package.json` key.
//!
//! The span is a location, never an identity: the diagnostic's fingerprint
//! still ignores it, so a finding keeps its baseline identity when lines are
//! inserted above the key.

use std::collections::BTreeMap;

use cargo_metadata::{Dependency, DependencyKind, Package};
use serde::Deserialize;

use crate::source_text::{SourceSpan, byte_range_span, line_starts};

type DependencyTable = BTreeMap<toml::Spanned<String>, toml::Value>;

/// The three dependency tables of a manifest, at its top level or under one
/// `[target.<platform>]`. Every other key is ignored without making the
/// document invalid.
#[derive(Debug, Default, Deserialize)]
struct DependencyTables {
    dependencies: Option<DependencyTable>,
    #[serde(rename = "dev-dependencies")]
    dev_dependencies: Option<DependencyTable>,
    #[serde(rename = "build-dependencies")]
    build_dependencies: Option<DependencyTable>,
}

impl DependencyTables {
    fn of(&self, kind: DependencyKind) -> Option<&DependencyTable> {
        match kind {
            DependencyKind::Normal => self.dependencies.as_ref(),
            DependencyKind::Development => self.dev_dependencies.as_ref(),
            DependencyKind::Build => self.build_dependencies.as_ref(),
            _ => None,
        }
    }
}

/// The same three tables plus the platform ones. They are spelled again
/// rather than flattened in: `#[serde(flatten)]` buffers the document, and a
/// buffered key loses the span `toml::Spanned` reads from the parser.
#[derive(Debug, Default, Deserialize)]
struct DependencyDocument {
    dependencies: Option<DependencyTable>,
    #[serde(rename = "dev-dependencies")]
    dev_dependencies: Option<DependencyTable>,
    #[serde(rename = "build-dependencies")]
    build_dependencies: Option<DependencyTable>,
    target: Option<BTreeMap<String, DependencyTables>>,
}

/// One member manifest, read once for every dependency it declares.
pub(super) struct DeclaredKeys {
    source: String,
    starts: Vec<usize>,
    document: DependencyDocument,
}

impl DeclaredKeys {
    /// The member's manifest, or nothing when it cannot be read or parsed:
    /// the findings then keep no span, as they did before this reader, and
    /// the metadata-derived verdicts are unchanged.
    pub(super) fn read(package: &Package) -> Option<Self> {
        let source =
            super::read_bounded_toml("manifest-unreadable", package.manifest_path.as_std_path())
                .ok()
                .flatten()?;
        let document = toml::from_str(&source).ok()?;
        Some(Self {
            starts: line_starts(&source),
            source,
            document,
        })
    }

    /// The span of the key declaring `dependency`.
    ///
    /// An entry inherited with `workspace = true` keeps none: what it is
    /// judged on is written in the root's `[workspace.dependencies]`, not on
    /// the member line. A platform table the metadata spells differently from
    /// the manifest keeps none either, rather than borrowing another table's
    /// key.
    pub(super) fn span(&self, dependency: &Dependency) -> Option<SourceSpan> {
        let table = match &dependency.target {
            None => {
                let document = &self.document;
                match dependency.kind {
                    DependencyKind::Normal => document.dependencies.as_ref(),
                    DependencyKind::Development => document.dev_dependencies.as_ref(),
                    DependencyKind::Build => document.build_dependencies.as_ref(),
                    _ => None,
                }
            }
            Some(platform) => self
                .document
                .target
                .as_ref()?
                .get(&platform.to_string())?
                .of(dependency.kind),
        };
        let key = dependency.rename.as_deref().unwrap_or(&dependency.name);
        let (spanned, value) = table?
            .iter()
            .find(|(candidate, _)| candidate.get_ref() == key)?;
        let inherited = value
            .get("workspace")
            .and_then(toml::Value::as_bool)
            .unwrap_or(false);
        (!inherited).then(|| byte_range_span(spanned.span(), &self.starts, &self.source))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dependency(name: &str, kind: Option<&str>, target: Option<&str>) -> Dependency {
        serde_json::from_value(serde_json::json!({
            "name": name,
            "source": "registry+https://github.com/rust-lang/crates.io-index",
            "req": "*",
            "kind": kind,
            "rename": null,
            "optional": false,
            "uses_default_features": true,
            "features": [],
            "target": target,
            "registry": null,
            "path": null,
        }))
        .expect("a dependency record should deserialize")
    }

    /// US-002: each table answers for its own entries, a key is located on
    /// its own line in every spelling, and an inherited entry stays span-less.
    #[test]
    fn every_dependency_table_anchors_its_key_and_inheritance_keeps_none() {
        let source = "[package]\nname = \"a\"\n\n[dependencies]\nserde = \"*\"\n\
                      shared = { workspace = true }\n\n[dev-dependencies.proptest]\nversion = \"*\"\n\n\
                      [target.'cfg(unix)'.build-dependencies]\ncc = \"*\"\n";
        let keys = DeclaredKeys {
            starts: line_starts(source),
            source: source.to_owned(),
            document: toml::from_str(source).expect("the manifest should parse"),
        };
        let line = |dependency: &Dependency| keys.span(dependency).map(|span| span.line_start);
        assert_eq!(line(&dependency("serde", None, None)), Some(5));
        assert_eq!(line(&dependency("shared", None, None)), None);
        assert_eq!(line(&dependency("proptest", Some("dev"), None)), Some(8));
        assert_eq!(line(&dependency("proptest", None, None)), None);
        assert_eq!(line(&dependency("cc", Some("build"), Some("cfg(unix)"))), Some(12));
        assert_eq!(line(&dependency("cc", Some("build"), None)), None);
    }
}
