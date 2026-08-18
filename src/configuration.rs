use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::Path;

use serde::Deserialize;

use crate::internal_error::InternalError;
use crate::policy::{
    BlockingLevel, CATEGORIES, RuleLevel, find, validate_category_selector, validate_rule_selector,
};
use crate::scan_target::ResolvedScanTarget;
use crate::structure::StructureSettings;

pub(crate) const CONFIG_FILE_NAME: &str = "rust-doctor.toml";
const MAX_CONFIG_BYTES: u64 = 65_536;
const GUARDED_CONFIG_BYTES: u64 = MAX_CONFIG_BYTES + 1;

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct WorkspaceConfiguration {
    pub(crate) file_name: Option<&'static str>,
    pub(crate) blocking: Option<BlockingLevel>,
    pub(crate) categories: BTreeMap<String, RuleLevel>,
    pub(crate) rules: BTreeMap<String, RuleLevel>,
    pub(crate) structure: StructureSettings,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigurationDocument {
    blocking: Option<BlockingLevel>,
    #[serde(default)]
    categories: BTreeMap<String, RuleLevel>,
    #[serde(default)]
    rules: BTreeMap<String, RuleLevel>,
    structure: Option<StructureDocument>,
}

/// The `[structure]` table: thresholds the complexity detector reads. A key
/// left out keeps its shipped default; a key written must be usable, so zero,
/// which would report every function, is refused rather than interpreted.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct StructureDocument {
    cyclomatic_threshold: Option<u32>,
    cognitive_threshold: Option<u32>,
}

pub(crate) fn load(target: &ResolvedScanTarget) -> Result<WorkspaceConfiguration, InternalError> {
    load_path(&target.workspace_root().join(CONFIG_FILE_NAME))
}

fn load_path(path: &Path) -> Result<WorkspaceConfiguration, InternalError> {
    match path.symlink_metadata() {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(WorkspaceConfiguration::default());
        }
        Err(_) => return Err(config_unreadable()),
        Ok(_) => {}
    }

    let metadata = fs::metadata(path).map_err(|_| config_unreadable())?;
    if !metadata.is_file() {
        return Err(InternalError::new(
            "configuration",
            "config-not-file",
            "rust-doctor.toml is not a regular file.",
        ));
    }

    let file = File::open(path).map_err(|_| config_unreadable())?;
    let mut bytes = Vec::new();
    file.take(GUARDED_CONFIG_BYTES)
        .read_to_end(&mut bytes)
        .map_err(|_| config_unreadable())?;
    if bytes.len() as u64 > MAX_CONFIG_BYTES {
        return Err(InternalError::new(
            "configuration",
            "config-too-large",
            "rust-doctor.toml exceeds 65536 bytes.",
        ));
    }

    let document = String::from_utf8(bytes).map_err(|_| {
        InternalError::new(
            "configuration",
            "config-invalid-utf8",
            "rust-doctor.toml must be UTF-8.",
        )
    })?;
    let parsed: ConfigurationDocument = toml::from_str(&document)
        .map_err(|error| invalid_document(&document, error.span().map(|span| span.start)))?;
    validate_document(&parsed)?;

    let mut structure = StructureSettings::default();
    if let Some(thresholds) = &parsed.structure {
        if let Some(threshold) = thresholds.cyclomatic_threshold {
            structure.cyclomatic_threshold = threshold;
        }
        if let Some(threshold) = thresholds.cognitive_threshold {
            structure.cognitive_threshold = threshold;
        }
    }

    Ok(WorkspaceConfiguration {
        file_name: Some(CONFIG_FILE_NAME),
        blocking: parsed.blocking,
        categories: parsed.categories,
        rules: parsed.rules,
        structure,
    })
}

fn validate_document(document: &ConfigurationDocument) -> Result<(), InternalError> {
    for selector in document.rules.keys() {
        if let Err(error) = validate_rule_selector(selector) {
            return Err(configuration_selector_error(error.code));
        }
        if find(selector).is_none() {
            return Err(configuration_selector_error("unknown-rule"));
        }
    }
    for selector in document.categories.keys() {
        if let Err(error) = validate_category_selector(selector) {
            return Err(configuration_selector_error(error.code));
        }
        if CATEGORIES.binary_search(&selector.as_str()).is_err() {
            return Err(configuration_selector_error("unknown-category"));
        }
    }
    if let Some(structure) = &document.structure
        && [structure.cyclomatic_threshold, structure.cognitive_threshold]
            .iter()
            .flatten()
            .any(|threshold| !(1..=10_000).contains(threshold))
    {
        return Err(InternalError::new(
            "configuration",
            "config-invalid-threshold",
            "Structure thresholds in rust-doctor.toml must be between 1 and 10000.",
        ));
    }
    Ok(())
}

