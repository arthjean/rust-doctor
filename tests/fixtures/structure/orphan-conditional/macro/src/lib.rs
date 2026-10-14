//! A module declared inside a macro's tokens, the way `cfg_if!` writes it.

macro_rules! cfg_if {
    ($(if #[cfg($meta:meta)] { $($item:item)* })*) => {
        $( $( #[cfg($meta)] $item )* )*
    };
}

cfg_if! {
    if #[cfg(unix)] {
        mod imp;
    }
}

#[cfg(unix)]
pub fn name() -> &'static str {
    imp::name()
}
