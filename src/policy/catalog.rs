#[cfg(test)]
use std::collections::BTreeSet;

use serde::Serialize;
#[cfg(test)]
use serde_json::Value;

use super::RuleLevel;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Producer {
    Clippy,
    CargoHealth,
    SourceKernel,
}

/// Criticality of a rule, independent of `default_level` and of the effective
/// severity of a diagnostic.
///
/// The tier only drives the `core-v2` score: it imposes a cap on the dimension
/// concerned and on the overall score. It enters neither `base_severity` nor
/// `fingerprint()`, so it moves no baseline.
///
/// The declared order runs from gravest to least grave: `P0 < P1 < P2 < P3`, so
/// the worst tier of a set is its minimum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum RuleTier {
    P0,
    P1,
    P2,
    P3,
}

impl RuleTier {
    #[cfg(test)]
    pub(crate) const ALL: [Self; 4] = [Self::P0, Self::P1, Self::P2, Self::P3];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::P0 => "P0",
            Self::P1 => "P1",
            Self::P2 => "P2",
            Self::P3 => "P3",
        }
    }

    /// Closed reading of a published tier. Any other value is refused without
    /// echoing the input.
    #[cfg(test)]
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "P0" => Some(Self::P0),
            "P1" => Some(Self::P1),
            "P2" => Some(Self::P2),
            "P3" => Some(Self::P3),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(crate) struct RuleDefinition {
    pub(crate) id: &'static str,
    pub(crate) category: &'static str,
    pub(crate) producer: Producer,
    pub(crate) default_level: RuleLevel,
    pub(crate) tier: RuleTier,
    pub(crate) help: &'static str,
}

/// Admissible categories, sorted: `find`/`validate_catalog` look them up by
/// binary search. Each maps to a score dimension through
/// `audit::category_mapping`, so opening a category makes its dimension
/// reachable.
pub(crate) const CATEGORIES: [&str; 6] = [
    "correctness",
    "dependencies",
    "maintainability",
    "performance",
    "reliability",
    "security",
];

