//! W5: price-aggregator edges and the pool/oracle decimals boundary.
//!
//! H1 (priority): the controller never cross-checks an oracle's `asset_decimals`
//! against the pool's `asset_decimals` for the same asset. Valuation does not read
//! the decimals; liquidation planning, repayment USD and seizure units do. The
//! `rv_finding_*` tests are HARDENING findings reachable only through a decimals
//! mismatch that governance is documented to prevent (threat model, "Token
//! assumptions"). H2-H9 cover dual-source freshness and tolerance, leg age spread,
//! future skew, the sanity band, scaled sources, TWAP shape, quotes/prices
//! agreement, and fail-closed views.

use common::types::{PriceFeedRaw, PriceStatus, SeizeMode};
use controller::types::{
    AssetOracle, FeedSource, IndependencePolicy, OracleAssetRef, OracleReadMode, PriceKey,
    PriceSource, ProviderRef, ReflectorFeedRef,
};
use governance::op::{AdminOperation, ConfigureAssetOracleArgs};
use soroban_sdk::testutils::Ledger as _;
use soroban_sdk::{contract, contractimpl, contracttype, vec, Address, Env, Symbol, Vec};
use test_harness::errors::{self, OracleError};
use test_harness::mock_reflector::{
    MockKey, MockReflector, MockReflectorClient, ReflectorAsset, ReflectorPriceData,
};
use test_harness::{
    assert_contract_error, hub_asset, map_try_ok_value, scaled_single_config,
    seed_liquidatable_usdc_eth, tolerance_band, usd, usd_frac, LendingTest, ALICE, BOB, LIQUIDATOR,
};

/// Ledger time the scenarios start from. The harness builds at 1000, too close to
/// zero to backdate feeds by the offsets used below.
const T0: u64 = 100_000;
/// 10% of the 3 ETH debt in 7-decimal base units.
const TEN_PCT_ETH: i128 = 3_000_000;
/// One whole ETH in 7-decimal base units.
const ONE_ETH: i128 = 10_000_000;
/// One whole USDC in 7-decimal base units.
const ONE_USDC: i128 = 10_000_000;

// ---------------------------------------------------------------------------
// Scripted Reflector: a test-only Reflector whose history and last price are
// set verbatim, so timestamps and observation shapes are exact (WAD-precision
// prices with `decimals = 18`, no 1e4 quantisation of the mock).
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone)]
enum ScriptKey {
    Decimals,
    Last,
    History,
}

#[contract]
pub struct ScriptedReflector;

#[contractimpl]
impl ScriptedReflector {
    pub fn set_decimals(env: Env, decimals: u32) {
        env.storage()
            .instance()
            .set(&ScriptKey::Decimals, &decimals);
    }

    pub fn set_last(env: Env, last: Option<ReflectorPriceData>) {
        env.storage().instance().set(&ScriptKey::Last, &last);
    }

    pub fn set_history(env: Env, history: Vec<ReflectorPriceData>) {
        env.storage().instance().set(&ScriptKey::History, &history);
    }

    pub fn base(env: Env) -> ReflectorAsset {
        ReflectorAsset::Other(Symbol::new(&env, "USD"))
    }

    pub fn decimals(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&ScriptKey::Decimals)
            .unwrap_or(18)
    }

    pub fn resolution(_env: Env) -> u32 {
        300
    }

    pub fn lastprice(env: Env, _asset: ReflectorAsset) -> Option<ReflectorPriceData> {
        let last: Option<Option<ReflectorPriceData>> =
            env.storage().instance().get(&ScriptKey::Last);
        last.flatten()
    }

    pub fn prices(
        env: Env,
        _asset: ReflectorAsset,
        _records: u32,
    ) -> Option<Vec<ReflectorPriceData>> {
        env.storage().instance().get(&ScriptKey::History)
    }
}

// ---------------------------------------------------------------------------
// Shared helpers.
// ---------------------------------------------------------------------------

/// One whole dollar in WAD, usable in constant contexts.
const ONE_USD: i128 = usd(1);

/// Expected outcome of one TWAP history shape.
enum TwapExpect {
    Accept { timestamp_offset: i64 },
    Reject(u32),
}

/// A TWAP history shape: label, (price, offset) observations, expected outcome.
type TwapCase = (&'static str, &'static [(i128, i64)], TwapExpect);

/// A scenario builder that returns a fixture and the key to price.
type ScenarioBuild = fn() -> (LendingTest, PriceKey);

/// Ledger timestamp `offset` seconds away from `T0`.
fn at(offset: i64) -> u64 {
    (T0 as i64 + offset) as u64
}

/// Moves the ledger clock without touching the sequence, so temporary mock
/// entries keep their TTL (the same approach as the staleness tests).
fn set_time(t: &LendingTest, ts: u64) {
    t.env.ledger().with_mut(|li| li.timestamp = ts);
}

/// Collapses a generated `try_*` result into one `Result`. Only a Rust panic
/// (an `InvokeError`) is unexpected here and fails the test.
fn flat<T, E: Into<soroban_sdk::Error>>(
    result: Result<Result<T, E>, Result<soroban_sdk::Error, soroban_sdk::InvokeError>>,
) -> Result<T, soroban_sdk::Error> {
    match result {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(err)) => Err(err.into()),
        Err(Ok(err)) => Err(err),
        Err(Err(_)) => panic!("expected a contract error, got an InvokeError"),
    }
}

fn key_of(t: &LendingTest, name: &str) -> PriceKey {
    PriceKey::Token(t.resolve_asset(name))
}

fn oracle_of(t: &LendingTest, key: &PriceKey) -> AssetOracle {
    t.price_agg_client().oracle(key).expect("oracle configured")
}

/// Stores `oracle` through the testing-only seed path (no validation).
fn seed(t: &LendingTest, key: &PriceKey, oracle: &AssetOracle) {
    t.price_agg_client().seed_oracle(key, oracle);
}

/// Governance immediate configuration; panics if the aggregator refuses it.
fn configure(t: &LendingTest, key: &PriceKey, oracle: AssetOracle) {
    t.gov_client().execute_immediate(
        &t.admin,
        &AdminOperation::ConfigureAssetOracle(ConfigureAssetOracleArgs {
            key: key.clone(),
            oracle,
        }),
    );
}

fn try_configure(
    t: &LendingTest,
    key: &PriceKey,
    oracle: AssetOracle,
) -> Result<(), soroban_sdk::Error> {
    flat(t.gov_client().try_execute_immediate(
        &t.admin,
        &AdminOperation::ConfigureAssetOracle(ConfigureAssetOracleArgs {
            key: key.clone(),
            oracle,
        }),
    ))
    .map(|_| ())
}

fn set_band(
    t: &LendingTest,
    key: &PriceKey,
    min: i128,
    max: i128,
) -> Result<(), soroban_sdk::Error> {
    flat(
        t.gov_iface_client()
            .try_set_sanity_band(&t.admin, key, &min, &max),
    )
    .map(|_| ())
}

