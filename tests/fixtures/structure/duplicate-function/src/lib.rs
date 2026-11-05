















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
