//! Version 2 (exact-output) programs: graph checks, backward pricing, per-hop fill checks, and
//! the input refund.

use crate::errors::Error;
use crate::types::StrategyPayload;
use crate::{Router, RouterClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{token, xdr::ToXdr, Address, Bytes, Env, Vec};

use super::support::{
    aquarius_mock, comet_mock, new_asset, signed_sushi_mock, soroswap_mock, tunable_aquarius_mock,
    tunable_phoenix_mock,
};

const SOROSWAP: u8 = 0;
const AQUARIUS: u8 = 1;
const PHOENIX: u8 = 2;
const SUSHI: u8 = 3;
const COMET: u8 = 4;

/// `Mode::Fixed(1)`: the output weight at `amounts[1]`.
const WEIGHT_1: u8 = 3;
/// `Mode::Fixed(2)`: the output weight at `amounts[2]`.
const WEIGHT_2: u8 = 4;

/// Encodes a version 2 payload with no referral. `assets[0]` is the input token,
/// `assets[token_out]` the output token, and `amounts[0]` the exact payout.
fn payload(
    env: &Env,
    assets: &[&Address],
    token_out: u8,
    amounts: &[i128],
    ops: &[[u8; 5]],
) -> Bytes {
    let mut raw = alloc::vec![2, 0, token_out, 0, 0, 0, 0, 0, ops.len() as u8, 0];
    for op in ops {
        raw.extend_from_slice(op);
    }
    let mut registry = Vec::new(env);
    for asset in assets {
        registry.push_back((*asset).clone());
    }
    StrategyPayload {
        amounts: Vec::from_slice(env, amounts),
        assets: registry,
        ops: Bytes::from_slice(env, &raw),
    }
    .to_xdr(env)
}

struct Setup<'a> {
    env: &'a Env,
    router: RouterClient<'a>,
    sender: Address,
    admin: Address,
}

impl<'a> Setup<'a> {
    fn new(env: &'a Env) -> Self {
        env.mock_all_auths();
        let admin = Address::generate(env);
        let router = RouterClient::new(env, &env.register(Router, (admin.clone(),)));
        Setup {
            env,
            router,
            sender: Address::generate(env),
            admin,
        }
    }

    fn asset(&self) -> (Address, token::StellarAssetClient<'a>) {
        new_asset(self.env, &self.admin)
    }

    fn aquarius(&self, token_a: &Address, token_b: &Address) -> Address {
        let pool = self.env.register(aquarius_mock::AqPool, ());
        aquarius_mock::AqPoolClient::new(self.env, &pool).init(token_a, token_b);
        pool
    }

    fn balance(&self, token: &Address, holder: &Address) -> i128 {
        token::Client::new(self.env, token).balance(holder)
    }

    fn rejects(&self, total_in: i128, xdr: &Bytes, error: Error) {
        assert_eq!(
            self.router
                .try_execute_strategy(&self.sender, &total_in, xdr)
                .unwrap_err()
                .unwrap(),
            error.into()
        );
    }
}

#[test]
fn exact_deposit_on_a_pool_listing_the_output_first_refunds_nothing() {
    let env = Env::default();
    let s = Setup::new(&env);
    let (a, sac_a) = s.asset();
    let (b, sac_b) = s.asset();
    // The pool lists the output token first, so the hop swaps index 1 for index 0.
    let pool = s.aquarius(&b, &a);
    sac_b.mint(&pool, &500);
    sac_a.mint(&s.sender, &500);

    let xdr = payload(
        &env,
        &[&a, &b, &pool],
        1,
        &[500, 1],
        &[[AQUARIUS, WEIGHT_1, 2, 0, 1]],
    );
    assert_eq!(s.router.execute_strategy(&s.sender, &500, &xdr), 500);
    assert_eq!(s.balance(&a, &s.sender), 0);
    assert_eq!(s.balance(&b, &s.sender), 500);
    assert_eq!(s.balance(&a, &pool), 500);
    assert_eq!(s.balance(&a, &s.router.address), 0);
}

#[test]
fn split_gives_each_producer_its_weighted_share_and_the_first_the_remainder() {
    let env = Env::default();
    let s = Setup::new(&env);
    let (a, sac_a) = s.asset();
    let (c, sac_c) = s.asset();
    let first = s.aquarius(&a, &c);
    let second = s.aquarius(&a, &c);
    sac_c.mint(&first, &1_000);
    sac_c.mint(&second, &1_000);
    sac_a.mint(&s.sender, &400);

    // Weights 2 and 1 over a 400 payout: the second hop takes floor(400 / 3) = 133 and the
    // first hop takes the remaining 267.
    let xdr = payload(
        &env,
        &[&a, &c, &first, &second],
        1,
        &[400, 2, 1],
        &[[AQUARIUS, WEIGHT_1, 2, 0, 1], [AQUARIUS, WEIGHT_2, 3, 0, 1]],
    );
    assert_eq!(s.router.execute_strategy(&s.sender, &400, &xdr), 400);
    assert_eq!(s.balance(&a, &first), 267);
    assert_eq!(s.balance(&a, &second), 133);
    assert_eq!(s.balance(&c, &s.sender), 400);
}

