// Integration test crates are not covered by clippy.toml allow-in-tests settings.
#![allow(clippy::unwrap_used)]

use rust_doctor::api::{
    BatchProjectRequest, BatchScanRequest, CacheInvalidation, CancellationToken, ScanApiError,
    ScanRequest, invalidate_cache, scan, scan_batch,
};
use rust_doctor::config::{AdapterPolicy, RuleConfig, RuleLevel};
use std::fs;

fn project() -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    fs::create_dir(directory.path().join("src")).unwrap();
    fs::write(
        directory.path().join("Cargo.toml"),
        "[package]\nname='api-fixture'\nversion='0.1.0'\nedition='2024'\nrust-version='1.97'\n",
    )
    .unwrap();
    fs::write(
        directory.path().join("src/lib.rs"),
        "pub fn value() -> u8 { 1 }\n",
    )
    .unwrap();
    directory
}

#[test]
fn public_api_returns_report_v1_with_typed_overrides_and_no_network() {
    let directory = project();
    let requested_root = directory.path().join("src");
    let mut request = ScanRequest::new(&requested_root);
    request.options.adapters = AdapterPolicy::none();
    request.options.config_overrides.rules.insert(
        "unwrap-in-production".to_string(),
        RuleConfig {
            severity: Some(RuleLevel::Error),
            ..RuleConfig::default()
        },
    );

    let report = scan(request).unwrap();
    assert_eq!(report.schema_version, "1.0");
    assert!(report.report_constructed);
    assert_eq!(report.requested_root, requested_root.to_string_lossy());
    assert_eq!(report.resolved_root.as_deref(), directory.path().to_str());
}

#[test]
fn batch_preserves_order_and_success_when_one_project_fails() {
    let directory = project();
    let missing = directory.path().join("missing");
    let options = rust_doctor::api::ScanOptions {
        adapters: AdapterPolicy::none(),
        ..rust_doctor::api::ScanOptions::default()
    };
    let result = scan_batch(BatchScanRequest {
        projects: vec![
            BatchProjectRequest::new(directory.path()),
            BatchProjectRequest::new(&missing),
        ],
        options,
        max_parallelism: 2,
    })
    .unwrap();

    assert_eq!(result.aggregate.projects, 2);
    assert_eq!(result.aggregate.succeeded, 1);
    assert_eq!(result.aggregate.failed, 1);
    assert_eq!(result.projects[0].requested_root, directory.path());
    assert!(result.projects[0].result.is_ok());
    assert_eq!(result.projects[1].requested_root, missing);
    assert!(result.projects[1].result.is_err());
}

#[test]
fn invalid_policy_is_typed_and_cache_invalidation_is_scoped() {
    let directory = project();
    let mut request = ScanRequest::new(directory.path());
    request.options.adapters = AdapterPolicy::none();
    request.options.config_overrides.rules.insert(
        "not-a-rule".to_string(),
        RuleConfig {
            severity: Some(RuleLevel::Warning),
            ..RuleConfig::default()
        },
    );
    assert!(matches!(scan(request), Err(ScanApiError::Config(_))));

    fs::write(directory.path().join(".rust-doctor-cache.json"), "{}").unwrap();
    assert_eq!(
        invalidate_cache(&directory.path().join("src")).unwrap(),
        CacheInvalidation::Removed
    );
    assert_eq!(
        invalidate_cache(directory.path()).unwrap(),
        CacheInvalidation::NotPresent
    );
}

#[test]
fn cancelled_batch_does_not_schedule_projects() {
    let directory = project();
    let cancellation = CancellationToken::default();
    cancellation.cancel();
    let options = rust_doctor::api::ScanOptions {
        adapters: AdapterPolicy::none(),
        cancellation,
        ..rust_doctor::api::ScanOptions::default()
    };
    let result = scan_batch(BatchScanRequest {
        projects: vec![
            BatchProjectRequest::new(directory.path()),
            BatchProjectRequest::new(directory.path()),
        ],
        options,
        max_parallelism: 1,
    })
    .unwrap();

    assert_eq!(result.aggregate.succeeded, 0);
    assert_eq!(result.aggregate.failed, 2);
    assert!(!result.aggregate.all_successes_complete);
    assert!(
        result
            .projects
            .iter()
            .all(|project| matches!(project.result, Err(ScanApiError::Cancelled)))
    );
}