fn configuration_selector_error(code: &'static str) -> InternalError {
    let message = match code {
        "invalid-rule-selector" => "Invalid rule selector in rust-doctor.toml.",
        "unknown-rule" => "Unknown rule selector in rust-doctor.toml.",
        "invalid-category-selector" => "Invalid category selector in rust-doctor.toml.",
        "unknown-category" => "Unknown category selector in rust-doctor.toml.",
        _ => "Invalid selector in rust-doctor.toml.",
    };
    InternalError::new("configuration", code, message)
}

fn invalid_document(document: &str, offset: Option<usize>) -> InternalError {
    let message = offset.map_or_else(
        || "Invalid rust-doctor.toml.".to_owned(),
        |offset| {
            let (line, column) = line_column(document, offset);
            format!("Invalid rust-doctor.toml at line {line}, column {column}.")
        },
    );
    InternalError::new("configuration", "config-invalid", message)
}

fn line_column(document: &str, offset: usize) -> (usize, usize) {
    let mut boundary = offset.min(document.len());
    while !document.is_char_boundary(boundary) {
        boundary -= 1;
    }
    let prefix = &document[..boundary];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let column = prefix
        .rsplit('\n')
        .next()
        .map_or(1, |tail| tail.chars().count() + 1);
    (line, column)
}

