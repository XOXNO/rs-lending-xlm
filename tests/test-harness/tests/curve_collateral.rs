#![cfg(test)]

use common::{
    constants::WAD,
    errors::OracleError,
    types::{
        AssetOracle, CurveStableLpSource, FeedNature, FeedSource, IndependencePolicy, MultiFeedRef,
        OracleTolerance, PriceKey, PriceSource, ProviderRef, SeizeMode,
    },
};
use governance::op::{AdminOperation, ConfigureAssetOracleArgs, CreatePoolArgs, SpokeAssetArgs};
use soroban_sdk::{
    contract, contractimpl, testutils::Address as _, token, vec, Address, BytesN, Env, Executable,
    IntoVal, String, Symbol, Vec,
};
use test_harness::{
    hub_asset,
    mock_redstone::{MockRedStonePriceFeed, MockRedStonePriceFeedClient},
    usdc_preset, LendingTest, DEFAULT_MARKET_PARAMS, HARNESS_HUB, HARNESS_SPOKE,
};

mod pool_abi {
    soroban_sdk::contractimport!(
        file = "../../contracts/swap-aggregator/tests/fixtures/curve/pool-2coin.wasm"
    );
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

fn feed_oracle(env: &Env, feed: &Address, id: &String) -> AssetOracle {
    AssetOracle {
        asset_decimals: 7,
        max_price_stale_seconds: 900,
        sources: vec![
            env,
            PriceSource::Feed(FeedSource {
                provider: ProviderRef::RedStone(MultiFeedRef {
                    contract: feed.clone(),
                    feed_id: id.clone(),
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
        min_sanity_price_wad: WAD * 9 / 10,
        max_sanity_price_wad: WAD * 11 / 10,
    }
}

fn exercise_collateral(count: u32) {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    let usdc = t.resolve_asset("USDC");
    let usdc_oracle = t.market_oracle_config(&usdc);
    let path = std::env::var("PRICE_AGGREGATOR_WASM_PATH").unwrap_or_else(|_| {
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../target/wasm32v1-none/release/price_aggregator.wasm"
        )
        .into()
    });
    let wasm = std::fs::read(path).expect("Build price-aggregator WASM before this test");
    let aggregator = t.env.register(wasm.as_slice(), (&t.governance,));
    t.gov_client().set_price_aggregator(&aggregator);
    t.gov_client().execute_immediate(
        &t.admin,
        &AdminOperation::SetPriceAggregator(aggregator.clone()),
    );
    t.price_aggregator = aggregator;
    t.gov_client().execute_immediate(
        &t.admin,
        &AdminOperation::ConfigureAssetOracle(ConfigureAssetOracleArgs {
            key: PriceKey::Token(usdc),
            oracle: usdc_oracle,
        }),
    );
    let env = &t.env;
    let borrower = Address::generate(env);
    let liquidator = Address::generate(env);
    let feed = env.register(MockRedStonePriceFeed, ());
    let prices = MockRedStonePriceFeedClient::new(env, &feed);
    let gov = t.gov_client();
    let mut coins = Vec::new(env);
    let mut feed_ids = Vec::new(env);
    for i in 0..count {
        let coin = env
            .register_stellar_asset_contract_v2(t.admin.clone())
            .address();
        token::StellarAssetClient::new(env, &coin).mint(&borrower, &10_000_000_000);
        let id = String::from_str(env, &format!("coin-{i}"));
        prices.set_price(&id, &WAD);
        gov.execute_immediate(
            &t.admin,
            &AdminOperation::ConfigureAssetOracle(ConfigureAssetOracleArgs {
                key: PriceKey::Token(coin.clone()),
                oracle: feed_oracle(env, &feed, &id),
            }),
        );
        coins.push_back(coin);
        feed_ids.push_back(id);
    }
    let wasm: &[u8] = if count == 2 {
        include_bytes!("../../../contracts/swap-aggregator/tests/fixtures/curve/pool-2coin.wasm")
    } else {
        include_bytes!("../../../contracts/swap-aggregator/tests/fixtures/curve/pool-3coin.wasm")
    };
    let factory = env.register(Factory, ());
    let hash = env.deployer().upload_contract_wasm(wasm);
    let lp = FactoryClient::new(env, &factory).new_pool(
        &hash,
        &pool_abi::PoolConstructorArgs {
            admin_fee: 5_000_000_000,
            amplification: 100,
            asset_types: Vec::from_iter(env, (0..count).map(|_| 0)),
            factory: factory.clone(),
            fee: 30_000_000,
            methods: vec![env],
            min_locked_liquidity: 1_000,
            name: String::from_str(env, "Curve LP"),
            offpeg_fee_multiplier: 20_000_000_000,
            oracles: vec![env],
            symbol: String::from_str(env, "LP"),
            tokens: coins.clone(),
        },
    );
    let curve = pool_abi::Client::new(env, &lp);
    curve.add_liquidity(
        &borrower,
        &Vec::from_iter(env, (0..count).map(|_| 10_000_000_000i128)),
        &0,
        &borrower,
    );
    assert_eq!(lp.executable(), Some(Executable::Wasm(hash.clone())));
    let mut oracle = feed_oracle(env, &feed, &feed_ids.get_unchecked(0));
    oracle.asset_decimals = 18;
    oracle.min_sanity_price_wad = WAD * 85 / 100;
    oracle.sources = vec![
        env,
        PriceSource::CurveStableLp(CurveStableLpSource {
            pool: lp.clone(),
            coins,
            wasm_hash: hash,
            min_pool_value_wad: 100 * WAD,
        }),
    ];
    gov.execute_immediate(
        &t.admin,
        &AdminOperation::ConfigureAssetOracle(ConfigureAssetOracleArgs {
            key: PriceKey::Token(lp.clone()),
            oracle,
        }),
    );
    gov.execute_immediate(
        &t.admin,
        &AdminOperation::CreateLiquidityPool(CreatePoolArgs {
            hub_id: HARNESS_HUB,
            asset: lp.clone(),
            params: DEFAULT_MARKET_PARAMS.to_market_params(&lp, 18),
        }),
    );
    gov.execute_immediate(
        &t.admin,
        &AdminOperation::AddAssetToSpoke(SpokeAssetArgs {
            hub_id: HARNESS_HUB,
            asset: lp.clone(),
            spoke_id: HARNESS_SPOKE,
            can_collateral: true,
            can_borrow: false,
            paused: false,
            frozen: false,
            no_seize: false,
            ltv: 7_500,
            threshold: 8_000,
            bonus: 500,
            liquidation_fees: 0,
            supply_cap: 1_000_000 * WAD,
            borrow_cap: 0,
        }),
    );

    let ctrl = t.ctrl_client();
    let vault = ctrl.get_pool_address();
    let lp_token = token::Client::new(env, &lp);
    let lp_before = lp_token.balance(&borrower);
    let supply_before = curve.total_supply();
    let deposit = 1_000 * WAD;
    let account = ctrl.supply(
        &borrower,
        &0,
        &HARNESS_SPOKE,
        &vec![env, (hub_asset(lp.clone()), deposit)],
    );
    assert_eq!(lp_token.balance(&vault), deposit);
    assert_eq!(lp_token.balance(&borrower), lp_before - deposit);
    let debt_asset = t.resolve_asset("USDC");
    let debt = hub_asset(debt_asset.clone());
    ctrl.borrow(
        &borrower,
        &account,
        &vec![env, (debt.clone(), 740 * 10_000_000)],
        &None,
    );
    let healthy = ctrl.get_health_factor(&account);
    assert!(healthy > WAD);
    let collateral_before = ctrl.get_total_collateral_usd(&account);

    let stale_ms = (env.ledger().timestamp() - 901) * 1_000;
    prices.set_price_data(
        &feed_ids.get_unchecked(count - 1),
        &WAD,
        &stale_ms,
        &stale_ms,
    );
    let key = PriceKey::Token(lp.clone());
    let status = t
        .price_agg_client()
        .quotes(&vec![env, key.clone()])
        .get(key)
        .unwrap();
    assert_eq!(status.error_code, Some(OracleError::PriceFeedStale as u32));
    assert!(ctrl.try_get_health_factor(&account).is_err());
    prices.set_price(&feed_ids.get_unchecked(count - 1), &(WAD * 9 / 10));
    let depegged = ctrl.get_health_factor(&account);
    assert!(depegged < WAD);
    assert!(depegged < healthy);
    assert_eq!(
        ctrl.get_total_collateral_usd(&account),
        collateral_before * 9 / 10
    );

    let debt_before = ctrl.get_total_borrow_usd(&account);
    token::StellarAssetClient::new(env, &debt_asset).mint(&liquidator, &10_000_000_000);
    ctrl.liquidate(
        &liquidator,
        &account,
        &vec![env, (debt.clone(), 1_000 * 10_000_000)],
        &SeizeMode::Transfer,
    );
    let seized = lp_token.balance(&liquidator);
    assert!(seized > 0 && seized < deposit);
    assert_eq!(lp_token.balance(&vault), deposit - seized);
    assert!(ctrl.get_total_borrow_usd(&account) < debt_before);

    if ctrl.get_total_borrow_usd(&account) > 0 {
        token::StellarAssetClient::new(env, &debt_asset).mint(&borrower, &10_000_000_000);
        ctrl.repay(&borrower, &account, &vec![env, (debt, 1_000 * 10_000_000)]);
        assert_eq!(ctrl.get_total_borrow_usd(&account), 0);
    }
    let before_withdraw = lp_token.balance(&borrower);
    let withdrawals = ctrl.withdraw(
        &borrower,
        &account,
        &vec![env, (hub_asset(lp.clone()), 0)],
        &None,
    );
    let received = withdrawals.get_unchecked(0).1;
    assert!(received > 0);
    assert_eq!(lp_token.balance(&borrower), before_withdraw + received);
    assert_eq!(lp_token.balance(&vault), 0);
    assert_eq!(lp_token.balance(&borrower) + seized, lp_before);
    assert_eq!(curve.total_supply(), supply_before);
}

#[test]
fn curve_two_coin_collateral_lifecycle() {
    exercise_collateral(2);
}

#[test]
fn curve_three_coin_collateral_lifecycle() {
    exercise_collateral(3);
}
