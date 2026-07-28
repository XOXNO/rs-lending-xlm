//! Soroswap venue adapter tests.
//!
//! The router sizes Soroswap output from **live reserves** (0.3% ceil fee,
//! floor out) and ignores a stale `hop.amount_out`. Pair orientation follows
//! the sorted `(token_0, token_1)` address invariant.

use crate::errors::Error;
use crate::types::{SwapHop, SwapPath, SwapVenue};
use crate::{Router, RouterClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{token, vec, Address, Env};

use super::super::support::{new_asset, one_hop_path, soroswap_mock, strategy_xdr};

#[test]
fn soroswap_single_hop_derives_output_from_live_reserves() {
    let env = Env::default();
    env.mock_all_auths();

    let router_addr = env.register(Router, (Address::generate(&env),));
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (asset_x, sac_x) = new_asset(&env, &admin);
    let (asset_y, sac_y) = new_asset(&env, &admin);
    // Soroswap's factory stores tokens canonically sorted (`token_0 < token_1`
    // under the host's address ordering). The router now derives the pair's
    // orientation from that invariant instead of reading `token_0`/`token_1`,
    // so the mock must be set up sorted too: smaller address = token_0 = the
    // input token for this hop.
    let ((token_in, sac_in), (token_out, sac_out)) = if asset_x < asset_y {
        ((asset_x, sac_x), (asset_y, sac_y))
    } else {
        ((asset_y, sac_y), (asset_x, sac_x))
    };

    // 2:1 reserves. The router's on-chain Soroswap math (0.3% ceil fee, floor
    // output) honors exactly 995 for amount_in = 500:
    //   fee     = ceil(500 * 3 / 1000)                 = 2
    //   in_less = 500 - 2                               = 498
    //   out     = 498 * 2_000_000 / (1_000_000 + 498)   = 995
    // which is the largest output the pair's k-invariant accepts for this input
    // (the mock asserts that invariant, so an over-sized request would panic).
    let reserve_0: i128 = 1_000_000;
    let reserve_1: i128 = 2_000_000;
    let reserve_derived_out: i128 = 995;
    let pool = env.register(soroswap_mock::SoroswapPair, ());
    soroswap_mock::SoroswapPairClient::new(&env, &pool)
        .init(&token_in, &token_out, &reserve_0, &reserve_1);

    sac_in.mint(&pool, &reserve_0);
    sac_out.mint(&pool, &reserve_1);
    sac_in.mint(&sender, &1_000);

    // Stale hop.amount_out is ignored; router sizes from live reserves.
    let stale_quoted_out: i128 = 375;
    // min_out under live output so this test hits sizing, not aggregate slippage.
    let total_min_out: i128 = 900;

    let swap_xdr = strategy_xdr(
        &env,
        token_in.clone(),
        token_out.clone(),
        total_min_out,
        vec![
            &env,
            SwapPath {
                split_ppm: 1_000_000,
                hops: vec![
                    &env,
                    SwapHop {
                        venue: SwapVenue::Soroswap,
                        amount_out: stale_quoted_out,
                        pool,
                        token_in: token_in.clone(),
                        token_out: token_out.clone(),
                    },
                ],
            },
        ],
    );

    let out = RouterClient::new(&env, &router_addr).execute_strategy(&sender, &500, &swap_xdr);
    assert_eq!(out, reserve_derived_out);
    assert_ne!(out, stale_quoted_out);
    assert_eq!(token::Client::new(&env, &token_in).balance(&sender), 500);
    assert_eq!(
        token::Client::new(&env, &token_out).balance(&sender),
        reserve_derived_out
    );
}

#[test]
fn soroswap_reverse_orientation() {
    let env = Env::default();
    env.mock_all_auths();
    let router_addr = env.register(Router, (Address::generate(&env),));
    let client = RouterClient::new(&env, &router_addr);
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (ax, sacx) = new_asset(&env, &admin);
    let (ay, sacy) = new_asset(&env, &admin);
    let ((t0, sac0), (t1, sac1)) = if ax < ay {
        ((ax, sacx), (ay, sacy))
    } else {
        ((ay, sacy), (ax, sacx))
    };
    let pool = env.register(soroswap_mock::SoroswapPair, ());
    soroswap_mock::SoroswapPairClient::new(&env, &pool).init(&t0, &t1, &2_000_000, &1_000_000);
    sac0.mint(&pool, &2_000_000);
    sac1.mint(&pool, &1_000_000);
    sac1.mint(&sender, &1_000);
    // swap t1 (token_1) -> t0 (token_0): reverse direction
    let xdr = strategy_xdr(
        &env,
        t1.clone(),
        t0.clone(),
        900,
        vec![
            &env,
            one_hop_path(
                &env,
                SwapVenue::Soroswap,
                pool,
                t1.clone(),
                t0.clone(),
                1_000_000,
            ),
        ],
    );
    let out = client.execute_strategy(&sender, &500, &xdr);
    assert!(out >= 900);
    assert!(token::Client::new(&env, &t0).balance(&sender) >= 900);
}

#[test]
fn soroswap_zero_output_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let router_addr = env.register(Router, (Address::generate(&env),));
    let client = RouterClient::new(&env, &router_addr);
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (ax, sacx) = new_asset(&env, &admin);
    let (ay, sacy) = new_asset(&env, &admin);
    let ((t0, sac0), (t1, _sac1)) = if ax < ay {
        ((ax, sacx), (ay, sacy))
    } else {
        ((ay, sacy), (ax, sacx))
    };

    // A pool with no output reserves cannot produce a swap.
    let pool0 = env.register(soroswap_mock::SoroswapPair, ());
    soroswap_mock::SoroswapPairClient::new(&env, &pool0).init(&t0, &t1, &1_000_000, &0);
    sac0.mint(&sender, &1_000);
    let xdr = strategy_xdr(
        &env,
        t0.clone(),
        t1.clone(),
        1,
        vec![
            &env,
            one_hop_path(
                &env,
                SwapVenue::Soroswap,
                pool0,
                t0.clone(),
                t1.clone(),
                1_000_000,
            ),
        ],
    );
    assert_eq!(
        client
            .try_execute_strategy(&sender, &100, &xdr)
            .unwrap_err()
            .unwrap(),
        Error::ZeroOutput.into()
    );

    // A one-unit input is consumed by fee rounding and produces no output.
    let pool1 = env.register(soroswap_mock::SoroswapPair, ());
    soroswap_mock::SoroswapPairClient::new(&env, &pool1).init(&t0, &t1, &1_000_000, &1_000_000);
    let xdr = strategy_xdr(
        &env,
        t0.clone(),
        t1.clone(),
        1,
        vec![
            &env,
            one_hop_path(
                &env,
                SwapVenue::Soroswap,
                pool1,
                t0.clone(),
                t1.clone(),
                1_000_000,
            ),
        ],
    );
    assert_eq!(
        client
            .try_execute_strategy(&sender, &1, &xdr)
            .unwrap_err()
            .unwrap(),
        Error::ZeroOutput.into()
    );
}

