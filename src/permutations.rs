pub(crate) fn next_permutation(order: &mut [usize]) -> bool {
    let Some(pivot) = order
        .windows(2)
        .rposition(|pair| matches!(pair, [left, right] if left < right))
    else {
        return false;
    };
    let Some(&head) = order.get(pivot) else {
        return false;
    };
    let Some(successor) = order.iter().rposition(|value| *value > head) else {
        return false;
    };
    order.swap(pivot, successor);
    if let Some(tail) = order.get_mut(pivot + 1..) {
        tail.reverse();
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_walk_enumerates_every_order_once_and_stops_on_the_last() {
        let mut order = [0, 1, 2];
        let mut seen = vec![order];
        while next_permutation(&mut order) {
            seen.push(order);
        }
        assert_eq!(
            seen,
            [
                [0, 1, 2],
                [0, 2, 1],
                [1, 0, 2],
                [1, 2, 0],
                [2, 0, 1],
                [2, 1, 0],
            ]
        );

        assert!(!next_permutation(&mut [2, 1, 0]));
        assert!(!next_permutation(&mut [0]));
        assert!(!next_permutation(&mut []));
    }
}