pub(crate) static CLIPPY_ARC_WITH_NON_SEND_SYNC: RuleDefinition = RuleDefinition {
    id: "clippy::arc_with_non_send_sync",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Use Rc for single-threaded sharing, or make the inner value Send and Sync before sharing it across threads.",
};
pub(crate) static CLIPPY_AWAIT_HOLDING_LOCK: RuleDefinition = RuleDefinition {
    id: "clippy::await_holding_lock",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Drop the guard before the await point, or use a lock designed to be held across await.",
};
pub(crate) static CLIPPY_AWAIT_HOLDING_REFCELL_REF: RuleDefinition = RuleDefinition {
    id: "clippy::await_holding_refcell_ref",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Copy the borrowed value and drop the borrow before the await point.",
};
pub(crate) static CLIPPY_DBG_MACRO: RuleDefinition = RuleDefinition {
    id: "clippy::dbg_macro",
    category: "maintainability",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P3,
    help: "Remove dbg! or replace it with intentional logging.",
};
pub(crate) static CLIPPY_EXIT: RuleDefinition = RuleDefinition {
    id: "clippy::exit",
    category: "reliability",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P2,
    help: "Return an error to the caller and let the entry point decide the exit status.",
};
pub(crate) static CLIPPY_EXPECT_USED: RuleDefinition = RuleDefinition {
    id: "clippy::expect_used",
    category: "reliability",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P3,
    help: "Propagate the error with ? or handle the missing value explicitly instead of panicking.",
};
pub(crate) static CLIPPY_FORMAT_COLLECT: RuleDefinition = RuleDefinition {
    id: "clippy::format_collect",
    category: "performance",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P3,
    help: "Write into one String with write! or push_str instead of allocating one String per item.",
};
pub(crate) static CLIPPY_INDEXING_SLICING: RuleDefinition = RuleDefinition {
    id: "clippy::indexing_slicing",
    category: "reliability",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P3,
    help: "Use get or get_mut and handle the absent element instead of indexing, which panics out of bounds.",
};
pub(crate) static CLIPPY_LARGE_TYPES_PASSED_BY_VALUE: RuleDefinition = RuleDefinition {
    id: "clippy::large_types_passed_by_value",
    category: "performance",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P3,
    help: "Pass the large value by reference to avoid copying it at every call.",
};
pub(crate) static CLIPPY_MANUAL_MEMCPY: RuleDefinition = RuleDefinition {
    id: "clippy::manual_memcpy",
    category: "performance",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P3,
    help: "Use copy_from_slice or clone_from_slice instead of copying element by element.",
};
pub(crate) static CLIPPY_MEM_FORGET: RuleDefinition = RuleDefinition {
    id: "clippy::mem_forget",
    category: "reliability",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P2,
    help: "Avoid leaking a value with drop semantics; use an explicit ownership or lifetime strategy.",
};
pub(crate) static CLIPPY_MISSING_SAFETY_DOC: RuleDefinition = RuleDefinition {
    id: "clippy::missing_safety_doc",
    category: "maintainability",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P3,
    help: "Document in a `# Safety` section the invariants the caller must uphold before calling.",
};
pub(crate) static CLIPPY_MUT_MUTEX_LOCK: RuleDefinition = RuleDefinition {
    id: "clippy::mut_mutex_lock",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P2,
    help: "Use get_mut when the mutex is already exclusively borrowed; locking it again can deadlock.",
};
pub(crate) static CLIPPY_NON_SEND_FIELDS_IN_SEND_TY: RuleDefinition = RuleDefinition {
    id: "clippy::non_send_fields_in_send_ty",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Remove the unsafe Send implementation or ensure every field is safe to send between threads.",
};
pub(crate) static CLIPPY_PANIC: RuleDefinition = RuleDefinition {
    id: "clippy::panic",
    category: "reliability",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P3,
    help: "Return an error instead of aborting the process on an input the caller can recover from.",
};
pub(crate) static CLIPPY_PANIC_IN_RESULT_FN: RuleDefinition = RuleDefinition {
    id: "clippy::panic_in_result_fn",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P2,
    help: "A function that already returns Result should report the failure as Err instead of panicking.",
};
pub(crate) static CLIPPY_PERMISSIONS_SET_READONLY_FALSE: RuleDefinition = RuleDefinition {
    id: "clippy::permissions_set_readonly_false",
    category: "security",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Set explicit Unix permission bits instead of clearing readonly on Unix.",
};
pub(crate) static CLIPPY_PRINT_STDERR: RuleDefinition = RuleDefinition {
    id: "clippy::print_stderr",
    category: "maintainability",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P3,
    help: "Write to a caller-provided writer or a logger instead of hard-wiring stderr.",
};
pub(crate) static CLIPPY_PRINT_STDOUT: RuleDefinition = RuleDefinition {
    id: "clippy::print_stdout",
    category: "maintainability",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P3,
    help: "Write to a caller-provided writer or a logger instead of hard-wiring stdout.",
};
pub(crate) static CLIPPY_PTR_ARG: RuleDefinition = RuleDefinition {
    id: "clippy::ptr_arg",
    category: "performance",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P3,
    help: "Take &[T] or &str so callers can pass any borrowed slice without owning one first.",
};
pub(crate) static CLIPPY_RC_BUFFER: RuleDefinition = RuleDefinition {
    id: "clippy::rc_buffer",
    category: "performance",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P3,
    help: "Share the slice itself with Rc<str> or Rc<[T]> instead of wrapping an owned buffer.",
};
pub(crate) static CLIPPY_RC_MUTEX: RuleDefinition = RuleDefinition {
    id: "clippy::rc_mutex",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P2,
    help: "Use RefCell inside Rc for single-threaded sharing, or Arc<Mutex<T>> when the value really crosses threads.",
};
pub(crate) static CLIPPY_REDUNDANT_ALLOCATION: RuleDefinition = RuleDefinition {
    id: "clippy::redundant_allocation",
    category: "performance",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P2,
    help: "Remove the inner allocation; one pointer indirection is enough.",
};
pub(crate) static CLIPPY_STABLE_SORT_PRIMITIVE: RuleDefinition = RuleDefinition {
    id: "clippy::stable_sort_primitive",
    category: "performance",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P3,
    help: "Use sort_unstable on primitives; stability carries no meaning and costs an allocation.",
};
pub(crate) static CLIPPY_STRING_SLICE: RuleDefinition = RuleDefinition {
    id: "clippy::string_slice",
    category: "reliability",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P3,
    help: "Use get on the string range and handle the absent slice; byte indexing panics inside a UTF-8 character.",
};
pub(crate) static CLIPPY_SUSPICIOUS_COMMAND_ARG_SPACE: RuleDefinition = RuleDefinition {
    id: "clippy::suspicious_command_arg_space",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P2,
    help: "Pass each process argument separately instead of embedding spaces in one argument.",
};
pub(crate) static CLIPPY_TODO: RuleDefinition = RuleDefinition {
    id: "clippy::todo",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P2,
    help: "Replace todo! with the intended implementation or remove the reachable placeholder.",
};
pub(crate) static CLIPPY_TOO_MANY_ARGUMENTS: RuleDefinition = RuleDefinition {
    id: "clippy::too_many_arguments",
    category: "maintainability",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P3,
    help: "Group the related parameters into a struct so the signature names what it takes.",
};
pub(crate) static CLIPPY_UNIMPLEMENTED: RuleDefinition = RuleDefinition {
    id: "clippy::unimplemented",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Implement this code path or remove the reachable placeholder.",
};
pub(crate) static CLIPPY_UNNECESSARY_TO_OWNED: RuleDefinition = RuleDefinition {
    id: "clippy::unnecessary_to_owned",
    category: "performance",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P3,
    help: "Pass the borrowed value directly; the callee never needs the owned copy.",
};
pub(crate) static CLIPPY_UNREACHABLE: RuleDefinition = RuleDefinition {
    id: "clippy::unreachable",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P2,
    help: "Make the remaining case explicit or return an error; an unreachable! that is reached aborts the process.",
};
pub(crate) static CLIPPY_UNUSED_ASYNC: RuleDefinition = RuleDefinition {
    id: "clippy::unused_async",
    category: "maintainability",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P3,
    help: "Remove the async marker, or await the work the function was meant to drive.",
};
pub(crate) static CLIPPY_UNWRAP_USED: RuleDefinition = RuleDefinition {
    id: "clippy::unwrap_used",
    category: "reliability",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P3,
    help: "Propagate the error with ? or provide a default instead of panicking on the absent value.",
};
pub(crate) static CLIPPY_USELESS_VEC: RuleDefinition = RuleDefinition {
    id: "clippy::useless_vec",
    category: "performance",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P3,
    help: "Use an array or a slice literal; this value never needs a heap allocation.",
};
pub(crate) static CLIPPY_VEC_INIT_THEN_PUSH: RuleDefinition = RuleDefinition {
    id: "clippy::vec_init_then_push",
    category: "performance",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P3,
    help: "Build the vector with the vec! literal so it is allocated once at its final size.",
};
pub(crate) static CLIPPY_ZOMBIE_PROCESSES: RuleDefinition = RuleDefinition {
    id: "clippy::zombie_processes",
    category: "reliability",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P2,
    help: "Wait on the child process or otherwise reap it before the handle is dropped.",
};
pub(crate) static CARGO_DUPLICATE_MAJOR_VERSIONS: RuleDefinition = RuleDefinition {
    id: "rust_doctor::cargo::duplicate_major_versions",
    category: "dependencies",
    producer: Producer::CargoHealth,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P2,
    help: "Align the requirements so one major version of the crate is resolved; duplicates ship twice and their types do not interoperate.",
};
pub(crate) static CARGO_MISSING_LOCKFILE: RuleDefinition = RuleDefinition {
    id: "rust_doctor::cargo::missing_lockfile",
    category: "dependencies",
    producer: Producer::CargoHealth,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P2,
    help: "Commit Cargo.lock next to the manifest so every build of this binary resolves the same dependency versions.",
};
pub(crate) static CARGO_PATH_DEPENDENCY_OUTSIDE_WORKSPACE: RuleDefinition = RuleDefinition {
    id: "rust_doctor::cargo::path_dependency_outside_workspace",
    category: "dependencies",
    producer: Producer::CargoHealth,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Vendor the crate inside the workspace or publish it; a path leaving the workspace only resolves on the author's machine.",
};
pub(crate) static CARGO_UNBOUNDED_REGISTRY: RuleDefinition = RuleDefinition {
    id: "rust_doctor::cargo::unbounded_registry_dependency",
    category: "reliability",
    producer: Producer::CargoHealth,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P3,
    help: "Replace the unbounded version requirement with the minimum compatible version intended by the project.",
};
pub(crate) static CARGO_UNPINNED_GIT: RuleDefinition = RuleDefinition {
    id: "rust_doctor::cargo::unpinned_git_dependency",
    category: "security",
    producer: Producer::CargoHealth,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Set rev to the full 40-character commit SHA intended by the project.",
};
pub(crate) static SOURCE_DISABLED_TLS: RuleDefinition = RuleDefinition {
    id: "rust_doctor::source::disabled_tls_verification",
    category: "security",
    producer: Producer::SourceKernel,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P0,
    help: "Keep TLS verification enabled and configure the required trust roots or server name instead.",
};
pub(crate) static SOURCE_DYNAMIC_SHELL: RuleDefinition = RuleDefinition {
    id: "rust_doctor::source::dynamic_shell_command",
    category: "security",
    producer: Producer::SourceKernel,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P0,
    help: "Avoid the shell and pass values as separate Command arguments; otherwise apply shell-specific escaping at the trust boundary.",
};

