use super::*;
use common::oracle::providers::curve::{CurvePool, CurvePoolInfo, CurvePoolScales};
use common::types::{
    AssetOracle, FeedNature, FeedSource, IndependencePolicy, MultiFeedRef, OracleTolerance,
    PriceFeedRaw, PriceSource, ProviderRef, ScaledSource, MAX_CURVE_COINS, MAX_RESOLUTION_DEPTH,
};
use soroban_sdk::{
    contract, contractimpl, contracttype,
    testutils::{Address as _, Ledger},
    token, vec, Address, BytesN, ContractExecutable, Error, String, Symbol, Vec,
};

use crate::test_support::register_redstone_feed;
use crate::{registry, PriceAggregator, PriceAggregatorClient};

#[path = "../../../swap-aggregator/tests/fixtures/curve/factory.rs"]
mod curve_fixture;

use curve_fixture::pool_abi;

struct Fixture {
    env: Env,
    aggregator: Address,
    feed: Address,
    source: CurveStableLpSource,
}

impl Fixture {
    fn new(count: u32, real: bool) -> Self {
        let env = Env::default();
        env.mock_all_auths();
        env.cost_estimate().budget().reset_unlimited();
        env.ledger()
            .with_mut(|ledger| ledger.timestamp = 1_700_000_000);
        let user = Address::generate(&env);
        let aggregator = env.register(PriceAggregator, (&user,));
        let (feed, prices) = register_redstone_feed(&env);
        prices.set_price(&String::from_str(&env, "USD"), &WAD);
        let mut coins = Vec::new(&env);
        for _ in 0..count {
            let coin = env
                .register_stellar_asset_contract_v2(user.clone())
                .address();
            token::StellarAssetClient::new(&env, &coin).mint(&user, &10_000_000_000);
            coins.push_back(coin.clone());
            PriceAggregatorClient::new(&env, &aggregator)
                .set_oracle(&PriceKey::Token(coin), &feed_oracle(&env, &feed));
        }
        let pool = if real {
            let pool = curve_fixture::deploy(&env, &coins);
            pool_abi::Client::new(&env, &pool).add_liquidity(
                &user,
                &Vec::from_iter(&env, (0..count).map(|i| (i as i128 + 1) * 1_000_000_000)),
                &0,
                &user,
            );
            pool
        } else {
            env.register(MockPool, (state(&env, &coins),))
        };
        let Some(Executable::Wasm(wasm_hash)) = pool.executable() else {
            panic!("expected WASM identity");
        };
        let source = CurveStableLpSource {
            pool,
            coins,
            wasm_hash,
            min_pool_value_wad: WAD,
        };
        Self {
            env,
            aggregator,
            feed,
            source,
        }
    }

    fn key(&self) -> PriceKey {
        PriceKey::Token(self.source.pool.clone())
    }

    fn oracle(&self) -> AssetOracle {
        let mut oracle = feed_oracle(&self.env, &self.feed);
        oracle.asset_decimals = 18;
        oracle.sources = vec![&self.env, PriceSource::CurveStableLp(self.source.clone())];
        oracle.min_sanity_price_wad = WAD * 95 / 100;
        oracle.max_sanity_price_wad = WAD * 105 / 100;
        oracle
    }

    fn register(&self) {
        PriceAggregatorClient::new(&self.env, &self.aggregator)
            .set_oracle(&self.key(), &self.oracle());
    }

    fn price(&self) -> PriceFeedRaw {
        PriceAggregatorClient::new(&self.env, &self.aggregator)
            .prices(&vec![&self.env, self.key()])
            .get(self.key())
            .unwrap()
    }

    fn read_error(&self) -> OracleError {
        self.env.as_contract(&self.aggregator, || {
            read(
                &mut Session::new(&self.env),
                &self.key(),
                &self.source,
                18,
                0,
            )
            .unwrap_err()
        })
    }
}

