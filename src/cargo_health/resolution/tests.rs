use std::collections::BTreeSet;

use super::{
    DuplicateMajor, LockedPackage, Requirement, ResolvedVersion, compatibility_class,
    duplicate_major_versions,
};

fn locked(entries: &[(&str, &str, &[&str])]) -> Vec<LockedPackage> {
    entries
        .iter()
        .map(|(name, version, dependencies)| LockedPackage {
            name: (*name).to_owned(),
            version: (*version).to_owned(),
            source: None,
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

#[test]
fn major_comparison_is_order_independent_and_ignores_prerelease_metadata() {
    let found = duplicates(&[("a", "2.0.0", &[]), ("a", "1.0.0", &[])]);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name, "a");
    assert_eq!(
        found[0].versions,
        [
            version("1.0.0", Requirement::Unreached, false),
            version("2.0.0", Requirement::Unreached, false)
        ]
    );
    assert!(duplicates(&[("a", "1.0.0-rc.1", &[]), ("a", "1.0.0+build", &[])]).is_empty());
    assert!(duplicates(&[("a", "1.0.0", &[]), ("b", "2.0.0", &[])]).is_empty());
    assert!(duplicates(&[("a", "1.0.0", &[]), ("a", "1.0.0", &[])]).is_empty());
    assert!(duplicates(&[("a", "x.0.0", &[]), ("a", "1.0.0", &[])]).is_empty());
    assert_eq!(compatibility_class("10.2.3"), Some("10"));
    assert_eq!(compatibility_class(""), None);
}

#[test]
fn versions_compare_by_cargo_compatibility_class() {
    assert_eq!(compatibility_class("1.4.0"), Some("1"));
    assert_eq!(compatibility_class("0.59.0"), Some("0.59"));
    assert_eq!(compatibility_class("0.61.2"), Some("0.61"));
    assert_eq!(compatibility_class("0.0.3"), Some("0.0.3"));
    assert_eq!(compatibility_class("0.0.0"), Some("0.0.0"));
    assert_eq!(compatibility_class("1.0.0-rc.1+abc"), Some("1"));
    assert_eq!(compatibility_class("0.7.0-alpha.2"), Some("0.7"));
    assert_eq!(compatibility_class("1.0"), None, "a lockfile version is three numbers");
    assert_eq!(compatibility_class("1.x.0"), None);

    assert_eq!(duplicates(&[("winsys", "0.59.0", &[]), ("winsys", "0.61.2", &[])]).len(), 1);
    assert_eq!(duplicates(&[("tiny", "0.0.3", &[]), ("tiny", "0.0.4", &[])]).len(), 1);
    assert!(duplicates(&[("same", "1.4.0", &[]), ("same", "1.9.2", &[])]).is_empty());
    assert!(duplicates(&[("same", "0.7.1", &[]), ("same", "0.7.15", &[])]).is_empty());
    assert!(duplicates(&[("rc", "1.0.0-rc.1+abc", &[]), ("rc", "1.2.0", &[])]).is_empty());
}

fn members(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

fn version(version: &str, requirement: Requirement, members_only: bool) -> ResolvedVersion {
    ResolvedVersion {
        version: version.to_owned(),
        requirement,
        members_only,
    }
}

#[test]
fn a_duplicate_is_attributed_to_who_requires_it() {
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
    let found = duplicate_major_versions(&packages, &members(&["app"]));
    assert_eq!(found.len(), 2, "{found:?}");

    let serde = &found[0];
    assert_eq!(serde.name, "serde");
    assert_eq!(
        serde.versions,
        [
            version("1.0.0", Requirement::Unreached, false),
            version("2.0.0", Requirement::Direct(vec!["app".to_owned()]), true)
        ]
    );
    assert_eq!(
        serde.message(),
        "Crate \"serde\" is resolved with incompatible versions 1.0.0, 2.0.0 (required by app)."
    );
    assert_eq!(serde.removable_by(), Some("app"));

    let winnow = &found[1];
    assert_eq!(
        winnow.message(),
        "Crate \"winnow\" is resolved with incompatible versions 0.7.15 (through syntax), 1.0.4 (through toml)."
    );
    assert_eq!(winnow.removable_by(), None, "no member manifest can remove it");
}

#[test]
fn a_version_only_members_require_is_one_a_member_can_remove() {
    let packages = locked(&[
        ("app", "0.1.0", &["png 0.5.0", "image"]),
        ("image", "1.0.0", &["png 0.6.0"]),
        ("png", "0.5.0", &[]),
        ("png", "0.6.0", &[]),
    ]);
    let found = duplicate_major_versions(&packages, &members(&["app"]));
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].removable_by(), Some("app"));
    assert_eq!(
        found[0].message(),
        "Crate \"png\" is resolved with incompatible versions 0.5.0 (required by app), 0.6.0 (through image)."
    );

    let shared = locked(&[
        ("app", "0.1.0", &["png 0.5.0", "image", "decoder"]),
        ("decoder", "1.0.0", &["png 0.5.0"]),
        ("image", "1.0.0", &["png 0.6.0"]),
        ("png", "0.5.0", &[]),
        ("png", "0.6.0", &[]),
    ]);
    let found = duplicate_major_versions(&shared, &members(&["app"]));
    assert_eq!(found[0].removable_by(), None, "a third-party crate also requires 0.5");
}

#[test]
fn a_through_label_never_names_a_member_and_ties_break_by_name() {
    let packages = locked(&[
        ("app", "0.1.0", &["core", "zeta", "alpha"]),
        ("core", "0.1.0", &["lib"]),
        ("lib", "1.0.0", &["hb 1.0.0"]),
        ("zeta", "1.0.0", &["hb 2.0.0"]),
        ("alpha", "1.0.0", &["hb 2.0.0"]),
        ("hb", "1.0.0", &[]),
        ("hb", "2.0.0", &[]),
    ]);
    let found = duplicate_major_versions(&packages, &members(&["app", "core"]));
    assert_eq!(
        found[0].message(),
        "Crate \"hb\" is resolved with incompatible versions 1.0.0 (through lib), 2.0.0 (through alpha)."
    );
    assert_eq!(found[0].removable_by(), None);
}

#[test]
fn a_required_by_label_names_three_members_then_counts_the_rest() {
    let packages = locked(&[
        ("echo", "0.1.0", &["log 0.4.0"]),
        ("bravo", "0.1.0", &["log 0.4.0"]),
        ("delta", "0.1.0", &["log 0.4.0"]),
        ("alpha", "0.1.0", &["log 0.4.0"]),
        ("charlie", "0.1.0", &["log 0.4.0"]),
        ("tracer", "1.0.0", &["log 1.0.0"]),
        ("alpha2", "0.1.0", &["tracer"]),
        ("log", "0.4.0", &[]),
        ("log", "1.0.0", &[]),
    ]);
    let found = duplicate_major_versions(
        &packages,
        &members(&["echo", "bravo", "delta", "alpha", "charlie", "alpha2"]),
    );
    assert_eq!(
        found[0].message(),
        "Crate \"log\" is resolved with incompatible versions 0.4.0 (required by alpha, bravo, charlie +2 more), 1.0.0 (through tracer)."
    );
    assert_eq!(found[0].removable_by(), Some("alpha"));
}

#[test]
fn versions_are_listed_in_semver_order() {
    let found = duplicates(&[("w", "0.10.1", &[]), ("w", "0.6.4", &[]), ("w", "0.9.5", &[])]);
    assert_eq!(
        found[0].message(),
        "Crate \"w\" is resolved with incompatible versions 0.6.4, 0.9.5, 0.10.1."
    );
    let pre = duplicates(&[("p", "2.0.0", &[]), ("p", "2.0.0-rc.1", &[]), ("p", "1.10.0", &[])]);
    let listed: Vec<&str> = pre[0].versions.iter().map(|resolved| resolved.version.as_str()).collect();
    assert_eq!(listed, ["1.10.0", "2.0.0-rc.1", "2.0.0"]);
}