fn config_unreadable() -> InternalError {
    InternalError::new(
        "configuration",
        "config-unreadable",
        "Could not read rust-doctor.toml.",
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::symlink;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::SystemTime;

    use crate::permutations::next_permutation;
    use crate::policy::{PolicyInput, PolicyPlan};

    use super::*;

    static NEXT_CASE: AtomicUsize = AtomicUsize::new(0);

    fn temporary_path() -> std::path::PathBuf {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/configuration-kernel-loader")
            .join(format!(
                "{}-{}",
                std::process::id(),
                NEXT_CASE.fetch_add(1, Ordering::Relaxed)
            ));
        fs::create_dir_all(&path).unwrap();
        path.join(CONFIG_FILE_NAME)
    }

    fn write(contents: &[u8]) -> std::path::PathBuf {
        let path = temporary_path();
        fs::write(&path, contents).unwrap();
        path
    }

    fn error(contents: &[u8]) -> InternalError {
        load_path(&write(contents)).unwrap_err()
    }

    #[test]
    fn absent_empty_comments_and_complete_document_are_distinct_and_sorted() {
        let absent = load_path(&temporary_path()).unwrap();
        assert_eq!(absent, WorkspaceConfiguration::default());

        for contents in [b"".as_slice(), b"# comment\n".as_slice()] {
            let loaded = load_path(&write(contents)).unwrap();
            assert_eq!(loaded.file_name, Some(CONFIG_FILE_NAME));
            assert!(loaded.blocking.is_none());
            assert!(loaded.categories.is_empty());
            assert!(loaded.rules.is_empty());
        }

        let loaded = load_path(&write(
            br#"blocking = "warning"

[categories]
security = "error"
correctness = "off"

[rules]
"rust_doctor::source::dynamic_shell_command" = "error"
"clippy::todo" = "off"
"#,
        ))
        .unwrap();
        assert_eq!(loaded.blocking, Some(BlockingLevel::Warning));
        assert_eq!(
            loaded
                .categories
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["correctness", "security"]
        );
        assert_eq!(
            loaded.rules.keys().map(String::as_str).collect::<Vec<_>>(),
            ["clippy::todo", "rust_doctor::source::dynamic_shell_command"]
        );
    }

    #[test]
    fn size_and_utf8_boundaries_fail_before_parsing() {
        let valid = vec![b' '; MAX_CONFIG_BYTES as usize];
        assert!(load_path(&write(&valid)).is_ok());

        let too_large = vec![b' '; GUARDED_CONFIG_BYTES as usize];
        let too_large = error(&too_large);
        assert_eq!(
            (too_large.stage, too_large.code),
            ("configuration", "config-too-large")
        );

        let invalid_utf8 = error(&[0xff]);
        assert_eq!(
            (invalid_utf8.stage, invalid_utf8.code),
            ("configuration", "config-invalid-utf8")
        );
    }

    #[test]
    fn toml_and_serde_reject_closed_schema_cases_atomically() {
        for contents in [
            "blocking = \"error\"\nblocking = \"none\"\n",
            "[rules]\n[ rules ]\n",
            "unknown = \"warn\"\n",
            "[unknown]\nvalue = \"warn\"\n",
            "[rules.\"clippy::todo\"]\nunknown = \"warn\"\n",
            "blocking = \"warn\"\n",
            "[rules]\n\"clippy::todo\" = \"warning\"\n",
        ] {
            let failure = error(contents.as_bytes());
            assert_eq!(
                (failure.stage, failure.code),
                ("configuration", "config-invalid")
            );
            assert!(failure.message.starts_with("Invalid rust-doctor.toml"));
            assert!(!failure.message.contains(contents.trim()));
        }
    }

    #[test]
    fn selector_errors_are_closed_and_never_echo_hostile_keys() {
        let cases = [
            ("[rules]\n\"UNKNOWN\" = \"warn\"\n", "invalid-rule-selector"),
            ("[rules]\n\"clippy::unknown\" = \"warn\"\n", "unknown-rule"),
            (
                "[categories]\n\"bad-key\" = \"warn\"\n",
                "invalid-category-selector",
            ),
            ("[categories]\nunknown = \"warn\"\n", "unknown-category"),
        ];
        for (contents, expected_code) in cases {
            let failure = error(contents.as_bytes());
            assert_eq!(
                (failure.stage, failure.code),
                ("configuration", expected_code)
            );
            assert!(!failure.message.contains(contents.trim()));
        }

        let hostile =
            "[rules]\n\"/home/private https://secret credential=token \\u001b[31m\" = \"warn\"\n";
        let failure = error(hostile.as_bytes());
        assert_eq!(failure.code, "invalid-rule-selector");
        for sentinel in ["/home/private", "https://", "credential=", "\u{1b}"] {
            assert!(!failure.message.contains(sentinel));
        }
    }

    #[test]
    fn parser_position_is_bounded_for_multiline_utf8() {
        let document = "# précision\nblocking = \"invalid\"\n";
        let failure = error(document.as_bytes());
        assert_eq!(failure.code, "config-invalid");
        assert!(failure.message.contains("line 2, column"));
        assert!(!failure.message.contains("précision"));
        assert!(!failure.message.contains("invalid"));
    }

    #[cfg(unix)]
    #[test]
    fn non_file_and_unreadable_targets_have_distinct_codes() {
        let directory = temporary_path();
        fs::create_dir(&directory).unwrap();
        let not_file = load_path(&directory).unwrap_err();
        assert_eq!(not_file.code, "config-not-file");

        let dangling = temporary_path();
        symlink("missing", &dangling).unwrap();
        let unreadable = load_path(&dangling).unwrap_err();
        assert_eq!(unreadable.code, "config-unreadable");
    }

    #[test]
    fn loading_preserves_content_size_and_mtime() {
        let path = write(b"blocking = \"error\"\n");
        let before_contents = fs::read(&path).unwrap();
        let before = fs::metadata(&path).unwrap();
        let before_modified = before.modified().unwrap_or(SystemTime::UNIX_EPOCH);

        let loaded = load_path(&path).unwrap();

        let after = fs::metadata(&path).unwrap();
        assert_eq!(loaded.file_name, Some(CONFIG_FILE_NAME));
        assert_eq!(fs::read(&path).unwrap(), before_contents);
        assert_eq!(after.len(), before.len());
        assert_eq!(
            after.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            before_modified
        );
    }

    #[test]
    fn twenty_mixed_toml_and_request_orders_serialize_to_one_plan() {
        #[derive(Clone, Copy)]
        enum RequestOverride {
            Rule(&'static str, RuleLevel),
            Category(&'static str, RuleLevel),
        }

        let entries = [
            "blocking = \"warning\"",
            "categories.security = \"off\"",
            "categories.correctness = \"error\"",
            "rules.\"clippy::todo\" = \"warn\"",
            "rules.\"rust_doctor::source::dynamic_shell_command\" = \"error\"",
        ];
        let overrides = [
            RequestOverride::Category("correctness", RuleLevel::Warn),
            RequestOverride::Category("security", RuleLevel::Error),
            RequestOverride::Rule("clippy::todo", RuleLevel::Off),
            RequestOverride::Rule(
                "rust_doctor::source::dynamic_shell_command",
                RuleLevel::Warn,
            ),
        ];
        let mut config_order = [0, 1, 2, 3, 4];
        let mut request_order = [0, 1, 2, 3];
        let mut config_orders = BTreeSet::new();
        let mut request_orders = BTreeSet::new();
        // The plan is compared as a plan. Comparing a serialization of it would
        // ask the private type to carry a published shape for one assertion.
        let mut expected_plan = None;

        for _ in 0..20 {
            assert!(config_orders.insert(config_order));
            assert!(request_orders.insert(request_order));
            let document = config_order
                .iter()
                .map(|&index| entries[index])
                .collect::<Vec<_>>()
                .join("\n")
                + "\n";
            let configuration = load_path(&write(document.as_bytes())).unwrap();
            let mut request = PolicyInput::default();
            for index in request_order {
                request = match overrides[index] {
                    RequestOverride::Rule(selector, level) => request.with_rule(selector, level),
                    RequestOverride::Category(selector, level) => {
                        request.with_category(selector, level)
                    }
                };
            }
            let validated = request.validate().unwrap();
            let plan = PolicyPlan::compile_with_configuration(&validated, &configuration);
            match &expected_plan {
                Some(expected) => assert_eq!(&plan, expected),
                None => expected_plan = Some(plan),
            }

            assert!(next_permutation(&mut config_order));
            assert!(next_permutation(&mut request_order));
        }

        assert_eq!(config_orders.len(), 20);
        assert_eq!(request_orders.len(), 20);
        assert!(expected_plan.is_some());
    }
}
