use std::collections::BTreeMap;

use cargo_metadata::{Dependency, DependencyKind, Package};
use serde::Deserialize;

use crate::source_text::{SourceSpan, byte_range_span, line_starts};

type DependencyTable = BTreeMap<toml::Spanned<String>, toml::Value>;

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

#[derive(Debug, Default, Deserialize)]
struct DependencyDocument {
    dependencies: Option<DependencyTable>,
    #[serde(rename = "dev-dependencies")]
    dev_dependencies: Option<DependencyTable>,
    #[serde(rename = "build-dependencies")]
    build_dependencies: Option<DependencyTable>,
    target: Option<BTreeMap<String, DependencyTables>>,
}

pub(super) struct DeclaredKeys {
    source: String,
    starts: Vec<usize>,
    document: DependencyDocument,
}

impl DeclaredKeys {
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
