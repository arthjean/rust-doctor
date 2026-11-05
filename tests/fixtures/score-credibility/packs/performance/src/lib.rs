











mod negatives;

pub use negatives::*;

use std::rc::Rc;


pub fn positive_format_collect(values: &[u8]) -> String {
    values.iter().map(|value| format!("{value};")).collect()
}


#[allow(clippy::indexing_slicing, reason = "the admission contract requires a silent counterpart")]
pub fn positive_manual_memcpy(source: &[u8], destination: &mut [u8]) {
    for index in 0..source.len() {
        destination[index] = source[index];
    }
}


pub fn positive_stable_sort_primitive(values: &mut [u64]) {
    values.sort();
}


pub fn positive_unnecessary_to_owned(value: &str) -> usize {
    borrowed_length(&value.to_owned())
}


pub fn positive_useless_vec() -> u64 {
    let values = vec![1_u64, 2, 3];
    values.iter().sum()
}


pub fn positive_vec_init_then_push(first: u64, second: u64) -> Vec<u64> {
    let mut values = Vec::new();
    values.push(first);
    values.push(second);
    values
}


fn positive_large_types_passed_by_value(values: [u64; 64]) -> usize {
    values.len()
}


fn positive_rc_buffer(value: Rc<String>) -> usize {
    value.len()
}


fn positive_redundant_allocation(value: Rc<Box<u64>>) -> u64 {
    **value
}


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


pub fn positive_ptr_arg(values: &Vec<u8>) -> usize {
    values.len()
}
