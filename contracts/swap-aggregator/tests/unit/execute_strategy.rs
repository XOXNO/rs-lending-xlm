use crate::errors::Error;
use crate::types::{SwapHop, SwapVenue};
use crate::{Router, RouterClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{token, Address, Env};

use super::support::{
    aquarius_mock, malicious_aquarius_mock, new_asset, one_hop_path, strategy_xdr, SwapPath,
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
        alloc::vec![one_hop_path(
            &env,
            SwapVenue::Aquarius,
            pool,
            token_a.clone(),
            token_b.clone(),
            1_000_000,
        ),],
    );

    let out = RouterClient::new(&env, &router_addr).execute_strategy(&sender, &500, &swap_xdr);
    assert_eq!(out, 500);
    assert_eq!(token::Client::new(&env, &token_a).balance(&sender), 500);
    assert_eq!(token::Client::new(&env, &token_b).balance(&sender), 500);
}

#[test]
fn execute_strategy_rejects_fake_venue_output() {
    let env = Env::default();
    env.mock_all_auths();

    let router_addr = env.register(Router, (Address::generate(&env),));
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (token_a, sac_a) = new_asset(&env, &admin);
    let (token_b, sac_b) = new_asset(&env, &admin);

    let pool = env.register(malicious_aquarius_mock::MaliciousAqPool, ());
    malicious_aquarius_mock::MaliciousAqPoolClient::new(&env, &pool)
        .init(&token_a, &token_b, &700u128, &0i128);

    sac_a.mint(&sender, &1);
    sac_b.mint(&router_addr, &700);

    let swap_xdr = strategy_xdr(
        &env,
        token_a.clone(),
        token_b.clone(),
        700,
        alloc::vec![one_hop_path(
            &env,
            SwapVenue::Aquarius,
            pool,
            token_a.clone(),
            token_b.clone(),
            1_000_000,
        ),],
    );

    let err = RouterClient::new(&env, &router_addr)
        .try_execute_strategy(&sender, &1, &swap_xdr)
        .unwrap_err();
    assert_eq!(err.unwrap(), Error::ZeroOutput.into());

    assert_eq!(token::Client::new(&env, &token_b).balance(&sender), 0);
    assert_eq!(
        token::Client::new(&env, &token_b).balance(&router_addr),
        700
    );
}

