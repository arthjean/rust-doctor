


pub fn above(input: &[u32], limit: u32) -> u32 {
    fn visit(input: &[u32], limit: u32) -> u32 {
        let mut total = 0;
        for value in input {
            if *value > limit {
                total += *value;
            } else {
                total += 1;
            }
        }
        total
    }
    let mut total = 0;
    for value in input {
        if *value > limit {
            total += visit(input, *value);
        }
    }
    total
}

pub fn beyond(values: &[u32], bound: u32) -> u32 {
    fn visit(input: &[u32], limit: u32) -> u32 {
        let mut total = 0;
        for value in input {
            if *value > limit {
                total += *value;
            } else {
                total += 1;
            }
        }
        total
    }
    let mut sum = 0;
    for value in values {
        if *value > bound {
            sum += visit(values, *value);
        }
    }
    sum
}
