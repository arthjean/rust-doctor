//! A pass that panics degrades into an error at its stage, as a pass that
//! returns an error does, instead of taking the run down with it.
//!
//! Every producer runs under `catch_unwind`. What a contained pass leaves
//! behind is dropped whole: its scan is `None`, and the report carries one
//! `pass-panicked` error at its stage with a fixed sentence, so the score
//! stops calling itself authoritative and every other pass's findings stay.
//! The panic's own message never reaches the report or stderr, since it can
//! quote any text the pass was reading.
//!
//! The process keeps a single panic hook, installed on the first contained
//! pass. It stays silent for a panic raised on a thread inside a contained
//! pass and hands every other panic to the hook it replaced, so a library
//! embedding `inspect` keeps its own reporting outside the passes.
//!
//! A debug build panics on request at the end of the pass that
//! `RUST_DOCTOR_INJECT_PANIC` names, which is how the tests prove containment
//! from the binary a user runs. A release build never reads the variable.

use std::cell::Cell;
use std::panic::{self, AssertUnwindSafe};
use std::sync::{Mutex, Once, PoisonError};

/// The code of the error a contained panic publishes.
pub(crate) const PASS_PANICKED: &str = "pass-panicked";

/// One pass that runs contained: the stage its error is published at, the
/// sentence that error carries, and the name fault injection selects it by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Pass {
    pub(crate) stage: &'static str,
    pub(crate) message: &'static str,
    site: &'static str,
}

const SOURCE_FAILED: &str = "The source pass failed; the score is not authoritative.";
const DEPENDENCIES_FAILED: &str =
    "The dependencies pass failed; the score is not authoritative.";

pub(crate) const CARGO_HEALTH: Pass = Pass {
    stage: "dependencies",
    message: DEPENDENCIES_FAILED,
    site: "cargo_health",
};
pub(crate) const REPO: Pass = Pass {
    stage: "repo",
    message: "The repo pass failed; the score is not authoritative.",
    site: "repo",
};
pub(crate) const WALK: Pass = Pass {
    stage: "source",
    message: SOURCE_FAILED,
    site: "walk",
};
pub(crate) const SOURCE_KERNEL: Pass = Pass {
    stage: "source",
    message: SOURCE_FAILED,
    site: "source_kernel",
};
pub(crate) const DEPENDENCY_TRUTH: Pass = Pass {
    stage: "dependencies",
    message: DEPENDENCIES_FAILED,
    site: "dependency_truth",
};
pub(crate) const STRUCTURE: Pass = Pass {
    stage: "structure",
    message: "The structure pass failed; the score is not authoritative.",
    site: "structure",
};
pub(crate) const CLIPPY: Pass = Pass {
    stage: "execution",
    message: "The execution pass failed; the score is not authoritative.",
    site: "clippy",
};

/// The injection site inside the Clippy stream reader, at Cargo's last
/// message, while the `cargo clippy` child and its process group still run.
const CLIPPY_STREAM: &str = "clippy_stream";

/// The stream reader's injection point, handed every message it reads.
pub(crate) fn observe_stream(message: &cargo_metadata::Message) {
    if matches!(message, cargo_metadata::Message::BuildFinished(_)) {
        inject(CLIPPY_STREAM);
    }
}

/// The passes of one run that panicked, gathered from every thread it uses.
#[derive(Debug, Default)]
pub(crate) struct Panicked(Mutex<Vec<Pass>>);

impl Panicked {
    /// Runs `pass`, and `None` when it panicked, which is then recorded.
    pub(crate) fn contain<T>(&self, pass: Pass, run: impl FnOnce() -> T) -> Option<T> {
        install_hook();
        let outcome = {
            let _inside = Inside::enter();
            panic::catch_unwind(AssertUnwindSafe(|| {
                let value = run();
                inject(pass.site);
                value
            }))
        };
        // The payload is dropped unread: it can quote what the pass read.
        outcome.map_err(|_| self.record(pass)).ok()
    }

    /// Records a pass whose thread ended in a panic outside `contain`.
    pub(crate) fn record(&self, pass: Pass) {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).push(pass);
    }

    pub(crate) fn into_passes(self) -> Vec<Pass> {
        self.0.into_inner().unwrap_or_else(PoisonError::into_inner)
    }
}

thread_local! {
    /// How many contained passes this thread is inside of.
    static INSIDE: Cell<usize> = const { Cell::new(0) };
}

static HOOK: Once = Once::new();

/// Marks the current thread as inside a contained pass until dropped, which
/// unwinding does only after the hook has run.
struct Inside;

impl Inside {
    fn enter() -> Self {
        INSIDE.with(|depth| depth.set(depth.get() + 1));
        Self
    }
}

impl Drop for Inside {
    fn drop(&mut self) {
        INSIDE.with(|depth| depth.set(depth.get().saturating_sub(1)));
    }
}

fn install_hook() {
    HOOK.call_once(|| {
        let previous = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            if INSIDE.with(Cell::get) == 0 {
                previous(info);
            }
        }));
    });
}

/// The variable a debug build reads to panic at one injection site.
#[cfg(debug_assertions)]
pub(crate) const INJECT_PANIC: &str = "RUST_DOCTOR_INJECT_PANIC";

/// Panics when a debug build was asked to at `site`.
#[cfg(debug_assertions)]
pub fn inject(site: &str) {
    if std::env::var_os(INJECT_PANIC).is_some_and(|value| value == site) {
        #[allow(clippy::panic, reason = "the fault a debug build injects on request")]
        {
            panic!("injected fault at {site}, a payload no report may quote");
        }
    }
}

#[cfg(not(debug_assertions))]
pub const fn inject(_site: &str) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_contained_panic_is_recorded_once_and_its_neighbors_run() {
        let panicked = Panicked::default();
        #[allow(clippy::panic, reason = "the panic under test")]
        let lost: Option<()> = panicked.contain(STRUCTURE, || panic!("payload"));
        assert_eq!(lost, None);
        assert_eq!(panicked.contain(REPO, || 7), Some(7));
        assert_eq!(panicked.into_passes(), vec![STRUCTURE]);
        assert_eq!(INSIDE.with(Cell::get), 0);
    }

    fn manifest() -> toml::Table {
        let manifest = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
            .unwrap_or_default()
            .parse::<toml::Table>()
            .unwrap_or_default();
        assert!(manifest.contains_key("package"));
        manifest
    }

    /// `catch_unwind` cannot stop a panic a profile turns into an abort.
    #[test]
    fn no_profile_aborts_on_panic() {
        let manifest = manifest();
        let profiles = manifest.get("profile").and_then(toml::Value::as_table);
        for (name, profile) in profiles.into_iter().flatten() {
            assert_ne!(
                profile.get("panic").and_then(toml::Value::as_str),
                Some("abort"),
                "[profile.{name}] aborts on panic, so no pass can be contained"
            );
        }
    }
    /// The panics containment exists for are kept out at the source too:
    /// CI's `-D warnings` turns this lint into a refusal of any unchecked
    /// string slice in production code.
    #[test]
    fn the_crate_lints_its_own_string_slices() {
        let lint = manifest()
            .get("lints")
            .and_then(|lints| lints.get("clippy"))
            .and_then(|clippy| clippy.get("string_slice"))
            .and_then(toml::Value::as_str)
            .map(str::to_owned);
        assert_eq!(lint.as_deref(), Some("warn"));
    }
}
