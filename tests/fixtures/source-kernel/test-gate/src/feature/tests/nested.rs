


pub fn deep() -> usize {
    let mut total = 0;
    for step in 0..10 {
        total += step * 3;
        total -= step;
    }
    let doubled = total * 2;
    doubled + 7
}
