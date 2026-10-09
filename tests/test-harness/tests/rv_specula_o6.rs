//! Specula CR-13 (oracle): `fair_stable_lp_price_wad` traps on a U256
//! overflow inside `solve_stable_d` for reserves that pass the per-leg
//! `MAX_NORMALIZED_RESERVE_WAD` guard, instead of returning `InvalidPrice`.
//!
//! The guard bounds each leg at 1e34 WAD but not their ratio. On the first
//! Newton step `d = xa + xb`, and line 52 computes `d^3 / (2 xa)`, which
//! exceeds 2^256 once `xb^3 > 2^257 xa`. The thresholds below were rebuilt
//! with Python big integers from the same loop; the test pins them against
//! the SDK's `U256` so a fix (checked math or an imbalance cap) flips the
//! `trap` expectations to `Err(InvalidPrice)` and makes this file fail.

use common::constants::WAD;
use common::errors::OracleError;
use common::oracle::lp::{LpLeg, LpSupply};
use common::oracle::lp_stable::fair_stable_lp_price_wad;
use soroban_sdk::Env;
use std::panic::{catch_unwind, AssertUnwindSafe};

/// Legs and shares are 7-decimal raw units, as on the mainnet stable pool.
const DEC: u32 = 7;
/// Amplification of the mainnet-style pool used by the `common` snapshots.
const AMP: u128 = 1500;
/// One whole 7-decimal token.
const UNIT: i128 = 10_000_000;
/// Raw reserve that normalizes to exactly `MAX_NORMALIZED_RESERVE_WAD`
/// (1e34 WAD / 1e11 WAD per stroop).
const GUARD_RAW: i128 = 100_000_000_000_000_000_000_000;
/// Smallest raw `xb` (with `xa` = 1 stroop) whose first Newton step
/// overflows U256: 285_038_131_134_715_960_088_740_268_133 WAD / 1e11,
/// rounded up to the next stroop.
const OVERFLOW_RAW: i128 = 2_850_381_311_347_159_601;

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

/// Runs the pricer and separates a host trap (panic) from a returned
/// `Result`. The panic message is kept so a failure names the host error.
fn price(
    reserve_a: i128,
    reserve_b: i128,
    price_a: i128,
    price_b: i128,
) -> Result<Result<i128, OracleError>, String> {
    let env = Env::default();
    catch_unwind(AssertUnwindSafe(|| {
        fair_stable_lp_price_wad(
            &env,
            &leg(reserve_a, price_a),
            &leg(reserve_b, price_b),
            &supply(1_000_000_000),
            AMP,
        )
    }))
    .map_err(|payload| {
        payload
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_else(|| "non-string panic payload".to_string())
    })
}

/// CR-13 as stated: one stroop against a leg at the guard traps instead of
/// erroring. The guard admits both legs, then line 52 overflows.
#[test]
fn stable_lp_price_traps_inside_the_reserve_guard() {
    let trap = price(1, GUARD_RAW, WAD, WAD).expect_err("expected a host trap, got a Result");
    assert!(
        trap.contains("ArithDomain") || trap.contains("overflow"),
        "trap message should name the U256 overflow: {trap}"
    );
    let trap = price(GUARD_RAW, 1, WAD, WAD).expect_err("leg order must not matter");
    assert!(
        trap.contains("ArithDomain") || trap.contains("overflow"),
        "trap message should name the U256 overflow: {trap}"
    );
}

/// Pins the exact boundary from the big-integer model: one stroop below the
/// threshold converges and prices, one stroop at it traps.
#[test]
fn stable_lp_overflow_threshold_matches_the_integer_model() {
    let priced =
        price(1, OVERFLOW_RAW - 1, WAD, WAD).expect("one stroop below the threshold must not trap");
    assert!(
        priced.is_ok(),
        "below the threshold the pricer must converge: {priced:?}"
    );
    price(1, OVERFLOW_RAW, WAD, WAD)
        .expect_err("at the threshold the first Newton step must overflow");
}

/// Reachability bound for the mainnet 7-decimal stable pool: a pool holding a
/// single stroop against 1e9 or even 1e11 whole tokens still prices without
/// a trap. The trap needs more than 2.85e11 whole tokens on one side.
#[test]
fn mainnet_scale_imbalance_stays_below_the_trap() {
    for whole_tokens in [1_000_000_000i128, 100_000_000_000] {
        let priced = price(1, whole_tokens * UNIT, WAD, WAD)
            .unwrap_or_else(|trap| panic!("{whole_tokens} tokens vs 1 stroop trapped: {trap}"));
        assert!(
            priced.is_ok(),
            "{whole_tokens} tokens vs 1 stroop: {priced:?}"
        );
    }
}

/// The proposed fix also wraps the final `d.mul(min_price)`; that product is
/// at most 2e34 * i128::MAX ~ 3.4e72 < 2^256, so it cannot trap. With both
/// legs at the guard and both prices at `i128::MAX` the pricer returns `InvalidPrice`
/// from `try_u256_to_i128`, which is the fail-closed path.
#[test]
fn final_multiplication_never_traps() {
    let priced = price(GUARD_RAW, GUARD_RAW, i128::MAX, i128::MAX)
        .expect("final d * min_price fits U256 and must not trap");
    assert_eq!(priced, Err(OracleError::InvalidPrice));
}