fn feed_oracle(env: &Env, feed: &Address) -> AssetOracle {
    AssetOracle {
        asset_decimals: 7,
        max_price_stale_seconds: 900,
        sources: vec![
            env,
            PriceSource::Feed(FeedSource {
                provider: ProviderRef::RedStone(MultiFeedRef {
                    contract: feed.clone(),
                    feed_id: String::from_str(env, "USD"),
                    nature: FeedNature::Fundamental,
                }),
                decimals: 8,
                max_stale_seconds: 900,
            }),
        ],
        tolerance: OracleTolerance {
            upper_ratio_bps: 10_500,
            lower_ratio_bps: 9_524,
        },
        independence: IndependencePolicy::RequireDisjoint,
        min_sanity_price_wad: WAD * 95 / 100,
        max_sanity_price_wad: WAD * 105 / 100,
    }
}

#[test]
fn real_two_and_three_coin_lp_prices_use_virtual_price_and_every_coin_feed() {
    for count in [2, 3] {
        let f = Fixture::new(count, true);
        f.register();
        let virtual_price = pool_abi::Client::new(&f.env, &f.source.pool).get_virtual_price();
        assert_eq!(f.price().price_wad, virtual_price);
        // Unaccounted token donations must not inflate collateral value.
        token::StellarAssetClient::new(&f.env, &f.source.coins.get_unchecked(0))
            .mint(&f.source.pool, &1_000_000_000);
        assert_eq!(f.price().price_wad, virtual_price);
        for coin in f.source.coins.iter() {
            let (feed, prices) = register_redstone_feed(&f.env);
            prices.set_price(&String::from_str(&f.env, "USD"), &(WAD * 97 / 100));
            let client = PriceAggregatorClient::new(&f.env, &f.aggregator);
            client.set_oracle(&PriceKey::Token(coin.clone()), &feed_oracle(&f.env, &feed));
            assert_eq!(f.price().price_wad, virtual_price * 97 / 100);
            client.set_oracle(&PriceKey::Token(coin), &feed_oracle(&f.env, &f.feed));
        }
    }
}

#[contracttype]
#[derive(Clone)]
struct State {
    info: CurvePoolInfo,
    price: i128,
    supply: i128,
    decimals: u32,
    /// 1 reverts `get_virtual_price`, 2 reverts `total_supply`.
    fail: u32,
}

fn state(env: &Env, coins: &Vec<Address>) -> State {
    State {
        info: CurvePoolInfo {
            coin_count: coins.len(),
            coin_decimals: Vec::from_iter(env, coins.iter().map(|_| 7)),
            coins: coins.clone(),
            factory: Address::generate(env),
            min_locked_liquidity: 1_000,
            rates: Vec::from_iter(env, coins.iter().map(|_| 10i128.pow(29))),
            scales: CurvePoolScales {
                amplification_scale: 100,
                fee_denominator: 10_000_000_000,
                precision: WAD,
            },
        },
        price: WAD + 1,
        supply: 1_000 * WAD,
        decimals: 18,
        fail: 0,
    }
}

#[contract]
struct MockPool;

#[contractimpl]
impl MockPool {
    pub fn __constructor(env: Env, state: State) {
        Self::set_state(env, state);
    }

    pub fn set_state(env: Env, state: State) {
        env.storage()
            .instance()
            .set(&Symbol::new(&env, "state"), &state);
    }

    pub fn state(env: Env) -> State {
        env.storage()
            .instance()
            .get(&Symbol::new(&env, "state"))
            .unwrap()
    }

    pub fn upgrade(env: Env, hash: BytesN<32>) {
        env.deployer()
            .update_current_contract(ContractExecutable::Wasm(hash));
    }
}

#[contractimpl]
impl CurvePool for MockPool {
    fn pool_info(env: Env) -> CurvePoolInfo {
        Self::state(env).info
    }
    fn get_virtual_price(env: Env) -> i128 {
        let state = Self::state(env);
        assert!(state.fail != 1, "virtual price reverted");
        state.price
    }
    fn total_supply(env: Env) -> i128 {
        let state = Self::state(env);
        assert!(state.fail != 2, "total supply reverted");
        state.supply
    }
    fn decimals(env: Env) -> u32 {
        Self::state(env).decimals
    }
}

