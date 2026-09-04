use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume};
use soroban_sdk::{Env, U256};

use crate::constants::WAD;
use crate::oracle::lp::{fair_lp_price_wad, isqrt_of_product, LpLeg, LpSupply};
use crate::oracle::lp_stable::fair_stable_lp_price_wad;

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

#[rule]
fn isqrt_is_the_integer_floor_of_the_root(e: Env, a: u64, b: u64) {
    let product = U256::from_u128(&e, u128::from(a)).mul(&U256::from_u128(&e, u128::from(b)));
    let root = isqrt_of_product(&e, u128::from(a), u128::from(b));
    let next = root.add(&U256::from_u32(&e, 1));

    // Known model gap, so this rule runs from its own conf outside the local
    // smoke set. The prover models `context/obj_cmp` on two U256 objects as
    // "equal digests give 0, otherwise a havoc in {1, -1}", so every ordering
    // comparison in `isqrt_of_product` (`n <= one`, `while y < x`) is
    // nondeterministic. Run 33850503287 returned a = 0x55555555555556, b = 3
    // (n = 2^56 + 2) with the `n <= one` branch taken; the Rust function
    // returns 2^28 and both assertions hold. Until U256 ordering is modelled,
    // any verdict here is about the model, not the code.
    cvlr_assert!(root.mul(&root) <= product);
    cvlr_assert!(next.mul(&next) > product);
}

#[rule]
fn stable_lp_price_rejects_degenerate_inputs(
    e: Env,
    reserve_a: i128,
    reserve_b: i128,
    price_a: i128,
    price_b: i128,
    total_shares: i128,
    amp: u128,
) {
    cvlr_assume!(
        reserve_a <= 0 || reserve_b <= 0 || price_a <= 0 || price_b <= 0 || total_shares <= 0
    );

    let priced = fair_stable_lp_price_wad(
        &e,
        &leg(reserve_a, price_a),
        &leg(reserve_b, price_b),
        &supply(total_shares),
        amp,
    );

    cvlr_assert!(priced.is_err());
}

#[rule]
fn stable_lp_price_rejects_out_of_range_amp(e: Env, amp: u128) {
    cvlr_assume!(amp == 0 || amp > 1_000_000);

    let priced = fair_stable_lp_price_wad(
        &e,
        &leg(1_000_000, WAD),
        &leg(1_000_000, WAD),
        &supply(2_000_000),
        amp,
    );

    cvlr_assert!(priced.is_err());
}

#[rule]
fn stable_lp_price_is_symmetric_under_leg_swap(
    e: Env,
    reserve_a: i128,
    reserve_b: i128,
    price_a: i128,
    price_b: i128,
    total_shares: i128,
    amp: u128,
) {
    cvlr_assume!(reserve_a > 0 && reserve_a <= 1_000_000_000_000);
    cvlr_assume!(reserve_b > 0 && reserve_b <= 1_000_000_000_000);
    cvlr_assume!(price_a > 0 && price_a <= 1_000_000 * WAD);
    cvlr_assume!(price_b > 0 && price_b <= 1_000_000 * WAD);
    cvlr_assume!(total_shares > 0 && total_shares <= 1_000_000_000_000);
    cvlr_assume!(amp >= 1 && amp <= 1_000_000);

    let forward = fair_stable_lp_price_wad(
        &e,
        &leg(reserve_a, price_a),
        &leg(reserve_b, price_b),
        &supply(total_shares),
        amp,
    );
    let swapped = fair_stable_lp_price_wad(
        &e,
        &leg(reserve_b, price_b),
        &leg(reserve_a, price_a),
        &supply(total_shares),
        amp,
    );

    cvlr_assert!(forward.is_ok() == swapped.is_ok());
    if let (Ok(forward_wad), Ok(swapped_wad)) = (forward, swapped) {
        cvlr_assert!(forward_wad == swapped_wad);
    }
}

#[rule]
fn stable_lp_price_tracks_only_the_cheaper_leg(
    e: Env,
    reserve_a: i128,
    reserve_b: i128,
    price_low: i128,
    price_hi_1: i128,
    price_hi_2: i128,
    total_shares: i128,
    amp: u128,
) {
    cvlr_assume!(reserve_a > 0 && reserve_a <= 1_000_000_000_000);
    cvlr_assume!(reserve_b > 0 && reserve_b <= 1_000_000_000_000);
    cvlr_assume!(total_shares > 0 && total_shares <= 1_000_000_000_000);
    cvlr_assume!(amp >= 1 && amp <= 1_000_000);
    cvlr_assume!(price_low > 0 && price_low <= 1_000_000 * WAD);
    cvlr_assume!(price_hi_1 >= price_low && price_hi_1 <= 1_000_000 * WAD);
    cvlr_assume!(price_hi_2 >= price_low && price_hi_2 <= 1_000_000 * WAD);

    let priced_1 = fair_stable_lp_price_wad(
        &e,
        &leg(reserve_a, price_low),
        &leg(reserve_b, price_hi_1),
        &supply(total_shares),
        amp,
    );
    let priced_2 = fair_stable_lp_price_wad(
        &e,
        &leg(reserve_a, price_low),
        &leg(reserve_b, price_hi_2),
        &supply(total_shares),
        amp,
    );

    cvlr_assert!(priced_1.is_ok() == priced_2.is_ok());
    if let (Ok(wad_1), Ok(wad_2)) = (priced_1, priced_2) {
        cvlr_assert!(wad_1 == wad_2);
    }
}
