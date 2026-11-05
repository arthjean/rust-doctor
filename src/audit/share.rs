use std::fmt;
use std::fmt::Write as _;

use super::{Audit, SCORE_MODEL};

const SHARE_BASE_URL: &str = "https://rust-doctor.com/share";
const MAX_SHARED_COUNT: usize = 1_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShareError {
    ScoreUnavailable,
    NonAuthoritative,
    InvalidPayload,
}

impl fmt::Display for ShareError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ScoreUnavailable => "the audit does not contain a score",
            Self::NonAuthoritative => "the audit score is not authoritative",
            Self::InvalidPayload => "the audit exceeds the public share bounds",
        })
    }
}

impl std::error::Error for ShareError {}

impl Audit {
    pub fn share_url(&self) -> Result<String, ShareError> {
        let score = self.score.as_ref().ok_or(ShareError::ScoreUnavailable)?;
        if !score.authoritative {
            return Err(ShareError::NonAuthoritative);
        }
        if !self.is_valid() {
            return Err(ShareError::InvalidPayload);
        }

        let production = self.production_totals();
        build_share_url(
            score.value,
            production.errors,
            production.warnings,
            production.info,
            self.source_files,
            self.production_lines,
        )
    }
}

pub(super) fn build_share_url(
    score: u8,
    errors: usize,
    warnings: usize,
    info: usize,
    source_files: usize,
    production_lines: usize,
) -> Result<String, ShareError> {
    if score > 100
        || [errors, warnings, info, source_files, production_lines]
            .into_iter()
            .any(|count| count > MAX_SHARED_COUNT)
    {
        return Err(ShareError::InvalidPayload);
    }

    let mut url = format!("{SHARE_BASE_URL}?s={score}&m={SCORE_MODEL}");
    for (key, count) in [
        ("e", errors),
        ("w", warnings),
        ("i", info),
        ("f", source_files),
        ("l", production_lines),
    ] {
        if count > 0 {
            let _ = write!(url, "&{key}={count}");
        }
    }
    Ok(url)
}
