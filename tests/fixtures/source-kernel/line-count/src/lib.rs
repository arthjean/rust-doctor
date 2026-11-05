




pub mod helper;

pub fn shipped() -> usize {
    1
}



#[cfg(test)]
mod tests {
    use super::shipped;

    #[test]
    fn it_ships() {
        assert_eq!(shipped(), 1);
    }
}