/// `prices([key])` as a `Result`: the aggregator's own contract error on failure.
fn try_price(t: &LendingTest, key: &PriceKey) -> Result<PriceFeedRaw, soroban_sdk::Error> {
    let keys = vec![&t.env, key.clone()];
    flat(t.price_agg_client().try_prices(&keys))
        .map(|map| map.get(key.clone()).expect("key priced"))
}

/// `quotes([key])`: never panics on an unusable price.
fn quote(t: &LendingTest, key: &PriceKey) -> PriceStatus {
    let keys = vec![&t.env, key.clone()];
    t.price_agg_client()
        .quotes(&keys)
        .get(key.clone())
        .expect("key quoted")
}

/// The contract error `prices` panics with, if any.
fn price_code(t: &LendingTest, key: &PriceKey) -> Option<u32> {
    try_price(t, key).err().map(|err| err.get_code())
}

fn quote_code(t: &LendingTest, key: &PriceKey) -> Option<u32> {
    quote(t, key).error_code
}

/// `prices` reverts with `code` and `quotes` reports the same code, invalid.
fn assert_price_rejects(t: &LendingTest, key: &PriceKey, code: u32) {
    assert_contract_error(try_price(t, key), code);
    let status = quote(t, key);
    assert!(!status.valid, "quote must be invalid, got {status:?}");
    assert_eq!(status.error_code, Some(code), "quote code, got {status:?}");
}

fn reflector_feed(
    contract: &Address,
    asset: &Address,
    read_mode: OracleReadMode,
    decimals: u32,
    max_stale_seconds: u64,
) -> PriceSource {
    PriceSource::Feed(FeedSource {
        provider: ProviderRef::Reflector(ReflectorFeedRef {
            contract: contract.clone(),
            asset: OracleAssetRef::Stellar(asset.clone()),
            read_mode,
        }),
        decimals,
        max_stale_seconds,
    })
}

fn single_oracle(
    env: &Env,
    source: PriceSource,
    min_wad: i128,
    max_wad: i128,
    max_stale_seconds: u64,
) -> AssetOracle {
    AssetOracle {
        asset_decimals: 7,
        max_price_stale_seconds: max_stale_seconds,
        sources: vec![env, source],
        tolerance: tolerance_band(env, 500),
        independence: IndependencePolicy::RequireDisjoint,
        min_sanity_price_wad: min_wad,
        max_sanity_price_wad: max_wad,
    }
}

/// Standard two-asset book with fresh mock prices at `T0`.
fn fresh_standard() -> LendingTest {
    let t = LendingTest::new().standard_two_asset().build();
    set_time(&t, T0);
    t.refresh_oracle_prices();
    t
}

/// Writes `points` (price, offset from `T0`) as the scripted reflector's history.
fn script_points(env: &Env, reflector: &Address, points: &[(i128, i64)]) {
    let mut history: Vec<ReflectorPriceData> = Vec::new(env);
    for (price, offset) in points {
        history.push_back(ReflectorPriceData {
            price: *price,
            timestamp: at(*offset),
        });
    }
    ScriptedReflectorClient::new(env, reflector).set_history(&history);
}

/// Three contiguous observations at `price`, newest at `newest` (resolution 300).
fn flat3(price: i128, newest: i64) -> [(i128, i64); 3] {
    [
        (price, newest),
        (price, newest - 300),
        (price, newest - 600),
    ]
}

/// Registers a scripted reflector (18 decimals) and governs USDC onto a
/// single-source TWAP(3) read of it with band [0.99, 1.01].
fn scripted_usdc(t: &LendingTest, points: &[(i128, i64)]) -> Address {
    let usdc = t.resolve_asset("USDC");
    let reflector = t.env.register(ScriptedReflector, ());
    ScriptedReflectorClient::new(&t.env, &reflector).set_decimals(&18);
    script_points(&t.env, &reflector, points);
    let source = reflector_feed(&reflector, &usdc, OracleReadMode::Twap(3), 18, 900);
    let oracle = single_oracle(&t.env, source, usd_frac(99, 100), usd_frac(101, 100), 900);
    configure(t, &PriceKey::Token(usdc), oracle);
    reflector
}

/// Standard book with USDC on the scripted TWAP(3) read; `T0` clock.
fn scripted_fixture(points: &[(i128, i64)]) -> (LendingTest, PriceKey, Address) {
    let t = LendingTest::new().standard_two_asset().build();
    set_time(&t, T0);
    t.refresh_oracle_prices();
    let reflector = scripted_usdc(&t, points);
    let key = key_of(&t, "USDC");
    (t, key, reflector)
}

/// Standard book with USDC on a seeded spot read of the scripted reflector
/// (governance refuses spot-only configs, so the harness seeds them too), last
/// observation at `last_offset` seconds from `T0`.
fn spot_fixture(last_offset: i64) -> (LendingTest, PriceKey) {
    let t = LendingTest::new().standard_two_asset().build();
    set_time(&t, T0);
    t.refresh_oracle_prices();
    let usdc = t.resolve_asset("USDC");
    let reflector = t.env.register(ScriptedReflector, ());
    let client = ScriptedReflectorClient::new(&t.env, &reflector);
    client.set_decimals(&18);
    let key = PriceKey::Token(usdc.clone());
    let source = reflector_feed(&reflector, &usdc, OracleReadMode::Spot, 18, 900);
    let oracle = single_oracle(&t.env, source, usd_frac(99, 100), usd_frac(101, 100), 900);
    seed(&t, &key, &oracle);
    client.set_last(&Some(ReflectorPriceData {
        price: usd(1),
        timestamp: at(last_offset),
    }));
    (t, key)
}

/// Raises the staleness budget of every source and the asset, so an older leg is
/// not stale on its own and only the leg-spread bound can trip. Test-only seed.
fn widen_stale(t: &LendingTest, key: &PriceKey, seconds: u64) {
    let mut oracle = oracle_of(t, key);
    oracle.max_price_stale_seconds = seconds;
    for index in 0..oracle.sources.len() {
        if let PriceSource::Feed(mut feed) = oracle.sources.get_unchecked(index) {
            feed.max_stale_seconds = seconds;
            oracle.sources.set(index, PriceSource::Feed(feed));
        }
    }
    seed(t, key, &oracle);
}

/// Factor leg for a scaled source: a mock quoting `base` as its base asset, with
/// `asset` priced at `price` on both the spot and TWAP reads at `T0`.
fn factor_reflector(t: &LendingTest, base: &Address, asset: &Address, price: i128) -> Address {
    let addr = t.env.register(MockReflector, ());
    let client = MockReflectorClient::new(&t.env, &addr);
    client.set_base_stellar(base);
    client.set_price(asset, &price);
    client.set_twap_price(asset, &price);
    addr
}

/// ETH = factor (ETH in USDC, 2000) x USDC. `min_factor` may be raised to put the
/// factor out of its band (governance accepts it: `probe` ignores that failure).
fn scaled_eth_fixture(min_factor: Option<i128>) -> (LendingTest, PriceKey, PriceKey) {
    scaled_eth_fixture_bounds(min_factor, None)
}

