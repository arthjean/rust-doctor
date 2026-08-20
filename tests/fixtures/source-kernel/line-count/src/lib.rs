//! Every line of this file is counted, whatever it holds.

// A comment line is a line. The denominator measures the file the reader
// maintains, not the subset the compiler keeps.

pub mod helper;

pub fn shipped() -> usize {
    1
}

// The blank line above and the one below are counted too.

#[cfg(test)]
mod tests {
    use super::shipped;

    #[test]
    fn it_ships() {
        assert_eq!(shipped(), 1);
    }
}
