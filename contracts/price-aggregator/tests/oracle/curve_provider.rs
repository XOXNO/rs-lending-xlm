use super::*;
use common::oracle::providers::curve::{CurvePool, CurvePoolInfo, CurvePoolScales};
use common::types::{
    FeedNature, FeedSource, IndependencePolicy, MultiFeedRef, OracleTolerance, PriceSource,
    ProviderRef,
};
use soroban_sdk::{
    contract, contractimpl, contracttype,
    testutils::{Address as _, Ledger},
    vec, Address, BytesN, IntoVal, String, Symbol, Vec,
};

use crate::{PriceAggregator, PriceAggregatorClient};

mod pool_abi {
    soroban_sdk::contractimport!(file = "../swap-aggregator/tests/fixtures/curve/pool-2coin.wasm");
}

#[contract]
struct Factory;

#[contractimpl]
impl Factory {
    pub fn admin(env: Env) -> Address {
        env.storage()
            .instance()
            .get(&Symbol::new(&env, "admin"))
            .unwrap()
    }

    pub fn new_pool(env: Env, hash: BytesN<32>, args: pool_abi::PoolConstructorArgs) -> Address {
        env.storage()
            .instance()
            .set(&Symbol::new(&env, "admin"), &args.factory);
        let deployer = env
            .deployer()
            .with_current_contract(BytesN::from_array(&env, &[0; 32]));
        env.authorize_as_current_contract(vec![
            &env,
            soroban_sdk::auth::InvokerContractAuthEntry::Contract(
                soroban_sdk::auth::SubContractInvocation {
                    context: soroban_sdk::auth::ContractContext {
                        contract: deployer.deployed_address(),
                        fn_name: Symbol::new(&env, "__constructor"),
                        args: (args.clone(),).into_val(&env),
                    },
                    sub_invocations: vec![&env],
                },
            ),
        ]);
        deployer.deploy_contract(soroban_sdk::ContractExecutable::Wasm(hash), (args,))
    }
}

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
        let (feed, prices) = crate::test_support::register_redstone_feed(&env);
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
            let wasm: &[u8] = if count == 2 {
                include_bytes!("../../../swap-aggregator/tests/fixtures/curve/pool-2coin.wasm")
            } else {
                include_bytes!("../../../swap-aggregator/tests/fixtures/curve/pool-3coin.wasm")
            };
            let factory = env.register(Factory, ());
            let args = pool_abi::PoolConstructorArgs {
                admin_fee: 5_000_000_000,
                amplification: 100,
                asset_types: Vec::from_iter(&env, (0..count).map(|_| 0)),
                factory: factory.clone(),
                fee: 30_000_000,
                methods: vec![&env],
                min_locked_liquidity: 1_000,
                name: String::from_str(&env, "Curve LP"),
                offpeg_fee_multiplier: 20_000_000_000,
                oracles: vec![&env],
                symbol: String::from_str(&env, "LP"),
                tokens: coins.clone(),
            };
            let hash = env.deployer().upload_contract_wasm(wasm);
            let pool = FactoryClient::new(&env, &factory).new_pool(&hash, &args);
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

    fn price(&self) -> common::types::PriceFeedRaw {
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
            let (feed, prices) = crate::test_support::register_redstone_feed(&f.env);
            prices.set_price(&String::from_str(&f.env, "USD"), &(WAD * 97 / 100));
            let client = PriceAggregatorClient::new(&f.env, &f.aggregator);
            client.set_oracle(&PriceKey::Token(coin.clone()), &feed_oracle(&f.env, &feed));
            assert_eq!(f.price().price_wad, virtual_price * 97 / 100);
            prices.set_price(&String::from_str(&f.env, "USD"), &WAD);
            assert_eq!(f.price().price_wad, virtual_price);
            client.set_oracle(&PriceKey::Token(coin), &feed_oracle(&f.env, &f.feed));
        }
        f.env.ledger().with_mut(|ledger| ledger.timestamp += 901);
        assert_eq!(f.read_error(), OracleError::PriceFeedStale);
    }
}

#[contracttype]
#[derive(Clone)]
struct State {
    info: CurvePoolInfo,
    price: i128,
    supply: i128,
    decimals: u32,
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
            .update_current_contract(soroban_sdk::ContractExecutable::Wasm(hash));
    }
}

#[contractimpl]
impl CurvePool for MockPool {
    fn pool_info(env: Env) -> CurvePoolInfo {
        Self::state(env).info
    }
    fn get_virtual_price(env: Env) -> i128 {
        Self::state(env).price
    }
    fn total_supply(env: Env) -> i128 {
        Self::state(env).supply
    }
    fn decimals(env: Env) -> u32 {
        Self::state(env).decimals
    }
}

