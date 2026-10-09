//! Second opinion on Specula oracle CR-10 and CR-11: TWAP admission in
//! `reflector::attest` bounds `(records - 1) * resolution` against the leg
//! budget, while `read_twap` stamps the leg with its oldest sample and accepts
//! `records + 1` samples (what the upstream Reflector `prices()` returns for a
//! complete history: it walks back until a record at or before
//! `last - records * resolution` is included). No admission rule relates that
//! window to `MAX_LEG_AGE_SPREAD_SECONDS` on a Market+Market dual.
//!
//! Every scenario goes through production entry points: governance
//! `ConfigureAssetOracle` for admission (the immediate path skips only the
//! delay, not validation or attestation), `prices`/`quotes` for reads, and
//! the controller's `supply`/`borrow` for the lending effect.
//!
//! The harness mock returns exactly `records` samples with the newest at the
//! stamped time. The upstream shape (`records + 1` samples, oldest one round
//! further back) is emulated by stamping the newest harness sample one round
//! before the Reflector's last round; the aggregator sees the same oldest
//! timestamp either way.

use common::errors::OracleError;
use common::oracle::observation::{MAX_LEG_AGE_SPREAD_SECONDS, MAX_TWAP_RECORDS};
use common::types::{PriceFeedRaw, PriceStatus};
use controller::types::{
    AssetOracle, FeedSource, IndependencePolicy, OracleAssetRef, OracleReadMode, PriceKey,
    PriceSource, ProviderRef, ReflectorFeedRef,
};
use governance::op::{AdminOperation, ConfigureAssetOracleArgs};
use soroban_sdk::testutils::Ledger as _;
use soroban_sdk::{vec, Address, Env, Vec};
use test_harness::errors;
use test_harness::mock_reflector::{MockReflector, MockReflectorClient};
use test_harness::{
    assert_contract_error, tight_single_source_band, tolerance_band, usd, LendingTest, ALICE, BOB,
    DEFAULT_MAX_SANITY_PRICE_WAD, DEFAULT_MIN_SANITY_PRICE_WAD,
};

/// Ledger time the scenarios start from; far enough from zero to backdate a
/// TWAP window.
const T0: u64 = 100_000;
/// Mock Reflector default resolution, the mainnet Reflector round length.
const RESOLUTION: u64 = 300;
const REFLECTOR_DECIMALS: u32 = 14;
const TEN_ETH: f64 = 10.0;
/// 0.01 WBTC at the fixture's 60_000 USD: 600 USD of debt against 20_000 USD
/// of ETH, inside any LTV the presets use.
const SMALL_WBTC_DEBT: f64 = 0.01;
/// 600 USD of USDC debt, the same size on the fixture's 1 USD price.
const SMALL_USDC_DEBT: f64 = 600.0;
const INVALID_ORACLE_RESOLUTION: u32 = OracleError::InvalidOracleResolution as u32;

/// Ledger timestamp `offset` seconds away from `T0`.
fn at(offset: i64) -> u64 {
    (T0 as i64 + offset) as u64
}

/// Moves the ledger clock without touching the sequence, so temporary mock
/// entries keep their TTL.
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

/// Governance immediate configuration as a `Result`.
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

/// Governance immediate configuration; panics if the aggregator refuses it.
fn configure(t: &LendingTest, key: &PriceKey, oracle: AssetOracle) {
    try_configure(t, key, oracle).expect("admission accepts the configuration");
}

/// `quotes([key])`: never panics on an unusable price.
fn quote(t: &LendingTest, key: &PriceKey) -> PriceStatus {
    let keys = vec![&t.env, key.clone()];
    t.price_agg_client()
        .quotes(&keys)
        .get(key.clone())
        .expect("key quoted")
}

/// `prices([key])` as a `Result`: the aggregator's own contract error on failure.
fn try_price(t: &LendingTest, key: &PriceKey) -> Result<PriceFeedRaw, soroban_sdk::Error> {
    let keys = vec![&t.env, key.clone()];
    flat(t.price_agg_client().try_prices(&keys)).map(|map| map.get(key.clone()).expect("priced"))
}

fn reflector_feed(
    contract: &Address,
    asset: &Address,
    read_mode: OracleReadMode,
    max_stale_seconds: u64,
) -> PriceSource {
    PriceSource::Feed(FeedSource {
        provider: ProviderRef::Reflector(ReflectorFeedRef {
            contract: contract.clone(),
            asset: OracleAssetRef::Stellar(asset.clone()),
            read_mode,
        }),
        decimals: REFLECTOR_DECIMALS,
        max_stale_seconds,
    })
}

