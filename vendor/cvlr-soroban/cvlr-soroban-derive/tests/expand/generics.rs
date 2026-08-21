#![allow(dead_code)]

use cvlr_soroban_derive::contractevent;

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GenericEvent<'a, T>
where
    T: Clone + Eq,
{
    #[topic]
    pub label: &'a str,
    pub payload: T,
}

fn main() {
    let env = soroban_sdk::Env::default();
    let event = GenericEvent {
        label: "alpha",
        payload: 9_u32,
    };

    event.publish(&env);
}
