use std::rc::Rc;

struct Shared {
    values: Rc<Vec<u8>>,
}

pub fn total() -> usize {
    let shared = Shared {
        values: Rc::new(vec![1, 2, 3]),
    };
    shared.values.len()
}

pub fn answer(value: Option<u8>) -> u8 {
    value.unwrap()
}
