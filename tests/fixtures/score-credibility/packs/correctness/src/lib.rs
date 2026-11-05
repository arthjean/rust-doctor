











mod negatives;

pub use negatives::*;


pub fn positive_absurd_extreme_comparisons(value: u8) -> bool {
    value > u8::MAX
}


#[allow(unused_assignments, reason = "the admission contract requires a silent counterpart")]
pub fn positive_almost_swapped(mut first: u8, mut second: u8) -> (u8, u8) {
    first = second;
    second = first;
    (first, second)
}


pub fn positive_approx_constant() -> f64 {
    3.14159
}


pub async fn positive_async_yields_async() -> u8 {
    async { async { 1 } }.await.await
}


pub fn positive_bad_bit_mask(value: u8) -> bool {
    value & 2 == 1
}


pub fn positive_cast_slice_different_sizes(values: &[i32]) -> *const [u8] {
    values as *const [i32] as *const [u8]
}


pub fn positive_char_indices_as_byte_indices(text: &str) -> usize {
    let mut total = 0;
    for (index, _) in text.chars().enumerate() {
        total += text.get(index..).map_or(0, str::len);
    }
    total
}


#[deprecated(since = "forever")]
pub fn positive_deprecated_semver() {}


#[derive(Ord, PartialEq, Eq)]
pub struct PositiveDeriveOrd(u8);

impl PartialOrd for PositiveDeriveOrd {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        other.0.partial_cmp(&self.0)
    }
}


#[derive(Hash)]
pub struct PositiveDerivedHash(u8);

impl PartialEq for PositiveDerivedHash {
    fn eq(&self, other: &Self) -> bool {
        self.0 % 2 == other.0 % 2
    }
}


#[repr(u8)]
pub enum PositiveOpcode {
    Load = 0,
    Store,
}

pub fn positive_eager_transmute(byte: u8) -> Option<PositiveOpcode> {

    (byte < 2).then_some(unsafe { std::mem::transmute::<u8, PositiveOpcode>(byte) })
}


#[repr(usize)]
pub enum PositiveWide {
    Big = 0x1_0000_0000,
}


pub fn positive_eq_op(value: u8) -> bool {
    value == value
}


pub fn positive_erasing_op(value: u8) -> u8 {
    value * 0
}


pub fn positive_if_let_mutex(lock: &std::sync::Mutex<u8>) -> u8 {
    if let Ok(guard) = lock.lock() {
        *guard
    } else {
        lock.lock().map_or(0, |guard| *guard)
    }
}


pub fn positive_ifs_same_cond(value: u8) -> u8 {
    if value == 1 {
        1
    } else if value == 1 {
        2
    } else {
        3
    }
}


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


pub fn positive_impossible_comparisons(value: u8) -> bool {
    value > 5 && value < 3
}


pub fn positive_ineffective_bit_mask(value: u8) -> bool {
    value | 1 > 3
}


pub fn positive_infinite_iter() -> Vec<u8> {
    std::iter::repeat(1).collect()
}


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


#[allow(unused_attributes, reason = "the admission contract requires a silent counterpart")]
pub trait PositiveInline {
    #[inline]
    fn value(&self) -> u8;
}


pub fn positive_invisible_characters() -> &'static str {
    "a​b"
}


pub fn positive_inverted_saturating_sub(first: u32, second: u32) -> u32 {
    if first > second { second - first } else { 0 }
}


#[allow(for_loops_over_fallibles, reason = "the admission contract requires a silent counterpart")]
pub fn positive_iter_next_loop(values: &[u8]) -> u8 {
    let mut total = 0;
    for value in values.iter().next() {
        total += value;
    }
    total
}


pub fn positive_iter_skip_zero(values: &[u8]) -> usize {
    values.iter().skip(0).count()
}


pub fn positive_iterator_step_by_zero() -> usize {
    (0..10).step_by(0).count()
}


pub fn positive_match_str_case_mismatch(text: &str) -> u8 {
    match text.to_ascii_lowercase().as_str() {
        "Foo" => 1,
        _ => 0,
    }
}


#[allow(deprecated, invalid_value, reason = "the admission contract requires a silent counterpart")]
pub fn positive_mem_replace_with_uninit(values: &mut Vec<u8>) -> Vec<u8> {

    unsafe { std::mem::replace(values, std::mem::uninitialized()) }
}


pub fn positive_min_max(value: i32) -> i32 {
    std::cmp::min(0, std::cmp::max(100, value))
}


pub fn positive_mistyped_literal_suffixes() -> i32 {
    123_32
}


pub fn positive_modulo_one(value: u8) -> u8 {
    value % 1
}


pub fn positive_mut_from_ref(value: &u8) -> &mut u8 {
    let pointer: *mut u8 = Box::into_raw(Box::new(*value));

    unsafe { &mut *pointer }
}


pub fn positive_never_loop(values: &[u8]) -> u8 {
    let mut total = 0;
    loop {
        total += values.len() as u8;
        break;
    }
    total
}


