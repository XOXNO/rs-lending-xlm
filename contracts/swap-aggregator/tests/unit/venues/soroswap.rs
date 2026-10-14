use crate::errors::Error;
use crate::types::{SwapHop, SwapVenue};
use crate::{Router, RouterClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{token, Address, Env};

use super::super::support::{new_asset, one_hop_path, soroswap_mock, strategy_xdr, SwapPath};

#[test]
fn soroswap_single_hop_derives_output_from_live_reserves() {
    let env = Env::default();
    env.mock_all_auths();

    let router_addr = env.register(Router, (Address::generate(&env),));
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (asset_x, sac_x) = new_asset(&env, &admin);
    let (asset_y, sac_y) = new_asset(&env, &admin);

    let ((token_in, sac_in), (token_out, sac_out)) = if asset_x < asset_y {
        ((asset_x, sac_x), (asset_y, sac_y))
    } else {
        ((asset_y, sac_y), (asset_x, sac_x))
    };

    let reserve_0: i128 = 1_000_000;
    let reserve_1: i128 = 2_000_000;
    let reserve_derived_out: i128 = 995;
    let pool = env.register(soroswap_mock::SoroswapPair, ());
    soroswap_mock::SoroswapPairClient::new(&env, &pool)
        .init(&token_in, &token_out, &reserve_0, &reserve_1);

    sac_in.mint(&pool, &reserve_0);
    sac_out.mint(&pool, &reserve_1);
    sac_in.mint(&sender, &1_000);

    let total_min_out: i128 = 900;

    let swap_xdr = strategy_xdr(
        &env,
        token_in.clone(),
        token_out.clone(),
        total_min_out,
        alloc::vec![SwapPath {
            split_ppm: 1_000_000,
            hops: alloc::vec![SwapHop {
                venue: SwapVenue::Soroswap,
                pool,
                token_in: token_in.clone(),
                token_out: token_out.clone(),
            },],
        },],
    );

    let out = RouterClient::new(&env, &router_addr).execute_strategy(&sender, &500, &swap_xdr);
    assert_eq!(out, reserve_derived_out);
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

    let xdr = strategy_xdr(
        &env,
        t1.clone(),
        t0.clone(),
        900,
        alloc::vec![one_hop_path(
            &env,
            SwapVenue::Soroswap,
            pool,
            t1.clone(),
            t0.clone(),
            1_000_000,
        ),],
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

    let pool0 = env.register(soroswap_mock::SoroswapPair, ());
    soroswap_mock::SoroswapPairClient::new(&env, &pool0).init(&t0, &t1, &1_000_000, &0);
    sac0.mint(&sender, &1_000);
    let xdr = strategy_xdr(
        &env,
        t0.clone(),
        t1.clone(),
        1,
        alloc::vec![one_hop_path(
            &env,
            SwapVenue::Soroswap,
            pool0,
            t0.clone(),
            t1.clone(),
            1_000_000,
        ),],
    );
    assert_eq!(
        client
            .try_execute_strategy(&sender, &100, &xdr)
            .unwrap_err()
            .unwrap(),
        Error::ZeroOutput.into()
    );

    let pool1 = env.register(soroswap_mock::SoroswapPair, ());
    soroswap_mock::SoroswapPairClient::new(&env, &pool1).init(&t0, &t1, &1_000_000, &1_000_000);
    let xdr = strategy_xdr(
        &env,
        t0.clone(),
        t1.clone(),
        1,
        alloc::vec![one_hop_path(
            &env,
            SwapVenue::Soroswap,
            pool1,
            t0.clone(),
            t1.clone(),
            1_000_000,
        ),],
    );
    assert_eq!(
        client
            .try_execute_strategy(&sender, &1, &xdr)
            .unwrap_err()
            .unwrap(),
        Error::ZeroOutput.into()
    );
}

/// An 18-decimal pair with 5e24 reserves on each side quotes without trapping.
///
/// A 1e18 input makes `in_less * reserve_out` about 5e42, past `i128::MAX`.
/// `soroswap_amount_out` must widen that product. A raw `i128` multiply traps
/// on-chain too, because the release profile sets `overflow-checks = true`.
#[test]
fn large_reserve_pair_does_not_trap() {
    let env = Env::default();
    env.mock_all_auths();

    let router_addr = env.register(Router, (Address::generate(&env),));
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (asset_x, sac_x) = new_asset(&env, &admin);
    let (asset_y, sac_y) = new_asset(&env, &admin);
    let ((token_in, sac_in), (token_out, sac_out)) = if asset_x < asset_y {
        ((asset_x, sac_x), (asset_y, sac_y))
    } else {
        ((asset_y, sac_y), (asset_x, sac_x))
    };

    // 5,000,000 units of an 18-decimal token on each side.
    let reserve: i128 = 5_000_000 * 1_000_000_000_000_000_000;
    let amount_in: i128 = 1_000_000_000_000_000_000;
    let pool = env.register(soroswap_mock::SoroswapPair, ());
    soroswap_mock::SoroswapPairClient::new(&env, &pool)
        .init(&token_in, &token_out, &reserve, &reserve);

    sac_in.mint(&pool, &reserve);
    sac_out.mint(&pool, &reserve);
    sac_in.mint(&sender, &amount_in);

    let swap_xdr = strategy_xdr(
        &env,
        token_in.clone(),
        token_out.clone(),
        1,
        alloc::vec![one_hop_path(
            &env,
            SwapVenue::Soroswap,
            pool,
            token_in.clone(),
            token_out.clone(),
            1_000_000,
        ),],
    );

    // in_less = 997e15; 997e15 * 5e24 / (5e24 + 997e15), floored.
    let expected: i128 = 996_999_801_198_239_641;
    let out =
        RouterClient::new(&env, &router_addr).execute_strategy(&sender, &amount_in, &swap_xdr);
    assert_eq!(out, expected);
    assert_eq!(
        token::Client::new(&env, &token_out).balance(&sender),
        expected
    );
}

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
        alloc::vec![one_hop_path(
            &env,
            SwapVenue::Soroswap,
            pool,
            t0.clone(),
            t1.clone(),
            1_000_000,
        ),],
    );
    assert_eq!(
        RouterClient::new(&env, &router_addr)
            .try_execute_strategy(&sender, &500, &xdr)
            .unwrap_err()
            .unwrap(),
        Error::ZeroOutput.into()
    );
}