pub(crate) const CATALOG: [&RuleDefinition; 43] = [
    &CLIPPY_ARC_WITH_NON_SEND_SYNC,
    &CLIPPY_AWAIT_HOLDING_LOCK,
    &CLIPPY_AWAIT_HOLDING_REFCELL_REF,
    &CLIPPY_DBG_MACRO,
    &CLIPPY_EXIT,
    &CLIPPY_EXPECT_USED,
    &CLIPPY_FORMAT_COLLECT,
    &CLIPPY_INDEXING_SLICING,
    &CLIPPY_LARGE_TYPES_PASSED_BY_VALUE,
    &CLIPPY_MANUAL_MEMCPY,
    &CLIPPY_MEM_FORGET,
    &CLIPPY_MISSING_SAFETY_DOC,
    &CLIPPY_MUT_MUTEX_LOCK,
    &CLIPPY_NON_SEND_FIELDS_IN_SEND_TY,
    &CLIPPY_PANIC,
    &CLIPPY_PANIC_IN_RESULT_FN,
    &CLIPPY_PERMISSIONS_SET_READONLY_FALSE,
    &CLIPPY_PRINT_STDERR,
    &CLIPPY_PRINT_STDOUT,
    &CLIPPY_PTR_ARG,
    &CLIPPY_RC_BUFFER,
    &CLIPPY_RC_MUTEX,
    &CLIPPY_REDUNDANT_ALLOCATION,
    &CLIPPY_STABLE_SORT_PRIMITIVE,
    &CLIPPY_STRING_SLICE,
    &CLIPPY_SUSPICIOUS_COMMAND_ARG_SPACE,
    &CLIPPY_TODO,
    &CLIPPY_TOO_MANY_ARGUMENTS,
    &CLIPPY_UNIMPLEMENTED,
    &CLIPPY_UNNECESSARY_TO_OWNED,
    &CLIPPY_UNREACHABLE,
    &CLIPPY_UNUSED_ASYNC,
    &CLIPPY_UNWRAP_USED,
    &CLIPPY_USELESS_VEC,
    &CLIPPY_VEC_INIT_THEN_PUSH,
    &CLIPPY_ZOMBIE_PROCESSES,
    &CARGO_DUPLICATE_MAJOR_VERSIONS,
    &CARGO_MISSING_LOCKFILE,
    &CARGO_PATH_DEPENDENCY_OUTSIDE_WORKSPACE,
    &CARGO_UNBOUNDED_REGISTRY,
    &CARGO_UNPINNED_GIT,
    &SOURCE_DISABLED_TLS,
    &SOURCE_DYNAMIC_SHELL,
];

