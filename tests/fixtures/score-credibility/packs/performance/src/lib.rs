//! Positive fixture of the performance pack.
//!
//! Every function triggers exactly one lint of the pack. The verdict is frozen
//! on the `dev` profile of the normative toolchain, the one `cargo clippy`
//! runs: no lint whose result depends on the optimization level is admitted
//! into the pack.
//!
//! Three lints of the pack, `large_types_passed_by_value`, `rc_buffer` and
//! `redundant_allocation`, only aim at non-exported items: Clippy refuses to
//! propose a signature change on a public API. The fixture therefore triggers
//! them on private items, exercised by a public entry point.

mod negatives;

pub use negatives::*;

use std::rc::Rc;

/// clippy::format_collect
pub fn positive_format_collect(values: &[u8]) -> String {
    values.iter().map(|value| format!("{value};")).collect()
}

/// clippy::manual_memcpy
#[allow(clippy::indexing_slicing)]
pub fn positive_manual_memcpy(source: &[u8], destination: &mut [u8]) {
    for index in 0..source.len() {
        destination[index] = source[index];
    }
}

/// clippy::stable_sort_primitive
pub fn positive_stable_sort_primitive(values: &mut [u64]) {
    values.sort();
}

/// clippy::unnecessary_to_owned
pub fn positive_unnecessary_to_owned(value: &str) -> usize {
    borrowed_length(&value.to_owned())
}

/// clippy::useless_vec
pub fn positive_useless_vec() -> u64 {
    let values = vec![1_u64, 2, 3];
    values.iter().sum()
}

/// clippy::vec_init_then_push
pub fn positive_vec_init_then_push(first: u64, second: u64) -> Vec<u64> {
    let mut values = Vec::new();
    values.push(first);
    values.push(second);
    values
}

/// clippy::large_types_passed_by_value
fn positive_large_types_passed_by_value(values: [u64; 64]) -> usize {
    values.len()
}

/// clippy::rc_buffer
fn positive_rc_buffer(value: Rc<String>) -> usize {
    value.len()
}

/// clippy::redundant_allocation
fn positive_redundant_allocation(value: Rc<Box<u64>>) -> u64 {
    **value
}

/// Exercises the three lints reserved for non-exported items.
pub fn positive_private_signatures(
    values: [u64; 64],
    buffer: Rc<String>,
    boxed: Rc<Box<u64>>,
) -> u64 {
    let sizes = positive_large_types_passed_by_value(values) + positive_rc_buffer(buffer);
    positive_redundant_allocation(boxed) + sizes as u64
}

fn borrowed_length(value: &str) -> usize {
    value.len()
}

/// clippy::ptr_arg
pub fn positive_ptr_arg(values: &Vec<u8>) -> usize {
    values.len()
}
