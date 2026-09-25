//! Positive fixture of the correctness pack.
//!
//! Every item triggers exactly one member of Clippy's `correctness` group, the
//! lints the toolchain denies by default. The scan carries them as warnings:
//! `docs/correctness-group-2026-09.md` records the flag order that allows it and
//! the classification that admitted each member. The crate is edition 2021
//! because `if_let_mutex` stays silent from edition 2024 on, where if-let
//! rescoping removed the deadlock it reports.
//!
//! A positive that also trips a catalogued rule of another pack carries a
//! reasoned `#[allow]` of that rule alone, so each item reports one lint.

mod negatives;

pub use negatives::*;

/// clippy::absurd_extreme_comparisons
pub fn positive_absurd_extreme_comparisons(value: u8) -> bool {
    value > u8::MAX
}

/// clippy::almost_swapped
#[allow(unused_assignments, reason = "the admission contract requires a silent counterpart")]
pub fn positive_almost_swapped(mut first: u8, mut second: u8) -> (u8, u8) {
    first = second;
    second = first;
    (first, second)
}

/// clippy::approx_constant
pub fn positive_approx_constant() -> f64 {
    3.14159
}

/// clippy::async_yields_async
pub async fn positive_async_yields_async() -> u8 {
    async { async { 1 } }.await.await
}

/// clippy::bad_bit_mask
pub fn positive_bad_bit_mask(value: u8) -> bool {
    value & 2 == 1
}

/// clippy::cast_slice_different_sizes
pub fn positive_cast_slice_different_sizes(values: &[i32]) -> *const [u8] {
    values as *const [i32] as *const [u8]
}

/// clippy::char_indices_as_byte_indices
pub fn positive_char_indices_as_byte_indices(text: &str) -> usize {
    let mut total = 0;
    for (index, _) in text.chars().enumerate() {
        total += text.get(index..).map_or(0, str::len);
    }
    total
}

/// clippy::deprecated_semver
#[deprecated(since = "forever")]
pub fn positive_deprecated_semver() {}

/// clippy::derive_ord_xor_partial_ord
#[derive(Ord, PartialEq, Eq)]
pub struct PositiveDeriveOrd(u8);

impl PartialOrd for PositiveDeriveOrd {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        other.0.partial_cmp(&self.0)
    }
}

/// clippy::derived_hash_with_manual_eq
#[derive(Hash)]
pub struct PositiveDerivedHash(u8);

impl PartialEq for PositiveDerivedHash {
    fn eq(&self, other: &Self) -> bool {
        self.0 % 2 == other.0 % 2
    }
}

/// clippy::eager_transmute
#[repr(u8)]
pub enum PositiveOpcode {
    Load = 0,
    Store,
}

pub fn positive_eager_transmute(byte: u8) -> Option<PositiveOpcode> {
    (byte < 2).then_some(unsafe { std::mem::transmute::<u8, PositiveOpcode>(byte) })
}

/// clippy::enum_clike_unportable_variant
#[repr(usize)]
pub enum PositiveWide {
    Big = 0x1_0000_0000,
}

/// clippy::eq_op
pub fn positive_eq_op(value: u8) -> bool {
    value == value
}

/// clippy::erasing_op
pub fn positive_erasing_op(value: u8) -> u8 {
    value * 0
}

/// clippy::if_let_mutex
pub fn positive_if_let_mutex(lock: &std::sync::Mutex<u8>) -> u8 {
    if let Ok(guard) = lock.lock() {
        *guard
    } else {
        lock.lock().map_or(0, |guard| *guard)
    }
}

/// clippy::ifs_same_cond
pub fn positive_ifs_same_cond(value: u8) -> u8 {
    if value == 1 {
        1
    } else if value == 1 {
        2
    } else {
        3
    }
}

/// clippy::impl_hash_borrow_with_str_and_bytes
pub struct PositiveName(String);

