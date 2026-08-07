use crate::errors::Error;
use crate::types::SwapVenue;
use crate::{Router, RouterClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{token, vec, Address, Env, Vec};

use super::super::support::{
    aquarius_lp_mock, aquarius_mock, lp_strategy_xdr, new_asset, one_hop_path,
};

fn lp_pool<'a>(
    env: &'a Env,
    admin: &Address,
    reserve: i128,
) -> (
    Address,
    Address,
    (Address, token::StellarAssetClient<'a>),
    (Address, token::StellarAssetClient<'a>),
) {
    lp_pool_seeded(env, admin, reserve, reserve)
}

fn lp_pool_seeded<'a>(
    env: &'a Env,
    admin: &Address,
    reserve_a: i128,
    reserve_b: i128,
) -> (
    Address,
    Address,
    (Address, token::StellarAssetClient<'a>),
    (Address, token::StellarAssetClient<'a>),
) {
    let a = new_asset(env, admin);
    let b = new_asset(env, admin);
    let pool = env.register(aquarius_lp_mock::AqLpPool, ());
    let share = env
        .register_stellar_asset_contract_v2(pool.clone())
        .address();
    aquarius_lp_mock::AqLpPoolClient::new(env, &pool).init(&a.0, &b.0, &share);

    let seeder = Address::generate(env);
    a.1.mint(&seeder, &reserve_a);
    b.1.mint(&seeder, &reserve_b);
    aquarius_lp_mock::AqLpPoolClient::new(env, &pool).deposit(
        &seeder,
        &vec![env, reserve_a as u128, reserve_b as u128],
        &0u128,
    );

    (pool, share, a, b)
}

#[test]
fn mint_lp_from_single_token_routes_half_and_deposits_both() {
    let env = Env::default();
    env.mock_all_auths();

    let router_addr = env.register(Router, (Address::generate(&env),));
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (pool, share, (token_a, sac_a), (token_b, sac_b)) = lp_pool(&env, &admin, 1_000_000);

    let swap_pool = env.register(aquarius_mock::AqPool, ());
    aquarius_mock::AqPoolClient::new(&env, &swap_pool).init(&token_a, &token_b);
    sac_b.mint(&swap_pool, &1_000_000);

    sac_a.mint(&sender, &1_000);

    let xdr = lp_strategy_xdr(
        &env,
        token_a.clone(),
        share.clone(),
        1,
        vec![
            &env,
            one_hop_path(
                &env,
                SwapVenue::Aquarius,
                swap_pool,
                token_a.clone(),
                token_b.clone(),
                500_000,
            ),
        ],
        None,
        Vec::new(&env),
        Some(pool.clone()),
        1,
        0,
        false,
    );

    let shares = RouterClient::new(&env, &router_addr).execute_strategy(&sender, &1_000, &xdr);

    assert_eq!(shares, 500);
    assert_eq!(token::Client::new(&env, &share).balance(&sender), 500);
    assert_eq!(token::Client::new(&env, &token_a).balance(&sender), 0);
    assert_eq!(token::Client::new(&env, &token_b).balance(&sender), 0);

    assert_eq!(token::Client::new(&env, &token_a).balance(&router_addr), 0);
    assert_eq!(token::Client::new(&env, &token_b).balance(&router_addr), 0);
    assert_eq!(token::Client::new(&env, &share).balance(&router_addr), 0);
}

#[test]
fn mint_pre_balances_a_lopsided_input_rather_than_charging_for_it() {
    let env = Env::default();
    env.mock_all_auths();

    let router_addr = env.register(Router, (Address::generate(&env),));
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (pool, share, (token_a, sac_a), (token_b, sac_b)) = lp_pool(&env, &admin, 1_000_000);

    let swap_pool = env.register(aquarius_mock::AqPool, ());
    aquarius_mock::AqPoolClient::new(&env, &swap_pool).init(&token_a, &token_b);
    sac_b.mint(&swap_pool, &1_000_000);
    sac_a.mint(&sender, &1_000);

    let xdr = lp_strategy_xdr(
        &env,
        token_a.clone(),
        share.clone(),
        1,
        vec![
            &env,
            one_hop_path(
                &env,
                SwapVenue::Aquarius,
                swap_pool,
                token_a.clone(),
                token_b.clone(),
                200_000,
            ),
        ],
        None,
        Vec::new(&env),
        Some(pool),
        1,
        300,
        true,
    );

    let shares = RouterClient::new(&env, &router_addr).execute_strategy(&sender, &1_000, &xdr);

    let router = RouterClient::new(&env, &router_addr);

    assert_eq!(shares, 497);
    assert_eq!(token::Client::new(&env, &share).balance(&sender), 497);
    assert_eq!(token::Client::new(&env, &token_a).balance(&sender), 0);
    assert!(
        router.admin_fee_balance(&token_a) <= 1_000 && router.admin_fee_balance(&token_b) <= 1_000,
        "residual must be dust, got {} / {}",
        router.admin_fee_balance(&token_a),
        router.admin_fee_balance(&token_b)
    );
}

#[test]
fn burn_lp_to_single_token_routes_both_constituents() {
    let env = Env::default();
    env.mock_all_auths();

    let router_addr = env.register(Router, (Address::generate(&env),));
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (pool, share, (token_a, _sac_a), (token_b, sac_b)) = lp_pool(&env, &admin, 1_000_000);

    let (_, sac_a2) = (token_a.clone(), _sac_a);
    sac_a2.mint(&sender, &1_000);
    sac_b.mint(&sender, &1_000);
    aquarius_lp_mock::AqLpPoolClient::new(&env, &pool).deposit(
        &sender,
        &vec![&env, 1_000u128, 1_000u128],
        &0u128,
    );
    assert_eq!(token::Client::new(&env, &share).balance(&sender), 1_000);

    let swap_pool = env.register(aquarius_mock::AqPool, ());
    aquarius_mock::AqPoolClient::new(&env, &swap_pool).init(&token_a, &token_b);
    sac_a2.mint(&swap_pool, &1_000_000);

    let xdr = lp_strategy_xdr(
        &env,
        share.clone(),
        token_a.clone(),
        1,
        vec![
            &env,
            one_hop_path(
                &env,
                SwapVenue::Aquarius,
                swap_pool,
                token_b.clone(),
                token_a.clone(),
                1_000_000,
            ),
        ],
        Some(pool),
        vec![&env, 0i128, 0i128],
        None,
        0,
        0,
        false,
    );

    let out = RouterClient::new(&env, &router_addr).execute_strategy(&sender, &1_000, &xdr);

    assert_eq!(out, 2_000);
    assert_eq!(token::Client::new(&env, &token_a).balance(&sender), 2_000);
    assert_eq!(token::Client::new(&env, &share).balance(&sender), 0);
    assert_eq!(token::Client::new(&env, &share).balance(&router_addr), 0);
    assert_eq!(token::Client::new(&env, &token_b).balance(&router_addr), 0);
}

#[test]
fn burn_honours_per_constituent_minimums() {
    let env = Env::default();
    env.mock_all_auths();

    let router_addr = env.register(Router, (Address::generate(&env),));
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (pool, share, (token_a, sac_a), (token_b, sac_b)) = lp_pool(&env, &admin, 1_000_000);

    sac_a.mint(&sender, &1_000);
    sac_b.mint(&sender, &1_000);
    aquarius_lp_mock::AqLpPoolClient::new(&env, &pool).deposit(
        &sender,
        &vec![&env, 1_000u128, 1_000u128],
        &0u128,
    );

    let swap_pool = env.register(aquarius_mock::AqPool, ());
    aquarius_mock::AqPoolClient::new(&env, &swap_pool).init(&token_a, &token_b);
    sac_a.mint(&swap_pool, &1_000_000);

    let xdr = lp_strategy_xdr(
        &env,
        share.clone(),
        token_a.clone(),
        1,
        vec![
            &env,
            one_hop_path(
                &env,
                SwapVenue::Aquarius,
                swap_pool,
                token_b.clone(),
                token_a.clone(),
                1_000_000,
            ),
        ],
        Some(pool),
        vec![&env, 5_000i128, 0i128],
        None,
        0,
        0,
        false,
    );

    assert!(RouterClient::new(&env, &router_addr)
        .try_execute_strategy(&sender, &1_000, &xdr)
        .is_err());
}

#[test]
fn mint_rejects_pool_that_does_not_issue_the_declared_share_token() {
    let env = Env::default();
    env.mock_all_auths();

    let router_addr = env.register(Router, (Address::generate(&env),));
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (pool, _share, (token_a, sac_a), (token_b, sac_b)) = lp_pool(&env, &admin, 1_000_000);

    let (impostor, _) = new_asset(&env, &admin);

    let swap_pool = env.register(aquarius_mock::AqPool, ());
    aquarius_mock::AqPoolClient::new(&env, &swap_pool).init(&token_a, &token_b);
    sac_b.mint(&swap_pool, &1_000_000);
    sac_a.mint(&sender, &1_000);

    let xdr = lp_strategy_xdr(
        &env,
        token_a.clone(),
        impostor,
        1,
        vec![
            &env,
            one_hop_path(
                &env,
                SwapVenue::Aquarius,
                swap_pool,
                token_a.clone(),
                token_b.clone(),
                500_000,
            ),
        ],
        None,
        Vec::new(&env),
        Some(pool),
        1,
        0,
        false,
    );

    assert_eq!(
        RouterClient::new(&env, &router_addr)
            .try_execute_strategy(&sender, &1_000, &xdr)
            .unwrap_err()
            .unwrap(),
        Error::LpTokenMismatch.into()
    );
}

#[test]
fn mint_enforces_min_shares() {
    let env = Env::default();
    env.mock_all_auths();

    let router_addr = env.register(Router, (Address::generate(&env),));
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (pool, share, (token_a, sac_a), (token_b, sac_b)) = lp_pool(&env, &admin, 1_000_000);

    let swap_pool = env.register(aquarius_mock::AqPool, ());
    aquarius_mock::AqPoolClient::new(&env, &swap_pool).init(&token_a, &token_b);
    sac_b.mint(&swap_pool, &1_000_000);
    sac_a.mint(&sender, &1_000);

    let xdr = lp_strategy_xdr(
        &env,
        token_a.clone(),
        share,
        1,
        vec![
            &env,
            one_hop_path(
                &env,
                SwapVenue::Aquarius,
                swap_pool,
                token_a.clone(),
                token_b.clone(),
                500_000,
            ),
        ],
        None,
        Vec::new(&env),
        Some(pool),
        10_000,
        0,
        false,
    );

    assert!(RouterClient::new(&env, &router_addr)
        .try_execute_strategy(&sender, &1_000, &xdr)
        .is_err());
}

#[test]
fn swap_batch_must_still_route_its_whole_input() {
    let env = Env::default();
    env.mock_all_auths();

    let router_addr = env.register(Router, (Address::generate(&env),));
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (token_a, sac_a) = new_asset(&env, &admin);
    let (token_b, sac_b) = new_asset(&env, &admin);

    let swap_pool = env.register(aquarius_mock::AqPool, ());
    aquarius_mock::AqPoolClient::new(&env, &swap_pool).init(&token_a, &token_b);
    sac_b.mint(&swap_pool, &1_000_000);
    sac_a.mint(&sender, &1_000);

    let xdr = lp_strategy_xdr(
        &env,
        token_a.clone(),
        token_b.clone(),
        1,
        vec![
            &env,
            one_hop_path(
                &env,
                SwapVenue::Aquarius,
                swap_pool,
                token_a.clone(),
                token_b.clone(),
                500_000,
            ),
        ],
        None,
        Vec::new(&env),
        None,
        0,
        0,
        false,
    );

    assert_eq!(
        RouterClient::new(&env, &router_addr)
            .try_execute_strategy(&sender, &1_000, &xdr)
            .unwrap_err()
            .unwrap(),
        Error::SplitPpmMismatch.into()
    );
}

#[test]
fn mint_on_an_unbalanced_pool_settles_on_measured_deltas() {
    let env = Env::default();
    env.mock_all_auths();

    let router_addr = env.register(Router, (Address::generate(&env),));
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (pool, share, (token_a, sac_a), (token_b, sac_b)) =
        lp_pool_seeded(&env, &admin, 1_000_000, 1_618_034);

    let seeder = Address::generate(&env);
    sac_a.mint(&seeder, &333_333);
    sac_b.mint(&seeder, &700_000);
    aquarius_lp_mock::AqLpPoolClient::new(&env, &pool).deposit(
        &seeder,
        &vec![&env, 333_333u128, 700_000u128],
        &0u128,
    );

    let swap_pool = env.register(aquarius_mock::AqPool, ());
    aquarius_mock::AqPoolClient::new(&env, &swap_pool).init(&token_a, &token_b);
    sac_b.mint(&swap_pool, &1_000_000);
    sac_a.mint(&sender, &3_000);

    let xdr = lp_strategy_xdr(
        &env,
        token_a.clone(),
        share.clone(),
        1,
        vec![
            &env,
            one_hop_path(
                &env,
                SwapVenue::Aquarius,
                swap_pool,
                token_a.clone(),
                token_b.clone(),
                666_666,
            ),
        ],
        None,
        Vec::new(&env),
        Some(pool),
        1,
        0,
        false,
    );

    let shares = RouterClient::new(&env, &router_addr).execute_strategy(&sender, &3_000, &xdr);

    let router = RouterClient::new(&env, &router_addr);
    assert!(shares > 0);
    assert_eq!(token::Client::new(&env, &share).balance(&sender), shares);

    assert_eq!(
        token::Client::new(&env, &token_a).balance(&router_addr),
        router.admin_fee_balance(&token_a)
    );
    assert_eq!(
        token::Client::new(&env, &token_b).balance(&router_addr),
        router.admin_fee_balance(&token_b)
    );
}

#[test]
fn mint_accepts_a_single_sided_deposit_with_no_paths() {
    let env = Env::default();
    env.mock_all_auths();

    let router_addr = env.register(Router, (Address::generate(&env),));
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (pool, share, (token_a, sac_a), (_token_b, _sac_b)) = lp_pool(&env, &admin, 1_000_000);

    sac_a.mint(&sender, &1_000);

    let xdr = lp_strategy_xdr(
        &env,
        token_a.clone(),
        share.clone(),
        1,
        Vec::new(&env),
        None,
        Vec::new(&env),
        Some(pool),
        1,
        0,
        false,
    );

    let err = RouterClient::new(&env, &router_addr)
        .try_execute_strategy(&sender, &1_000, &xdr)
        .err();
    assert!(
        !matches!(err, Some(Ok(e)) if e == Error::EmptyBatch.into()),
        "single-sided deposit must not be rejected as an empty batch"
    );
}

#[test]
fn mint_pre_balances_even_a_wildly_skewed_input() {
    let env = Env::default();
    env.mock_all_auths();

    let router_addr = env.register(Router, (Address::generate(&env),));
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (pool, share, (token_a, sac_a), (token_b, sac_b)) = lp_pool(&env, &admin, 100_000_000_000);

    let swap_pool = env.register(aquarius_mock::AqPool, ());
    aquarius_mock::AqPoolClient::new(&env, &swap_pool).init(&token_a, &token_b);
    sac_b.mint(&swap_pool, &100_000_000_000);
    sac_a.mint(&sender, &10_000_000_000);

    let xdr = lp_strategy_xdr(
        &env,
        token_a.clone(),
        share,
        1,
        vec![
            &env,
            one_hop_path(
                &env,
                SwapVenue::Aquarius,
                swap_pool,
                token_a.clone(),
                token_b.clone(),
                200_000,
            ),
        ],
        None,
        Vec::new(&env),
        Some(pool),
        1,
        // Off-chain optimum for 8B/2B against 100B/100B @ 30 bps (the LP
        // pool's fee). A nearby wrong guess (e.g. 2_903_518_000) leaves
        // residual above the allowance and reverts with ExcessiveResidual.
        2_903_506_438,
        true,
    );

    let shares =
        RouterClient::new(&env, &router_addr).execute_strategy(&sender, &10_000_000_000, &xdr);
    let router = RouterClient::new(&env, &router_addr);
    assert!(shares > 0);

    let residual = router.admin_fee_balance(&token_a) + router.admin_fee_balance(&token_b);
    assert!(
        residual <= 10_000,
        "expected dust-level residual, got {residual}"
    );
}

#[test]
fn burn_rejects_a_constituent_that_cannot_reach_the_output() {
    let env = Env::default();
    env.mock_all_auths();

    let router_addr = env.register(Router, (Address::generate(&env),));
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (pool, share, (_token_a, sac_a), (token_b, sac_b)) = lp_pool(&env, &admin, 1_000_000);
    let (token_c, sac_c) = new_asset(&env, &admin);

    sac_a.mint(&sender, &100_000);
    sac_b.mint(&sender, &100_000);
    aquarius_lp_mock::AqLpPoolClient::new(&env, &pool).deposit(
        &sender,
        &vec![&env, 100_000u128, 100_000u128],
        &0u128,
    );
    let shares = token::Client::new(&env, &share).balance(&sender);

    let swap_pool = env.register(aquarius_mock::AqPool, ());
    aquarius_mock::AqPoolClient::new(&env, &swap_pool).init(&token_b, &token_c);
    sac_c.mint(&swap_pool, &1_000_000);

    let xdr = lp_strategy_xdr(
        &env,
        share,
        token_c.clone(),
        1,
        vec![
            &env,
            one_hop_path(
                &env,
                SwapVenue::Aquarius,
                swap_pool,
                token_b.clone(),
                token_c.clone(),
                1_000_000,
            ),
        ],
        Some(pool),
        vec![&env, 0i128, 0i128],
        None,
        0,
        0,
        false,
    );

    assert_eq!(
        RouterClient::new(&env, &router_addr)
            .try_execute_strategy(&sender, &shares, &xdr)
            .unwrap_err()
            .unwrap(),
        Error::ExcessiveResidual.into()
    );
}

#[test]
fn stable_pool_is_not_pre_balanced() {
    let env = Env::default();
    env.mock_all_auths();

    let router_addr = env.register(Router, (Address::generate(&env),));
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (pool, share, (token_a, sac_a), (_token_b, _sac_b)) = lp_pool(&env, &admin, 1_000_000);
    aquarius_lp_mock::AqLpPoolClient::new(&env, &pool).set_stable();

    sac_a.mint(&sender, &1_000);
    let pool_a_before = token::Client::new(&env, &token_a).balance(&pool);

    let xdr = lp_strategy_xdr(
        &env,
        token_a.clone(),
        share.clone(),
        1,
        Vec::new(&env),
        None,
        Vec::new(&env),
        Some(pool.clone()),
        1,
        0,
        false,
    );

    let shares = RouterClient::new(&env, &router_addr).execute_strategy(&sender, &1_000, &xdr);

    assert_eq!(
        shares, 500,
        "single-sided stable deposit must not be pre-swapped"
    );
    assert_eq!(
        token::Client::new(&env, &token_a).balance(&pool) - pool_a_before,
        1_000
    );
    assert_eq!(token::Client::new(&env, &share).balance(&sender), shares);
}

#[test]
fn mint_min_shares_boundary_is_inclusive() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let build = |min_shares: i128| {
        let router_addr = env.register(Router, (Address::generate(&env),));
        let sender = Address::generate(&env);
        let (pool, share, (token_a, sac_a), (_tb, _sb)) = lp_pool(&env, &admin, 1_000_000);
        aquarius_lp_mock::AqLpPoolClient::new(&env, &pool).set_stable();
        sac_a.mint(&sender, &1_000);
        let xdr = lp_strategy_xdr(
            &env,
            token_a,
            share,
            1,
            Vec::new(&env),
            None,
            Vec::new(&env),
            Some(pool),
            min_shares,
            0,
            false,
        );
        (router_addr, sender, xdr)
    };

    let (router_addr, sender, xdr) = build(1);
    let achievable = RouterClient::new(&env, &router_addr).execute_strategy(&sender, &1_000, &xdr);

    let (router_addr, sender, xdr) = build(achievable);
    assert_eq!(
        RouterClient::new(&env, &router_addr).execute_strategy(&sender, &1_000, &xdr),
        achievable,
        "exactly meeting min_shares must succeed"
    );

    let (router_addr, sender, xdr) = build(achievable + 1);
    assert!(
        RouterClient::new(&env, &router_addr)
            .try_execute_strategy(&sender, &1_000, &xdr)
            .is_err(),
        "asking above what the deposit can mint must fail"
    );
}

#[test]
fn burn_min_amounts_boundary_is_inclusive() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);

    let build = |min: i128| {
        let router_addr = env.register(Router, (Address::generate(&env),));
        let sender = Address::generate(&env);
        let (pool, share, (token_a, sac_a), (token_b, sac_b)) = lp_pool(&env, &admin, 1_000_000);
        sac_a.mint(&sender, &1_000);
        sac_b.mint(&sender, &1_000);
        aquarius_lp_mock::AqLpPoolClient::new(&env, &pool).deposit(
            &sender,
            &vec![&env, 1_000u128, 1_000u128],
            &0u128,
        );
        let shares = token::Client::new(&env, &share).balance(&sender);

        let swap_pool = env.register(aquarius_mock::AqPool, ());
        aquarius_mock::AqPoolClient::new(&env, &swap_pool).init(&token_a, &token_b);
        sac_a.mint(&swap_pool, &1_000_000);

        let xdr = lp_strategy_xdr(
            &env,
            share,
            token_a.clone(),
            1,
            vec![
                &env,
                one_hop_path(
                    &env,
                    SwapVenue::Aquarius,
                    swap_pool,
                    token_b.clone(),
                    token_a,
                    1_000_000,
                ),
            ],
            Some(pool),
            vec![&env, min, min],
            None,
            0,
            0,
            false,
        );
        (router_addr, sender, shares, xdr)
    };

    let (router_addr, sender, shares, xdr) = build(1_000);
    assert!(
        RouterClient::new(&env, &router_addr)
            .try_execute_strategy(&sender, &shares, &xdr)
            .is_ok(),
        "a minimum met exactly must be accepted"
    );

    let (router_addr, sender, shares, xdr) = build(1_001);
    assert!(
        RouterClient::new(&env, &router_addr)
            .try_execute_strategy(&sender, &shares, &xdr)
            .is_err(),
        "a minimum above what the burn releases must fail"
    );
}

