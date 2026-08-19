use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::str::FromStr;

use clap::ValueEnum;
use serde::{Deserialize, Serialize};

use crate::configuration::WorkspaceConfiguration;

mod catalog;
#[cfg(test)]
mod coverage;
mod noise;

pub use catalog::RuleTier;
pub use catalog::{CatalogEntry, catalog};
use catalog::find_in;
pub(crate) use catalog::{
    CARGO_DUPLICATE_MAJOR_VERSIONS, CARGO_MISSING_LOCKFILE,
    CARGO_PATH_DEPENDENCY_OUTSIDE_WORKSPACE, CARGO_PERMISSIVE_LINT_TABLE,
    CARGO_PERMISSIVE_RUSTFLAGS, CARGO_RELEASE_DEBUG_SYMBOLS, CARGO_TEST_ONLY_DEPENDENCY,
    CARGO_UNBOUNDED_REGISTRY, CARGO_UNCHECKED_RELEASE_OVERFLOW, CARGO_UNPINNED_GIT,
    CARGO_UNUSED_DEPENDENCY, CATALOG, CATEGORIES, Producer, REPO_HARDCODED_CREDENTIAL,
    REPO_TRACKED_SECRET_FILE, REPO_UNIGNORED_BUILD_OUTPUT, RuleDefinition, SOURCE_DISABLED_TLS,
    SOURCE_DYNAMIC_SHELL, STRUCTURE_COMPLEX_FUNCTION, STRUCTURE_CRATE_LEVEL_ALLOW,
    STRUCTURE_DUPLICATE_FUNCTION_BODY, STRUCTURE_NEAR_DUPLICATE_FUNCTION_BODY,
    STRUCTURE_ORPHAN_MODULE_FILE, STRUCTURE_OVERSIZED_UNIT, STRUCTURE_STACKED_ALLOW,
    STRUCTURE_UNREASONED_ALLOW, STRUCTURE_UNREFERENCED_FEATURE, find,
};
pub(crate) use noise::corpus_noise;

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

/// The two override kinds are the same pair, a selector and a level, read from
/// the same `KEY=LEVEL` syntax and rendered back the same way. They stay
/// distinct types so a category selector cannot reach `with_rule_override`, and
/// they share one body so their parsing, their rendering and their shape cannot
/// drift apart.
macro_rules! selector_override {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub struct $name {
            selector: String,
            level: RuleLevel,
        }

        impl $name {
            pub fn new(selector: impl Into<String>, level: RuleLevel) -> Self {
                Self {
                    selector: selector.into(),
                    level,
                }
            }

            /// The pair validation reads, so the two kinds go through one loop.
            fn parts(&self) -> (&str, RuleLevel) {
                (&self.selector, self.level)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "{}={}", self.selector, self.level)
            }
        }

        impl FromStr for $name {
            type Err = &'static str;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                let (selector, level) = parse_override(value)?;
                Ok(Self::new(selector, level))
            }
        }
    };
}

selector_override!(RuleOverride);
selector_override!(CategoryOverride);

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

    /// Reads the overrides once, against the shipped catalog. What comes back
    /// is the only thing a plan compiles from, so a plan cannot be built out of
    /// overrides nobody validated and validation cannot run twice.
    pub(crate) fn validate(&self) -> Result<ValidatedPolicy<'_>, PolicyError> {
        ValidatedPolicy::of(self, &CATALOG)
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PlannedRule {
    definition: &'static RuleDefinition,
    level: RuleLevel,
    source: RuleLevelSource,
}