impl std::hash::Hash for PositiveName {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl std::borrow::Borrow<str> for PositiveName {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl std::borrow::Borrow<[u8]> for PositiveName {
    fn borrow(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

/// clippy::impossible_comparisons
pub fn positive_impossible_comparisons(value: u8) -> bool {
    value > 5 && value < 3
}

/// clippy::ineffective_bit_mask
pub fn positive_ineffective_bit_mask(value: u8) -> bool {
    value | 1 > 3
}

/// clippy::infinite_iter
pub fn positive_infinite_iter() -> Vec<u8> {
    std::iter::repeat(1).collect()
}

/// clippy::inherent_to_string_shadow_display
pub struct PositiveLabel;

impl PositiveLabel {
    pub fn to_string(&self) -> String {
        String::from("inherent")
    }
}

impl std::fmt::Display for PositiveLabel {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("display")
    }
}

/// clippy::inline_fn_without_body
#[allow(unused_attributes, reason = "the admission contract requires a silent counterpart")]
pub trait PositiveInline {
    #[inline]
    fn value(&self) -> u8;
}

/// clippy::invisible_characters
pub fn positive_invisible_characters() -> &'static str {
    "a​b"
}

/// clippy::inverted_saturating_sub
pub fn positive_inverted_saturating_sub(first: u32, second: u32) -> u32 {
    if first > second { second - first } else { 0 }
}

/// clippy::iter_next_loop
#[allow(for_loops_over_fallibles, reason = "the admission contract requires a silent counterpart")]
pub fn positive_iter_next_loop(values: &[u8]) -> u8 {
    let mut total = 0;
    for value in values.iter().next() {
        total += value;
    }
    total
}

/// clippy::iter_skip_zero
pub fn positive_iter_skip_zero(values: &[u8]) -> usize {
    values.iter().skip(0).count()
}

/// clippy::iterator_step_by_zero
pub fn positive_iterator_step_by_zero() -> usize {
    (0..10).step_by(0).count()
}

/// clippy::match_str_case_mismatch
pub fn positive_match_str_case_mismatch(text: &str) -> u8 {
    match text.to_ascii_lowercase().as_str() {
        "Foo" => 1,
        _ => 0,
    }
}

/// clippy::mem_replace_with_uninit
#[allow(deprecated, invalid_value, reason = "the admission contract requires a silent counterpart")]
pub fn positive_mem_replace_with_uninit(values: &mut Vec<u8>) -> Vec<u8> {
    unsafe { std::mem::replace(values, std::mem::uninitialized()) }
}

/// clippy::min_max
pub fn positive_min_max(value: i32) -> i32 {
    std::cmp::min(0, std::cmp::max(100, value))
}

/// clippy::mistyped_literal_suffixes
pub fn positive_mistyped_literal_suffixes() -> i32 {
    123_32
}

/// clippy::modulo_one
pub fn positive_modulo_one(value: u8) -> u8 {
    value % 1
}

/// clippy::mut_from_ref
pub fn positive_mut_from_ref(value: &u8) -> &mut u8 {
    let pointer: *mut u8 = Box::into_raw(Box::new(*value));
    unsafe { &mut *pointer }
}

/// clippy::never_loop
pub fn positive_never_loop(values: &[u8]) -> u8 {
    let mut total = 0;
    loop {
        total += values.len() as u8;
        break;
    }
    total
}

/// clippy::non_octal_unix_permissions
pub fn positive_non_octal_unix_permissions() -> std::fs::Permissions {
    use std::os::unix::fs::PermissionsExt;
    std::fs::Permissions::from_mode(644)
}

/// clippy::nonsensical_open_options
pub fn positive_nonsensical_open_options() -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new().read(true).truncate(true).open("data")
}

/// clippy::not_unsafe_ptr_arg_deref
pub fn positive_not_unsafe_ptr_arg_deref(pointer: *const u8) -> u8 {
    unsafe { *pointer }
}

/// clippy::option_env_unwrap
#[allow(clippy::unwrap_used, reason = "the admission contract requires a silent counterpart")]
pub fn positive_option_env_unwrap() -> &'static str {
    option_env!("RUST_DOCTOR_NEVER_SET").unwrap()
}

/// clippy::out_of_bounds_indexing
#[allow(clippy::indexing_slicing, reason = "the admission contract requires a silent counterpart")]
pub fn positive_out_of_bounds_indexing() -> usize {
    let values = [1, 2, 3];
    values[0..5].len()
}

/// clippy::panicking_overflow_checks
pub fn positive_panicking_overflow_checks(first: u32, second: u32) -> bool {
    first + second < first
}

/// clippy::panicking_unwrap
#[allow(clippy::unwrap_used, reason = "the admission contract requires a silent counterpart")]
pub fn positive_panicking_unwrap(value: Option<u8>) -> u8 {
    if value.is_none() { value.unwrap() } else { 0 }
}

/// clippy::possible_missing_comma
pub fn positive_possible_missing_comma() -> [i32; 5] {
    [
        -1, -2, -3
        -4, -5, -6
    ]
}

