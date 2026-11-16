macro_rules! modules {
    ($label:ident: $($module:ident),* $(,)?) => {
        pub const LABEL: &str = stringify!($label);
        $(pub mod $module;)*
    };
}

macro_rules! checked {
    ($($module:ident),*) => {
        $(#[cfg(test)] mod $module;)*
    };
}

modules!(registry: alpha, beta, gamma);
checked!(checks);

pub fn total() -> u32 {
    alpha::value() + beta::value() + gamma::schedule(3, &[4, 5], 2)
}