impl PlannedRule {
    /// A level the reader chose, at the request or through a configuration
    /// file, rather than the one the catalog ships. It is read from the source
    /// instead of being stored beside it: a second field for the same fact is
    /// a second field to keep true.
    const fn restamped(&self) -> bool {
        !matches!(self.source, RuleLevelSource::Default)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PolicyPlan {
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
        Ok(Self::compile_with_configuration(
            &input.validate()?,
            &WorkspaceConfiguration::default(),
        ))
    }

    pub(crate) fn compile_with_configuration(
        policy: &ValidatedPolicy<'_>,
        configuration: &WorkspaceConfiguration,
    ) -> Self {
        let rules = compile_rules(&CATALOG, policy, configuration);
        let (blocking, blocking_source) = if let Some(blocking) = policy.input.blocking {
            (blocking, BlockingLevelSource::Request)
        } else if let Some(blocking) = configuration.blocking {
            (blocking, BlockingLevelSource::Config)
        } else {
            (BlockingLevel::default(), BlockingLevelSource::Default)
        };
        Self {
            rules,
            blocking,
            blocking_source,
            config_file: configuration.file_name,
        }
    }

    fn planned(&self, id: &str) -> Option<&PlannedRule> {
        by_id(&self.rules, id, |rule| rule.definition.id)
    }

    pub(crate) fn level(&self, id: &str) -> Option<RuleLevel> {
        self.planned(id).map(|rule| rule.level)
    }

    pub(crate) fn is_active(&self, id: &str) -> bool {
        self.level(id).is_some_and(RuleLevel::is_active)
    }

    pub(crate) fn restamp_level(&self, id: &str) -> Option<RuleLevel> {
        self.planned(id)
            .filter(|rule| rule.restamped())
            .map(|rule| rule.level)
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

/// The rules of one producer that the plan left on.
///
/// A producer asks the plan once and reads the answer per finding, rather than
/// hoisting one boolean per rule at the top of its entry point. A boolean per
/// rule is a place the next rule has to be declared twice, once to be read and
/// once to be counted, and the counting is what an eight-clause negated
/// conjunction used to decide for a whole pass, silently. The set is derived
/// from the catalog's own `producer` field, so no producer keeps a second list
/// of the rules it owns.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct ActiveRules {
    on: BTreeSet<&'static str>,
}

impl ActiveRules {
    pub(crate) fn of(plan: &PolicyPlan, producer: Producer) -> Self {
        Self {
            on: plan
                .active_rules(producer)
                .map(|(definition, _)| definition.id)
                .collect(),
        }
    }

    /// The set a test of one family names for itself.
    #[cfg(test)]
    pub(crate) fn from_rules(rules: impl IntoIterator<Item = &'static RuleDefinition>) -> Self {
        Self {
            on: rules.into_iter().map(|rule| rule.id).collect(),
        }
    }

    pub(crate) fn on(&self, rule: &'static RuleDefinition) -> bool {
        self.on.contains(rule.id)
    }

    pub(crate) fn any_of(&self, rules: &[&'static RuleDefinition]) -> bool {
        rules.iter().any(|rule| self.on(rule))
    }

    pub(crate) fn any(&self) -> bool {
        !self.on.is_empty()
    }
}

fn active_rules_in(
    rules: &[PlannedRule],
    producer: Producer,
) -> impl Iterator<Item = (&'static RuleDefinition, RuleLevel)> + '_ {
    rules.iter().filter_map(move |planned| {
        (planned.definition.producer == producer && planned.level.is_active())
            .then_some((planned.definition, planned.level))
    })
}

fn compile_rules<const N: usize>(
    catalog: &[&'static RuleDefinition; N],
    policy: &ValidatedPolicy<'_>,
    configuration: &WorkspaceConfiguration,
) -> [PlannedRule; N] {
    catalog.map(|definition| {
        let (level, source) = if let Some(level) = policy.rules.get(definition.id).copied() {
            (level, RuleLevelSource::RequestRule)
        } else if let Some(level) = policy.categories.get(definition.category).copied() {
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
        }
    })
}

/// Overrides read once against a catalog: every selector is spelled the way a
/// selector may be spelled, names something that catalog knows, and appears
/// once. A plan compiles from this and from nothing else, so the question is
/// asked on one side of the boundary and answered on the other.
#[derive(Debug)]
pub(crate) struct ValidatedPolicy<'a> {
    input: &'a PolicyInput,
    rules: BTreeMap<&'a str, RuleLevel>,
    categories: BTreeMap<&'a str, RuleLevel>,
}

impl<'a> ValidatedPolicy<'a> {
    fn of(input: &'a PolicyInput, catalog: &[&RuleDefinition]) -> Result<Self, PolicyError> {
        Ok(Self {
            input,
            rules: accepted(
                input.rule_overrides.iter().map(RuleOverride::parts),
                validate_rule_selector,
                |selector| find_in(catalog, selector).is_some(),
                PolicyError::unknown_rule(),
                PolicyError::duplicate_rule(),
            )?,
            categories: accepted(
                input.category_overrides.iter().map(CategoryOverride::parts),
                validate_category_selector,
                |selector| CATEGORIES.binary_search(&selector).is_ok(),
                PolicyError::unknown_category(),
                PolicyError::duplicate_category(),
            )?,
        })
    }
}

/// One override list, accepted or refused. The rule list and the category list
/// differ by how a selector is spelled, by what knows it and by which errors
/// they carry, and by nothing else, so they go through one loop rather than
/// through two that have to stay parallel.
fn accepted<'a>(
    overrides: impl Iterator<Item = (&'a str, RuleLevel)>,
    well_formed: fn(&str) -> Result<(), PolicyError>,
    known: impl Fn(&str) -> bool,
    unknown: PolicyError,
    duplicate: PolicyError,
) -> Result<BTreeMap<&'a str, RuleLevel>, PolicyError> {
    let mut accepted = BTreeMap::new();
    for (selector, level) in overrides {
        well_formed(selector)?;
        if !known(selector) {
            return Err(unknown);
        }
        if accepted.insert(selector, level).is_some() {
            return Err(duplicate);
        }
    }
    Ok(accepted)
}

/// One entry of a table sorted by identifier.
///
/// Four tables of this module are sorted and searched that way, and reaching
/// the answer through `get` rather than through an index keeps the lookup total
/// on a table whose sorting only a test holds.
fn by_id<'a, T>(sorted: &'a [T], id: &str, key: impl Fn(&T) -> &str) -> Option<&'a T> {
    let found = sorted.binary_search_by_key(&id, |entry| key(entry)).ok()?;
    sorted.get(found)
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
    use crate::permutations::next_permutation;
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
        let request = request.validate().expect("layered policy should validate");
        let plan = PolicyPlan::compile_with_configuration(&request, &configuration);
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
        let explicit = explicit
            .validate()
            .expect("request blocking should validate");
        let explicit = PolicyPlan::compile_with_configuration(&explicit, &configuration);
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
    fn twenty_override_orders_compile_to_the_same_plan() {
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
            match &expected_plan {
                Some(expected) => assert_eq!(&plan, expected),
                None => expected_plan = Some(plan),
            }

            assert!(
                next_permutation(&mut order),
                "twenty permutations are below the full permutation count"
            );
        }
        assert_eq!(orders.len(), 20);
    }
}