#[test]
fn temp_budget_probe_mainnet_scale() {
    let env = Env::default();
    env.mock_all_auths();
    env.cost_estimate().budget().reset_unlimited();

    let router_addr = env.register(Router, (Address::generate(&env),));
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (pool, share, (token_a, sac_a), (token_b, _sac_b)) =
        lp_pool_seeded(&env, &admin, 3_137_000_000_000, 574_000_000_000);

    sac_a.mint(&sender, &100_000_000);

    let xdr = lp_strategy_xdr(
        &env,
        token_a.clone(),
        share.clone(),
        1,
        vec![
            &env,
            one_hop_path(
                &env,
                SwapVenue::Aquarius,
                pool.clone(),
                token_a.clone(),
                token_b.clone(),
                500_755,
            ),
        ],
        None,
        Vec::new(&env),
        Some(pool.clone()),
        1,
        0,
        false,
    );

    env.cost_estimate().budget().reset_unlimited();
    let shares =
        RouterClient::new(&env, &router_addr).execute_strategy(&sender, &100_000_000, &xdr);
    let cpu = env.cost_estimate().budget().cpu_instruction_cost();
    assert!(shares > 0);
    assert!(
        cpu < 100_000_000,
        "CPU bomb reproduced: {} instructions",
        cpu
    );
}

