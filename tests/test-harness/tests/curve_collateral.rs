#![cfg(test)]
//! Curve LP collateral against the production price-aggregator and router WASMs
//! and the real Curve pool executables. Mainnet per-invocation resource limits
//! stay enforced, and `Scene::metered` runs a call under the default Soroban
//! budget. The controller and the leaf price feed are native, so their own
//! instructions are not metered. The figures are a lower bound: SAC coins,
//! single-source coin oracles and one debt leg. Custom-WASM coins or feeds need
//! re-measuring.

#[path = "../../../contracts/swap-aggregator/tests/fixtures/curve/factory.rs"]
mod curve;

use common::{
    constants::{POSITION_LIMIT_MAX, WAD},
    errors::OracleError,
    types::{
        AssetOracle, CurveStableLpSource, FeedNature, FeedSource, IndependencePolicy, MultiFeedRef,
        OracleTolerance, PositionMode, PriceKey, PriceSource, ProviderRef, SeizeMode,
    },
};
use curve::pool_abi;
use governance::op::{AdminOperation, CreatePoolArgs, SpokeAssetArgs};
use soroban_sdk::{
    contracttype, testutils::Address as _, token, vec, xdr::ToXdr, Address, Bytes, Env, Error,
    Executable, String, Vec,
};
use test_harness::{
    hub_asset,
    mock_redstone::{MockRedStonePriceFeed, MockRedStonePriceFeedClient},
    usdc_preset, LendingTest, DEFAULT_MARKET_PARAMS, HARNESS_HUB, HARNESS_SPOKE,
};

/// 1 000 units of a 7-decimal coin: each holder's balance and each pool seed.
const COIN: i128 = 10_000_000_000;

