//! Blend v2 backstop stand-in for live test pools.
//!
//! A Blend pool reads `pool_data` once, in `set_status(0)`, to check that its
//! backstop is funded. This contract answers that call only. Any other
//! backstop call fails, so a new dependency on the backstop shows at once.

#![no_std]

use soroban_sdk::{contract, contractimpl, contracttype, Address, Env};

/// One whole token at Blend's 7 decimals.
const SCALAR_7: i128 = 10_000_000;

/// Field set and order of Blend v2 `PoolBackstopData`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PoolBackstopData {
    pub tokens: i128,
    pub shares: i128,
    pub q4w_pct: i128,
    pub blnd: i128,
    pub usdc: i128,
    pub token_spot_price: i128,
}

#[contract]
pub struct MockBlendBackstop;

#[contractimpl]
impl MockBlendBackstop {
    /// Reports a backstop above Blend's threshold with no queued withdrawals.
    ///
    /// The threshold needs `blnd^4 * usdc >= 100_000^5` in whole tokens;
    /// 1e6 of each gives 1e30.
    pub fn pool_data(_env: Env, _pool: Address) -> PoolBackstopData {
        let whole = 1_000_000 * SCALAR_7;
        PoolBackstopData {
            tokens: whole,
            shares: whole,
            q4w_pct: 0,
            blnd: whole,
            usdc: whole,
            token_spot_price: SCALAR_7,
        }
    }
}
