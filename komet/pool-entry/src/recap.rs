//! `recapitalize` (contracts/pool/src/ops/recapitalize.rs) with the backing
//! shortfall of `guards::backing_shortfall`. Fixed markets (supplied S,
//! borrowed B, supply index Is, borrow index Ib, 7 decimals, revenue R);
//! symbolic cash and amount with 0 <= cash and 0 <= amount <= shortfall (no
//! refund, so no token transfer). The shortfall is specified independently:
//!   sf = max(0, floor((floor(S*Is/RAY) - ceil(B*Ib/RAY)) / 10^20) - cash)
//! (0 when claims do not exceed debt). Claims: the call applies exactly
//! `amount` (= min(amount, sf)), cash increases by exactly that, and supplied,
//! borrowed, revenue, both indexes and the timestamp are unchanged
//! (INV-ACCT-04). A separate claim checks `sf` itself through an
//! over-shortfall probe: amount = sf + 1 is the boundary that would refund.

use common::constants::RAY;
use common::types::PoolStateRaw;
use pool::LiquidityPool;
use soroban_sdk::{Address, Env, I256};

use crate::{install_state, market_key, now_ms, read_state};

/// (S, B, Is, Ib, R) of the fixed markets.
pub const MARKETS: [(i128, i128, i128, i128, i128); 2] = [
    (
        5_000_000_000_000_000_000_000_000_000_000_000,
        3_000_000_000_000_000_000_000_000_000_000_000,
        RAY,
        RAY,
        7_000_000_000_000_000_000_000_000,
    ),
    (
        4_000_000_000_000_000_000_000_000_000_000_011,
        3_500_000_000_000_000_000_000_000_000_000_003,
        RAY / 10 * 11 + 13,
        RAY / 4 * 5 + 7,
        0,
    ),
];
const NATIVE_TO_RAY: i128 = 100_000_000_000_000_000_000; // 10^(27 - 7)

fn w(env: &Env, v: i128) -> I256 {
    I256::from_i128(env, v)
}

fn state(k: usize, cash: i128, ts: u64) -> PoolStateRaw {
    let (s, b, is, ib, r) = MARKETS[k];
    PoolStateRaw {
        supplied: s,
        borrowed: b,
        revenue: r,
        borrow_index: ib,
        supply_index: is,
        last_timestamp: ts,
        cash,
    }
}

/// Independent shortfall before cash: floor((floor(S*Is/RAY) - ceil(B*Ib/RAY)) / 10^20), >= 0.
pub fn uncovered(env: &Env, k: usize) -> i128 {
    let (s, b, is, ib, _) = MARKETS[k];
    let ray = w(env, RAY);
    let claims = w(env, s).mul(&w(env, is)).div(&ray);
    let debt_num = w(env, b).mul(&w(env, ib));
    let debt = debt_num.add(&ray).sub(&w(env, 1)).div(&ray);
    if claims <= debt {
        return 0;
    }
    claims
        .sub(&debt)
        .div(&w(env, NATIVE_TO_RAY))
        .to_i128()
        .unwrap_or(i128::MAX)
}

#[inline(never)]
fn recap(env: &Env, amount: i128) -> i128 {
    use pool_interface::LiquidityPoolInterface;
    // The refund is zero on every path of these claims, so the payer is
    // never paid; use this contract's own address.
    let payer: Address = env.current_contract_address();
    LiquidityPool::recapitalize(env.clone(), market_key(env), payer, amount).actual_amount
}

pub fn exact(env: &Env, k: usize, cash: i128, amount: i128) -> bool {
    let Some(ts) = now_ms(env) else { return true };
    let u = uncovered(env, k);
    if cash < 0 || amount < 0 || cash > i128::MAX - u {
        return true;
    }
    let sf = if u > cash { u - cash } else { 0 };
    if amount > sf {
        return true;
    }
    install_state(env, &state(k, cash, ts));
    let applied = recap(env, amount);
    let after = read_state(env);
    let (s, b, is, ib, r) = MARKETS[k];
    applied == amount
        && after.cash == cash + amount
        && after.supplied == s
        && after.borrowed == b
        && after.revenue == r
        && after.supply_index == is
        && after.borrow_index == ib
        && after.last_timestamp == ts
}

/// Deliberately false: recapitalizing never increases cash.
pub fn wrong(env: &Env) -> bool {
    let Some(ts) = now_ms(env) else { return true };
    install_state(env, &state(0, 0, ts));
    let applied = recap(env, 1);
    applied == 1 && read_state(env).cash == 0
}
