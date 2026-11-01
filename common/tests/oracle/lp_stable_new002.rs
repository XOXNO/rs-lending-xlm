//! Regression tests for NEW-002 (Specula run 3 CR-6 and LP-4): `solve_stable_d`
//! on heavily imbalanced pools.
//!
//! - Defect 1: dividing by the larger reserve first let the integer Newton
//!   iteration settle into a cycle whose iterates differ by more than 1, so
//!   it never met the `|d - d_prev| <= 1` exit and returned `InvalidPrice`.
//! - Defect 2: on an extremely lopsided pool the first Newton step overflowed
//!   `U256` and the host trapped instead of returning `InvalidPrice`.

use super::*;
use crate::constants::WAD;
use crate::oracle::lp::isqrt_of_product;

/// One token at 7 decimals, normalized to WAD.
const TOKEN_WAD: i128 = WAD;

/// Exact `floor(D)` of the 10^10 : 10^7 token pool at amp 2, computed off
/// chain by bisection on `ann * (xa + xb) + D = ann * D + D^3 / (4 xa xb)`
/// with `ann = 2 * amp`.
const TWO_CYCLE_ROOT: u128 = 2_362_217_299_922_534_376_201_222_040;

fn u256_to_u128(d: &U256) -> u128 {
    d.to_u128().expect("D fits u128 in these fixtures")
}

/// The reference failure. Before the fix the iterates alternate between
/// root + 1 and root + 3 from iteration 11 to 255, and the solver returns
/// `InvalidPrice`.
#[test]
fn new002_two_cycle_pool_solves() {
    let env = Env::default();
    let got = solve_stable_d(&env, 10_000_000_000 * TOKEN_WAD, 10_000_000 * TOKEN_WAD, 2);
    let d = u256_to_u128(&got.expect("a valid 1,000:1 pool at amp 2 must solve"));
    let tolerance = TWO_CYCLE_ROOT / 1_000_000_000_000;
    assert!(
        d.abs_diff(TWO_CYCLE_ROOT) <= tolerance,
        "D {d} is not within {tolerance} of the exact root {TWO_CYCLE_ROOT}"
    );
}

/// Control, passes today: the same pool with the legs swapped converges to
/// the exact root in 10 iterations. `D` is symmetric in the reserves, so
/// only the solver's path differs.
#[test]
fn new002_two_cycle_pool_with_legs_swapped_solves_exactly() {
    let env = Env::default();
    let got = solve_stable_d(&env, 10_000_000 * TOKEN_WAD, 10_000_000_000 * TOKEN_WAD, 2);
    assert_eq!(got.map(|d| u256_to_u128(&d)), Ok(TWO_CYCLE_ROOT));
}

/// Defect 2. 10^15 tokens against 1,000 at amp 100: the first step needs a
/// 259-bit intermediate. Before the fix the host traps with
/// `Error(Object, ArithDomain)`; the documented contract is a `Result`.
#[test]
fn new002_extreme_imbalance_returns_a_result_instead_of_trapping() {
    let env = Env::default();
    let got = solve_stable_d(
        &env,
        1_000_000_000_000_000 * TOKEN_WAD,
        1_000 * TOKEN_WAD,
        100,
    );
    assert!(
        matches!(got, Ok(_) | Err(OracleError::InvalidPrice)),
        "unexpected result {got:?}"
    );
}

/// Defect 2 through the public pricing entry point the Aquarius provider
/// calls. Prices are 1 WAD, supply is one share.
#[test]
fn new002_extreme_imbalance_price_returns_a_result_instead_of_trapping() {
    let env = Env::default();
    let leg = |tokens: i128| LpLeg {
        reserve: tokens * 10_000_000,
        decimals: 7,
        price_wad: WAD,
    };
    let supply = LpSupply {
        total_shares: 10_000_000,
        decimals: 7,
    };
    let got =
        fair_stable_lp_price_wad(&env, &leg(1_000_000_000_000_000), &leg(1_000), &supply, 100);
    assert!(
        matches!(got, Ok(_) | Err(OracleError::InvalidPrice)),
        "unexpected result {got:?}"
    );
}

/// Defect 1 across a grid of admitted pools: amp in {1, 2, 5, 10, 100,
/// 1,000}, imbalance 500:1 to 1,000,000:1, larger leg 10^3 to 10^11 tokens,
/// both leg orders. Every pool must solve, inside the invariant's bounds
/// `[2 sqrt(xa xb), xa + xb]`. Pools this size never overflow, so a failure
/// here is always the non-convergence of Defect 1.
#[test]
fn new002_imbalanced_pool_grid_solves() {
    let env = Env::default();
    let mut failures = 0u32;
    let mut first_failure = None;
    let mut total = 0u32;
    for amp in [1u128, 2, 5, 10, 100, 1_000] {
        for ratio in [500i128, 1_000, 10_000, 1_000_000] {
            for exp in 3..=11u32 {
                let big = 10i128.pow(exp) * TOKEN_WAD;
                let small = big / ratio;
                for (xa, xb) in [(big, small), (small, big)] {
                    total += 1;
                    // One solve per default budget, as in a real invocation.
                    env.cost_estimate().budget().reset_default();
                    match solve_stable_d(&env, xa, xb, amp) {
                        Ok(d) => {
                            let d = u256_to_u128(&d);
                            let lower =
                                2 * u256_to_u128(&isqrt_of_product(&env, xa as u128, xb as u128));
                            let upper = (xa + xb) as u128;
                            assert!(
                                d + 2 >= lower && d <= upper,
                                "D {d} outside [{lower}, {upper}]: xa {xa}, xb {xb}, amp {amp}"
                            );
                        }
                        Err(e) => {
                            failures += 1;
                            first_failure.get_or_insert((xa, xb, amp, e));
                        }
                    }
                }
            }
        }
    }
    assert!(
        failures == 0,
        "{failures} of {total} admitted pools failed to solve; first (xa, xb, amp, error): {first_failure:?}"
    );
}
