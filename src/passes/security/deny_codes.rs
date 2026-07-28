use crate::diagnostics::Category;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DenyCheck {
    Advisories,
    Bans,
    Licenses,
    Sources,
}

impl DenyCheck {
    pub(super) const fn exit_bit(self) -> i32 {
        match self {
            Self::Advisories => 1,
            Self::Bans => 2,
            Self::Licenses => 4,
            Self::Sources => 8,
        }
    }

    pub(super) const fn rule(self) -> &'static str {
        match self {
            Self::Advisories => "deny-advisory",
            Self::Bans => "deny-ban",
            Self::Licenses => "deny-license",
            Self::Sources => "deny-source",
        }
    }

    pub(super) const fn category(self) -> Category {
        match self {
            Self::Advisories => Category::Dependencies,
            Self::Bans | Self::Licenses | Self::Sources => Category::Cargo,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DenyCode {
    Check(DenyCheck),
    IncompleteEvidence,
}

struct CodeContract {
    minor: &'static str,
    advisories: &'static [&'static str],
    bans: &'static [&'static str],
    licenses: &'static [&'static str],
    sources: &'static [&'static str],
}

const ADVISORIES: &[&str] = &[
    "vulnerability",
    "notice",
    "unmaintained",
    "unsound",
    "yanked",
    "advisory-ignored",
    "yanked-ignored",
    "advisory-not-detected",
    "yanked-not-detected",
    "unknown-advisory",
];
const ADVISORY_FAILURES: &[&str] = &["index-failure", "index-cache-load-failure"];
const BANS_018: &[&str] = &[
    "banned",
    "allowed",
    "not-allowed",
    "duplicate",
    "skipped",
    "wildcard",
    "unmatched-skip",
    "unnecessary-skip",
    "allowed-by-wrapper",
    "unmatched-wrapper",
    "skipped-by-root",
    "unmatched-skip-root",
    "build-script-not-allowed",
    "exact-features-mismatch",
    "feature-not-explicitly-allowed",
    "feature-banned",
    "unknown-feature",
    "default-feature-enabled",
    "path-bypassed",
    "path-bypassed-by-glob",
    "checksum-match",
    "checksum-mismatch",
    "denied-by-extension",
    "detected-executable",
    "detected-executable-script",
    "unable-to-check-path",
    "features-enabled",
    "unmatched-bypass",
    "unmatched-path-bypass",
    "unmatched-glob",
    "unused-wrapper",
    "workspace-duplicate",
    "unresolved-workspace-dependency",
    "unused-workspace-dependency",
];
const BANS_019: &[&str] = &[
    "banned",
    "allowed",
    "not-allowed",
    "duplicate",
    "skipped",
    "wildcard",
    "unmatched-skip",
    "unnecessary-skip",
    "allowed-by-wrapper",
    "unmatched-wrapper",
    "skipped-by-root",
    "unmatched-skip-root",
    "build-script-not-allowed",
    "exact-features-mismatch",
    "feature-not-explicitly-allowed",
    "feature-banned",
    "unknown-feature",
    "default-feature-enabled",
    "path-bypassed",
    "path-bypassed-by-glob",
    "checksum-match",
    "checksum-mismatch",
    "denied-by-extension",
    "detected-executable",
    "detected-executable-script",
    "unable-to-check-path",
    "features-enabled",
    "unmatched-bypass",
    "unmatched-path-bypass",
    "unmatched-glob",
    "unused-wrapper",
    "workspace-duplicate",
    "unresolved-workspace-dependency",
    "unused-workspace-dependency",
    "non-utf8-path",
    "non-root-path",
];
const LICENSES_018: &[&str] = &[
    "accepted",
    "rejected",
    "unlicensed",
    "skipped-private-workspace-crate",
    "license-not-encountered",
    "license-exception-not-encountered",
    "missing-clarification-file",
];
const LICENSES_019: &[&str] = &[
    "accepted",
    "rejected",
    "unlicensed",
    "skipped-private-workspace-crate",
    "license-not-encountered",
    "license-exception-not-encountered",
    "missing-clarification-file",
    "parse-error",
    "empty-license-field",
    "no-license-field",
    "gather-failure",
];
const SOURCES: &[&str] = &[
    "git-source-underspecified",
    "allowed-source",
    "allowed-by-organization",
    "source-not-allowed",
    "unmatched-source",
    "unmatched-organization",
];
const CONTRACTS: &[CodeContract] = &[
    CodeContract {
        minor: "0.18",
        advisories: ADVISORIES,
        bans: BANS_018,
        licenses: LICENSES_018,
        sources: SOURCES,
    },
    CodeContract {
        minor: "0.19",
        advisories: ADVISORIES,
        bans: BANS_019,
        licenses: LICENSES_019,
        sources: SOURCES,
    },
];

pub(super) fn classify(version: &str, code: &str) -> Result<DenyCode, String> {
    let contract = contract(version).ok_or_else(|| {
        format!("cargo-deny version `{version}` has no qualified semantic-code table")
    })?;
    if ADVISORY_FAILURES.contains(&code) {
        return Ok(DenyCode::IncompleteEvidence);
    }
    for (codes, check) in [
        (contract.advisories, DenyCheck::Advisories),
        (contract.bans, DenyCheck::Bans),
        (contract.licenses, DenyCheck::Licenses),
        (contract.sources, DenyCheck::Sources),
    ] {
        if codes.contains(&code) {
            return Ok(DenyCode::Check(check));
        }
    }
    Err(format!(
        "cargo-deny {minor} emitted unknown semantic code `{code}`",
        minor = contract.minor
    ))
}

fn contract(version: &str) -> Option<&'static CodeContract> {
    let observed = version.split_whitespace().find_map(|token| {
        let mut parts = token.trim_start_matches('v').split('.');
        let major = parts.next()?;
        let minor = parts.next()?;
        (major.bytes().all(|byte| byte.is_ascii_digit())
            && minor.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| format!("{major}.{minor}"))
    })?;
    CONTRACTS
        .iter()
        .find(|candidate| candidate.minor == observed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_qualified_code_has_one_stable_disposition() {
        for contract in CONTRACTS {
            let version = format!("cargo-deny {}.0", contract.minor);
            for (codes, expected) in [
                (contract.advisories, DenyCheck::Advisories),
                (contract.bans, DenyCheck::Bans),
                (contract.licenses, DenyCheck::Licenses),
                (contract.sources, DenyCheck::Sources),
            ] {
                for code in codes {
                    assert_eq!(classify(&version, code), Ok(DenyCode::Check(expected)));
                }
            }
            for code in ADVISORY_FAILURES {
                assert_eq!(classify(&version, code), Ok(DenyCode::IncompleteEvidence));
            }
        }
        assert_eq!(
            classify("cargo-deny 0.18.3", "not-allowed"),
            Ok(DenyCode::Check(DenyCheck::Bans))
        );
        assert_eq!(
            classify("cargo-deny 0.18.3", "git-source-underspecified"),
            Ok(DenyCode::Check(DenyCheck::Sources))
        );
    }
}
