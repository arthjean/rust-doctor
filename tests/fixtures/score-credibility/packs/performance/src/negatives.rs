




use std::rc::Rc;


pub fn negative_format_collect(values: &[u8]) -> String {
    let mut rendered = String::new();
    for value in values {
        rendered.push_str(&value.to_string());
        rendered.push(';');
    }
    rendered
}


pub fn negative_manual_memcpy(source: &[u8], destination: &mut [u8]) {
    destination.copy_from_slice(source);
}


pub fn negative_stable_sort_primitive(values: &mut [u64]) {
    values.sort_unstable();
}


pub fn negative_unnecessary_to_owned(value: &str) -> usize {
    borrowed_length(value)
}


pub fn negative_useless_vec() -> u64 {
    let values = [1_u64, 2, 3];
    values.iter().sum()
}


pub fn negative_vec_init_then_push(first: u64, second: u64) -> Vec<u64> {
    vec![first, second]
}


fn negative_large_types_passed_by_value(values: &[u64; 64]) -> usize {
    values.len()
}


fn negative_rc_buffer(value: Rc<str>) -> usize {
    value.len()
}


fn negative_redundant_allocation(value: Rc<u64>) -> u64 {
    *value
}


pub fn negative_private_signatures(values: &[u64; 64], buffer: Rc<str>, boxed: Rc<u64>) -> u64 {
    let sizes = negative_large_types_passed_by_value(values) + negative_rc_buffer(buffer);
    negative_redundant_allocation(boxed) + sizes as u64
}


#[allow(clippy::format_collect, reason = "the admission contract requires a silent counterpart")]
pub fn negative_allowed_format_collect(values: &[u8]) -> String {
    values.iter().map(|value| format!("{value};")).collect()
}


#[allow(clippy::indexing_slicing, clippy::manual_memcpy, reason = "the admission contract requires a silent counterpart")]
pub fn negative_allowed_manual_memcpy(source: &[u8], destination: &mut [u8]) {
    for index in 0..source.len() {
        destination[index] = source[index];
    }
}


#[allow(clippy::stable_sort_primitive, reason = "the admission contract requires a silent counterpart")]
pub fn negative_allowed_stable_sort_primitive(values: &mut [u64]) {
    values.sort();
}


#[allow(clippy::unnecessary_to_owned, reason = "the admission contract requires a silent counterpart")]
pub fn negative_allowed_unnecessary_to_owned(value: &str) -> usize {
    borrowed_length(&value.to_owned())
}


#[allow(clippy::useless_vec, reason = "the admission contract requires a silent counterpart")]
pub fn negative_allowed_useless_vec() -> u64 {
    let values = vec![1_u64, 2, 3];
    values.iter().sum()
}


#[allow(clippy::vec_init_then_push, reason = "the admission contract requires a silent counterpart")]
pub fn negative_allowed_vec_init_then_push(first: u64, second: u64) -> Vec<u64> {
    let mut values = Vec::new();
    values.push(first);
    values.push(second);
    values
}


#[allow(clippy::large_types_passed_by_value, reason = "the admission contract requires a silent counterpart")]
fn negative_allowed_large_types_passed_by_value(values: [u64; 64]) -> usize {
    values.len()
}


#[allow(clippy::rc_buffer, reason = "the admission contract requires a silent counterpart")]
fn negative_allowed_rc_buffer(value: Rc<String>) -> usize {
    value.len()
}


#[allow(clippy::redundant_allocation, reason = "the admission contract requires a silent counterpart")]
fn negative_allowed_redundant_allocation(value: Rc<Box<u64>>) -> u64 {
    **value
}


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


pub fn negative_ptr_arg(values: &[u8]) -> usize {
    values.len()
}


#[allow(clippy::ptr_arg, reason = "the admission contract requires a silent counterpart")]
pub fn negative_allowed_ptr_arg(values: &Vec<u8>) -> usize {
    values.len()
}
