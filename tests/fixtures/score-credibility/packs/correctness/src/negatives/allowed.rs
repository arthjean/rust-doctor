



#[allow(clippy::absurd_extreme_comparisons, reason = "the admission contract requires a silent counterpart")]
pub mod negative_allowed_absurd_extreme_comparisons {
    pub fn allowed_absurd_extreme_comparisons(value: u8) -> bool {
        value > u8::MAX
    }
}


#[allow(clippy::almost_swapped, reason = "the admission contract requires a silent counterpart")]
pub mod negative_allowed_almost_swapped {
    #[allow(unused_assignments, reason = "the admission contract requires a silent counterpart")]
    pub fn allowed_almost_swapped(mut first: u8, mut second: u8) -> (u8, u8) {
        first = second;
        second = first;
        (first, second)
    }
}


#[allow(clippy::approx_constant, reason = "the admission contract requires a silent counterpart")]
pub mod negative_allowed_approx_constant {
    pub fn allowed_approx_constant() -> f64 {
        3.14159
    }
}


#[allow(clippy::async_yields_async, reason = "the admission contract requires a silent counterpart")]
pub mod negative_allowed_async_yields_async {
    pub async fn allowed_async_yields_async() -> u8 {
        async { async { 1 } }.await.await
    }
}


#[allow(clippy::bad_bit_mask, reason = "the admission contract requires a silent counterpart")]
pub mod negative_allowed_bad_bit_mask {
    pub fn allowed_bad_bit_mask(value: u8) -> bool {
        value & 2 == 1
    }
}


#[allow(clippy::cast_slice_different_sizes, reason = "the admission contract requires a silent counterpart")]
pub mod negative_allowed_cast_slice_different_sizes {
    pub fn allowed_cast_slice_different_sizes(values: &[i32]) -> *const [u8] {
        values as *const [i32] as *const [u8]
    }
}


#[allow(clippy::char_indices_as_byte_indices, reason = "the admission contract requires a silent counterpart")]
pub mod negative_allowed_char_indices_as_byte_indices {
    pub fn allowed_char_indices_as_byte_indices(text: &str) -> usize {
        let mut total = 0;
        for (index, _) in text.chars().enumerate() {
            total += text.get(index..).map_or(0, str::len);
        }
        total
    }
}


#[allow(clippy::deprecated_semver, reason = "the admission contract requires a silent counterpart")]
pub mod negative_allowed_deprecated_semver {
    #[deprecated(since = "forever")]
    pub fn allowed_deprecated_semver() {}
}


#[allow(clippy::derive_ord_xor_partial_ord, reason = "the admission contract requires a silent counterpart")]
pub mod negative_allowed_derive_ord_xor_partial_ord {
    #[derive(Ord, PartialEq, Eq)]
    pub struct AllowedDeriveOrd(u8);

    impl PartialOrd for AllowedDeriveOrd {
        fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
            other.0.partial_cmp(&self.0)
        }
    }
}


#[allow(clippy::derived_hash_with_manual_eq, reason = "the admission contract requires a silent counterpart")]
pub mod negative_allowed_derived_hash_with_manual_eq {
    #[derive(Hash)]
    pub struct AllowedDerivedHash(u8);

    impl PartialEq for AllowedDerivedHash {
        fn eq(&self, other: &Self) -> bool {
            self.0 % 2 == other.0 % 2
        }
    }
}


#[allow(clippy::eager_transmute, reason = "the admission contract requires a silent counterpart")]
pub mod negative_allowed_eager_transmute {
    #[repr(u8)]
    pub enum AllowedOpcode {
        Load = 0,
        Store,
    }

    pub fn allowed_eager_transmute(byte: u8) -> Option<AllowedOpcode> {

        (byte < 2).then_some(unsafe { std::mem::transmute::<u8, AllowedOpcode>(byte) })
    }
}


#[allow(clippy::enum_clike_unportable_variant, reason = "the admission contract requires a silent counterpart")]
pub mod negative_allowed_enum_clike_unportable_variant {
    #[repr(usize)]
    pub enum AllowedWide {
        Big = 0x1_0000_0000,
    }
}


#[allow(clippy::eq_op, reason = "the admission contract requires a silent counterpart")]
pub mod negative_allowed_eq_op {
    pub fn allowed_eq_op(value: u8) -> bool {
        value == value
    }
}