fn scaled_eth_fixture_bounds(
    min_factor: Option<i128>,
    max_factor: Option<i128>,
) -> (LendingTest, PriceKey, PriceKey) {
    let t = LendingTest::new().standard_two_asset().build();
    set_time(&t, T0);
    t.refresh_oracle_prices();
    let usdc = t.resolve_asset("USDC");
    let eth = t.resolve_asset("ETH");
    let factor = factor_reflector(&t, &usdc, &eth, usd(2_000));
    let mut cfg = scaled_single_config(
        &t.env,
        &factor,
        &eth,
        PriceKey::Token(usdc.clone()),
        usd(2_000),
        500,
    );
    if min_factor.is_some() || max_factor.is_some() {
        if let PriceSource::Scaled(mut scaled) = cfg.sources.get_unchecked(0) {
            if let Some(min) = min_factor {
                scaled.min_factor_wad = min;
            }
            if let Some(max) = max_factor {
                scaled.max_factor_wad = max;
            }
            cfg.sources.set(0, PriceSource::Scaled(scaled));
        }
    }
    configure(&t, &PriceKey::Token(eth.clone()), cfg);
    (t, PriceKey::Token(eth), PriceKey::Token(usdc))
}

/// ETH quoted in USDC and USDC quoted in ETH, written with `seed_oracle`
/// (governance refuses the second of the two, see the cycle test).
fn cycle_fixture() -> (LendingTest, PriceKey, PriceKey) {
    let t = LendingTest::new().standard_two_asset().build();
    set_time(&t, T0);
    t.refresh_oracle_prices();
    let usdc = t.resolve_asset("USDC");
    let eth = t.resolve_asset("ETH");
    let factor_eth = factor_reflector(&t, &usdc, &eth, usd(2_000));
    let factor_usdc = factor_reflector(&t, &eth, &usdc, usd(1) / 2_000);
    let eth_cfg = scaled_single_config(
        &t.env,
        &factor_eth,
        &eth,
        PriceKey::Token(usdc.clone()),
        usd(2_000),
        500,
    );
    let usdc_cfg = scaled_single_config(
        &t.env,
        &factor_usdc,
        &usdc,
        PriceKey::Token(eth.clone()),
        usd(1) / 2_000,
        500,
    );
    seed(&t, &PriceKey::Token(usdc.clone()), &usdc_cfg);
    seed(&t, &PriceKey::Token(eth.clone()), &eth_cfg);
    (t, PriceKey::Token(eth), PriceKey::Token(usdc))
}

// ---------------------------------------------------------------------------
// H1: oracle decimals vs pool decimals (priority).
// ---------------------------------------------------------------------------

/// Overwrites the stored oracle `asset_decimals` for `asset_name` (seed path). The
/// pool keeps its own decimals (7); this is the mismatch H1 describes.
fn seed_decimals(t: &LendingTest, asset_name: &str, decimals: u32) {
    let key = key_of(t, asset_name);
    let mut oracle = oracle_of(t, &key);
    oracle.asset_decimals = decimals;
    seed(t, &key, &oracle);
}

/// Alice: 10,000 USDC supplied, 3 ETH borrowed, USDC at $0.50 (liquidatable).
/// BOB supplies 10 ETH so that a bad-debt write-off reaches a lender, which the
/// ETH supply index shows.
fn liquidatable() -> LendingTest {
    let mut t = LendingTest::new().standard_two_asset().build();
    seed_liquidatable_usdc_eth(&mut t);
    t.supply(BOB, "ETH", 10.0);
    t
}

/// ETH supply index (RAY) for the lender-side check.
fn eth_supply_index(t: &LendingTest) -> i128 {
    t.ctrl_client()
        .get_market_index(&hub_asset(t.resolve_asset("ETH")))
        .supply_index
}

/// One liquidation of Alice's ETH debt with USDC collateral, Transfer mode:
/// the estimate, then the execution, with balances read before and after.
#[derive(Debug)]
struct LiqRun {
    offered: i128,
    planned_seized: i128,
    planned_fee: i128,
    planned_eth_refund: i128,
    bonus_bps: i128,
    executed: bool,
    error: String,
    alice_usdc_before: i128,
    alice_usdc_after: i128,
    alice_eth_before: i128,
    alice_eth_after: i128,
    liquidator_usdc: i128,
    liquidator_eth_paid: i128,
}

impl LiqRun {
    /// Collateral that left Alice's supply (gross, before the protocol fee).
    fn executed_seized(&self) -> i128 {
        self.alice_usdc_before - self.alice_usdc_after
    }

    /// Debt Alice's ETH borrow lost.
    fn debt_retired(&self) -> i128 {
        self.alice_eth_before - self.alice_eth_after
    }
}

fn run_liquidation(t: &mut LendingTest, offer: i128) -> LiqRun {
    let alice = t.account_id(ALICE);
    let liquidator = t.get_or_create_user(LIQUIDATOR);
    let eth = t.resolve_asset("ETH");
    let payments = vec![&t.env, (hub_asset(eth.clone()), offer)];
    let estimate =
        t.ctrl_client()
            .get_liquidation_estimate(&alice, &payments, &SeizeMode::Transfer);
    let planned_seized = estimate.seized_collaterals.get(0).map_or(0, |p| p.amount);
    let planned_fee = estimate.protocol_fees.get(0).map_or(0, |p| p.amount);
    let planned_eth_refund: i128 = estimate
        .refunds
        .iter()
        .filter(|p| p.asset == eth)
        .map(|p| p.amount)
        .sum();
    let bonus_bps = estimate.bonus_rate_bps;

    let alice_usdc_before = t.supply_balance_raw(ALICE, "USDC");
    let alice_eth_before = t.borrow_balance_raw(ALICE, "ETH");
    t.resolve_market("ETH")
        .token_admin
        .mint(&liquidator, &offer);
    let result = map_try_ok_value(t.ctrl_client().try_liquidate(
        &liquidator,
        &alice,
        &payments,
        &SeizeMode::Transfer,
    ));
    let executed = result.is_ok();
    let error = match &result {
        Ok(_) => String::from("ok"),
        Err(err) => format!("{err:?}"),
    };
    let eth_left = t.token_balance_raw(LIQUIDATOR, "ETH");
    LiqRun {
        offered: offer,
        planned_seized,
        planned_fee,
        planned_eth_refund,
        bonus_bps,
        executed,
        error,
        alice_usdc_before,
        alice_usdc_after: t.supply_balance_raw(ALICE, "USDC"),
        alice_eth_before,
        alice_eth_after: t.borrow_balance_raw(ALICE, "ETH"),
        liquidator_usdc: t.token_balance_raw(LIQUIDATOR, "USDC"),
        liquidator_eth_paid: if executed { offer - eth_left } else { 0 },
    }
}

