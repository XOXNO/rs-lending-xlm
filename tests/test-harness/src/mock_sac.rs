//! Minimal SAC stub: exposes `decimals` only (no `symbol`) for token-shape
//! validation coverage (governance market-creation checks).

use soroban_sdk::{contract, contractimpl, Env};

#[contract]
pub struct MockSacNoSymbol;

#[contractimpl]
impl MockSacNoSymbol {
    pub fn decimals(_env: Env) -> u32 {
        7
    }
}
