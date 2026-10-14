//! How long each phase of one scan took, published only when asked.
//!
//! One recorder is shared by every pass of a run, both sides of a baseline
//! comparison included, so a phase that runs twice is published as its sum.
//! Clippy runs beside the native passes, so the phases overlap and their sum
//! exceeds `total`: the difference is what the overlap saved.
//!
//! Nothing here reaches a report unless `--timings` asked for it. A duration
//! differs on every run, and a default report stays byte-deterministic.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use serde::Serialize;

/// Milliseconds per phase. A phase the run did not reach is absent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Timings {
    /// The three toolchain version probes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub probes: Option<u64>,
    /// Discovery, `cargo metadata` and the configuration file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<u64>,
    /// `clippy-driver -W help`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lint_list: Option<u64>,
    /// Clippy until the first workspace member was checked: the dependencies.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clippy_dependencies: Option<u64>,
    /// Clippy from the first workspace member checked to its exit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clippy_linting: Option<u64>,
    /// The workspace walk, its narrowing and the exclusions gathered from it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub walk: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_kernel: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub structure: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cargo_health: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo: Option<u64>,
    /// Wall-clock milliseconds from the request to the report.
    pub total: u64,
}

impl Timings {
    /// The phases the run reached, in the order a scan meets them.
    pub fn phases(&self) -> impl Iterator<Item = (&'static str, u64)> {
        [
            ("probes", self.probes),
            ("metadata", self.metadata),
            ("lint list", self.lint_list),
            ("clippy dependencies", self.clippy_dependencies),
            ("clippy linting", self.clippy_linting),
            ("walk", self.walk),
            ("source kernel", self.source_kernel),
            ("structure", self.structure),
            ("cargo health", self.cargo_health),
            ("repo", self.repo),
        ]
        .into_iter()
        .filter_map(|(name, milliseconds)| Some((name, milliseconds?)))
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum Phase {
    Probes,
    Metadata,
    LintList,
    ClippyDependencies,
    ClippyLinting,
    Walk,
    SourceKernel,
    Structure,
    CargoHealth,
    Repo,
}

/// Where every pass of one run adds the time it took. Cheap to clone, and
/// shared across the threads a run overlaps its passes on.
#[derive(Debug, Clone)]
pub(crate) struct TimingRecorder {
    started: Instant,
    recorded: Arc<Mutex<Timings>>,
}

impl TimingRecorder {
    pub(crate) fn start() -> Self {
        Self {
            started: Instant::now(),
            recorded: Arc::default(),
        }
    }

    pub(crate) fn record(&self, phase: Phase, elapsed: Duration) {
        let mut timings = self.recorded.lock().unwrap_or_else(PoisonError::into_inner);
        let slot = match phase {
            Phase::Probes => &mut timings.probes,
            Phase::Metadata => &mut timings.metadata,
            Phase::LintList => &mut timings.lint_list,
            Phase::ClippyDependencies => &mut timings.clippy_dependencies,
            Phase::ClippyLinting => &mut timings.clippy_linting,
            Phase::Walk => &mut timings.walk,
            Phase::SourceKernel => &mut timings.source_kernel,
            Phase::Structure => &mut timings.structure,
            Phase::CargoHealth => &mut timings.cargo_health,
            Phase::Repo => &mut timings.repo,
        };
        *slot = Some(slot.unwrap_or(0).saturating_add(milliseconds(elapsed)));
    }

    pub(crate) fn finish(&self) -> Timings {
        let recorded = *self.recorded.lock().unwrap_or_else(PoisonError::into_inner);
        Timings {
            total: milliseconds(self.started.elapsed()),
            ..recorded
        }
    }
}

/// Runs `work` and adds its duration to `phase` when the run records timings.
pub(crate) fn timed<T>(
    recorder: Option<&TimingRecorder>,
    phase: Phase,
    work: impl FnOnce() -> T,
) -> T {
    let started = Instant::now();
    let answer = work();
    if let Some(recorder) = recorder {
        recorder.record(phase, started.elapsed());
    }
    answer
}

fn milliseconds(elapsed: Duration) -> u64 {
    u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_phase_run_twice_is_published_as_its_sum_and_an_unreached_one_is_absent() {
        let recorder = TimingRecorder::start();
        recorder.record(Phase::Metadata, Duration::from_millis(40));
        recorder.record(Phase::Metadata, Duration::from_millis(2));
        let timings = recorder.finish();
        assert_eq!(timings.metadata, Some(42));
        assert_eq!(timings.probes, None);
        assert_eq!(timings.phases().collect::<Vec<_>>(), [("metadata", 42)]);
        let json = serde_json::to_value(timings).unwrap();
        assert_eq!(json["metadata"], 42);
        assert!(json.get("probes").is_none());
        assert!(json.get("total").is_some());
    }
}
