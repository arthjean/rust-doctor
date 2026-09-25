//! The public share link: the score and the counts it rests on, in a URL the
//! rust-doctor.com page reads, and nothing else.

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

        // The counts the block already publishes, not a third summation over the same
        // categories that nothing kept in step with the first two.
        let (_, occurrences) = self.totals();
        build_share_url(
            score.value,
            occurrences.errors,
            occurrences.warnings,
            occurrences.info,
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

    // The model comes first and is never omitted: every number after it is a
    // reading of one scale, and a payload that names none is a score the page
    // it opens has to guess the meaning of. `core-v2` and `core-v3` publish
    // the same shape over the same rules for very different values.
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
