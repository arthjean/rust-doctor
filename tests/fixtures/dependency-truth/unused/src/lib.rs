//! Every reference form US-005 collects, one dependency it must not credit.

use used_via_use::VALUE;
extern crate used_extern;

pub fn qualified() -> u8 {
    ::used_qualified::value()
}

pub fn from_macro() -> u8 {
    used_macro::probe!()
}

pub fn combined() -> u8 {
    alias_probe::value() + shared_helper::value() + used_extern::value() + VALUE
}
