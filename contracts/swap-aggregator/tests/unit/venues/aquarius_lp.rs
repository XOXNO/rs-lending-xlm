use crate::errors::Error;
use crate::types::SwapVenue;
use crate::{Router, RouterClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{token, vec, Address, Env, Vec};

use super::super::support::{
    aquarius_lp_mock, aquarius_mock, lp_strategy_xdr, new_asset, one_hop_path,
};

/// Registers an LP pool seeded with `reserve` of each token, plus its share
/// token. Returns `(pool, share, token_a, token_b)`.
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

/// Same, but with an unbalanced seed so total shares land off the reserve
/// values — the state where deposit rounding actually splits.
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

    // Seed the pool by depositing as a bootstrap provider.
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

    // A 1:1 swap venue so half the input becomes token_b.
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
    );

    let shares = RouterClient::new(&env, &router_addr).execute_strategy(&sender, &1_000, &xdr);

    // 500 of each side deposited into a balanced 1:1 pool.
    assert_eq!(shares, 500);
    assert_eq!(token::Client::new(&env, &share).balance(&sender), 500);
    assert_eq!(token::Client::new(&env, &token_a).balance(&sender), 0);
    assert_eq!(token::Client::new(&env, &token_b).balance(&sender), 0);
    // Router keeps nothing.
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

    // Route only 20% to token_b, so the router arrives holding a badly lopsided
    // pair. The pre-balance step swaps the excess before depositing, so almost
    // nothing is left over — previously this handed ~75% of one side to the
    // protocol as revenue.
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
    );

    let shares = RouterClient::new(&env, &router_addr).execute_strategy(&sender, &1_000, &xdr);

    let router = RouterClient::new(&env, &router_addr);
    // Without pre-balancing this minted 200 shares and handed ~600 units to the
    // protocol as revenue; balancing on-chain turns the same input into 497.
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

    // Give the sender LP shares by depositing directly.
    let (_, sac_a2) = (token_a.clone(), _sac_a);
    sac_a2.mint(&sender, &1_000);
    sac_b.mint(&sender, &1_000);
    aquarius_lp_mock::AqLpPoolClient::new(&env, &pool).deposit(
        &sender,
        &vec![&env, 1_000u128, 1_000u128],
        &0u128,
    );
    assert_eq!(token::Client::new(&env, &share).balance(&sender), 1_000);

    // token_b leg routes back into token_a; the token_a leg needs no path.
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
    );

    let out = RouterClient::new(&env, &router_addr).execute_strategy(&sender, &1_000, &xdr);

    // 1000 shares release 1000 of each side; the token_b half swaps 1:1 back.
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

    // Demand more of constituent 0 than 1000 shares can release.
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

    // An unrelated token stands in for the pool's real share token.
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

    // Partial splits are only legal when a mint leg consumes the remainder.
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
    );

    assert_eq!(
        RouterClient::new(&env, &router_addr)
            .try_execute_strategy(&sender, &1_000, &xdr)
            .unwrap_err()
            .unwrap(),
        Error::SplitPpmMismatch.into()
    );
}

/// An unbalanced pool must still mint, with the unconsumed remainder returned.
///
/// The pool pulls the full amounts and refunds the surplus, so the router
/// authorizes the full amounts; an invoker auth entry matches on argument
/// equality, so authorizing a predicted subset instead fails outright with
/// `Auth(InvalidAction)`. The pool is seeded unbalanced and deposited into once
/// so total shares sit off the reserve values — on a 1:1 pool the rounding
/// never splits and this case is invisible.
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

    // Routes 1999 of the 3000 into token B, leaving 1001 token A — the exact
    // pair where the two roundings disagree.
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
    );

    let shares = RouterClient::new(&env, &router_addr).execute_strategy(&sender, &3_000, &xdr);

    let router = RouterClient::new(&env, &router_addr);
    assert!(shares > 0);
    assert_eq!(token::Client::new(&env, &share).balance(&sender), shares);
    // Anything the pool declined is revenue, so the router's holdings equal the
    // accrued fee balances exactly — nothing is stranded unaccounted for.
    assert_eq!(
        token::Client::new(&env, &token_a).balance(&router_addr),
        router.admin_fee_balance(&token_a)
    );
    assert_eq!(
        token::Client::new(&env, &token_b).balance(&router_addr),
        router.admin_fee_balance(&token_b)
    );
}

/// A single-sided deposit needs no swap at all, so the batch carries no paths.
/// That is the optimal shape on a stable pool, where balancing costs more in
/// swap fees than the imbalance fee it avoids — rejecting it as an empty batch
/// would make the best stable route unexpressible.
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
    );

    // The mock is constant-product, which mints nothing for a one-sided
    // deposit; what matters here is that the payload is accepted rather than
    // rejected as an empty batch.
    let err = RouterClient::new(&env, &router_addr)
        .try_execute_strategy(&sender, &1_000, &xdr)
        .err();
    assert!(
        !matches!(err, Some(Ok(e)) if e == Error::EmptyBatch.into()),
        "single-sided deposit must not be rejected as an empty batch"
    );
}