/// clippy::read_line_without_trim
pub fn positive_read_line_without_trim() -> Option<i32> {
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).ok()?;
    line.parse::<i32>().ok()
}

/// clippy::recursive_format_impl
pub struct PositiveRecursive;

impl std::fmt::Display for PositiveRecursive {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let rendered = self.to_string();
        formatter.write_str(&rendered)
    }
}

/// clippy::redundant_comparisons
pub fn positive_redundant_comparisons(value: u8) -> bool {
    value > 5 && value > 3
}

/// clippy::reversed_empty_ranges
pub fn positive_reversed_empty_ranges() -> u8 {
    let mut total = 0;
    for value in 10..0 {
        total += value;
    }
    total
}

/// clippy::self_assignment
pub struct PositivePoint {
    pub x: u8,
}

#[allow(dead_code, reason = "the admission contract requires a silent counterpart")]
pub fn positive_self_assignment(point: &mut PositivePoint) {
    point.x = point.x;
}

/// clippy::size_of_in_element_count
pub fn positive_size_of_in_element_count(count: usize) -> [u16; 8] {
    let source = [2_u16; 8];
    let mut target = [0_u16; 8];
    unsafe {
        std::ptr::copy_nonoverlapping(source.as_ptr(), target.as_mut_ptr(), count * std::mem::size_of::<u16>());
    }
    target
}

/// clippy::suspicious_splitn
pub fn positive_suspicious_splitn(text: &str) -> usize {
    text.splitn(0, ',').count()
}

/// clippy::transmute_null_to_fn
pub fn positive_transmute_null_to_fn() -> fn() {
    unsafe { std::mem::transmute::<*const (), fn()>(std::ptr::null()) }
}

/// clippy::transmuting_null
pub fn positive_transmuting_null() -> &'static u8 {
    unsafe { std::mem::transmute::<*const u8, &u8>(std::ptr::null()) }
}

/// clippy::uninit_assumed_init
#[allow(invalid_value, reason = "the admission contract requires a silent counterpart")]
pub fn positive_uninit_assumed_init() -> [u8; 4] {
    unsafe { std::mem::MaybeUninit::uninit().assume_init() }
}

/// clippy::uninit_vec
pub fn positive_uninit_vec() -> Vec<u8> {
    let mut values: Vec<u8> = Vec::with_capacity(10);
    unsafe {
        values.set_len(10);
    }
    values
}

/// clippy::unit_cmp
pub fn positive_unit_cmp(value: u8) -> bool {
    let first = if value == 0 {};
    let second = if value == 1 {};
    first == second
}

/// clippy::unit_hash
pub fn positive_unit_hash<H: std::hash::Hasher>(value: u8, state: &mut H) {
    use std::hash::Hash;
    let unit = if value == 0 {};
    unit.hash(state);
}

/// clippy::unit_return_expecting_ord
#[allow(unused_must_use, reason = "the admission contract requires a silent counterpart")]
pub fn positive_unit_return_expecting_ord(values: &mut [u8]) {
    values.sort_by_key(|value| {
        value.count_ones();
    });
}

/// clippy::unsound_collection_transmute
pub fn positive_unsound_collection_transmute(values: Vec<u8>) -> Vec<u32> {
    unsafe { std::mem::transmute::<Vec<u8>, Vec<u32>>(values) }
}

/// clippy::unused_io_amount
pub fn positive_unused_io_amount(file: &mut std::fs::File) -> std::io::Result<()> {
    use std::io::Write;
    file.write(b"data")?;
    Ok(())
}

/// clippy::useless_attribute
#[allow(dead_code, reason = "the lint reads this attribute, not the item")]
extern crate alloc as positive_useless_attribute;

/// clippy::vec_resize_to_zero
pub fn positive_vec_resize_to_zero(values: &mut Vec<u8>) {
    values.resize(0, 5);
}

/// clippy::while_immutable_condition
pub fn positive_while_immutable_condition(limit: u8) -> u8 {
    let index = 0;
    while index < limit {}
    index
}

/// clippy::wrong_transmute
pub fn positive_wrong_transmute(value: f64) -> *const u8 {
    unsafe { std::mem::transmute::<f64, *const u8>(value) }
}

/// clippy::zst_offset
pub fn positive_zst_offset() -> *const () {
    let pointer: *const () = &();
    unsafe { pointer.offset(1) }
}
