#[derive(Debug, Clone, PartialEq, Eq)]
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