// A pool with an empty INPUT reserve cannot honor any output; the amount-out
// math must return zero (→ ZeroOutput) instead of dividing against a zero
// reserve and requesting the pool's whole output side.
#[test]
fn soroswap_zero_input_reserve_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let router_addr = env.register(Router, (Address::generate(&env),));
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (ax, sacx) = new_asset(&env, &admin);
    let (ay, sacy) = new_asset(&env, &admin);
    let ((t0, sac0), (t1, sac1)) = if ax < ay {
        ((ax, sacx), (ay, sacy))
    } else {
        ((ay, sacy), (ax, sacx))
    };

    let pool = env.register(soroswap_mock::SoroswapPair, ());
    soroswap_mock::SoroswapPairClient::new(&env, &pool).init(&t0, &t1, &0, &1_000_000);
    sac1.mint(&pool, &1_000_000);
    sac0.mint(&sender, &500);

    let xdr = strategy_xdr(
        &env,
        t0.clone(),
        t1.clone(),
        1,
        vec![
            &env,
            one_hop_path(
                &env,
                SwapVenue::Soroswap,
                pool,
                t0.clone(),
                t1.clone(),
                1_000_000,
            ),
        ],
    );
    assert_eq!(
        RouterClient::new(&env, &router_addr)
            .try_execute_strategy(&sender, &500, &xdr)
            .unwrap_err()
            .unwrap(),
        Error::ZeroOutput.into()
    );
}
