use std::fmt;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress<'a> {
    Discovered { directory: &'a str },
    Dependencies,
    Linting {
        package: &'a str,
        done: usize,
        total: usize,
    },
    NativePasses,
}

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