/// A burn whose constituents both need routing must execute both paths.
///
/// `execute_paths` groups paths by `token_in` and processes each group once,
/// skipping any path whose index is not the group's first
/// (`i != first_index_for_token(..)`). Every other burn test routes a single
/// path, because one constituent is already the output token -- so only one
/// distinct `token_in` exists and `first_index_for_token` can only ever return
/// 0. Burning into a third token forces two groups.
///
/// Break this catches: `first_index_for_token` returning a constant 0 (the
/// surviving mutant in `.cargo/mutants.toml`). The second group's index would
/// never equal 0, so that path would be skipped, its constituent stranded in
/// the vault, and only half the position converted.
#[test]
fn burn_routes_every_constituent_through_its_own_path() {
    let env = Env::default();
    env.mock_all_auths();

    let router_addr = env.register(Router, (Address::generate(&env),));
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (pool, share, (token_a, sac_a), (token_b, sac_b)) = lp_pool(&env, &admin, 1_000_000);
    let (token_c, sac_c) = new_asset(&env, &admin);

    sac_a.mint(&sender, &1_000);
    sac_b.mint(&sender, &1_000);
    aquarius_lp_mock::AqLpPoolClient::new(&env, &pool).deposit(
        &sender,
        &vec![&env, 1_000u128, 1_000u128],
        &0u128,
    );

    // Neither constituent is the output token, so both need their own hop.
    let pool_ac = env.register(aquarius_mock::AqPool, ());
    aquarius_mock::AqPoolClient::new(&env, &pool_ac).init(&token_a, &token_c);
    sac_c.mint(&pool_ac, &1_000_000);
    let pool_bc = env.register(aquarius_mock::AqPool, ());
    aquarius_mock::AqPoolClient::new(&env, &pool_bc).init(&token_b, &token_c);
    sac_c.mint(&pool_bc, &1_000_000);

    let xdr = lp_strategy_xdr(
        &env,
        share.clone(),
        token_c.clone(),
        1,
        vec![
            &env,
            one_hop_path(
                &env,
                SwapVenue::Aquarius,
                pool_ac,
                token_a.clone(),
                token_c.clone(),
                1_000_000,
            ),
            one_hop_path(
                &env,
                SwapVenue::Aquarius,
                pool_bc,
                token_b.clone(),
                token_c.clone(),
                1_000_000,
            ),
        ],
        Some(pool),
        vec![&env, 0i128, 0i128],
        None,
        0,
        0,
        false,
    );

    let out = RouterClient::new(&env, &router_addr).execute_strategy(&sender, &1_000, &xdr);

    // 1000 of each constituent, each swapped 1:1 -- not 1000 from one leg alone.
    assert_eq!(out, 2_000);
    assert_eq!(token::Client::new(&env, &token_c).balance(&sender), 2_000);
    assert_eq!(token::Client::new(&env, &token_a).balance(&router_addr), 0);
    assert_eq!(token::Client::new(&env, &token_b).balance(&router_addr), 0);
}