pub(crate) fn find(id: &str) -> Option<&'static RuleDefinition> {
    find_in(&CATALOG, id)
}

pub(super) fn find_in<'a>(catalog: &'a [&RuleDefinition], id: &str) -> Option<&'a RuleDefinition> {
    catalog
        .binary_search_by_key(&id, |definition| definition.id)
        .ok()
        .map(|index| catalog[index])
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CatalogError {
    DuplicateId,
    Unsorted,
    EmptyMetadata,
    UnknownCategory,
    InvalidProducer,
    InvalidTier,
    TierOutsideCategoryWindow,
}

/// Severity window each category admits, from its worst admissible tier to its
/// mildest. The category bounds how bad the defect can be: a maintainability
/// finding is never a `P0`, a security finding is never a `P3`.
///
/// These are the windows the shipped rules occupy. Widening one is a decision
/// about what the score means, so it is made here, once, instead of drifting
/// one rule at a time as the catalog grows.
#[cfg(test)]
const TIER_WINDOWS: [(&str, RuleTier, RuleTier); CATEGORIES.len()] = [
    ("correctness", RuleTier::P1, RuleTier::P2),
    ("dependencies", RuleTier::P1, RuleTier::P2),
    ("maintainability", RuleTier::P3, RuleTier::P3),
    ("performance", RuleTier::P2, RuleTier::P3),
    ("reliability", RuleTier::P2, RuleTier::P3),
    ("security", RuleTier::P0, RuleTier::P1),
];

