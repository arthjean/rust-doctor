//! Tokio's runtime attributes, passing the function through unchanged.

use proc_macro::TokenStream;

#[proc_macro_attribute]
pub fn main(_attributes: TokenStream, item: TokenStream) -> TokenStream {
    item
}

#[proc_macro_attribute]
pub fn test(_attributes: TokenStream, item: TokenStream) -> TokenStream {
    item
}
