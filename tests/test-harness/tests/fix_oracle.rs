//! Regression tests for the price-aggregator fixes on top of PR #196, ported
//! from the PR #196 verification (checks numbered `oracle-N`). Each test
//! asserts the safe outcome:
//!
//! - oracle-1, oracle-2 (CR-7, INV-ORACLE-05): a contract shared by a pair's
//!   legs that also serves one leg of the quote key is held to the quote's
//!   tolerance, so a 1.16 ratio, or a 25% tolerance edit on the shared quote,
//!   is refused #226.
//! - oracle-4, oracle-5 (CR-10): a Reflector budget covers `(records + 2)`
//!   resolutions (two for a spot read), so reads stay fresh while Reflector
//!   still serves its last round.
//! - oracle-6, oracle-extra-1 (CR-6), review ORA-2: `MAX_ORACLE_KEYS` keeps
//!   the registry scan of a revalidating `set_oracle` / `set_tolerance` under
//!   the SDK 28 mainnet limits, measured on the real aggregator WASM for
//!   chains and for dependents with two `Scaled` legs at the depth limit.
//! - oracle-11: feeds report at least eight decimals.
//! - oracle-3, -7, -8, -9, -12 pin behaviour PR #196 already got right.
//!
//! Every scenario uses production entry points (governance
//! `ConfigureAssetOracle` / `EditOracleTolerance`, the aggregator's
//! `set_oracle`, `prices` / `quotes` / `price_spread`, and the controller's
//! `supply` / `borrow`), except where a shape admission refuses has to be
//! seeded with the testing-only `seed_oracle`.

use common::errors::OracleError;
use common::oracle::observation::MAX_LEG_AGE_SPREAD_SECONDS;
use common::types::{OracleTolerance, PriceFeedRaw, PriceStatus, MAX_RESOLUTION_DEPTH};
use controller::constants::MAX_REASONABLE_PRICE_WAD;
use controller::types::{
    AssetOracle, FeedNature, FeedSource, IndependencePolicy, MultiFeedRef, OracleAssetRef,
    OracleReadMode, PriceKey, PriceSource, ProviderRef, ReflectorFeedRef, ScaledSource,
};
use governance::op::{AdminOperation, ConfigureAssetOracleArgs, EditToleranceArgs};
use price_aggregator::PriceAggregatorClient;
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::xdr::ScErrorType;
use soroban_sdk::{vec, Address, Env, String, Symbol, Vec};
use test_harness::errors;
use test_harness::mock_redstone::{MockRedStonePriceFeed, MockRedStonePriceFeedClient};
use test_harness::mock_reflector::{MockReflector, MockReflectorClient};
use test_harness::oracle::redstone::register_redstone_adapter;
use test_harness::{
    assert_contract_error, redstone_single_config, tight_single_source_band, tolerance_band, usd,
    usd_frac, LendingTest, ALICE, BOB, DEFAULT_MAX_SANITY_PRICE_WAD, DEFAULT_MIN_SANITY_PRICE_WAD,
};

const T0: u64 = 100_000;
const RESOLUTION: u64 = 300;
const TWAP3: OracleReadMode = OracleReadMode::Twap(3);
/// The smallest budget admission accepts for `Twap(3)` at 300 s:
/// `(3 + 2) * 300`.
const TWAP3_BUDGET: u64 = 1_500;
/// The minimum `Twap(3)` budget under PR #196's `(records + 1)` rule.
const PLUS_ONE_TWAP3_MIN: u64 = 1_200;
const SLOW_BUDGET: u64 = 57_600;
const REFLECTOR_DECIMALS: u32 = 14;
const REDSTONE_DECIMALS: u32 = 8;
const WAD: i128 = 1_000_000_000_000_000_000;
/// Mirrors the crate-private `registry::MAX_ORACLE_KEYS` (256 in PR #196,
/// 128 before review finding ORA-2 measured the branching dependents).
const MAX_ORACLE_KEYS: u32 = 112;
/// `(BPS + 1_000) / (BPS - 1_000)`: the widest max/min a single-source band
/// admits, 11/9, in WAD.
const CAP_RATIO_WAD: i128 = 11 * WAD / 9;

const INVALID_STALENESS_CONFIG: u32 = OracleError::InvalidStalenessConfig as u32;
const INVALID_ORACLE_DECIMALS: u32 = OracleError::InvalidOracleDecimals as u32;
const INVALID_ORACLE_RESOLUTION: u32 = OracleError::InvalidOracleResolution as u32;
const BAND_TOO_WIDE: u32 = OracleError::SanityBandTooWideForSingleSource as u32;
const ORACLE_REGISTRY_FULL: u32 = OracleError::OracleRegistryFull as u32;
const DEPTH: u32 = OracleError::OracleDepthExceeded as u32;

/// soroban-sdk 28.0.0 `InvocationResourceLimits::mainnet()`.
const MAINNET_LEDGER_ENTRIES: u32 = 400;
const MAINNET_INSTRUCTIONS: i64 = 400_000_000;
const MAINNET_MEM_BYTES: i64 = 41_943_040;

fn at(offset: i64) -> u64 {
    (T0 as i64 + offset) as u64
}

fn set_time(t: &LendingTest, ts: u64) {
    t.env.ledger().with_mut(|li| li.timestamp = ts);
}

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

/// Outcome of a call that may trap: `Ok`, a typed contract error, or anything
/// else (host error type or an abort), spelled out for the record.
#[derive(Debug, PartialEq)]
enum Outcome {
    Ok,
    Contract(u32),
    Other(std::string::String),
}

fn outcome<T, E: Into<soroban_sdk::Error>>(
    result: Result<Result<T, E>, Result<soroban_sdk::Error, soroban_sdk::InvokeError>>,
) -> Outcome {
    let err = match result {
        Ok(Ok(_)) => return Outcome::Ok,
        Ok(Err(err)) => err.into(),
        Err(Ok(err)) => err,
        Err(Err(invoke)) => return Outcome::Other(format!("{invoke:?}")),
    };
    if err.is_type(ScErrorType::Contract) {
        Outcome::Contract(err.get_code())
    } else {
        Outcome::Other(format!("{err:?}"))
    }
}

fn configure_op(key: &PriceKey, oracle: AssetOracle) -> AdminOperation {
    AdminOperation::ConfigureAssetOracle(ConfigureAssetOracleArgs {
        key: key.clone(),
        oracle,
    })
}

fn try_configure(
    t: &LendingTest,
    key: &PriceKey,
    oracle: AssetOracle,
) -> Result<(), soroban_sdk::Error> {
    flat(
        t.gov_client()
            .try_execute_immediate(&t.admin, &configure_op(key, oracle)),
    )
    .map(|_| ())
}

fn configure(t: &LendingTest, key: &PriceKey, oracle: AssetOracle) {
    try_configure(t, key, oracle).expect("admission accepts the configuration");
}

fn try_edit_tolerance(
    t: &LendingTest,
    key: &PriceKey,
    tolerance_bps: u32,
) -> Result<(), soroban_sdk::Error> {
    flat(t.gov_client().try_execute_immediate(
        &t.admin,
        &AdminOperation::EditOracleTolerance(EditToleranceArgs {
            key: key.clone(),
            tolerance: tolerance_bps,
        }),
    ))
    .map(|_| ())
}

fn seed(t: &LendingTest, key: &PriceKey, oracle: &AssetOracle) {
    t.price_agg_client().seed_oracle(key, oracle);
}

fn quote(t: &LendingTest, key: &PriceKey) -> PriceStatus {
    let keys = vec![&t.env, key.clone()];
    t.price_agg_client()
        .quotes(&keys)
        .get(key.clone())
        .expect("key quoted")
}

fn try_price(t: &LendingTest, key: &PriceKey) -> Result<PriceFeedRaw, soroban_sdk::Error> {
    let keys = vec![&t.env, key.clone()];
    flat(t.price_agg_client().try_prices(&keys)).map(|map| map.get(key.clone()).expect("priced"))
}

fn reflector_feed(
    contract: &Address,
    asset: &Address,
    read_mode: OracleReadMode,
    max_stale_seconds: u64,
) -> FeedSource {
    FeedSource {
        provider: ProviderRef::Reflector(ReflectorFeedRef {
            contract: contract.clone(),
            asset: OracleAssetRef::Stellar(asset.clone()),
            read_mode,
        }),
        decimals: REFLECTOR_DECIMALS,
        max_stale_seconds,
    }
}

fn redstone_feed(
    env: &Env,
    adapter: &Address,
    feed_id: &str,
    nature: FeedNature,
    max_stale_seconds: u64,
) -> FeedSource {
    FeedSource {
        provider: ProviderRef::RedStone(MultiFeedRef {
            contract: adapter.clone(),
            feed_id: String::from_str(env, feed_id),
            nature,
        }),
        decimals: REDSTONE_DECIMALS,
        max_stale_seconds,
    }
}

fn scaled(
    factor: FeedSource,
    quote: PriceKey,
    min_factor_wad: i128,
    max_factor_wad: i128,
) -> PriceSource {
    PriceSource::Scaled(ScaledSource {
        factor,
        quote,
        min_factor_wad,
        max_factor_wad,
    })
}

#[allow(clippy::too_many_arguments)]
fn oracle(
    env: &Env,
    items: &[PriceSource],
    max_price_stale_seconds: u64,
    tolerance_bps: u32,
    independence: IndependencePolicy,
    min_sanity_price_wad: i128,
    max_sanity_price_wad: i128,
) -> AssetOracle {
    let mut sources = Vec::new(env);
    for item in items {
        sources.push_back(item.clone());
    }
    AssetOracle {
        asset_decimals: 7,
        max_price_stale_seconds,
        sources,
        tolerance: tolerance_band(env, tolerance_bps),
        independence,
        min_sanity_price_wad,
        max_sanity_price_wad,
    }
}

