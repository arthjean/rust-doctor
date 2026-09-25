//! Negative fixture of the correctness pack.
//!
//! Each member has an idiomatic twin, the form a fix produces, and a twin that
//! keeps the positive form behind a reasoned `#[allow]`. The idiomatic twins are
//! here and the allowed ones in `allowed`, one module each, so a twin that
//! declares a type keeps its name.

mod allowed;

pub use allowed::*;

/// negative_absurd_extreme_comparisons
pub fn negative_absurd_extreme_comparisons(value: u8) -> bool {
    value == u8::MAX
}

/// negative_almost_swapped
pub fn negative_almost_swapped(mut first: u8, mut second: u8) -> (u8, u8) {
    std::mem::swap(&mut first, &mut second);
    (first, second)
}

/// negative_approx_constant
pub fn negative_approx_constant() -> f64 {
    std::f64::consts::PI
}

/// negative_async_yields_async
pub async fn negative_async_yields_async() -> u8 {
    async { async { 1 }.await }.await
}

/// negative_bad_bit_mask
pub fn negative_bad_bit_mask(value: u8) -> bool {
    value & 2 == 2
}

/// negative_cast_slice_different_sizes
pub fn negative_cast_slice_different_sizes(values: &[i32]) -> *const [u8] {
    std::ptr::slice_from_raw_parts(values.as_ptr().cast::<u8>(), std::mem::size_of_val(values))
}

/// negative_char_indices_as_byte_indices
pub fn negative_char_indices_as_byte_indices(text: &str) -> usize {
    let mut total = 0;
    for (index, _) in text.char_indices() {
        total += text.get(index..).map_or(0, str::len);
    }
    total
}

/// negative_deprecated_semver
#[deprecated(since = "1.0.0")]
pub fn negative_deprecated_semver() {}

/// negative_derive_ord_xor_partial_ord
#[derive(PartialOrd, Ord, PartialEq, Eq)]
pub struct NegativeDeriveOrd(u8);

/// negative_derived_hash_with_manual_eq
#[derive(Hash, PartialEq)]
pub struct NegativeDerivedHash(u8);

/// negative_eager_transmute
#[repr(u8)]
pub enum NegativeOpcode {
    Load = 0,
    Store,
}

pub fn negative_eager_transmute(byte: u8) -> Option<NegativeOpcode> {
    (byte < 2).then(|| unsafe { std::mem::transmute::<u8, NegativeOpcode>(byte) })
}

/// negative_enum_clike_unportable_variant
#[repr(u64)]
pub enum NegativeWide {
    Big = 0x1_0000_0000,
}

/// negative_eq_op
pub fn negative_eq_op(value: u8, other: u8) -> bool {
    value == other
}

/// negative_erasing_op
pub fn negative_erasing_op(value: u8) -> u8 {
    value.wrapping_mul(2)
}

/// negative_if_let_mutex
pub fn negative_if_let_mutex(lock: &std::sync::Mutex<u8>) -> u8 {
    let read = lock.lock().map(|guard| *guard);
    read.unwrap_or_else(|_| lock.lock().map_or(0, |guard| *guard))
}

/// negative_ifs_same_cond
pub fn negative_ifs_same_cond(value: u8) -> u8 {
    if value == 1 {
        1
    } else if value == 2 {
        2
    } else {
        3
    }
}

/// negative_impl_hash_borrow_with_str_and_bytes
pub struct NegativeName(String);

impl std::hash::Hash for NegativeName {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl std::borrow::Borrow<str> for NegativeName {
    fn borrow(&self) -> &str {
        &self.0
    }
}

/// negative_impossible_comparisons
pub fn negative_impossible_comparisons(value: u8) -> bool {
    value > 3 && value < 5
}

/// negative_ineffective_bit_mask
pub fn negative_ineffective_bit_mask(value: u8) -> bool {
    value > 3
}

/// negative_infinite_iter
pub fn negative_infinite_iter() -> Vec<u8> {
    std::iter::repeat(1).take(3).collect()
}

/// negative_inherent_to_string_shadow_display
pub struct NegativeLabel;

impl std::fmt::Display for NegativeLabel {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("display")
    }
}

/// negative_inline_fn_without_body
pub trait NegativeInline {
    fn value(&self) -> u8;
}

/// negative_invisible_characters
pub fn negative_invisible_characters() -> &'static str {
    "a\u{200B}b"
}

/// negative_inverted_saturating_sub
pub fn negative_inverted_saturating_sub(first: u32, second: u32) -> u32 {
    first.saturating_sub(second)
}

/// negative_iter_next_loop
pub fn negative_iter_next_loop(values: &[u8]) -> u8 {
    let mut total = 0;
    for value in values {
        total += value;
    }
    total
}

/// negative_iter_skip_zero
pub fn negative_iter_skip_zero(values: &[u8]) -> usize {
    values.iter().skip(1).count()
}

/// negative_iterator_step_by_zero
pub fn negative_iterator_step_by_zero() -> usize {
    (0..10).step_by(2).count()
}

/// negative_match_str_case_mismatch
pub fn negative_match_str_case_mismatch(text: &str) -> u8 {
    match text.to_ascii_lowercase().as_str() {
        "foo" => 1,
        _ => 0,
    }
}

/// negative_mem_replace_with_uninit
pub fn negative_mem_replace_with_uninit(values: &mut Vec<u8>) -> Vec<u8> {
    std::mem::take(values)
}

/// negative_min_max
pub fn negative_min_max(value: i32) -> i32 {
    value.clamp(0, 100)
}

