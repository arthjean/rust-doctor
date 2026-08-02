use std::rc::Rc;

pub struct UnsafeSend {
    value: Rc<u8>,
}

unsafe impl Send for UnsafeSend {}
