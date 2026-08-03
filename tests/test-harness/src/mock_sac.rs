use soroban_sdk::{contract, contractimpl, Env};

#[contract]
pub struct MockSacNoSymbol;

#[contractimpl]
impl MockSacNoSymbol {
    pub fn decimals(_env: Env) -> u32 {
        7
    }
}