/// H1(1): valuation (total collateral, total debt, health factor) does not read the
/// oracle decimals, so a mismatch is invisible to every health check. Governance
/// refuses a decimals change on a live oracle (InvalidOracleDecimals).
#[test]
fn rv_oracle_decimals_change_leaves_valuation_and_is_neutralised() {
    for (name, decimals) in [("USDC", 6u32), ("USDC", 8), ("ETH", 6), ("ETH", 8)] {
        let t = liquidatable();
        let before = (
            t.total_collateral_raw(ALICE),
            t.total_debt_raw(ALICE),
            t.health_factor_raw(ALICE),
        );
        seed_decimals(&t, name, decimals);
        let after = (
            t.total_collateral_raw(ALICE),
            t.total_debt_raw(ALICE),
            t.health_factor_raw(ALICE),
        );
        assert_eq!(
            before, after,
            "{name} oracle decimals {decimals}: valuation read the decimals"
        );
    }

    // Pinned values at USDC $0.50: 10,000 USDC = $5,000; 3 ETH = $6,000.
    let t = liquidatable();
    assert_eq!(t.total_collateral_raw(ALICE), usd(5_000));
    assert_eq!(t.total_debt_raw(ALICE), usd(6_000));

    // Governance does not reject a decimals change: `resolve_oracle` overwrites
    // the proposed `asset_decimals` with the stored oracle's value, so the call
    // succeeds and the stored unit stays 7.
    let key = key_of(&t, "USDC");
    let mut proposed = oracle_of(&t, &key);
    proposed.asset_decimals = 6;
    try_configure(&t, &key, proposed).expect("governance neutralises the change");
    assert_eq!(
        oracle_of(&t, &key).asset_decimals,
        7,
        "stored unit unchanged"
    );

    // A direct owner call to the aggregator skips that resolution and is refused.
    let mut direct = oracle_of(&t, &key);
    direct.asset_decimals = 6;
    assert_contract_error(
        flat(t.price_agg_client().try_set_oracle(&key, &direct)),
        OracleError::InvalidOracleDecimals as u32,
    );
}

/// Control for H1: with the correct decimals the plan equals the execution. The
/// seizure leaves Alice's supply as planned, the liquidator receives the seizure
/// less the planned protocol fee, and the pool retires exactly the offered ETH.
#[test]
fn rv_liquidate_correct_decimals_plan_equals_execution() {
    let mut t = liquidatable();
    let run = run_liquidation(&mut t, TEN_PCT_ETH);
    assert_eq!(run.error, "ok");
    assert_eq!(run.offered, TEN_PCT_ETH);
    assert_eq!(run.bonus_bps, 500);
    assert_eq!(run.planned_seized, 12_600_000_000, "1,260 USDC planned");
    assert_eq!(
        run.executed_seized(),
        run.planned_seized,
        "executed = planned"
    );
    assert_eq!(run.planned_fee, 72_000_000, "7.2 USDC protocol fee");
    assert_eq!(run.liquidator_usdc, run.executed_seized() - run.planned_fee);
    assert_eq!(run.liquidator_eth_paid, TEN_PCT_ETH);
    assert_eq!(run.planned_eth_refund, 0);
    assert_eq!(run.debt_retired(), TEN_PCT_ETH);
}

/// H1(3), token level: an over-offer of 1.0 ETH against ETH's oracle at 6 decimals
/// (pool 7) is refunded correctly in token units. Pulled plus refunded equals
/// offered. The valuation error is not a refund error: the plan prices the pulled
/// tokens 10x too high, which the RV-FINDING test below shows. Alice's debt
/// retirement is not asserted here because the mismatched plan seizes the whole
/// position and the remaining debt is socialised as bad debt.
#[test]
fn rv_liquidate_eth_decimals_six_overoffer_refund_conserves_tokens() {
    let mut t = liquidatable();
    seed_decimals(&t, "ETH", 6);
    let run = run_liquidation(&mut t, ONE_ETH);
    assert!(run.executed, "liquidation must execute: {run:?}");
    assert_eq!(
        run.liquidator_eth_paid + run.planned_eth_refund,
        ONE_ETH,
        "pulled + refunded must equal offered: {run:?}"
    );
    assert!(run.liquidator_eth_paid > 0, "tokens were pulled: {run:?}");
}

/// H1(2a) HARDENING: USDC oracle decimals 6 against pool 7. The seizure is planned
/// and executed in base units scaled by the oracle's decimals, so Alice loses one
/// tenth of the collateral that the correctly configured run seizes for the same
/// repayment. Run with `--ignored` to see the values.
#[test]
#[ignore = "RV-FINDING: HARDENING. USDC oracle decimals 6 vs pool 7: 10% repay seizes 1/10 of the correct collateral"]
fn rv_finding_liquidate_usdc_decimals_six_under_seizes_tenth() {
    let baseline = run_liquidation(&mut liquidatable(), TEN_PCT_ETH);
    let mut t = liquidatable();
    seed_decimals(&t, "USDC", 6);
    let run = run_liquidation(&mut t, TEN_PCT_ETH);
    assert_eq!(
        run.executed_seized(),
        baseline.executed_seized(),
        "SAFE: same repayment must seize the same collateral. baseline={baseline:?} mismatch={run:?}"
    );
}

/// H1(2b) HARDENING: USDC oracle decimals 8 against pool 7. The plan is 1,260 USDC
/// in 8-decimal units (1.26e11). The pool reads that as 7-decimal units, a request
/// for 12,600 USDC, which exceeds the 10,000 USDC position and becomes a full close.
/// The remaining 2.7 ETH of Alice's debt is then socialised to ETH suppliers.
/// Run with `--ignored` to see the values.
#[test]
#[ignore = "RV-FINDING: HARDENING. USDC oracle decimals 8 vs pool 7: 10% repay closes the whole 10,000 USDC position and socialises 2.7 ETH"]
fn rv_finding_liquidate_usdc_decimals_eight_overseizes_full_balance() {
    let baseline = run_liquidation(&mut liquidatable(), TEN_PCT_ETH);
    let mut t = liquidatable();
    seed_decimals(&t, "USDC", 8);
    let index_before = eth_supply_index(&t);
    let run = run_liquidation(&mut t, TEN_PCT_ETH);
    let index_after = eth_supply_index(&t);
    assert!(
        run.executed_seized() == baseline.executed_seized() && index_after == index_before,
        "SAFE: same repayment seizes the same collateral and socialises no debt. \
         baseline={baseline:?} mismatch={run:?} eth_supply_index {index_before} -> {index_after}"
    );
}

/// H1(3) HARDENING: ETH oracle decimals 6 against pool 7, over-offer of 1.0 ETH. The
/// plan values each ETH base unit 10x, so the trimmed plan pulls 0.238 ETH (refunding
/// 0.762 ETH, which is correct in token terms) for a credited repayment worth $4,762.
/// The seizure then covers the whole 10,000 USDC position, and the 2.76 ETH left of
/// Alice's debt is socialised to ETH suppliers. Run with `--ignored`.
#[test]
#[ignore = "RV-FINDING: HARDENING. ETH oracle decimals 6 vs pool 7: 1.0 ETH offer pays 0.238 ETH, seizes the whole USDC position and socialises 2.76 ETH"]
fn rv_finding_liquidate_eth_decimals_six_overoffer_pays_fraction_for_full_seizure() {
    let baseline = run_liquidation(&mut liquidatable(), ONE_ETH);
    let mut t = liquidatable();
    seed_decimals(&t, "ETH", 6);
    let index_before = eth_supply_index(&t);
    let run = run_liquidation(&mut t, ONE_ETH);
    let index_after = eth_supply_index(&t);
    assert!(
        run.liquidator_eth_paid == baseline.liquidator_eth_paid
            && run.executed_seized() == baseline.executed_seized()
            && index_after == index_before,
        "SAFE: same offer pulls the same ETH, seizes the same collateral and socialises no debt. \
         baseline={baseline:?} mismatch={run:?} eth_supply_index {index_before} -> {index_after}"
    );
}

