use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use clap::ValueEnum;
use serde::{Deserialize, Serialize};

use crate::configuration::WorkspaceConfiguration;

mod catalog;
#[cfg(test)]
mod coverage;

pub use catalog::RuleTier;
use catalog::find_in;
pub(crate) use catalog::{
    CARGO_DUPLICATE_MAJOR_VERSIONS, CARGO_MISSING_LOCKFILE,
    CARGO_PATH_DEPENDENCY_OUTSIDE_WORKSPACE, CARGO_UNBOUNDED_REGISTRY, CARGO_UNPINNED_GIT, CATALOG,
    CATEGORIES, Producer, RuleDefinition, SOURCE_DISABLED_TLS, SOURCE_DYNAMIC_SHELL,
    STRUCTURE_COMPLEX_FUNCTION, STRUCTURE_DUPLICATE_FUNCTION_BODY,
    STRUCTURE_NEAR_DUPLICATE_FUNCTION_BODY, STRUCTURE_ORPHAN_MODULE_FILE, STRUCTURE_OVERSIZED_UNIT,
    STRUCTURE_UNREASONED_ALLOW, STRUCTURE_UNREFERENCED_FEATURE, find,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum RuleLevel {
    Off,
    Warn,
    Error,
}

impl RuleLevel {
    pub(crate) const fn is_active(self) -> bool {
        !matches!(self, Self::Off)
    }

    pub(crate) const fn clippy_flag(self) -> Option<&'static str> {
        match self {
            Self::Off => None,
            Self::Warn | Self::Error => Some("-W"),
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}

impl fmt::Display for RuleLevel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum BlockingLevel {
    None,
    #[default]
    Error,
    Warning,
}

impl BlockingLevel {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Error => "error",
            Self::Warning => "warning",
        }
    }
}

impl fmt::Display for BlockingLevel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleOverride {
    selector: String,
    level: RuleLevel,
}

impl RuleOverride {
    pub fn new(selector: impl Into<String>, level: RuleLevel) -> Self {
        Self {
            selector: selector.into(),
            level,
        }
    }
}

impl fmt::Display for RuleOverride {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}={}", self.selector, self.level)
    }
}

impl FromStr for RuleOverride {
    type Err = &'static str;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (selector, level) = parse_override(value)?;
        Ok(Self::new(selector, level))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CategoryOverride {
    selector: String,
    level: RuleLevel,
}

impl CategoryOverride {
    pub fn new(selector: impl Into<String>, level: RuleLevel) -> Self {
        Self {
            selector: selector.into(),
            level,
        }
    }
}

impl fmt::Display for CategoryOverride {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}={}", self.selector, self.level)
    }
}

impl FromStr for CategoryOverride {
    type Err = &'static str;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (selector, level) = parse_override(value)?;
        Ok(Self::new(selector, level))
    }
}

fn parse_override(value: &str) -> Result<(&str, RuleLevel), &'static str> {
    let (selector, level) = value
        .split_once('=')
        .ok_or("expected KEY=LEVEL with LEVEL one of: off, warn, error")?;
    if selector.is_empty() {
        return Err("KEY must not be empty; LEVEL must be one of: off, warn, error");
    }
    let level = match level {
        "off" => RuleLevel::Off,
        "warn" => RuleLevel::Warn,
        "error" => RuleLevel::Error,
        _ => return Err("LEVEL must be one of: off, warn, error"),
    };
    Ok((selector, level))
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PolicyInput {
    rule_overrides: Vec<RuleOverride>,
    category_overrides: Vec<CategoryOverride>,
    blocking: Option<BlockingLevel>,
}

impl PolicyInput {
    #[cfg(test)]
    pub(crate) fn with_rule(mut self, selector: impl Into<String>, level: RuleLevel) -> Self {
        self.rule_overrides.push(RuleOverride::new(selector, level));
        self
    }

    #[cfg(test)]
    pub(crate) fn with_category(mut self, selector: impl Into<String>, level: RuleLevel) -> Self {
        self.category_overrides
            .push(CategoryOverride::new(selector, level));
        self
    }

    pub(crate) fn with_blocking(mut self, blocking: BlockingLevel) -> Self {
        self.blocking = Some(blocking);
        self
    }

    pub(crate) fn push_rule(&mut self, rule_override: RuleOverride) {
        self.rule_overrides.push(rule_override);
    }

    pub(crate) fn push_category(&mut self, category_override: CategoryOverride) {
        self.category_overrides.push(category_override);
    }

