//! One score per workspace member, beside the workspace's.
//!
//! A member's score is the core-v4 model run over that member's production
//! lines and the findings attributed to it, nothing else: the formula is the
//! workspace's, and so are the reasons the scan gives against it. A finding no
//! member claims, a manifest-level or repository finding, or one in a file two
//! members reach, weighs on the workspace score alone.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use super::{Audit, AuditScore, ScoreReason, SourceFileInventory, is_scorable_workspace};
use crate::report::{Diagnostic, Status};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PackageAudit {
    pub name: String,
    pub source_files: usize,
    pub production_lines: usize,
    pub score: Option<AuditScore>,
    /// Carried for a narrower scope to rebuild from, never published.
    #[serde(skip)]
    inventory_is_complete: bool,
}

impl PackageAudit {
    pub(super) fn is_valid(&self) -> bool {
        is_scorable_workspace(self.source_files, self.production_lines) == self.score.is_some()
            && self.score.as_ref().is_none_or(AuditScore::is_valid)
    }

    const fn inventory(&self) -> SourceFileInventory {
        SourceFileInventory {
            files: self.source_files,
            production_lines: self.production_lines,
            complete: self.inventory_is_complete,
        }
    }
}

/// Scores every member over its own findings.
pub(super) fn score(
    members: impl IntoIterator<Item = (String, SourceFileInventory)>,
    status: Status,
    reasons: &BTreeSet<ScoreReason>,
    diagnostics: &[Diagnostic],
) -> Vec<PackageAudit> {
    let mut owned = BTreeMap::<&str, Vec<Diagnostic>>::new();
    for diagnostic in diagnostics {
        if let Some(package) = diagnostic.package.as_deref() {
            owned.entry(package).or_default().push(diagnostic.clone());
        }
    }
    members
        .into_iter()
        .map(|(name, inventory)| {
            let own = owned.get(name.as_str()).map_or(&[][..], Vec::as_slice);
            let audit = Audit::build_with_inventory(inventory, status, reasons.clone(), own);
            PackageAudit {
                name,
                source_files: inventory.files,
                production_lines: inventory.production_lines,
                score: audit.score,
                inventory_is_complete: inventory.complete,
            }
        })
        .collect()
}

/// The same members, scored again over a narrower set of findings.
pub(super) fn rescore(
    packages: &[PackageAudit],
    status: Status,
    reasons: &BTreeSet<ScoreReason>,
    diagnostics: &[Diagnostic],
) -> Vec<PackageAudit> {
    score(
        packages
            .iter()
            .map(|package| (package.name.clone(), package.inventory())),
        status,
        reasons,
        diagnostics,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{DiagnosticSource, DiagnosticSpan, Severity};

    fn todo(package: Option<&str>) -> Diagnostic {
        Diagnostic {
            id: format!("todo-{package:?}"),
            source: DiagnosticSource::Clippy,
            code: Some("clippy::todo".to_owned()),
            base_severity: Severity::Warning,
            severity: Severity::Warning,
            category: Some("correctness".to_owned()),
            message: "todo".to_owned(),
            help: None,
            package: package.map(str::to_owned),
            target: None,
            context: None,
            unscored: None,
            path: Some("src/lib.rs".to_owned()),
            span: Some(DiagnosticSpan {
                line_start: 1,
                column_start: 1,
                line_end: 1,
                column_end: 2,
            }),
            related: Vec::new(),
            similarity_basis_points: None,
            complexity: None,
            suggestion: None,
            occurrences: 1,
        }
    }

    #[test]
    fn a_finding_no_member_claims_weighs_on_the_workspace_alone() {
        let inventory = SourceFileInventory {
            files: 1,
            production_lines: 50,
            complete: true,
        };
        let members = || vec![("alpha".to_owned(), inventory), ("beta".to_owned(), inventory)];
        let diagnostics = [todo(Some("alpha")), todo(None)];
        let audit = Audit::build_from_inventory(
            inventory,
            members(),
            Status::Complete,
            BTreeSet::new(),
            &diagnostics,
        );
        assert_eq!(audit.packages.len(), 2);
        let (alpha, beta) = (&audit.packages[0], &audit.packages[1]);
        let clean = Audit::build_from_inventory(inventory, members(), Status::Complete, BTreeSet::new(), &[]);
        let one = Audit::build_from_inventory(
            inventory,
            members(),
            Status::Complete,
            BTreeSet::new(),
            &diagnostics[..1],
        );
        let value = |audit: &Audit| audit.score.as_ref().map(|score| score.value);
        let member = |package: &PackageAudit| package.score.as_ref().map(|score| score.value);
        // alpha is charged its one finding, beta nothing, and the unattributed
        // finding moves the workspace score only.
        assert_eq!(member(alpha), value(&one));
        assert_eq!(member(beta), value(&clean));
        assert_eq!(audit.totals().0.total, 2);
        assert!(audit.is_valid());
        assert_eq!(audit, audit.rebuild_for_scope(Status::Complete, &diagnostics));
    }
}
