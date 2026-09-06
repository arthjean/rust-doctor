//! The resolved graph read on its own: which majors collide, and who asked for each.

use std::collections::BTreeSet;

use super::{DuplicateMajor, LockedPackage, Requirement, duplicate_major_versions, major_of};

fn locked(entries: &[(&str, &str, &[&str])]) -> Vec<LockedPackage> {
    entries
        .iter()
        .map(|(name, version, dependencies)| LockedPackage {
            name: (*name).to_owned(),
            version: (*version).to_owned(),
            dependencies: dependencies.iter().map(|dependency| (*dependency).to_owned()).collect(),
        })
        .collect()
}

fn no_members() -> BTreeSet<String> {
    BTreeSet::new()
}

fn duplicates(entries: &[(&str, &str, &[&str])]) -> Vec<DuplicateMajor> {
    duplicate_major_versions(&locked(entries), &no_members())
}

/// The ordering of major versions is independent of file order and ignores
/// pre-releases and build metadata.
#[test]
fn major_comparison_is_order_independent_and_ignores_prerelease_metadata() {
    let found = duplicates(&[("a", "2.0.0", &[]), ("a", "1.0.0", &[])]);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name, "a");
    assert_eq!(
        found[0].versions,
        [
            ("1.0.0".to_owned(), Requirement::Unreached),
            ("2.0.0".to_owned(), Requirement::Unreached)
        ]
    );
    assert!(duplicates(&[("a", "1.0.0-rc.1", &[]), ("a", "1.0.0+build", &[])]).is_empty());
    assert!(duplicates(&[("a", "1.0.0", &[]), ("b", "2.0.0", &[])]).is_empty());
    assert!(duplicates(&[("a", "1.0.0", &[]), ("a", "1.0.0", &[])]).is_empty());
    // An unreadable version cannot be compared, so it cannot ground a
    // duplication verdict.
    assert!(duplicates(&[("a", "x.0.0", &[]), ("a", "1.0.0", &[])]).is_empty());
    assert_eq!(major_of("10.2.3"), Some("10"));
    assert_eq!(major_of(""), None);
}

/// Each version of a duplicated crate is attributed to the member that requires it or to the
/// member's dependency it arrives under.
///
/// The shape is this crate's own lockfile: `winnow` 0.7 under `ra_ap_parser` and 1.0 under
/// `toml`, neither named by any manifest here. The old message said the two majors were
/// resolved and nothing about which dependency to bump.
#[test]
fn a_duplicate_is_attributed_to_who_requires_it() {
    let members: BTreeSet<String> = ["app".to_owned()].into_iter().collect();
    let packages = locked(&[
        ("app", "0.1.0", &["toml 1.0.0", "syntax", "serde 2.0.0"]),
        ("toml", "1.0.0", &["winnow 1.0.4"]),
        ("syntax", "0.0.1", &["parser"]),
        ("parser", "0.0.1", &["winnow 0.7.15 (registry+https://example.invalid)"]),
        ("winnow", "1.0.4", &[]),
        ("winnow", "0.7.15", &[]),
        ("serde", "2.0.0", &[]),
        ("serde", "1.0.0", &[]),
        ("orphan", "3.0.0", &["serde 1.0.0"]),
    ]);
    let found = duplicate_major_versions(&packages, &members);
    assert_eq!(found.len(), 2, "{found:?}");

    let serde = &found[0];
    assert_eq!(serde.name, "serde");
    assert_eq!(
        serde.versions,
        [
            ("1.0.0".to_owned(), Requirement::Unreached),
            ("2.0.0".to_owned(), Requirement::Direct("app".to_owned()))
        ]
    );
    assert_eq!(
        serde.message(),
        "Crate \"serde\" is resolved with incompatible major versions 1.0.0, 2.0.0 (required by app)."
    );

    let winnow = &found[1];
    assert_eq!(winnow.name, "winnow");
    assert_eq!(
        winnow.versions,
        [
            ("0.7.15".to_owned(), Requirement::Through("syntax".to_owned())),
            ("1.0.4".to_owned(), Requirement::Through("toml".to_owned()))
        ]
    );
    assert_eq!(
        winnow.message(),
        "Crate \"winnow\" is resolved with incompatible major versions 0.7.15 (through syntax), 1.0.4 (through toml)."
    );
}
