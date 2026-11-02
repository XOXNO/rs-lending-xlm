#![cfg(test)]
//! Curve LP collateral against the production price-aggregator and router WASMs
//! and the real Curve pool executables. Mainnet per-invocation resource limits
//! stay enforced, and `Scene::metered` runs a call under the default Soroban
//! budget. The controller and the leaf price feed are native, so their own
//! instructions are not metered. The figures are a lower bound: SAC coins,
//! single-source coin oracles and one debt leg.
//! `curve_lp_liquidation_worst_case_all_wasm` measures the worst case with every
//! contract on the path as WASM, custom-WASM coins and two-source WASM feeds.

#[path = "../../../contracts/swap-aggregator/tests/fixtures/curve/factory.rs"]
mod curve;

use common::{
    constants::{POSITION_LIMIT_MAX, WAD},
    errors::OracleError,
    oracle::providers::chainlink::CHAINLINK_DECIMALS,
    types::{
        AssetOracle, ChainlinkFeedRef, CurveStableLpSource, FeedNature, FeedSource,
        IndependencePolicy, MultiFeedRef, OracleTolerance, PositionMode, PriceKey, PriceSource,
        ProviderRef, SeizeMode,
    },
};
use controller::ControllerClient;
use curve::pool_abi;
use governance::op::{AdminOperation, CreatePoolArgs, SpokeAssetArgs};
use soroban_sdk::{
    contractclient, contracttype,
    testutils::{budget::ContractCostType, Address as _, Ledger, MockAuthInvoke},
    token, vec, xdr,
    xdr::ToXdr,
    Address, Bytes, BytesN, ContractExecutable, Env, Error, Executable, IntoVal, String,
    TryIntoVal, Vec,
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

/// The Chainlink setter of `mock_redstone.wasm`, which the harness's native
/// RedStone mock lacks.
#[contractclient(name = "ChainlinkMockClient")]
#[allow(dead_code)]
trait ChainlinkMock {
    fn set_chainlink_price(env: Env, data_id: BytesN<32>, price_wad: i128);
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
    /// Chainlink anchor feed. `None` keeps coin oracles single-source.
    anchor: Option<Address>,
    /// Custom token WASM for coins. `None` makes them SACs.
    token_wasm: Option<std::vec::Vec<u8>>,
}

impl Scene {
    /// USDC market whose oracle now lives on the production price-aggregator WASM.
    /// Setup runs unmetered; mainnet resource limits stay enforced per invocation.
    fn new() -> Self {
        let t = LendingTest::new()
            .with_market(usdc_preset())
            .with_budget_enabled()
            .build();
        let feed = t.env.register(MockRedStonePriceFeed, ());
        Self::with_aggregator_wasm(t, feed, None, None)
    }

    /// Like `new`, but each coin is a `production_fixture.wasm` token priced by
    /// a RedStone primary and a Chainlink anchor, both `mock_redstone.wasm`.
    /// Setup runs unmetered and without resource limits.
    fn wasm() -> Self {
        let t = LendingTest::new().with_market(usdc_preset()).build();
        let feed_wasm = release_wasm("MOCK_REDSTONE_WASM_PATH", "mock_redstone");
        let feed = t.env.register(feed_wasm.as_slice(), ());
        let anchor = t.env.register(feed_wasm.as_slice(), ());
        let token_wasm = release_wasm("PRODUCTION_FIXTURE_WASM_PATH", "production_fixture");
        Self::with_aggregator_wasm(t, feed, Some(anchor), Some(token_wasm))
    }

    fn with_aggregator_wasm(
        mut t: LendingTest,
        feed: Address,
        anchor: Option<Address>,
        token_wasm: Option<std::vec::Vec<u8>>,
    ) -> Self {
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
        Self {
            t,
            feed,
            anchor,
            token_wasm,
        }
    }

    fn prices(&self) -> MockRedStonePriceFeedClient<'_> {
        MockRedStonePriceFeedClient::new(&self.t.env, &self.feed)
    }

    fn feed_id(&self, n: u32) -> String {
        String::from_str(&self.t.env, &format!("coin-{n}"))
    }

    fn data_id(&self, n: u32) -> BytesN<32> {
        let mut id = [0; 32];
        id[28..].copy_from_slice(&n.to_be_bytes());
        BytesN::from_array(&self.t.env, &id)
    }

    /// Sets coin `n`'s price on its RedStone feed and, if present, its Chainlink anchor.
    fn set_price(&self, n: u32, price_wad: i128) {
        self.prices().set_price(&self.feed_id(n), &price_wad);
        if let Some(anchor) = &self.anchor {
            ChainlinkMockClient::new(&self.t.env, anchor)
                .set_chainlink_price(&self.data_id(n), &price_wad);
        }
    }

    /// A 7-decimal coin priced at $1 by feed `coin-{n}` (plus its Chainlink
    /// anchor, if any), with `COIN` minted to `holder`.
    fn coin(&self, n: u32, holder: &Address) -> Address {
        let env = &self.t.env;
        let coin = match &self.token_wasm {
            Some(wasm) => env.register(
                wasm.as_slice(),
                (
                    self.t.admin.clone(),
                    7u32,
                    String::from_str(env, &format!("C{n}")),
                ),
            ),
            None => env
                .register_stellar_asset_contract_v2(self.t.admin.clone())
                .address(),
        };
        // `production_fixture` exposes the SAC `mint(to, amount)` signature.
        token::StellarAssetClient::new(env, &coin).mint(holder, &COIN);
        self.set_price(n, WAD);
        let mut oracle = feed_oracle(env, &self.feed, &self.feed_id(n));
        if let Some(anchor) = &self.anchor {
            oracle.sources.push_back(PriceSource::Feed(FeedSource {
                provider: ProviderRef::Chainlink(ChainlinkFeedRef {
                    contract: anchor.clone(),
                    data_id: self.data_id(n),
                }),
                decimals: CHAINLINK_DECIMALS,
                max_stale_seconds: 900,
            }));
        }
        self.t.configure_market_oracle(&coin, &oracle);
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
        self.list(&lp, 18, false);
        lp
    }

    /// Creates `asset`'s market and lists it on the harness spoke as
    /// collateral, and as borrowable only when `can_borrow`.
    fn list(&self, asset: &Address, decimals: u32, can_borrow: bool) {
        let cap = 1_000_000 * 10i128.pow(decimals);
        let gov = self.t.gov_client();
        gov.execute_immediate(
            &self.t.admin,
            &AdminOperation::CreateLiquidityPool(CreatePoolArgs {
                hub_id: HARNESS_HUB,
                asset: asset.clone(),
                params: DEFAULT_MARKET_PARAMS.to_market_params(asset, decimals),
            }),
        );
        gov.execute_immediate(
            &self.t.admin,
            &AdminOperation::AddAssetToSpoke(SpokeAssetArgs {
                hub_id: HARNESS_HUB,
                asset: asset.clone(),
                spoke_id: HARNESS_SPOKE,
                can_collateral: true,
                can_borrow,
                paused: false,
                frozen: false,
                no_seize: false,
                ltv: 7_500,
                threshold: 8_000,
                bonus: 500,
                liquidation_fees: 0,
                supply_cap: cap,
                borrow_cap: if can_borrow { cap } else { 0 },
            }),
        );
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

/// SDK 28 `InvocationResourceLimits::mainnet()`, the same limits the harness
/// enforces per invocation: (label, limit).
const MAINNET_LIMITS: [(&str, u64); 7] = [
    ("cpu", 400_000_000),
    ("mem", 41_943_040),
    ("footprint entries", 400),
    ("write entries", 200),
    ("disk read bytes", 200_000),
    ("write bytes", 132_096),
    ("event bytes", 16_384),
];

/// CURVE-6 worst case with every contract on the liquidation path running as
/// WASM: `POSITION_LIMIT_MAX` three-coin Curve LP collateral legs over fifteen
/// coins and `POSITION_LIMIT_MAX` debt legs. Coins and debt assets are
/// `production_fixture.wasm` tokens, each priced by a RedStone primary and a
/// Chainlink anchor on `mock_redstone.wasm`. The book is built natively, then
/// the controller, pool, position NFT and price aggregator switch to the deploy
/// WASMs and each liquidation replays from one ledger snapshot with
/// source-account auth, as in `meta/bench_liquidate_max_positions.rs`. Prints
/// usage against the mainnet per-transaction limits; asserts no ceiling.
#[test]
fn curve_lp_liquidation_worst_case_all_wasm() {
    const PAY: i128 = 100 * 10_000_000;
    let s = Scene::wasm();
    let (t, env) = (&s.t, &s.t.env);
    let (borrower, supplier) = (Address::generate(env), Address::generate(env));
    let ctrl = t.ctrl_client();
    let vault = ctrl.get_pool_address();
    let mut legs = Vec::new(env);
    let mut lps = std::vec::Vec::new();
    for leg in 0..POSITION_LIMIT_MAX {
        let coins = Vec::from_iter(env, (0..3).map(|i| s.coin(leg * 3 + i, &borrower)));
        let lp = s.list_pool(&coins, &borrower);
        legs.push_back((hub_asset(lp.clone()), 1_000 * WAD));
        lps.push(lp);
    }
    let account = ctrl.supply(&borrower, &0, &HARNESS_SPOKE, &legs);
    let debts: std::vec::Vec<Address> = (0..POSITION_LIMIT_MAX)
        .map(|d| s.coin(POSITION_LIMIT_MAX * 3 + d, &supplier))
        .collect();
    for debt in &debts {
        s.list(debt, 7, true);
    }
    let pairs = |amount: i128| {
        Vec::from_iter(
            env,
            debts.iter().map(|debt| (hub_asset(debt.clone()), amount)),
        )
    };
    ctrl.supply(&supplier, &0, &HARNESS_SPOKE, &pairs(COIN));
    ctrl.borrow(&borrower, &account, &pairs(740 * 10_000_000), &None);
    for n in 0..POSITION_LIMIT_MAX * 3 {
        s.set_price(n, WAD * 9 / 10);
    }
    assert!(ctrl.get_health_factor(&account) < WAD);
    let positions = ctrl.get_account_positions(&account);
    assert_eq!(
        (positions.0.len(), positions.1.len()),
        (POSITION_LIMIT_MAX, POSITION_LIMIT_MAX)
    );
    let debt_before = ctrl.get_total_borrow_usd(&account);
    let source = xdr::AccountId(xdr::PublicKey::PublicKeyTypeEd25519(xdr::Uint256([72; 32])));
    let liquidator: Address = xdr::ScAddress::Account(source.clone())
        .try_into_val(env)
        .unwrap();
    for debt in &debts {
        token::StellarAssetClient::new(env, debt).mint(&liquidator, &COIN);
    }

    let wasm_dir = std::env::var_os("STRESS_BUDGET_WASM_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../artifacts/wasm/deploy")
        });
    for (address, filename) in [
        (&t.controller, "controller.wasm"),
        (&t.price_aggregator, "price_aggregator.wasm"),
        (&vault, "pool.wasm"),
        (&t.position_nft, "position_nft.wasm"),
    ] {
        let path = wasm_dir.join(filename);
        let wasm = std::fs::read(&path).unwrap_or_else(|_| {
            panic!(
                "{} not found; run `make deploy-artifacts` first",
                path.display()
            )
        });
        let hash = env
            .deployer()
            .upload_contract_wasm(Bytes::from_slice(env, &wasm));
        env.as_contract(address, || {
            env.deployer()
                .update_current_contract(ContractExecutable::Wasm(hash))
        });
    }
    let snapshot = env.to_ledger_snapshot();
    // Validators cache parsed WASM outside the transaction's CPU allowance.
    env.host()
        .ensure_module_cache_contains_host_storage_contracts()
        .unwrap();
    let module_cache = env.host().take_module_cache().unwrap();

    for (mode, delay) in [
        (SeizeMode::Transfer, 0),
        (SeizeMode::Transfer, 5),
        (SeizeMode::Credit(0), 0),
        (SeizeMode::Credit(0), 5),
    ] {
        // A fresh env has no native dispatch, so a native contract on the
        // path would fail the call instead of running unmetered.
        let env = Env::from_ledger_snapshot(snapshot.clone());
        env.host().set_module_cache(module_cache.clone()).unwrap();
        env.cost_estimate().disable_resource_limits();
        let rebind = |address: &Address| -> Address {
            xdr::ScAddress::from(address).try_into_val(&env).unwrap()
        };
        let (controller, signer, pool) =
            (rebind(&t.controller), rebind(&liquidator), rebind(&vault));
        let assets: std::vec::Vec<Address> = debts.iter().map(rebind).collect();
        let mut offered = Vec::new(&env);
        let mut transfers = std::vec::Vec::new();
        for asset in &assets {
            offered.push_back((hub_asset(asset.clone()), PAY));
            transfers.push(MockAuthInvoke {
                contract: asset,
                fn_name: "transfer",
                args: (signer.clone(), pool.clone(), PAY).into_val(&env),
                sub_invokes: &[],
            });
        }
        let auth = xdr::SorobanAuthorizationEntry {
            credentials: xdr::SorobanCredentials::SourceAccount,
            root_invocation: (&MockAuthInvoke {
                contract: &controller,
                fn_name: "liquidate",
                args: (signer.clone(), account, offered.clone(), mode).into_val(&env),
                sub_invokes: &transfers,
            })
                .into(),
        };
        env.host().set_source_account(source.clone()).unwrap();
        env.set_auths(&[auth]);
        env.ledger().with_mut(|info| {
            info.timestamp += delay;
            info.sequence_number += (delay / 5) as u32;
        });
        env.cost_estimate().budget().reset_unlimited();
        let client = ControllerClient::new(&env, &controller);
        let result = client.try_liquidate(&signer, &account, &offered, &mode);
        let budget = env.cost_estimate().budget();
        let r = env.cost_estimate().resources();
        let label = format!("liquidate 5 Curve LP + 5 debt, all WASM, {mode:?}, +{delay}s");
        std::println!("\n========== {label} ==========");
        let used = [
            r.instructions as u64,
            r.mem_bytes as u64,
            u64::from(r.disk_read_entries + r.memory_read_entries + r.write_entries),
            u64::from(r.write_entries),
            u64::from(r.disk_read_bytes),
            u64::from(r.write_bytes),
            u64::from(r.contract_events_size_bytes),
        ];
        for ((name, limit), used) in MAINNET_LIMITS.iter().zip(used) {
            std::println!(
                "  {name:<18} {used:>11} / {limit:>11}  ({:>5.1}%)",
                used as f64 * 100.0 / *limit as f64
            );
        }
        std::println!(
            "  read entries: disk={} memory={}; budget cpu={} mem={}",
            r.disk_read_entries,
            r.memory_read_entries,
            budget.cpu_instruction_cost(),
            budget.memory_bytes_cost()
        );
        let mut rows: std::vec::Vec<_> = ContractCostType::VARIANTS
            .iter()
            .map(|ct| (*ct, budget.tracker(*ct)))
            .filter(|(_, tr)| tr.cpu > 0 || tr.mem > 0)
            .collect();
        rows.sort_by_key(|(_, tr)| std::cmp::Reverse(tr.cpu));
        for (ct, tr) in rows {
            std::println!(
                "  {:<32} cpu={:>11} mem={:>10} iters={:>7}",
                format!("{ct:?}"),
                tr.cpu,
                tr.mem,
                tr.iterations
            );
        }
        let receiver = result
            .expect("host invocation must succeed")
            .expect("u64 result");
        assert!(client.get_total_borrow_usd(&account) < debt_before);
        if mode == SeizeMode::Transfer {
            assert_eq!(receiver, 0);
            assert!(lps
                .iter()
                .any(|lp| token::Client::new(&env, &rebind(lp)).balance(&signer) > 0));
        } else {
            assert!(!client.get_account_positions(&receiver).0.is_empty());
        }
    }
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