fn oracle(
    env: &Env,
    items: &[PriceSource],
    max_price_stale_seconds: u64,
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
        tolerance: tolerance_band(env, 500),
        independence: IndependencePolicy::RequireDisjoint,
        min_sanity_price_wad,
        max_sanity_price_wad,
    }
}

/// A fresh mock Reflector quoting in USD.
fn new_reflector(t: &LendingTest) -> Address {
    t.env.register(MockReflector, ())
}

/// Stamps the newest TWAP sample; the mock spaces the rest one resolution
/// apart behind it.
fn twap_at(t: &LendingTest, reflector: &Address, asset: &Address, price_wad: i128, ts: u64) {
    MockReflectorClient::new(&t.env, reflector).set_twap_price_at(asset, &price_wad, &ts);
}

fn spot_at(t: &LendingTest, reflector: &Address, asset: &Address, price_wad: i128, ts: u64) {
    MockReflectorClient::new(&t.env, reflector).set_price_at(asset, &price_wad, &ts);
}

/// USDC, ETH and WBTC with fresh mock prices at `T0`. ETH is reconfigured by
/// each scenario; WBTC is the debt leg because its price is untouched.
fn fresh_fixture() -> LendingTest {
    let t = LendingTest::new().three_asset_usdc_eth_wbtc().build();
    set_time(&t, T0);
    t.refresh_oracle_prices();
    t
}

/// ETH on a sole `Twap(3)` leg over `reflector` with `budget` as both the leg
/// and the asset staleness budget.
fn twap3_single(t: &LendingTest, reflector: &Address, budget: u64) -> AssetOracle {
    let eth = t.resolve_asset("ETH");
    let (min_wad, max_wad) = tight_single_source_band(usd(2_000));
    oracle(
        &t.env,
        &[reflector_feed(
            reflector,
            &eth,
            OracleReadMode::Twap(3),
            budget,
        )],
        budget,
        min_wad,
        max_wad,
    )
}

fn assert_stale(t: &LendingTest, key: &PriceKey, label: &str) {
    let status = quote(t, key);
    assert!(status.stale && !status.valid, "{label}: {status:?}");
    assert_eq!(status.error_code, Some(errors::PRICE_FEED_STALE), "{label}");
    assert_contract_error(try_price(t, key), errors::PRICE_FEED_STALE);
}

fn assert_fresh(t: &LendingTest, key: &PriceKey, label: &str) -> PriceFeedRaw {
    let status = quote(t, key);
    assert!(status.valid && !status.stale, "{label}: {status:?}");
    try_price(t, key).unwrap_or_else(|err| panic!("{label}: strict read refused: {err:?}"))
}

// ---------------------------------------------------------------------------
// CR-10: admission bounds (records - 1) * resolution; the read is dated
// records * resolution (upstream shape) or more behind the ledger.
// ---------------------------------------------------------------------------

/// `attest` admits a `Twap(3)` leg whose 600 s window equals its 600 s budget
/// and refuses 599 s with #222: the rule is exactly `(records - 1) * resolution`.
#[test]
fn cr10_attest_admits_a_window_equal_to_the_budget_and_refuses_one_second_less() {
    let t = fresh_fixture();
    let eth_key = PriceKey::Token(t.resolve_asset("ETH"));
    let reflector = new_reflector(&t);
    let window = (3 - 1) * RESOLUTION;

    assert_contract_error(
        try_configure(&t, &eth_key, twap3_single(&t, &reflector, window - 1)),
        INVALID_ORACLE_RESOLUTION,
    );
    configure(&t, &eth_key, twap3_single(&t, &reflector, window));
}