#[allow(clippy::erasing_op, reason = "the admission contract requires a silent counterpart")]
pub mod negative_allowed_erasing_op {
    pub fn allowed_erasing_op(value: u8) -> u8 {
        value * 0
    }
}


#[allow(clippy::if_let_mutex, reason = "the admission contract requires a silent counterpart")]
pub mod negative_allowed_if_let_mutex {
    pub fn allowed_if_let_mutex(lock: &std::sync::Mutex<u8>) -> u8 {
        if let Ok(guard) = lock.lock() {
            *guard
        } else {
            lock.lock().map_or(0, |guard| *guard)
        }
    }
}


#[allow(clippy::ifs_same_cond, reason = "the admission contract requires a silent counterpart")]
pub mod negative_allowed_ifs_same_cond {
    pub fn allowed_ifs_same_cond(value: u8) -> u8 {
        if value == 1 {
            1
        } else if value == 1 {
            2
        } else {
            3
        }
    }
}


#[allow(clippy::impl_hash_borrow_with_str_and_bytes, reason = "the admission contract requires a silent counterpart")]
pub mod negative_allowed_impl_hash_borrow_with_str_and_bytes {
    pub struct AllowedName(String);

    impl std::hash::Hash for AllowedName {
        fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
            self.0.hash(state);
        }
    }

    impl std::borrow::Borrow<str> for AllowedName {
        fn borrow(&self) -> &str {
            &self.0
        }
    }

    impl std::borrow::Borrow<[u8]> for AllowedName {
        fn borrow(&self) -> &[u8] {
            self.0.as_bytes()
        }
    }
}


#[allow(clippy::impossible_comparisons, reason = "the admission contract requires a silent counterpart")]
pub mod negative_allowed_impossible_comparisons {
    pub fn allowed_impossible_comparisons(value: u8) -> bool {
        value > 5 && value < 3
    }
}


#[allow(clippy::ineffective_bit_mask, reason = "the admission contract requires a silent counterpart")]
pub mod negative_allowed_ineffective_bit_mask {
    pub fn allowed_ineffective_bit_mask(value: u8) -> bool {
        value | 1 > 3
    }
}


#[allow(clippy::infinite_iter, reason = "the admission contract requires a silent counterpart")]
pub mod negative_allowed_infinite_iter {
    pub fn allowed_infinite_iter() -> Vec<u8> {
        std::iter::repeat(1).collect()
    }
}


#[allow(clippy::inherent_to_string_shadow_display, reason = "the admission contract requires a silent counterpart")]
pub mod negative_allowed_inherent_to_string_shadow_display {
    pub struct AllowedLabel;

    impl AllowedLabel {
        pub fn to_string(&self) -> String {
            String::from("inherent")
        }
    }

    impl std::fmt::Display for AllowedLabel {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("display")
        }
    }
}


#[allow(clippy::inline_fn_without_body, reason = "the admission contract requires a silent counterpart")]
pub mod negative_allowed_inline_fn_without_body {
    #[allow(unused_attributes, reason = "the admission contract requires a silent counterpart")]
    pub trait AllowedInline {
        #[inline]
        fn value(&self) -> u8;
    }
}


#[allow(clippy::invisible_characters, reason = "the admission contract requires a silent counterpart")]
pub mod negative_allowed_invisible_characters {
    pub fn allowed_invisible_characters() -> &'static str {
        "a​b"
    }
}


#[allow(clippy::inverted_saturating_sub, reason = "the admission contract requires a silent counterpart")]
pub mod negative_allowed_inverted_saturating_sub {
    pub fn allowed_inverted_saturating_sub(first: u32, second: u32) -> u32 {
        if first > second { second - first } else { 0 }
    }
}


#[allow(clippy::iter_next_loop, reason = "the admission contract requires a silent counterpart")]
pub mod negative_allowed_iter_next_loop {
    #[allow(for_loops_over_fallibles, reason = "the admission contract requires a silent counterpart")]
    pub fn allowed_iter_next_loop(values: &[u8]) -> u8 {
        let mut total = 0;
        for value in values.iter().next() {
            total += value;
        }
        total
    }
}