/// R-C fix (router_adapter): the adapter reads the pair's `token_0` / `token_1`
/// and orients the quote and the `swap` call from them, as the Sushi adapter
/// does. A pair that stores the higher address as `token_0` is quoted from the
/// right reserves and pays out the reserve-derived amount. (A genuine Soroswap
/// pair refuses that order at initialization; this models a non-canonical pair
/// contract routed at the Soroswap opcode.)
///
/// Before the fix the adapter derived the orientation from address order, read
/// the reserves the wrong way round, asked the pair for its input token back,
/// and the hop failed with `ZeroOutput`.
#[test]
fn soroswap_direction_is_read_from_the_pair_not_from_address_order() {
    let env = Env::default();
    env.mock_all_auths();
    let router_addr = env.register(Router, (Address::generate(&env),));
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (ax, sacx) = new_asset(&env, &admin);
    let (ay, sacy) = new_asset(&env, &admin);
    let ((t_lo, sac_lo), (t_hi, sac_hi)) = if ax < ay {
        ((ax, sacx), (ay, sacy))
    } else {
        ((ay, sacy), (ax, sacx))
    };

    // token_0 is the HIGHER address: reserve_0 = 2_000_000 t_hi, reserve_1 = 1_000_000 t_lo.
    let pool = env.register(soroswap_mock::SoroswapPair, ());
    soroswap_mock::SoroswapPairClient::new(&env, &pool).init(&t_hi, &t_lo, &2_000_000, &1_000_000);
    sac_hi.mint(&pool, &2_000_000);
    sac_lo.mint(&pool, &1_000_000);
    sac_lo.mint(&sender, &500);

    let xdr = strategy_xdr(
        &env,
        t_lo.clone(),
        t_hi.clone(),
        1,
        alloc::vec![one_hop_path(
            &env,
            SwapVenue::Soroswap,
            pool.clone(),
            t_lo.clone(),
            t_hi.clone(),
            1_000_000,
        )],
    );

    // in_less = 500 - ceil(1.5) = 498; floor(498 * 2_000_000 / 1_000_498) = 995.
    let out = RouterClient::new(&env, &router_addr).execute_strategy(&sender, &500, &xdr);
    assert_eq!(out, 995);
    assert_eq!(token::Client::new(&env, &t_hi).balance(&sender), 995);
    assert_eq!(token::Client::new(&env, &t_lo).balance(&sender), 0);
    assert_eq!(token::Client::new(&env, &t_lo).balance(&pool), 1_000_500);
    assert_eq!(token::Client::new(&env, &t_hi).balance(&router_addr), 0);
    assert_eq!(token::Client::new(&env, &t_lo).balance(&router_addr), 0);
}