#[test]
fn configuration_and_every_read_reject_binding_scale_rate_and_hash_drift() {
    let mut f = Fixture::new(3, false);
    f.register();
    let pool = MockPoolClient::new(&f.env, &f.source.pool);
    let original = pool.state();
    let client = PriceAggregatorClient::new(&f.env, &f.aggregator);
    for case in 0..8 {
        let mut changed = original.clone();
        match case {
            0 => {
                changed.info.coin_decimals.pop_back();
            }
            1 => {
                changed.info.rates.pop_back();
            }
            2 => {
                // Rate matches the claimed 6 decimals, so only the live token binding rejects.
                changed.info.coin_decimals.set(2, 6);
                changed.info.rates.set(2, 10i128.pow(30));
            }
            3 => changed.info.rates.set(2, 10i128.pow(29) + 1),
            4 => changed.info.scales.precision = 10_000_000,
            5 => changed.decimals = 7,
            6 => changed.info.coin_decimals.set(2, 19),
            _ => {
                changed
                    .info
                    .coins
                    .set(0, original.info.coins.get_unchecked(2));
                changed
                    .info
                    .coins
                    .set(2, original.info.coins.get_unchecked(0));
            }
        }
        pool.set_state(&changed);
        assert_eq!(
            f.read_error(),
            OracleError::UnsupportedCurvePool,
            "case {case}"
        );
        assert_eq!(
            client.try_set_oracle(&f.key(), &f.oracle()),
            Err(Ok(Error::from_contract_error(
                OracleError::UnsupportedCurvePool as u32
            ))),
            "case {case}"
        );
    }
    pool.set_state(&original);
    f.env.as_contract(&f.aggregator, || {
        assert!(bound_pool(&f.env, &PriceKey::Token(f.feed.clone()), &f.source, 18).is_none());
        assert!(bound_pool(&f.env, &f.key(), &f.source, 7).is_none());
    });
    // Healthy pool state, wrong pinned executable: only the hash check rejects.
    let pinned = f.source.wasm_hash.clone();
    f.source.wasm_hash = BytesN::from_array(&f.env, &[7; 32]);
    assert_eq!(f.read_error(), OracleError::UnsupportedCurvePool);
    assert_eq!(
        client.try_set_oracle(&f.key(), &f.oracle()),
        Err(Ok(Error::from_contract_error(
            OracleError::UnsupportedCurvePool as u32
        )))
    );
    f.source.wasm_hash = pinned;
    let hash = f.env.deployer().upload_contract_wasm(pool_abi::WASM);
    pool.upgrade(&hash);
    assert_eq!(f.read_error(), OracleError::UnsupportedCurvePool);
}

#[test]
fn price_and_supply_fail_closed_and_liquidity_uses_conservative_value() {
    let mut f = Fixture::new(2, false);
    f.register();
    let pool = MockPoolClient::new(&f.env, &f.source.pool);
    let original = pool.state();
    // The last pair floors to WAD - 1, one wei under the WAD liquidity floor.
    for (price, supply) in [(0, WAD), (-1, WAD), (WAD, 0), (WAD, -1), (WAD - 1, WAD + 1)] {
        let mut changed = original.clone();
        changed.price = price;
        changed.supply = supply;
        pool.set_state(&changed);
        assert_eq!(f.read_error(), OracleError::InsufficientCurveLiquidity);
    }
    for (price, supply, err) in [
        (i128::MAX, i128::MAX, OracleError::SanityBoundViolated),
        (-WAD, -2 * WAD, OracleError::InvalidPrice),
    ] {
        let mut changed = original.clone();
        changed.price = price;
        changed.supply = supply;
        pool.set_state(&changed);
        f.env.as_contract(&f.aggregator, || {
            assert_eq!(
                engine::resolve_nested(&mut Session::new(&f.env), &f.key(), 0).unwrap_err(),
                err
            );
        });
    }
    for fail in [1, 2] {
        pool.set_state(&State {
            fail,
            ..original.clone()
        });
        assert_eq!(f.read_error(), OracleError::NoLastPrice, "fail {fail}");
    }
    pool.set_state(&original);
    f.source.min_pool_value_wad = (WAD + 1) * 1_000 + 1;
    assert_eq!(f.read_error(), OracleError::InsufficientCurveLiquidity);
    f.source.min_pool_value_wad -= 1;
    f.register();
    assert_eq!(f.price().price_wad, WAD + 1);
    // The liquidity floor uses the min-coin price, not the virtual price: 970 WAD, not 1000.
    let (feed, prices) = register_redstone_feed(&f.env);
    prices.set_price(&String::from_str(&f.env, "USD"), &(WAD * 97 / 100));
    let coin = PriceKey::Token(f.source.coins.get_unchecked(0));
    PriceAggregatorClient::new(&f.env, &f.aggregator)
        .set_oracle(&coin, &feed_oracle(&f.env, &feed));
    f.source.min_pool_value_wad = 970 * WAD + 1;
    assert_eq!(f.read_error(), OracleError::InsufficientCurveLiquidity);
}

