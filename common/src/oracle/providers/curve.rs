//! Views used to value plain Curve StableSwap LP tokens.

use soroban_sdk::{contractclient, contracttype, Address, Env, Vec};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CurvePoolScales {
    pub amplification_scale: i128,
    pub fee_denominator: i128,
    pub precision: i128,
}

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

#[contractclient(name = "CurvePoolClient")]
pub trait CurvePool {
    fn pool_info(env: Env) -> CurvePoolInfo;
    fn get_virtual_price(env: Env) -> i128;
    fn total_supply(env: Env) -> i128;
    fn decimals(env: Env) -> u32;
}
