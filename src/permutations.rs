//! The next permutation in lexicographic order.
//!
//! Two determinism proofs enumerate orders this way, one for the layers a
//! policy compiles from and one for the entries a configuration file carries.
//! They each used to carry a copy of the walk, one of them inlined in the body
//! of its test. A proof is worth what its enumeration is worth, so the
//! enumeration is written once.

/// Rewrites `order` as the next permutation in lexicographic order, and answers
/// whether there was one.
///
/// `false` means the last permutation was already there: a caller enumerating
/// fewer orders than the factorial asserts on the answer rather than trusting
/// the count it wrote.
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
    // Every value greater than the head sits in the descending run after the
    // pivot, since the pivot is the last position that rises, so searching from
    // the end lands inside that run.
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

        // A sequence with nothing to raise is the last one, whatever its width.
        assert!(!next_permutation(&mut [2, 1, 0]));
        assert!(!next_permutation(&mut [0]));
        assert!(!next_permutation(&mut []));
    }
}