#[test]
fn bounded_coin_lists_and_single_source_policy() {
    let f = Fixture::new(MAX_CURVE_COINS, false);
    f.register();
    assert_eq!(f.price().price_wad, WAD + 1);
    let client = PriceAggregatorClient::new(&f.env, &f.aggregator);
    let rejects = |source: CurveStableLpSource, err: OracleError| {
        let mut oracle = f.oracle();
        oracle.sources = vec![&f.env, PriceSource::CurveStableLp(source)];
        assert_eq!(
            client.try_set_oracle(&f.key(), &oracle),
            Err(Ok(Error::from_contract_error(err as u32)))
        );
    };
    let with = |coins: Vec<Address>| CurveStableLpSource {
        coins,
        ..f.source.clone()
    };
    let mut coins = f.source.coins.clone();
    coins.push_back(coins.get_unchecked(0));
    rejects(with(coins), OracleError::InvalidOracleBase);
    let mut coins = vec![&f.env, f.source.coins.get_unchecked(0)];
    rejects(with(coins.clone()), OracleError::InvalidOracleBase);
    coins.push_back(f.source.pool.clone());
    rejects(with(coins), OracleError::OracleCycleDetected);
    let mut coins = f.source.coins.clone();
    coins.set(2, coins.get_unchecked(0));
    rejects(with(coins), OracleError::UnsupportedCurvePool);
    let mut source = f.source.clone();
    source.min_pool_value_wad = 0;
    rejects(source, OracleError::InvalidOracleBase);
    let mut oracle = f.oracle();
    oracle
        .sources
        .push_back(feed_oracle(&f.env, &f.feed).sources.get_unchecked(0));
    assert_eq!(
        client.try_set_oracle(&f.key(), &oracle),
        Err(Ok(Error::from_contract_error(
            OracleError::SourceCountOutOfRange as u32
        )))
    );
}

#[test]
fn third_coin_controls_freshness() {
    let f = Fixture::new(3, false);
    f.register();
    let key = PriceKey::Token(f.source.coins.get_unchecked(2));
    let (feed, prices) = register_redstone_feed(&f.env);
    let usd = String::from_str(&f.env, "USD");
    prices.set_price(&usd, &WAD);
    let client = PriceAggregatorClient::new(&f.env, &f.aggregator);
    let oracle = feed_oracle(&f.env, &feed);
    client.set_oracle(&key, &oracle);
    let now = f.env.ledger().timestamp();
    prices.set_price_data(&usd, &WAD, &((now - 200) * 1_000), &(now * 1_000));
    assert_eq!(f.price().timestamp, now - 200);
    prices.set_price_data(&usd, &WAD, &((now - 901) * 1_000), &(now * 1_000));
    assert_eq!(f.read_error(), OracleError::PriceFeedStale);
}

/// A token that reports the decimals it was built with. A Stellar asset
/// contract always reports 7.
#[contract]
struct DecimalsToken;

#[contractimpl]
impl DecimalsToken {
    pub fn __constructor(env: Env, decimals: u32) {
        env.storage()
            .instance()
            .set(&Symbol::new(&env, "decimals"), &decimals);
    }