// ---------------------------------------------------------------------------
// H2: dual-source fail-closed and the tolerance edge (INV-ORACLE-02).
// ---------------------------------------------------------------------------

/// A stale spot leg is not papered over by the fresh TWAP leg: borrow fails with
/// PriceFeedStale, and `quotes` shows the stale flag with a nonzero candidate.
#[test]
fn rv_dual_source_stale_spot_leg_borrow_rejects_price_feed_stale() {
    let mut t = LendingTest::new().dual_source_two_asset();
    set_time(&t, T0);
    t.refresh_oracle_prices();
    let usdc = t.resolve_asset("USDC");
    let key = key_of(&t, "USDC");
    // Spot leg age 1000 s, above the 900 s budget. The TWAP leg is fresh at T0.
    t.mock_reflector_client()
        .set_price_at(&usdc, &usd(1), &at(-1_000));
    t.supply(ALICE, "USDC", 100_000.0);

    assert_contract_error(t.try_borrow(ALICE, "ETH", 1.0), errors::PRICE_FEED_STALE);

    let status = quote(&t, &key);
    assert!(status.stale, "stale flag: {status:?}");
    assert!(!status.valid, "valid must be false: {status:?}");
    assert_eq!(status.error_code, Some(errors::PRICE_FEED_STALE));
    assert_eq!(status.final_wad, usd(1), "candidate is the midpoint");
    assert_eq!(status.price_timestamp, at(-1_000), "older leg timestamp");

    // INV-ORACLE-02: a stale leg is reported before disagreement. With the TWAP
    // leg also off-band (1.20 vs 1.00), both flags are set and the code is stale.
    t.set_safe_price("USDC", usd_frac(12, 10));
    let both = quote(&t, &key);
    assert!(both.stale && both.deviation && !both.valid, "{both:?}");
    assert_eq!(
        both.error_code,
        Some(errors::PRICE_FEED_STALE),
        "stale first"
    );
}

/// INV-ORACLE-02 rounds the midpoint down. The scripted reflector (18 decimals)
/// keeps WAD precision: the TWAP leg is 1e18 + 1 and the spot leg 1e18 + 2, an odd
/// sum, so the midpoint is floor(1e18 + 1.5) = 1e18 + 1.
#[test]
fn rv_dual_source_midpoint_rounds_down_on_odd_wad_sum() {
    let t = LendingTest::new().standard_two_asset().build();
    set_time(&t, T0);
    t.refresh_oracle_prices();
    let usdc = t.resolve_asset("USDC");
    let key = PriceKey::Token(usdc.clone());
    let reflector = t.env.register(ScriptedReflector, ());
    let client = ScriptedReflectorClient::new(&t.env, &reflector);
    client.set_decimals(&18);
    script_points(&t.env, &reflector, &flat3(usd(1) + 1, 0));
    client.set_last(&Some(ReflectorPriceData {
        price: usd(1) + 2,
        timestamp: at(0),
    }));
    let sources = vec![
        &t.env,
        reflector_feed(&reflector, &usdc, OracleReadMode::Twap(3), 18, 900),
        reflector_feed(&reflector, &usdc, OracleReadMode::Spot, 18, 900),
    ];
    let oracle = AssetOracle {
        asset_decimals: 7,
        max_price_stale_seconds: 900,
        sources,
        tolerance: tolerance_band(&t.env, 500),
        independence: IndependencePolicy::RequireDisjoint,
        min_sanity_price_wad: usd_frac(99, 100),
        max_sanity_price_wad: usd_frac(101, 100),
    };
    seed(&t, &key, &oracle);

    let q = quote(&t, &key);
    assert!(q.valid, "{q:?}");
    assert_eq!(q.primary_wad, usd(1) + 1, "TWAP leg");
    assert_eq!(q.secondary_wad, usd(1) + 2, "spot leg");
    assert_eq!(q.final_wad, usd(1) + 1, "floor of the midpoint");
    assert_eq!(try_price(&t, &key).expect("priced").price_wad, usd(1) + 1);
}

/// Legs at 1.05 (TWAP, primary) and 1.00 (spot, anchor) sit exactly on the 10,500
/// bps upper bound: accepted at the floored midpoint 1.025. At 10,501 bps the
/// price is refused with UnsafePriceNotAllowed.
#[test]
fn rv_dual_source_tolerance_edge_accepts_midpoint_rejects_one_bp_over() {
    let t = LendingTest::new().dual_source_two_asset();
    set_time(&t, T0);
    t.refresh_oracle_prices();
    let key = key_of(&t, "USDC");
    assert_eq!(oracle_of(&t, &key).tolerance.upper_ratio_bps, 10_500);

    t.set_safe_price("USDC", usd_frac(105, 100));
    let edge = quote(&t, &key);
    assert!(edge.valid && !edge.deviation, "edge accepted: {edge:?}");
    assert_eq!(edge.primary_wad, usd_frac(105, 100));
    assert_eq!(edge.secondary_wad, usd(1));
    let midpoint = usd_frac(1_025, 1_000);
    assert_eq!(edge.final_wad, midpoint, "quote midpoint");
    assert_eq!(
        try_price(&t, &key).expect("edge priced").price_wad,
        midpoint
    );

    t.set_safe_price("USDC", usd_frac(10_501, 10_000));
    assert_price_rejects(&t, &key, errors::UNSAFE_PRICE);
    let beyond = quote(&t, &key);
    assert!(beyond.deviation, "deviation flag: {beyond:?}");
    assert_eq!(beyond.final_wad, usd_frac(102_505, 100_000));
}

// ---------------------------------------------------------------------------
// H3: leg age spread (MAX_LEG_AGE_SPREAD_SECONDS = 3600).
// ---------------------------------------------------------------------------

/// Both harness dual legs are Reflector reads, so both are Market nature and the
/// spread bound applies. (The harness dual config has no Fundamental leg, so it
/// needs no adaptation.) The older leg is the TWAP oldest sample at -3600 for a
/// newest sample at -3000; the staleness budget is raised so only the spread trips.
#[test]
fn rv_dual_source_leg_age_spread_3600_accepts_3601_marks_stale() {
    let t = LendingTest::new().dual_source_two_asset();
    set_time(&t, T0);
    t.refresh_oracle_prices();
    let usdc = t.resolve_asset("USDC");
    let key = PriceKey::Token(usdc.clone());
    widen_stale(&t, &key, 4_000);

    let mock = t.mock_reflector_client();
    mock.set_price_at(&usdc, &usd(1), &at(0));
    mock.set_twap_price_at(&usdc, &usd(1), &at(-3_000));
    let accepted = quote(&t, &key);
    assert!(
        accepted.valid && !accepted.stale,
        "3600 s spread: {accepted:?}"
    );
    assert_eq!(
        accepted.price_timestamp,
        at(-3_600),
        "older leg sets timestamp"
    );

    mock.set_twap_price_at(&usdc, &usd(1), &at(-3_001));
    let stale = quote(&t, &key);
    assert!(stale.stale && !stale.valid, "3601 s spread: {stale:?}");
    assert_eq!(stale.error_code, Some(errors::PRICE_FEED_STALE));
    assert_contract_error(try_price(&t, &key), errors::PRICE_FEED_STALE);
}