/// Reads a WASM that `make build` writes; `var` overrides its path.
fn release_wasm(var: &str, name: &str) -> std::vec::Vec<u8> {
    let path = std::env::var(var).unwrap_or_else(|_| {
        format!(
            "{}/../../target/wasm32v1-none/release/{name}.wasm",
            env!("CARGO_MANIFEST_DIR")
        )
    });
    std::fs::read(&path).unwrap_or_else(|_| panic!("{path} not found; run `make build` first"))
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

/// Mirror of the router's `StrategyPayload`, which encodes as the same XDR map.
#[contracttype]
struct StrategyPayload {
    amounts: Vec<i128>,
    assets: Vec<Address>,
    ops: Bytes,
}

/// One opcode-7 (Curve) hop from `token_in` to `token_out` with a floor of one.
/// One side is the LP token, which is also the pool: an LP output mints, an LP
/// input burns to one coin.
fn curve_route(env: &Env, lp: &Address, token_in: &Address, token_out: &Address) -> Bytes {
    let pool = u8::from(token_in != lp);
    StrategyPayload {
        amounts: vec![env, 1],
        assets: vec![env, token_in.clone(), token_out.clone()],
        // Version 1, assets 0 -> 1, floor amounts[0], referral 0, one op, no weights;
        // then opcode 7 in mode All over (pool, token_in, token_out).
        ops: Bytes::from_array(env, &[1, 0, 1, 0, 0, 0, 0, 0, 1, 0, 7, 0, pool, 0, 1]),
    }
    .to_xdr(env)
}

struct Scene {
    t: LendingTest,
    feed: Address,
}

impl Scene {
    /// USDC market whose oracle now lives on the production price-aggregator WASM.
    /// Setup runs unmetered; mainnet resource limits stay enforced per invocation.
    fn new() -> Self {
        let mut t = LendingTest::new()
            .with_market(usdc_preset())
            .with_budget_enabled()
            .build();
        t.env.cost_estimate().budget().reset_unlimited();
        let usdc = t.resolve_asset("USDC");
        let usdc_oracle = t.market_oracle_config(&usdc);
        let wasm = release_wasm("PRICE_AGGREGATOR_WASM_PATH", "price_aggregator");
        let aggregator = t.env.register(wasm.as_slice(), (&t.governance,));
        t.gov_client().set_price_aggregator(&aggregator);
        t.gov_client().execute_immediate(
            &t.admin,
            &AdminOperation::SetPriceAggregator(aggregator.clone()),
        );
        t.price_aggregator = aggregator;
        t.configure_market_oracle(&usdc, &usdc_oracle);
        let feed = t.env.register(MockRedStonePriceFeed, ());
        Self { t, feed }
    }

    fn prices(&self) -> MockRedStonePriceFeedClient<'_> {
        MockRedStonePriceFeedClient::new(&self.t.env, &self.feed)
    }

    fn feed_id(&self, n: u32) -> String {
        String::from_str(&self.t.env, &format!("coin-{n}"))
    }

    /// A 7-decimal coin priced at $1 by feed `coin-{n}`, with `COIN` minted to `holder`.
    fn coin(&self, n: u32, holder: &Address) -> Address {
        let env = &self.t.env;
        let coin = env
            .register_stellar_asset_contract_v2(self.t.admin.clone())
            .address();
        token::StellarAssetClient::new(env, &coin).mint(holder, &COIN);
        self.prices().set_price(&self.feed_id(n), &WAD);
        self.t
            .configure_market_oracle(&coin, &feed_oracle(env, &self.feed, &self.feed_id(n)));
        coin
    }

    /// Deploys a real Curve pool over `coins`, seeds it with `COIN` of each from
    /// `holder`, and lists its LP as collateral-only.
    fn list_pool(&self, coins: &Vec<Address>, holder: &Address) -> Address {
        let env = &self.t.env;
        let lp = curve::deploy(env, coins);
        pool_abi::Client::new(env, &lp).add_liquidity(
            holder,
            &Vec::from_iter(env, coins.iter().map(|_| COIN)),
            &0,
            holder,
        );
        let Some(Executable::Wasm(wasm_hash)) = lp.executable() else {
            panic!("Curve pool must run a WASM executable");
        };
        let mut oracle = feed_oracle(env, &self.feed, &self.feed_id(0));
        oracle.asset_decimals = 18;
        oracle.min_sanity_price_wad = WAD * 85 / 100;
        oracle.sources = vec![
            env,
            PriceSource::CurveStableLp(CurveStableLpSource {
                pool: lp.clone(),
                coins: coins.clone(),
                wasm_hash,
                min_pool_value_wad: 100 * WAD,
            }),
        ];
        self.t.configure_market_oracle(&lp, &oracle);
        let gov = self.t.gov_client();
        gov.execute_immediate(
            &self.t.admin,
            &AdminOperation::CreateLiquidityPool(CreatePoolArgs {
                hub_id: HARNESS_HUB,
                asset: lp.clone(),
                params: DEFAULT_MARKET_PARAMS.to_market_params(&lp, 18),
            }),
        );
        gov.execute_immediate(
            &self.t.admin,
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
        lp
    }

    /// Runs `call` under the default Soroban budget, prints what it used, and
    /// restores the unmetered budget.
    fn metered<T>(&self, label: &str, call: impl FnOnce() -> T) -> T {
        let mut budget = self.t.env.cost_estimate().budget();
        budget.reset_default();
        // Only the test env's auth snapshots and diagnostics run in shadow mode,
        // which a signed transaction does not pay; execution stays bounded.
        self.t
            .env
            .host()
            .set_shadow_budget_limits(u64::MAX, u64::MAX)
            .unwrap();
        let result = call();
        std::println!(
            "{label}: cpu={} mem={}",
            budget.cpu_instruction_cost(),
            budget.memory_bytes_cost()
        );
        budget.reset_unlimited();
        result
    }
}

#[test]
fn curve_three_coin_collateral_lifecycle() {
    let s = Scene::new();
    let (t, env) = (&s.t, &s.t.env);
    let borrower = Address::generate(env);
    let liquidator = Address::generate(env);
    let coins = Vec::from_iter(env, (0..3).map(|n| s.coin(n, &borrower)));
    let lp = s.list_pool(&coins, &borrower);
    let curve = pool_abi::Client::new(env, &lp);

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
    s.prices()
        .set_price_data(&s.feed_id(2), &WAD, &stale_ms, &stale_ms);
    let key = PriceKey::Token(lp.clone());
    let status = t
        .price_agg_client()
        .quotes(&vec![env, key.clone()])
        .get(key)
        .unwrap();
    assert_eq!(status.error_code, Some(OracleError::PriceFeedStale as u32));
    assert_eq!(
        ctrl.try_get_health_factor(&account),
        Err(Ok(Error::from_contract_error(
            OracleError::PriceFeedStale as u32
        )))
    );
    s.prices().set_price(&s.feed_id(2), &(WAD * 9 / 10));
    let depegged = ctrl.get_health_factor(&account);
    assert!(depegged < WAD);
    assert!(depegged < healthy);
    assert_eq!(
        ctrl.get_total_collateral_usd(&account),
        collateral_before * 9 / 10
    );

    let debt_before = ctrl.get_total_borrow_usd(&account);
    token::StellarAssetClient::new(env, &debt_asset).mint(&liquidator, &COIN);
    ctrl.liquidate(
        &liquidator,
        &account,
        &vec![env, (debt.clone(), COIN)],
        &SeizeMode::Transfer,
    );
    let seized = lp_token.balance(&liquidator);
    assert!(seized > 0 && seized < deposit);
    assert_eq!(lp_token.balance(&vault), deposit - seized);
    assert!(ctrl.get_total_borrow_usd(&account) < debt_before);

    if ctrl.get_total_borrow_usd(&account) > 0 {
        token::StellarAssetClient::new(env, &debt_asset).mint(&borrower, &COIN);
        ctrl.repay(&borrower, &account, &vec![env, (debt, COIN)]);
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

/// CURVE-6: liquidating an account whose `POSITION_LIMIT_MAX` collateral legs
/// are all three-coin Curve LP, over fifteen distinct coins, fits one invocation.
/// One debt leg only, so this is a lower bound on the worst case.
#[test]
fn curve_lp_liquidation_at_position_limit_fits_default_budget() {
    let s = Scene::new();
    let (t, env) = (&s.t, &s.t.env);
    let borrower = Address::generate(env);
    let liquidator = Address::generate(env);
    let deposit = 1_000 * WAD;
    let mut legs = Vec::new(env);
    let mut lps = std::vec::Vec::new();
    for leg in 0..POSITION_LIMIT_MAX {
        let coins = Vec::from_iter(env, (0..3).map(|i| s.coin(leg * 3 + i, &borrower)));
        let lp = s.list_pool(&coins, &borrower);
        legs.push_back((hub_asset(lp.clone()), deposit));
        lps.push(lp);
    }
    let ctrl = t.ctrl_client();
    let account = ctrl.supply(&borrower, &0, &HARNESS_SPOKE, &legs);
    assert_eq!(
        ctrl.get_account_positions(&account).0.len(),
        POSITION_LIMIT_MAX
    );
    let usdc = t.resolve_asset("USDC");
    let debt = hub_asset(usdc.clone());
    ctrl.borrow(
        &borrower,
        &account,
        &vec![env, (debt.clone(), 3_700 * 10_000_000)],
        &None,
    );
    for n in 0..POSITION_LIMIT_MAX * 3 {
        s.prices().set_price(&s.feed_id(n), &(WAD * 9 / 10));
    }
    assert!(ctrl.get_health_factor(&account) < WAD);

    let debt_before = ctrl.get_total_borrow_usd(&account);
    token::StellarAssetClient::new(env, &usdc).mint(&liquidator, &COIN);
    s.metered("liquidate 5 Curve LP legs", || {
        ctrl.liquidate(
            &liquidator,
            &account,
            &vec![env, (debt, COIN)],
            &SeizeMode::Transfer,
        )
    });
    assert!(ctrl.get_total_borrow_usd(&account) < debt_before);
    assert!(lps
        .iter()
        .any(|lp| token::Client::new(env, lp).balance(&liquidator) > 0));
}

/// CRV-RT-08: multiply USDC debt into Curve LP through the production router's
/// opcode 7 (LP mint), then repay the debt by burning that LP to USDC.
#[test]
fn curve_lp_multiply_and_repay_through_router_fit_default_budget() {
    let s = Scene::new();
    let (t, env) = (&s.t, &s.t.env);
    let (holder, user) = (Address::generate(env), Address::generate(env));
    let usdc = t.resolve_asset("USDC");
    let usdc_admin = &t.resolve_market("USDC").token_admin;
    usdc_admin.mint(&holder, &COIN);
    usdc_admin.mint(&user, &COIN);
    let coins = vec![env, usdc.clone(), s.coin(0, &holder), s.coin(1, &holder)];
    let lp = s.list_pool(&coins, &holder);
    let wasm = release_wasm("SWAP_AGGREGATOR_WASM_PATH", "swap_aggregator");
    let router = env.register(wasm.as_slice(), (&t.admin,));
    let ctrl = t.ctrl_client();
    ctrl.set_swap_aggregator(&router);
    let (collateral, debt) = (hub_asset(lp.clone()), hub_asset(usdc.clone()));
    let lp_token = token::Client::new(env, &lp);
    let vault = ctrl.get_pool_address();

    let account = s.metered("multiply USDC into Curve LP", || {
        ctrl.multiply(
            &user,
            &0,
            &HARNESS_SPOKE,
            &collateral,
            &COIN,
            &debt,
            &PositionMode::Multiply,
            &curve_route(env, &lp, &usdc, &lp),
            &Some((debt.clone(), COIN)),
            &None,
        )
    });
    let minted = lp_token.balance(&vault);
    assert!(minted > 0);
    assert!(ctrl.get_total_borrow_usd(&account) > 0);

    s.metered("repay USDC debt by burning Curve LP", || {
        ctrl.repay_debt_with_collateral(
            &user,
            &account,
            &collateral,
            &(minted * 6 / 10),
            &debt,
            &curve_route(env, &lp, &lp, &usdc),
            &true,
        )
    });
    assert_eq!(ctrl.get_total_borrow_usd(&account), 0);
    assert_eq!(lp_token.balance(&vault), 0);
    assert!(lp_token.balance(&user) > 0);
    for asset in [&lp, &usdc] {
        assert_eq!(token::Client::new(env, asset).balance(&router), 0);
    }
}
