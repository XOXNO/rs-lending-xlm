//! Aquarius venue adapter tests.
//!
//! Covers the 1:1 happy path, pool membership checks, and zero-report rejection
//! (including the malicious pool that lies about delivered output).

use crate::errors::Error;
use crate::types::SwapVenue;
use crate::{Router, RouterClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{token, vec, Address, Env};

use super::super::support::{
    aquarius_mock, malicious_aquarius_mock, new_asset, one_hop_path, strategy_xdr,
};

#[test]
fn aquarius_single_hop_happy_path() {
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
        500,
        vec![
            &env,
            one_hop_path(
                &env,
                SwapVenue::Aquarius,
                pool,
                token_a.clone(),
                token_b.clone(),
                1_000_000,
            ),
        ],
    );

    let out = RouterClient::new(&env, &router_addr).execute_strategy(&sender, &500, &swap_xdr);
    assert_eq!(out, 500);
    assert_eq!(token::Client::new(&env, &token_a).balance(&sender), 500);
    assert_eq!(token::Client::new(&env, &token_b).balance(&sender), 500);
}

#[test]
fn aquarius_token_not_in_pool_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let router_addr = env.register(Router, (Address::generate(&env),));
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (token_a, _) = new_asset(&env, &admin);
    let (token_b, _) = new_asset(&env, &admin);
    let (token_c, sac_c) = new_asset(&env, &admin);
    let pool = env.register(aquarius_mock::AqPool, ());
    aquarius_mock::AqPoolClient::new(&env, &pool).init(&token_a, &token_b);
    sac_c.mint(&sender, &1_000);
    let xdr = strategy_xdr(
        &env,
        token_c.clone(),
        token_b.clone(),
        1,
        vec![
            &env,
            one_hop_path(
                &env,
                SwapVenue::Aquarius,
                pool,
                token_c.clone(),
                token_b.clone(),
                1_000_000,
            ),
        ],
    );
    assert_eq!(
        RouterClient::new(&env, &router_addr)
            .try_execute_strategy(&sender, &100, &xdr)
            .unwrap_err()
            .unwrap(),
        Error::BrokenTokenChain.into()
    );
}

#[test]
fn aquarius_zero_report_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let router_addr = env.register(Router, (Address::generate(&env),));
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (token_a, sac_a) = new_asset(&env, &admin);
    let (token_b, _) = new_asset(&env, &admin);
    let pool = env.register(malicious_aquarius_mock::MaliciousAqPool, ());
    malicious_aquarius_mock::MaliciousAqPoolClient::new(&env, &pool)
        .init(&token_a, &token_b, &0u128, &0i128);
    sac_a.mint(&sender, &1);
    let xdr = strategy_xdr(
        &env,
        token_a.clone(),
        token_b.clone(),
        1,
        vec![
            &env,
            one_hop_path(
                &env,
                SwapVenue::Aquarius,
                pool,
                token_a.clone(),
                token_b.clone(),
                1_000_000,
            ),
        ],
    );
    assert_eq!(
        RouterClient::new(&env, &router_addr)
            .try_execute_strategy(&sender, &1, &xdr)
            .unwrap_err()
            .unwrap(),
        Error::ZeroOutput.into()
    );
}
