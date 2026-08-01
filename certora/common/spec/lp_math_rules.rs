use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume};
use soroban_sdk::{Env, U256};

use crate::constants::WAD;
use crate::oracle::lp::{fair_lp_price_wad, isqrt_of_product, LpLeg, LpSupply};

/// Stellar assets are 7 decimals; the rules below fix that so the prover reasons
/// about the arithmetic rather than the decimals ladder, which `policy` bounds.
const DEC: u32 = 7;

fn leg(reserve: i128, price_wad: i128) -> LpLeg {
    LpLeg {
        reserve,
        decimals: DEC,
        price_wad,
    }
}

fn supply(total_shares: i128) -> LpSupply {
    LpSupply {
        total_shares,
        decimals: DEC,
    }
}

/// The share price divides by supply and is fed to a collateral valuation, so
/// every degenerate input has to fail closed rather than produce a number or
/// trap. A trap would be worse than an error: it aborts the whole transaction
/// instead of marking one asset unusable.
#[rule]
fn lp_price_rejects_degenerate_inputs(
    e: Env,
    reserve_a: i128,
    reserve_b: i128,
    price_a: i128,
    price_b: i128,
    total_shares: i128,
) {
    cvlr_assume!(
        reserve_a <= 0 || reserve_b <= 0 || price_a <= 0 || price_b <= 0 || total_shares <= 0
    );

    let priced = fair_lp_price_wad(
        &e,
        &leg(reserve_a, price_a),
        &leg(reserve_b, price_b),
        &supply(total_shares),
    );

    cvlr_assert!(priced.is_err());
}

/// `share_decimals` past WAD has no upscale factor, so it must surface as an
/// error. This is the shape a mis-attested share token would take.
#[rule]
fn lp_price_rejects_share_decimals_past_wad(e: Env, share_decimals: u32) {
    cvlr_assume!(share_decimals > 18 && share_decimals <= 40);

    let priced = fair_lp_price_wad(
        &e,
        &leg(1_000_000, WAD),
        &leg(1_000_000, WAD),
        &LpSupply {
            total_shares: 1_000_000,
            decimals: share_decimals,
        },
    );

    cvlr_assert!(priced.is_err());
}

/// `2*sqrt(Va*Vb)` is symmetric, so which reserve is called `a` cannot change the
/// price. This is what makes a key_a/key_b ordering mistake at listing harmless,
/// and it is asserted here rather than left to the reviewer's arithmetic.
#[rule]
fn lp_price_is_symmetric_under_leg_swap(
    e: Env,
    reserve_a: i128,
    reserve_b: i128,
    price_a: i128,
    price_b: i128,
    total_shares: i128,
) {
    cvlr_assume!(reserve_a > 0 && reserve_a <= 1_000_000_000_000);
    cvlr_assume!(reserve_b > 0 && reserve_b <= 1_000_000_000_000);
    cvlr_assume!(price_a > 0 && price_a <= 1_000_000 * WAD);
    cvlr_assume!(price_b > 0 && price_b <= 1_000_000 * WAD);
    cvlr_assume!(total_shares > 0 && total_shares <= 1_000_000_000_000);

    let forward = fair_lp_price_wad(
        &e,
        &leg(reserve_a, price_a),
        &leg(reserve_b, price_b),
        &supply(total_shares),
    );
    let swapped = fair_lp_price_wad(
        &e,
        &leg(reserve_b, price_b),
        &leg(reserve_a, price_a),
        &supply(total_shares),
    );

    cvlr_assert!(forward.is_ok() == swapped.is_ok());
    if let (Ok(forward_wad), Ok(swapped_wad)) = (forward, swapped) {
        cvlr_assert!(forward_wad == swapped_wad);
    }
}

/// The fair value rounds down through the root, which is only conservative if it
/// really is the floor: `r*r <= a*b < (r+1)*(r+1)`. An over-estimating root would
/// over-price every LP share that uses it.
#[rule]
fn isqrt_is_the_integer_floor_of_the_root(e: Env, a: u64, b: u64) {
    let product = U256::from_u128(&e, u128::from(a)).mul(&U256::from_u128(&e, u128::from(b)));
    let root = isqrt_of_product(&e, u128::from(a), u128::from(b));
    let next = root.add(&U256::from_u32(&e, 1));

    cvlr_assert!(root.mul(&root) <= product);
    cvlr_assert!(next.mul(&next) > product);
}
