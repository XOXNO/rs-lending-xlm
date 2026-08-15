use super::*;
use crate::test_support::in_contract;
use common::constants::{MAX_REASONABLE_PRICE_WAD, WAD};
use common::types::{FeedNature, MultiFeedRef};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{String, Symbol};

fn properties(env: &Env, depth: u32) -> SourceProperties {
    SourceProperties {
        has_unsmoothed_market_leg: false,
        trust: Vec::new(env),
        loosest_max_stale_seconds: 0,
        depth,
    }
}

fn trusting(trust: &Vec<Address>) -> SourceProperties {
    SourceProperties {
        has_unsmoothed_market_leg: false,
        trust: trust.clone(),
        loosest_max_stale_seconds: 0,
        depth: 0,
    }
}

fn spot(env: &Env) -> SourceProperties {
    SourceProperties {
        has_unsmoothed_market_leg: true,
        ..properties(env, 0)
    }
}

fn lp(
    env: &Env,
    token_a: Address,
    token_b: Address,
    key_a: PriceKey,
    key_b: PriceKey,
) -> AquariusLpSource {
    AquariusLpSource {
        pool: Address::generate(env),
        token_a,
        token_b,
        key_a,
        key_b,
        reserve_a_decimals: 7,
        reserve_b_decimals: 7,
        min_pool_value_wad: WAD,
    }
}

fn scaled(env: &Env, min_factor_wad: i128, max_factor_wad: i128) -> ScaledSource {
    ScaledSource {
        factor: FeedSource {
            provider: ProviderRef::RedStone(MultiFeedRef {
                contract: Address::generate(env),
                feed_id: String::from_str(env, "F"),
                nature: FeedNature::Fundamental,
            }),
            decimals: 8,
            max_stale_seconds: 3_600,
        },
        quote: PriceKey::Ref(Symbol::new(env, "Q")),
        min_factor_wad,
        max_factor_wad,
    }
}

#[test]
fn test_a_chain_sitting_exactly_at_the_cap_is_admitted() {
    let env = Env::default();
    in_contract(&env, || {
        composition_depth(&env, &properties(&env, MAX_RESOLUTION_DEPTH));
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #229)")]
fn test_a_chain_one_level_past_the_cap_is_rejected() {
    let env = Env::default();
    in_contract(&env, || {
        composition_depth(&env, &properties(&env, MAX_RESOLUTION_DEPTH + 1));
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #38)")]
fn test_two_unsmoothed_legs_leave_the_config_spot_only() {
    let env = Env::default();
    in_contract(&env, || {
        let second = spot(&env);
        smoothing(&env, &spot(&env), Some(&second));
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #38)")]
fn test_a_lone_unsmoothed_leg_is_spot_only() {
    let env = Env::default();
    in_contract(&env, || smoothing(&env, &spot(&env), None));
}

#[test]
fn test_one_smoothed_leg_carries_the_pair() {
    let env = Env::default();
    in_contract(&env, || {
        let second = properties(&env, 0);
        smoothing(&env, &spot(&env), Some(&second));
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #232)")]
fn test_an_empty_allow_shared_declaration_is_refused_even_when_nothing_is_shared() {
    let env = Env::default();
    in_contract(&env, || {
        let first = trusting(&Vec::from_array(&env, [Address::generate(&env)]));
        let second = trusting(&Vec::from_array(&env, [Address::generate(&env)]));
        independence(
            &env,
            &first,
            &second,
            &IndependencePolicy::AllowShared(Vec::new(&env)),
        );
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #232)")]
fn test_a_declaration_naming_a_different_contract_does_not_cover_the_overlap() {
    let env = Env::default();
    in_contract(&env, || {
        let shared = Address::generate(&env);
        let legs = trusting(&Vec::from_array(&env, [shared]));
        independence(
            &env,
            &legs,
            &legs,
            &IndependencePolicy::AllowShared(Vec::from_array(&env, [Address::generate(&env)])),
        );
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #232)")]
fn test_a_declaration_wider_than_the_overlap_is_refused() {
    let env = Env::default();
    in_contract(&env, || {
        let shared = Address::generate(&env);
        let legs = trusting(&Vec::from_array(&env, [shared.clone()]));
        independence(
            &env,
            &legs,
            &legs,
            &IndependencePolicy::AllowShared(Vec::from_array(
                &env,
                [shared, Address::generate(&env)],
            )),
        );
    });
}

#[test]
fn test_a_declaration_matching_the_overlap_exactly_is_accepted() {
    let env = Env::default();
    in_contract(&env, || {
        let shared = Address::generate(&env);
        let legs = trusting(&Vec::from_array(&env, [shared.clone()]));
        independence(
            &env,
            &legs,
            &legs,
            &IndependencePolicy::AllowShared(Vec::from_array(&env, [shared])),
        );
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #220)")]
fn test_an_lp_whose_two_reserves_are_the_same_token_is_refused() {
    let env = Env::default();
    in_contract(&env, || {
        let token = Address::generate(&env);
        aquarius_lp_shape(
            &env,
            &lp(
                &env,
                token.clone(),
                token,
                PriceKey::Ref(Symbol::new(&env, "A")),
                PriceKey::Ref(Symbol::new(&env, "B")),
            ),
        );
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #220)")]
fn test_the_first_lp_leg_key_must_price_the_reserve_it_is_bound_to() {
    let env = Env::default();
    in_contract(&env, || {
        let token_b = Address::generate(&env);
        aquarius_lp_shape(
            &env,
            &lp(
                &env,
                Address::generate(&env),
                token_b.clone(),
                PriceKey::Token(Address::generate(&env)),
                PriceKey::Token(token_b),
            ),
        );
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #220)")]
fn test_the_second_lp_leg_key_must_price_the_reserve_it_is_bound_to() {
    let env = Env::default();
    in_contract(&env, || {
        let token_a = Address::generate(&env);
        aquarius_lp_shape(
            &env,
            &lp(
                &env,
                token_a.clone(),
                Address::generate(&env),
                PriceKey::Token(token_a),
                PriceKey::Token(Address::generate(&env)),
            ),
        );
    });
}

#[test]
fn test_an_lp_bound_to_its_own_reserve_tokens_is_accepted() {
    let env = Env::default();
    in_contract(&env, || {
        let token_a = Address::generate(&env);
        let token_b = Address::generate(&env);
        aquarius_lp_shape(
            &env,
            &lp(
                &env,
                token_a.clone(),
                token_b.clone(),
                PriceKey::Token(token_a),
                PriceKey::Token(token_b),
            ),
        );
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #224)")]
fn test_a_non_positive_factor_floor_is_refused() {
    let env = Env::default();
    in_contract(&env, || factor_bounds(&env, &scaled(&env, 0, WAD)));
}

#[test]
#[should_panic(expected = "Error(Contract, #224)")]
fn test_a_factor_ceiling_past_the_reasonable_price_cap_is_refused() {
    let env = Env::default();
    in_contract(&env, || {
        factor_bounds(&env, &scaled(&env, WAD, MAX_REASONABLE_PRICE_WAD + 1))
    });
}

#[test]
fn test_a_factor_ceiling_exactly_at_the_reasonable_price_cap_is_accepted() {
    let env = Env::default();
    in_contract(&env, || {
        factor_bounds(&env, &scaled(&env, WAD, MAX_REASONABLE_PRICE_WAD))
    });
}
