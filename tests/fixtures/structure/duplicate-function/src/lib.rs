//! Trigger and silence of the two duplication rules.
//!
//! `total_above` and `sum_over_bound` are the same function: the names differ,
//! the local differs, the literal differs, and nothing else does. They form one
//! exact family of two, and the diagnostic points at the first of them in
//! sorted order.
//!
//! `clamped_total` in the module below is that same function with one branch
//! added, which is the shape a copy takes once it is edited. It forms a near
//! family with the exact one, scored rather than equal.
//!
//! Everything after them is the neighbouring form the rules must leave alone,
//! which is what proves they were checked for over-reach: two functions that
//! really are identical but too small to be worth a line of a report, and one
//! function of comparable size that does something else.

pub mod edited;

pub fn total_above(values: &[u32], limit: u32) -> u32 {
    let mut total = 0;
    for value in values {
        if *value > limit {
            total += *value;
        } else {
            total -= limit;
        }
    }
    total
}

pub fn sum_over_bound(numbers: &[u32], bound: u32) -> u32 {
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

pub fn first_flag(value: u32) -> bool {
    value > 0
}

pub fn second_flag(other: u32) -> bool {
    other > 1
}

pub fn shout(text: &str, mark: char) -> String {
    let mut collected = String::new();
    for letter in text.chars() {
        if letter == mark {
            collected.push(letter.to_ascii_uppercase());
        } else {
            collected.push('-');
        }
    }
    collected
}

#[cfg(test)]
mod tests {
    fn counted(limit: u32) -> u32 {
        let mut index = 0;
        let mut seen = 0;
        while index < limit {
            seen += index;
            index += 1;
        }
        seen
    }

    fn tallied(bound: u32) -> u32 {
        let mut step = 0;
        let mut kept = 0;
        while step < bound {
            kept += step;
            step += 1;
        }
        kept
    }

    #[test]
    fn the_module_exists_to_carry_its_family() {
        assert_eq!(counted(3), tallied(3));
    }
}
