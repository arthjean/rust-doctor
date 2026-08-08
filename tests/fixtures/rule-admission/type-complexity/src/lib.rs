//! Trigger and silence of `clippy::type_complexity`.
//!
//! `routes` writes the whole nested type in its signature, which is the form
//! the lint reports. `named_routes` carries the same shape behind the alias
//! the lint's help asks for, and stays quiet.

use std::collections::HashMap;

pub fn routes() -> HashMap<String, Vec<Box<(String, Vec<(String, u32)>, fn(&str) -> u32)>>> {
    HashMap::new()
}

pub type Routes = HashMap<String, Vec<Box<(String, Vec<(String, u32)>, fn(&str) -> u32)>>>;

pub fn named_routes() -> Routes {
    HashMap::new()
}
