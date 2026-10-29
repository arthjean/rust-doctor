//! `unsafe_code` is allowed by default and no catalogued rule reports it, so
//! allowing it file-wide hides nothing from the scan.
#![allow(unsafe_code)]

pub fn length(text: &str) -> usize {
    text.len()
}