pub fn positive_non_octal_unix_permissions() -> std::fs::Permissions {
    use std::os::unix::fs::PermissionsExt;
    std::fs::Permissions::from_mode(644)
}


pub fn positive_nonsensical_open_options() -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new().read(true).truncate(true).open("data")
}


pub fn positive_not_unsafe_ptr_arg_deref(pointer: *const u8) -> u8 {

    unsafe { *pointer }
}


#[allow(clippy::unwrap_used, reason = "the admission contract requires a silent counterpart")]
pub fn positive_option_env_unwrap() -> &'static str {
    option_env!("RUST_DOCTOR_NEVER_SET").unwrap()
}


#[allow(clippy::indexing_slicing, reason = "the admission contract requires a silent counterpart")]
pub fn positive_out_of_bounds_indexing() -> usize {
    let values = [1, 2, 3];
    values[0..5].len()
}


pub fn positive_panicking_overflow_checks(first: u32, second: u32) -> bool {
    first + second < first
}


#[allow(clippy::unwrap_used, reason = "the admission contract requires a silent counterpart")]
pub fn positive_panicking_unwrap(value: Option<u8>) -> u8 {
    if value.is_none() { value.unwrap() } else { 0 }
}


pub fn positive_possible_missing_comma() -> [i32; 5] {
    [
        -1, -2, -3
        -4, -5, -6
    ]
}


pub fn positive_read_line_without_trim() -> Option<i32> {
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).ok()?;
    line.parse::<i32>().ok()
}


pub struct PositiveRecursive;

impl std::fmt::Display for PositiveRecursive {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let rendered = self.to_string();
        formatter.write_str(&rendered)
    }
}


pub fn positive_redundant_comparisons(value: u8) -> bool {
    value > 5 && value > 3
}


pub fn positive_reversed_empty_ranges() -> u8 {
    let mut total = 0;
    for value in 10..0 {
        total += value;
    }
    total
}


pub struct PositivePoint {
    pub x: u8,
}

#[allow(dead_code, reason = "the admission contract requires a silent counterpart")]
pub fn positive_self_assignment(point: &mut PositivePoint) {
    point.x = point.x;
}


pub fn positive_size_of_in_element_count(count: usize) -> [u16; 8] {
    let source = [2_u16; 8];
    let mut target = [0_u16; 8];

    unsafe {
        std::ptr::copy_nonoverlapping(source.as_ptr(), target.as_mut_ptr(), count * std::mem::size_of::<u16>());
    }
    target
}


pub fn positive_suspicious_splitn(text: &str) -> usize {
    text.splitn(0, ',').count()
}


pub fn positive_transmute_null_to_fn() -> fn() {

    unsafe { std::mem::transmute::<*const (), fn()>(std::ptr::null()) }
}


pub fn positive_transmuting_null() -> &'static u8 {

    unsafe { std::mem::transmute::<*const u8, &u8>(std::ptr::null()) }
}


#[allow(invalid_value, reason = "the admission contract requires a silent counterpart")]
pub fn positive_uninit_assumed_init() -> [u8; 4] {

    unsafe { std::mem::MaybeUninit::uninit().assume_init() }
}


pub fn positive_uninit_vec() -> Vec<u8> {
    let mut values: Vec<u8> = Vec::with_capacity(10);

    unsafe {
        values.set_len(10);
    }
    values
}


pub fn positive_unit_cmp(value: u8) -> bool {
    let first = if value == 0 {};
    let second = if value == 1 {};
    first == second
}


pub fn positive_unit_hash<H: std::hash::Hasher>(value: u8, state: &mut H) {
    use std::hash::Hash;
    let unit = if value == 0 {};
    unit.hash(state);
}


#[allow(unused_must_use, reason = "the admission contract requires a silent counterpart")]
pub fn positive_unit_return_expecting_ord(values: &mut [u8]) {
    values.sort_by_key(|value| {
        value.count_ones();
    });
}


pub fn positive_unsound_collection_transmute(values: Vec<u8>) -> Vec<u32> {

    unsafe { std::mem::transmute::<Vec<u8>, Vec<u32>>(values) }
}


pub fn positive_unused_io_amount(file: &mut std::fs::File) -> std::io::Result<()> {
    use std::io::Write;
    file.write(b"data")?;
    Ok(())
}


#[allow(dead_code, reason = "the lint reads this attribute, not the item")]
extern crate alloc as positive_useless_attribute;


pub fn positive_vec_resize_to_zero(values: &mut Vec<u8>) {
    values.resize(0, 5);
}


pub fn positive_while_immutable_condition(limit: u8) -> u8 {
    let index = 0;
    while index < limit {}
    index
}


pub fn positive_wrong_transmute(value: f64) -> *const u8 {

    unsafe { std::mem::transmute::<f64, *const u8>(value) }
}


pub fn positive_zst_offset() -> *const () {
    let pointer: *const () = &();

    unsafe { pointer.offset(1) }
}
