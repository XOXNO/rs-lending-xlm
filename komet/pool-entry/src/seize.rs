//! `seize_positions` with one Borrow (bad-debt) entry ->
//! `interest::apply_bad_debt_to_supply_index` (contracts/pool/src/interest.rs)
//! and `burn_debt`. Fixed market: supplied S, supply index I, borrowed B,
//! borrow index RAY, cash 0, revenue 0; symbolic seized position `pos`.
//!
//! Domain (stated bound): pos >= 2^64 (bad debt >= 2^64 Ray units), pos <= B,
//! and either bad >= T or the remaining value T - bad is at least
//! T * 2^64 / RAY. Inside it every production multiply-divide takes the exact
//! I256 path. T = halfup(S*I/RAY) is the total supplied value.
//!
//! Claims (I' = new supply index, c = min(bad, T)):
//!  (b) 10^24 <= I' <= I                                   (INV-IDX-02)
//!  (c) I' > 10^24  =>  I'*T <= I*(T - c)                  (INV-IDX-03, ADR-0012)
//!  (d) I' >= floor(I*(T - c)/T) - I/RAY - 1
//!  (e) bad >= T  =>  I' == 10^24
//!  and borrowed' == B - pos, supplied/revenue/cash/borrow index unchanged.

use common::constants::{RAY, SUPPLY_INDEX_FLOOR_RAW};
use common::types::{AccountPositionType, PoolSeizeEntry, PoolStateRaw, ScaledPositionRaw};
use pool::LiquidityPool;
use soroban_sdk::{vec, Env, I256};

use crate::{install_state, market_key, now_ms, read_state};

pub const WIDE: i128 = 1 << 64;
/// (supplied S, supply index I) of the two fixed markets.
pub const MARKETS: [(i128, i128); 2] = [
    (1_000_000_000_000_000_000_000_000_000_000_000, RAY),
    (
        3_000_000_000_000_000_000_000_000_000_000_007,
        RAY / 4 * 5 + 3,
    ),
];
/// Fixed outstanding scaled debt (borrow index RAY).
pub const BORROWED: i128 = 2_000_000_000_000_000_000_000_000_000_000_000;

fn w(env: &Env, v: i128) -> I256 {
    I256::from_i128(env, v)
}

fn state(k: usize, ts: u64) -> PoolStateRaw {
    PoolStateRaw {
        supplied: MARKETS[k].0,
        borrowed: BORROWED,
        revenue: 0,
        borrow_index: RAY,
        supply_index: MARKETS[k].1,
        last_timestamp: ts,
        cash: 0,
    }
}

#[inline(never)]
fn seize(env: &Env, pos: i128) {
    use pool_interface::LiquidityPoolInterface;
    LiquidityPool::seize_positions(
        env.clone(),
        vec![
            env,
            PoolSeizeEntry {
                hub_asset: market_key(env),
                side: AccountPositionType::Borrow,
                position: ScaledPositionRaw { scaled_amount: pos },
            },
        ],
    );
}

/// Total supplied value T = floor((S*I + RAY/2) / RAY), computed independently.
fn total_value(env: &Env, k: usize) -> I256 {
    w(env, MARKETS[k].0)
        .mul(&w(env, MARKETS[k].1))
        .add(&w(env, RAY / 2))
        .div(&w(env, RAY))
}

pub fn bad_debt_claims(env: &Env, k: usize, pos: i128) -> bool {
    let Some(ts) = now_ms(env) else { return true };
    if pos < WIDE || pos > BORROWED {
        return true;
    }
    let ray = w(env, RAY);
    let t = total_value(env, k);
    let bad = w(env, pos); // ceil(pos * RAY / RAY) at borrow index RAY
    let wiped = bad >= t;
    // Remaining value large enough for the exact I256 path, or zero.
    if !wiped && t.sub(&bad).mul(&ray) < w(env, WIDE).mul(&t) {
        return true;
    }
    install_state(env, &state(k, ts));
    seize(env, pos);
    let after = read_state(env);

    let i = w(env, MARKETS[k].1);
    let i_new = w(env, after.supply_index);
    let floor = w(env, SUPPLY_INDEX_FLOOR_RAW);
    let c = if wiped { t.clone() } else { bad };
    let kept = t.sub(&c);

    let unchanged = after.supplied == MARKETS[k].0
        && after.revenue == 0
        && after.cash == 0
        && after.borrow_index == RAY
        && after.borrowed == BORROWED - pos
        && after.last_timestamp == ts;
    let bounded = i_new >= floor && i_new <= i;
    let loss_lower = i_new == floor || i_new.mul(&t) <= i.mul(&kept);
    // I' >= q - I/RAY - 1 with q = floor(I*kept/T), in the rational form
    // (I' + 1)*RAY + I >= q*RAY; q <= I*kept/T, so this is (d) up to < 1.
    let q = i.mul(&kept).div(&t);
    let loss_upper = i_new.add(&w(env, 1)).mul(&ray).add(&i) >= q.mul(&ray);
    let wipe = !wiped || i_new == floor;
    unchanged && bounded && loss_lower && loss_upper && wipe
}

pub fn zero_debt(env: &Env, k: usize) -> bool {
    let Some(ts) = now_ms(env) else { return true };
    install_state(env, &state(k, ts));
    seize(env, 0);
    let after = read_state(env);
    after.supply_index == MARKETS[k].1 && after.borrowed == BORROWED
}

/// Deliberately false: a seizure of real debt claimed to cost suppliers nothing.
pub fn no_loss_wrong(env: &Env) -> bool {
    let Some(ts) = now_ms(env) else { return true };
    install_state(env, &state(0, ts));
    seize(env, 1_000_000_000_000_000_000_000_000_000_000);
    read_state(env).supply_index == MARKETS[0].1
}
