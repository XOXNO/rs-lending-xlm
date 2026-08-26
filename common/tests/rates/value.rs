use super::*;
use crate::constants::{RAY, WAD};
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
}
