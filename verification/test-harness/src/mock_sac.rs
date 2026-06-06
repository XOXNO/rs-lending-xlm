//! Minimal SAC stub: exposes `decimals` only (no `symbol`) for controller
//! `validate_and_fetch_token_decimals` coverage.

use soroban_sdk::{contract, contractimpl, Env};

#[contract]
pub struct MockSacNoSymbol;

#[contractimpl]
impl MockSacNoSymbol {
    pub fn decimals(_env: Env) -> u32 {
        7
    }
}