#[allow(clippy::iter_skip_zero, reason = "the admission contract requires a silent counterpart")]
pub mod negative_allowed_iter_skip_zero {
    pub fn allowed_iter_skip_zero(values: &[u8]) -> usize {
        values.iter().skip(0).count()
    }
}


#[allow(clippy::iterator_step_by_zero, reason = "the admission contract requires a silent counterpart")]
pub mod negative_allowed_iterator_step_by_zero {
    pub fn allowed_iterator_step_by_zero() -> usize {
        (0..10).step_by(0).count()
    }
}


#[allow(clippy::match_str_case_mismatch, reason = "the admission contract requires a silent counterpart")]
pub mod negative_allowed_match_str_case_mismatch {
    pub fn allowed_match_str_case_mismatch(text: &str) -> u8 {
        match text.to_ascii_lowercase().as_str() {
            "Foo" => 1,
            _ => 0,
        }
    }
}


#[allow(clippy::mem_replace_with_uninit, reason = "the admission contract requires a silent counterpart")]
pub mod negative_allowed_mem_replace_with_uninit {
    #[allow(deprecated, invalid_value, reason = "the admission contract requires a silent counterpart")]
    pub fn allowed_mem_replace_with_uninit(values: &mut Vec<u8>) -> Vec<u8> {

        unsafe { std::mem::replace(values, std::mem::uninitialized()) }
    }
}


#[allow(clippy::min_max, reason = "the admission contract requires a silent counterpart")]
pub mod negative_allowed_min_max {
    pub fn allowed_min_max(value: i32) -> i32 {
        std::cmp::min(0, std::cmp::max(100, value))
    }
}


#[allow(clippy::mistyped_literal_suffixes, reason = "the admission contract requires a silent counterpart")]
pub mod negative_allowed_mistyped_literal_suffixes {
    pub fn allowed_mistyped_literal_suffixes() -> i32 {
        123_32
    }
}


#[allow(clippy::modulo_one, reason = "the admission contract requires a silent counterpart")]
pub mod negative_allowed_modulo_one {
    pub fn allowed_modulo_one(value: u8) -> u8 {
        value % 1
    }
}


#[allow(clippy::mut_from_ref, reason = "the admission contract requires a silent counterpart")]
pub mod negative_allowed_mut_from_ref {
    pub fn allowed_mut_from_ref(value: &u8) -> &mut u8 {
        let pointer: *mut u8 = Box::into_raw(Box::new(*value));

        unsafe { &mut *pointer }
    }
}


#[allow(clippy::never_loop, reason = "the admission contract requires a silent counterpart")]
pub mod negative_allowed_never_loop {
    pub fn allowed_never_loop(values: &[u8]) -> u8 {
        let mut total = 0;
        loop {
            total += values.len() as u8;
            break;
        }
        total
    }
}


#[allow(clippy::non_octal_unix_permissions, reason = "the admission contract requires a silent counterpart")]
pub mod negative_allowed_non_octal_unix_permissions {
    pub fn allowed_non_octal_unix_permissions() -> std::fs::Permissions {
        use std::os::unix::fs::PermissionsExt;
        std::fs::Permissions::from_mode(644)
    }
}


#[allow(clippy::nonsensical_open_options, reason = "the admission contract requires a silent counterpart")]
pub mod negative_allowed_nonsensical_open_options {
    pub fn allowed_nonsensical_open_options() -> std::io::Result<std::fs::File> {
        std::fs::OpenOptions::new().read(true).truncate(true).open("data")
    }
}


#[allow(clippy::not_unsafe_ptr_arg_deref, reason = "the admission contract requires a silent counterpart")]
pub mod negative_allowed_not_unsafe_ptr_arg_deref {
    pub fn allowed_not_unsafe_ptr_arg_deref(pointer: *const u8) -> u8 {

        unsafe { *pointer }
    }
}


#[allow(clippy::option_env_unwrap, reason = "the admission contract requires a silent counterpart")]
pub mod negative_allowed_option_env_unwrap {
    #[allow(clippy::unwrap_used, reason = "the admission contract requires a silent counterpart")]
    pub fn allowed_option_env_unwrap() -> &'static str {
        option_env!("RUST_DOCTOR_NEVER_SET").unwrap()
    }
}


