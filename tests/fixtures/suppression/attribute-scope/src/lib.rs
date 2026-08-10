#![allow(clippy::unwrap_used)]
#![deny(unused_must_use)]
#![warn(dead_code)]

mod imports {
    #![allow(unused_imports)]

    pub fn present() {}
}

mod reasoned {
    #![allow(dead_code, reason = "the scope is what the detector judges")]

    pub fn also_present() {}
}

#[allow(dead_code)]
pub fn single_allow() {}

#[allow(dead_code)]
#[allow(unused_variables)]
pub fn stacked_pair() {
    let ignored = 1;
}

#[allow(dead_code, unused_imports, unreachable_code, unused_variables)]
pub fn wide_allow() {}

#[cfg_attr(test, allow(dead_code, unused_imports, unreachable_code, unused_variables))]
pub fn gated_allow() {}

pub fn uses_the_modules() {
    imports::present();
    reasoned::also_present();
}
