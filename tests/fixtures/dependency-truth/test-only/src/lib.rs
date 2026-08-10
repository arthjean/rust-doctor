//! One dependency the shipped code uses, one only an inline test module
//! reaches, one only the integration test reaches, one nothing reaches.

pub fn shipped() -> u8 {
    probe_both::value()
}

#[cfg(test)]
mod tests {
    #[test]
    fn covered() {
        assert_eq!(probe_inline::value(), 1);
    }
}
