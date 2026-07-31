use crate::errors::Error;
use crate::types::SwapVenue;
use crate::{Router, RouterClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{token, vec, Address, Env};

use super::support::{aquarius_mock, new_asset, one_hop_path, strategy_xdr};

#[test]
fn split_ppm_must_sum_to_one_million() {
    let env = Env::default();
    env.mock_all_auths();
    let router_addr = env.register(Router, (Address::generate(&env),));
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (token_a, sac_a) = new_asset(&env, &admin);
    let (token_b, _) = new_asset(&env, &admin);
    let pool = env.register(aquarius_mock::AqPool, ());
    sac_a.mint(&sender, &1_000);

    let swap_xdr = strategy_xdr(
        &env,
        token_a.clone(),
        token_b.clone(),
        1,
        vec![
            &env,
            one_hop_path(
                &env,
                SwapVenue::Aquarius,
                pool.clone(),
                token_a.clone(),
                token_b.clone(),
                600_000,
            ),
            one_hop_path(&env, SwapVenue::Aquarius, pool, token_a, token_b, 200_000),
        ],
    );
    let err = RouterClient::new(&env, &router_addr)
        .try_execute_strategy(&sender, &100, &swap_xdr)
        .unwrap_err();
    assert_eq!(err.unwrap(), Error::SplitPpmMismatch.into());
}

#[test]
fn split_ppm_zero_path_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let router_addr = env.register(Router, (Address::generate(&env),));
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (token_a, sac_a) = new_asset(&env, &admin);
    let (token_b, _) = new_asset(&env, &admin);
    let pool = env.register(aquarius_mock::AqPool, ());
    sac_a.mint(&sender, &1_000);

    let swap_xdr = strategy_xdr(
        &env,
        token_a.clone(),
        token_b.clone(),
        1,
        vec![
            &env,
            one_hop_path(
                &env,
                SwapVenue::Aquarius,
                pool.clone(),
                token_a.clone(),
                token_b.clone(),
                1_000_000,
            ),
            one_hop_path(&env, SwapVenue::Aquarius, pool, token_a, token_b, 0),
        ],
    );
    let err = RouterClient::new(&env, &router_addr)
        .try_execute_strategy(&sender, &100, &swap_xdr)
        .unwrap_err();
    assert_eq!(err.unwrap(), Error::ZeroSplitPpm.into());
}

#[test]
fn two_path_split_consumes_full_total_in_with_rounding_absorbed() {
    let env = Env::default();
    env.mock_all_auths();
    let router_addr = env.register(Router, (Address::generate(&env),));
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (token_a, sac_a) = new_asset(&env, &admin);
    let (token_b, sac_b) = new_asset(&env, &admin);
    let pool = env.register(aquarius_mock::AqPool, ());
    aquarius_mock::AqPoolClient::new(&env, &pool).init(&token_a, &token_b);
    sac_a.mint(&sender, &1_000);
    sac_b.mint(&pool, &1_000);

    let swap_xdr = strategy_xdr(
        &env,
        token_a.clone(),
        token_b.clone(),
        7,
        vec![
            &env,
            one_hop_path(
                &env,
                SwapVenue::Aquarius,
                pool.clone(),
                token_a.clone(),
                token_b.clone(),
                333_333,
            ),
            one_hop_path(
                &env,
                SwapVenue::Aquarius,
                pool,
                token_a,
                token_b.clone(),
                666_667,
            ),
        ],
    );
    let out = RouterClient::new(&env, &router_addr).execute_strategy(&sender, &7, &swap_xdr);
    assert_eq!(out, 7, "last path must absorb PPM rounding");
    assert_eq!(token::Client::new(&env, &token_b).balance(&sender), 7);
}
