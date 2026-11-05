use std::env;
use std::ffi::{OsStr, OsString};
use std::io::{self, IsTerminal};

pub(crate) const NON_INTERACTIVE_MARKERS: [&str; 26] = [
    "CLAUDECODE",
    "CODEX_SANDBOX",
    "CURSOR_AGENT",
    "CLAUDE_CODE",
    "CODEX_CI",
    "OPENCODE",
    "GOOSE_TERMINAL",
    "AMP_THREAD_ID",
    "CLINE_ACTIVE",
    "AUGMENT_AGENT",
    "TRAE_AI_SHELL_ID",
    "AGENT_SESSION_ID",
    "AGENT_THREAD_ID",
    "GIT_DIR",
    "GIT_INDEX_FILE",
    "GITHUB_ACTIONS",
    "GITLAB_CI",
    "BUILDKITE",
    "CIRCLECI",
    "TF_BUILD",
    "JENKINS_URL",
    "TEAMCITY_VERSION",
    "CODEBUILD_BUILD_ID",
    "BITBUCKET_BUILD_NUMBER",
    "TRAVIS",
    "DRONE",
];

const DEFAULT_WIDTH: usize = 80;

#[derive(Debug, Clone, Copy)]
pub(crate) struct Streams {
    pub(crate) stdin: bool,
    pub(crate) stdout: bool,
    pub(crate) stderr: bool,
    pub(crate) columns: Option<usize>,
}

