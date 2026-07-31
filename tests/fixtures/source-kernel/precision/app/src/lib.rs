mod directory;
mod nested;
mod negative;
mod positives;
mod private;
mod shadowed;
mod shared;

mod inline {
    #[path = "custom.rs"]
    mod custom;
}

include!("ignored.rs");

pub fn library() {}