/// negative_mistyped_literal_suffixes
pub fn negative_mistyped_literal_suffixes() -> i32 {
    123_i32
}

/// negative_modulo_one
pub fn negative_modulo_one(value: u8) -> u8 {
    value % 2
}

/// negative_mut_from_ref
pub fn negative_mut_from_ref(value: &mut u8) -> &mut u8 {
    value
}

/// negative_never_loop
pub fn negative_never_loop(values: &[u8]) -> u8 {
    let mut total = 0;
    for value in values {
        if *value == 0 {
            break;
        }
        total += value;
    }
    total
}

/// negative_non_octal_unix_permissions
pub fn negative_non_octal_unix_permissions() -> std::fs::Permissions {
    use std::os::unix::fs::PermissionsExt;
    std::fs::Permissions::from_mode(0o644)
}

/// negative_nonsensical_open_options
pub fn negative_nonsensical_open_options() -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new().write(true).truncate(true).open("data")
}

/// negative_not_unsafe_ptr_arg_deref
/// Reads the byte the pointer designates.
///
/// # Safety
///
/// `pointer` must be non-null, aligned, and valid for a read of one byte.
pub unsafe fn negative_not_unsafe_ptr_arg_deref(pointer: *const u8) -> u8 {
    unsafe { *pointer }
}

/// negative_option_env_unwrap
pub fn negative_option_env_unwrap() -> &'static str {
    env!("CARGO_PKG_NAME")
}

/// negative_out_of_bounds_indexing
pub fn negative_out_of_bounds_indexing() -> usize {
    let values = [1, 2, 3];
    values.get(0..5).map_or(0, <[i32]>::len)
}

/// negative_panicking_overflow_checks
pub fn negative_panicking_overflow_checks(first: u32, second: u32) -> bool {
    first.checked_add(second).is_none()
}

/// negative_panicking_unwrap
pub fn negative_panicking_unwrap(value: Option<u8>) -> u8 {
    value.unwrap_or(0)
}

/// negative_possible_missing_comma
pub fn negative_possible_missing_comma() -> [i32; 6] {
    [
        -1, -2, -3,
        -4, -5, -6,
    ]
}

/// negative_read_line_without_trim
pub fn negative_read_line_without_trim() -> Option<i32> {
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).ok()?;
    line.trim().parse::<i32>().ok()
}

/// negative_recursive_format_impl
pub struct NegativeRecursive;

impl std::fmt::Display for NegativeRecursive {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("recursive")
    }
}

/// negative_redundant_comparisons
pub fn negative_redundant_comparisons(value: u8) -> bool {
    value > 5
}

/// negative_reversed_empty_ranges
pub fn negative_reversed_empty_ranges() -> u8 {
    let mut total = 0;
    for value in (0..10).rev() {
        total += value;
    }
    total
}

/// negative_self_assignment
pub struct NegativePoint {
    pub x: u8,
}

pub fn negative_self_assignment(point: &mut NegativePoint, other: &NegativePoint) {
    point.x = other.x;
}

/// negative_size_of_in_element_count
pub fn negative_size_of_in_element_count(count: usize) -> [u16; 8] {
    let source = [2_u16; 8];
    let mut target = [0_u16; 8];
    unsafe {
        std::ptr::copy_nonoverlapping(source.as_ptr(), target.as_mut_ptr(), count);
    }
    target
}

/// negative_suspicious_splitn
pub fn negative_suspicious_splitn(text: &str) -> usize {
    text.splitn(2, ',').count()
}

/// negative_transmute_null_to_fn
pub fn negative_transmute_null_to_fn() -> Option<fn()> {
    None
}

/// negative_transmuting_null
pub fn negative_transmuting_null() -> Option<&'static u8> {
    None
}

/// negative_uninit_assumed_init
pub fn negative_uninit_assumed_init() -> [u8; 4] {
    [0; 4]
}

/// negative_uninit_vec
pub fn negative_uninit_vec() -> Vec<u8> {
    vec![0; 10]
}

/// negative_unit_cmp
pub fn negative_unit_cmp(value: u8) -> bool {
    let first = value == 0;
    let second = value == 1;
    first == second
}

/// negative_unit_hash
pub fn negative_unit_hash<H: std::hash::Hasher>(value: u8, state: &mut H) {
    use std::hash::Hash;
    value.hash(state);
}

/// negative_unit_return_expecting_ord
pub fn negative_unit_return_expecting_ord(values: &mut [u8]) {
    values.sort_by_key(|value| value.count_ones());
}

/// negative_unsound_collection_transmute
pub fn negative_unsound_collection_transmute(values: Vec<u8>) -> Vec<u32> {
    values.into_iter().map(u32::from).collect()
}

/// negative_unused_io_amount
pub fn negative_unused_io_amount(file: &mut std::fs::File) -> std::io::Result<()> {
    use std::io::Write;
    file.write_all(b"data")
}

/// negative_useless_attribute
extern crate alloc as negative_useless_attribute;

/// negative_vec_resize_to_zero
pub fn negative_vec_resize_to_zero(values: &mut Vec<u8>) {
    values.clear();
}

/// negative_while_immutable_condition
pub fn negative_while_immutable_condition(limit: u8) -> u8 {
    let mut index = 0;
    while index < limit {
        index += 1;
    }
    index
}

/// negative_wrong_transmute
pub fn negative_wrong_transmute(value: f64) -> *const u8 {
    std::ptr::without_provenance(value.to_bits() as usize)
}

/// negative_zst_offset
pub fn negative_zst_offset() -> *const u8 {
    let pointer: *const u8 = &0;
    pointer.wrapping_offset(1)
}