/// Returns `(token_in, token_out)` sorted the way a Soroswap pair stores them, with a pair
/// holding `reserve_in` and `reserve_out`.
fn soroswap_pair(s: &Setup, reserve_in: i128, reserve_out: i128) -> (Address, Address, Address) {
    let (x, sac_x) = s.asset();
    let (y, sac_y) = s.asset();
    let ((t0, sac0), (t1, sac1)) = if x < y {
        ((x, sac_x), (y, sac_y))
    } else {
        ((y, sac_y), (x, sac_x))
    };
    let pair = s.env.register(soroswap_mock::SoroswapPair, ());
    soroswap_mock::SoroswapPairClient::new(s.env, &pair).init(&t0, &t1, &reserve_in, &reserve_out);
    sac0.mint(&pair, &reserve_in);
    sac1.mint(&pair, &reserve_out);
    sac0.mint(&s.sender, &2_000);
    (t0, t1, pair)
}

#[test]
fn soroswap_exact_output_spends_the_fee_inclusive_reverse_quote() {
    let env = Env::default();
    let s = Setup::new(&env);
    let (t0, t1, pair) = soroswap_pair(&s, 1_000_000, 1_000_000);

    // Net input ceil(1e6 * 1000 / 999_000) = 1002, then ceil(1002 * 1000 / 997) = 1006 with the
    // 0.3% fee. The pair's k check rejects any smaller input.
    let xdr = payload(
        &env,
        &[&t0, &t1, &pair],
        1,
        &[1_000, 1],
        &[[SOROSWAP, WEIGHT_1, 2, 0, 1]],
    );
    assert_eq!(s.router.execute_strategy(&s.sender, &2_000, &xdr), 1_000);
    assert_eq!(s.balance(&t0, &s.sender), 2_000 - 1_006);
    assert_eq!(s.balance(&t1, &s.sender), 1_000);
}

#[test]
fn soroswap_exact_output_rejects_a_target_the_pair_cannot_supply() {
    let env = Env::default();
    let s = Setup::new(&env);
    let (t0, t1, pair) = soroswap_pair(&s, 1_000_000, 1_000);

    let xdr = payload(
        &env,
        &[&t0, &t1, &pair],
        1,
        &[1_000, 1],
        &[[SOROSWAP, WEIGHT_1, 2, 0, 1]],
    );
    s.rejects(2_000, &xdr, Error::SlippageExceeded);
    assert_eq!(s.balance(&t0, &s.sender), 2_000);
}

#[test]
fn sushi_exact_output_requests_the_output_amount() {
    let env = Env::default();
    let s = Setup::new(&env);
    let (a, sac_a) = s.asset();
    let (b, sac_b) = s.asset();
    let pool = env.register(signed_sushi_mock::SignedSushiPool, ());
    signed_sushi_mock::SignedSushiPoolClient::new(&env, &pool).init(&a, &b);
    sac_b.mint(&pool, &1_000);
    sac_a.mint(&s.sender, &300);

    // Two input units per output unit: 100 out costs exactly 200 in.
    let xdr = payload(
        &env,
        &[&a, &b, &pool],
        1,
        &[100, 1],
        &[[SUSHI, WEIGHT_1, 2, 0, 1]],
    );
    assert_eq!(s.router.execute_strategy(&s.sender, &300, &xdr), 100);
    assert_eq!(s.balance(&a, &s.sender), 100);
    assert_eq!(s.balance(&b, &s.sender), 100);
}

