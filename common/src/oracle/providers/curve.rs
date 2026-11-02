//! Views used to value plain Curve StableSwap LP tokens.

use soroban_sdk::{contractclient, contracttype, Address, Env, Vec};

/// Fixed-point scales a Curve pool reports in `pool_info`. The price
/// aggregator accepts a pool only when `precision` equals WAD.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CurvePoolScales {
    pub amplification_scale: i128,
    pub fee_denominator: i128,
    pub precision: i128,
}

/// Static pool metadata returned by `pool_info`. The price aggregator rechecks
/// `coins` (ordered), `coin_decimals`, `rates` and `scales.precision` against
/// the configured source on every read.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CurvePoolInfo {
    pub coin_count: u32,
    pub coin_decimals: Vec<u32>,
    pub coins: Vec<Address>,
    pub factory: Address,
    pub min_locked_liquidity: i128,
    pub rates: Vec<i128>,
    pub scales: CurvePoolScales,
}

/// Client interface for a plain Curve StableSwap pool, which is also its own
/// LP token.
#[contractclient(name = "CurvePoolClient")]
pub trait CurvePool {
    /// Returns the pool's coins, decimals, rates and scales.
    fn pool_info(env: Env) -> CurvePoolInfo;
    /// Returns the invariant D per LP share, scaled by `precision`.
    fn get_virtual_price(env: Env) -> i128;
    /// Returns the LP token's total supply.
    fn total_supply(env: Env) -> i128;
    /// Returns the LP token's decimals.
    fn decimals(env: Env) -> u32;
}