#[test]
fn configuration_and_every_read_reject_binding_scale_rate_and_hash_drift() {
    let f = Fixture::new(3, false);
    f.register();
    let pool = MockPoolClient::new(&f.env, &f.source.pool);
    let original = pool.state();
    for case in 0..10 {
        let mut changed = original.clone();
        match case {
            0 => changed
                .info
                .coins
                .set(2, changed.info.coins.get_unchecked(0)),
            1 => changed.info.coin_count = 2,
            2 => {
                changed.info.coin_decimals.pop_back();
            }
            3 => {
                changed.info.rates.pop_back();
            }
            4 => changed.info.coin_decimals.set(2, 6),
            5 => changed.info.rates.set(2, 10i128.pow(29) + 1),
            6 => changed.info.scales.precision = 10_000_000,
            7 => changed.decimals = 7,
            8 => changed.info.coin_decimals.set(2, 19),
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
        assert!(PriceAggregatorClient::new(&f.env, &f.aggregator)
            .try_set_oracle(&f.key(), &f.oracle())
            .is_err());
    }
    pool.set_state(&original);
    let mut wrong = f.source.clone();
    wrong.wasm_hash = BytesN::from_array(&f.env, &[0xAA; 32]);
    f.env.as_contract(&f.aggregator, || {
        assert_eq!(
            read(&mut Session::new(&f.env), &f.key(), &wrong, 18, 0).unwrap_err(),
            OracleError::UnsupportedCurvePool,
        );
        assert!(bound_pool(&f.env, &PriceKey::Token(f.feed.clone()), &f.source, 18).is_none());
        assert!(bound_pool(&f.env, &f.key(), &f.source, 7).is_none());
    });
    let hash = f.env.deployer().upload_contract_wasm(pool_abi::WASM);
    pool.upgrade(&hash);
    assert_eq!(f.read_error(), OracleError::UnsupportedCurvePool);
}

#[test]
fn price_and_supply_fail_closed_and_liquidity_uses_conservative_value() {
    let mut f = Fixture::new(2, false);
    let pool = MockPoolClient::new(&f.env, &f.source.pool);
    let original = pool.state();
    for (price, supply) in [
        (0, WAD),
        (-1, WAD),
        (WAD, 0),
        (WAD, -1),
        (i128::MAX, i128::MAX),
    ] {
        let mut changed = original.clone();
        changed.price = price;
        changed.supply = supply;
        pool.set_state(&changed);
        assert_eq!(f.read_error(), OracleError::InvalidPrice);
    }
    pool.set_state(&original);
    f.source.min_pool_value_wad = (WAD + 1) * 1_000 + 1;
    assert_eq!(f.read_error(), OracleError::InsufficientCurveLiquidity);
    f.source.min_pool_value_wad -= 1;
    f.register();
    assert_eq!(f.price().price_wad, WAD + 1);
    assert_eq!(floor_product(&f.env, WAD + 1, WAD - 1).unwrap(), WAD - 1);
}

#[test]
fn bounded_coin_lists_and_single_source_policy() {
    let f = Fixture::new(8, false);
    f.register();
    assert_eq!(f.price().price_wad, WAD + 1);
    let mut source = f.source.clone();
    source.coins.push_back(Address::generate(&f.env));
    assert!(!shape(&source));
    source.coins = vec![&f.env, Address::generate(&f.env)];
    assert!(!shape(&source));
    source.coins.push_back(source.pool.clone());
    assert!(!shape(&source));
    source = f.source.clone();
    source.coins.set(7, source.coins.get_unchecked(0));
    assert!(!shape(&source));
    let mut oracle = f.oracle();
    oracle
        .sources
        .push_back(feed_oracle(&f.env, &f.feed).sources.get_unchecked(0));
    assert!(PriceAggregatorClient::new(&f.env, &f.aggregator)
        .try_set_oracle(&f.key(), &oracle)
        .is_err());
}

#[test]
fn third_coin_controls_freshness_and_dependency_revalidation() {
    let f = Fixture::new(3, false);
    f.register();
    let key = PriceKey::Token(f.source.coins.get_unchecked(2));
    let (feed, prices) = crate::test_support::register_redstone_feed(&f.env);
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
    prices.set_price(&usd, &WAD);

    let mut changed = oracle.clone();
    changed.max_price_stale_seconds = 1_800;
    let PriceSource::Feed(mut source) = changed.sources.get_unchecked(0) else {
        unreachable!()
    };
    source.max_stale_seconds = 1_800;
    changed.sources.set(0, PriceSource::Feed(source));
    assert!(client.try_set_oracle(&key, &changed).is_err());
    assert_eq!(client.oracle(&key).unwrap(), oracle);

    let PriceSource::Feed(factor) = oracle.sources.get_unchecked(0) else {
        unreachable!()
    };
    changed = oracle.clone();
    changed.sources = vec![
        &f.env,
        PriceSource::Scaled(common::types::ScaledSource {
            factor,
            quote: f.key(),
            min_factor_wad: WAD * 95 / 100,
            max_factor_wad: WAD * 105 / 100,
        }),
    ];
    assert!(client.try_set_oracle(&key, &changed).is_err());
    client.remove_oracle(&key);
    assert_eq!(f.read_error(), OracleError::OracleNotConfigured);
}

#[test]
fn cached_lp_still_walks_the_third_coin_for_depth_and_cycles() {
    let f = Fixture::new(3, false);
    f.register();
    f.env.as_contract(&f.aggregator, || {
        let mut session = Session::new(&f.env);
        engine::resolve_nested(&mut session, &f.key(), 0).unwrap();
        let mut third = feed_oracle(&f.env, &f.feed);
        let PriceSource::Feed(factor) = third.sources.get_unchecked(0) else {
            unreachable!()
        };
        let mut scaled = common::types::ScaledSource {
            factor,
            quote: PriceKey::Token(f.source.coins.get_unchecked(0)),
            min_factor_wad: WAD * 95 / 100,
            max_factor_wad: WAD * 105 / 100,
        };
        let key = PriceKey::Token(f.source.coins.get_unchecked(2));
        third.sources = vec![&f.env, PriceSource::Scaled(scaled.clone())];
        crate::registry::store_oracle(&f.env, &key, &third);
        assert_eq!(
            engine::resolve_nested(
                &mut session,
                &f.key(),
                common::types::MAX_RESOLUTION_DEPTH - 1
            )
            .unwrap_err(),
            OracleError::OracleDepthExceeded,
        );
        scaled.quote = f.key();
        third.sources = vec![&f.env, PriceSource::Scaled(scaled)];
        crate::registry::store_oracle(&f.env, &key, &third);
        assert_eq!(
            engine::resolve_nested(&mut session, &f.key(), 0).unwrap_err(),
            OracleError::OracleCycleDetected,
        );
    });
}
