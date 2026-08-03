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
    sushi_mock::SushiPoolClient::new(&env, &pool).init(&token_a, &token_b);
    sac_b.mint(&sender, &1_000);
    sac_a.mint(&pool, &1_000);

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

#[test]
fn sushi_direction_requires_exact_pair_match() {
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