/// R-C fix (router_adapter): a pair that does not hold the hop's two tokens fails
/// closed with `BrokenTokenChain` from its own token views, before the router
/// quotes or transfers anything. Before the fix the hop transferred the input,
/// asked for the pair's other token, and failed with `ZeroOutput`.
#[test]
fn soroswap_rejects_a_pair_that_does_not_hold_the_hop_tokens() {
    let env = Env::default();
    env.mock_all_auths();
    let router_addr = env.register(Router, (Address::generate(&env),));
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (token_in, sac_in) = new_asset(&env, &admin);
    let (token_out, _) = new_asset(&env, &admin);
    let (stranger, sac_stranger) = new_asset(&env, &admin);

    // The pair trades token_in against an unrelated token, not token_out.
    let pool = env.register(soroswap_mock::SoroswapPair, ());
    soroswap_mock::SoroswapPairClient::new(&env, &pool)
        .init(&token_in, &stranger, &1_000_000, &1_000_000);
    sac_in.mint(&pool, &1_000_000);
    sac_stranger.mint(&pool, &1_000_000);
    sac_in.mint(&sender, &500);

    let xdr = strategy_xdr(
        &env,
        token_in.clone(),
        token_out.clone(),
        1,
        alloc::vec![one_hop_path(
            &env,
            SwapVenue::Soroswap,
            pool.clone(),
            token_in.clone(),
            token_out.clone(),
            1_000_000,
        )],
    );
    assert_eq!(
        RouterClient::new(&env, &router_addr)
            .try_execute_strategy(&sender, &500, &xdr)
            .unwrap_err()
            .unwrap(),
        Error::BrokenTokenChain.into()
    );
    assert_eq!(token::Client::new(&env, &token_in).balance(&sender), 500);
    assert_eq!(
        token::Client::new(&env, &token_in).balance(&pool),
        1_000_000
    );
    assert_eq!(
        token::Client::new(&env, &stranger).balance(&pool),
        1_000_000
    );
}

