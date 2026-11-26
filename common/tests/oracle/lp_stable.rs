//! Input guards of `fair_stable_lp_price_wad` and the `solve_stable_d` bounds
//! it reaches: each bad input is rejected on its own, with every other input
//! valid, and each normalized reserve is accepted exactly at its cap.
use crate::constants::WAD;
use crate::errors::OracleError;
use crate::oracle::lp::{LpLeg, LpSupply};
use crate::oracle::lp_stable::fair_stable_lp_price_wad;
use soroban_sdk::Env;

const RESERVE: i128 = 1_000_000_000;
const SHARES: i128 = 2_000_000_000;
const AMP: u128 = 100;
/// `solve_stable_d`'s `MIN_AMP` and `MAX_AMP`.
const MIN_AMP: u128 = 1;
const MAX_AMP: u128 = 1_000_000;
/// `solve_stable_d`'s `MAX_NORMALIZED_RESERVE_WAD`. An 18-decimal reserve is
/// already WAD-normalized, so the raw reserve hits the cap directly.
const CAP: i128 = 10i128.pow(34);

fn leg(reserve: i128, decimals: u32, price_wad: i128) -> LpLeg {
    LpLeg {
        reserve,
        decimals,
        price_wad,
    }
}

fn supply(total_shares: i128, decimals: u32) -> LpSupply {
    LpSupply {
        total_shares,
        decimals,
    }
}

/// Inputs `[a.reserve, b.reserve, a.price_wad, b.price_wad, total_shares]`,
/// all at 7 decimals.
fn price(env: &Env, inputs: [i128; 5], amp: u128) -> Result<i128, OracleError> {
    let [reserve_a, reserve_b, price_a, price_b, shares] = inputs;
    fair_stable_lp_price_wad(
        env,
        &leg(reserve_a, 7, price_a),
        &leg(reserve_b, 7, price_b),
        &supply(shares, 7),
        amp,
    )
}

/// Balanced 18-decimal pool with the given reserves, priced at par, whose
/// share supply equals the summed reserves at the cap.
fn price_at_cap(env: &Env, reserve_a: i128, reserve_b: i128) -> Result<i128, OracleError> {
    fair_stable_lp_price_wad(
        env,
        &leg(reserve_a, 18, WAD),
        &leg(reserve_b, 18, WAD),
        &supply(2 * CAP, 18),
        AMP,
    )
}

fn assert_near_par(price: Result<i128, OracleError>, what: &str) {
    let price = price.unwrap_or_else(|e| panic!("{what}: {e:?}"));
    assert!(
        (price - WAD).abs() <= 1_000,
        "{what}: price {price} not at par"
    );
}

#[test]
fn valid_inputs_price_the_pool_at_par() {
    let env = Env::default();
    for amp in [MIN_AMP, AMP, MAX_AMP] {
        assert_near_par(
            price(&env, [RESERVE, RESERVE, WAD, WAD, SHARES], amp),
            "balanced pool",
        );
    }
}

/// A guard joined with `&&` instead of `||` lets a single bad input through,
/// and a zero price then marks the pool at `Ok(0)`.
#[test]
fn each_non_positive_input_alone_is_rejected() {
    let env = Env::default();
    let valid = [RESERVE, RESERVE, WAD, WAD, SHARES];
    for field in 0..valid.len() {
        for bad in [0, -1] {
            let mut inputs = valid;
            inputs[field] = bad;
            assert_eq!(
                price(&env, inputs, AMP),
                Err(OracleError::InvalidPrice),
                "input {field} = {bad} must be rejected"
            );
        }
    }
}

#[test]
fn out_of_range_amp_alone_is_rejected() {
    let env = Env::default();
    for amp in [MIN_AMP - 1, MAX_AMP + 1] {
        assert_eq!(
            price(&env, [RESERVE, RESERVE, WAD, WAD, SHARES], amp),
            Err(OracleError::InvalidPrice),
            "amp {amp} must be rejected"
        );
    }
}

/// Both legs exactly at the cap solve, because the bound is inclusive.
#[test]
fn reserves_exactly_at_cap_are_accepted() {
    let env = Env::default();
    assert_near_par(price_at_cap(&env, CAP, CAP), "both legs at cap");
}

/// One leg a single unit over the cap, the other exactly at it, is rejected
/// in either leg order. The pool stays balanced, so only the cap check can
/// reject it.
#[test]
fn one_reserve_over_cap_alone_is_rejected() {
    let env = Env::default();
    for (reserve_a, reserve_b) in [(CAP + 1, CAP), (CAP, CAP + 1)] {
        assert_eq!(
            price_at_cap(&env, reserve_a, reserve_b),
            Err(OracleError::InvalidPrice),
            "{reserve_a}:{reserve_b} must be rejected"
        );
    }
}