impl Streams {
    fn current() -> Self {
        let stdout = io::stdout().is_terminal();
        Self {
            stdin: io::stdin().is_terminal(),
            stdout,
            stderr: io::stderr().is_terminal(),
            columns: stdout
                .then(|| console::Term::stdout().size_checked())
                .flatten()
                .map(|(_, columns)| usize::from(columns)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TerminalProfile {
    pub(crate) interactive: bool,
    pub(crate) color: bool,
    pub(crate) animate: bool,
    pub(crate) rewrite_progress: bool,
    pub(crate) width: usize,
}

impl TerminalProfile {
    pub(crate) fn detect(json: bool, yes: bool) -> Self {
        Self::resolve(json, yes, Streams::current(), |name| env::var_os(name))
    }

    pub(crate) fn resolve(
        json: bool,
        yes: bool,
        streams: Streams,
        variable: impl Fn(&str) -> Option<OsString>,
    ) -> Self {
        let dumb = variable("TERM").as_deref() == Some(OsStr::new("dumb"));
        let unattended = variable("CI").as_deref().is_some_and(|value| !ci_is_off(value))
            || NON_INTERACTIVE_MARKERS
                .iter()
                .any(|marker| variable(marker).as_deref().is_some_and(|value| !value.is_empty()));
        let interactive = streams.stdin && streams.stdout && !json && !yes && !dumb && !unattended;
        let color = streams.stdout
            && variable("NO_COLOR").is_none_or(|value| value.is_empty())
            && !dumb;
        let width = if !streams.stdout || dumb {
            DEFAULT_WIDTH
        } else {
            streams
                .columns
                .filter(|columns| *columns > 0)
                .or_else(|| {
                    variable("COLUMNS")
                        .as_deref()
                        .and_then(OsStr::to_str)
                        .and_then(|value| value.parse().ok())
                })
                .filter(|width| *width > 0)
                .unwrap_or(DEFAULT_WIDTH)
        };
        Self {
            interactive,
            color,
            animate: interactive && color,
            rewrite_progress: streams.stderr && !dumb && !unattended,
            width,
        }
    }
}

fn ci_is_off(value: &OsStr) -> bool {
    value
        .to_str()
        .is_some_and(|value| value.is_empty() || value == "0" || value.eq_ignore_ascii_case("false"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TERMINALS: Streams = Streams {
        stdin: true,
        stdout: true,
        stderr: true,
        columns: None,
    };

    fn with(pairs: &[(&'static str, &'static str)]) -> impl Fn(&str) -> Option<OsString> {
        let pairs = pairs.to_vec();
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| OsString::from(value))
        }
    }

    fn profile(yes: bool, streams: Streams, pairs: &[(&'static str, &'static str)]) -> TerminalProfile {
        TerminalProfile::resolve(false, yes, streams, with(pairs))
    }

    type Case = (&'static str, bool, Streams, &'static [(&'static str, &'static str)], [bool; 4]);

    #[test]
    fn each_marker_flag_and_stream_decides_what_the_run_shows() {
        let redirected_stderr = Streams { stderr: false, ..TERMINALS };
        let cases: [Case; 11] = [
            ("a reader at a terminal", false, TERMINALS, &[], [true, true, true, true]),
            ("a git hook", false, TERMINALS, &[("GIT_DIR", ".git")], [false, true, false, false]),
            ("an agent", false, TERMINALS, &[("CLAUDECODE", "1")], [false, true, false, false]),
            ("CI=false", false, TERMINALS, &[("CI", "false")], [true, true, true, true]),
            ("CI=true", false, TERMINALS, &[("CI", "true")], [false, true, false, false]),
            ("--yes", true, TERMINALS, &[], [false, true, false, true]),
            ("TERM=dumb", false, TERMINALS, &[("TERM", "dumb")], [false, false, false, false]),
            ("NO_COLOR", false, TERMINALS, &[("NO_COLOR", "1")], [true, false, false, true]),
            ("NO_COLOR=0", false, TERMINALS, &[("NO_COLOR", "0")], [true, false, false, true]),
            ("NO_COLOR=", false, TERMINALS, &[("NO_COLOR", "")], [true, true, true, true]),
            ("a redirected stderr", false, redirected_stderr, &[], [true, true, true, false]),
        ];
        for (name, yes, streams, pairs, expected) in cases {
            let resolved = profile(yes, streams, pairs);
            assert_eq!(
                [resolved.interactive, resolved.color, resolved.animate, resolved.rewrite_progress],
                expected,
                "{name}"
            );
        }
        let json = TerminalProfile::resolve(true, false, TERMINALS, with(&[]));
        assert!(!json.interactive && !json.animate);
    }

    #[test]
    fn every_marker_closes_the_report_and_an_empty_one_counts_as_unset() {
        for marker in NON_INTERACTIVE_MARKERS {
            let set = move |name: &str| (name == marker).then(|| "1".into());
            let resolved = TerminalProfile::resolve(false, false, TERMINALS, set);
            assert!(!resolved.interactive && !resolved.animate, "{marker}=1");
            assert!(!resolved.rewrite_progress, "{marker}=1 rewrote the progress line");
            let empty = move |name: &str| (name == marker).then(OsString::new);
            let resolved = TerminalProfile::resolve(false, false, TERMINALS, empty);
            assert!(resolved.interactive && resolved.animate, "{marker}= closed the report");
        }
        for off in ["false", "FALSE", "False", "0", ""] {
            assert!(profile(false, TERMINALS, &[("CI", off)]).interactive, "CI={off:?}");
        }
        for on in ["1", "true", "yes", "off", "no"] {
            assert!(!profile(false, TERMINALS, &[("CI", on)]).interactive, "CI={on:?}");
        }
    }

    #[test]
    fn the_width_is_the_terminal_then_columns_then_eighty() {
        let sized = Streams { columns: Some(120), ..TERMINALS };
        assert_eq!(profile(false, sized, &[("COLUMNS", "140")]).width, 120);
        assert_eq!(profile(false, TERMINALS, &[("COLUMNS", "140")]).width, 140);
        assert_eq!(profile(false, TERMINALS, &[("COLUMNS", "wide")]).width, 80);
        assert_eq!(profile(false, TERMINALS, &[]).width, 80);
        assert_eq!(profile(false, Streams { columns: Some(0), ..TERMINALS }, &[]).width, 80);
        assert_eq!(profile(false, TERMINALS, &[("COLUMNS", "79")]).width, 79);
        let narrow = Streams { columns: Some(30), ..TERMINALS };
        assert_eq!(profile(false, narrow, &[]).width, 30);
        let redirected = Streams { stdout: false, ..sized };
        assert_eq!(profile(false, redirected, &[("COLUMNS", "140")]).width, 80);
        assert_eq!(profile(false, sized, &[("TERM", "dumb")]).width, 80);
    }

    #[test]
    fn the_skill_names_every_non_interactive_marker() {
        let skill = include_str!("../skills/rust-doctor/SKILL.md");
        assert!(skill.contains("NON_INTERACTIVE_MARKERS"));
        for marker in NON_INTERACTIVE_MARKERS {
            assert!(skill.contains(&format!("`{marker}`")), "SKILL.md omits `{marker}`");
        }
    }
}