#[test]
fn execute_strategy_credits_only_delivered_output_not_reported() {
    let env = Env::default();
    env.mock_all_auths();

    let router_addr = env.register(Router, (Address::generate(&env),));
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (token_a, sac_a) = new_asset(&env, &admin);
    let (token_b, sac_b) = new_asset(&env, &admin);

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
        alloc::vec![one_hop_path(
            &env,
            SwapVenue::Aquarius,
            pool,
            token_a.clone(),
            token_b.clone(),
            1_000_000,
        ),],
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
        alloc::vec![one_hop_path(
            &env,
            SwapVenue::Aquarius,
            pool,
            token_a.clone(),
            token_b.clone(),
            1_000_000,
        ),],
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
    let (token_b, sac_b) = new_asset(&env, &admin);
    let pool = env.register(aquarius_mock::AqPool, ());

    sac_a.mint(&sender, &1_000);
    sac_b.mint(&sender, &1_000);

    let (token_c, _) = new_asset(&env, &admin);

    // `total_in` is pulled in token_b, but the route's head consumes token_a,
    // which the vault never holds.
    let swap_xdr = strategy_xdr(
        &env,
        token_b.clone(),
        token_c.clone(),
        1,
        alloc::vec![one_hop_path(
            &env,
            SwapVenue::Aquarius,
            pool,
            token_a.clone(),
            token_c,
            1_000_000,
        ),],
    );

    let err = RouterClient::new(&env, &router_addr)
        .try_execute_strategy(&sender, &500, &swap_xdr)
        .unwrap_err();
    assert_eq!(err.unwrap(), Error::InvalidAmount.into());
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
    let swap_xdr = strategy_xdr(&env, token_a, token_b, 1, alloc::vec::Vec::new());
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
        alloc::vec![one_hop_path(
            &env,
            SwapVenue::Aquarius,
            pool,
            token_a,
            token_b,
            1_000_000
        ),],
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

    // The second hop chains off the first with `Prev`, but names a token the
    // first hop never produced.
    let (token_c, _) = new_asset(&env, &admin);
    let (token_d, _) = new_asset(&env, &admin);
    let hops = alloc::vec![
        SwapHop {
            venue: SwapVenue::Aquarius,
            pool: pool.clone(),
            token_in: token_a.clone(),
            token_out: token_c.clone(),
        },
        SwapHop {
            venue: SwapVenue::Aquarius,
            pool,
            token_in: token_d,
            token_out: token_b.clone(),
        },
    ];
    let swap_xdr = strategy_xdr(
        &env,
        token_a,
        token_b.clone(),
        1,
        alloc::vec![SwapPath {
            split_ppm: 1_000_000,
            hops,
        },],
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
        alloc::vec![one_hop_path(
            &env,
            SwapVenue::Aquarius,
            pool,
            token_a.clone(),
            token_a,
            1_000_000,
        ),],
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
        alloc::vec![one_hop_path(
            &env,
            SwapVenue::Aquarius,
            pool.clone(),
            token_a.clone(),
            token_b.clone(),
            1_000_000,
        ),],
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
        alloc::vec![one_hop_path(
            &env,
            SwapVenue::Aquarius,
            pool,
            token_a,
            token_b,
            1_000_000
        ),],
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
fn a_program_with_no_instructions_is_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let router_addr = env.register(Router, (Address::generate(&env),));
    let client = RouterClient::new(&env, &router_addr);
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (token_a, sac_a) = new_asset(&env, &admin);
    let (token_b, _) = new_asset(&env, &admin);
    sac_a.mint(&sender, &1_000);

    let hopless = alloc::vec![SwapPath {
        split_ppm: 1_000_000,
        hops: alloc::vec::Vec::new(),
    },];
    let xdr = strategy_xdr(&env, token_a, token_b, 1, hopless);
    assert_eq!(
        client
            .try_execute_strategy(&sender, &100, &xdr)
            .unwrap_err()
            .unwrap(),
        Error::EmptyBatch.into()
    );
}

/// Split weights no longer have to be declared to sum to 1e6 up front — the
/// residual guard is what enforces it, by rejecting anything left unrouted.
#[test]
fn an_under_routed_split_leaves_funds_behind_and_reverts() {
    let env = Env::default();
    env.mock_all_auths();
    let router_addr = env.register(Router, (Address::generate(&env),));
    let client = RouterClient::new(&env, &router_addr);
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (token_a, sac_a) = new_asset(&env, &admin);
    let (token_b, sac_b) = new_asset(&env, &admin);
    let pool = env.register(aquarius_mock::AqPool, ());
    aquarius_mock::AqPoolClient::new(&env, &pool).init(&token_a, &token_b);
    sac_a.mint(&sender, &10_000_000_000);
    sac_b.mint(&pool, &10_000_000_000);

    // Only 60% of the input is routed; the remaining 40% is far above the
    // residual allowance for this trade size.
    let partial = alloc::vec![one_hop_path(
        &env,
        SwapVenue::Aquarius,
        pool,
        token_a.clone(),
        token_b.clone(),
        600_000,
    )];
    let xdr = strategy_xdr(&env, token_a, token_b, 1, partial);
    assert_eq!(
        client
            .try_execute_strategy(&sender, &10_000_000_000, &xdr)
            .unwrap_err()
            .unwrap(),
        Error::ExcessiveResidual.into()
    );
}

/// The residual guard's boundary, driven through the real enforcement path:
/// an unrouted remainder of exactly the allowance is accepted, one raw unit
/// more is rejected. An input of 1_000_000 puts the trade in the dust-floor
/// regime (allowance = 1_000 whatever the venue credits), and makes one
/// weight-ppm equal one raw unit, so the boundary is exact.
#[test]
fn a_residual_of_exactly_the_allowance_passes_and_one_unit_more_reverts() {
    let run = |weight_ppm| {
        let env = Env::default();
        env.mock_all_auths();
        let router_addr = env.register(Router, (Address::generate(&env),));
        let client = RouterClient::new(&env, &router_addr);
        let sender = Address::generate(&env);
        let admin = Address::generate(&env);
        let (token_a, sac_a) = new_asset(&env, &admin);
        let (token_b, sac_b) = new_asset(&env, &admin);
        let pool = env.register(aquarius_mock::AqPool, ());
        aquarius_mock::AqPoolClient::new(&env, &pool).init(&token_a, &token_b);
        sac_a.mint(&sender, &1_000_000);
        sac_b.mint(&pool, &1_000_000_000);

        let path = alloc::vec![one_hop_path(
            &env,
            SwapVenue::Aquarius,
            pool,
            token_a.clone(),
            token_b.clone(),
            weight_ppm,
        )];
        let xdr = strategy_xdr(&env, token_a, token_b, 1, path);
        client.try_execute_strategy(&sender, &1_000_000, &xdr)
    };

    assert!(
        run(999_000).is_ok(),
        "an unrouted residual of exactly the allowance (1_000) must pass"
    );
    assert_eq!(
        run(998_999).unwrap_err().unwrap(),
        Error::ExcessiveResidual.into(),
        "one raw unit past the allowance must revert"
    );
}

/// A leg that ends somewhere other than `token_out` strands its output, which
/// the residual guard rejects for any material amount.
#[test]
fn a_path_ending_off_target_reverts() {
    let env = Env::default();
    env.mock_all_auths();
    let router_addr = env.register(Router, (Address::generate(&env),));
    let client = RouterClient::new(&env, &router_addr);
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (token_a, sac_a) = new_asset(&env, &admin);
    let (token_b, sac_b) = new_asset(&env, &admin);
    let (token_c, sac_c) = new_asset(&env, &admin);
    let pool_ab = env.register(aquarius_mock::AqPool, ());
    aquarius_mock::AqPoolClient::new(&env, &pool_ab).init(&token_a, &token_b);
    let pool_ac = env.register(aquarius_mock::AqPool, ());
    aquarius_mock::AqPoolClient::new(&env, &pool_ac).init(&token_a, &token_c);
    sac_a.mint(&sender, &10_000_000_000);
    sac_b.mint(&pool_ab, &10_000_000_000);
    sac_c.mint(&pool_ac, &10_000_000_000);

    let mismatched = alloc::vec![
        one_hop_path(
            &env,
            SwapVenue::Aquarius,
            pool_ab,
            token_a.clone(),
            token_b.clone(),
            500_000,
        ),
        one_hop_path(
            &env,
            SwapVenue::Aquarius,
            pool_ac,
            token_a.clone(),
            token_c,
            500_000,
        ),
    ];
    let xdr = strategy_xdr(&env, token_a, token_b, 1, mismatched);
    assert_eq!(
        client
            .try_execute_strategy(&sender, &10_000_000_000, &xdr)
            .unwrap_err()
            .unwrap(),
        Error::ExcessiveResidual.into()
    );
}