    pub fn decimals(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&Symbol::new(&env, "decimals"))
            .unwrap()
    }
}

/// A two-coin mock pool whose second coin is a token with `decimals` decimals.
/// The pool reports the same decimals and the matching fixed rate, so the live
/// token binding and the rate check both hold and only the 18-decimal bound can
/// refuse the pool. The coin is priced by the fixture's feed; the pool binding
/// does not read that oracle.
fn fixture_with_coin_decimals(decimals: u32) -> Fixture {
    let mut f = Fixture::new(2, false);
    let coin = f.env.register(DecimalsToken, (decimals,));
    PriceAggregatorClient::new(&f.env, &f.aggregator).set_oracle(
        &PriceKey::Token(coin.clone()),
        &feed_oracle(&f.env, &f.feed),
    );
    f.source.coins.set(1, coin);
    let pool = MockPoolClient::new(&f.env, &f.source.pool);
    let mut state = pool.state();
    state.info.coins = f.source.coins.clone();
    state.info.coin_decimals.set(1, decimals);
    state.info.rates.set(1, 10i128.pow(36 - decimals));
    pool.set_state(&state);
    f
}

#[test]
fn a_coin_with_exactly_18_decimals_is_accepted() {
    let f = fixture_with_coin_decimals(18);
    f.register();
    assert_eq!(f.price().price_wad, WAD + 1);
}

#[test]
fn a_coin_with_more_than_18_decimals_is_refused_at_configuration_and_on_every_read() {
    let f = fixture_with_coin_decimals(19);
    assert_eq!(f.read_error(), OracleError::UnsupportedCurvePool);
    assert_eq!(
        PriceAggregatorClient::new(&f.env, &f.aggregator).try_set_oracle(&f.key(), &f.oracle()),
        Err(Ok(Error::from_contract_error(
            OracleError::UnsupportedCurvePool as u32
        )))
    );
}

/// Stores an oracle that scales `quote` by a factor of exactly one and returns
/// its key. It writes straight to the registry: `set_oracle` would refuse the
/// over-deep chain on its structural depth check, before the engine resolves a
/// price.
fn scaled_onto(f: &Fixture, quote: PriceKey) -> PriceKey {
    let key = PriceKey::Token(Address::generate(&f.env));
    let mut oracle = feed_oracle(&f.env, &f.feed);
    let PriceSource::Feed(factor) = oracle.sources.get_unchecked(0) else {
        panic!("feed_oracle has one feed source");
    };
    oracle.sources = vec![
        &f.env,
        PriceSource::Scaled(ScaledSource {
            factor,
            quote,
            min_factor_wad: WAD,
            max_factor_wad: WAD,
        }),
    ];
    f.env.as_contract(&f.aggregator, || {
        registry::store_oracle(&f.env, &key, &oracle);
    });
    key
}

/// Every scaled level and the Curve LP itself add one level of depth. With
/// `MAX_RESOLUTION_DEPTH - 1` scaled levels above the LP its coins sit exactly
/// at the cap and the chain prices. One more level pushes the coins past the
/// cap, and the read fails on the depth limit.
#[test]
fn a_scaled_chain_over_a_curve_lp_is_refused_once_its_coins_pass_the_depth_cap() {
    let f = Fixture::new(2, false);
    f.register();
    let mut at_the_cap = f.key();
    for _ in 1..MAX_RESOLUTION_DEPTH {
        at_the_cap = scaled_onto(&f, at_the_cap);
    }
    let past_the_cap = scaled_onto(&f, at_the_cap.clone());

    f.env.as_contract(&f.aggregator, || {
        assert_eq!(
            engine::resolve_nested(&mut Session::new(&f.env), &at_the_cap, 0)
                .unwrap()
                .price_wad,
            WAD + 1
        );
        assert_eq!(
            engine::resolve_nested(&mut Session::new(&f.env), &past_the_cap, 0).unwrap_err(),
            OracleError::OracleDepthExceeded
        );
    });
}