/// The admitted boundary configuration is usable only at the instant of a
/// Reflector round under the harness's optimistic `records`-sample shape, and
/// at no instant under the upstream `records + 1` shape. The controller's
/// borrow path fails with #206 in both cases.
///
/// The debt leg is USDC: the fixture seeds pool cash without supply shares,
/// so a market that already carries debt trips the pool utilization gate
/// (#127) before the controller prices anything once revenue accrues.
#[test]
fn cr10_a_boundary_window_is_stale_on_every_read_under_the_upstream_shape() {
    let mut t = fresh_fixture();
    let eth = t.resolve_asset("ETH");
    let eth_key = PriceKey::Token(eth.clone());
    let reflector = new_reflector(&t);
    let window = (3 - 1) * RESOLUTION;
    twap_at(&t, &reflector, &eth, usd(2_000), at(0));
    configure(&t, &eth_key, twap3_single(&t, &reflector, window));

    // Harness shape at the round instant: oldest sample exactly at the budget.
    let feed = assert_fresh(&t, &eth_key, "round instant, harness shape");
    assert_eq!(
        feed.timestamp,
        at(-(window as i64)),
        "dated to the oldest sample"
    );
    t.supply(ALICE, "ETH", TEN_ETH);
    t.supply(BOB, "ETH", TEN_ETH);
    t.borrow(ALICE, "WBTC", SMALL_WBTC_DEBT);

    // One second later, no new round: already stale. The other markets are
    // re-stamped on the harness Reflector, which ETH no longer reads.
    set_time(&t, at(1));
    t.refresh_oracle_prices();
    assert_stale(&t, &eth_key, "one second after the round, harness shape");
    assert_contract_error(
        t.try_borrow(BOB, "USDC", SMALL_USDC_DEBT),
        errors::PRICE_FEED_STALE,
    );

    // Upstream shape at the next round instant: `prices(3)` returns the
    // current period plus the window, so the oldest sample sits
    // 3 * 300 = 900 s behind the round, beyond the 600 s budget even with
    // zero ledger lag.
    let next_round = at(RESOLUTION as i64);
    twap_at(&t, &reflector, &eth, usd(2_000), next_round - RESOLUTION);
    set_time(&t, next_round);
    t.refresh_oracle_prices();
    assert_stale(&t, &eth_key, "round instant, upstream shape");
    assert_contract_error(
        t.try_borrow(BOB, "USDC", SMALL_USDC_DEBT),
        errors::PRICE_FEED_STALE,
    );
}

/// Control for the fix: a budget of `(records + 2) * resolution` keeps the
/// upstream shape fresh through the whole liveness window Reflector itself
/// serves (`ledger - last < 2 * resolution`).
#[test]
fn cr10_control_a_records_plus_two_budget_covers_the_upstream_shape() {
    let t = fresh_fixture();
    let eth = t.resolve_asset("ETH");
    let eth_key = PriceKey::Token(eth.clone());
    let reflector = new_reflector(&t);
    let budget = (3 + 2) * RESOLUTION;
    // Upstream shape for a round at T0: oldest sample at T0 - 900.
    twap_at(&t, &reflector, &eth, usd(2_000), at(-(RESOLUTION as i64)));
    configure(&t, &eth_key, twap3_single(&t, &reflector, budget));

    assert_fresh(&t, &eth_key, "round instant");
    set_time(&t, at(2 * RESOLUTION as i64 - 1));
    let feed = assert_fresh(&t, &eth_key, "last ledger Reflector still serves");
    assert_eq!(feed.timestamp, at(-(3 * RESOLUTION as i64)));
    set_time(&t, at(2 * RESOLUTION as i64 + 1));
    assert_stale(&t, &eth_key, "past the budget");
}

// ---------------------------------------------------------------------------
// CR-11: no admission rule ties a TWAP window to MAX_LEG_AGE_SPREAD_SECONDS
// on a Market+Market dual.
// ---------------------------------------------------------------------------

/// ETH on `Twap(records)` over `twap` (resolution `resolution`) blended with
/// a spot read on `spot`; both legs are Market by nature. `twap_budget` is
/// the TWAP leg and asset budget, the spot leg keeps the spread bound.
fn market_market_dual(
    t: &LendingTest,
    twap: &Address,
    spot: &Address,
    records: u32,
    twap_budget: u64,
) -> AssetOracle {
    let eth = t.resolve_asset("ETH");
    oracle(
        &t.env,
        &[
            reflector_feed(twap, &eth, OracleReadMode::Twap(records), twap_budget),
            reflector_feed(spot, &eth, OracleReadMode::Spot, MAX_LEG_AGE_SPREAD_SECONDS),
        ],
        twap_budget.max(MAX_LEG_AGE_SPREAD_SECONDS),
        DEFAULT_MIN_SANITY_PRICE_WAD,
        DEFAULT_MAX_SANITY_PRICE_WAD,
    )
}

