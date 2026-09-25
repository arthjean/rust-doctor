//! Clippy's `correctness` group, admitted as one pack.
//!
//! These are the lints the toolchain denies by default because the code they
//! match is outright wrong. The scan carries each one as a warning, like every
//! other catalogued Clippy rule, and switching one off leaves it allowed rather
//! than denied. `docs/correctness-group-2026-09.md` holds the measurement behind
//! both claims and the classification that admitted these 64 members out of the
//! group's 67.

use super::{Producer, RuleDefinition, RuleLevel, RuleTier};

pub(crate) static CLIPPY_ABSURD_EXTREME_COMPARISONS: RuleDefinition = RuleDefinition {
    id: "clippy::absurd_extreme_comparisons",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Rewrite the comparison against the type's real range: as written it always yields the same answer.",
};
pub(crate) static CLIPPY_ALMOST_SWAPPED: RuleDefinition = RuleDefinition {
    id: "clippy::almost_swapped",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Use std::mem::swap: the second assignment reads the value the first one already overwrote.",
};
pub(crate) static CLIPPY_APPROX_CONSTANT: RuleDefinition = RuleDefinition {
    id: "clippy::approx_constant",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Use the matching constant from std::f64::consts (or f32) instead of a truncated literal.",
};
pub(crate) static CLIPPY_ASYNC_YIELDS_ASYNC: RuleDefinition = RuleDefinition {
    id: "clippy::async_yields_async",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Await the inner future inside the block, so the caller receives its output rather than another future.",
};
pub(crate) static CLIPPY_BAD_BIT_MASK: RuleDefinition = RuleDefinition {
    id: "clippy::bad_bit_mask",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Fix the mask or the constant: this bit-mask comparison can never be true, or is always true.",
};
pub(crate) static CLIPPY_CAST_SLICE_DIFFERENT_SIZES: RuleDefinition = RuleDefinition {
    id: "clippy::cast_slice_different_sizes",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Rebuild the slice pointer with std::ptr::slice_from_raw_parts and a length counted in the new element type.",
};
pub(crate) static CLIPPY_CHAR_INDICES_AS_BYTE_INDICES: RuleDefinition = RuleDefinition {
    id: "clippy::char_indices_as_byte_indices",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Iterate with char_indices(), which yields byte offsets, before using the index to slice or index the string.",
};
pub(crate) static CLIPPY_DEPRECATED_SEMVER: RuleDefinition = RuleDefinition {
    id: "clippy::deprecated_semver",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Put a valid semantic version, such as \"1.2.0\", in the since field of #[deprecated].",
};
pub(crate) static CLIPPY_DERIVE_ORD_XOR_PARTIAL_ORD: RuleDefinition = RuleDefinition {
    id: "clippy::derive_ord_xor_partial_ord",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Derive both Ord and PartialOrd, or implement both by hand so they agree.",
};
pub(crate) static CLIPPY_DERIVED_HASH_WITH_MANUAL_EQ: RuleDefinition = RuleDefinition {
    id: "clippy::derived_hash_with_manual_eq",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Derive both Hash and PartialEq, or implement Hash by hand on the same fields PartialEq compares.",
};
pub(crate) static CLIPPY_EAGER_TRANSMUTE: RuleDefinition = RuleDefinition {
    id: "clippy::eager_transmute",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Move the transmute inside a closure passed to then(), so it only runs once the bound check has passed.",
};
pub(crate) static CLIPPY_ENUM_CLIKE_UNPORTABLE_VARIANT: RuleDefinition = RuleDefinition {
    id: "clippy::enum_clike_unportable_variant",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Give the enum a fixed-width repr such as u64, or keep every discriminant within 32 bits.",
};
pub(crate) static CLIPPY_EQ_OP: RuleDefinition = RuleDefinition {
    id: "clippy::eq_op",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Compare against the value you meant: both operands are the same expression, so the result is fixed.",
};
pub(crate) static CLIPPY_ERASING_OP: RuleDefinition = RuleDefinition {
    id: "clippy::erasing_op",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Remove the operation or fix the operand: multiplying by zero or masking with zero always yields zero.",
};
pub(crate) static CLIPPY_IF_LET_MUTEX: RuleDefinition = RuleDefinition {
    id: "clippy::if_let_mutex",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Bind the first lock's result before the if let, so no guard is still held when the else branch locks again.",
};
pub(crate) static CLIPPY_IFS_SAME_COND: RuleDefinition = RuleDefinition {
    id: "clippy::ifs_same_cond",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Change the repeated condition: the later branch is unreachable because an earlier one tests the same thing.",
};
pub(crate) static CLIPPY_IMPL_HASH_BORROW_WITH_STR_AND_BYTES: RuleDefinition = RuleDefinition {
    id: "clippy::impl_hash_borrow_with_str_and_bytes",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Drop the Borrow<[u8]> impl, since str and [u8] hash differently and map lookups through it can fail.",
};
pub(crate) static CLIPPY_IMPOSSIBLE_COMPARISONS: RuleDefinition = RuleDefinition {
    id: "clippy::impossible_comparisons",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Fix the bounds: no value satisfies both comparisons, so the condition is always false.",
};
pub(crate) static CLIPPY_INEFFECTIVE_BIT_MASK: RuleDefinition = RuleDefinition {
    id: "clippy::ineffective_bit_mask",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Drop the mask, which cannot change the comparison's result, or fix the constant it was meant to use.",
};
pub(crate) static CLIPPY_INFINITE_ITER: RuleDefinition = RuleDefinition {
    id: "clippy::infinite_iter",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Bound the iterator with take() or a terminating condition before consuming it.",
};
pub(crate) static CLIPPY_INHERENT_TO_STRING_SHADOW_DISPLAY: RuleDefinition = RuleDefinition {
    id: "clippy::inherent_to_string_shadow_display",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Remove the inherent to_string and let the Display implementation provide it.",
};
pub(crate) static CLIPPY_INLINE_FN_WITHOUT_BODY: RuleDefinition = RuleDefinition {
    id: "clippy::inline_fn_without_body",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Remove #[inline] from the method declaration, or put it on the implementations that have a body.",
};
pub(crate) static CLIPPY_INVERTED_SATURATING_SUB: RuleDefinition = RuleDefinition {
    id: "clippy::inverted_saturating_sub",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Swap the operands: the guard proves the subtraction as written underflows.",
};
pub(crate) static CLIPPY_INVISIBLE_CHARACTERS: RuleDefinition = RuleDefinition {
    id: "clippy::invisible_characters",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Replace the invisible character with its \\u{...} escape, or delete it if it is accidental.",
};
pub(crate) static CLIPPY_ITER_NEXT_LOOP: RuleDefinition = RuleDefinition {
    id: "clippy::iter_next_loop",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Iterate over the collection itself; iterating over next() runs the loop at most once.",
};
pub(crate) static CLIPPY_ITER_SKIP_ZERO: RuleDefinition = RuleDefinition {
    id: "clippy::iter_skip_zero",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Remove skip(0), or skip the number of elements you actually meant to.",
};
pub(crate) static CLIPPY_ITERATOR_STEP_BY_ZERO: RuleDefinition = RuleDefinition {
    id: "clippy::iterator_step_by_zero",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Pass a non-zero step to step_by, which panics on zero.",
};
pub(crate) static CLIPPY_LINT_GROUPS_PRIORITY: RuleDefinition = RuleDefinition {
    id: "clippy::lint_groups_priority",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Give the lint group a lower priority, such as priority = -1, so the individual lint levels in the same table override it.",
};
pub(crate) static CLIPPY_MATCH_STR_CASE_MISMATCH: RuleDefinition = RuleDefinition {
    id: "clippy::match_str_case_mismatch",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Write the match arm in the case the scrutinee was converted to, or it can never match.",
};
pub(crate) static CLIPPY_MEM_REPLACE_WITH_UNINIT: RuleDefinition = RuleDefinition {
    id: "clippy::mem_replace_with_uninit",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Use std::mem::take, or replace with a real value, instead of an uninitialized or zeroed one.",
};
pub(crate) static CLIPPY_MIN_MAX: RuleDefinition = RuleDefinition {
    id: "clippy::min_max",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Fix the bound order, or use clamp(min, max): as nested, the result is always the same constant.",
};
pub(crate) static CLIPPY_MISTYPED_LITERAL_SUFFIXES: RuleDefinition = RuleDefinition {
    id: "clippy::mistyped_literal_suffixes",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Write the suffix with its type prefix, such as _i32 or _u8, instead of a digit group that looks like one.",
};
pub(crate) static CLIPPY_MODULO_ONE: RuleDefinition = RuleDefinition {
    id: "clippy::modulo_one",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Remove the modulo by one, which is always zero, or use the divisor you meant.",
};
pub(crate) static CLIPPY_MUT_FROM_REF: RuleDefinition = RuleDefinition {
    id: "clippy::mut_from_ref",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Take &mut self or a &mut argument, or return a shared reference: a &mut obtained from a & reference is unsound.",
};
pub(crate) static CLIPPY_NEVER_LOOP: RuleDefinition = RuleDefinition {
    id: "clippy::never_loop",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Remove the loop that never iterates twice, or fix its control flow.",
};
pub(crate) static CLIPPY_NON_OCTAL_UNIX_PERMISSIONS: RuleDefinition = RuleDefinition {
    id: "clippy::non_octal_unix_permissions",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Write the permission bits as an octal literal, such as 0o644.",
};
pub(crate) static CLIPPY_NONSENSICAL_OPEN_OPTIONS: RuleDefinition = RuleDefinition {
    id: "clippy::nonsensical_open_options",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Remove the option that contradicts the others, such as truncate without write access.",
};
pub(crate) static CLIPPY_NOT_UNSAFE_PTR_ARG_DEREF: RuleDefinition = RuleDefinition {
    id: "clippy::not_unsafe_ptr_arg_deref",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Mark the function unsafe and document its safety contract, or take a reference instead of a raw pointer.",
};
pub(crate) static CLIPPY_OPTION_ENV_UNWRAP: RuleDefinition = RuleDefinition {
    id: "clippy::option_env_unwrap",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Use env! for a variable required at build time, or handle the None option_env! returns when it is unset.",
};
pub(crate) static CLIPPY_OUT_OF_BOUNDS_INDEXING: RuleDefinition = RuleDefinition {
    id: "clippy::out_of_bounds_indexing",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Index within the array's length, or use get() to handle a range that may exceed it.",
};
pub(crate) static CLIPPY_PANICKING_OVERFLOW_CHECKS: RuleDefinition = RuleDefinition {
    id: "clippy::panicking_overflow_checks",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Use checked_add or overflowing_add: in debug builds the addition panics before the comparison can detect overflow.",
};
pub(crate) static CLIPPY_PANICKING_UNWRAP: RuleDefinition = RuleDefinition {
    id: "clippy::panicking_unwrap",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Remove the unwrap on the branch where the value is known to be None or Err, or fix the condition.",
};
pub(crate) static CLIPPY_POSSIBLE_MISSING_COMMA: RuleDefinition = RuleDefinition {
    id: "clippy::possible_missing_comma",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Add the missing comma between the array elements, or parenthesize the subtraction if it is intended.",
};
pub(crate) static CLIPPY_READ_LINE_WITHOUT_TRIM: RuleDefinition = RuleDefinition {
    id: "clippy::read_line_without_trim",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Trim the line before parsing it: read_line keeps the trailing newline, so the parse always fails.",
};
pub(crate) static CLIPPY_RECURSIVE_FORMAT_IMPL: RuleDefinition = RuleDefinition {
    id: "clippy::recursive_format_impl",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Format the fields instead of self: formatting self inside its own Display or Debug impl recurses forever.",
};
pub(crate) static CLIPPY_REDUNDANT_COMPARISONS: RuleDefinition = RuleDefinition {
    id: "clippy::redundant_comparisons",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Remove the comparison implied by the other one.",
};
pub(crate) static CLIPPY_REVERSED_EMPTY_RANGES: RuleDefinition = RuleDefinition {
    id: "clippy::reversed_empty_ranges",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Write the range from low to high and call rev() to iterate downward.",
};
pub(crate) static CLIPPY_SELF_ASSIGNMENT: RuleDefinition = RuleDefinition {
    id: "clippy::self_assignment",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Assign the value you meant: assigning a place to itself does nothing.",
};
pub(crate) static CLIPPY_SIZE_OF_IN_ELEMENT_COUNT: RuleDefinition = RuleDefinition {
    id: "clippy::size_of_in_element_count",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Pass the element count, not a byte count: this function already multiplies by the element size.",
};
pub(crate) static CLIPPY_SUSPICIOUS_SPLITN: RuleDefinition = RuleDefinition {
    id: "clippy::suspicious_splitn",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Pass a count of at least 2 to splitn, which with 0 or 1 never splits.",
};
pub(crate) static CLIPPY_TRANSMUTE_NULL_TO_FN: RuleDefinition = RuleDefinition {
    id: "clippy::transmute_null_to_fn",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Use Option<fn()> and None instead of transmuting a null pointer into a function pointer.",
};
pub(crate) static CLIPPY_TRANSMUTING_NULL: RuleDefinition = RuleDefinition {
    id: "clippy::transmuting_null",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Use Option<&T> and None instead of transmuting a null pointer into a reference.",
};
pub(crate) static CLIPPY_UNINIT_ASSUMED_INIT: RuleDefinition = RuleDefinition {
    id: "clippy::uninit_assumed_init",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Initialize the value, or keep it as MaybeUninit until every byte is written.",
};
pub(crate) static CLIPPY_UNINIT_VEC: RuleDefinition = RuleDefinition {
    id: "clippy::uninit_vec",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Fill the vector with real values, for instance vec![0; n] or resize, instead of set_len over uninitialized memory.",
};
pub(crate) static CLIPPY_UNIT_CMP: RuleDefinition = RuleDefinition {
    id: "clippy::unit_cmp",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Compare the values the expressions produce, not unit, which always compares equal.",
};
pub(crate) static CLIPPY_UNIT_HASH: RuleDefinition = RuleDefinition {
    id: "clippy::unit_hash",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Hash the value you meant: hashing unit adds nothing to the hasher.",
};
pub(crate) static CLIPPY_UNIT_RETURN_EXPECTING_ORD: RuleDefinition = RuleDefinition {
    id: "clippy::unit_return_expecting_ord",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Return the sort key from the closure: a trailing semicolon makes it return unit.",
};
pub(crate) static CLIPPY_UNSOUND_COLLECTION_TRANSMUTE: RuleDefinition = RuleDefinition {
    id: "clippy::unsound_collection_transmute",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Convert the elements with into_iter().map(...).collect() instead of transmuting the collection.",
};
pub(crate) static CLIPPY_UNUSED_IO_AMOUNT: RuleDefinition = RuleDefinition {
    id: "clippy::unused_io_amount",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Use write_all or read_exact, or handle the byte count write and read return.",
};
pub(crate) static CLIPPY_USELESS_ATTRIBUTE: RuleDefinition = RuleDefinition {
    id: "clippy::useless_attribute",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Move the attribute to the item it should govern, or make it an inner #! attribute if it was meant for the module.",
};
pub(crate) static CLIPPY_VEC_RESIZE_TO_ZERO: RuleDefinition = RuleDefinition {
    id: "clippy::vec_resize_to_zero",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Use clear() to empty the vector, or swap the resize arguments if you meant to grow it.",
};
pub(crate) static CLIPPY_WHILE_IMMUTABLE_CONDITION: RuleDefinition = RuleDefinition {
    id: "clippy::while_immutable_condition",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Update a variable of the condition inside the loop, or the loop never ends once entered.",
};
pub(crate) static CLIPPY_WRONG_TRANSMUTE: RuleDefinition = RuleDefinition {
    id: "clippy::wrong_transmute",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Transmute only between types whose values mean the same thing, or convert with the dedicated method instead.",
};
pub(crate) static CLIPPY_ZST_OFFSET: RuleDefinition = RuleDefinition {
    id: "clippy::zst_offset",
    category: "correctness",
    producer: Producer::Clippy,
    default_level: RuleLevel::Warn,
    tier: RuleTier::P1,
    help: "Remove pointer arithmetic on a zero-sized type, which never moves the pointer.",
};
