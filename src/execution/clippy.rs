use super::ScanExecution;

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
