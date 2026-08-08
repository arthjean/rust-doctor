//! Negative fixture of the panic and placeholders pack.
//!
//! The `negative_*` block is the idiomatic form the neighbouring lint must not
//! report. The `negative_allowed_*` block covers local neutralization through
//! `#[allow]`, required by the admission contract.

use std::fmt::Write as _;

/// negative_unwrap_used
pub fn negative_unwrap_used(value: Option<u8>) -> u8 {
    value.unwrap_or_default()
}

/// negative_expect_used
pub fn negative_expect_used(value: Option<u8>) -> Result<u8, u8> {
    value.ok_or(0)
}

/// negative_panic
pub fn negative_panic(value: u8) -> Result<u8, u8> {
    if value == 0 {
        return Err(0);
    }
    Ok(value)
}

/// negative_unreachable
pub fn negative_unreachable(value: bool) -> u8 {
    match value {
        true => 1,
        false => 0,
    }
}

/// negative_exit
pub fn negative_exit(value: u8) -> Result<u8, u8> {
    if value == 0 { Err(1) } else { Ok(value) }
}

/// negative_indexing_slicing
pub fn negative_indexing_slicing(values: &[u8]) -> Option<u8> {
    values.first().copied()
}

/// negative_string_slice
pub fn negative_string_slice(value: &str) -> Option<&str> {
    value.get(0..2)
}

/// negative_panic_in_result_fn
pub fn negative_panic_in_result_fn(value: u8) -> Result<u8, u8> {
    if value == 0 { Err(0) } else { Ok(value) }
}

/// negative_print_stdout
pub fn negative_print_stdout(value: u8) -> String {
    let mut rendered = String::new();
    let _ = write!(rendered, "value: {value}");
    rendered
}

/// negative_print_stderr
pub fn negative_print_stderr(value: u8, sink: &mut String) {
    let _ = writeln!(sink, "value: {value}");
}

/// negative_allowed_unwrap_used
#[allow(clippy::unwrap_used, reason = "the admission contract requires a silent counterpart")]
pub fn negative_allowed_unwrap_used(value: Option<u8>) -> u8 {
    value.unwrap()
}

/// negative_allowed_expect_used
#[allow(clippy::expect_used, reason = "the admission contract requires a silent counterpart")]
pub fn negative_allowed_expect_used(value: Option<u8>) -> u8 {
    value.expect("locally allowed")
}

/// negative_allowed_panic
#[allow(clippy::panic, reason = "the admission contract requires a silent counterpart")]
pub fn negative_allowed_panic() -> u8 {
    panic!("locally allowed")
}

/// negative_allowed_unreachable
#[allow(clippy::unreachable, reason = "the admission contract requires a silent counterpart")]
pub fn negative_allowed_unreachable(value: u8) -> u8 {
    match value {
        0..=254 => value,
        _ => unreachable!(),
    }
}

/// negative_allowed_exit
#[allow(clippy::exit, reason = "the admission contract requires a silent counterpart")]
pub fn negative_allowed_exit() -> u8 {
    std::process::exit(1)
}

/// negative_allowed_indexing_slicing
#[allow(clippy::indexing_slicing, reason = "the admission contract requires a silent counterpart")]
pub fn negative_allowed_indexing_slicing(values: &[u8]) -> u8 {
    values[0]
}

/// negative_allowed_string_slice
#[allow(clippy::string_slice, reason = "the admission contract requires a silent counterpart")]
pub fn negative_allowed_string_slice(value: &str) -> &str {
    &value[0..2]
}

/// negative_allowed_panic_in_result_fn
#[allow(clippy::panic, clippy::panic_in_result_fn, reason = "the admission contract requires a silent counterpart")]
pub fn negative_allowed_panic_in_result_fn(value: u8) -> Result<u8, u8> {
    if value == 0 {
        panic!("locally allowed");
    }
    Ok(value)
}

/// negative_allowed_print_stdout
#[allow(clippy::print_stdout, reason = "the admission contract requires a silent counterpart")]
pub fn negative_allowed_print_stdout(value: u8) {
    println!("value: {value}");
}

/// negative_allowed_print_stderr
#[allow(clippy::print_stderr, reason = "the admission contract requires a silent counterpart")]
pub fn negative_allowed_print_stderr(value: u8) {
    eprintln!("value: {value}");
}
