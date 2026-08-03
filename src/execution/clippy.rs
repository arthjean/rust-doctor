use super::ScanExecution;
use crate::policy::{PolicyPlan, Producer, RuleDefinition, RuleLevel};

const BASE_ARGS: [&str; 5] = [
    "clippy",
    "--workspace",
    "--all-targets",
    "--no-deps",
    "--message-format=json",
];

pub(super) fn arguments_for_plan(plan: &PolicyPlan) -> Vec<&'static str> {
    arguments_for_rules(plan.active_rules(Producer::Clippy))
}

pub(crate) fn arguments_for_rules<'a>(
    rules: impl IntoIterator<Item = (&'a RuleDefinition, RuleLevel)>,
) -> Vec<&'static str> {
    let mut arguments = Vec::with_capacity(BASE_ARGS.len() + 1 + 16);
    arguments.extend(BASE_ARGS);
    arguments.push("--");
    for (definition, level) in rules {
        if let Some(flag) = level.clippy_flag() {
            arguments.extend([flag, definition.id]);
        }
    }
    arguments
}

#[derive(Debug, Default)]
pub(crate) enum ClippyExecution {
    #[default]
    NotRun,
    Disabled,
    Finished(ScanExecution),
}

impl ClippyExecution {
    pub(crate) const fn finished(&self) -> Option<&ScanExecution> {
        match self {
            Self::Finished(scan) => Some(scan),
            Self::NotRun | Self::Disabled => None,
        }
    }

    pub(crate) const fn has_outcome(&self) -> bool {
        !matches!(self, Self::NotRun)
    }

    pub(super) fn is_complete(&self) -> bool {
        match self {
            Self::Disabled => true,
            Self::Finished(scan) => {
                scan.exit_success == Some(true)
                    && scan.build_finished == Some(true)
                    && scan.malformed_messages == 0
                    && scan.errors.is_empty()
            }
            Self::NotRun => false,
        }
    }

    #[cfg(test)]
    pub(super) fn into_finished(self) -> Option<ScanExecution> {
        match self {
            Self::Finished(scan) => Some(scan),
            Self::NotRun | Self::Disabled => None,
        }
    }
}

#[cfg(test)]
impl From<Option<ScanExecution>> for ClippyExecution {
    fn from(scan: Option<ScanExecution>) -> Self {
        scan.map_or(Self::NotRun, Self::Finished)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::PolicyInput;

    #[test]
    fn arguments_prune_off_rules_but_keep_error_rules_at_warning() {
        let input = PolicyInput::default()
            .with_rule("clippy::dbg_macro", RuleLevel::Off)
            .with_rule("clippy::todo", RuleLevel::Error);
        let plan = PolicyPlan::compile(&input).expect("policy should compile");
        let arguments = arguments_for_plan(&plan);

        assert!(!arguments.contains(&"clippy::dbg_macro"));
        assert!(
            arguments
                .windows(2)
                .any(|pair| pair == ["-W", "clippy::todo"])
        );
        assert!(!arguments.contains(&"-D"));

        // Éteindre chaque catégorie éteint chaque règle, quel que soit le
        // volume du catalogue: la commande retombe alors sur ses seules bases.
        let all_off = crate::policy::CATEGORIES
            .iter()
            .fold(PolicyInput::default(), |input, category| {
                input.with_category(*category, RuleLevel::Off)
            });
        let all_off = PolicyPlan::compile(&all_off).expect("policy should compile");
        assert_eq!(
            arguments_for_plan(&all_off),
            [
                "clippy",
                "--workspace",
                "--all-targets",
                "--no-deps",
                "--message-format=json",
                "--",
            ]
        );
    }
}