#[allow(clippy::out_of_bounds_indexing, reason = "the admission contract requires a silent counterpart")]
pub mod negative_allowed_out_of_bounds_indexing {
    #[allow(clippy::indexing_slicing, reason = "the admission contract requires a silent counterpart")]
    pub fn allowed_out_of_bounds_indexing() -> usize {
        let values = [1, 2, 3];
        values[0..5].len()
    }
}


#[allow(clippy::panicking_overflow_checks, reason = "the admission contract requires a silent counterpart")]
pub mod negative_allowed_panicking_overflow_checks {
    pub fn allowed_panicking_overflow_checks(first: u32, second: u32) -> bool {
        first + second < first
    }
}


#[allow(clippy::panicking_unwrap, reason = "the admission contract requires a silent counterpart")]
pub mod negative_allowed_panicking_unwrap {
    #[allow(clippy::unwrap_used, reason = "the admission contract requires a silent counterpart")]
    pub fn allowed_panicking_unwrap(value: Option<u8>) -> u8 {
        if value.is_none() { value.unwrap() } else { 0 }
    }
}


#[allow(clippy::possible_missing_comma, reason = "the admission contract requires a silent counterpart")]
pub mod negative_allowed_possible_missing_comma {
    pub fn allowed_possible_missing_comma() -> [i32; 5] {
        [
            -1, -2, -3
            -4, -5, -6
        ]
    }
}


#[allow(clippy::read_line_without_trim, reason = "the admission contract requires a silent counterpart")]
pub mod negative_allowed_read_line_without_trim {
    pub fn allowed_read_line_without_trim() -> Option<i32> {
        let mut line = String::new();
        std::io::stdin().read_line(&mut line).ok()?;
        line.parse::<i32>().ok()
    }
}


#[allow(clippy::recursive_format_impl, reason = "the admission contract requires a silent counterpart")]
pub mod negative_allowed_recursive_format_impl {
    pub struct AllowedRecursive;

    impl std::fmt::Display for AllowedRecursive {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            let rendered = self.to_string();
            formatter.write_str(&rendered)
        }
    }
}


#[allow(clippy::redundant_comparisons, reason = "the admission contract requires a silent counterpart")]
pub mod negative_allowed_redundant_comparisons {
    pub fn allowed_redundant_comparisons(value: u8) -> bool {
        value > 5 && value > 3
    }
}


#[allow(clippy::reversed_empty_ranges, reason = "the admission contract requires a silent counterpart")]
pub mod negative_allowed_reversed_empty_ranges {
    pub fn allowed_reversed_empty_ranges() -> u8 {
        let mut total = 0;
        for value in 10..0 {
            total += value;
        }
        total
    }
}


#[allow(clippy::self_assignment, reason = "the admission contract requires a silent counterpart")]
pub mod negative_allowed_self_assignment {
    pub struct AllowedPoint {
        pub x: u8,
    }

    #[allow(dead_code, reason = "the admission contract requires a silent counterpart")]
    pub fn allowed_self_assignment(point: &mut AllowedPoint) {
        point.x = point.x;
    }
}


#[allow(clippy::size_of_in_element_count, reason = "the admission contract requires a silent counterpart")]
pub mod negative_allowed_size_of_in_element_count {
    pub fn allowed_size_of_in_element_count(count: usize) -> [u16; 8] {
        let source = [2_u16; 8];
        let mut target = [0_u16; 8];

        unsafe {
            std::ptr::copy_nonoverlapping(source.as_ptr(), target.as_mut_ptr(), count * std::mem::size_of::<u16>());
        }
        target
    }
}


#[allow(clippy::suspicious_splitn, reason = "the admission contract requires a silent counterpart")]
pub mod negative_allowed_suspicious_splitn {
    pub fn allowed_suspicious_splitn(text: &str) -> usize {
        text.splitn(0, ',').count()
    }
}


#[allow(clippy::transmute_null_to_fn, reason = "the admission contract requires a silent counterpart")]
pub mod negative_allowed_transmute_null_to_fn {
    pub fn allowed_transmute_null_to_fn() -> fn() {

        unsafe { std::mem::transmute::<*const (), fn()>(std::ptr::null()) }
    }
}


