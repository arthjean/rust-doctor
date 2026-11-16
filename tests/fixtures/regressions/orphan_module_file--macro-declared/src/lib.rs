macro_rules! modules {
    ($($module:ident),* $(,)?) => {
        $(pub mod $module;)*
    };
}

modules!(engine, palette);
