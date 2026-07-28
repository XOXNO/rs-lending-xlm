//! `execute_strategy` surface: decode, validation, and measurement guards.
//!
//! These cases pin fail-closed behaviour that is independent of any single
//! venue: empty/broken payloads, aggregate slippage, same-token routes, and
//! balance-delta accounting when a pool lies about its output.

use crate::errors::Error;
use crate::types::{SwapHop, SwapPath, SwapVenue};
use crate::{Router, RouterClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{token, vec, Address, Env};

use super::support::{
    aquarius_mock, malicious_aquarius_mock, new_asset, one_hop_path, strategy_xdr,
};

#[test]
fn execute_strategy_route_bytes_decode_and_execute() {
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

// A route pool that reports output it never delivered must not let the caller
// drain the router's own `token_out` balance (e.g. accrued fees). The per-hop
// balance-delta check credits zero and reverts.
#[test]
fn execute_strategy_rejects_fake_venue_output() {
    let env = Env::default();
    env.mock_all_auths();

    let router_addr = env.register(Router, (Address::generate(&env),));
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (token_a, sac_a) = new_asset(&env, &admin);
    let (token_b, sac_b) = new_asset(&env, &admin);

    // Pool claims 700 out but transfers nothing.
    let pool = env.register(malicious_aquarius_mock::MaliciousAqPool, ());
    malicious_aquarius_mock::MaliciousAqPoolClient::new(&env, &pool)
        .init(&token_a, &token_b, &700u128, &0i128);

    // Attacker holds 1 token_a; the router holds 700 token_b of accrued fees.
    sac_a.mint(&sender, &1);
    sac_b.mint(&router_addr, &700);

    let swap_xdr = strategy_xdr(
        &env,
        token_a.clone(),
        token_b.clone(),
        700,
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

    let err = RouterClient::new(&env, &router_addr)
        .try_execute_strategy(&sender, &1, &swap_xdr)
        .unwrap_err();
    assert_eq!(err.unwrap(), Error::ZeroOutput.into());
    // Router fees untouched, attacker gained nothing.
    assert_eq!(token::Client::new(&env, &token_b).balance(&sender), 0);
    assert_eq!(
        token::Client::new(&env, &token_b).balance(&router_addr),
        700
    );
}

// When a pool over-reports, the router credits only what actually arrived.
#[test]
fn execute_strategy_credits_only_delivered_output_not_reported() {
    let env = Env::default();
    env.mock_all_auths();

    let router_addr = env.register(Router, (Address::generate(&env),));
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (token_a, sac_a) = new_asset(&env, &admin);
    let (token_b, sac_b) = new_asset(&env, &admin);

    // Pool reports 700 but only delivers 500.
    let pool = env.register(malicious_aquarius_mock::MaliciousAqPool, ());
    malicious_aquarius_mock::MaliciousAqPoolClient::new(&env, &pool)
        .init_with_pull(&token_a, &token_b, &700u128, &500i128, &true);
    sac_b.mint(&pool, &500);
    sac_a.mint(&sender, &1);

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

    let out = RouterClient::new(&env, &router_addr).execute_strategy(&sender, &1, &swap_xdr);
    assert_eq!(out, 500);
    assert_eq!(token::Client::new(&env, &token_a).balance(&router_addr), 0);
    assert_eq!(token::Client::new(&env, &token_b).balance(&sender), 500);
    assert_eq!(token::Client::new(&env, &token_b).balance(&router_addr), 0);
}

#[test]
fn execute_strategy_rejects_output_without_input_spend() {
    let env = Env::default();
    env.mock_all_auths();

    let router_addr = env.register(Router, (Address::generate(&env),));
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (token_a, sac_a) = new_asset(&env, &admin);
    let (token_b, sac_b) = new_asset(&env, &admin);
    let pool = env.register(malicious_aquarius_mock::MaliciousAqPool, ());
    malicious_aquarius_mock::MaliciousAqPoolClient::new(&env, &pool)
        .init(&token_a, &token_b, &500u128, &500i128);

    sac_a.mint(&sender, &1);
    sac_b.mint(&pool, &500);

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

    let err = RouterClient::new(&env, &router_addr)
        .try_execute_strategy(&sender, &1, &swap_xdr)
        .unwrap_err();
    assert_eq!(err.unwrap(), Error::InvalidAmount.into());
    assert_eq!(token::Client::new(&env, &token_a).balance(&router_addr), 0);
    assert_eq!(token::Client::new(&env, &token_b).balance(&sender), 0);
}

#[test]
fn execute_strategy_rejects_wrong_token_in_endpoint() {
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
        token_b.clone(),
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

    let err = RouterClient::new(&env, &router_addr)
        .try_execute_strategy(&sender, &500, &swap_xdr)
        .unwrap_err();
    assert_eq!(err.unwrap(), Error::BrokenTokenChain.into());
    assert_eq!(token::Client::new(&env, &token_a).balance(&sender), 1_000);
}

#[test]
fn execute_strategy_errors_on_empty_payload() {
    let env = Env::default();
    env.mock_all_auths();
    let router_addr = env.register(Router, (Address::generate(&env),));
    let sender = Address::generate(&env);
    let token_a = Address::generate(&env);
    let token_b = Address::generate(&env);
    let swap_xdr = strategy_xdr(&env, token_a, token_b, 1, vec![&env]);
    let err = RouterClient::new(&env, &router_addr)
        .try_execute_strategy(&sender, &1, &swap_xdr)
        .unwrap_err();
    assert_eq!(err.unwrap(), Error::EmptyBatch.into());
}

#[test]
fn execute_strategy_errors_on_aggregate_slippage() {
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
        1_000,
        vec![
            &env,
            one_hop_path(&env, SwapVenue::Aquarius, pool, token_a, token_b, 1_000_000),
        ],
    );
    let err = RouterClient::new(&env, &router_addr)
        .try_execute_strategy(&sender, &100, &swap_xdr)
        .unwrap_err();
    assert_eq!(err.unwrap(), Error::SlippageExceeded.into());
}

#[test]
fn execute_strategy_errors_on_broken_token_chain() {
    let env = Env::default();
    env.mock_all_auths();
    let router_addr = env.register(Router, (Address::generate(&env),));
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (token_a, sac_a) = new_asset(&env, &admin);
    let (token_b, _) = new_asset(&env, &admin);
    let pool = env.register(aquarius_mock::AqPool, ());
    sac_a.mint(&sender, &1_000);

    let hops = vec![
        &env,
        SwapHop {
            venue: SwapVenue::Aquarius,
            amount_out: 0,
            pool: pool.clone(),
            token_in: token_a.clone(),
            token_out: token_a.clone(),
        },
        SwapHop {
            venue: SwapVenue::Aquarius,
            amount_out: 0,
            pool,
            token_in: token_b.clone(),
            token_out: token_b.clone(),
        },
    ];
    let swap_xdr = strategy_xdr(
        &env,
        token_a,
        token_b.clone(),
        1,
        vec![
            &env,
            SwapPath {
                split_ppm: 1_000_000,
                hops,
            },
        ],
    );
    let err = RouterClient::new(&env, &router_addr)
        .try_execute_strategy(&sender, &100, &swap_xdr)
        .unwrap_err();
    assert_eq!(err.unwrap(), Error::BrokenTokenChain.into());
}

#[test]
fn execute_strategy_rejects_same_token_in_and_out() {
    let env = Env::default();
    env.mock_all_auths();
    let router_addr = env.register(Router, (Address::generate(&env),));
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (token_a, sac_a) = new_asset(&env, &admin);
    let pool = env.register(aquarius_mock::AqPool, ());
    sac_a.mint(&sender, &1_000);

    let swap_xdr = strategy_xdr(
        &env,
        token_a.clone(),
        token_a.clone(),
        1,
        vec![
            &env,
            one_hop_path(
                &env,
                SwapVenue::Aquarius,
                pool,
                token_a.clone(),
                token_a,
                1_000_000,
            ),
        ],
    );
    let err = RouterClient::new(&env, &router_addr)
        .try_execute_strategy(&sender, &100, &swap_xdr)
        .unwrap_err();
    assert_eq!(err.unwrap(), Error::SameToken.into());
}

#[test]
fn execute_strategy_rejects_nonpositive_amounts() {
    let env = Env::default();
    env.mock_all_auths();
    let router_addr = env.register(Router, (Address::generate(&env),));
    let client = RouterClient::new(&env, &router_addr);
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (token_a, sac_a) = new_asset(&env, &admin);
    let (token_b, _) = new_asset(&env, &admin);
    let pool = env.register(aquarius_mock::AqPool, ());
    sac_a.mint(&sender, &1_000);
    let xdr0 = strategy_xdr(
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
        ],
    );
    assert_eq!(
        client
            .try_execute_strategy(&sender, &0, &xdr0)
            .unwrap_err()
            .unwrap(),
        Error::InvalidAmount.into()
    );
    let xdr1 = strategy_xdr(
        &env,
        token_a.clone(),
        token_b.clone(),
        0,
        vec![
            &env,
            one_hop_path(&env, SwapVenue::Aquarius, pool, token_a, token_b, 1_000_000),
        ],
    );
    assert_eq!(
        client
            .try_execute_strategy(&sender, &100, &xdr1)
            .unwrap_err()
            .unwrap(),
        Error::SlippageExceeded.into()
    );
}

#[test]
fn validate_batch_shape_empty_and_endpoint_errors() {
    let env = Env::default();
    env.mock_all_auths();
    let router_addr = env.register(Router, (Address::generate(&env),));
    let client = RouterClient::new(&env, &router_addr);
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (token_a, sac_a) = new_asset(&env, &admin);
    let (token_b, _) = new_asset(&env, &admin);
    let (token_c, _) = new_asset(&env, &admin);
    let pool = env.register(aquarius_mock::AqPool, ());
    sac_a.mint(&sender, &1_000);

    let empty_first = vec![
        &env,
        SwapPath {
            split_ppm: 1_000_000,
            hops: vec![&env],
        },
    ];
    let xdr = strategy_xdr(&env, token_a.clone(), token_b.clone(), 1, empty_first);
    assert_eq!(
        client
            .try_execute_strategy(&sender, &100, &xdr)
            .unwrap_err()
            .unwrap(),
        Error::EmptyPath.into()
    );

    let second_empty = vec![
        &env,
        one_hop_path(
            &env,
            SwapVenue::Aquarius,
            pool.clone(),
            token_a.clone(),
            token_b.clone(),
            500_000,
        ),
        SwapPath {
            split_ppm: 500_000,
            hops: vec![&env],
        },
    ];
    let xdr = strategy_xdr(&env, token_a.clone(), token_b.clone(), 1, second_empty);
    assert_eq!(
        client
            .try_execute_strategy(&sender, &100, &xdr)
            .unwrap_err()
            .unwrap(),
        Error::EmptyPath.into()
    );

    let mismatched = vec![
        &env,
        one_hop_path(
            &env,
            SwapVenue::Aquarius,
            pool.clone(),
            token_a.clone(),
            token_b.clone(),
            500_000,
        ),
        one_hop_path(
            &env,
            SwapVenue::Aquarius,
            pool,
            token_a.clone(),
            token_c.clone(),
            500_000,
        ),
    ];
    let xdr = strategy_xdr(&env, token_a.clone(), token_b.clone(), 1, mismatched);
    assert_eq!(
        client
            .try_execute_strategy(&sender, &100, &xdr)
            .unwrap_err()
            .unwrap(),
        Error::BrokenTokenChain.into()
    );
}
