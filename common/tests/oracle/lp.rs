//! Input guard of `fair_lp_price_wad`: each non-positive reserve, price, or
//! share supply is rejected on its own, with every other input valid.
use crate::constants::WAD;
use crate::errors::OracleError;
use crate::oracle::lp::{fair_lp_price_wad, LpLeg, LpSupply};
use soroban_sdk::Env;

const RESERVE: i128 = 1_000_000_000;
const SHARES: i128 = 1_000_000_000;

fn leg(reserve: i128, price_wad: i128) -> LpLeg {
    LpLeg {
        reserve,
        decimals: 7,
        price_wad,
    }
}

fn supply(total_shares: i128) -> LpSupply {
    LpSupply {
        total_shares,
        decimals: 7,
    }
}

/// Inputs `[a.reserve, b.reserve, a.price_wad, b.price_wad, total_shares]`.
fn price(env: &Env, inputs: [i128; 5]) -> Result<i128, OracleError> {
    let [reserve_a, reserve_b, price_a, price_b, shares] = inputs;
    fair_lp_price_wad(
        env,
        &leg(reserve_a, price_a),
        &leg(reserve_b, price_b),
        &supply(shares),
    )
}

#[test]
fn valid_inputs_price_the_pool() {
    let env = Env::default();
    assert_eq!(
        price(&env, [RESERVE, RESERVE, WAD, WAD, SHARES]),
        Ok(2 * WAD)
    );
}

/// A guard joined with `&&` instead of `||` lets a single bad input through,
/// and a zero reserve or price then prices the pool at `Ok(0)`.
#[test]
fn each_non_positive_input_alone_is_rejected() {
    let env = Env::default();
    let valid = [RESERVE, RESERVE, WAD, WAD, SHARES];
    for field in 0..valid.len() {
        for bad in [0, -1] {
            let mut inputs = valid;
            inputs[field] = bad;
            assert_eq!(
                price(&env, inputs),
                Err(OracleError::InvalidPrice),
                "input {field} = {bad} must be rejected"
            );
        }
    }
}
