//! Sushi venue adapter tests.
//!
//! Direction comes from an exact `(token0, token1)` pair match. A half-match
//! (only one side of the hop is in the pool) must fail closed as a broken chain.

use crate::errors::Error;
use crate::types::SwapVenue;
use crate::{Router, RouterClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{token, vec, Address, Env};

use super::super::support::{new_asset, one_hop_path, strategy_xdr, sushi_mock};

#[test]
fn sushi_single_hop_happy_path() {
    let env = Env::default();
    env.mock_all_auths();

    let router_addr = env.register(Router, (Address::generate(&env),));
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (token_a, sac_a) = new_asset(&env, &admin);
    let (token_b, sac_b) = new_asset(&env, &admin);
    let pool = env.register(sushi_mock::SushiPool, ());
    sushi_mock::SushiPoolClient::new(&env, &pool).init(&token_a, &token_b);

    sac_a.mint(&sender, &1_000);
    sac_b.mint(&pool, &1_000);

    let swap_xdr = strategy_xdr(
        &env,
        token_a.clone(),
        token_b.clone(),
        300,
        vec![
            &env,
            one_hop_path(
                &env,
                SwapVenue::Sushi,
                pool,
                token_a.clone(),
                token_b.clone(),
                1_000_000,
            ),
        ],
    );

    let out = RouterClient::new(&env, &router_addr).execute_strategy(&sender, &300, &swap_xdr);
    assert_eq!(out, 300);
    assert_eq!(token::Client::new(&env, &token_a).balance(&sender), 700);
    assert_eq!(token::Client::new(&env, &token_b).balance(&sender), 300);
}

#[test]
fn sushi_reverse_direction() {
    let env = Env::default();
    env.mock_all_auths();
    let router_addr = env.register(Router, (Address::generate(&env),));
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (token_a, sac_a) = new_asset(&env, &admin);
    let (token_b, sac_b) = new_asset(&env, &admin);
    let pool = env.register(sushi_mock::SushiPool, ());
    sushi_mock::SushiPoolClient::new(&env, &pool).init(&token_a, &token_b); // token0=a, token1=b
    sac_b.mint(&sender, &1_000);
    sac_a.mint(&pool, &1_000);
    // swap b (token1) -> a (token0): zero_for_one == false
    let xdr = strategy_xdr(
        &env,
        token_b.clone(),
        token_a.clone(),
        300,
        vec![
            &env,
            one_hop_path(
                &env,
                SwapVenue::Sushi,
                pool,
                token_b.clone(),
                token_a.clone(),
                1_000_000,
            ),
        ],
    );
    let out = RouterClient::new(&env, &router_addr).execute_strategy(&sender, &300, &xdr);
    assert_eq!(out, 300);
    assert_eq!(token::Client::new(&env, &token_b).balance(&sender), 700);
    assert_eq!(token::Client::new(&env, &token_a).balance(&sender), 300);
}

// A hop whose tokens only half-match the pool's pair (one side matches, the
// other is a third token) must be rejected as a broken chain, in both
// orientations.
#[test]
fn sushi_direction_requires_exact_pair_match() {
    // token_in matches token0 but token_out is a third token.
    {
        let env = Env::default();
        env.mock_all_auths();
        let router_addr = env.register(Router, (Address::generate(&env),));
        let sender = Address::generate(&env);
        let admin = Address::generate(&env);
        let (token_a, sac_a) = new_asset(&env, &admin);
        let (token_b, sac_b) = new_asset(&env, &admin);
        let (token_c, _) = new_asset(&env, &admin);
        let pool = env.register(sushi_mock::SushiPool, ());
        sushi_mock::SushiPoolClient::new(&env, &pool).init(&token_a, &token_b);
        sac_a.mint(&sender, &500);
        sac_b.mint(&pool, &500);

        let xdr = strategy_xdr(
            &env,
            token_a.clone(),
            token_c.clone(),
            1,
            vec![
                &env,
                one_hop_path(&env, SwapVenue::Sushi, pool, token_a, token_c, 1_000_000),
            ],
        );
        assert_eq!(
            RouterClient::new(&env, &router_addr)
                .try_execute_strategy(&sender, &500, &xdr)
                .unwrap_err()
                .unwrap(),
            Error::BrokenTokenChain.into()
        );
    }
    // token_out matches token0 but token_in is a third token.
    {
        let env = Env::default();
        env.mock_all_auths();
        let router_addr = env.register(Router, (Address::generate(&env),));
        let sender = Address::generate(&env);
        let admin = Address::generate(&env);
        let (token_a, sac_a) = new_asset(&env, &admin);
        let (token_b, sac_b) = new_asset(&env, &admin);
        let (token_c, sac_c) = new_asset(&env, &admin);
        let pool = env.register(sushi_mock::SushiPool, ());
        sushi_mock::SushiPoolClient::new(&env, &pool).init(&token_a, &token_b);
        sac_c.mint(&sender, &500);
        sac_a.mint(&pool, &500);
        sac_b.mint(&pool, &500);

        let xdr = strategy_xdr(
            &env,
            token_c.clone(),
            token_a.clone(),
            1,
            vec![
                &env,
                one_hop_path(&env, SwapVenue::Sushi, pool, token_c, token_a, 1_000_000),
            ],
        );
        assert_eq!(
            RouterClient::new(&env, &router_addr)
                .try_execute_strategy(&sender, &500, &xdr)
                .unwrap_err()
                .unwrap(),
            Error::BrokenTokenChain.into()
        );
    }
}
