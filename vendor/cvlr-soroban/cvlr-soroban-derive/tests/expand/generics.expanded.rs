#![allow(dead_code)]
use cvlr_soroban_derive::contractevent;
pub struct GenericEvent<'a, T>
where
    T: Clone + Eq,
{
    pub label: &'a str,
    pub payload: T,
}
#[automatically_derived]
impl<'a, T: ::core::clone::Clone> ::core::clone::Clone for GenericEvent<'a, T>
where
    T: Clone + Eq,
{
    #[inline]
    fn clone(&self) -> GenericEvent<'a, T> {
        GenericEvent {
            label: ::core::clone::Clone::clone(&self.label),
            payload: ::core::clone::Clone::clone(&self.payload),
        }
    }
}
#[automatically_derived]
impl<'a, T: ::core::fmt::Debug> ::core::fmt::Debug for GenericEvent<'a, T>
where
    T: Clone + Eq,
{
    #[inline]
    fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
        ::core::fmt::Formatter::debug_struct_field2_finish(
            f,
            "GenericEvent",
            "label",
            &self.label,
            "payload",
            &&self.payload,
        )
    }
}
#[automatically_derived]
impl<'a, T: ::core::cmp::Eq> ::core::cmp::Eq for GenericEvent<'a, T>
where
    T: Clone + Eq,
{
    #[inline]
    #[doc(hidden)]
    #[coverage(off)]
    fn assert_receiver_is_total_eq(&self) -> () {
        let _: ::core::cmp::AssertParamIsEq<&'a str>;
        let _: ::core::cmp::AssertParamIsEq<T>;
    }
}
#[automatically_derived]
impl<'a, T> ::core::marker::StructuralPartialEq for GenericEvent<'a, T>
where
    T: Clone + Eq,
{}
#[automatically_derived]
impl<'a, T: ::core::cmp::PartialEq> ::core::cmp::PartialEq for GenericEvent<'a, T>
where
    T: Clone + Eq,
{
    #[inline]
    fn eq(&self, other: &GenericEvent<'a, T>) -> bool {
        self.label == other.label && self.payload == other.payload
    }
}
impl<'a, T> GenericEvent<'a, T>
where
    T: Clone + Eq,
{
    pub fn publish(&self, _env: &soroban_sdk::Env) {}
}
fn main() {
    let env = soroban_sdk::Env::default();
    let event = GenericEvent {
        label: "alpha",
        payload: 9_u32,
    };
    event.publish(&env);
}
