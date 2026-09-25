//! The line a scan draws on stderr while it runs.
//!
//! A cold scan compiles for minutes, and the static `Scanning Rust files...`
//! it used to print read as a hang for all of them. On a terminal one line is
//! rewritten in place, at most every 100 ms, and cleared before anything else
//! is printed. Anywhere else, an agent, a CI log, a git hook, nothing is
//! rewritten: each phase prints once, so a log carries three lines at most.

use std::collections::BTreeSet;
use std::io::{self, Write};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use rust_doctor::progress::{Progress, ProgressSink};

const REDRAW_INTERVAL: Duration = Duration::from_millis(100);

/// Erases the current terminal line and returns to its start.
const ERASE_LINE: &str = "\r\x1b[2K";

#[derive(Debug)]
pub(crate) struct ProgressLine {
    rewrite: bool,
    state: Mutex<State>,
}

#[derive(Debug, Default)]
struct State {
    last_drawn: Option<Instant>,
    last_phase: Option<&'static str>,
    printed: BTreeSet<&'static str>,
    on_screen: bool,
}

impl ProgressLine {
    /// `rewrite` is whether stderr is a terminal that understands the erase
    /// sequence.
    pub(crate) fn new(rewrite: bool) -> Arc<Self> {
        Arc::new(Self {
            rewrite,
            state: Mutex::new(State::default()),
        })
    }

    pub(crate) fn sink(self: &Arc<Self>) -> ProgressSink {
        let line = Arc::clone(self);
        ProgressSink::new(move |progress| line.show(progress))
    }

    fn show(&self, progress: Progress<'_>) {
        let (phase, text) = describe(progress);
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let mut stderr = io::stderr().lock();
        if !self.rewrite {
            if state.printed.insert(phase) {
                let _ = writeln!(stderr, "{text}");
            }
            return;
        }
        let now = Instant::now();
        let changed = state.last_phase != Some(phase);
        let due = state
            .last_drawn
            .is_none_or(|drawn| now.duration_since(drawn) >= REDRAW_INTERVAL);
        if changed || due {
            let _ = write!(stderr, "{ERASE_LINE}{text}");
            let _ = stderr.flush();
            state.last_drawn = Some(now);
            state.last_phase = Some(phase);
            state.on_screen = true;
        }
    }

    /// Erases the line, so the report or an error starts on a clean one.
    pub(crate) fn clear(&self) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.on_screen {
            let mut stderr = io::stderr().lock();
            let _ = write!(stderr, "{ERASE_LINE}");
            let _ = stderr.flush();
            state.on_screen = false;
        }
    }
}

fn describe(progress: Progress<'_>) -> (&'static str, String) {
    match progress {
        Progress::Dependencies => ("dependencies", "Compiling dependencies...".to_owned()),
        Progress::Linting {
            package,
            done,
            total,
        } => (
            "linting",
            format!(
                "Linting {} ({done}/{total})",
                rust_doctor::terminal_text::sanitize(package)
            ),
        ),
        Progress::NativePasses => ("native passes", "Running native passes...".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_a_terminal_each_phase_prints_once() {
        let line = ProgressLine::new(false);
        for done in 1..=40 {
            line.show(Progress::Linting {
                package: "member",
                done,
                total: 40,
            });
        }
        line.show(Progress::Dependencies);
        line.show(Progress::NativePasses);
        line.show(Progress::Dependencies);
        let state = line.state.lock().unwrap();
        assert_eq!(state.printed.len(), 3);
        assert!(!state.on_screen);
    }

    #[test]
    fn on_a_terminal_a_phase_redraws_at_most_every_interval() {
        let line = ProgressLine::new(true);
        line.show(Progress::Dependencies);
        let first = line.state.lock().unwrap().last_drawn;
        line.show(Progress::Dependencies);
        assert_eq!(line.state.lock().unwrap().last_drawn, first);
        line.show(Progress::NativePasses);
        assert_eq!(line.state.lock().unwrap().last_phase, Some("native passes"));
        line.clear();
        assert!(!line.state.lock().unwrap().on_screen);
    }
}
