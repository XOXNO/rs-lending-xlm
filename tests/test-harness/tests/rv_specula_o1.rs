//! Second opinion on Specula oracle CR-1 and CR-2: the Market+Market leg-age
//! bound in `engine::blend` keys off `engine::source_nature`, which labels a
//! `Scaled` leg by its factor feed alone, while `read_scaled` stamps the leg
//! with `min(factor, quote)`. Nature and age therefore come from different
//! parts of the composition.
//!
//! Every scenario goes through production entry points: governance
//! `ConfigureAssetOracle` for admission (the immediate path skips only the
//! delay, not validation), `prices`/`quotes` for reads, and the controller's
//! `supply`/`borrow` for the lending effect. The quote legs use the harness
//! mock Reflector; the factor and anchor legs use fresh mocks so the
//! independence policy is `RequireDisjoint` unless the scenario says otherwise.

use common::oracle::observation::MAX_LEG_AGE_SPREAD_SECONDS;
use common::types::{PriceFeedRaw, PriceStatus};
use controller::constants::MAX_REASONABLE_PRICE_WAD;
use controller::types::{
    AssetOracle, FeedNature, FeedSource, IndependencePolicy, MultiFeedRef, OracleAssetRef,
    OracleReadMode, PriceKey, PriceSource, ProviderRef, ReflectorFeedRef, ScaledSource,
};
use governance::op::{AdminOperation, ConfigureAssetOracleArgs};
use soroban_sdk::testutils::Ledger as _;
use soroban_sdk::{vec, Address, Env, String, Vec};
use test_harness::errors;
use test_harness::mock_redstone::MockRedStonePriceFeedClient;
use test_harness::mock_reflector::{MockReflector, MockReflectorClient};
use test_harness::oracle::redstone::register_redstone_adapter;
use test_harness::{
    assert_contract_error, tolerance_band, usd, usd_frac, LendingTest, ALICE, BOB,
    DEFAULT_MAX_SANITY_PRICE_WAD, DEFAULT_MIN_SANITY_PRICE_WAD,
};

/// Ledger time the scenarios start from; far enough from zero to backdate a
/// feed by the 16 h budget below.
const T0: u64 = 100_000;
/// Harness budget for a market leg (mainnet uses 3_600 on its Reflector legs).
const MARKET_BUDGET: u64 = 900;
/// Mainnet's RedStone fundamental budget: 16 h.
const SLOW_BUDGET: u64 = 57_600;
/// Mock Reflector resolution; a TWAP(3) read is stamped two rounds before its
/// newest sample.
const RESOLUTION: i64 = 300;
const TWAP_SPAN: i64 = 2 * RESOLUTION;
const REFLECTOR_DECIMALS: u32 = 14;
const REDSTONE_DECIMALS: u32 = 8;
const TWAP: OracleReadMode = OracleReadMode::Twap(3);
const TEN_ETH: f64 = 10.0;
/// 0.23 WBTC at the fixture's 60_000 USD: 13_800 USD of debt.
const WBTC_DEBT: f64 = 0.23;

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