#[test]
fn graph_check_rejects_a_reused_pool_a_late_producer_and_a_hop_out_of_the_sink() {
    let env = Env::default();
    let s = Setup::new(&env);
    let (a, sac_a) = s.asset();
    let (b, sac_b) = s.asset();
    let (c, sac_c) = s.asset();
    let ab = s.aquarius(&a, &b);
    let bc = s.aquarius(&b, &c);
    let ab2 = s.aquarius(&a, &b);
    let ac = s.aquarius(&a, &c);
    for pool in [&ab, &ab2] {
        sac_b.mint(pool, &1_000);
    }
    for pool in [&bc, &ac] {
        sac_c.mint(pool, &1_000);
    }
    sac_a.mint(&s.sender, &1_000);

    // Two splits through the same pool.
    let reused = payload(
        &env,
        &[&a, &c, &ac],
        1,
        &[100, 1],
        &[[AQUARIUS, WEIGHT_1, 2, 0, 1], [AQUARIUS, WEIGHT_1, 2, 0, 1]],
    );
    // `b` is produced again after a hop consumed it.
    let late_producer = payload(
        &env,
        &[&a, &b, &c, &ab, &bc, &ab2],
        2,
        &[100, 1],
        &[
            [AQUARIUS, WEIGHT_1, 3, 0, 1],
            [AQUARIUS, WEIGHT_1, 4, 1, 2],
            [AQUARIUS, WEIGHT_1, 5, 0, 1],
        ],
    );
    // The sink `b` feeds a further hop.
    let spends_sink = payload(
        &env,
        &[&a, &b, &c, &ab, &bc],
        1,
        &[100, 1],
        &[[AQUARIUS, WEIGHT_1, 3, 0, 1], [AQUARIUS, WEIGHT_1, 4, 1, 2]],
    );
    for xdr in [reused, late_producer, spends_sink] {
        s.rejects(1_000, &xdr, Error::BrokenTokenChain);
    }
    assert_eq!(s.balance(&a, &s.sender), 1_000);
}

#[test]
fn comet_hop_must_spend_the_source_token() {
    let env = Env::default();
    let s = Setup::new(&env);
    let (a, sac_a) = s.asset();
    let (b, sac_b) = s.asset();
    let (c, sac_c) = s.asset();
    let ab = s.aquarius(&a, &b);
    let comet = env.register(comet_mock::CometPool, ());
    sac_b.mint(&ab, &1_000);
    sac_c.mint(&comet, &1_000);
    sac_a.mint(&s.sender, &1_000);

    // Comet has no inverse quote, so the `b` it spends cannot be priced into the first hop.
    let xdr = payload(
        &env,
        &[&a, &b, &c, &ab, &comet],
        2,
        &[100, 1],
        &[[AQUARIUS, WEIGHT_1, 3, 0, 1], [COMET, WEIGHT_1, 4, 1, 2]],
    );
    s.rejects(1_000, &xdr, Error::InvalidRouteXdr);
}

#[test]
fn zero_input_quote_is_rejected() {
    let env = Env::default();
    let s = Setup::new(&env);
    let (a, sac_a) = s.asset();
    let (b, sac_b) = s.asset();
    let pool = env.register(tunable_aquarius_mock::TunableAqPool, ());
    tunable_aquarius_mock::TunableAqPoolClient::new(&env, &pool).init(&a, &b, &0, &0);
    sac_b.mint(&pool, &1_000);
    sac_a.mint(&s.sender, &1_000);

    let xdr = payload(
        &env,
        &[&a, &b, &pool],
        1,
        &[100, 1],
        &[[AQUARIUS, WEIGHT_1, 2, 0, 1]],
    );
    s.rejects(1_000, &xdr, Error::InvalidAmount);
}

#[test]
fn comet_spend_that_leaves_too_little_for_a_later_source_hop_is_rejected() {
    let env = Env::default();
    let s = Setup::new(&env);
    let (a, sac_a) = s.asset();
    let (c, sac_c) = s.asset();
    let comet = env.register(comet_mock::CometPool, ());
    let ac = s.aquarius(&a, &c);
    sac_c.mint(&comet, &1_000);
    sac_c.mint(&ac, &1_000);
    sac_a.mint(&s.sender, &120);

    // Each hop buys 50. The priced Aquarius leg needs 50, which the deposit covers, but Comet
    // runs first, may use the whole deposit, and spends 100.
    let xdr = payload(
        &env,
        &[&a, &c, &comet, &ac],
        1,
        &[100, 1],
        &[[COMET, WEIGHT_1, 2, 0, 1], [AQUARIUS, WEIGHT_1, 3, 0, 1]],
    );
    s.rejects(120, &xdr, Error::SlippageExceeded);
    assert_eq!(s.balance(&a, &s.sender), 120);
}

/// Runs one Aquarius hop that quotes `estimate` for 100 out and delivers `100 + bonus`.
fn tunable_aquarius_hop(estimate: u128, bonus: i128) {
    let env = Env::default();
    let s = Setup::new(&env);
    let (a, sac_a) = s.asset();
    let (b, sac_b) = s.asset();
    let pool = env.register(tunable_aquarius_mock::TunableAqPool, ());
    tunable_aquarius_mock::TunableAqPoolClient::new(&env, &pool).init(&a, &b, &estimate, &bonus);
    sac_b.mint(&pool, &1_000);
    sac_a.mint(&s.sender, &1_000);

    let xdr = payload(
        &env,
        &[&a, &b, &pool],
        1,
        &[100, 1],
        &[[AQUARIUS, WEIGHT_1, 2, 0, 1]],
    );
    s.rejects(1_000, &xdr, Error::SlippageExceeded);
    assert_eq!(s.balance(&a, &s.sender), 1_000);
}

