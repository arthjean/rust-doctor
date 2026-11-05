










pub mod reached;

#[path = "renamed/other.rs"]
pub mod aliased;



#[cfg(windows)]
pub mod platform;

include!("table.rs");

pub fn size() -> usize {
    TABLE.len()
}
