struct Resource;

impl Drop for Resource {
    fn drop(&mut self) {}
}

pub fn positive() {
    std::mem::forget(Resource);
}
