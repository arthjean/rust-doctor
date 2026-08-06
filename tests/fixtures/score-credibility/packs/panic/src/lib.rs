//! Positive fixture of the panic and placeholders pack.
//!
//! Every function triggers exactly one lint of the pack. When two lints of the
//! pack aim at the same expression, the neighbouring lint is neutralized
//! locally by `#[allow]` so that the count stays one diagnostic per lint.

mod negatives;

pub use negatives::*;

/// clippy::unwrap_used
pub fn positive_unwrap_used(value: Option<u8>) -> u8 {
    value.unwrap()
}

/// clippy::expect_used
pub fn positive_expect_used(value: Option<u8>) -> u8 {
    value.expect("the caller guarantees a value")
}

/// clippy::panic
pub fn positive_panic(value: u8) -> u8 {
    if value == 0 {
        panic!("the value must not be zero");
    }
    value
}

/// clippy::unreachable
pub fn positive_unreachable(value: u8) -> u8 {
    match value {
        0..=254 => value,
        _ => unreachable!(),
    }
}

/// clippy::exit
pub fn positive_exit(value: u8) -> u8 {
    if value == 0 {
        std::process::exit(1);
    }
    value
}

/// clippy::indexing_slicing
pub fn positive_indexing_slicing(values: &[u8]) -> u8 {
    values[0]
}

/// clippy::string_slice
pub fn positive_string_slice(value: &str) -> &str {
    &value[0..2]
}

/// clippy::panic_in_result_fn
#[allow(clippy::panic)]
pub fn positive_panic_in_result_fn(value: u8) -> Result<u8, u8> {
    if value == 0 {
        panic!("the value must not be zero");
    }
    Ok(value)
}

/// clippy::print_stdout
pub fn positive_print_stdout(value: u8) {
    println!("value: {value}");
}

/// clippy::print_stderr
pub fn positive_print_stderr(value: u8) {
    eprintln!("value: {value}");
}
