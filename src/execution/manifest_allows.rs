use std::collections::{BTreeMap, BTreeSet};

use cargo_metadata::Metadata;

use super::messages::{CapturedMessage, CompilerMessageData, ScanExecution};
use crate::policy::PolicyPlan;

pub(super) type Hidden = BTreeMap<(String, String), usize>;

pub(super) fn honor(
    scan: &mut ScanExecution,
    metadata: &Metadata,
    plan: &PolicyPlan,
    allows: &BTreeSet<(String, String)>,
) -> Hidden {
    if allows.is_empty() {
        return Hidden::new();
    }
    let mut sites = BTreeMap::<(String, String), BTreeSet<Site>>::new();
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
        let entry = ((*package).to_owned(), code.to_owned());
        if plan.restamp_level(code).is_some() || !allows.contains(&entry) {
            return true;
        }
        sites.entry(entry).or_default().insert(Site::of(data));
        false
    });
    sites
        .into_iter()
        .map(|(entry, sites)| (entry, sites.len()))
        .collect()
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Site {
    file: Option<String>,
    line: usize,
    column: usize,
    message: String,
}

impl Site {
    fn of(data: &CompilerMessageData) -> Self {
        let primary = data
            .message
            .spans
            .iter()
            .find(|span| span.is_primary)
            .or_else(|| data.message.spans.first());
        Self {
            file: primary.map(|span| span.file_name.clone()),
            line: primary.map_or(0, |span| span.line_start),
            column: primary.map_or(0, |span| span.column_start),
            message: data.message.message.clone(),
        }
    }
}