fn reflector_feed(contract: &Address, asset: &Address, max_stale_seconds: u64) -> FeedSource {
    FeedSource {
        provider: ProviderRef::Reflector(ReflectorFeedRef {
            contract: contract.clone(),
            asset: OracleAssetRef::Stellar(asset.clone()),
            read_mode: TWAP,
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

/// `factor x quote` with the widest factor band admission allows.
fn scaled(factor: FeedSource, quote: PriceKey) -> PriceSource {
    PriceSource::Scaled(ScaledSource {
        factor,
        quote,
        min_factor_wad: 1,
        max_factor_wad: MAX_REASONABLE_PRICE_WAD,
    })
}

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

/// A fresh mock Reflector. Quoting in USD (the default) it may serve a bare
/// feed; quoting in `base` it may only be a `Scaled` factor over `Token(base)`.
fn new_reflector(t: &LendingTest, base: Option<&Address>) -> Address {
    let addr = t.env.register(MockReflector, ());
    if let Some(base) = base {
        MockReflectorClient::new(&t.env, &addr).set_base_stellar(base);
    }
    addr
}

fn twap_at(t: &LendingTest, reflector: &Address, asset: &Address, price_wad: i128, ts: u64) {
    MockReflectorClient::new(&t.env, reflector).set_twap_price_at(asset, &price_wad, &ts);
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

/// USDC, ETH and WBTC with fresh mock prices at `T0`. WBTC is the debt leg of
/// the lending checks because its price does not depend on the USDC quote.
fn fresh_fixture() -> LendingTest {
    let t = LendingTest::new().three_asset_usdc_eth_wbtc().build();
    set_time(&t, T0);
    t.refresh_oracle_prices();
    t
}

// ---------------------------------------------------------------------------
// CR-1: a Scaled leg labelled by a Fundamental factor hides its market quote
// from the leg-age bound.
// ---------------------------------------------------------------------------

/// USDC: one smoothed Reflector read with a 16 h budget. Admission caps a
/// market leg at the spread bound only when its partner is market-nature too
/// (`configs/script.sh`); a sole source is uncapped on-chain and off-chain.
/// The band admits 0.85..1.03 so the counterfactual quote can follow the
/// market. ETH: `rate (RedStone, `nature`) x USDC` blended with a fresh market
/// feed on its own Reflector. Returns the fixture, the ETH key and the
/// partner Reflector.
fn cr1_fixture(nature: FeedNature, factor_budget: u64) -> (LendingTest, PriceKey, Address) {
    let t = fresh_fixture();
    let usdc = t.resolve_asset("USDC");
    let eth = t.resolve_asset("ETH");
    let usdc_key = PriceKey::Token(usdc.clone());
    let eth_key = PriceKey::Token(eth.clone());

    configure(
        &t,
        &usdc_key,
        oracle(
            &t.env,
            &[PriceSource::Feed(reflector_feed(
                &t.mock_reflector,
                &usdc,
                SLOW_BUDGET,
            ))],
            SLOW_BUDGET,
            500,
            IndependencePolicy::RequireDisjoint,
            usd_frac(85, 100),
            usd_frac(103, 100),
        ),
    );

    let adapter = register_redstone_adapter(&t, &[("RATE", usd(2_000))]);
    let partner = new_reflector(&t, None);
    twap_at(&t, &partner, &eth, usd(2_000), at(0));
    configure(
        &t,
        &eth_key,
        oracle(
            &t.env,
            &[
                scaled(
                    redstone_feed(&t.env, &adapter, "RATE", nature, factor_budget),
                    usdc_key.clone(),
                ),
                PriceSource::Feed(reflector_feed(&partner, &eth, MARKET_BUDGET)),
            ],
            SLOW_BUDGET,
            1_500,
            IndependencePolicy::RequireDisjoint,
            DEFAULT_MIN_SANITY_PRICE_WAD,
            DEFAULT_MAX_SANITY_PRICE_WAD,
        ),
    );

    let fresh = quote(&t, &eth_key);
    assert!(fresh.valid && !fresh.stale, "fresh blend: {fresh:?}");
    assert_eq!(fresh.final_wad, usd(2_000));
    (t, eth_key, partner)
}

/// Freezes the USDC quote with its oldest TWAP sample exactly at the 16 h
/// budget (still inside it) and moves the ETH market feed down 10%.
fn freeze_quote_and_drop_market(t: &LendingTest, partner: &Address) {
    let usdc = t.resolve_asset("USDC");
    let eth = t.resolve_asset("ETH");
    let budget = SLOW_BUDGET as i64;
    twap_at(t, &t.mock_reflector, &usdc, usd(1), at(TWAP_SPAN - budget));
    twap_at(t, partner, &eth, usd(1_800), at(0));

    let frozen = quote(t, &PriceKey::Token(usdc));
    assert!(
        frozen.valid && !frozen.stale,
        "quote inside budget: {frozen:?}"
    );
    assert_eq!(
        frozen.price_timestamp,
        at(-budget),
        "oldest sample at budget"
    );
}

/// `source_nature` labels the Scaled leg Fundamental, so `blend` never
/// compares its age with the market partner's: the 16 h old quote is blended
/// and the controller lends on it. A quote that had followed the market
/// refuses the same borrow with #100.
#[test]
fn cr1_fundamental_factor_hides_a_16h_old_market_quote_from_the_spread_bound() {
    let (mut t, eth_key, partner) = cr1_fixture(FeedNature::Fundamental, SLOW_BUDGET);
    freeze_quote_and_drop_market(&t, &partner);

    let blended = quote(&t, &eth_key);
    assert!(
        blended.valid && !blended.stale && !blended.deviation,
        "blend accepted: {blended:?}"
    );
    assert_eq!(blended.final_wad, usd(1_900), "midpoint of 2_000 and 1_800");
    assert_eq!(blended.price_timestamp, at(-(SLOW_BUDGET as i64)));
    let feed = try_price(&t, &eth_key).expect("strict read accepts the blend");
    let spread = at(-TWAP_SPAN) - feed.timestamp;
    assert!(
        spread > MAX_LEG_AGE_SPREAD_SECONDS,
        "legs {spread} s apart exceed the {MAX_LEG_AGE_SPREAD_SECONDS} s bound"
    );

    // 10 ETH at the blend: 19_000 USD, 14_250 of LTV room at 75%; 0.23 WBTC
    // is 13_800. At the market feed the room is 13_500.
    t.supply(ALICE, "ETH", TEN_ETH);
    t.borrow(ALICE, "WBTC", WBTC_DEBT);

    let usdc = t.resolve_asset("USDC");
    twap_at(&t, &t.mock_reflector, &usdc, usd_frac(90, 100), at(0));
    let tracked = quote(&t, &eth_key);
    assert_eq!(tracked.final_wad, usd(1_800), "quote following the market");
    t.supply(BOB, "ETH", TEN_ETH);
    assert_contract_error(
        t.try_borrow(BOB, "WBTC", WBTC_DEBT),
        errors::INSUFFICIENT_COLLATERAL,
    );
}

/// Control: the same composition with the factor labelled Market is admitted
/// too (the factor budget is kept at the spread bound so `configs/script.sh`
/// would also pass it), and the same frozen quote then trips the bound with
/// #206. The gate reads the factor's label, not what the leg is made of.
#[test]
fn cr1_control_a_market_labelled_factor_trips_the_bound_on_the_same_quote() {
    let (t, eth_key, partner) = cr1_fixture(FeedNature::Market, MAX_LEG_AGE_SPREAD_SECONDS);
    freeze_quote_and_drop_market(&t, &partner);

    let status = quote(&t, &eth_key);
    assert!(!status.valid && status.stale, "bound trips: {status:?}");
    assert_eq!(status.error_code, Some(errors::PRICE_FEED_STALE));
    assert_contract_error(try_price(&t, &eth_key), errors::PRICE_FEED_STALE);
}

// ---------------------------------------------------------------------------
// CR-2 direction A: a Scaled leg labelled Market inherits a Fundamental age
// from its quote and trips the bound on a valid asset.
// ---------------------------------------------------------------------------

/// USDC on mainnet's shape (Reflector TWAP + RedStone Fundamental, 16 h
/// budget). ETH: `factor (Reflector quoting in USDC, Market) x USDC` blended
/// with `partner`. Returns the fixture, the ETH key and the RedStone adapter
/// that carries USDC's fundamental leg (and the "ETHF" feed at 2_000).
fn cr2_fixture(
    partner: impl FnOnce(&LendingTest, &Address) -> PriceSource,
    independence: impl FnOnce(&Env, &Address) -> IndependencePolicy,
) -> (LendingTest, PriceKey, Address) {
    let t = fresh_fixture();
    let usdc = t.resolve_asset("USDC");
    let eth = t.resolve_asset("ETH");
    let usdc_key = PriceKey::Token(usdc.clone());
    let eth_key = PriceKey::Token(eth.clone());

    let adapter = register_redstone_adapter(&t, &[("USDC", usd(1)), ("ETHF", usd(2_000))]);
    configure(
        &t,
        &usdc_key,
        oracle(
            &t.env,
            &[
                PriceSource::Feed(reflector_feed(&t.mock_reflector, &usdc, MARKET_BUDGET)),
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
    let second = partner(&t, &adapter);
    configure(
        &t,
        &eth_key,
        oracle(
            &t.env,
            &[
                scaled(
                    reflector_feed(&factor, &eth, MARKET_BUDGET),
                    usdc_key.clone(),
                ),
                second,
            ],
            SLOW_BUDGET,
            500,
            independence(&t.env, &adapter),
            DEFAULT_MIN_SANITY_PRICE_WAD,
            DEFAULT_MAX_SANITY_PRICE_WAD,
        ),
    );

    let fresh = quote(&t, &eth_key);
    assert!(fresh.valid && !fresh.stale, "fresh blend: {fresh:?}");
    assert_eq!(fresh.final_wad, usd(2_000));
    (t, eth_key, adapter)
}

/// Lags USDC's fundamental leg by five hours. USDC itself stays valid: a
/// Fundamental leg may lag its Market partner by its own budget.
fn lag_usdc_fundamental_leg(t: &LendingTest, adapter: &Address) {
    redstone_at(t, adapter, "USDC", usd(1), at(-18_000));
    let usdc = quote(t, &PriceKey::Token(t.resolve_asset("USDC")));
    assert!(usdc.valid && !usdc.stale, "USDC stays valid: {usdc:?}");
    assert_eq!(usdc.price_timestamp, at(-18_000), "USDC carries the lag");
}

/// Both ETH legs are Market by label, so the bound applies, but the Scaled
/// leg is stamped `min(factor, USDC)` and USDC is stamped by its lagging
/// Fundamental leg. Every feed is inside its own budget and ETH still reads
/// #206; a borrow against it fails closed (INV-ORACLE-01).
#[test]
fn cr2_market_factor_inherits_the_fundamental_quote_age_and_trips_the_bound() {
    let (mut t, eth_key, adapter) = cr2_fixture(
        |t, _| {
            let eth = t.resolve_asset("ETH");
            let partner = new_reflector(t, None);
            twap_at(t, &partner, &eth, usd(2_000), at(0));
            PriceSource::Feed(reflector_feed(&partner, &eth, MARKET_BUDGET))
        },
        |_, _| IndependencePolicy::RequireDisjoint,
    );
    t.supply(ALICE, "ETH", TEN_ETH);

    lag_usdc_fundamental_leg(&t, &adapter);

    let status = quote(&t, &eth_key);
    assert!(!status.valid && status.stale, "bound misfires: {status:?}");
    assert_eq!(status.error_code, Some(errors::PRICE_FEED_STALE));
    assert_contract_error(try_price(&t, &eth_key), errors::PRICE_FEED_STALE);
    assert_contract_error(t.try_borrow(ALICE, "WBTC", 0.01), errors::PRICE_FEED_STALE);
}

/// Mainnet's Scaled shape (USTRY, CETES, AQUA): Market factor, Fundamental
/// partner, `AllowShared` on the RedStone adapter. The same quote lag is
/// harmless there. No mainnet listing pairs a Scaled leg with a Market
/// partner, so neither direction is reachable without a new listing.
#[test]
fn mainnet_shape_market_factor_with_fundamental_partner_ignores_the_quote_lag() {
    let (mut t, eth_key, adapter) = cr2_fixture(
        |t, adapter| {
            PriceSource::Feed(redstone_feed(
                &t.env,
                adapter,
                "ETHF",
                FeedNature::Fundamental,
                SLOW_BUDGET,
            ))
        },
        |env, adapter| IndependencePolicy::AllowShared(vec![env, adapter.clone()]),
    );
    t.supply(ALICE, "ETH", TEN_ETH);

    lag_usdc_fundamental_leg(&t, &adapter);

    let status = quote(&t, &eth_key);
    assert!(
        status.valid && !status.stale,
        "mainnet shape prices: {status:?}"
    );
    assert_eq!(status.final_wad, usd(2_000));
    assert_eq!(status.price_timestamp, at(-18_000), "blend carries the lag");
    t.borrow(ALICE, "WBTC", 0.01);
}
