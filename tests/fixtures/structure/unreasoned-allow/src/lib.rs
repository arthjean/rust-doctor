//! Trigger and silence of `rust_doctor::structure::unreasoned_allow_attribute`.
//!
//! Three families, chosen to exercise the three shapes a structural finding
//! takes. Two exemptions on `dead_code` sit in this file and form one family
//! whose `related` array stays inside it. Two exemptions on `unused_variables`
//! sit one here and one in the module below, and form a family whose `related`
//! array crosses files, because the rule reports what the codebase silences,
//! not what a file silences. The exemption inside the `#[cfg(test)]` module is
//! a family of one, which publishes no `related` key at all, and it is marked,
//! because no Cargo target kind names a test module living inside a library.
//!
//! Everything after them is the neighbouring form the rule must leave alone,
//! which is what proves it was checked for over-reach: an exemption that states
//! its reason, an `#[expect]` that expires by itself, and a `cfg_attr` whose
//! attribute never exists in the tree this pass reads.

pub mod reached;

#[allow(dead_code)]
pub struct First {
    unread: u8,
}

#[allow(dead_code)]
pub struct Second {
    unread: u8,
}

#[allow(unused_variables)]
pub fn alone(ignored: u8) {}

#[allow(dead_code, reason = "the field belongs to the published shape")]
pub struct Reasoned {
    unread: u8,
}

#[expect(dead_code)]
pub struct Expected {
    unread: u8,
}

#[cfg_attr(test, allow(dead_code))]
pub struct Conditional {
    pub read_by_the_caller: u8,
}

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use super::First;

    #[test]
    fn the_module_exists_to_carry_its_exemption() {}
}
