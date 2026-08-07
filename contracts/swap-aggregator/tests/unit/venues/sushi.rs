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

/// A second hop into a token the router already holds must credit only the
/// delta *that* hop produced.
///
/// `dispatch_hop` measures every venue's output as
/// `output_balance() - before_out` and discards whatever the venue function
/// returns. Every other Sushi test routes one 100% path, so `before_out` is
/// always zero and the subtraction is indistinguishable from an addition.
/// Splitting across two Sushi pools makes the second hop start with the first
/// hop's proceeds already sitting in the router.
///
/// Break this catches: `checked_sub` becoming `checked_add` in
/// `venues::dispatch_hop`. The second hop would credit 450 instead of 150 and
/// the vault would try to pay out 600 against a 300 balance. Verified: this is
/// the only Sushi test that fails under that mutation.
///
/// Note the venue-level `amount_out` in `venues::sushi::swap` is dead -- its
/// value is dropped by the dispatcher -- so the `- -> +` mutant recorded for
/// that line in `.cargo/mutants.toml` is a genuine equivalent mutant.
#[test]
fn sushi_split_credits_only_the_delta_of_each_hop() {
    let env = Env::default();
    env.mock_all_auths();

    let router_addr = env.register(Router, (Address::generate(&env),));
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (token_a, sac_a) = new_asset(&env, &admin);
    let (token_b, sac_b) = new_asset(&env, &admin);

    let pool_one = env.register(sushi_mock::SushiPool, ());
    sushi_mock::SushiPoolClient::new(&env, &pool_one).init(&token_a, &token_b);
    let pool_two = env.register(sushi_mock::SushiPool, ());
    sushi_mock::SushiPoolClient::new(&env, &pool_two).init(&token_a, &token_b);

    sac_a.mint(&sender, &1_000);
    sac_b.mint(&pool_one, &1_000);
    sac_b.mint(&pool_two, &1_000);

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
                pool_one,
                token_a.clone(),
                token_b.clone(),
                500_000,
            ),
            one_hop_path(
                &env,
                SwapVenue::Sushi,
                pool_two,
                token_a.clone(),
                token_b.clone(),
                500_000,
            ),
        ],
    );

    let out = RouterClient::new(&env, &router_addr).execute_strategy(&sender, &300, &swap_xdr);

    // 150 per leg at the mock's 1:1 rate -- not 150 + (300 + 150).
    assert_eq!(out, 300);
    assert_eq!(token::Client::new(&env, &token_a).balance(&sender), 700);
    assert_eq!(token::Client::new(&env, &token_b).balance(&sender), 300);
    assert_eq!(token::Client::new(&env, &token_b).balance(&router_addr), 0);
}
