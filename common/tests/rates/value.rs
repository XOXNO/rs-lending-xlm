use super::*;
use crate::constants::{RAY, WAD};
use crate::math::test_support::{ceil_of, floor_of, half_up_of, Sampler};
use crate::rates::test_support::{
    assert_exact, knife_edge_shares, sample_index, unmetered_env, KNIFE_EDGE_INDEX,
};
use soroban_sdk::Env;

#[test]
fn position_value_half_up_matches_inline() {
    let env = Env::default();
    let scaled = Ray::from(10 * RAY);
    let index = Ray::from(RAY + RAY / 10);
    let price = Wad::from(2 * WAD);

    let got = position_value(&env, scaled, index, price);
    let expected = scaled.mul(&env, index).to_wad(&env).mul(&env, price);
    assert_eq!(got, expected);
}

#[test]
fn position_value_floor_and_ceil_bound_half_up() {
    let env = Env::default();
    let scaled = Ray::from(RAY + RAY * 4 / 10);
    let index = Ray::ONE;
    let price = Wad::from(WAD + WAD / 3);

    let half = position_value(&env, scaled, index, price);
    let floor = position_value_floor(&env, scaled, index, price);
    let ceil = position_value_ceil(&env, scaled, index, price);
    assert!(floor <= half);
    assert!(half <= ceil);
    assert_eq!(ceil.raw(), 1_866_666_666_666_666_667);
}

/// Each valuation rounds all three steps in its one direction: the multiply
/// by the index, the rescale from RAY to WAD, and the multiply by the price.
#[test]
fn exact_position_value_rounds_every_step_in_one_direction() {
    type Rounder = fn(i128, i128, i128) -> Option<i128>;
    type Valuation = fn(&Env, Ray, Ray, Wad) -> Wad;
    let valuations: [(Valuation, Rounder); 3] = [
        (position_value, half_up_of),
        (position_value_floor, floor_of),
        (position_value_ceil, ceil_of),
    ];

    let env = unmetered_env();
    let check = |scaled: i128, index: i128, price: i128| {
        let exact = |round: Rounder| {
            let actual = round(scaled, index, RAY)?;
            let actual_wad = round(actual, 1, RAY / WAD)?;
            round(actual_wad, price, WAD)
        };
        for (valuation, round) in valuations {
            assert_exact(
                &env,
                exact(round),
                |env| valuation(env, Ray::from(scaled), Ray::from(index), Wad::from(price)).raw(),
                (scaled, index, price),
            );
        }
        // The ceiling can overflow where the half-up value still fits, so it
        // is compared only where it is representable.
        if let (Some(floor), Some(half_up)) = (exact(floor_of), exact(half_up_of)) {
            assert!(floor <= half_up, "{scaled} at {index} priced {price}");
            assert!(
                exact(ceil_of).is_none_or(|ceil| half_up <= ceil),
                "{scaled} at {index} priced {price}"
            );
        }
    };

    // Values next to a rounding boundary of the rescale to WAD, where the
    // rounding of the multiply by the index decides the result. A price of
    // one carries that result through unchanged.
    for scaled in knife_edge_shares(RAY / WAD) {
        check(scaled, KNIFE_EDGE_INDEX, WAD);
    }
    let mut sampler = Sampler::new(61);
    for _ in 0..3_000 {
        let index = sample_index(&mut sampler);
        check(sampler.non_negative(), index, sampler.non_negative());
    }
}