    pub(crate) fn failure_blocking(&self) -> BlockingLevel {
        self.blocking.unwrap_or_default()
    }

    pub(crate) fn validate(&self) -> Result<(), PolicyError> {
        validated_overrides(self).map(|_| ())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RuleLevelSource {
    Default,
    ConfigCategory,
    ConfigRule,
    RequestCategory,
    RequestRule,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BlockingLevelSource {
    Default,
    Config,
    Request,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
struct PlannedRule {
    definition: &'static RuleDefinition,
    level: RuleLevel,
    source: RuleLevelSource,
    restamp: bool,
}

/// `serde` only implements `Serialize` up to arrays of 32 elements. The plan
/// carries as many as the catalog does, so serialization goes through the
/// slice: same published shape, with no bound on the catalog size.
fn serialize_planned_rules<S>(
    rules: &[PlannedRule; CATALOG.len()],
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    rules.as_slice().serialize(serializer)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct PolicyPlan {
    #[serde(serialize_with = "serialize_planned_rules")]
    rules: [PlannedRule; CATALOG.len()],
    blocking: BlockingLevel,
    blocking_source: BlockingLevelSource,
    config_file: Option<&'static str>,
}

impl Default for PolicyPlan {
    fn default() -> Self {
        Self {
            rules: CATALOG.map(|definition| PlannedRule {
                definition,
                level: definition.default_level,
                source: RuleLevelSource::Default,
                restamp: false,
            }),
            blocking: BlockingLevel::default(),
            blocking_source: BlockingLevelSource::Default,
            config_file: None,
        }
    }
}

impl PolicyPlan {
    #[cfg(test)]
    pub(crate) fn compile(input: &PolicyInput) -> Result<Self, PolicyError> {
        Self::compile_with_configuration(input, &WorkspaceConfiguration::default())
    }

    pub(crate) fn compile_with_configuration(
        input: &PolicyInput,
        configuration: &WorkspaceConfiguration,
    ) -> Result<Self, PolicyError> {
        let rules = compile_rules(&CATALOG, input, configuration)?;
        let (blocking, blocking_source) = if let Some(blocking) = input.blocking {
            (blocking, BlockingLevelSource::Request)
        } else if let Some(blocking) = configuration.blocking {
            (blocking, BlockingLevelSource::Config)
        } else {
            (BlockingLevel::default(), BlockingLevelSource::Default)
        };
        Ok(Self {
            rules,
            blocking,
            blocking_source,
            config_file: configuration.file_name,
        })
    }

    pub(crate) fn level(&self, id: &str) -> Option<RuleLevel> {
        self.rules
            .binary_search_by_key(&id, |rule| rule.definition.id)
            .ok()
            .map(|index| self.rules[index].level)
    }

    pub(crate) fn is_active(&self, id: &str) -> bool {
        self.level(id).is_some_and(RuleLevel::is_active)
    }

    pub(crate) fn restamp_level(&self, id: &str) -> Option<RuleLevel> {
        self.rules
            .binary_search_by_key(&id, |rule| rule.definition.id)
            .ok()
            .and_then(|index| self.rules[index].restamp.then_some(self.rules[index].level))
    }

    pub(crate) fn active_rules(
        &self,
        producer: Producer,
    ) -> impl Iterator<Item = (&'static RuleDefinition, RuleLevel)> + '_ {
        active_rules_in(&self.rules, producer)
    }

    pub(crate) const fn blocking(&self) -> BlockingLevel {
        self.blocking
    }

    pub(crate) const fn blocking_source(&self) -> BlockingLevelSource {
        self.blocking_source
    }

    pub(crate) const fn config_file(&self) -> Option<&'static str> {
        self.config_file
    }

    pub(crate) fn effective_rules(
        &self,
    ) -> impl Iterator<Item = (&'static RuleDefinition, RuleLevel, RuleLevelSource)> + '_ {
        self.rules
            .iter()
            .map(|planned| (planned.definition, planned.level, planned.source))
    }
}

fn active_rules_in<const N: usize>(
    rules: &[PlannedRule; N],
    producer: Producer,
) -> impl Iterator<Item = (&'static RuleDefinition, RuleLevel)> + '_ {
    rules.iter().filter_map(move |planned| {
        (planned.definition.producer == producer && planned.level.is_active())
            .then_some((planned.definition, planned.level))
    })
}

fn compile_rules<const N: usize>(
    catalog: &[&'static RuleDefinition; N],
    input: &PolicyInput,
    configuration: &WorkspaceConfiguration,
) -> Result<[PlannedRule; N], PolicyError> {
    let (rule_overrides, category_overrides) = validated_overrides_in(input, catalog)?;
    Ok(catalog.map(|definition| {
        let (level, source) = if let Some(level) = rule_overrides.get(definition.id).copied() {
            (level, RuleLevelSource::RequestRule)
        } else if let Some(level) = category_overrides.get(definition.category).copied() {
            (level, RuleLevelSource::RequestCategory)
        } else if let Some(level) = configuration.rules.get(definition.id).copied() {
            (level, RuleLevelSource::ConfigRule)
        } else if let Some(level) = configuration.categories.get(definition.category).copied() {
            (level, RuleLevelSource::ConfigCategory)
        } else {
            (definition.default_level, RuleLevelSource::Default)
        };
        PlannedRule {
            definition,
            level,
            source,
            restamp: source != RuleLevelSource::Default,
        }
    }))
}

type ValidatedOverrides<'a> = (BTreeMap<&'a str, RuleLevel>, BTreeMap<&'a str, RuleLevel>);

fn validated_overrides(input: &PolicyInput) -> Result<ValidatedOverrides<'_>, PolicyError> {
    validated_overrides_in(input, &CATALOG)
}

fn validated_overrides_in<'a>(
    input: &'a PolicyInput,
    catalog: &[&RuleDefinition],
) -> Result<ValidatedOverrides<'a>, PolicyError> {
    let mut rule_overrides = BTreeMap::new();
    for rule_override in &input.rule_overrides {
        validate_rule_selector(&rule_override.selector)?;
        if find_in(catalog, &rule_override.selector).is_none() {
            return Err(PolicyError::unknown_rule());
        }
        if rule_overrides
            .insert(rule_override.selector.as_str(), rule_override.level)
            .is_some()
        {
            return Err(PolicyError::duplicate_rule());
        }
    }

    let mut category_overrides = BTreeMap::new();
    for category_override in &input.category_overrides {
        validate_category_selector(&category_override.selector)?;
        if CATEGORIES
            .binary_search(&category_override.selector.as_str())
            .is_err()
        {
            return Err(PolicyError::unknown_category());
        }
        if category_overrides
            .insert(category_override.selector.as_str(), category_override.level)
            .is_some()
        {
            return Err(PolicyError::duplicate_category());
        }
    }

    Ok((rule_overrides, category_overrides))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PolicyError {
    pub(crate) code: &'static str,
    pub(crate) message: &'static str,
}

impl PolicyError {
    const fn invalid_rule() -> Self {
        Self {
            code: "invalid-rule-selector",
            message: "Invalid rule selector.",
        }
    }

    const fn unknown_rule() -> Self {
        Self {
            code: "unknown-rule",
            message: "Unknown rule selector.",
        }
    }

    const fn duplicate_rule() -> Self {
        Self {
            code: "duplicate-rule-override",
            message: "Duplicate rule override.",
        }
    }

    const fn invalid_category() -> Self {
        Self {
            code: "invalid-category-selector",
            message: "Invalid category selector.",
        }
    }

    const fn unknown_category() -> Self {
        Self {
            code: "unknown-category",
            message: "Unknown category selector.",
        }
    }

    const fn duplicate_category() -> Self {
        Self {
            code: "duplicate-category-override",
            message: "Duplicate category override.",
        }
    }
}

pub(crate) fn validate_rule_selector(selector: &str) -> Result<(), PolicyError> {
    if !(1..=128).contains(&selector.len())
        || !selector.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b':')
        })
    {
        return Err(PolicyError::invalid_rule());
    }
    Ok(())
}

pub(crate) fn validate_category_selector(selector: &str) -> Result<(), PolicyError> {
    if !(1..=32).contains(&selector.len())
        || !selector.bytes().all(|byte| byte.is_ascii_lowercase())
    {
        return Err(PolicyError::invalid_category());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use serde_json::Value;

    fn oracle() -> Value {
        serde_json::from_str(include_str!("../tests/fixtures/policy-gate/oracle.json"))
            .expect("policy oracle should be valid JSON")
    }

    #[test]
    fn oracle_pins_toolchain_levels_and_exit_contract() {
        let oracle = oracle();
        assert_eq!(
            oracle["toolchain"]["rustc"],
            "rustc 1.97.1 (8bab26f4f 2026-07-14)"
        );
        assert_eq!(
            oracle["toolchain"]["cargo"],
            "cargo 1.97.1 (c980f4866 2026-06-30)"
        );
        assert_eq!(
            oracle["toolchain"]["clippy"],
            "clippy 0.1.97 (8bab26f4f6 2026-07-14)"
        );
        assert_eq!(oracle["toolchain"]["clap"], "4.6.4");

        for value in ["off", "warn", "error"] {
            assert!(RuleLevel::from_str(value, false).is_ok(), "{value}");
        }
        for value in ["none", "error", "warning"] {
            assert!(BlockingLevel::from_str(value, false).is_ok(), "{value}");
        }
        for value in ["allow", "warning", "deny", "forbid", "ERROR"] {
            assert!(RuleLevel::from_str(value, false).is_err(), "{value}");
        }
        for value in ["off", "warn", "info", "ERROR"] {
            assert!(BlockingLevel::from_str(value, false).is_err(), "{value}");
        }

        let exits = oracle["exit_contract"]
            .as_array()
            .expect("exit contract should be an array");
        assert_eq!(exits.len(), 5);
        let distinct: BTreeSet<_> = exits.iter().map(Value::to_string).collect();
        assert_eq!(distinct.len(), 5);
        assert_eq!(exits[0]["scan_status"], "complete");
        assert_eq!(exits[1]["scan_status"], "complete");
        assert_eq!(exits[0]["complete"], true);
        assert_eq!(exits[1]["complete"], true);
    }

    #[test]
    fn default_category_and_rule_precedence_are_closed_and_order_independent() {
        let default =
            PolicyPlan::compile(&PolicyInput::default()).expect("default policy should compile");
        assert_eq!(default.blocking(), BlockingLevel::Error);
        assert!(
            CATALOG
                .iter()
                .all(|definition| default.level(definition.id) == Some(RuleLevel::Warn))
        );

        let category = PolicyInput::default().with_category("security", RuleLevel::Off);
        let category = PolicyPlan::compile(&category).expect("category policy should compile");
        assert_eq!(category.level("clippy::todo"), Some(RuleLevel::Warn));
        assert_eq!(
            category.level("rust_doctor::cargo::unpinned_git_dependency"),
            Some(RuleLevel::Off)
        );

        let input = PolicyInput::default()
            .with_rule(
                "rust_doctor::source::dynamic_shell_command",
                RuleLevel::Error,
            )
            .with_category("security", RuleLevel::Off)
            .with_blocking(BlockingLevel::Warning);
        let plan = PolicyPlan::compile(&input).expect("mixed policy should compile");
        assert_eq!(
            plan.level("rust_doctor::source::dynamic_shell_command"),
            Some(RuleLevel::Error)
        );
        assert_eq!(
            plan.level("rust_doctor::source::disabled_tls_verification"),
            Some(RuleLevel::Off)
        );
        assert_eq!(plan.blocking(), BlockingLevel::Warning);
    }

    #[test]
    fn configuration_and_request_layers_have_closed_precedence_and_provenance() {
        let configuration = WorkspaceConfiguration {
            file_name: Some("rust-doctor.toml"),
            blocking: Some(BlockingLevel::Warning),
            categories: BTreeMap::from([
                ("correctness".to_owned(), RuleLevel::Off),
                ("security".to_owned(), RuleLevel::Off),
            ]),
            rules: BTreeMap::from([
                ("clippy::todo".to_owned(), RuleLevel::Error),
                (
                    "rust_doctor::source::dynamic_shell_command".to_owned(),
                    RuleLevel::Error,
                ),
            ]),
            structure: crate::structure::StructureSettings::default(),
        };
        let request = PolicyInput::default()
            .with_category("correctness", RuleLevel::Warn)
            .with_rule(
                "rust_doctor::source::dynamic_shell_command",
                RuleLevel::Warn,
            );
        let plan = PolicyPlan::compile_with_configuration(&request, &configuration)
            .expect("layered policy should compile");
        let rules: BTreeMap<_, _> = plan
            .effective_rules()
            .map(|(definition, level, source)| (definition.id, (level, source)))
            .collect();

        assert_eq!(plan.config_file(), Some("rust-doctor.toml"));
        assert_eq!(plan.blocking(), BlockingLevel::Warning);
        assert_eq!(plan.blocking_source(), BlockingLevelSource::Config);
        assert_eq!(
            rules["clippy::todo"],
            (RuleLevel::Warn, RuleLevelSource::RequestCategory)
        );
        assert_eq!(
            rules["clippy::unimplemented"],
            (RuleLevel::Warn, RuleLevelSource::RequestCategory)
        );
        assert_eq!(
            rules["rust_doctor::source::dynamic_shell_command"],
            (RuleLevel::Warn, RuleLevelSource::RequestRule)
        );
        assert_eq!(
            rules["rust_doctor::source::disabled_tls_verification"],
            (RuleLevel::Off, RuleLevelSource::ConfigCategory)
        );
        assert_eq!(
            rules["clippy::dbg_macro"],
            (RuleLevel::Warn, RuleLevelSource::Default)
        );

        let explicit = PolicyInput::default().with_blocking(BlockingLevel::None);
        let explicit = PolicyPlan::compile_with_configuration(&explicit, &configuration)
            .expect("request blocking should compile");
        assert_eq!(explicit.blocking(), BlockingLevel::None);
        assert_eq!(explicit.blocking_source(), BlockingLevelSource::Request);
    }

    #[test]
    fn duplicate_unknown_and_hostile_selectors_use_closed_errors_without_echoing_input() {
        let cases = [
            (
                PolicyInput::default()
                    .with_rule("clippy::todo", RuleLevel::Warn)
                    .with_rule("clippy::todo", RuleLevel::Error),
                "duplicate-rule-override",
            ),
            (
                PolicyInput::default()
                    .with_category("security", RuleLevel::Warn)
                    .with_category("security", RuleLevel::Off),
                "duplicate-category-override",
            ),
            (
                PolicyInput::default().with_rule("unknown::rule", RuleLevel::Warn),
                "unknown-rule",
            ),
            (
                PolicyInput::default().with_category("style", RuleLevel::Warn),
                "unknown-category",
            ),
            (
                PolicyInput::default().with_rule("bad/\u{001b}[31mselector", RuleLevel::Warn),
                "invalid-rule-selector",
            ),
            (
                PolicyInput::default().with_category("", RuleLevel::Warn),
                "invalid-category-selector",
            ),
            (
                PolicyInput::default().with_rule("a".repeat(129), RuleLevel::Warn),
                "invalid-rule-selector",
            ),
            (
                PolicyInput::default().with_category("a".repeat(33), RuleLevel::Warn),
                "invalid-category-selector",
            ),
        ];

        for (input, code) in cases {
            let error = PolicyPlan::compile(&input).expect_err("invalid policy should fail");
            assert_eq!(error.code, code);
            assert!(!error.message.contains('\u{001b}'));
            assert!(!error.message.contains('/'));
            assert!(error.message.len() < 64);
        }
    }

    #[test]
    fn twenty_override_orders_serialize_to_the_same_plan() {
        #[derive(Clone, Copy)]
        enum Override {
            Rule(&'static str, RuleLevel),
            Category(&'static str, RuleLevel),
        }

        let overrides = [
            Override::Category("security", RuleLevel::Off),
            Override::Category("correctness", RuleLevel::Error),
            Override::Rule(
                "rust_doctor::source::dynamic_shell_command",
                RuleLevel::Error,
            ),
            Override::Rule("clippy::todo", RuleLevel::Warn),
        ];
        let mut expected_plan = None;
        let mut orders = BTreeSet::new();
        let mut order = [0, 1, 2, 3];
        for _ in 0..20 {
            assert!(orders.insert(order));
            let mut input = PolicyInput::default();
            for index in order {
                input = match overrides[index] {
                    Override::Rule(selector, level) => input.with_rule(selector, level),
                    Override::Category(selector, level) => input.with_category(selector, level),
                };
            }
            let plan = PolicyPlan::compile(&input).expect("permuted policy should compile");
            if let Some(expected) = &expected_plan {
                assert_eq!(&plan, expected);
            } else {
                expected_plan = Some(plan);
            }

            let pivot = (0..order.len() - 1)
                .rev()
                .find(|&index| order[index] < order[index + 1])
                .expect("twenty permutations are below the full permutation count");
            let successor = (pivot + 1..order.len())
                .rev()
                .find(|&index| order[pivot] < order[index])
                .expect("permutation successor should exist");
            order.swap(pivot, successor);
            order[pivot + 1..].reverse();
        }
        assert_eq!(orders.len(), 20);
    }
}