#[test]
fn priced_hop_that_spends_less_than_its_quote_is_rejected() {
    tunable_aquarius_hop(110, 0);
}

#[test]
fn priced_hop_that_delivers_more_than_its_target_is_rejected() {
    tunable_aquarius_hop(100, 1);
}

#[test]
fn comet_hop_that_spends_nothing_is_rejected() {
    let env = Env::default();
    let s = Setup::new(&env);
    let (a, sac_a) = s.asset();
    let (b, sac_b) = s.asset();
    let pool = env.register(comet_mock::NoPullCometPool, ());
    sac_b.mint(&pool, &1_000);
    sac_a.mint(&s.sender, &1_000);

    let xdr = payload(
        &env,
        &[&a, &b, &pool],
        1,
        &[100, 1],
        &[[COMET, WEIGHT_1, 2, 0, 1]],
    );
    s.rejects(1_000, &xdr, Error::SlippageExceeded);
    assert_eq!(s.balance(&a, &s.sender), 1_000);
}

/// Registers a Phoenix pool over virtual reserves `reserve_in` and `reserve_out` that quotes
/// `estimate` in reverse and delivers its forward quote plus `bias`.
fn tunable_phoenix(
    s: &Setup,
    a: &Address,
    c: &Address,
    (reserve_in, reserve_out): (i128, i128),
    estimate: i128,
    bias: i128,
) -> Address {
    let pool = s.env.register(tunable_phoenix_mock::TunablePhoenix, ());
    tunable_phoenix_mock::TunablePhoenixClient::new(s.env, &pool).init(
        a,
        c,
        &reserve_in,
        &reserve_out,
        &estimate,
        &bias,
    );
    pool
}

#[test]
fn phoenix_split_rejects_an_underfilled_leg_even_when_a_sibling_overfills() {
    let env = Env::default();
    let s = Setup::new(&env);
    let (a, sac_a) = s.asset();
    let (c, sac_c) = s.asset();
    let deep = (1_000_000_000, 1_000_000_000);
    let over = tunable_phoenix(&s, &a, &c, deep, 50, 1);
    let under = tunable_phoenix(&s, &a, &c, deep, 50, -1);
    sac_c.mint(&over, &1_000);
    sac_c.mint(&under, &1_000);
    sac_a.mint(&s.sender, &1_000);

    // Together the legs deliver the 100 payout, but the second leg delivers below its 50.
    let xdr = payload(
        &env,
        &[&a, &c, &over, &under],
        1,
        &[100, 1],
        &[[PHOENIX, WEIGHT_1, 2, 0, 1], [PHOENIX, WEIGHT_1, 3, 0, 1]],
    );
    s.rejects(1_000, &xdr, Error::SlippageExceeded);
}

#[test]
fn phoenix_low_estimate_on_an_imbalanced_pool_still_buys_with_the_least_input() {
    let env = Env::default();
    let s = Setup::new(&env);
    let (a, sac_a) = s.asset();
    let (c, sac_c) = s.asset();
    // 2000 * x / (1e6 + x) first reaches 1000 at x = 1e6. The reverse estimate of 2000 quotes
    // about 3, so the router must search far above it.
    let pool = tunable_phoenix(&s, &a, &c, (1_000_000, 2_000), 2_000, 0);
    sac_c.mint(&pool, &2_000);
    sac_a.mint(&s.sender, &1_100_000);

    let xdr = payload(
        &env,
        &[&a, &c, &pool],
        1,
        &[1_000, 1],
        &[[PHOENIX, WEIGHT_1, 2, 0, 1]],
    );
    assert_eq!(
        s.router.execute_strategy(&s.sender, &1_100_000, &xdr),
        1_000
    );
    assert_eq!(s.balance(&a, &s.sender), 100_000);
    assert_eq!(s.balance(&a, &pool), 1_000_000);
    assert_eq!(s.balance(&c, &s.sender), 1_000);
}

#[test]
fn phoenix_estimate_far_above_the_least_input_is_rejected() {
    let env = Env::default();
    let s = Setup::new(&env);
    let (a, sac_a) = s.asset();
    let (c, sac_c) = s.asset();
    // 100 out needs 52_632 in. The estimate of 1e7 is more than the 2^20 - 1 units the
    // downward search covers.
    let pool = tunable_phoenix(&s, &a, &c, (1_000_000, 2_000), 10_000_000, 0);
    sac_c.mint(&pool, &2_000);
    sac_a.mint(&s.sender, &100_000);

    let xdr = payload(
        &env,
        &[&a, &c, &pool],
        1,
        &[100, 1],
        &[[PHOENIX, WEIGHT_1, 2, 0, 1]],
    );
    s.rejects(100_000, &xdr, Error::SlippageExceeded);
    assert_eq!(s.balance(&a, &s.sender), 100_000);
}