/// Even a wildly mis-allocated deposit is rescued by pre-balancing.
///
/// Only 20% of the input is routed into the other constituent, so the router
/// holds roughly 4:1 against a 1:1 pool. Balancing on-chain against the real
/// balances turns that into a near-exact deposit; without it the pool would
/// decline most of one side and the sender would be charged for it.
#[test]
fn mint_pre_balances_even_a_wildly_skewed_input() {
    let env = Env::default();
    env.mock_all_auths();

    let router_addr = env.register(Router, (Address::generate(&env),));
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (pool, share, (token_a, sac_a), (token_b, sac_b)) = lp_pool(&env, &admin, 100_000_000_000);

    // Route only 20% into token_b, so the pool can take a balanced pair and
    // leaves ~60% of token_a behind — orders of magnitude past dust.
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
    );

    let shares =
        RouterClient::new(&env, &router_addr).execute_strategy(&sender, &10_000_000_000, &xdr);
    let router = RouterClient::new(&env, &router_addr);
    assert!(shares > 0);
    // Residual must be dust against a 10_000_000_000 input, not a share of it.
    let residual = router.admin_fee_balance(&token_a) + router.admin_fee_balance(&token_b);
    assert!(
        residual <= 10_000,
        "expected dust-level residual, got {residual}"
    );
}

/// A burn whose released constituents cannot all reach `token_out` is refused.
///
/// The router books leftovers as revenue, so without this the sender would hand
/// over a whole constituent — half their position — and be told the trade
/// succeeded. Here only the token_b leg is routed and token_a is left stranded,
/// which must revert rather than settle.
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

    // Only token_b can reach token_c; token_a has no path and would be stranded.
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
    );

    assert_eq!(
        RouterClient::new(&env, &router_addr)
            .try_execute_strategy(&sender, &shares, &xdr)
            .unwrap_err()
            .unwrap(),
        Error::ExcessiveResidual.into()
    );
}

/// A stable pool must NOT be pre-balanced.
///
/// Stable pools consume every amount offered and price the imbalance into the
/// shares, which costs less than the swap fee a rebalance would pay. Swapping
/// first would burn a fee to fix something the pool does not charge for, so the
/// router leaves a lopsided stable deposit exactly as it is.
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

    // Single-sided: a constant-product pool would have to swap half away first.
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
    );

    let shares = RouterClient::new(&env, &router_addr).execute_strategy(&sender, &1_000, &xdr);

    // The whole 1000 reaches the pool as a deposit. A pre-swap would route part
    // of it through the pool first and pay a fee, so the shares would come out
    // lower — that share count is what makes this test able to tell the two
    // apart, since the pool's token_a balance rises by 1000 either way.
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

/// `min_shares` is a floor, not a strict bound: minting exactly the requested
/// amount must succeed, one more than achievable must not.
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
        );
        (router_addr, sender, xdr)
    };

    // Learn what this deposit actually mints.
    let (router_addr, sender, xdr) = build(1);
    let achievable = RouterClient::new(&env, &router_addr).execute_strategy(&sender, &1_000, &xdr);

    let (router_addr, sender, xdr) = build(achievable);
    assert_eq!(
        RouterClient::new(&env, &router_addr).execute_strategy(&sender, &1_000, &xdr),
        achievable,
        "exactly meeting min_shares must succeed"
    );

    // Asking for one more than achievable must fail. The pool's own floor
    // rejects it first, so this asserts the outcome rather than our error code.
    let (router_addr, sender, xdr) = build(achievable + 1);
    assert!(
        RouterClient::new(&env, &router_addr)
            .try_execute_strategy(&sender, &1_000, &xdr)
            .is_err(),
        "asking above what the deposit can mint must fail"
    );
}

/// `burn_min_amounts` is a floor: receiving exactly the requested minimum must
/// succeed, one unit more must not.
#[test]
fn burn_min_amounts_boundary_is_inclusive() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);

    // Burning 1000 of 1_001_000 shares against equal reserves releases exactly
    // 1000 of each side.
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
        );
        (router_addr, sender, shares, xdr)
    };

    // Exactly the released amount is acceptable.
    let (router_addr, sender, shares, xdr) = build(1_000);
    assert!(
        RouterClient::new(&env, &router_addr)
            .try_execute_strategy(&sender, &shares, &xdr)
            .is_ok(),
        "a minimum met exactly must be accepted"
    );

    // One unit beyond what the burn can release is not. The pool enforces its
    // own floor first, so this asserts the outcome rather than our error code.
    let (router_addr, sender, shares, xdr) = build(1_001);
    assert!(
        RouterClient::new(&env, &router_addr)
            .try_execute_strategy(&sender, &shares, &xdr)
            .is_err(),
        "a minimum above what the burn releases must fail"
    );
}
