


mod helpers;



#[path = "../shared.rs"]
mod shared_again;



#[path = "../../benches/dual.rs"]
mod dual;

pub fn fixture() -> usize {
    helpers::seed()
}