// ---------------------------------------------------------------------------
// H4: future skew (INV-ORACLE-04, MAX_FUTURE_SKEW_SECONDS = 60).
// ---------------------------------------------------------------------------

/// Spot: a timestamp at now+60 is kept, now+61 is dropped, leaving no reading.
#[test]
fn rv_spot_future_skew_plus60_accepts_plus61_rejects() {
    let (t, key) = spot_fixture(60);
    let accepted = try_price(&t, &key).expect("now+60 is within the skew allowance");
    assert_eq!(accepted.price_wad, usd(1));
    assert_eq!(accepted.timestamp, at(60));

    let (t, key) = spot_fixture(61);
    assert_price_rejects(&t, &key, errors::NO_LAST_PRICE);
}

/// TWAP: the boundary applies to the newest observation. A future-dated
/// observation anywhere in the history fails the read. A non-newest observation
/// at +60 is rejected by the ordering (spacing) check, not by INV-ORACLE-04.
#[test]
fn rv_twap_future_skew_newest_boundary_and_middle_observation_fail_closed() {
    let p = usd(1);
    let (t, key, _) = scripted_fixture(&flat3(p, 60));
    let accepted = try_price(&t, &key).expect("newest at now+60 is accepted");
    assert_eq!(accepted.price_wad, p);
    assert_eq!(
        accepted.timestamp,
        at(-540),
        "oldest sample sets the timestamp"
    );

    let (t, key, _) = scripted_fixture(&flat3(p, 61));
    assert_price_rejects(&t, &key, errors::NO_LAST_PRICE);

    let (t, key, _) = scripted_fixture(&[(p, 0), (p, 61), (p, -600)]);
    assert_price_rejects(&t, &key, errors::NO_LAST_PRICE);

    let (t, key, _) = scripted_fixture(&[(p, 0), (p, 60), (p, -600)]);
    assert_price_rejects(&t, &key, errors::NO_LAST_PRICE);

    // Oldest position: +61 fails the future rule; +60 fails the ordering check.
    let (t, key, _) = scripted_fixture(&[(p, 0), (p, -300), (p, 61)]);
    assert_price_rejects(&t, &key, errors::NO_LAST_PRICE);
    let (t, key, _) = scripted_fixture(&[(p, 0), (p, -300), (p, 60)]);
    assert_price_rejects(&t, &key, errors::NO_LAST_PRICE);
}

// ---------------------------------------------------------------------------
// H5: sanity band (SanityBoundViolated, SanityBandMustTighten, DoS.1).
// ---------------------------------------------------------------------------

/// The band [0.99, 1.01] is inclusive at both ends. One wad outside either end is
/// refused. The scripted 18-decimal reflector makes min-1 and max+1 exact.
#[test]
fn rv_sanity_band_edges_accept_bounds_reject_one_wad_outside() {
    let (t, key, reflector) = scripted_fixture(&flat3(usd_frac(99, 100), 0));
    let lo = usd_frac(99, 100);
    let hi = usd_frac(101, 100);

    for (price, accept) in [(lo, true), (lo - 1, false), (hi, true), (hi + 1, false)] {
        script_points(&t.env, &reflector, &flat3(price, 0));
        if accept {
            let priced = try_price(&t, &key).unwrap_or_else(|e| panic!("{price}: {e:?}"));
            assert_eq!(priced.price_wad, price, "accepted price");
            assert!(quote(&t, &key).valid, "quote valid at {price}");
        } else {
            assert_price_rejects(&t, &key, errors::SANITY_BOUND_VIOLATED);
        }
    }
}

/// DoS.1: governance narrows USDC's band past its live price. Widening stays
/// refused. Every valuation-dependent call fails closed with SanityBoundViolated,
/// while supply, which needs no price, still works.
#[test]
fn rv_sanity_band_narrowed_past_price_fails_closed_supply_survives() {
    let (mut t, usdc_key, reflector) = scripted_fixture(&flat3(usd(1), 0));
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    t.assert_healthy(ALICE);

    // 0.995 is inside [0.99, 1.01]; the account stays healthy.
    script_points(&t.env, &reflector, &flat3(usd_frac(995, 1_000), 0));
    assert!(try_price(&t, &usdc_key).is_ok(), "0.995 is still in band");

    // [0.997, 1.01] is a valid tightening (64.8 bps half width) that excludes 0.995.
    set_band(&t, &usdc_key, usd_frac(997, 1_000), usd_frac(101, 100)).expect("narrowing accepted");
    assert_contract_error(
        set_band(&t, &usdc_key, usd_frac(98, 100), usd_frac(102, 100)),
        errors::SANITY_BAND_MUST_TIGHTEN,
    );
    assert_price_rejects(&t, &usdc_key, errors::SANITY_BOUND_VIOLATED);

    assert_contract_error(
        t.try_borrow(ALICE, "ETH", 0.5),
        errors::SANITY_BOUND_VIOLATED,
    );
    assert_contract_error(
        t.try_withdraw(ALICE, "USDC", 100.0),
        errors::SANITY_BOUND_VIOLATED,
    );

    // Make Alice liquidatable on the ETH leg: debt 8,700 against 7,960 weighted.
    t.set_price("ETH", usd(2_900));
    let liquidator = t.get_or_create_user(LIQUIDATOR);
    let alice = t.account_id(ALICE);
    let eth = t.resolve_asset("ETH");
    let payments = vec![&t.env, (hub_asset(eth.clone()), TEN_PCT_ETH)];
    t.resolve_market("ETH")
        .token_admin
        .mint(&liquidator, &TEN_PCT_ETH);
    let liquidated =
        flat(
            t.ctrl_client()
                .try_liquidate(&liquidator, &alice, &payments, &SeizeMode::Transfer),
        )
        .map(|_| ());
    assert_contract_error(liquidated, errors::SANITY_BOUND_VIOLATED);

    let before = t.supply_balance_raw(ALICE, "USDC");
    t.try_supply(ALICE, "USDC", 1_000.0)
        .expect("supply needs no price");
    assert_eq!(
        t.supply_balance_raw(ALICE, "USDC") - before,
        1_000 * ONE_USDC,
        "supply credited exactly 1,000 USDC"
    );
}

// ---------------------------------------------------------------------------
// H6: scaled sources (factor band, nested quote, cycles).
// ---------------------------------------------------------------------------

