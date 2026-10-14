use serde::Serialize;

use super::RuleLevel;
use clippy::*;
use correctness::*;
pub(crate) use native::*;

mod clippy;
mod correctness;
mod native;
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

impl Producer {
    /// The name `rules list --json` publishes.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Clippy => "clippy",
            Self::CargoHealth => "cargo-health",
            Self::SourceKernel => "source-kernel",
            Self::Structure => "structure",
            Self::Repo => "repo",
        }
    }
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
    pub diff_scope: DiffScope,
    /// What the rule catches, in at most eight words.
    pub title: &'static str,
    /// When a finding of the rule is not a defect: the check to apply before
    /// editing the code.
    pub not_a_defect_when: &'static str,
    /// The crates the rule's detectors match and the versions they were
    /// validated against.
    pub requires: &'static [CrateGate],
}

/// A crate a rule's detectors match, and the version range they were written
/// against, as a Cargo requirement: `>=1, <2`, or `*` for any version.
///
/// A unit is judged when at least one gated crate reaches it and every gated
/// crate that does sits inside its range: CORS is read from `tower-http` or
/// from `actix-cors`, and a unit with neither has nothing to read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct CrateGate {
    #[serde(rename = "crate")]
    pub krate: &'static str,
    pub versions: &'static str,
}

/// How far from the site a rule reports the change that caused it can sit,
/// which is what `--scope contextual` reads, as react-doctor's per-rule
/// `diffScope` does.
///
/// Most rules report the line that holds the defect, and a lines scope judges
/// them exactly. A few report a site the change did not touch: Clippy puts
/// `await_holding_lock` on the guard's `let` when the edit added an `.await`
/// below it, and `blocking_call_in_async` turns on an `async` keyword above
/// the call. Those reach their enclosing function. A span-less finding about a
/// lockfile reaches its whole file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DiffScope {
    Line,
    Item,
    File,
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
            diff_scope: definition.diff_scope,
            title: definition.title,
            not_a_defect_when: definition.not_a_defect_when,
            requires: definition.requires,
        })
        .collect()
}

/// Criticality of a rule, independent of `default_level` and of the effective
/// severity of a diagnostic.
///
/// The tier only drives the `core-v3` score: it imposes a cap on the dimension
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
    pub(crate) diff_scope: DiffScope,
    pub(crate) title: &'static str,
    pub(crate) not_a_defect_when: &'static str,
    pub(crate) requires: &'static [CrateGate],
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

