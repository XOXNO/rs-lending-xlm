//! Stand-in for `stellar_access::ownable` (OpenZeppelin Stellar), reached as
//! `stellar_access::ownable` through `extern crate self as stellar_access` in
//! `lib.rs`. Only `ops::revenue::apply` (owner lookup before the payout
//! transfer) names it, and no extraction root reaches it.

use soroban_sdk::{Address, Env};

pub fn get_owner(_env: &Env) -> Option<Address> {
    unimplemented!("ownable storage")
}