/// A factor read outside [min_factor, max_factor] is FactorOutOfBounds, below the
/// minimum and above the maximum. The in-band control prices ETH at 2,000.
#[test]
fn rv_scaled_factor_out_of_band_rejects_230() {
    let (t, eth, _usdc) = scaled_eth_fixture(None);
    assert_eq!(
        try_price(&t, &eth)
            .expect("in-band factor prices")
            .price_wad,
        usd(2_000)
    );

    let (t, eth, _usdc) = scaled_eth_fixture(Some(usd(2_100)));
    assert_price_rejects(&t, &eth, OracleError::FactorOutOfBounds as u32);

    // Above the maximum: factor 2,000 against a 1,900 ceiling.
    let (t, eth, _usdc) = scaled_eth_fixture_bounds(None, Some(usd(1_900)));
    assert_price_rejects(&t, &eth, OracleError::FactorOutOfBounds as u32);
}

/// A stale nested quote (USDC) makes the scaled ETH price unusable with the quote's
/// PriceFeedStale. The factor leg stays fresh, so the stale quote is the cause.
#[test]
fn rv_scaled_stale_nested_quote_rejects_206() {
    let (t, eth, usdc) = scaled_eth_fixture(None);
    assert_eq!(
        try_price(&t, &eth).expect("fresh quote prices").price_wad,
        usd(2_000)
    );

    let usdc_asset = t.resolve_asset("USDC");
    t.mock_reflector_client()
        .set_twap_price_at(&usdc_asset, &usd(1), &at(-2_000));
    assert_price_rejects(&t, &eth, errors::PRICE_FEED_STALE);
    assert!(try_price(&t, &usdc).is_err(), "the quote itself is stale");
}

/// A scaled cycle (ETH quoted in USDC, USDC quoted in ETH) is refused by governance
/// at configuration (OracleCycleDetected). Seeded in directly, `prices` reports the
/// cycle on both keys and `quotes` carries the same code. Nothing loops.
#[test]
fn rv_scaled_cycle_seeded_reports_225_and_config_refuses_225() {
    let code = OracleError::OracleCycleDetected as u32;
    let (t, eth, usdc) = cycle_fixture();
    assert_price_rejects(&t, &eth, code);
    assert_price_rejects(&t, &usdc, code);

    // Configuration refusal, fresh book: USDC seeded to point at ETH, then ETH
    // configured to point back at USDC through governance.
    let t = LendingTest::new().standard_two_asset().build();
    set_time(&t, T0);
    t.refresh_oracle_prices();
    let usdc = t.resolve_asset("USDC");
    let eth = t.resolve_asset("ETH");
    let factor_usdc = factor_reflector(&t, &eth, &usdc, usd(1) / 2_000);
    let usdc_cfg = scaled_single_config(
        &t.env,
        &factor_usdc,
        &usdc,
        PriceKey::Token(eth.clone()),
        usd(1) / 2_000,
        500,
    );
    seed(&t, &PriceKey::Token(usdc.clone()), &usdc_cfg);
    let factor_eth = factor_reflector(&t, &usdc, &eth, usd(2_000));
    let eth_cfg = scaled_single_config(
        &t.env,
        &factor_eth,
        &eth,
        PriceKey::Token(usdc.clone()),
        usd(2_000),
        500,
    );
    assert_contract_error(
        try_configure(&t, &PriceKey::Token(eth.clone()), eth_cfg),
        code,
    );
}

// ---------------------------------------------------------------------------
// H7: TWAP(3) history shape (spacing, count, span, emptiness, zero price).
// ---------------------------------------------------------------------------

/// Accepted shapes are exactly those the window allows. Every rejected shape gives
/// NoLastPrice (210) from `prices` and `quotes`: `read_twap(..).ok()` discards the
/// specific TWAP error, so the cause is not visible to callers.
#[test]
fn rv_twap3_history_shapes_fail_closed_with_observed_codes() {
    let cases: [TwapCase; 6] = [
        (
            "two observations spanning the full 600 s window",
            &[(ONE_USD, 0), (ONE_USD, -600)],
            TwapExpect::Accept {
                timestamp_offset: -600,
            },
        ),
        (
            "spacing 299 s below the 300 s resolution",
            &[(ONE_USD, 0), (ONE_USD, -299), (ONE_USD, -599)],
            TwapExpect::Reject(errors::NO_LAST_PRICE),
        ),
        (
            "five observations, above records + 1",
            &[
                (ONE_USD, 0),
                (ONE_USD, -300),
                (ONE_USD, -600),
                (ONE_USD, -900),
                (ONE_USD, -1_200),
            ],
            TwapExpect::Reject(errors::NO_LAST_PRICE),
        ),
        (
            "two observations, span 300 s short of 600 s",
            &[(ONE_USD, 0), (ONE_USD, -300)],
            TwapExpect::Reject(errors::NO_LAST_PRICE),
        ),
        (
            "empty history",
            &[],
            TwapExpect::Reject(errors::NO_LAST_PRICE),
        ),
        (
            "zero-price samples",
            &[(0, 0), (0, -300), (0, -600)],
            TwapExpect::Reject(errors::NO_LAST_PRICE),
        ),
    ];

    for (name, points, expect) in cases {
        let (t, key, _) = scripted_fixture(points);
        match expect {
            TwapExpect::Accept { timestamp_offset } => {
                let priced = try_price(&t, &key).unwrap_or_else(|e| panic!("{name}: {e:?}"));
                assert_eq!(priced.price_wad, ONE_USD, "{name}: price");
                assert_eq!(priced.timestamp, at(timestamp_offset), "{name}: timestamp");
                assert!(quote(&t, &key).valid, "{name}: quote valid");
            }
            TwapExpect::Reject(code) => {
                assert_eq!(price_code(&t, &key), Some(code), "{name}: prices code");
                assert_eq!(quote_code(&t, &key), Some(code), "{name}: quotes code");
                assert!(!quote(&t, &key).valid, "{name}: quote must be invalid");
            }
        }
    }
}

// ---------------------------------------------------------------------------
// H8: prices() and quotes() agree on every failure mode.
// ---------------------------------------------------------------------------

fn s_good() -> (LendingTest, PriceKey) {
    let t = fresh_standard();
    let key = key_of(&t, "USDC");
    (t, key)
}

fn s_single_stale() -> (LendingTest, PriceKey) {
    let t = fresh_standard();
    let usdc = t.resolve_asset("USDC");
    t.mock_reflector_client()
        .set_twap_price_at(&usdc, &usd(1), &at(-2_000));
    let key = key_of(&t, "USDC");
    (t, key)
}

fn s_single_above_band() -> (LendingTest, PriceKey) {
    let (t, key, _) = scripted_fixture(&flat3(usd_frac(102, 100), 0));
    (t, key)
}

fn s_single_zero_price() -> (LendingTest, PriceKey) {
    let t = fresh_standard();
    let usdc = t.resolve_asset("USDC");
    t.mock_reflector_client().set_twap_price(&usdc, &0);
    let key = key_of(&t, "USDC");
    (t, key)
}