pub(crate) const CATALOG: [&RuleDefinition; 142] = [
    &CLIPPY_ABSURD_EXTREME_COMPARISONS,
    &CLIPPY_ALMOST_SWAPPED,
    &CLIPPY_APPROX_CONSTANT,
    &CLIPPY_ARC_WITH_NON_SEND_SYNC,
    &CLIPPY_ASYNC_YIELDS_ASYNC,
    &CLIPPY_AWAIT_HOLDING_LOCK,
    &CLIPPY_AWAIT_HOLDING_REFCELL_REF,
    &CLIPPY_BAD_BIT_MASK,
    &CLIPPY_CAST_SLICE_DIFFERENT_SIZES,
    &CLIPPY_CHAR_INDICES_AS_BYTE_INDICES,
    &CLIPPY_CROSSPOINTER_TRANSMUTE,
    &CLIPPY_DBG_MACRO,
    &CLIPPY_DEPRECATED_SEMVER,
    &CLIPPY_DERIVE_ORD_XOR_PARTIAL_ORD,
    &CLIPPY_DERIVED_HASH_WITH_MANUAL_EQ,
    &CLIPPY_EAGER_TRANSMUTE,
    &CLIPPY_ENUM_CLIKE_UNPORTABLE_VARIANT,
    &CLIPPY_EQ_OP,
    &CLIPPY_ERASING_OP,
    &CLIPPY_EXIT,
    &CLIPPY_EXPECT_USED,
    &CLIPPY_FORMAT_COLLECT,
    &CLIPPY_IF_LET_MUTEX,
    &CLIPPY_IFS_SAME_COND,
    &CLIPPY_IMPL_HASH_BORROW_WITH_STR_AND_BYTES,
    &CLIPPY_IMPOSSIBLE_COMPARISONS,
    &CLIPPY_INDEXING_SLICING,
    &CLIPPY_INEFFECTIVE_BIT_MASK,
    &CLIPPY_INFINITE_ITER,
    &CLIPPY_INHERENT_TO_STRING_SHADOW_DISPLAY,
    &CLIPPY_INLINE_FN_WITHOUT_BODY,
    &CLIPPY_INVERTED_SATURATING_SUB,
    &CLIPPY_INVISIBLE_CHARACTERS,
    &CLIPPY_ITER_NEXT_LOOP,
    &CLIPPY_ITER_SKIP_ZERO,
    &CLIPPY_ITERATOR_STEP_BY_ZERO,
    &CLIPPY_LARGE_TYPES_PASSED_BY_VALUE,
    &CLIPPY_LINT_GROUPS_PRIORITY,
    &CLIPPY_MANUAL_MEMCPY,
    &CLIPPY_MATCH_STR_CASE_MISMATCH,
    &CLIPPY_MEM_FORGET,
    &CLIPPY_MEM_REPLACE_WITH_UNINIT,
    &CLIPPY_MIN_MAX,
    &CLIPPY_MISSING_SAFETY_DOC,
    &CLIPPY_MISSING_TRANSMUTE_ANNOTATIONS,
    &CLIPPY_MISTYPED_LITERAL_SUFFIXES,
    &CLIPPY_MODULO_ONE,
    &CLIPPY_MUT_FROM_REF,
    &CLIPPY_MUT_MUTEX_LOCK,
    &CLIPPY_NEVER_LOOP,
    &CLIPPY_NON_OCTAL_UNIX_PERMISSIONS,
    &CLIPPY_NON_SEND_FIELDS_IN_SEND_TY,
    &CLIPPY_NONSENSICAL_OPEN_OPTIONS,
    &CLIPPY_NOT_UNSAFE_PTR_ARG_DEREF,
    &CLIPPY_OPTION_ENV_UNWRAP,
    &CLIPPY_OUT_OF_BOUNDS_INDEXING,
    &CLIPPY_PANIC,
    &CLIPPY_PANIC_IN_RESULT_FN,
    &CLIPPY_PANICKING_OVERFLOW_CHECKS,
    &CLIPPY_PANICKING_UNWRAP,
    &CLIPPY_PERMISSIONS_SET_READONLY_FALSE,
    &CLIPPY_POSSIBLE_MISSING_COMMA,
    &CLIPPY_PRINT_STDERR,
    &CLIPPY_PRINT_STDOUT,
    &CLIPPY_PTR_ARG,
    &CLIPPY_RC_BUFFER,
    &CLIPPY_RC_MUTEX,
    &CLIPPY_READ_LINE_WITHOUT_TRIM,
    &CLIPPY_RECURSIVE_FORMAT_IMPL,
    &CLIPPY_REDUNDANT_ALLOCATION,
    &CLIPPY_REDUNDANT_COMPARISONS,
    &CLIPPY_REVERSED_EMPTY_RANGES,
    &CLIPPY_SELF_ASSIGNMENT,
    &CLIPPY_SIZE_OF_IN_ELEMENT_COUNT,
    &CLIPPY_STABLE_SORT_PRIMITIVE,
    &CLIPPY_STRING_SLICE,
    &CLIPPY_SUSPICIOUS_COMMAND_ARG_SPACE,
    &CLIPPY_SUSPICIOUS_SPLITN,
    &CLIPPY_TODO,
    &CLIPPY_TOO_MANY_ARGUMENTS,
    &CLIPPY_TRANSMUTE_NULL_TO_FN,
    &CLIPPY_TRANSMUTING_NULL,
    &CLIPPY_TYPE_COMPLEXITY,
    &CLIPPY_UNDOCUMENTED_UNSAFE_BLOCKS,
    &CLIPPY_UNIMPLEMENTED,
    &CLIPPY_UNINIT_ASSUMED_INIT,
    &CLIPPY_UNINIT_VEC,
    &CLIPPY_UNIT_CMP,
    &CLIPPY_UNIT_HASH,
    &CLIPPY_UNIT_RETURN_EXPECTING_ORD,
    &CLIPPY_UNNECESSARY_TO_OWNED,
    &CLIPPY_UNREACHABLE,
    &CLIPPY_UNSOUND_COLLECTION_TRANSMUTE,
    &CLIPPY_UNUSED_ASYNC,
    &CLIPPY_UNUSED_IO_AMOUNT,
    &CLIPPY_UNWRAP_USED,
    &CLIPPY_USELESS_ATTRIBUTE,
    &CLIPPY_USELESS_VEC,
    &CLIPPY_VEC_INIT_THEN_PUSH,
    &CLIPPY_VEC_RESIZE_TO_ZERO,
    &CLIPPY_WHILE_IMMUTABLE_CONDITION,
    &CLIPPY_WRONG_TRANSMUTE,
    &CLIPPY_ZOMBIE_PROCESSES,
    &CLIPPY_ZST_OFFSET,
    &CARGO_DUPLICATE_MAJOR_VERSIONS,
    &CARGO_MISSING_LOCKFILE,
    &CARGO_PATH_DEPENDENCY_OUTSIDE_WORKSPACE,
    &CARGO_PERMISSIVE_LINT_TABLE,
    &CARGO_PERMISSIVE_RUSTFLAGS,
    &CARGO_RELEASE_DEBUG_SYMBOLS,
    &CARGO_TEST_ONLY_DEPENDENCY,
    &CARGO_UNBOUNDED_REGISTRY,
    &CARGO_UNCHECKED_RELEASE_OVERFLOW,
    &CARGO_UNMAINTAINED_DEPENDENCY,
    &CARGO_UNPINNED_GIT,
    &CARGO_UNUSED_DEPENDENCY,
    &CARGO_VULNERABLE_DEPENDENCY,
    &REPO_HARDCODED_CREDENTIAL,
    &REPO_IGNORED_LOCKFILE,
    &REPO_SECRET_IN_BUILD_STEP,
    &REPO_TRACKED_SECRET_FILE,
    &REPO_UNIGNORED_BUILD_OUTPUT,
    &SOURCE_BLOCK_IN_PLACE_CURRENT_THREAD,
    &SOURCE_BLOCK_ON_IN_ASYNC,
    &SOURCE_BLOCKING_IN_ASYNC,
    &SOURCE_DISABLED_JWT_VALIDATION,
    &SOURCE_DISABLED_TLS,
    &SOURCE_DYNAMIC_SHELL,
    &SOURCE_INSECURE_AUTH_COOKIE,
    &SOURCE_PERMISSIVE_CORS,
    &SOURCE_SECRET_ENV_FALLBACK,
    &SOURCE_SQL_FROM_FORMAT,
    &SOURCE_UNCHECKED_HTTP_STATUS,
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
