






pub fn schedule(mut pending: u32, weights: &[u32], bound: u32) -> u32 {
    let mut total = 0;
    while pending > 0 {
        for weight in weights {
            if *weight > bound {
                if total > *weight {
                    match total % 3 {
                        0 => total += 1,
                        1 => {
                            if pending % 2 == 0 {
                                total += 2;
                            } else {
                                total += 3;
                            }
                        }
                        _ => {
                            if bound > 1 {
                                total += 4;
                            }
                        }
                    }
                } else if *weight % 5 == 0 {
                    total += 5;
                } else if *weight % 6 == 0 {
                    total += 6;
                } else if *weight % 7 == 0 {
                    total += 7;
                } else if *weight % 8 == 0 {
                    total += 8;
                } else if *weight % 9 == 0 {
                    total += 9;
                } else if *weight % 10 == 0 {
                    total += 10;
                } else if *weight % 11 == 0 {
                    total += 11;
                } else if *weight % 12 == 0 {
                    total += 12;
                } else if *weight % 13 == 0 {
                    total += 13;
                } else if *weight % 14 == 0 {
                    total += 14;
                } else {
                    total += 6;
                }
            }
        }
        pending -= 1;
    }
    total
}

pub fn apply(value: u32, bound: u32) -> u32 {
    if value > bound {
        bound
    } else {
        value
    }
}