/// A `Twap(12)` leg on a 600 s feed spans 6_600 s. On-chain admission accepts
/// it (`configs/script.sh` would refuse the 6_600 s budget off-chain), the
/// blend is stale whenever the spot partner is within 3_000 s of the TWAP's
/// newest sample, and the controller never lends on it. Only a spot partner
/// lagging by 3_000 s or more (inside its own 3_600 s budget) yields a price.
#[test]
fn cr11_a_long_twap_window_on_a_market_market_dual_is_admitted_but_unpriceable() {
    let mut t = fresh_fixture();
    let eth = t.resolve_asset("ETH");
    let eth_key = PriceKey::Token(eth.clone());
    let twap = new_reflector(&t);
    let spot = new_reflector(&t);
    let resolution = 2 * RESOLUTION;
    MockReflectorClient::new(&t.env, &twap).set_resolution(&(resolution as u32));
    let span = u64::from(MAX_TWAP_RECORDS - 1) * resolution;
    assert!(
        span > MAX_LEG_AGE_SPREAD_SECONDS,
        "window {span} exceeds the bound"
    );

    twap_at(&t, &twap, &eth, usd(2_000), at(0));
    spot_at(&t, &spot, &eth, usd(2_000), at(0));
    configure(
        &t,
        &eth_key,
        market_market_dual(&t, &twap, &spot, MAX_TWAP_RECORDS, span),
    );

    // Synchronous legs, both inside their budgets: the spread alone trips.
    assert_stale(&t, &eth_key, "synchronous legs");
    t.supply(ALICE, "ETH", TEN_ETH);
    assert_contract_error(
        t.try_borrow(ALICE, "WBTC", SMALL_WBTC_DEBT),
        errors::PRICE_FEED_STALE,
    );

    // The partner must lag by span - bound = 3_000 s before the dual prices.
    let needed_lag = (span - MAX_LEG_AGE_SPREAD_SECONDS) as i64;
    spot_at(&t, &spot, &eth, usd(2_000), at(-needed_lag + 1));
    assert_stale(&t, &eth_key, "partner 2_999 s behind");
    spot_at(&t, &spot, &eth, usd(2_000), at(-needed_lag));
    let feed = assert_fresh(&t, &eth_key, "partner 3_000 s behind");
    assert_eq!(
        feed.timestamp,
        at(-(span as i64)),
        "dated to the TWAP's oldest sample"
    );
}

/// `Twap(12)` on the mainnet 300 s resolution with a 3_600 s budget passes
/// both the on-chain rules and the `configs/script.sh` leg-spread rule. Under
/// the upstream shape its oldest sample is 3_600 s behind the round, so the
/// blend is stale as soon as the Market partner is one second fresher.
/// `Twap(10)` keeps 600 s of slack, the Reflector liveness window.
#[test]
fn cr11_twap12_at_300s_passes_every_admission_rule_and_goes_stale_when_the_partner_is_fresher() {
    let t = fresh_fixture();
    let eth = t.resolve_asset("ETH");
    let eth_key = PriceKey::Token(eth.clone());
    let twap = new_reflector(&t);
    let spot = new_reflector(&t);

    // Upstream shape for a round at T0: 13 samples, oldest at T0 - 3_600.
    twap_at(&t, &twap, &eth, usd(2_000), at(-(RESOLUTION as i64)));
    spot_at(&t, &spot, &eth, usd(2_000), at(0));
    configure(
        &t,
        &eth_key,
        market_market_dual(
            &t,
            &twap,
            &spot,
            MAX_TWAP_RECORDS,
            MAX_LEG_AGE_SPREAD_SECONDS,
        ),
    );
    assert_fresh(&t, &eth_key, "partner on the same round");

    // The partner's round lands one second later (inside the future skew
    // allowance, ledger unchanged): the TWAP leg is still inside its own
    // budget, only the spread trips.
    spot_at(&t, &spot, &eth, usd(2_000), at(1));
    assert_stale(&t, &eth_key, "partner one second fresher");

    // Control: (records + 2) * resolution <= bound leaves the full Reflector
    // liveness window for the partner to run ahead.
    let records = (MAX_LEG_AGE_SPREAD_SECONDS / RESOLUTION - 2) as u32;
    assert_eq!(records, 10);
    configure(
        &t,
        &eth_key,
        market_market_dual(&t, &twap, &spot, records, MAX_LEG_AGE_SPREAD_SECONDS),
    );
    let ahead = (2 * RESOLUTION - 1) as i64;
    spot_at(&t, &spot, &eth, usd(2_000), at(ahead));
    set_time(&t, at(ahead));
    assert_fresh(&t, &eth_key, "Twap(10) with the partner 599 s ahead");
}