fn s_dual_deviation() -> (LendingTest, PriceKey) {
    let t = LendingTest::new().dual_source_two_asset();
    set_time(&t, T0);
    t.refresh_oracle_prices();
    t.set_safe_price("USDC", usd_frac(11, 10));
    let key = key_of(&t, "USDC");
    (t, key)
}

fn s_dual_stale_leg() -> (LendingTest, PriceKey) {
    let t = LendingTest::new().dual_source_two_asset();
    set_time(&t, T0);
    t.refresh_oracle_prices();
    let usdc = t.resolve_asset("USDC");
    t.mock_reflector_client()
        .set_price_at(&usdc, &usd(1), &at(-1_000));
    let key = key_of(&t, "USDC");
    (t, key)
}

fn s_dual_partial() -> (LendingTest, PriceKey) {
    let t = LendingTest::new().dual_source_two_asset();
    set_time(&t, T0);
    t.refresh_oracle_prices();
    let usdc = t.resolve_asset("USDC");
    t.env.as_contract(&t.mock_reflector, || {
        t.env
            .storage()
            .temporary()
            .remove(&MockKey::Spot(usdc.clone()));
    });
    let key = key_of(&t, "USDC");
    (t, key)
}

fn s_scaled_factor_out_of_band() -> (LendingTest, PriceKey) {
    let (t, eth, _) = scaled_eth_fixture(Some(usd(2_100)));
    (t, eth)
}

fn s_scaled_stale_quote() -> (LendingTest, PriceKey) {
    let (t, eth, _) = scaled_eth_fixture(None);
    let usdc = t.resolve_asset("USDC");
    t.mock_reflector_client()
        .set_twap_price_at(&usdc, &usd(1), &at(-2_000));
    (t, eth)
}

fn s_scaled_cycle() -> (LendingTest, PriceKey) {
    let (t, eth, _) = cycle_fixture();
    (t, eth)
}

fn s_twap_spacing_299() -> (LendingTest, PriceKey) {
    let (t, key, _) = scripted_fixture(&[(usd(1), 0), (usd(1), -299), (usd(1), -599)]);
    (t, key)
}

fn s_spot_future_61() -> (LendingTest, PriceKey) {
    spot_fixture(61)
}

/// For each failure mode, `prices` reverts with exactly the code `quotes` reports,
/// and `quotes` reports the price invalid. The good case agrees on the value.
#[test]
fn rv_prices_and_quotes_agree_on_every_failure_mode() {
    let scenarios: [(&str, ScenarioBuild, Option<u32>); 12] = [
        ("good single TWAP", s_good, None),
        (
            "single TWAP stale",
            s_single_stale,
            Some(errors::PRICE_FEED_STALE),
        ),
        (
            "single TWAP above sanity band",
            s_single_above_band,
            Some(errors::SANITY_BOUND_VIOLATED),
        ),
        (
            "single TWAP zero price",
            s_single_zero_price,
            Some(errors::NO_LAST_PRICE),
        ),
        (
            "dual 10% deviation",
            s_dual_deviation,
            Some(errors::UNSAFE_PRICE),
        ),
        (
            "dual stale spot leg",
            s_dual_stale_leg,
            Some(errors::PRICE_FEED_STALE),
        ),
        (
            "dual partial (spot missing)",
            s_dual_partial,
            Some(errors::UNSAFE_PRICE),
        ),
        (
            "scaled factor out of band",
            s_scaled_factor_out_of_band,
            Some(OracleError::FactorOutOfBounds as u32),
        ),
        (
            "scaled stale nested quote",
            s_scaled_stale_quote,
            Some(errors::PRICE_FEED_STALE),
        ),
        (
            "scaled cycle (seeded)",
            s_scaled_cycle,
            Some(OracleError::OracleCycleDetected as u32),
        ),
        (
            "TWAP spacing 299 s",
            s_twap_spacing_299,
            Some(errors::NO_LAST_PRICE),
        ),
        (
            "spot observation at +61 s",
            s_spot_future_61,
            Some(errors::NO_LAST_PRICE),
        ),
    ];

    for (name, build, expected) in scenarios {
        let (t, key) = build();
        assert_eq!(price_code(&t, &key), expected, "{name}: prices error code");
        assert_eq!(quote_code(&t, &key), expected, "{name}: quotes error code");
        let status = quote(&t, &key);
        assert_eq!(
            status.valid,
            expected.is_none(),
            "{name}: quote validity {status:?}"
        );
        if expected.is_none() {
            let priced = try_price(&t, &key).expect("good case prices");
            assert_eq!(status.final_wad, priced.price_wad, "{name}: value agrees");
            assert_eq!(priced.price_wad, usd(1), "{name}: value");
        }
    }
}

// ---------------------------------------------------------------------------
// H9: fail-closed controller views when USDC's price is invalid.
// ---------------------------------------------------------------------------

/// With USDC stale, `get_market_indexes_detailed` returns a row marked invalid and
/// keeps ETH's row valid. Valuation views (health factor, total collateral) revert
/// with PriceFeedStale. Supply needs no price and works, including for an account
/// with ETH debt. Borrow fails closed.
#[test]
fn rv_market_indexes_flag_invalid_usdc_health_factor_reverts_supply_still_works() {
    let mut t = LendingTest::new().standard_two_asset().build();
    set_time(&t, T0);
    t.refresh_oracle_prices();
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 1.0);
    let alice = t.account_id(ALICE);
    let usdc = t.resolve_asset("USDC");
    let eth = t.resolve_asset("ETH");

    let mock = t.mock_reflector_client();
    mock.set_price_at(&usdc, &usd(1), &at(-2_000));
    mock.set_twap_price_at(&usdc, &usd(1), &at(-2_000));

    let assets = vec![&t.env, hub_asset(usdc.clone()), hub_asset(eth.clone())];
    let rows = t.ctrl_client().get_market_indexes_detailed(&assets);
    let usdc_row = rows.get(0).expect("USDC row");
    let eth_row = rows.get(1).expect("ETH row");
    assert!(!usdc_row.valid, "USDC row invalid: {usdc_row:?}");
    assert_eq!(usdc_row.error_code, Some(errors::PRICE_FEED_STALE));
    assert_eq!(usdc_row.price_wad, usd(1), "diagnostic candidate");
    assert!(eth_row.valid, "ETH row valid: {eth_row:?}");
    assert_eq!(eth_row.error_code, None);

    assert_contract_error(
        flat(t.ctrl_client().try_get_health_factor(&alice)),
        errors::PRICE_FEED_STALE,
    );
    assert_contract_error(
        flat(t.ctrl_client().try_get_total_collateral_usd(&alice)),
        errors::PRICE_FEED_STALE,
    );

    let before = t.supply_balance_raw(ALICE, "USDC");
    t.try_supply(ALICE, "USDC", 500.0)
        .expect("supply with a stale USDC price and ETH debt");
    assert_eq!(t.supply_balance_raw(ALICE, "USDC") - before, 500 * ONE_USDC);

    assert_contract_error(t.try_borrow(ALICE, "ETH", 0.1), errors::PRICE_FEED_STALE);
}
