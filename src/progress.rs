//! What a running scan is doing, for a caller that wants to show it.
//!
//! A cold scan spends minutes compiling, and a static "Scanning Rust files..."
//! line read as a hang for all of them. The library draws nothing: it hands
//! each phase to the sink the request carries, and the binary decides how to
//! show it, a line rewritten in place on a terminal, a few plain lines
//! anywhere else.

use std::fmt;
use std::sync::Arc;

/// One phase of a scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress<'a> {
    /// Cargo is compiling what the workspace depends on.
    Dependencies,
    /// Clippy has linted `done` of the `total` workspace members, `package`
    /// last.
    Linting {
        package: &'a str,
        done: usize,
        total: usize,
    },
    /// The native passes read the source, the manifests and git.
    NativePasses,
}

/// Where a scan reports its phases. Cheap to clone, shared across both sides
/// of a baseline comparison.
#[derive(Clone)]
pub struct ProgressSink(Arc<dyn Fn(Progress<'_>) + Send + Sync>);

impl ProgressSink {
    pub fn new(sink: impl Fn(Progress<'_>) + Send + Sync + 'static) -> Self {
        Self(Arc::new(sink))
    }

    pub(crate) fn report(&self, progress: Progress<'_>) {
        (self.0)(progress);
    }
}

impl fmt::Debug for ProgressSink {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ProgressSink")
    }
}