#[allow(clippy::transmuting_null, reason = "the admission contract requires a silent counterpart")]
pub mod negative_allowed_transmuting_null {
    pub fn allowed_transmuting_null() -> &'static u8 {

        unsafe { std::mem::transmute::<*const u8, &u8>(std::ptr::null()) }
    }
}


#[allow(clippy::uninit_assumed_init, reason = "the admission contract requires a silent counterpart")]
pub mod negative_allowed_uninit_assumed_init {
    #[allow(invalid_value, reason = "the admission contract requires a silent counterpart")]
    pub fn allowed_uninit_assumed_init() -> [u8; 4] {

        unsafe { std::mem::MaybeUninit::uninit().assume_init() }
    }
}


#[allow(clippy::uninit_vec, reason = "the admission contract requires a silent counterpart")]
pub mod negative_allowed_uninit_vec {
    pub fn allowed_uninit_vec() -> Vec<u8> {
        let mut values: Vec<u8> = Vec::with_capacity(10);

        unsafe {
            values.set_len(10);
        }
        values
    }
}


#[allow(clippy::unit_cmp, reason = "the admission contract requires a silent counterpart")]
pub mod negative_allowed_unit_cmp {
    pub fn allowed_unit_cmp(value: u8) -> bool {
        let first = if value == 0 {};
        let second = if value == 1 {};
        first == second
    }
}


#[allow(clippy::unit_hash, reason = "the admission contract requires a silent counterpart")]
pub mod negative_allowed_unit_hash {
    pub fn allowed_unit_hash<H: std::hash::Hasher>(value: u8, state: &mut H) {
        use std::hash::Hash;
        let unit = if value == 0 {};
        unit.hash(state);
    }
}


#[allow(clippy::unit_return_expecting_ord, reason = "the admission contract requires a silent counterpart")]
pub mod negative_allowed_unit_return_expecting_ord {
    #[allow(unused_must_use, reason = "the admission contract requires a silent counterpart")]
    pub fn allowed_unit_return_expecting_ord(values: &mut [u8]) {
        values.sort_by_key(|value| {
            value.count_ones();
        });
    }
}


#[allow(clippy::unsound_collection_transmute, reason = "the admission contract requires a silent counterpart")]
pub mod negative_allowed_unsound_collection_transmute {
    pub fn allowed_unsound_collection_transmute(values: Vec<u8>) -> Vec<u32> {

        unsafe { std::mem::transmute::<Vec<u8>, Vec<u32>>(values) }
    }
}


#[allow(clippy::unused_io_amount, reason = "the admission contract requires a silent counterpart")]
pub mod negative_allowed_unused_io_amount {
    pub fn allowed_unused_io_amount(file: &mut std::fs::File) -> std::io::Result<()> {
        use std::io::Write;
        file.write(b"data")?;
        Ok(())
    }
}


#[allow(clippy::useless_attribute, reason = "the admission contract requires a silent counterpart")]
pub mod negative_allowed_useless_attribute {
    #[allow(dead_code, reason = "the lint reads this attribute, not the item")]
    extern crate alloc as allowed_useless_attribute;
}


#[allow(clippy::vec_resize_to_zero, reason = "the admission contract requires a silent counterpart")]
pub mod negative_allowed_vec_resize_to_zero {
    pub fn allowed_vec_resize_to_zero(values: &mut Vec<u8>) {
        values.resize(0, 5);
    }
}


#[allow(clippy::while_immutable_condition, reason = "the admission contract requires a silent counterpart")]
pub mod negative_allowed_while_immutable_condition {
    pub fn allowed_while_immutable_condition(limit: u8) -> u8 {
        let index = 0;
        while index < limit {}
        index
    }
}


#[allow(clippy::wrong_transmute, reason = "the admission contract requires a silent counterpart")]
pub mod negative_allowed_wrong_transmute {
    pub fn allowed_wrong_transmute(value: f64) -> *const u8 {

        unsafe { std::mem::transmute::<f64, *const u8>(value) }
    }
}


#[allow(clippy::zst_offset, reason = "the admission contract requires a silent counterpart")]
pub mod negative_allowed_zst_offset {
    pub fn allowed_zst_offset() -> *const () {
        let pointer: *const () = &();

        unsafe { pointer.offset(1) }
    }
}