#[cfg(test)]
fn validate_catalog(catalog: &[&RuleDefinition]) -> Result<(), CatalogError> {
    let mut ids = BTreeSet::new();
    if catalog.iter().any(|definition| !ids.insert(definition.id)) {
        return Err(CatalogError::DuplicateId);
    }
    if catalog.windows(2).any(|pair| pair[0].id > pair[1].id) {
        return Err(CatalogError::Unsorted);
    }
    if catalog.iter().any(|definition| {
        definition.id.is_empty() || definition.category.is_empty() || definition.help.is_empty()
    }) {
        return Err(CatalogError::EmptyMetadata);
    }
    if catalog
        .iter()
        .any(|definition| CATEGORIES.binary_search(&definition.category).is_err())
    {
        return Err(CatalogError::UnknownCategory);
    }
    if catalog.iter().any(|definition| !match definition.producer {
        Producer::Clippy => definition.id.starts_with("clippy::"),
        Producer::CargoHealth => definition.id.starts_with("rust_doctor::cargo::"),
        Producer::SourceKernel => definition.id.starts_with("rust_doctor::source::"),
    }) {
        return Err(CatalogError::InvalidProducer);
    }
    for definition in catalog {
        let published = serde_json::to_value(definition).map_err(|_| CatalogError::InvalidTier)?;
        if published_tier(&published)? != definition.tier {
            return Err(CatalogError::InvalidTier);
        }
        let (_, worst, mildest) = TIER_WINDOWS
            .iter()
            .find(|(category, _, _)| *category == definition.category)
            .ok_or(CatalogError::UnknownCategory)?;
        if definition.tier < *worst || definition.tier > *mildest {
            return Err(CatalogError::TierOutsideCategoryWindow);
        }
    }
    Ok(())
}

