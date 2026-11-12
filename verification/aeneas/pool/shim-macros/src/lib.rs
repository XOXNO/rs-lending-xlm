//! Identity stand-ins for `#[contracttype]` and `#[contracterror]`.
//!
//! The real attributes derive the host (XDR) conversions of a type, which the
//! verified arithmetic never calls. Here they return the item unchanged, so
//! the production type definitions compile verbatim. The derives written in
//! the production source (`Clone`, `PartialEq`, ...) are kept as written.
extern crate proc_macro;
use proc_macro::TokenStream;

#[proc_macro_attribute]
pub fn contracttype(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

#[proc_macro_attribute]
pub fn contracterror(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}