/// A constituent left behind at exactly the residual allowance is swept, not
/// rejected.
///
/// `accrue_residual_as_revenue` reverts with `ExcessiveResidual` when a leftover
/// vault balance is strictly above `residual_allowance(credited)`.
/// `burn_rejects_a_constituent_that_cannot_reach_the_output` covers the
/// comfortably-over case (100_000 against a 1_000 floor); nothing pinned the
/// boundary itself, and the whole residual path had no other coverage.
///
/// Here token_b is deliberately unrouted and its 1_000 leftover sits exactly on
/// RESIDUAL_DUST_FLOOR, so it must be accepted and accrued to admin fees.
///
/// Break this catches: that `>` becoming `>=` (the surviving mutant in
/// `.cargo/mutants.toml`), which would reject a burn whose dust lands precisely
/// on the allowance.
#[test]
fn residual_exactly_at_the_allowance_is_accrued_not_rejected() {
    let env = Env::default();
    env.mock_all_auths();

    let router_addr = env.register(Router, (Address::generate(&env),));
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (pool, share, (token_a, sac_a), (token_b, sac_b)) = lp_pool(&env, &admin, 1_000_000);
    let (token_c, sac_c) = new_asset(&env, &admin);

    sac_a.mint(&sender, &1_000);
    sac_b.mint(&sender, &1_000);
    aquarius_lp_mock::AqLpPoolClient::new(&env, &pool).deposit(
        &sender,
        &vec![&env, 1_000u128, 1_000u128],
        &0u128,
    );

    // Only token_a is routed; token_b's 1_000 becomes residual.
    let pool_ac = env.register(aquarius_mock::AqPool, ());
    aquarius_mock::AqPoolClient::new(&env, &pool_ac).init(&token_a, &token_c);
    sac_c.mint(&pool_ac, &1_000_000);

    let xdr = lp_strategy_xdr(
        &env,
        share.clone(),
        token_c.clone(),
        1,
        vec![
            &env,
            one_hop_path(
                &env,
                SwapVenue::Aquarius,
                pool_ac,
                token_a.clone(),
                token_c.clone(),
                1_000_000,
            ),
        ],
        Some(pool),
        vec![&env, 0i128, 0i128],
        None,
        0,
        0,
        false,
    );

    let router = RouterClient::new(&env, &router_addr);
    let out = router.execute_strategy(&sender, &1_000, &xdr);

    assert_eq!(out, 1_000);
    assert_eq!(token::Client::new(&env, &token_c).balance(&sender), 1_000);
    // The stranded constituent is revenue, not a revert.
    assert_eq!(router.admin_fee_balance(&token_b), 1_000);
    assert_eq!(token::Client::new(&env, &token_b).balance(&sender), 0);
}
