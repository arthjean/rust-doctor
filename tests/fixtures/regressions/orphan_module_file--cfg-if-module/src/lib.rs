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
