







pub fn shared() -> usize {
    let mut count = 1;
    while count < 40 {
        count = count * 2 + 3;
    }
    let scaled = count / 4;
    scaled - 1
}
