//! The other member of the family, in an integration test target.

fn gather(numbers: &[u32], bound: u32) -> u32 {
    let mut sum = 1;
    for number in numbers {
        if *number > bound {
            sum += *number;
        } else {
            sum -= bound;
        }
    }
    sum
}

#[test]
fn the_target_exists_to_carry_its_member() {
    assert_eq!(gather(&[4, 5], 1), 10);
}
