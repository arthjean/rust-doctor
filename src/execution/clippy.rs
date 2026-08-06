use super::ScanExecution;
use crate::policy::{PolicyPlan, Producer, RuleDefinition, RuleLevel};

/// Cargo's default targets: libraries and binaries, that is, what the project
/// publishes.
///
/// `--all-targets` also compiled tests, benches, examples and build scripts.
/// Measured on the corpus on 2026-08-04, 69.9% of the pack findings came from
/// there, along with 1252 of the 1279 findings of the self-scan. An
/// `.unwrap()` under `#[cfg(test)]` is the expected failure mechanism of the
/// test, a `println!` in `build.rs` is the channel Cargo imposes, a `dbg!`
/// under `examples/` is the demonstration: none of them is a defect of the
/// shipped codebase, so none belongs in a score that judges it.
///
/// The filtering cannot happen afterwards: Cargo labels `test: true` on every
/// message under `--all-targets`, including those of a binary with no test at
/// all. The scope is therefore set here, at the source.
const BASE_ARGS: [&str; 4] = [
    "clippy",
    "--workspace",
    "--no-deps",
    "--message-format=json",
];

/// Silences everything Clippy warns about by default, so the only lints left
/// are the ones the catalog names right after it.
///
/// A lint the catalog does not know still reaches the report otherwise:
/// `report::diagnostics` only drops a diagnostic whose rule is catalogued and
/// inactive. It arrives with no category, no tier and no help, it cannot weigh
/// on the score, and its mere presence costs the score its authoritative flag.
/// Measured on four corpus repositories on 2026-08-06: 9 findings out of 164
/// came from there, and they were enough to disqualify three of the four
/// reports. Dropping them makes every finding one the tool can explain.
///
/// Order matters, `-W` after `-A` wins, so this stays the first argument of the
/// lint section.
const SILENCE_UNCATALOGUED: [&str; 2] = ["-A", "clippy::all"];

pub(super) fn arguments_for_plan(plan: &PolicyPlan) -> Vec<&'static str> {
    arguments_for_rules(plan.active_rules(Producer::Clippy))
}

pub(crate) fn arguments_for_rules<'a>(
    rules: impl IntoIterator<Item = (&'a RuleDefinition, RuleLevel)>,
) -> Vec<&'static str> {
    let mut arguments =
        Vec::with_capacity(BASE_ARGS.len() + 1 + SILENCE_UNCATALOGUED.len() + 16);
    arguments.extend(BASE_ARGS);
    arguments.push("--");
    arguments.extend(SILENCE_UNCATALOGUED);
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

        // Turning off every category turns off every rule, whatever the
        // catalog volume: the command then carries the silencing alone, so the
        // scan reports nothing rather than everything Clippy warns about.
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
                "--no-deps",
                "--message-format=json",
                "--",
                "-A",
                "clippy::all",
            ]
        );
    }
}
