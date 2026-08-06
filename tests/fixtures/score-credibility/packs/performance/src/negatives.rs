//! Negative fixture of the performance pack.
//!
//! The negatives of the three lints reserved for non-exported items are private
//! too, without which their silence would prove nothing.

use std::rc::Rc;

/// negative_format_collect
pub fn negative_format_collect(values: &[u8]) -> String {
    let mut rendered = String::new();
    for value in values {
        rendered.push_str(&value.to_string());
        rendered.push(';');
    }
    rendered
}

/// negative_manual_memcpy
pub fn negative_manual_memcpy(source: &[u8], destination: &mut [u8]) {
    destination.copy_from_slice(source);
}

/// negative_stable_sort_primitive
pub fn negative_stable_sort_primitive(values: &mut [u64]) {
    values.sort_unstable();
}

/// negative_unnecessary_to_owned
pub fn negative_unnecessary_to_owned(value: &str) -> usize {
    borrowed_length(value)
}

/// negative_useless_vec
pub fn negative_useless_vec() -> u64 {
    let values = [1_u64, 2, 3];
    values.iter().sum()
}

/// negative_vec_init_then_push
pub fn negative_vec_init_then_push(first: u64, second: u64) -> Vec<u64> {
    vec![first, second]
}

/// negative_large_types_passed_by_value
fn negative_large_types_passed_by_value(values: &[u64; 64]) -> usize {
    values.len()
}

/// negative_rc_buffer
fn negative_rc_buffer(value: Rc<str>) -> usize {
    value.len()
}

/// negative_redundant_allocation
fn negative_redundant_allocation(value: Rc<u64>) -> u64 {
    *value
}

/// Exercises the private negatives.
pub fn negative_private_signatures(values: &[u64; 64], buffer: Rc<str>, boxed: Rc<u64>) -> u64 {
    let sizes = negative_large_types_passed_by_value(values) + negative_rc_buffer(buffer);
    negative_redundant_allocation(boxed) + sizes as u64
}

/// negative_allowed_format_collect
#[allow(clippy::format_collect)]
pub fn negative_allowed_format_collect(values: &[u8]) -> String {
    values.iter().map(|value| format!("{value};")).collect()
}

/// negative_allowed_manual_memcpy
#[allow(clippy::indexing_slicing, clippy::manual_memcpy)]
pub fn negative_allowed_manual_memcpy(source: &[u8], destination: &mut [u8]) {
    for index in 0..source.len() {
        destination[index] = source[index];
    }
}

/// negative_allowed_stable_sort_primitive
#[allow(clippy::stable_sort_primitive)]
pub fn negative_allowed_stable_sort_primitive(values: &mut [u64]) {
    values.sort();
}

/// negative_allowed_unnecessary_to_owned
#[allow(clippy::unnecessary_to_owned)]
pub fn negative_allowed_unnecessary_to_owned(value: &str) -> usize {
    borrowed_length(&value.to_owned())
}

/// negative_allowed_useless_vec
#[allow(clippy::useless_vec)]
pub fn negative_allowed_useless_vec() -> u64 {
    let values = vec![1_u64, 2, 3];
    values.iter().sum()
}

/// negative_allowed_vec_init_then_push
#[allow(clippy::vec_init_then_push)]
pub fn negative_allowed_vec_init_then_push(first: u64, second: u64) -> Vec<u64> {
    let mut values = Vec::new();
    values.push(first);
    values.push(second);
    values
}

/// negative_allowed_large_types_passed_by_value
#[allow(clippy::large_types_passed_by_value)]
fn negative_allowed_large_types_passed_by_value(values: [u64; 64]) -> usize {
    values.len()
}

/// negative_allowed_rc_buffer
#[allow(clippy::rc_buffer)]
fn negative_allowed_rc_buffer(value: Rc<String>) -> usize {
    value.len()
}

/// negative_allowed_redundant_allocation
#[allow(clippy::redundant_allocation)]
fn negative_allowed_redundant_allocation(value: Rc<Box<u64>>) -> u64 {
    **value
}

/// Exercises the locally neutralized private negatives.
pub fn negative_allowed_private_signatures(
    values: [u64; 64],
    buffer: Rc<String>,
    boxed: Rc<Box<u64>>,
) -> u64 {
    let sizes =
        negative_allowed_large_types_passed_by_value(values) + negative_allowed_rc_buffer(buffer);
    negative_allowed_redundant_allocation(boxed) + sizes as u64
}

fn borrowed_length(value: &str) -> usize {
    value.len()
}
