






#[repr(transparent)]
pub struct Meters(pub u64);


pub fn positive(raw: u64) -> Meters {

    let meters = unsafe { std::mem::transmute(raw) };
    meters
}


pub fn negative(raw: u64) -> Meters {

    let meters = unsafe { std::mem::transmute::<u64, Meters>(raw) };
    meters
}
