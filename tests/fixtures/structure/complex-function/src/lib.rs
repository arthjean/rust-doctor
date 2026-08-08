//! Trigger and silence of the complexity rule.
//!
//! `schedule` buries its branches five levels deep and crosses the cognitive
//! threshold. `apply` is the neighbouring form the rule must leave alone: it
//! branches once and reads in one pass, which is what proves the rule was
//! checked for over-reach.

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
                } else if *weight % 2 == 0 {
                    total += 5;
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
