





mod negatives;

pub use negatives::*;


pub fn positive_unwrap_used(value: Option<u8>) -> u8 {
    value.unwrap()
}


pub fn positive_expect_used(value: Option<u8>) -> u8 {
    value.expect("the caller guarantees a value")
}


pub fn positive_panic(value: u8) -> u8 {
    if value == 0 {
        panic!("the value must not be zero");
    }
    value
}


pub fn positive_unreachable(value: u8) -> u8 {
    match value {
        0..=254 => value,
        _ => unreachable!(),
    }
}


pub fn positive_exit(value: u8) -> u8 {
    if value == 0 {
        std::process::exit(1);
    }
    value
}


pub fn positive_indexing_slicing(values: &[u8]) -> u8 {
    values[0]
}


pub fn positive_string_slice(value: &str) -> &str {
    &value[0..2]
}


#[allow(clippy::panic, reason = "the admission contract requires a silent counterpart")]
pub fn positive_panic_in_result_fn(value: u8) -> Result<u8, u8> {
    if value == 0 {
        panic!("the value must not be zero");
    }
    Ok(value)
}


pub fn positive_print_stdout(value: u8) {
    println!("value: {value}");
}


pub fn positive_print_stderr(value: u8) {
    eprintln!("value: {value}");
}
