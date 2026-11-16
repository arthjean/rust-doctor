#[macro_export]
macro_rules! modules {
    ($($module:ident),* $(,)?) => {
        $(pub mod $module;)*
    };
}