fn new_reflector(t: &LendingTest, base: Option<&Address>) -> Address {
    let addr = t.env.register(MockReflector, ());
    if let Some(base) = base {
        MockReflectorClient::new(&t.env, &addr).set_base_stellar(base);
    }
    addr
}

/// Stamps the newest TWAP sample; the mock returns `records` samples one
/// round apart behind it.
fn twap_at(t: &LendingTest, reflector: &Address, asset: &Address, price_wad: i128, ts: u64) {
    MockReflectorClient::new(&t.env, reflector).set_twap_price_at(asset, &price_wad, &ts);
}

fn spot_at(t: &LendingTest, reflector: &Address, asset: &Address, price_wad: i128, ts: u64) {
    MockReflectorClient::new(&t.env, reflector).set_price_at(asset, &price_wad, &ts);
}

fn redstone_at(t: &LendingTest, adapter: &Address, feed_id: &str, price_wad: i128, ts: u64) {
    let ms = ts * 1_000;
    MockRedStonePriceFeedClient::new(&t.env, adapter).set_price_data(
        &String::from_str(&t.env, feed_id),
        &price_wad,
        &ms,
        &ms,
    );
}

fn fresh_fixture() -> LendingTest {
    let t = LendingTest::new().three_asset_usdc_eth_wbtc().build();
    set_time(&t, T0);
    t.refresh_oracle_prices();
    t
}

fn assert_fresh(t: &LendingTest, key: &PriceKey, label: &str) -> PriceFeedRaw {
    let status = quote(t, key);
    assert!(status.valid && !status.stale, "{label}: {status:?}");
    try_price(t, key).unwrap_or_else(|err| panic!("{label}: strict read refused: {err:?}"))
}

fn assert_stale(t: &LendingTest, key: &PriceKey, label: &str) {
    let status = quote(t, key);
    assert!(status.stale && !status.valid, "{label}: {status:?}");
    assert_eq!(status.error_code, Some(errors::PRICE_FEED_STALE), "{label}");
    assert_contract_error(try_price(t, key), errors::PRICE_FEED_STALE);
}

/// `num / den` in WAD, for ratios of two WAD prices.
fn ratio_wad(num: i128, den: i128) -> i128 {
    num * 1_000_000 / (den / 1_000_000_000_000)
}

// ---------------------------------------------------------------------------
// CR-7 fixtures: a SolvBTC-shaped pair quoting WBTC, WBTC dual on Reflector +
// the same RedStone adapter X.
// ---------------------------------------------------------------------------

const RATIO_FEED: &str = "SOLV_RATIO";
const USD_FEED: &str = "SOLV_USD";
const QUOTE_FEED: &str = "WBTCF";
const FACTOR_BUDGET: u64 = 93_600;
const ANCHOR_BUDGET: u64 = 57_600;
const BAND_MIN: i128 = 20_000;
const BAND_MAX: i128 = 200_000;
const BTC: i128 = 60_000;

fn solv_pair(t: &LendingTest, adapter: &Address, max_factor_wad: i128) -> AssetOracle {
    let wbtc = PriceKey::Token(t.resolve_asset("WBTC"));
    oracle(
        &t.env,
        &[
            scaled(
                redstone_feed(
                    &t.env,
                    adapter,
                    RATIO_FEED,
                    FeedNature::Fundamental,
                    FACTOR_BUDGET,
                ),
                wbtc,
                WAD,
                max_factor_wad,
            ),
            PriceSource::Feed(redstone_feed(
                &t.env,
                adapter,
                USD_FEED,
                FeedNature::Fundamental,
                ANCHOR_BUDGET,
            )),
        ],
        FACTOR_BUDGET,
        500,
        IndependencePolicy::AllowShared(vec![&t.env, adapter.clone()]),
        usd(BAND_MIN),
        usd(BAND_MAX),
    )
}

fn wbtc_dual(t: &LendingTest, adapter: &Address, tolerance_bps: u32) -> AssetOracle {
    let wbtc = t.resolve_asset("WBTC");
    oracle(
        &t.env,
        &[
            PriceSource::Feed(reflector_feed(
                &t.mock_reflector,
                &wbtc,
                TWAP3,
                TWAP3_BUDGET,
            )),
            PriceSource::Feed(redstone_feed(
                &t.env,
                adapter,
                QUOTE_FEED,
                FeedNature::Fundamental,
                SLOW_BUDGET,
            )),
        ],
        SLOW_BUDGET,
        tolerance_bps,
        IndependencePolicy::RequireDisjoint,
        DEFAULT_MIN_SANITY_PRICE_WAD,
        DEFAULT_MAX_SANITY_PRICE_WAD,
    )
}

/// X publishes `ratio`, the WBTC fundamental and the SolvBTC USD anchor at
/// the current ledger time; Reflector keeps WBTC at 60_000.
fn publish_x(t: &LendingTest, adapter: &Address, ratio: i128, wbtcf: i128, solv_usd: i128) {
    let now = t.env.ledger().timestamp();
    redstone_at(t, adapter, RATIO_FEED, ratio, now);
    redstone_at(t, adapter, QUOTE_FEED, wbtcf, now);
    redstone_at(t, adapter, USD_FEED, solv_usd, now);
}

// ---------------------------------------------------------------------------
// oracle-1: CR-7 nested-quote reach with PR #196's own 1.16 fixture.
// ---------------------------------------------------------------------------

/// SAFE: the 1.16 pair is refused #226 (its true reach, 1.16 x 1.10 x 1.05 =
/// 1.340, exceeds 11/9), or, if admitted, the shared adapter cannot move the
/// strict price past max/min 11/9. The attack drives both extremes through the production
/// `prices` path and the controller's borrow.
#[test]
fn oracle_1_shared_adapter_reach_through_the_quote_is_bounded() {
    let mut t = fresh_fixture();
    let eth_key = PriceKey::Token(t.resolve_asset("ETH"));
    let wbtc_key = PriceKey::Token(t.resolve_asset("WBTC"));
    let x = register_redstone_adapter(
        &t,
        &[
            (RATIO_FEED, WAD),
            (USD_FEED, usd(BTC)),
            (QUOTE_FEED, usd(BTC)),
        ],
    );
    configure(&t, &wbtc_key, wbtc_dual(&t, &x, 1_000));
    let admitted = try_configure(&t, &eth_key, solv_pair(&t, &x, 116 * WAD / 100));
    std::println!("oracle-1: try_configure(1.16 pair) = {admitted:?}");
    if let Err(err) = admitted {
        assert_contract_error(Err::<(), _>(err), BAND_TOO_WIDE);
        return;
    }
    let honest = assert_fresh(&t, &eth_key, "honest").price_wad;
    assert_eq!(honest, usd(BTC));

    // High side: ratio at its 1.16 ceiling, WBTC fundamental +10% over
    // Reflector (inside WBTC's 10% tolerance), USD anchor +5% over leg one.
    publish_x(&t, &x, 116 * WAD / 100, usd(66_000), usd(76_734));
    let wbtc_high = assert_fresh(&t, &wbtc_key, "WBTC high").price_wad;
    let high_status = quote(&t, &eth_key);
    let high = assert_fresh(&t, &eth_key, "ETH high").price_wad;
    std::println!(
        "oracle-1 high: WBTC {} ETH {} legs {} / {}",
        wbtc_high / WAD,
        high / WAD,
        high_status.primary_wad / WAD,
        high_status.secondary_wad / WAD
    );

    // The lending effect: 1 ETH borrows 52_000 USDC at the inflated price
    // (LTV 75% of ~74_907), which the honest 60_000 (LTV cap 45_000) refuses.
    t.supply(BOB, "USDC", 200_000.0);
    t.supply(ALICE, "ETH", 1.0);
    let borrowed = t.try_borrow(ALICE, "USDC", 52_000.0);
    std::println!("oracle-1: borrow 52_000 USDC at the high price = {borrowed:?}");

    // Low side: ratio 1.00, WBTC fundamental -9.1%, anchor -4.76% under leg one.
    publish_x(&t, &x, WAD, usd(54_545), usd(54_546));
    let low_status = quote(&t, &eth_key);
    let low = assert_fresh(&t, &eth_key, "ETH low").price_wad;
    std::println!(
        "oracle-1 low: ETH {} legs {} / {}",
        low / WAD,
        low_status.primary_wad / WAD,
        low_status.secondary_wad / WAD
    );

    // Honest again: the account opened at the high price is under water.
    publish_x(&t, &x, WAD, usd(BTC), usd(BTC));
    assert_eq!(
        assert_fresh(&t, &eth_key, "honest again").price_wad,
        usd(BTC)
    );
    let hf = t.health_factor(ALICE);
    std::println!("oracle-1: HF at honest prices after the high-price borrow = {hf}");

    let reach = ratio_wad(high, low);
    assert!(
        reach <= CAP_RATIO_WAD,
        "oracle-1: the 1.16 pair was admitted (#226 expected) and the shared adapter moves \
         the strict price between {} and {} around an honest {} (max/min {}.{:04} > 11/9); \
         borrow at the high price = {:?}, HF at honest prices = {}",
        low / WAD,
        high / WAD,
        BTC,
        reach / WAD,
        (reach % WAD) / 100_000_000_000_000,
        borrowed.is_ok(),
        hf
    );
}

// ---------------------------------------------------------------------------
// oracle-2: CR-7 governed tolerance edit on the shared quote.
// ---------------------------------------------------------------------------

