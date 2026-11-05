







pub fn positive(value: &usize) -> usize {
    let pointer: *const usize = value;


    unsafe { std::mem::transmute::<*const usize, usize>(pointer) }
}


pub fn negative(value: &usize) -> usize {
    std::ptr::from_ref(value).addr()
}
