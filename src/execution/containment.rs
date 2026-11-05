use std::cell::Cell;
use std::panic::{self, AssertUnwindSafe};
use std::sync::{Mutex, Once, PoisonError};

pub(crate) const PASS_PANICKED: &str = "pass-panicked";

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

const CLIPPY_STREAM: &str = "clippy_stream";

pub(crate) fn observe_stream(message: &cargo_metadata::Message) {
    if matches!(message, cargo_metadata::Message::BuildFinished(_)) {
        inject(CLIPPY_STREAM);
    }
}

#[derive(Debug, Default)]
pub(crate) struct Panicked(Mutex<Vec<Pass>>);

impl Panicked {
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
        outcome.map_err(|_| self.record(pass)).ok()
    }

    pub(crate) fn record(&self, pass: Pass) {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).push(pass);
    }

    pub(crate) fn into_passes(self) -> Vec<Pass> {
        self.0.into_inner().unwrap_or_else(PoisonError::into_inner)
    }
}

thread_local! {
    static INSIDE: Cell<usize> = const { Cell::new(0) };
}

static HOOK: Once = Once::new();

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

#[cfg(debug_assertions)]
pub(crate) const INJECT_PANIC: &str = "RUST_DOCTOR_INJECT_PANIC";

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