fn solv_and_wbtc(t: &LendingTest) -> (PriceKey, PriceKey, Address) {
    let eth_key = PriceKey::Token(t.resolve_asset("ETH"));
    let wbtc_key = PriceKey::Token(t.resolve_asset("WBTC"));
    let x = register_redstone_adapter(
        t,
        &[
            (RATIO_FEED, WAD),
            (USD_FEED, usd(BTC)),
            (QUOTE_FEED, usd(BTC)),
        ],
    );
    configure(t, &eth_key, solv_pair(t, &x, 105 * WAD / 100));
    configure(t, &wbtc_key, wbtc_dual(t, &x, 500));
    (eth_key, wbtc_key, x)
}

/// SAFE: widening the shared WBTC quote to 25% is refused #226 through the
/// SolvBTC-shaped dependent (1.05 x 1.25 x 1.05 = 1.378 > 11/9), and both
/// keys keep pricing at 60_000. If the edit is admitted, the test drives the
/// two extremes and reports them in the failing assertion.
#[test]
fn oracle_2_a_quote_tolerance_edit_is_held_to_the_dependents_reach() {
    let t = fresh_fixture();
    let (eth_key, wbtc_key, x) = solv_and_wbtc(&t);
    let edit = try_edit_tolerance(&t, &wbtc_key, 2_500);
    std::println!("oracle-2: EditOracleTolerance(WBTC, 2_500) = {edit:?}");
    if edit.is_ok() {
        publish_x(&t, &x, 105 * WAD / 100, usd(75_000), usd(74_418));
        let high = assert_fresh(&t, &eth_key, "ETH high").price_wad;
        publish_x(&t, &x, WAD, usd(48_000), usd(51_429));
        let low = assert_fresh(&t, &eth_key, "ETH low").price_wad;
        let reach = ratio_wad(high, low);
        panic!(
            "oracle-2: EditOracleTolerance(WBTC, 2_500) was admitted (#226 expected); the \
             SolvBTC-shaped pair then prices between {} and {} around an honest {} \
             (max/min {}.{:04} > 11/9)",
            low / WAD,
            high / WAD,
            BTC,
            reach / WAD,
            (reach % WAD) / 100_000_000_000_000
        );
    }
    assert_contract_error(edit, BAND_TOO_WIDE);
    assert_eq!(assert_fresh(&t, &wbtc_key, "quote").price_wad, usd(BTC));
    assert_eq!(assert_fresh(&t, &eth_key, "dependent").price_wad, usd(BTC));
}

/// Control for oracle-2: a 1_082 bps edit is admitted, since
/// 1.05 x 1.1083 x 1.0501 stays inside 11/9.
#[test]
fn oracle_2_control_a_1082_bps_quote_edit_is_admitted() {
    let t = fresh_fixture();
    let (eth_key, wbtc_key, _) = solv_and_wbtc(&t);
    try_edit_tolerance(&t, &wbtc_key, 1_082).expect("1_082 bps admitted");
    assert_eq!(assert_fresh(&t, &eth_key, "dependent").price_wad, usd(BTC));
}

// ---------------------------------------------------------------------------
// oracle-3: CR-7 arithmetic on malformed shared factor bounds.
// ---------------------------------------------------------------------------

