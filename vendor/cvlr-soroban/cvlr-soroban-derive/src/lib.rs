use proc_macro::TokenStream;

mod mock_client;
mod rule;

#[proc_macro_attribute]
pub fn cvlr_mock_client(attr: TokenStream, item: TokenStream) -> TokenStream {
    mock_client::cvlr_mock_client(attr, item)
}

#[proc_macro]
pub fn declare_rule(input: TokenStream) -> TokenStream {
    rule::declare_rule(input)
}

#[proc_macro_attribute]
pub fn rule(attr: TokenStream, input: TokenStream) -> TokenStream {
    rule::rule(attr, input)
}

/// A compatibility stub for Soroban's `#[contractevent]`.
/// In CVLR builds we don't emit event metadata, but we still want event
/// structs to compile. We strip `#[topic]` so the attribute doesn't linger
/// as an unused field attribute.
/// # Example
/// ```
/// use cvlr_soroban_derive::contractevent;
/// use soroban_sdk::{Address, Symbol};
///
/// #[contractevent]
/// #[derive(Clone, Debug, Eq, PartialEq)]
/// pub struct RoleGranted {
///     #[topic]
///     pub role: Symbol,
///     #[topic]
///     pub account: Address,
///     pub caller: Address,
/// }
/// ```
#[proc_macro_attribute]
pub fn contractevent(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let mut output = syn::parse_macro_input!(item as syn::ItemStruct);

    // Remove #[topic] attributes from fields
    if let syn::Fields::Named(ref mut fields) = output.fields {
        for field in &mut fields.named {
            field.attrs.retain(|a| !a.path().is_ident("topic"));
        }
    }

    let ident = &output.ident;
    let generics = &output.generics;
    let (gen_impl, gen_types, gen_where) = generics.split_for_impl();

    quote::quote! {
        #output

        // stub out `publish`
        impl #gen_impl #ident #gen_types #gen_where {
            pub fn publish(&self, _env: &soroban_sdk::Env) {}
        }
    }
    .into()
}

/// A no-op attribute so `#[topic]` doesn't cause errors outside of
/// `#[contractevent]` contexts.
#[proc_macro_attribute]
pub fn topic(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}
