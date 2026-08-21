#![allow(dead_code)]

use cvlr_soroban_derive::contractevent;

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoleGranted {
    #[topic]
    pub role: u32,
    #[topic]
    pub account: u64,
    pub caller: bool,
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