/// The type already forbids a missing tier or one outside the four values. The
/// published form, however, is a string: that is where the invalid state
/// becomes representable again, so that is where the validation applies. The
/// error is closed and echoes neither the read value, nor a path, nor an escape
/// sequence.
#[cfg(test)]
fn published_tier(definition: &Value) -> Result<RuleTier, CatalogError> {
    definition
        .get("tier")
        .and_then(Value::as_str)
        .and_then(RuleTier::parse)
        .ok_or(CatalogError::InvalidTier)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::json;

    use super::*;
    use crate::configuration::WorkspaceConfiguration;
    use crate::policy::{PolicyInput, PolicyPlan, RuleLevelSource, active_rules_in, compile_rules};

    static SYNTHETIC_CLIPPY_RULE: RuleDefinition = RuleDefinition {
        id: "clippy::synthetic_rule",
        category: "correctness",
        producer: Producer::Clippy,
        default_level: RuleLevel::Warn,
        tier: RuleTier::P2,
        help: "Synthetic authoring proof.",
    };

    const SYNTHETIC_CATALOG: [&RuleDefinition; 44] = [
        &CLIPPY_ARC_WITH_NON_SEND_SYNC,
        &CLIPPY_AWAIT_HOLDING_LOCK,
        &CLIPPY_AWAIT_HOLDING_REFCELL_REF,
        &CLIPPY_DBG_MACRO,
        &CLIPPY_EXIT,
        &CLIPPY_EXPECT_USED,
        &CLIPPY_FORMAT_COLLECT,
        &CLIPPY_INDEXING_SLICING,
        &CLIPPY_LARGE_TYPES_PASSED_BY_VALUE,
        &CLIPPY_MANUAL_MEMCPY,
        &CLIPPY_MEM_FORGET,
        &CLIPPY_MISSING_SAFETY_DOC,
        &CLIPPY_MUT_MUTEX_LOCK,
        &CLIPPY_NON_SEND_FIELDS_IN_SEND_TY,
        &CLIPPY_PANIC,
        &CLIPPY_PANIC_IN_RESULT_FN,
        &CLIPPY_PERMISSIONS_SET_READONLY_FALSE,
        &CLIPPY_PRINT_STDERR,
        &CLIPPY_PRINT_STDOUT,
        &CLIPPY_PTR_ARG,
        &CLIPPY_RC_BUFFER,
        &CLIPPY_RC_MUTEX,
        &CLIPPY_REDUNDANT_ALLOCATION,
        &CLIPPY_STABLE_SORT_PRIMITIVE,
        &CLIPPY_STRING_SLICE,
        &CLIPPY_SUSPICIOUS_COMMAND_ARG_SPACE,
        &SYNTHETIC_CLIPPY_RULE,
        &CLIPPY_TODO,
        &CLIPPY_TOO_MANY_ARGUMENTS,
        &CLIPPY_UNIMPLEMENTED,
        &CLIPPY_UNNECESSARY_TO_OWNED,
        &CLIPPY_UNREACHABLE,
        &CLIPPY_UNUSED_ASYNC,
        &CLIPPY_UNWRAP_USED,
        &CLIPPY_USELESS_VEC,
        &CLIPPY_VEC_INIT_THEN_PUSH,
        &CLIPPY_ZOMBIE_PROCESSES,
        &CARGO_DUPLICATE_MAJOR_VERSIONS,
        &CARGO_MISSING_LOCKFILE,
        &CARGO_PATH_DEPENDENCY_OUTSIDE_WORKSPACE,
        &CARGO_UNBOUNDED_REGISTRY,
        &CARGO_UNPINNED_GIT,
        &SOURCE_DISABLED_TLS,
        &SOURCE_DYNAMIC_SHELL,
    ];

    fn historical_oracle() -> Value {
        serde_json::from_str(include_str!("../../tests/fixtures/policy-gate/oracle.json"))
            .expect("policy oracle should be valid JSON")
    }

    /// Identifiers published by the historical oracle, in catalog order. The
    /// catalog grows, the oracle does not move: the comparison therefore bears
    /// on this frozen subset, not on a filter by exclusion.
    const HISTORICAL_IDS: [&str; 7] = [
        "clippy::dbg_macro",
        "clippy::todo",
        "clippy::unimplemented",
        "rust_doctor::cargo::unbounded_registry_dependency",
        "rust_doctor::cargo::unpinned_git_dependency",
        "rust_doctor::source::disabled_tls_verification",
        "rust_doctor::source::dynamic_shell_command",
    ];

    #[test]
    fn catalog_is_the_exact_normative_inventory() {
        validate_catalog(&CATALOG).expect("canonical catalog should be valid");
        assert_eq!(CATALOG.len(), 43);
        assert_eq!(
            CATEGORIES,
            [
                "correctness",
                "dependencies",
                "maintainability",
                "performance",
                "reliability",
                "security",
            ]
        );

        let historical: Vec<_> = CATALOG
            .iter()
            .filter(|definition| HISTORICAL_IDS.contains(&definition.id))
            .copied()
            .collect();
        assert_eq!(historical.len(), HISTORICAL_IDS.len());
        assert_eq!(
            serde_json::to_value(historical).expect("historical catalog should serialize"),
            historical_oracle()["catalog"]
        );

        let clippy_ids: Vec<_> = CATALOG
            .iter()
            .filter(|definition| definition.producer == Producer::Clippy)
            .map(|definition| definition.id)
            .collect();
        assert_eq!(clippy_ids.len(), 36);
        assert!(
            clippy_ids
                .iter()
                .all(|id| id
                    .strip_prefix("clippy::")
                    .is_some_and(|lint| !lint.is_empty()
                        && lint
                            .bytes()
                            .all(|byte| byte.is_ascii_lowercase() || byte == b'_')))
        );

        // Every score dimension owns at least three rules, without which it
        // would stay frozen at 100 and its weight would be inert.
        let mut per_dimension = BTreeMap::new();
        for definition in CATALOG {
            let (_, dimension) = crate::audit::category_mapping(definition.category)
                .expect("every catalogued category should map to a dimension");
            *per_dimension.entry(dimension).or_insert(0_usize) += 1;
        }
        assert_eq!(per_dimension.len(), 5);
        assert!(
            per_dimension.values().all(|count| *count >= 3),
            "{per_dimension:?}"
        );
    }

    #[test]
    fn exact_lookup_and_producer_subsets_use_canonical_definitions() {
        assert!(find("clippy::to").is_none());
        assert!(find("clippy::todo_suffix").is_none());
        assert!(find("todo").is_none());
        for definition in CATALOG {
            assert!(std::ptr::eq(find(definition.id).unwrap(), definition));
        }

        let plan = PolicyPlan::default();
        assert_eq!(plan.active_rules(Producer::Clippy).count(), 36);
        assert_eq!(plan.active_rules(Producer::CargoHealth).count(), 5);
        assert_eq!(plan.active_rules(Producer::SourceKernel).count(), 2);
    }

    #[test]
    fn malformed_synthetic_catalogs_fail_deterministically() {
        let mut duplicate = CATALOG;
        duplicate[1] = duplicate[0];
        assert_eq!(validate_catalog(&duplicate), Err(CatalogError::DuplicateId));

        let mut unsorted = CATALOG;
        unsorted.swap(0, 1);
        assert_eq!(validate_catalog(&unsorted), Err(CatalogError::Unsorted));

        static EMPTY_HELP: RuleDefinition = RuleDefinition {
            help: "",
            // Derived from the first catalog entry: the substitution happens at
            // position 0, so the identifier must stay that one for a single
            // defect at a time to be under test.
            ..CLIPPY_ARC_WITH_NON_SEND_SYNC
        };
        let mut empty = CATALOG;
        empty[0] = &EMPTY_HELP;
        assert_eq!(validate_catalog(&empty), Err(CatalogError::EmptyMetadata));

        static UNKNOWN_CATEGORY: RuleDefinition = RuleDefinition {
            category: "style",
            // Derived from the first catalog entry: the substitution happens at
            // position 0, so the identifier must stay that one for a single
            // defect at a time to be under test.
            ..CLIPPY_ARC_WITH_NON_SEND_SYNC
        };
        let mut category = CATALOG;
        category[0] = &UNKNOWN_CATEGORY;
        assert_eq!(
            validate_catalog(&category),
            Err(CatalogError::UnknownCategory)
        );

        static TIER_TOO_MILD: RuleDefinition = RuleDefinition {
            tier: RuleTier::P3,
            // Derived from the first catalog entry: the substitution happens at
            // position 0, so the identifier must stay that one for a single
            // defect at a time to be under test.
            ..CLIPPY_ARC_WITH_NON_SEND_SYNC
        };
        let mut mild = CATALOG;
        mild[0] = &TIER_TOO_MILD;
        assert_eq!(
            validate_catalog(&mild),
            Err(CatalogError::TierOutsideCategoryWindow)
        );

        static TIER_TOO_SEVERE: RuleDefinition = RuleDefinition {
            tier: RuleTier::P0,
            ..CLIPPY_ARC_WITH_NON_SEND_SYNC
        };
        let mut severe = CATALOG;
        severe[0] = &TIER_TOO_SEVERE;
        assert_eq!(
            validate_catalog(&severe),
            Err(CatalogError::TierOutsideCategoryWindow)
        );

        static INVALID_PRODUCER: RuleDefinition = RuleDefinition {
            producer: Producer::SourceKernel,
            // Derived from the first catalog entry: the substitution happens at
            // position 0, so the identifier must stay that one for a single
            // defect at a time to be under test.
            ..CLIPPY_ARC_WITH_NON_SEND_SYNC
        };
        let mut producer = CATALOG;
        producer[0] = &INVALID_PRODUCER;
        assert_eq!(
            validate_catalog(&producer),
            Err(CatalogError::InvalidProducer)
        );

        static HOSTILE_DEFINITION: RuleDefinition = RuleDefinition {
            id: "/private/secret\u{1b}[31m",
            // Derived from the first catalog entry: the substitution happens at
            // position 0, so the identifier must stay that one for a single
            // defect at a time to be under test.
            ..CLIPPY_ARC_WITH_NON_SEND_SYNC
        };
        let error = validate_catalog(&[&HOSTILE_DEFINITION]).unwrap_err();
        let rendered = format!("{error:?}");
        assert_eq!(error, CatalogError::InvalidProducer);
        assert!(!rendered.contains("/private"));
        assert!(!rendered.contains('\u{1b}'));
    }

    /// The tier is a closed value on the published surface.
    ///
    /// In the code, the enumeration makes a missing tier or one outside the
    /// four values impossible to build. In the report, the tier is a string:
    /// that is the only surface where the invalid state exists, so that is the
    /// one the validation closes.
    #[test]
    fn an_absent_or_unknown_published_tier_fails_with_a_closed_error() {
        for definition in CATALOG {
            let published = serde_json::to_value(definition).unwrap();
            assert_eq!(published_tier(&published), Ok(definition.tier));
        }

        for hostile in [
            json!({ "id": "clippy::dbg_macro" }),
            json!({ "id": "clippy::dbg_macro", "tier": null }),
            json!({ "id": "clippy::dbg_macro", "tier": "P4" }),
            json!({ "id": "clippy::dbg_macro", "tier": "p0" }),
            json!({ "id": "clippy::dbg_macro", "tier": 0 }),
            json!({ "id": "clippy::dbg_macro", "tier": "/private/secret\u{1b}[31m" }),
        ] {
            let error = published_tier(&hostile).unwrap_err();
            let rendered = format!("{error:?}");
            assert_eq!(error, CatalogError::InvalidTier);
            assert!(!rendered.contains("/private"));
            assert!(!rendered.contains('\u{1b}'));
        }

        assert_eq!(
            RuleTier::ALL.map(RuleTier::as_str),
            ["P0", "P1", "P2", "P3"]
        );
        assert!(RuleTier::ALL.windows(2).all(|pair| pair[0] < pair[1]));
    }

    /// The tier neither copies the default level nor replaces it.
    ///
    /// The twelve rules share `RuleLevel::Warn` but cover four tiers, so no
    /// score function can deduce one from the other.
    #[test]
    fn the_tier_is_independent_from_the_default_level() {
        assert!(
            CATALOG
                .iter()
                .all(|definition| definition.default_level == RuleLevel::Warn)
        );
        let tiers: BTreeSet<_> = CATALOG.iter().map(|definition| definition.tier).collect();
        assert_eq!(tiers.len(), 4);

        let blocking: Vec<_> = CATALOG
            .iter()
            .filter(|definition| definition.tier == RuleTier::P0)
            .map(|definition| definition.id)
            .collect();
        assert_eq!(
            blocking,
            [
                "rust_doctor::source::disabled_tls_verification",
                "rust_doctor::source::dynamic_shell_command",
            ],
            "the P0 set stays restricted to exploitable security detectors",
        );
    }

    #[test]
    fn synthetic_thirteenth_clippy_rule_crosses_catalog_lookup_policy_and_arguments() {
        validate_catalog(&SYNTHETIC_CATALOG).expect("synthetic catalog should be valid");
        assert!(std::ptr::eq(
            find_in(&SYNTHETIC_CATALOG, SYNTHETIC_CLIPPY_RULE.id).unwrap(),
            &SYNTHETIC_CLIPPY_RULE
        ));

        let input = PolicyInput::default()
            .with_rule(SYNTHETIC_CLIPPY_RULE.id, RuleLevel::Error)
            .with_category("correctness", RuleLevel::Off);
        let rules = compile_rules(
            &SYNTHETIC_CATALOG,
            &input,
            &WorkspaceConfiguration::default(),
        )
        .expect("synthetic policy should compile");
        let synthetic = rules
            .iter()
            .find(|rule| rule.definition.id == SYNTHETIC_CLIPPY_RULE.id)
            .unwrap();
        assert_eq!(synthetic.level, RuleLevel::Error);
        assert_eq!(synthetic.source, RuleLevelSource::RequestRule);
        assert!(synthetic.restamp);
        assert_eq!(
            crate::execution::clippy_arguments_for_rules(active_rules_in(&rules, Producer::Clippy)),
            [
                "clippy",
                "--workspace",
                "--no-deps",
                "--message-format=json",
                "--",
                "-A",
                "clippy::all",
                "-W",
                "clippy::dbg_macro",
                "-W",
                "clippy::exit",
                "-W",
                "clippy::expect_used",
                "-W",
                "clippy::format_collect",
                "-W",
                "clippy::indexing_slicing",
                "-W",
                "clippy::large_types_passed_by_value",
                "-W",
                "clippy::manual_memcpy",
                "-W",
                "clippy::mem_forget",
                "-W",
                "clippy::missing_safety_doc",
                "-W",
                "clippy::panic",
                "-W",
                "clippy::permissions_set_readonly_false",
                "-W",
                "clippy::print_stderr",
                "-W",
                "clippy::print_stdout",
                "-W",
                "clippy::ptr_arg",
                "-W",
                "clippy::rc_buffer",
                "-W",
                "clippy::redundant_allocation",
                "-W",
                "clippy::stable_sort_primitive",
                "-W",
                "clippy::string_slice",
                "-W",
                "clippy::synthetic_rule",
                "-W",
                "clippy::too_many_arguments",
                "-W",
                "clippy::unnecessary_to_owned",
                "-W",
                "clippy::unused_async",
                "-W",
                "clippy::unwrap_used",
                "-W",
                "clippy::useless_vec",
                "-W",
                "clippy::vec_init_then_push",
                "-W",
                "clippy::zombie_processes",
            ]
        );
    }

    /// US-072: a category override bears on every rule of the opened category,
    /// `performance` like the others.
    #[test]
    fn a_category_override_reaches_every_rule_of_an_opened_category() {
        for category in ["performance", "dependencies"] {
            let input = PolicyInput::default().with_category(category, RuleLevel::Off);
            let rules = compile_rules(&CATALOG, &input, &WorkspaceConfiguration::default())
                .expect("an opened category should compile");
            let targeted: Vec<_> = rules
                .iter()
                .filter(|rule| rule.definition.category == category)
                .collect();

            assert!(targeted.len() >= 3, "{category}");
            assert!(
                targeted.iter().all(|rule| rule.level == RuleLevel::Off
                    && rule.source == RuleLevelSource::RequestCategory),
                "{category}"
            );
            assert!(
                rules
                    .iter()
                    .filter(|rule| rule.definition.category != category)
                    .all(|rule| rule.level == RuleLevel::Warn),
                "{category}"
            );
        }
    }
}