/// SAFE: malformed factor bounds on the shared Scaled factor fail with a
/// typed `Error(Contract, n)`, never a host trap. Records `n` per case.
#[test]
fn oracle_3_malformed_shared_factor_bounds_fail_typed() {
    let t = fresh_fixture();
    let eth_key = PriceKey::Token(t.resolve_asset("ETH"));
    let x = register_redstone_adapter(&t, &[(RATIO_FEED, WAD), (USD_FEED, usd(BTC))]);
    let cases: [(&str, i128, i128); 3] = [
        ("a: min 0, max 1.05", 0, 105 * WAD / 100),
        ("b: min = max = 1e36", 10i128.pow(36), 10i128.pow(36)),
        ("c: min 1, max MAX_REASONABLE", 1, MAX_REASONABLE_PRICE_WAD),
    ];
    let (tight_min, tight_max) = tight_single_source_band(usd(BTC));
    let mut report = std::vec::Vec::new();
    for (label, min_factor, max_factor) in cases {
        for (band_label, band) in [
            ("wide", (usd(BAND_MIN), usd(BAND_MAX))),
            ("tight", (tight_min, tight_max)),
        ] {
            let mut cfg = solv_pair(&t, &x, 105 * WAD / 100);
            let PriceSource::Scaled(mut leg) = cfg.sources.get_unchecked(0) else {
                panic!("first leg is scaled");
            };
            leg.min_factor_wad = min_factor;
            leg.max_factor_wad = max_factor;
            cfg.sources.set(0, PriceSource::Scaled(leg));
            cfg.min_sanity_price_wad = band.0;
            cfg.max_sanity_price_wad = band.1;
            let got = outcome(
                t.gov_client()
                    .try_execute_immediate(&t.admin, &configure_op(&eth_key, cfg)),
            );
            std::println!("oracle-3 {label} / {band_label} band: {got:?}");
            report.push((label, band_label, got));
        }
    }
    for (label, band_label, got) in &report {
        assert!(
            matches!(got, Outcome::Contract(_)),
            "oracle-3 {label} / {band_label}: expected a typed contract error, got {got:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// oracle-4: CR-10 TWAP liveness margin.
// ---------------------------------------------------------------------------

fn twap3_single(t: &LendingTest, reflector: &Address, budget: u64) -> AssetOracle {
    let eth = t.resolve_asset("ETH");
    let (min_wad, max_wad) = tight_single_source_band(usd(2_000));
    oracle(
        &t.env,
        &[PriceSource::Feed(reflector_feed(
            reflector, &eth, TWAP3, budget,
        ))],
        budget,
        500,
        IndependencePolicy::RequireDisjoint,
        min_wad,
        max_wad,
    )
}

/// Upstream shape: the newest sample one round back emulates Reflector's
/// `records + 1` history with its last round at `T0`, oldest at `T0 - 900`.
/// SAFE: the `(records + 1)` minimum (1_200) is either refused #222 or stays
/// fresh while Reflector still serves the `T0` round (through `T0 + 599`).
#[test]
fn oracle_4_twap_at_the_minimum_budget_is_fresh_while_reflector_serves() {
    let t = fresh_fixture();
    let eth = t.resolve_asset("ETH");
    let eth_key = PriceKey::Token(eth.clone());
    let reflector = new_reflector(&t, None);
    twap_at(&t, &reflector, &eth, usd(2_000), at(-(RESOLUTION as i64)));

    assert_contract_error(
        try_configure(
            &t,
            &eth_key,
            twap3_single(&t, &reflector, PLUS_ONE_TWAP3_MIN - 1),
        ),
        INVALID_ORACLE_RESOLUTION,
    );
    let admitted = try_configure(
        &t,
        &eth_key,
        twap3_single(&t, &reflector, PLUS_ONE_TWAP3_MIN),
    );
    std::println!("oracle-4: try_configure(Twap(3), 1_200) = {admitted:?}");
    if let Err(err) = admitted {
        assert_contract_error(Err::<(), _>(err), INVALID_ORACLE_RESOLUTION);
        return;
    }
    let feed = assert_fresh(&t, &eth_key, "round instant");
    assert_eq!(feed.timestamp, at(-(3 * RESOLUTION as i64)));
    for offset in [RESOLUTION as i64 + 1, 2 * RESOLUTION as i64 - 1] {
        set_time(&t, at(offset));
        let status = quote(&t, &eth_key);
        std::println!("oracle-4 at T0+{offset}: {status:?}");
        assert!(
            status.valid && !status.stale,
            "oracle-4: admitted at 1_200 s, the Twap(3) read is stale at T0+{offset} while \
             Reflector still serves its T0 round: {status:?}, prices = {:?}",
            try_price(&t, &eth_key)
        );
    }
}

/// Control: at 1_500 s the read stays fresh through `T0 + 599` and goes
/// stale at `T0 + 601`.
#[test]
fn oracle_4_control_twap_at_1500_is_fresh_through_the_serving_window() {
    let t = fresh_fixture();
    let eth = t.resolve_asset("ETH");
    let eth_key = PriceKey::Token(eth.clone());
    let reflector = new_reflector(&t, None);
    twap_at(&t, &reflector, &eth, usd(2_000), at(-(RESOLUTION as i64)));
    configure(&t, &eth_key, twap3_single(&t, &reflector, TWAP3_BUDGET));
    assert_fresh(&t, &eth_key, "round instant");
    set_time(&t, at(2 * RESOLUTION as i64 - 1));
    assert_fresh(&t, &eth_key, "T0+599");
    set_time(&t, at(2 * RESOLUTION as i64 + 1));
    assert_stale(&t, &eth_key, "T0+601");
}

// ---------------------------------------------------------------------------
// oracle-5: CR-10 spot-mode sibling.
// ---------------------------------------------------------------------------

fn spot_pair(t: &LendingTest, spot: &Address, adapter: &Address, budget: u64) -> AssetOracle {
    let eth = t.resolve_asset("ETH");
    oracle(
        &t.env,
        &[
            PriceSource::Feed(reflector_feed(spot, &eth, OracleReadMode::Spot, budget)),
            PriceSource::Feed(redstone_feed(
                &t.env,
                adapter,
                "ETHF",
                FeedNature::Fundamental,
                SLOW_BUDGET,
            )),
        ],
        SLOW_BUDGET,
        500,
        IndependencePolicy::RequireDisjoint,
        DEFAULT_MIN_SANITY_PRICE_WAD,
        DEFAULT_MAX_SANITY_PRICE_WAD,
    )
}

/// SAFE per budget: refused #222, or fresh at `T0`, `T0 + 301` and
/// `T0 + 599` (Reflector serves the `T0` round until `T0 + 600`).
fn spot_budget_is_live(budget: u64) {
    let t = fresh_fixture();
    let eth = t.resolve_asset("ETH");
    let eth_key = PriceKey::Token(eth.clone());
    let spot = new_reflector(&t, None);
    let adapter = register_redstone_adapter(&t, &[("ETHF", usd(2_000))]);
    spot_at(&t, &spot, &eth, usd(2_000), at(0));
    let admitted = try_configure(&t, &eth_key, spot_pair(&t, &spot, &adapter, budget));
    std::println!("oracle-5: try_configure(Spot, {budget}) = {admitted:?}");
    if let Err(err) = admitted {
        assert_contract_error(Err::<(), _>(err), INVALID_ORACLE_RESOLUTION);
        return;
    }
    for offset in [0i64, RESOLUTION as i64 + 1, 2 * RESOLUTION as i64 - 1] {
        set_time(&t, at(offset));
        let status = quote(&t, &eth_key);
        std::println!("oracle-5 budget {budget} at T0+{offset}: {status:?}");
        assert!(
            status.valid && !status.stale,
            "oracle-5: Spot leg admitted at {budget} s is stale at T0+{offset} while Reflector \
             still serves its T0 round: {status:?}, prices = {:?}",
            try_price(&t, &eth_key)
        );
    }
}

#[test]
fn oracle_5_spot_budget_300() {
    spot_budget_is_live(RESOLUTION);
}

#[test]
fn oracle_5_spot_budget_599() {
    spot_budget_is_live(2 * RESOLUTION - 1);
}

/// 600 s, the smallest spot budget admission accepts: fresh through
/// `T0 + 599`, stale at `T0 + 601`.
#[test]
fn oracle_5_spot_budget_600() {
    spot_budget_is_live(2 * RESOLUTION);
    let t = fresh_fixture();
    let eth = t.resolve_asset("ETH");
    let eth_key = PriceKey::Token(eth.clone());
    let spot = new_reflector(&t, None);
    let adapter = register_redstone_adapter(&t, &[("ETHF", usd(2_000))]);
    spot_at(&t, &spot, &eth, usd(2_000), at(0));
    configure(&t, &eth_key, spot_pair(&t, &spot, &adapter, 2 * RESOLUTION));
    set_time(&t, at(2 * RESOLUTION as i64 + 1));
    assert_stale(&t, &eth_key, "T0+601");
}

// ---------------------------------------------------------------------------
// oracle-6: CR-6 footprint, CPU and memory at the cap.
// ---------------------------------------------------------------------------

fn ref_template(t: &LendingTest) -> AssetOracle {
    let usdc = t.resolve_asset("USDC");
    let mut oracle = t
        .price_agg_client()
        .oracle(&PriceKey::Token(usdc))
        .expect("USDC oracle seeded by the fixture");
    oracle.asset_decimals = 0;
    oracle
}

fn ref_key(env: &Env, prefix: &str, i: u32) -> PriceKey {
    PriceKey::Ref(Symbol::new(env, &format!("{prefix}{i}")))
}

fn report_resources(env: &Env, label: &str) -> (u32, i64, i64) {
    let r = env.cost_estimate().resources();
    let entries = r.disk_read_entries + r.memory_read_entries + r.write_entries;
    std::println!(
        "oracle-6 {label}: entries {entries} (disk {} mem {} write {}), instructions {}, \
         mem_bytes {}, write_bytes {}, disk_read_bytes {}",
        r.disk_read_entries,
        r.memory_read_entries,
        r.write_entries,
        r.instructions,
        r.mem_bytes,
        r.write_bytes,
        r.disk_read_bytes
    );
    (entries, r.instructions, r.mem_bytes)
}

/// (a) The registry caps at `MAX_ORACLE_KEYS` keys with #233; a replacement
/// at the cap works; a governed tolerance edit at the cap fits 400 entries.
#[test]
fn oracle_6a_the_registry_cap_and_a_tolerance_edit_at_the_cap() {
    let t = LendingTest::new().standard_two_asset().build();
    let template = ref_template(&t);
    let mut registered = 2u32;
    let mut refused = None;
    for i in 0..MAX_ORACLE_KEYS {
        match flat(
            t.price_agg_client()
                .try_set_oracle(&ref_key(&t.env, "RK", i), &template),
        ) {
            Ok(()) => registered += 1,
            Err(err) => {
                refused = Some((i, err));
                break;
            }
        }
    }
    let (first_refused, err) = refused.expect("the registry must cap its index");
    assert_eq!(registered, MAX_ORACLE_KEYS);
    assert_contract_error(Err::<(), _>(err), ORACLE_REGISTRY_FULL);
    t.price_agg_client()
        .set_oracle(&ref_key(&t.env, "RK", 0), &template);
    assert_contract_error(
        flat(
            t.price_agg_client()
                .try_set_oracle(&ref_key(&t.env, "RK", first_refused), &template),
        ),
        ORACLE_REGISTRY_FULL,
    );

    let usdc_key = PriceKey::Token(t.resolve_asset("USDC"));
    try_edit_tolerance(&t, &usdc_key, 1_000).expect("tolerance edit at the cap");
    let (entries, _, _) = report_resources(&t.env, "6a EditOracleTolerance(USDC) at the cap");
    assert!(
        entries < MAINNET_LEDGER_ENTRIES,
        "{entries} entries at the cap"
    );
}

/// A dependent of `quote` (USDC unless a chain says otherwise):
/// `Scaled(RedStone 'R{i}' fundamental, quote)`, single-source, band +-1%.
fn usdc_dependent(env: &Env, adapter: &Address, quote: &PriceKey, i: u32) -> AssetOracle {
    let (min_wad, max_wad) = tight_single_source_band(usd(1));
    let mut o = oracle(
        env,
        &[scaled(
            redstone_feed(
                env,
                adapter,
                &format!("R{i}"),
                FeedNature::Fundamental,
                SLOW_BUDGET,
            ),
            quote.clone(),
            WAD / 2,
            2 * WAD,
        )],
        SLOW_BUDGET,
        500,
        IndependencePolicy::RequireDisjoint,
        min_wad,
        max_wad,
    );
    o.asset_decimals = 0;
    o
}

/// (b) Worst fan-out through the harness (natively registered aggregator, so
/// `instructions` misses VM execution and is a lower bound): dependents of
/// USDC fill the registry to the cap; a governed `EditOracleTolerance(USDC)`
/// and `ConfigureAssetOracle(USDC, same)` revalidate all of them.
#[test]
fn oracle_6b_worst_fan_out_through_governance_fits_mainnet_limits() {
    let t = LendingTest::new().standard_two_asset().build();
    let usdc_key = PriceKey::Token(t.resolve_asset("USDC"));
    let adapter = register_redstone_adapter(&t, &[("R0", WAD)]);
    for i in 0..MAX_ORACLE_KEYS - 2 {
        seed(
            &t,
            &ref_key(&t.env, "D", i),
            &usdc_dependent(&t.env, &adapter, &usdc_key, i),
        );
    }
    try_edit_tolerance(&t, &usdc_key, 1_000).expect("tolerance edit at the cap");
    let edit = report_resources(&t.env, "6b governed EditOracleTolerance(USDC) at the cap");
    let usdc_oracle = t.price_agg_client().oracle(&usdc_key).expect("USDC");
    configure(&t, &usdc_key, usdc_oracle);
    let reconf = report_resources(&t.env, "6b governed ConfigureAssetOracle(USDC) at the cap");
    for (label, (entries, insns, mem)) in [("edit", edit), ("reconfigure", reconf)] {
        assert!(
            entries < MAINNET_LEDGER_ENTRIES,
            "{label}: {entries} entries"
        );
        assert!(
            insns < MAINNET_INSTRUCTIONS,
            "{label}: {insns} instructions"
        );
        assert!(mem < MAINNET_MEM_BYTES, "{label}: {mem} bytes");
    }
}

/// The aggregator WASM the fan-out runs on: `ORACLE_FAN_OUT_WASM`, or the
/// release build `make build` leaves next to the pool WASM the harness loads.
/// The deploy artifact (`make build-one CRATE=price-aggregator`, then
/// `scripts/strip_spec_docs.py`) runs the same code optimized and costs less.
fn read_wasm() -> std::vec::Vec<u8> {
    let path = std::env::var("ORACLE_FAN_OUT_WASM")
        .unwrap_or_else(|_| "target/wasm32v1-none/release/price_aggregator.wasm".to_string());
    for candidate in [path.clone(), format!("../{path}"), format!("../../{path}")] {
        if let Ok(bytes) = std::fs::read(&candidate) {
            std::println!("oracle-6c: loaded {candidate} ({} bytes)", bytes.len());
            return bytes;
        }
    }
    panic!("aggregator WASM not found at {path}");
}

/// Registry size the fan-out fills: `ORACLE_FAN_OUT_KEYS` (to re-measure
/// the cap), or the cap itself.
fn fan_out_keys() -> u32 {
    std::env::var("ORACLE_FAN_OUT_KEYS")
        .ok()
        .and_then(|keys| keys.parse().ok())
        .unwrap_or(MAX_ORACLE_KEYS)
}

/// Shape of the dependents in the WASM fan-out.
#[derive(Clone, Copy, Debug)]
enum FanOut {
    /// Single-source `Scaled(RedStone factor, quote USDC)`, band +-1%.
    SingleSource,
    /// SolvBTC-shaped exempt pair over USDC: `Scaled(X ratio, quote USDC)` +
    /// `X` USD feed, `AllowShared([X])`, wide band.
    SharedPair,
    /// oracle-extra-1: the same pair one quote deeper, quoting the dual
    /// `K0 = Scaled(X factor, quote USDC) + Reflector feed`; editing USDC
    /// revalidates `K0` and every pair through it. `X` reaches one leg of
    /// `K0`, so the pairs stay exempt within `K0`'s tolerance.
    SharedPairDeep,
    /// The same pair at the resolution depth limit, quoting the dual
    /// `K1 = Scaled(X factor, quote K0) + Reflector feed`.
    SharedPairDeepest,
    /// Review ORA-2: two `Scaled` legs on the same depth-limit quote,
    /// `Scaled(R_i, quote K1) + Scaled(U_i, quote K1)`,
    /// `AllowShared([adapter, reflector])`, band +-1%. Validation walks the
    /// quote chain once per leg.
    DoubleScaledDeepest,
    /// Review ORA-2: the first leg quotes a separate depth-limit chain
    /// `L1 -> L0 -> V` that never reaches USDC, the second quotes `K1`:
    /// `Scaled(R_i, quote L1) + Scaled(U_i, quote K1)`. The dependency scan
    /// walks the whole `L` chain before it finds USDC through the second leg.
    SplitQuoteDeepest,
}

impl FanOut {
    /// Intermediate quote keys between USDC and the dependents.
    fn chain(self) -> u32 {
        match self {
            FanOut::SingleSource | FanOut::SharedPair => 0,
            FanOut::SharedPairDeep => 1,
            FanOut::SharedPairDeepest | FanOut::DoubleScaledDeepest | FanOut::SplitQuoteDeepest => {
                2
            }
        }
    }

    /// Keys of the side chain `V`, `L0`, `L1` that never reaches USDC.
    fn side_chain(self) -> u32 {
        match self {
            FanOut::SplitQuoteDeepest => 3,
            _ => 0,
        }
    }
}

/// A dual quote key over `quote`: `Scaled(adapter 'Q{level}', quote)` and a
/// Reflector TWAP leg on `asset`, both at 1 USD, band +-1%.
fn chain_key(
    env: &Env,
    adapter: &Address,
    reflector: &Address,
    asset: &Address,
    quote: &PriceKey,
    level: u32,
) -> AssetOracle {
    let (min_wad, max_wad) = tight_single_source_band(usd(1));
    let mut o = oracle(
        env,
        &[
            scaled(
                redstone_feed(
                    env,
                    adapter,
                    &format!("Q{level}"),
                    FeedNature::Fundamental,
                    SLOW_BUDGET,
                ),
                quote.clone(),
                WAD / 2,
                2 * WAD,
            ),
            PriceSource::Feed(reflector_feed(reflector, asset, TWAP3, TWAP3_BUDGET)),
        ],
        SLOW_BUDGET,
        500,
        IndependencePolicy::AllowShared(vec![env, reflector.clone()]),
        min_wad,
        max_wad,
    );
    o.asset_decimals = 0;
    o
}

/// A SolvBTC-shaped exempt pair over `quote` on `adapter`, priced at 1 USD.
fn shared_pair(env: &Env, adapter: &Address, quote: &PriceKey, i: u32) -> AssetOracle {
    let mut o = oracle(
        env,
        &[
            scaled(
                redstone_feed(
                    env,
                    adapter,
                    &format!("R{i}"),
                    FeedNature::Fundamental,
                    SLOW_BUDGET,
                ),
                quote.clone(),
                WAD,
                105 * WAD / 100,
            ),
            PriceSource::Feed(redstone_feed(
                env,
                adapter,
                &format!("U{i}"),
                FeedNature::Fundamental,
                SLOW_BUDGET,
            )),
        ],
        SLOW_BUDGET,
        500,
        IndependencePolicy::AllowShared(vec![env, adapter.clone()]),
        usd(1) / 2,
        2 * usd(1),
    );
    o.asset_decimals = 0;
    o
}

/// A pair of two `Scaled` legs on `adapter`, the first quoting
/// `first_quote`, the second `second_quote`, priced at 1 USD with a tight
/// band: both legs trust the adapter and the Reflector the quotes read, so
/// the pair is not exempt from the single-source band cap.
fn double_scaled(
    env: &Env,
    adapter: &Address,
    reflector: &Address,
    first_quote: &PriceKey,
    second_quote: &PriceKey,
    i: u32,
) -> AssetOracle {
    let leg = |feed: &str, quote: &PriceKey| {
        scaled(
            redstone_feed(env, adapter, feed, FeedNature::Fundamental, SLOW_BUDGET),
            quote.clone(),
            WAD / 2,
            2 * WAD,
        )
    };
    let (min_wad, max_wad) = tight_single_source_band(usd(1));
    let mut o = oracle(
        env,
        &[
            leg(&format!("R{i}"), first_quote),
            leg(&format!("U{i}"), second_quote),
        ],
        SLOW_BUDGET,
        500,
        IndependencePolicy::AllowShared(vec![env, adapter.clone(), reflector.clone()]),
        min_wad,
        max_wad,
    );
    o.asset_decimals = 0;
    o
}

/// (c) The fan-out against the real aggregator WASM (VM execution metered).
/// USDC, the shape's quote chain and its dependents are admitted through
/// `set_oracle` until the registry holds `keys` keys; at the cap one more new
/// key is refused with #233. Then the owner calls `set_tolerance(USDC)` and
/// `set_oracle(USDC, same)`, which revalidate every dependent. Returns the
/// two measurements as (entries, instructions, memory bytes).
fn wasm_fan_out(shape: FanOut, keys: u32) -> [(u32, i64, i64); 2] {
    let env = Env::default();
    env.mock_all_auths();
    env.cost_estimate().budget().reset_unlimited();
    env.cost_estimate().disable_resource_limits();
    env.ledger().with_mut(|li| {
        li.timestamp = T0;
        li.min_temp_entry_ttl = 1_000_000;
        li.min_persistent_entry_ttl = 1_000_000;
        li.max_entry_ttl = 10_000_000;
    });
    let wasm = read_wasm();
    let owner = Address::generate(&env);
    let agg_id = env.register(wasm.as_slice(), (owner.clone(),));
    let agg = PriceAggregatorClient::new(&env, &agg_id);

    let usdc = Address::generate(&env);
    let usdc_key = PriceKey::Token(usdc.clone());
    let reflector = env.register(MockReflector, ());
    let reflector_client = MockReflectorClient::new(&env, &reflector);
    reflector_client.set_price(&usdc, &usd(1));
    reflector_client.set_twap_price(&usdc, &usd(1));
    let adapter = env.register(MockRedStonePriceFeed, ());
    let adapter_client = MockRedStonePriceFeedClient::new(&env, &adapter);
    let set_feed = |feed: &str, price: i128| {
        adapter_client.set_price(&String::from_str(&env, feed), &price);
    };

    let (min_wad, max_wad) = tight_single_source_band(usd(1));
    let usdc_oracle = oracle(
        &env,
        &[PriceSource::Feed(reflector_feed(
            &reflector,
            &usdc,
            TWAP3,
            TWAP3_BUDGET,
        ))],
        TWAP3_BUDGET,
        500,
        IndependencePolicy::RequireDisjoint,
        min_wad,
        max_wad,
    );
    agg.set_oracle(&usdc_key, &usdc_oracle);

    let mut side_key = usdc_key.clone();
    if shape.side_chain() > 0 {
        let v_asset = Address::generate(&env);
        reflector_client.set_twap_price(&v_asset, &usd(1));
        side_key = PriceKey::Ref(Symbol::new(&env, "V"));
        let mut v_oracle = oracle(
            &env,
            &[PriceSource::Feed(reflector_feed(
                &reflector,
                &v_asset,
                TWAP3,
                TWAP3_BUDGET,
            ))],
            TWAP3_BUDGET,
            500,
            IndependencePolicy::RequireDisjoint,
            min_wad,
            max_wad,
        );
        v_oracle.asset_decimals = 0;
        agg.set_oracle(&side_key, &v_oracle);
        for level in 10..(10 + shape.side_chain() - 1) {
            let l = PriceKey::Ref(Symbol::new(&env, &format!("L{level}")));
            let l_asset = Address::generate(&env);
            reflector_client.set_twap_price(&l_asset, &usd(1));
            set_feed(&format!("Q{level}"), WAD);
            agg.set_oracle(
                &l,
                &chain_key(&env, &adapter, &reflector, &l_asset, &side_key, level),
            );
            side_key = l;
        }
    }
    let mut quote_key = usdc_key.clone();
    for level in 0..shape.chain() {
        let k = PriceKey::Ref(Symbol::new(&env, &format!("K{level}")));
        let k_asset = Address::generate(&env);
        reflector_client.set_twap_price(&k_asset, &usd(1));
        set_feed(&format!("Q{level}"), WAD);
        agg.set_oracle(
            &k,
            &chain_key(&env, &adapter, &reflector, &k_asset, &quote_key, level),
        );
        quote_key = k;
    }
    let dependents = keys - 1 - shape.chain() - shape.side_chain();
    for i in 0..dependents {
        set_feed(&format!("R{i}"), WAD);
        let cfg = match shape {
            FanOut::SingleSource => usdc_dependent(&env, &adapter, &usdc_key, i),
            FanOut::SharedPair | FanOut::SharedPairDeep | FanOut::SharedPairDeepest => {
                set_feed(&format!("U{i}"), usd(1));
                shared_pair(&env, &adapter, &quote_key, i)
            }
            FanOut::DoubleScaledDeepest => {
                set_feed(&format!("U{i}"), WAD);
                double_scaled(&env, &adapter, &reflector, &quote_key, &quote_key, i)
            }
            FanOut::SplitQuoteDeepest => {
                set_feed(&format!("U{i}"), WAD);
                double_scaled(&env, &adapter, &reflector, &side_key, &quote_key, i)
            }
        };
        agg.set_oracle(&ref_key(&env, "W", i), &cfg);
    }
    if keys == MAX_ORACLE_KEYS {
        set_feed("R998", WAD);
        let one_more = flat(agg.try_set_oracle(
            &ref_key(&env, "W", dependents),
            &usdc_dependent(&env, &adapter, &usdc_key, 998),
        ));
        assert_contract_error(one_more, ORACLE_REGISTRY_FULL);
    }

    let tolerance: OracleTolerance = tolerance_band(&env, 1_000);
    agg.set_tolerance(&usdc_key, &tolerance);
    let edit = report_resources(
        &env,
        &format!("6c WASM {shape:?} x {keys} keys set_tolerance(USDC)"),
    );
    let stored = agg.oracle(&usdc_key).expect("USDC");
    agg.set_oracle(&usdc_key, &stored);
    let reconf = report_resources(
        &env,
        &format!("6c WASM {shape:?} x {keys} keys set_oracle(USDC, same)"),
    );
    [edit, reconf]
}

/// Runs the fan-out at the cap (or `ORACLE_FAN_OUT_KEYS`) and holds both
/// revalidating calls to the SDK 28 mainnet transaction limits.
fn assert_fan_out_fits(shape: FanOut) {
    let keys = fan_out_keys();
    let [edit, reconf] = wasm_fan_out(shape, keys);
    for (label, (entries, insns, mem)) in [("set_tolerance", edit), ("set_oracle", reconf)] {
        assert!(
            entries < MAINNET_LEDGER_ENTRIES,
            "{shape:?} x {keys} {label}: {entries} entries"
        );
        assert!(
            insns < MAINNET_INSTRUCTIONS,
            "{shape:?} x {keys} {label}: {insns} instructions"
        );
        assert!(
            mem < MAINNET_MEM_BYTES,
            "{shape:?} x {keys} {label}: {mem} bytes"
        );
    }
}

#[test]
fn oracle_6c_wasm_fan_out_single_source_dependents() {
    assert_fan_out_fits(FanOut::SingleSource);
}

#[test]
fn oracle_6c_wasm_fan_out_shared_pair_dependents() {
    assert_fan_out_fits(FanOut::SharedPair);
}

/// oracle-extra-1: at PR #196's 256 keys this shape cost about 519M
/// instructions on the optimized deploy artifact; the cap and the memoized
/// dependency scan bring it to about 203M.
#[test]
fn oracle_extra_1_wasm_fan_out_shared_pairs_one_quote_deeper() {
    assert_fan_out_fits(FanOut::SharedPairDeep);
}

/// The same pairs two quotes deeper, at the resolution depth limit: the
/// costliest single-chain shape measured, about 246M instructions at the
/// cap.
#[test]
fn oracle_extra_1_wasm_fan_out_shared_pairs_at_the_depth_limit() {
    assert_fan_out_fits(FanOut::SharedPairDeepest);
}

/// Review ORA-2: a dependent whose two `Scaled` legs quote the same
/// depth-limit key, so validation walks the quote chain twice: about 455M
/// instructions at 128 keys before the dependency scan was memoized, 405M
/// after, about 351M at the 112-key cap.
#[test]
fn ora_2_wasm_fan_out_double_scaled_dependents_at_the_depth_limit() {
    assert_fan_out_fits(FanOut::DoubleScaledDeepest);
}

/// Review ORA-2: a dependent whose first leg quotes a separate depth-limit
/// chain that never reaches the edited key, so the dependency scan walks it
/// before it finds the edited key through the second leg: about 559M
/// instructions at 128 keys before the dependency scan was memoized, 412M
/// after, about 357M at the 112-key cap. The costliest shape measured.
#[test]
fn ora_2_wasm_fan_out_split_quote_dependents_at_the_depth_limit() {
    assert_fan_out_fits(FanOut::SplitQuoteDeepest);
}

// ---------------------------------------------------------------------------
// oracle-7: MC-2, the rv_specula_o8 chain (fixed by PR #196).
// ---------------------------------------------------------------------------

fn scaled_on(t: &LendingTest, quote: &PriceKey, name: &str) -> (PriceKey, AssetOracle) {
    let feed_id = String::from_str(&t.env, "RATIO");
    let adapter = register_redstone_adapter(t, &[("RATIO", usd(1))]);
    let mut cfg = redstone_single_config(&t.env, &adapter, &feed_id, usd(1), 500);
    let PriceSource::Feed(factor) = cfg.sources.get_unchecked(0) else {
        unreachable!("redstone_single_config builds one feed source")
    };
    let mut sources: Vec<PriceSource> = vec![&t.env];
    sources.push_back(PriceSource::Scaled(ScaledSource {
        factor,
        quote: quote.clone(),
        min_factor_wad: 1,
        max_factor_wad: DEFAULT_MAX_SANITY_PRICE_WAD,
    }));
    cfg.sources = sources;
    cfg.asset_decimals = 0;
    (PriceKey::Ref(Symbol::new(&t.env, name)), cfg)
}

fn quote_codes(t: &LendingTest, keys: &[&PriceKey]) -> std::vec::Vec<Option<u32>> {
    let mut batch: Vec<PriceKey> = vec![&t.env];
    for key in keys {
        batch.push_back((*key).clone());
    }
    let statuses = t.price_agg_client().quotes(&batch);
    keys.iter()
        .map(|key| statuses.get((*key).clone()).expect("key quoted").error_code)
        .collect()
}

/// `quotes([L4, L2, L1])`: L4 is over-deep (#229); L2 and L1 must stay valid
/// (on HEAD `c1d8bdc` they replayed #229 from the error cache).
#[test]
fn oracle_7_quotes_do_not_replay_a_depth_failure_at_a_shallower_position() {
    assert_eq!(MAX_RESOLUTION_DEPTH, 3);
    let t = LendingTest::new().standard_two_asset().build();
    t.env.ledger().with_mut(|li| li.timestamp = T0);
    t.refresh_oracle_prices();
    let usdc = t.resolve_asset("USDC");
    let mut q = PriceKey::Token(usdc);
    let mut keys: std::vec::Vec<PriceKey> = std::vec::Vec::new();
    for level in 1..=4 {
        let (key, cfg) = scaled_on(&t, &q, &format!("L{level}"));
        if level < 4 {
            try_configure(&t, &key, cfg).unwrap_or_else(|e| panic!("L{level} admitted: {e:?}"));
        } else {
            assert_contract_error(try_configure(&t, &key, cfg.clone()), DEPTH);
            seed(&t, &key, &cfg);
        }
        keys.push(key.clone());
        q = key;
    }
    let (l1, l2, l4) = (&keys[0], &keys[1], &keys[3]);
    assert_eq!(quote_codes(&t, &[l4]), [Some(DEPTH)]);
    let codes = quote_codes(&t, &[l4, l2, l1]);
    std::println!("oracle-7: quotes([L4, L2, L1]) codes = {codes:?}");
    assert_eq!(codes, [Some(DEPTH), None, None], "MC-2 replay");
    let mut batch: Vec<PriceKey> = vec![&t.env];
    batch.push_back(l4.clone());
    batch.push_back(l2.clone());
    let l2_status = t.price_agg_client().quotes(&batch).get(l2.clone()).unwrap();
    assert!(l2_status.valid, "{l2_status:?}");
    assert_eq!(l2_status.final_wad, WAD);
}

// ---------------------------------------------------------------------------
// oracle-8: CR-1 / CR-2 through the session cache, both batch orders.
// ---------------------------------------------------------------------------

fn seeded_cr1_shape(t: &LendingTest) -> (PriceKey, PriceKey) {
    let usdc = t.resolve_asset("USDC");
    let eth = t.resolve_asset("ETH");
    let usdc_key = PriceKey::Token(usdc.clone());
    let eth_key = PriceKey::Token(eth.clone());
    configure(
        t,
        &usdc_key,
        oracle(
            &t.env,
            &[PriceSource::Feed(reflector_feed(
                &t.mock_reflector,
                &usdc,
                TWAP3,
                SLOW_BUDGET,
            ))],
            SLOW_BUDGET,
            500,
            IndependencePolicy::RequireDisjoint,
            usd_frac(85, 100),
            usd_frac(103, 100),
        ),
    );
    let adapter = register_redstone_adapter(t, &[("RATE", usd(2_000))]);
    let partner = new_reflector(t, None);
    twap_at(t, &partner, &eth, usd(2_000), at(0));
    let eth_oracle = oracle(
        &t.env,
        &[
            scaled(
                redstone_feed(
                    &t.env,
                    &adapter,
                    "RATE",
                    FeedNature::Fundamental,
                    SLOW_BUDGET,
                ),
                usdc_key.clone(),
                1,
                MAX_REASONABLE_PRICE_WAD,
            ),
            PriceSource::Feed(reflector_feed(&partner, &eth, TWAP3, TWAP3_BUDGET)),
        ],
        SLOW_BUDGET,
        1_500,
        IndependencePolicy::RequireDisjoint,
        DEFAULT_MIN_SANITY_PRICE_WAD,
        DEFAULT_MAX_SANITY_PRICE_WAD,
    );
    assert_contract_error(
        try_configure(t, &eth_key, eth_oracle.clone()),
        INVALID_STALENESS_CONFIG,
    );
    seed(t, &eth_key, &eth_oracle);
    (usdc_key, eth_key)
}

/// CR-1: the quote's sole market feed is frozen 16 h old; ETH (fundamental
/// factor x USDC, plus a fresh market partner) must be #206 in every batch
/// order, through `prices`, `quotes` and `price_spread`; USDC stays valid.
#[test]
fn oracle_8_cr1_the_session_cache_carries_the_quotes_market_stamp() {
    let t = fresh_fixture();
    let usdc = t.resolve_asset("USDC");
    let (usdc_key, eth_key) = seeded_cr1_shape(&t);
    twap_at(
        &t,
        &t.mock_reflector,
        &usdc,
        usd(1),
        at(2 * RESOLUTION as i64 - SLOW_BUDGET as i64),
    );
    assert!(quote(&t, &usdc_key).valid, "the quote alone is in budget");
    for (label, keys) in [
        (
            "[USDC, ETH]",
            vec![&t.env, usdc_key.clone(), eth_key.clone()],
        ),
        (
            "[ETH, USDC]",
            vec![&t.env, eth_key.clone(), usdc_key.clone()],
        ),
    ] {
        let priced = flat(t.price_agg_client().try_prices(&keys));
        assert_contract_error(priced, errors::PRICE_FEED_STALE);
        let statuses = t.price_agg_client().quotes(&keys);
        let eth_status = statuses.get(eth_key.clone()).expect("ETH quoted");
        assert_eq!(
            eth_status.error_code,
            Some(errors::PRICE_FEED_STALE),
            "{label}: {eth_status:?}"
        );
        assert!(
            statuses.get(usdc_key.clone()).expect("USDC").valid,
            "{label}"
        );
    }
    assert_contract_error(
        flat(t.price_agg_client().try_price_spread(&eth_key)),
        errors::PRICE_FEED_STALE,
    );
}

/// CR-2: USDC dual (Reflector market + RedStone fundamental lagging 5 h); ETH
/// = Market factor x USDC + market partner must price at 2_000 with the
/// blended stamp `T0 - 18_000` in both batch orders.
#[test]
fn oracle_8_cr2_the_session_cache_carries_the_market_leg_not_the_blended_stamp() {
    let t = fresh_fixture();
    let usdc = t.resolve_asset("USDC");
    let eth = t.resolve_asset("ETH");
    let usdc_key = PriceKey::Token(usdc.clone());
    let eth_key = PriceKey::Token(eth.clone());
    let adapter = register_redstone_adapter(&t, &[("USDC", usd(1))]);
    configure(
        &t,
        &usdc_key,
        oracle(
            &t.env,
            &[
                PriceSource::Feed(reflector_feed(
                    &t.mock_reflector,
                    &usdc,
                    TWAP3,
                    TWAP3_BUDGET,
                )),
                PriceSource::Feed(redstone_feed(
                    &t.env,
                    &adapter,
                    "USDC",
                    FeedNature::Fundamental,
                    SLOW_BUDGET,
                )),
            ],
            SLOW_BUDGET,
            500,
            IndependencePolicy::RequireDisjoint,
            DEFAULT_MIN_SANITY_PRICE_WAD,
            DEFAULT_MAX_SANITY_PRICE_WAD,
        ),
    );
    let factor = new_reflector(&t, Some(&usdc));
    twap_at(&t, &factor, &eth, usd(2_000), at(0));
    let partner = new_reflector(&t, None);
    twap_at(&t, &partner, &eth, usd(2_000), at(0));
    configure(
        &t,
        &eth_key,
        oracle(
            &t.env,
            &[
                scaled(
                    reflector_feed(&factor, &eth, TWAP3, TWAP3_BUDGET),
                    usdc_key.clone(),
                    1,
                    MAX_REASONABLE_PRICE_WAD,
                ),
                PriceSource::Feed(reflector_feed(&partner, &eth, TWAP3, TWAP3_BUDGET)),
            ],
            SLOW_BUDGET,
            500,
            IndependencePolicy::RequireDisjoint,
            DEFAULT_MIN_SANITY_PRICE_WAD,
            DEFAULT_MAX_SANITY_PRICE_WAD,
        ),
    );
    redstone_at(&t, &adapter, "USDC", usd(1), at(-18_000));
    for (label, keys) in [
        (
            "[USDC, ETH]",
            vec![&t.env, usdc_key.clone(), eth_key.clone()],
        ),
        (
            "[ETH, USDC]",
            vec![&t.env, eth_key.clone(), usdc_key.clone()],
        ),
    ] {
        let prices = flat(t.price_agg_client().try_prices(&keys))
            .unwrap_or_else(|e| panic!("{label}: both keys price: {e:?}"));
        let feed = prices.get(eth_key.clone()).expect("ETH priced");
        assert_eq!(feed.price_wad, usd(2_000), "{label}");
        assert_eq!(
            feed.timestamp,
            at(-18_000),
            "{label}: blended stamp keeps the lag"
        );
        let status = t
            .price_agg_client()
            .quotes(&keys)
            .get(eth_key.clone())
            .expect("ETH quoted");
        assert!(status.valid, "{label}: {status:?}");
    }
    let spread = flat(t.price_agg_client().try_price_spread(&eth_key));
    assert!(spread.is_ok(), "price_spread: {spread:?}");
}

// ---------------------------------------------------------------------------
// oracle-9: CR-11 transitive market budget two quotes down.
// ---------------------------------------------------------------------------

#[test]
fn oracle_9_a_market_budget_two_quotes_down_is_held_to_the_bound() {
    let t = fresh_fixture();
    let usdc = t.resolve_asset("USDC");
    let eth = t.resolve_asset("ETH");
    let eth_key = PriceKey::Token(eth.clone());
    let k1 = PriceKey::Ref(Symbol::new(&t.env, "SKQ1"));
    let k2 = PriceKey::Ref(Symbol::new(&t.env, "SKQ2"));
    let adapter = register_redstone_adapter(&t, &[("F1", WAD), ("F0", usd(2_000))]);
    let partner = new_reflector(&t, None);
    twap_at(&t, &partner, &eth, usd(2_000), at(0));
    twap_at(&t, &t.mock_reflector, &usdc, usd(1), at(0));
    let (min_wad, max_wad) = tight_single_source_band(usd(1));
    let k2_oracle = |budget: u64| {
        let mut o = oracle(
            &t.env,
            &[PriceSource::Feed(reflector_feed(
                &t.mock_reflector,
                &usdc,
                TWAP3,
                budget,
            ))],
            budget,
            500,
            IndependencePolicy::RequireDisjoint,
            min_wad,
            max_wad,
        );
        o.asset_decimals = 0;
        o
    };
    let mut k1_oracle = oracle(
        &t.env,
        &[scaled(
            redstone_feed(&t.env, &adapter, "F1", FeedNature::Fundamental, SLOW_BUDGET),
            k2.clone(),
            1,
            MAX_REASONABLE_PRICE_WAD,
        )],
        SLOW_BUDGET,
        500,
        IndependencePolicy::RequireDisjoint,
        min_wad,
        max_wad,
    );
    k1_oracle.asset_decimals = 0;
    let eth_oracle = oracle(
        &t.env,
        &[
            scaled(
                redstone_feed(&t.env, &adapter, "F0", FeedNature::Fundamental, SLOW_BUDGET),
                k1.clone(),
                1,
                MAX_REASONABLE_PRICE_WAD,
            ),
            PriceSource::Feed(reflector_feed(&partner, &eth, TWAP3, TWAP3_BUDGET)),
        ],
        SLOW_BUDGET,
        500,
        IndependencePolicy::RequireDisjoint,
        DEFAULT_MIN_SANITY_PRICE_WAD,
        DEFAULT_MAX_SANITY_PRICE_WAD,
    );

    configure(&t, &k2, k2_oracle(7_200));
    configure(&t, &k1, k1_oracle);
    assert_contract_error(
        try_configure(&t, &eth_key, eth_oracle.clone()),
        INVALID_STALENESS_CONFIG,
    );
    configure(&t, &k2, k2_oracle(MAX_LEG_AGE_SPREAD_SECONDS));
    configure(&t, &eth_key, eth_oracle);
    assert_eq!(
        assert_fresh(&t, &eth_key, "two quotes down at the bound").price_wad,
        usd(2_000)
    );
    assert_contract_error(
        try_configure(&t, &k2, k2_oracle(7_200)),
        INVALID_STALENESS_CONFIG,
    );
}

// ---------------------------------------------------------------------------
// oracle-11: fifth-pass TWAP quantum, feeds below 8 decimals.
// ---------------------------------------------------------------------------

#[test]
fn oracle_11_feeds_below_eight_decimals_are_refused_at_admission() {
    let t = fresh_fixture();
    let eth = t.resolve_asset("ETH");
    let eth_key = PriceKey::Token(eth.clone());
    let reflector = new_reflector(&t, None);
    twap_at(&t, &reflector, &eth, usd(2_000), at(0));
    let (min_wad, max_wad) = tight_single_source_band(usd(2_000));
    let with_decimals = |decimals: u32| {
        MockReflectorClient::new(&t.env, &reflector).set_decimals(&decimals);
        let mut feed = reflector_feed(&reflector, &eth, TWAP3, TWAP3_BUDGET);
        feed.decimals = decimals;
        oracle(
            &t.env,
            &[PriceSource::Feed(feed)],
            TWAP3_BUDGET,
            500,
            IndependencePolicy::RequireDisjoint,
            min_wad,
            max_wad,
        )
    };
    let seven = try_configure(&t, &eth_key, with_decimals(7));
    std::println!("oracle-11: 7-decimal feed = {seven:?}");
    assert_contract_error(seven, INVALID_ORACLE_DECIMALS);
    configure(&t, &eth_key, with_decimals(8));
}

// ---------------------------------------------------------------------------
// oracle-12: mainnet oracle shapes and the operational limits.
// ---------------------------------------------------------------------------

struct Mainnet {
    t: LendingTest,
    btc: PriceKey,
    solv: PriceKey,
    xsolv: PriceKey,
    ustry: PriceKey,
    cetes: PriceKey,
    a: Address,
    r_stellar: Address,
    ustry_asset: Address,
}

const USTRY_PRICE: i128 = 107 * WAD / 100;
const CETES_PRICE: i128 = 669 * WAD / 10_000;

fn ustry_oracle(m: &Mainnet, band_bps: u32) -> AssetOracle {
    let env = &m.t.env;
    let usdc_key = PriceKey::Token(m.t.resolve_asset("USDC"));
    let (min_band, max_band) = if band_bps == 800 {
        (9_844 * WAD / 10_000, 11_556 * WAD / 10_000)
    } else {
        let half = USTRY_PRICE * i128::from(band_bps) / 10_000;
        (USTRY_PRICE - half, USTRY_PRICE + half)
    };
    oracle(
        env,
        &[
            scaled(
                reflector_feed(&m.r_stellar, &m.ustry_asset, TWAP3, 7_200),
                usdc_key,
                9_844 * WAD / 10_000,
                11_556 * WAD / 10_000,
            ),
            PriceSource::Feed(redstone_feed(
                env,
                &m.a,
                "USTRY",
                FeedNature::Fundamental,
                57_600,
            )),
        ],
        57_600,
        500,
        IndependencePolicy::AllowShared(vec![env, m.a.clone()]),
        min_band,
        max_band,
    )
}

fn solv_oracle(m: &Mainnet, max_factor_wad: i128) -> AssetOracle {
    let env = &m.t.env;
    oracle(
        env,
        &[
            scaled(
                redstone_feed(
                    env,
                    &m.a,
                    "SolvBTC_FUNDAMENTAL",
                    FeedNature::Fundamental,
                    93_600,
                ),
                m.btc.clone(),
                WAD,
                max_factor_wad,
            ),
            PriceSource::Feed(redstone_feed(
                env,
                &m.a,
                "SolvBTC_FUNDAMENTAL/USD",
                FeedNature::Fundamental,
                57_600,
            )),
        ],
        93_600,
        500,
        IndependencePolicy::AllowShared(vec![env, m.a.clone()]),
        usd(20_000),
        usd(300_000),
    )
}

/// Builds and admits the mainnet shapes (configs/mainnet/markets.json) with
/// mocks: RedStone adapter A, a USD-base Reflector for BTC and USDC, and a
/// USDC-base Reflector for the USTRY and CETES factors.
fn mainnet() -> Mainnet {
    let t = fresh_fixture();
    let env = &t.env;
    let usdc = t.resolve_asset("USDC");
    let usdc_key = PriceKey::Token(usdc.clone());
    let btc_asset = t.resolve_asset("WBTC");
    let a = register_redstone_adapter(
        &t,
        &[
            ("BTC", usd(BTC)),
            ("SolvBTC_FUNDAMENTAL", WAD),
            ("SolvBTC_FUNDAMENTAL/USD", usd(BTC)),
            ("USTRY", USTRY_PRICE),
            ("CETES", CETES_PRICE),
            ("USDC", usd(1)),
        ],
    );
    let r_usd = new_reflector(&t, None);
    twap_at(&t, &r_usd, &btc_asset, usd(BTC), at(0));
    twap_at(&t, &r_usd, &usdc, usd(1), at(0));
    let r_stellar = new_reflector(&t, Some(&usdc));
    let ustry_asset = Address::generate(env);
    let cetes_asset = Address::generate(env);
    twap_at(&t, &r_stellar, &ustry_asset, USTRY_PRICE, at(0));
    twap_at(&t, &r_stellar, &cetes_asset, CETES_PRICE, at(0));

    configure(
        &t,
        &usdc_key,
        oracle(
            env,
            &[
                PriceSource::Feed(reflector_feed(&r_usd, &usdc, TWAP3, 3_600)),
                PriceSource::Feed(redstone_feed(
                    env,
                    &a,
                    "USDC",
                    FeedNature::Fundamental,
                    57_600,
                )),
            ],
            57_600,
            500,
            IndependencePolicy::RequireDisjoint,
            95 * WAD / 100,
            105 * WAD / 100,
        ),
    );
    let btc = PriceKey::Ref(Symbol::new(env, "BTC"));
    configure(
        &t,
        &btc,
        oracle(
            env,
            &[
                PriceSource::Feed(reflector_feed(&r_usd, &btc_asset, TWAP3, 3_600)),
                PriceSource::Feed(redstone_feed(
                    env,
                    &a,
                    "BTC",
                    FeedNature::Fundamental,
                    46_800,
                )),
            ],
            57_600,
            1_000,
            IndependencePolicy::RequireDisjoint,
            usd(20_000),
            usd(300_000),
        ),
    );
    let m = Mainnet {
        btc,
        solv: PriceKey::Ref(Symbol::new(env, "SOLVBTC")),
        xsolv: PriceKey::Ref(Symbol::new(env, "XSOLVBTC")),
        ustry: PriceKey::Ref(Symbol::new(env, "USTRY")),
        cetes: PriceKey::Ref(Symbol::new(env, "CETES")),
        a: a.clone(),
        r_stellar: r_stellar.clone(),
        ustry_asset,
        t,
    };
    configure(&m.t, &m.solv, solv_oracle(&m, 105 * WAD / 100));
    configure(&m.t, &m.xsolv, solv_oracle(&m, 105 * WAD / 100));
    configure(&m.t, &m.ustry, ustry_oracle(&m, 800));
    let cetes = oracle(
        &m.t.env,
        &[
            scaled(
                reflector_feed(&r_stellar, &cetes_asset, TWAP3, 7_200),
                usdc_key,
                61_548 * WAD / 1_000_000,
                72_252 * WAD / 1_000_000,
            ),
            PriceSource::Feed(redstone_feed(
                &m.t.env,
                &a,
                "CETES",
                FeedNature::Fundamental,
                57_600,
            )),
        ],
        57_600,
        500,
        IndependencePolicy::AllowShared(vec![&m.t.env, a.clone()]),
        61_548 * WAD / 1_000_000,
        72_252 * WAD / 1_000_000,
    );
    configure(&m.t, &m.cetes, cetes);
    m
}

/// SAFE: every mainnet shape is admitted and prices.
#[test]
fn oracle_12_mainnet_shapes_are_admitted_and_price() {
    let m = mainnet();
    let usdc_key = PriceKey::Token(m.t.resolve_asset("USDC"));
    for (label, key, expected) in [
        ("USDC", &usdc_key, usd(1)),
        ("BTC", &m.btc, usd(BTC)),
        ("SolvBTC", &m.solv, usd(BTC)),
        ("xSolvBTC", &m.xsolv, usd(BTC)),
        ("USTRY", &m.ustry, USTRY_PRICE),
        ("CETES", &m.cetes, CETES_PRICE),
    ] {
        let feed = assert_fresh(&m.t, key, label);
        std::println!("oracle-12 {label}: {}", feed.price_wad);
        assert_eq!(feed.price_wad, expected, "{label}");
    }
}

/// The operational limits on the mainnet shapes: (i) a 1_200 bps USTRY band is
/// refused #226 (the shared adapter reaches the USTRY pair only through the
/// USDC quote, so the pair keeps the single-source cap); (ii) BTC tolerance
/// edits up to 1_083 bps are admitted and 1_084 bps is refused #226 through
/// the SolvBTC pairs' revalidation (1.05 x 1.1085 x 1.0501 > 11/9); (iii) a
/// SolvBTC factor ceiling of 1.06 is refused #226.
///
/// (ii) at 1_084 and (iii) flipped with the oracle fix (CR-7 quote reach): PR
/// #196 admitted both because it did not count the adapter's reach through
/// the BTC quote.
#[test]
fn oracle_12_operational_limits_on_the_mainnet_shapes() {
    let m = mainnet();
    let wide_ustry = try_configure(&m.t, &m.ustry, ustry_oracle(&m, 1_200));
    let edit_1082 = try_edit_tolerance(&m.t, &m.btc, 1_082);
    let edit_1083 = try_edit_tolerance(&m.t, &m.btc, 1_083);
    let edit_1084 = try_edit_tolerance(&m.t, &m.btc, 1_084);
    let solv_106 = try_configure(&m.t, &m.solv, solv_oracle(&m, 106 * WAD / 100));
    std::println!(
        "oracle-12 (i) USTRY 1_200 bps band = {wide_ustry:?}; (ii) BTC edit 1_082 = \
         {edit_1082:?}, 1_083 = {edit_1083:?}, 1_084 = {edit_1084:?}; (iii) SolvBTC ceiling \
         1.06 = {solv_106:?}"
    );
    assert_contract_error(wide_ustry, BAND_TOO_WIDE);
    assert!(edit_1082.is_ok() && edit_1083.is_ok());
    assert_contract_error(edit_1084, BAND_TOO_WIDE);
    assert_contract_error(solv_106, BAND_TOO_WIDE);
    assert_eq!(assert_fresh(&m.t, &m.solv, "SolvBTC").price_wad, usd(BTC));
    assert_eq!(assert_fresh(&m.t, &m.xsolv, "xSolvBTC").price_wad, usd(BTC));
}
