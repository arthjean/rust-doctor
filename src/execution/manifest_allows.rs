//! The Clippy findings a member's manifest already declined.
//!
//! The scan passes every catalogued lint as `-W`, and a command-line level
//! overrides a `[lints]` table, so an `allow` there silences nothing and the
//! report used to charge both the site and the allow. The `-W` cannot be
//! dropped instead: it applies to the whole workspace, and one member's allow
//! is not every member's. So the findings are filtered per package once Clippy
//! has run, and the allow itself stays reported, once, by
//! `permissive_lint_table`.
//!
//! A level the request set wins over the manifest: `--rule X=warn` or a
//! `rust-doctor.toml` entry is a reader asking to see the sites.

use std::collections::{BTreeMap, BTreeSet};

use cargo_metadata::Metadata;

use super::messages::{CapturedMessage, ScanExecution};
use crate::policy::PolicyPlan;

/// Drops the compiler messages of `scan` whose package and lint an effective
/// manifest allow names, unless the plan restamped that lint's level.
pub(super) fn honor(
    scan: &mut ScanExecution,
    metadata: &Metadata,
    plan: &PolicyPlan,
    allows: &BTreeSet<(String, String)>,
) {
    if allows.is_empty() {
        return;
    }
    let names: BTreeMap<&str, &str> = metadata
        .packages
        .iter()
        .map(|package| (package.id.repr.as_str(), package.name.as_str()))
        .collect();
    scan.messages.retain(|message| {
        let CapturedMessage::Compiler(data) = message else {
            return true;
        };
        let Some(code) = data.message.code.as_ref().map(|code| code.code.as_str()) else {
            return true;
        };
        let Some(package) = names.get(data.package_id.as_str()) else {
            return true;
        };
        plan.restamp_level(code).is_some()
            || !allows.contains(&((*package).to_owned(), code.to_owned()))
    });
}
