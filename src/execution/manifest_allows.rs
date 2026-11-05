use std::collections::{BTreeMap, BTreeSet};

use cargo_metadata::Metadata;

use super::messages::{CapturedMessage, ScanExecution};
use crate::policy::PolicyPlan;

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
