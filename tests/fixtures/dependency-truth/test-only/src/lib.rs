


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
