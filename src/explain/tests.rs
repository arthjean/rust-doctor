use std::collections::BTreeSet;

use super::*;
use crate::test_scratch::scratch;

#[test]
fn an_exact_id_or_a_unique_last_segment_resolves_and_a_typo_names_the_closest() {
    assert_eq!(resolve_rule("clippy::todo").map(|rule| rule.id), Ok("clippy::todo"));
    assert_eq!(resolve_rule("todo").map(|rule| rule.id), Ok("clippy::todo"));
    assert_eq!(
        resolve_rule("tood").map(|rule| rule.id),
        Err(ExplainError::Unknown {
            query: "tood".to_owned(),
            closest: Some("clippy::todo"),
        })
    );
    assert!(matches!(
        resolve_rule("no_such_rule_anywhere"),
        Err(ExplainError::Unknown { closest: None, .. })
    ));
}

#[test]
fn an_explanation_carries_the_catalog_and_the_corpus_measurement() {
    let explained = explain_rule("clippy::expect_used", None).unwrap();
    assert_eq!(explained.rule.category, "reliability");
    assert_eq!(explained.url, crate::presentation::rule_url("clippy::expect_used"));
    assert!(explained.corpus_reviewed_sites.is_some());
    assert!(explained.effective.is_none());
    let text = explained.text();
    for line in ["Producer: clippy", "Tier: ", "Default level: ", "Help: ", "Rule: https://", "Corpus noise: "] {
        assert!(text.contains(line), "{text}");
    }
}

/// The effective level comes from `rust-doctor.toml` alone, with every
/// `[[overrides]]` table that sets the rule or its category.
#[test]
fn a_workspace_names_the_effective_level_its_source_and_its_overrides() {
    let root = scratch("explain", "effective-level");
    std::fs::write(
        root.join("rust-doctor.toml"),
        "[rules]\n\"clippy::todo\" = \"error\"\n\n[[overrides]]\npaths = [\"tests/**\"]\nrules = { \"clippy::todo\" = \"off\" }\n\n[[overrides]]\npaths = [\"benches/**\"]\nrules = { \"clippy::dbg_macro\" = \"off\" }\n",
    )
    .unwrap();
    let explained = explain_rule("todo", Some(&root)).unwrap();
    let effective = explained.effective.as_ref().unwrap();
    assert_eq!((effective.level, effective.source), (RuleLevel::Error, RuleLevelSource::ConfigRule));
    assert_eq!(effective.overrides.len(), 1);
    let text = explained.text();
    assert!(text.contains("Effective level: error (config-rule)"), "{text}");
    assert!(text.contains("Override: off under tests/**"), "{text}");

    let bare = scratch("explain", "default-level");
    let explained = explain_rule("todo", Some(&bare)).unwrap();
    assert_eq!(explained.effective.unwrap().source, RuleLevelSource::Default);
}

#[test]
fn a_location_splits_from_the_right_and_refuses_a_non_positive_line() {
    let parsed: WhyLocation = "./src/a.rs:12:5".parse().unwrap();
    assert_eq!((parsed.path.as_str(), parsed.line, parsed.column), ("src/a.rs", 12, Some(5)));
    let parsed: WhyLocation = "C:/x/a.rs:3".parse().unwrap();
    assert_eq!((parsed.path.as_str(), parsed.line, parsed.column), ("C:/x/a.rs", 3, None));
    for refused in ["src/a.rs:0", "src/a.rs:-2", "src/a.rs", "src/a.rs:x", ":3", "src/a.rs:3:0"] {
        assert!(refused.parse::<WhyLocation>().is_err(), "{refused}");
    }
}

fn report() -> serde_json::Value {
    serde_json::json!({
        "policy": {
            "rules": [{ "id": "clippy::todo", "source": "default" }],
            "ignore": ["vendor/**"],
        },
        "diagnostics": [
            {
                "code": "clippy::todo", "severity": "warning", "message": "`todo!()` left",
                "help": "finish it", "path": "src/lib.rs",
                "span": { "line_start": 2, "column_start": 5, "line_end": 2, "column_end": 12 },
                "related": [],
            },
            {
                "code": "rust_doctor::structure::duplicate_function_body", "severity": "warning",
                "message": "duplicate", "path": "src/a.rs",
                "span": { "line_start": 1, "column_start": 1, "line_end": 3, "column_end": 2 },
                "related": [{ "path": "src/lib.rs", "span": { "line_start": 9, "column_start": 1, "line_end": 11, "column_end": 2 } }],
            },
        ],
        "suppressions": [
            { "path": "src/lib.rs", "line": 8, "rules": ["rust_doctor::structure::duplicate_function_body"], "status": "missing-reason" },
        ],
    })
}

#[test]
fn a_hit_carries_its_level_source_its_suppressions_and_the_directive_to_write() {
    let root = scratch("explain", "why-hits");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/lib.rs"), "pub fn f() -> u8 {\n    todo!()\n}\n").unwrap();
    let site = explain_site(&report(), "src/lib.rs:2".parse().unwrap(), &root, &BTreeSet::new());
    assert_eq!(site.hits.len(), 1);
    let hit = &site.hits[0];
    assert_eq!(hit.level_source.as_deref(), Some("default"));
    assert_eq!(
        hit.directive.as_deref(),
        Some("#[expect(clippy::todo, reason = \"<why this site is not a defect>\")]")
    );
    let text = site.text(&root);
    assert!(text.contains("> 2 |     todo!()"), "{text}");
    assert!(text.contains("To suppress this site: #[expect(clippy::todo"), "{text}");

    // A related location covers the line, and the directive above it is
    // shown with its status instead of a directive to write.
    let site = explain_site(&report(), "src/lib.rs:10".parse().unwrap(), &root, &BTreeSet::new());
    assert_eq!(site.hits.len(), 1);
    assert_eq!(site.hits[0].suppressions[0].status, "missing-reason");
    assert!(site.hits[0].directive.is_none());
}

#[test]
fn a_line_with_no_finding_says_whether_it_is_ignored_generated_or_clean() {
    let root = scratch("explain", "why-absent");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/gen.rs"), "// @generated by a tool\npub fn g() {}\n").unwrap();
    let none = BTreeSet::new();
    let absence = |location: &str, declared: &BTreeSet<String>| {
        explain_site(&report(), location.parse().unwrap(), &root, declared).absence
    };
    assert_eq!(absence("vendor/x/lib.rs:1", &none), Some(Absence::Ignored));
    assert_eq!(absence("src/gen.rs:2", &none), Some(Absence::GeneratedOrVendored));
    let declared = BTreeSet::from(["src/marked.rs".to_owned()]);
    assert_eq!(absence("src/marked.rs:1", &declared), Some(Absence::GeneratedOrVendored));
    assert_eq!(absence("src/lib.rs:40", &none), Some(Absence::NothingReported));
    let site = explain_site(&report(), "src/lib.rs:40".parse().unwrap(), &root, &none);
    assert_eq!(site.text(&root), "Nothing is reported at src/lib.rs:40.\n");
}
