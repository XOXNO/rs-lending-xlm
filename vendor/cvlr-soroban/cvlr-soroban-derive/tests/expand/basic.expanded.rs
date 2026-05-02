#![allow(dead_code)]
use cvlr_soroban_derive::contractevent;
pub struct RoleGranted {
    pub role: u32,
    pub account: u64,
    pub caller: bool,
}
#[automatically_derived]
impl ::core::clone::Clone for RoleGranted {
    #[inline]
    fn clone(&self) -> RoleGranted {
        RoleGranted {
            role: ::core::clone::Clone::clone(&self.role),
            account: ::core::clone::Clone::clone(&self.account),
            caller: ::core::clone::Clone::clone(&self.caller),
        }
    }
}
#[automatically_derived]
impl ::core::fmt::Debug for RoleGranted {
    #[inline]
    fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
        ::core::fmt::Formatter::debug_struct_field3_finish(
            f,
            "RoleGranted",
            "role",
            &self.role,
            "account",
            &self.account,
            "caller",
            &&self.caller,
        )
    }
}
#[automatically_derived]
impl ::core::cmp::Eq for RoleGranted {
    #[inline]
    #[doc(hidden)]
    #[coverage(off)]
    fn assert_receiver_is_total_eq(&self) -> () {
        let _: ::core::cmp::AssertParamIsEq<u32>;
        let _: ::core::cmp::AssertParamIsEq<u64>;
        let _: ::core::cmp::AssertParamIsEq<bool>;
    }
}
#[automatically_derived]
impl ::core::marker::StructuralPartialEq for RoleGranted {}
#[automatically_derived]
impl ::core::cmp::PartialEq for RoleGranted {
    #[inline]
    fn eq(&self, other: &RoleGranted) -> bool {
        self.role == other.role && self.account == other.account
            && self.caller == other.caller
    }
}
impl RoleGranted {
    pub fn publish(&self, _env: &soroban_sdk::Env) {}
}
fn main() {
    let env = soroban_sdk::Env::default();
    let event = RoleGranted {
        role: 7,
        account: 42,
        caller: true,
    };
    event.publish(&env);
}
