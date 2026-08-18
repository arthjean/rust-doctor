//! The one error shape every stage of the crate reports through.
//!
//! It lives on its own rather than inside the module that runs Cargo, because
//! naming an error is not an execution concern: the bounded git layer, the
//! configuration loader, the scan target resolver and the baseline snapshot all
//! raise one, and none of them has any business importing the orchestrator to
//! do it. Hosting it in `execution` made three of the modules `execution`
//! itself reads import it back, which is a cycle a reader has to hold in their
//! head for no return.

/// A failure that ends a stage, stamped with the stage that raised it.
///
/// `stage` and `code` are closed vocabularies published in the JSON report, so
/// both are `&'static str`: a value computed at runtime cannot become one.
/// `message` is the only part that carries detail, and `report::sanitize_text`
/// is what strips a path or a home directory out of it before publication.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct InternalError {
    pub(crate) stage: &'static str,
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

impl InternalError {
    pub(crate) fn new(stage: &'static str, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            stage,
            code,
            message: message.into(),
        }
    }
}