/// R-C (documentation pin, unchanged by the fix): a Soroswap pair exposes no fee
/// getter, so the adapter quotes the 30 BPS tier the Soroswap pair hard-codes. A
/// pair on a cheaper tier is under-requested (the sender receives exactly the
/// 30 BPS quote, below what the pair would pay); a pair on a dearer tier rejects
/// the request in its own k check and the strategy reverts with nothing moved.
#[test]
fn soroswap_fee_tier_mismatch_under_requests_or_fails_closed() {
    // UniswapV2 `getAmountOut` at `fee_num / 1_000`.
    fn pair_quote(amount_in: i128, reserve_in: i128, reserve_out: i128, fee_num: i128) -> i128 {
        let in_with_fee = amount_in * (1_000 - fee_num);
        in_with_fee * reserve_out / (reserve_in * 1_000 + in_with_fee)
    }

    for (fee_num, settles) in [(1_i128, true), (3, true), (10, false)] {
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
        let pair = soroswap_mock::SoroswapPairClient::new(&env, &pool);
        pair.init(&t0, &t1, &1_000_000, &2_000_000);
        pair.set_fee_num(&fee_num);
        sac0.mint(&pool, &1_000_000);
        sac1.mint(&pool, &2_000_000);
        sac0.mint(&sender, &500);

        let xdr = strategy_xdr(
            &env,
            t0.clone(),
            t1.clone(),
            1,
            alloc::vec![one_hop_path(
                &env,
                SwapVenue::Soroswap,
                pool.clone(),
                t0.clone(),
                t1.clone(),
                1_000_000,
            )],
        );
        let result =
            RouterClient::new(&env, &router_addr).try_execute_strategy(&sender, &500, &xdr);
        // The adapter's 30 BPS quote for this trade, whatever the pair's tier: the fee is
        // rounded up on the input, so it sits one unit under the pair's own 30 BPS quote (996).
        let adapter_quote = 995;
        if settles {
            assert_eq!(result.unwrap().unwrap(), adapter_quote, "fee_num {fee_num}");
            assert!(
                adapter_quote <= pair_quote(500, 1_000_000, 2_000_000, fee_num),
                "fee_num {fee_num}: never over-requests a cheaper tier"
            );
            assert_eq!(
                token::Client::new(&env, &t1).balance(&sender),
                adapter_quote
            );
        } else {
            assert!(
                adapter_quote > pair_quote(500, 1_000_000, 2_000_000, fee_num),
                "fee_num {fee_num}: the 30 BPS quote over-requests a dearer tier"
            );
            assert!(
                result.is_err(),
                "fee_num {fee_num}: the pair's k check refuses"
            );
            assert_eq!(token::Client::new(&env, &t0).balance(&sender), 500);
            assert_eq!(token::Client::new(&env, &t1).balance(&sender), 0);
        }
    }
}

/// A Soroswap-shaped pair that exposes its tokens under the Sushi names
/// (`token0`/`token1`) instead of `token_0`/`token_1`: a fork or legacy ABI.
mod skeptic_legacy_pair {
    use soroban_sdk::{contract, contractimpl, contracttype, token, Address, Env};

    #[contracttype]
    #[derive(Clone)]
    pub enum Key {
        T0,
        T1,
        R0,
        R1,
    }

    #[contract]
    pub struct SkepticLegacyPair;

    #[contractimpl]
    impl SkepticLegacyPair {
        pub fn init(env: Env, t0: Address, t1: Address, r0: i128, r1: i128) {
            let s = env.storage().instance();
            s.set(&Key::T0, &t0);
            s.set(&Key::T1, &t1);
            s.set(&Key::R0, &r0);
            s.set(&Key::R1, &r1);
        }

        pub fn token0(env: Env) -> Address {
            env.storage().instance().get(&Key::T0).unwrap()
        }

        pub fn token1(env: Env) -> Address {
            env.storage().instance().get(&Key::T1).unwrap()
        }

        pub fn get_reserves(env: Env) -> (i128, i128) {
            let s = env.storage().instance();
            (s.get(&Key::R0).unwrap(), s.get(&Key::R1).unwrap())
        }

        pub fn swap(env: Env, amount_0_out: i128, amount_1_out: i128, to: Address) {
            let pair = env.current_contract_address();
            if amount_0_out > 0 {
                let t0: Address = env.storage().instance().get(&Key::T0).unwrap();
                token::Client::new(&env, &t0).transfer(&pair, &to, &amount_0_out);
            }
            if amount_1_out > 0 {
                let t1: Address = env.storage().instance().get(&Key::T1).unwrap();
                token::Client::new(&env, &t1).transfer(&pair, &to, &amount_1_out);
            }
        }
    }
}

