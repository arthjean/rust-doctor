//! No `mod orphan;` exists anywhere in this crate, so Cargo never compiles
//! this file. Nothing but a structural pass over the module tree can say so.

pub fn never_compiled() -> u8 {
    1
}
