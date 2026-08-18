use serde::Serialize;

use super::RuleLevel;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod validate;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Producer {
    Clippy,
    CargoHealth,
    SourceKernel,
    Structure,
    Repo,
}

/// One catalogued rule, as the outside world reads it.
///
/// `RuleDefinition` stays crate-private because it is the shape the scan
/// compiles against; this is the published projection of it, and the reason the
/// website can state what the tool checks without anyone retyping the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct CatalogEntry {
    pub id: &'static str,
    pub category: &'static str,
    pub producer: Producer,
    pub default_level: RuleLevel,
    pub tier: RuleTier,
    pub help: &'static str,
}

/// Every catalogued rule, in catalog order.
#[must_use]
pub fn catalog() -> Vec<CatalogEntry> {
    CATALOG
        .iter()
        .map(|definition| CatalogEntry {
            id: definition.id,
            category: definition.category,
            producer: definition.producer,
            default_level: definition.default_level,
            tier: definition.tier,
            help: definition.help,
        })
        .collect()
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
    /// echoing the input. It is what every frozen record compares against, so
    /// it lives with the type rather than with any one of its readers.
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
pub(crate) static CLIPPY_TYPE_COMPLEXITY: RuleDefinition = RuleDefinition {
    id: "clippy::type_complexity",
    category: "maintainability",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P3,
    help: "Name the nested type with a type alias or a dedicated struct so signatures say what they carry.",
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
pub(crate) static CARGO_PERMISSIVE_LINT_TABLE: RuleDefinition = RuleDefinition {
    id: "rust_doctor::cargo::permissive_lint_table",
    category: "reliability",
    producer: Producer::CargoHealth,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P2,
    help: "Remove the allow entry from [lints] and fix what it silences; a manifest-level allow hides the rule from every scan of this workspace.",
};
pub(crate) static CARGO_PERMISSIVE_RUSTFLAGS: RuleDefinition = RuleDefinition {
    id: "rust_doctor::cargo::permissive_rustflags",
    // Reliability at P2, like the manifest lint table: a flag that caps or
    // silences lints neutralizes the scan itself for every build of the
    // workspace, which is graver than weakening the shipped artifact.
    category: "reliability",
    producer: Producer::CargoHealth,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P2,
    help: "Remove the flag from .cargo/config.toml and fix what it silences; the closed list judged here is --cap-lints allow, -A warnings and -C overflow-checks=off, each of which disables a check for every build of this workspace.",
};
pub(crate) static CARGO_RELEASE_DEBUG_SYMBOLS: RuleDefinition = RuleDefinition {
    id: "rust_doctor::cargo::release_debug_symbols",
    category: "reliability",
    producer: Producer::CargoHealth,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P3,
    help: "Set strip = \"symbols\" or remove debug from [profile.release]: full debug info ships absolute build paths inside the binary. Only [profile.release] itself is judged; profiles inheriting from it are not resolved.",
};
pub(crate) static CARGO_TEST_ONLY_DEPENDENCY: RuleDefinition = RuleDefinition {
    id: "rust_doctor::cargo::test_only_dependency",
    category: "dependencies",
    producer: Producer::CargoHealth,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P2,
    help: "Move the entry to [dev-dependencies]: only the test suite references it, and every consumer of the library compiles it anyway.",
};
pub(crate) static CARGO_UNUSED_DEPENDENCY: RuleDefinition = RuleDefinition {
    id: "rust_doctor::cargo::unused_dependency",
    category: "dependencies",
    producer: Producer::CargoHealth,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P2,
    help: "Remove the entry no source references, or switch this rule off with --rule or rust-doctor.toml for a crate needed for linking alone; references made only through macro expansion or doctests are not seen.",
};
pub(crate) static CARGO_UNCHECKED_RELEASE_OVERFLOW: RuleDefinition = RuleDefinition {
    id: "rust_doctor::cargo::unchecked_release_overflow",
    // The rule states a tradeoff, not a verdict: the Rust Performance Book
    // measures overflow checks at a few percent on integer-heavy code, and the
    // help names that cost so the reader can decline it.
    category: "reliability",
    producer: Producer::CargoHealth,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P3,
    help: "Set overflow-checks = true under [profile.release] so integer overflow panics instead of wrapping silently; the measured cost is a few percent on integer-heavy code, which is the tradeoff this finding asks you to decide.",
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
pub(crate) static REPO_HARDCODED_CREDENTIAL: RuleDefinition = RuleDefinition {
    id: "rust_doctor::repo::hardcoded_credential",
    category: "security",
    producer: Producer::Repo,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Remove the credential from the source and rotate it; the closed list judged here is AKIA, ghp_, github_pat_, sk-, xoxb- and BEGIN PRIVATE KEY blocks, and the matched value is never republished by the report.",
};
pub(crate) static REPO_TRACKED_SECRET_FILE: RuleDefinition = RuleDefinition {
    id: "rust_doctor::repo::tracked_secret_file",
    category: "security",
    producer: Producer::Repo,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Remove the file from version control with git rm --cached, rotate what it contains, and add its name to .gitignore so it stays out; the report names the path and never its contents.",
};
pub(crate) static REPO_UNIGNORED_BUILD_OUTPUT: RuleDefinition = RuleDefinition {
    id: "rust_doctor::repo::unignored_build_output",
    category: "maintainability",
    producer: Producer::Repo,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P3,
    help: "Add the target directory to .gitignore so build artifacts stay out of the repository; the directory judged is Cargo's, including a custom target-dir set in .cargo/config.toml, and any ignore source git honors counts.",
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
pub(crate) static STRUCTURE_COMPLEX_FUNCTION: RuleDefinition = RuleDefinition {
    id: "rust_doctor::structure::complex_function",
    category: "maintainability",
    producer: Producer::Structure,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P3,
    help: "Split the branching into smaller functions, or flatten it with early returns, so one reading holds the whole path.",
};
pub(crate) static STRUCTURE_CRATE_LEVEL_ALLOW: RuleDefinition = RuleDefinition {
    id: "rust_doctor::structure::crate_level_allow",
    // Reliability, like the manifest lint table: a file-wide allow neutralizes
    // the scan for everything the file will ever contain, which is a different
    // act from an untidy exemption on one item.
    category: "reliability",
    producer: Producer::Structure,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P2,
    help: "Scope the allow to the item that needs it; a file-wide or module-wide exemption, reasoned or not, also silences every future finding in its reach.",
};
pub(crate) static STRUCTURE_DUPLICATE_FUNCTION_BODY: RuleDefinition = RuleDefinition {
    id: "rust_doctor::structure::duplicate_function_body",
    category: "maintainability",
    producer: Producer::Structure,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P3,
    help: "Keep one definition and call it from the other sites, or make what differs between them a parameter.",
};
pub(crate) static STRUCTURE_NEAR_DUPLICATE_FUNCTION_BODY: RuleDefinition = RuleDefinition {
    id: "rust_doctor::structure::near_duplicate_function_body",
    category: "maintainability",
    producer: Producer::Structure,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P3,
    help: "Factor the shared shape into one function and pass what differs, or keep both and record why they must stay apart.",
};
pub(crate) static STRUCTURE_ORPHAN_MODULE_FILE: RuleDefinition = RuleDefinition {
    id: "rust_doctor::structure::orphan_module_file",
    category: "maintainability",
    producer: Producer::Structure,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P3,
    help: "Declare the file with a mod declaration, or delete it: Cargo compiles no file the module tree does not reach.",
};
pub(crate) static STRUCTURE_OVERSIZED_UNIT: RuleDefinition = RuleDefinition {
    id: "rust_doctor::structure::oversized_unit",
    category: "maintainability",
    producer: Producer::Structure,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P3,
    help: "Split the file, function, impl block or module along its responsibilities before growth makes the split harder.",
};
pub(crate) static STRUCTURE_STACKED_ALLOW: RuleDefinition = RuleDefinition {
    id: "rust_doctor::structure::stacked_allow_attribute",
    category: "maintainability",
    producer: Producer::Structure,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P3,
    help: "Keep the one exemption the item actually needs and fix what the others hide; attributes produced by cfg_attr are not counted here.",
};
pub(crate) static STRUCTURE_UNREASONED_ALLOW: RuleDefinition = RuleDefinition {
    id: "rust_doctor::structure::unreasoned_allow_attribute",
    category: "maintainability",
    producer: Producer::Structure,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P3,
    help: "Fix what the lint reports, or keep the allow and state why with reason = \"...\" so the exemption survives review.",
};
pub(crate) static STRUCTURE_UNREFERENCED_FEATURE: RuleDefinition = RuleDefinition {
    id: "rust_doctor::structure::unreferenced_feature",
    category: "maintainability",
    producer: Producer::Structure,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P3,
    help: "Delete the feature nothing reads, or declare the one a cfg already gates; switch this rule off for a stub deliberately kept as published surface.",
};

pub(crate) const CATALOG: [&RuleDefinition; 62] = [
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
    &CLIPPY_TYPE_COMPLEXITY,
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
    &CARGO_PERMISSIVE_LINT_TABLE,
    &CARGO_PERMISSIVE_RUSTFLAGS,
    &CARGO_RELEASE_DEBUG_SYMBOLS,
    &CARGO_TEST_ONLY_DEPENDENCY,
    &CARGO_UNBOUNDED_REGISTRY,
    &CARGO_UNCHECKED_RELEASE_OVERFLOW,
    &CARGO_UNPINNED_GIT,
    &CARGO_UNUSED_DEPENDENCY,
    &REPO_HARDCODED_CREDENTIAL,
    &REPO_TRACKED_SECRET_FILE,
    &REPO_UNIGNORED_BUILD_OUTPUT,
    &SOURCE_DISABLED_TLS,
    &SOURCE_DYNAMIC_SHELL,
    &STRUCTURE_COMPLEX_FUNCTION,
    &STRUCTURE_CRATE_LEVEL_ALLOW,
    &STRUCTURE_DUPLICATE_FUNCTION_BODY,
    &STRUCTURE_NEAR_DUPLICATE_FUNCTION_BODY,
    &STRUCTURE_ORPHAN_MODULE_FILE,
    &STRUCTURE_OVERSIZED_UNIT,
    &STRUCTURE_STACKED_ALLOW,
    &STRUCTURE_UNREASONED_ALLOW,
    &STRUCTURE_UNREFERENCED_FEATURE,
];

pub(crate) fn find(id: &str) -> Option<&'static RuleDefinition> {
    find_in(&CATALOG, id)
}

pub(super) fn find_in<'a>(catalog: &'a [&RuleDefinition], id: &str) -> Option<&'a RuleDefinition> {
    super::by_id(catalog, id, |definition| definition.id).copied()
}