/// Skeptic attack (router_adapter, R-C): a pair whose views report the input
/// token on both sides cannot be oriented either way; the guard refuses it with
/// `BrokenTokenChain` before the router quotes or transfers anything.
#[test]
fn skeptic_rc_pair_reporting_the_input_token_on_both_sides_is_refused_before_any_transfer() {
    let env = Env::default();
    env.mock_all_auths();
    let router_addr = env.register(Router, (Address::generate(&env),));
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (token_in, sac_in) = new_asset(&env, &admin);
    let (token_out, sac_out) = new_asset(&env, &admin);

    let pool = env.register(soroswap_mock::SoroswapPair, ());
    soroswap_mock::SoroswapPairClient::new(&env, &pool)
        .init(&token_in, &token_in, &1_000_000, &1_000_000);
    sac_in.mint(&pool, &1_000_000);
    sac_out.mint(&pool, &1_000_000);
    sac_in.mint(&sender, &500);

    let xdr = strategy_xdr(
        &env,
        token_in.clone(),
        token_out.clone(),
        1,
        alloc::vec![one_hop_path(
            &env,
            SwapVenue::Soroswap,
            pool.clone(),
            token_in.clone(),
            token_out.clone(),
            1_000_000,
        )],
    );
    assert_eq!(
        RouterClient::new(&env, &router_addr)
            .try_execute_strategy(&sender, &500, &xdr)
            .unwrap_err()
            .unwrap(),
        Error::BrokenTokenChain.into()
    );
    assert_eq!(token::Client::new(&env, &token_in).balance(&sender), 500);
    assert_eq!(
        token::Client::new(&env, &token_in).balance(&pool),
        1_000_000
    );
    assert_eq!(
        token::Client::new(&env, &token_out).balance(&pool),
        1_000_000
    );
}

/// Skeptic attack (router_adapter, R-C), availability pin: the guard makes
/// `token_0`/`token_1` part of the Soroswap opcode's required ABI. A pair that
/// names them differently (here the Sushi names) used to be quoted from
/// address order and settled; it now fails closed before any transfer. The
/// upstream Soroswap pair exports `token_0`/`token_1`, so this binds only forks
/// or legacy pairs routed at the Soroswap opcode.
#[test]
fn skeptic_rc_pair_without_the_token_views_now_fails_closed_before_any_transfer() {
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
    let pool = env.register(skeptic_legacy_pair::SkepticLegacyPair, ());
    skeptic_legacy_pair::SkepticLegacyPairClient::new(&env, &pool)
        .init(&t0, &t1, &1_000_000, &2_000_000);
    sac0.mint(&pool, &1_000_000);
    sac1.mint(&pool, &2_000_000);
    sac0.mint(&sender, &500);

    let xdr = strategy_xdr(
        &env,
        t0.clone(),
        t1.clone(),
        1,
        alloc::vec![one_hop_path(
            &env,
            SwapVenue::Soroswap,
            pool.clone(),
            t0.clone(),
            t1.clone(),
            1_000_000,
        )],
    );
    let result = RouterClient::new(&env, &router_addr).try_execute_strategy(&sender, &500, &xdr);
    assert!(
        result.is_err(),
        "a pair without token_0/token_1 must fail closed"
    );
    assert_eq!(token::Client::new(&env, &t0).balance(&sender), 500);
    assert_eq!(token::Client::new(&env, &t1).balance(&sender), 0);
    assert_eq!(token::Client::new(&env, &t0).balance(&pool), 1_000_000);
    assert_eq!(token::Client::new(&env, &t1).balance(&pool), 2_000_000);
}
