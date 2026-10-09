//! Second opinion on Specula oracle CR-7. `admin::validate_asset_oracle`
//! waives the 10% single-source sanity-band cap for any pair whose two trust
//! sets are not equal (`!validation::same_address_set`), so a pair that
//! shares a provider contract keeps the exemption. The shared contract then
//! owns one leg outright and the other leg through its factor, so what bounds
//! a compromised shared provider is the factor band times the tolerance, and
//! admission never compares that with the cap.
//!
//! Every scenario goes through production entry points: governance
//! `ConfigureAssetOracle` (the immediate path skips only the delay, not the
//! aggregator's validation), `quotes`/`prices` for reads, and the controller's
//! `supply`/`borrow` for the lending effect. ETH stands in for the mainnet
//! SolvBTC listing and WBTC for its `Ref("BTC")` quote. One mock RedStone
//! adapter carries both the ratio and the USD anchor, as the mainnet adapter
//! does; the factor bounds are the aggregator unit fixture's `[1.0, 2.0]`
//! unless a scenario pins mainnet's `[1.00, 1.05]`.

use common::errors::OracleError;
use common::types::{PriceFeedRaw, PriceStatus};
use controller::types::{
    AssetOracle, FeedNature, FeedSource, IndependencePolicy, MultiFeedRef, PriceKey, PriceSource,
    ProviderRef, ScaledSource,
};
use governance::op::{AdminOperation, ConfigureAssetOracleArgs};
use soroban_sdk::testutils::Ledger as _;
use soroban_sdk::{vec, Address, Env, String, Vec};
use test_harness::errors;
use test_harness::mock_redstone::MockRedStonePriceFeedClient;
use test_harness::oracle::redstone::register_redstone_adapter;
use test_harness::{assert_contract_error, tolerance_band, usd, LendingTest, ALICE};

const T0: u64 = 100_000;
const WAD: i128 = 1_000_000_000_000_000_000;
const REDSTONE_DECIMALS: u32 = 8;
/// Mainnet SolvBTC: factor and asset budget 26 h, USD anchor budget 16 h.
const FACTOR_BUDGET: u64 = 93_600;
const ANCHOR_BUDGET: u64 = 57_600;
/// Mainnet SolvBTC tolerance: `upper_ratio_bps` 10_500.
const TOLERANCE_BPS: u32 = 500;
/// The aggregator unit fixture's band: 8_182 bps of `(max-min)/(max+min)`,
/// more than eight times `MAX_SINGLE_SOURCE_SANITY_BAND_BPS`.
const BAND_MIN: i128 = 20_000;
const BAND_MAX: i128 = 200_000;
/// Harness WBTC price, the honest value of a 1.0 ratio.
const BTC: i128 = 60_000;
/// What a cap-compliant single-source band around `BTC` would let through.
const TEN_PERCENT_CEILING: i128 = BTC + BTC / 10;
const RATIO_FEED: &str = "SOLV_RATIO";
const USD_FEED: &str = "SOLV_USD";

const BAND_TOO_WIDE: u32 = OracleError::SanityBandTooWideForSingleSource as u32;
const FACTOR_OUT_OF_BOUNDS: u32 = OracleError::FactorOutOfBounds as u32;

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

/// Governance immediate configuration as a `Result` carrying the aggregator's
/// own contract error.
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

fn redstone_feed(
    env: &Env,
    adapter: &Address,
    feed_id: &str,
    max_stale_seconds: u64,
) -> FeedSource {
    FeedSource {
        provider: ProviderRef::RedStone(MultiFeedRef {
            contract: adapter.clone(),
            feed_id: String::from_str(env, feed_id),
            nature: FeedNature::Fundamental,
        }),
        decimals: REDSTONE_DECIMALS,
        max_stale_seconds,
    }
}

fn oracle(
    env: &Env,
    items: &[PriceSource],
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
        max_price_stale_seconds: FACTOR_BUDGET,
        sources,
        tolerance: tolerance_band(env, TOLERANCE_BPS),
        independence,
        min_sanity_price_wad,
        max_sanity_price_wad,
    }
}

/// `ratio (RedStone on `ratio_adapter`) x WBTC` blended with a USD anchor on
/// `anchor_adapter`, under the unit fixture's wide band.
fn solv_pair(
    t: &LendingTest,
    ratio_adapter: &Address,
    anchor_adapter: &Address,
    max_factor_wad: i128,
    independence: IndependencePolicy,
) -> AssetOracle {
    let wbtc = PriceKey::Token(t.resolve_asset("WBTC"));
    oracle(
        &t.env,
        &[
            PriceSource::Scaled(ScaledSource {
                factor: redstone_feed(&t.env, ratio_adapter, RATIO_FEED, FACTOR_BUDGET),
                quote: wbtc,
                min_factor_wad: WAD,
                max_factor_wad,
            }),
            PriceSource::Feed(redstone_feed(
                &t.env,
                anchor_adapter,
                USD_FEED,
                ANCHOR_BUDGET,
            )),
        ],
        independence,
        usd(BAND_MIN),
        usd(BAND_MAX),
    )
}

fn allow_shared(env: &Env, adapter: &Address) -> IndependencePolicy {
    IndependencePolicy::AllowShared(vec![env, adapter.clone()])
}

/// Publishes a ratio and a USD anchor on `adapter`, signed now.
fn post(t: &LendingTest, adapter: &Address, ratio_wad: i128, anchor_wad: i128) {
    let client = MockRedStonePriceFeedClient::new(&t.env, adapter);
    client.set_price(&String::from_str(&t.env, RATIO_FEED), &ratio_wad);
    client.set_price(&String::from_str(&t.env, USD_FEED), &anchor_wad);
}

/// USDC, ETH and WBTC at `T0`, plus one adapter publishing an honest 1.0
/// ratio and a 60_000 anchor. Returns the fixture, the ETH key and the adapter.
fn fixture() -> (LendingTest, PriceKey, Address) {
    let t = LendingTest::new().three_asset_usdc_eth_wbtc().build();
    set_time(&t, T0);
    t.refresh_oracle_prices();
    let adapter = register_redstone_adapter(&t, &[(RATIO_FEED, WAD), (USD_FEED, usd(BTC))]);
    let eth_key = PriceKey::Token(t.resolve_asset("ETH"));
    (t, eth_key, adapter)
}

/// Admission keeps the band-cap exemption for the shared-adapter pair: the
/// first leg's trust set is `{adapter, Reflector}` and the second's is
/// `{adapter}`, which differ, so an 8_182 bps band passes. The same band is
/// refused with #226 once the two trust sets are equal (two plain feeds on
/// the adapter) and for a single feed, so the predicate is set inequality,
/// not disjointness.
#[test]
fn cr7_a_shared_adapter_pair_keeps_the_band_cap_exemption() {
    let (t, eth_key, adapter) = fixture();

    try_configure(
        &t,
        &eth_key,
        solv_pair(
            &t,
            &adapter,
            &adapter,
            2 * WAD,
            allow_shared(&t.env, &adapter),
        ),
    )
    .expect("a shared-adapter pair is admitted under the wide band");
    let stored = t.price_agg_client().oracle(&eth_key).expect("stored");
    assert!(stored.is_dual());
    assert_eq!(stored.max_sanity_price_wad, usd(BAND_MAX));
    let honest = quote(&t, &eth_key);
    assert!(honest.valid, "honest blend: {honest:?}");
    assert_eq!(honest.final_wad, usd(BTC));

    let same_trust = oracle(
        &t.env,
        &[
            PriceSource::Feed(redstone_feed(&t.env, &adapter, USD_FEED, ANCHOR_BUDGET)),
            PriceSource::Feed(redstone_feed(&t.env, &adapter, RATIO_FEED, ANCHOR_BUDGET)),
        ],
        allow_shared(&t.env, &adapter),
        usd(BAND_MIN),
        usd(BAND_MAX),
    );
    assert_contract_error(try_configure(&t, &eth_key, same_trust), BAND_TOO_WIDE);

    let single = oracle(
        &t.env,
        &[PriceSource::Feed(redstone_feed(
            &t.env,
            &adapter,
            USD_FEED,
            ANCHOR_BUDGET,
        ))],
        IndependencePolicy::RequireDisjoint,
        usd(BAND_MIN),
        usd(BAND_MAX),
    );
    assert_contract_error(try_configure(&t, &eth_key, single), BAND_TOO_WIDE);
}

/// With the unit fixture's `[1.0, 2.0]` factor bounds, the shared adapter
/// moves the ratio to 1.9 and the anchor with it. Both legs agree, the blend
/// is inside the wide band and `prices` serves 114_000 for an asset worth
/// 60_000: 73% past what a cap-compliant band could admit. The controller
/// lends 720_000 against 600_000 of honest collateral, and the position is
/// under water once the adapter publishes honestly again.
#[test]
fn cr7_the_shared_adapter_walks_the_price_past_the_cap_and_the_controller_lends_on_it() {
    let (mut t, eth_key, adapter) = fixture();
    try_configure(
        &t,
        &eth_key,
        solv_pair(
            &t,
            &adapter,
            &adapter,
            2 * WAD,
            allow_shared(&t.env, &adapter),
        ),
    )
    .expect("admitted");

    post(&t, &adapter, WAD * 19 / 10, usd(114_000));
    let inflated = quote(&t, &eth_key);
    assert!(
        inflated.valid && !inflated.deviation && !inflated.stale,
        "inflated blend accepted: {inflated:?}"
    );
    assert_eq!(inflated.final_wad, usd(114_000));
    assert!(
        inflated.final_wad > usd(TEN_PERCENT_CEILING),
        "past the single-source cap's ceiling"
    );
    let feed = try_price(&t, &eth_key).expect("strict read accepts the blend");
    assert_eq!(feed.price_wad, usd(114_000));

    // 10 ETH at 114_000 is 1_140_000 of collateral and 855_000 of LTV room at
    // 75%; 12 WBTC at 60_000 is 720_000 of debt.
    t.supply(ALICE, "ETH", 10.0);
    t.borrow(ALICE, "WBTC", 12.0);
    assert_eq!(t.total_debt_raw(ALICE), usd(720_000));

    post(&t, &adapter, WAD, usd(BTC));
    let honest = quote(&t, &eth_key);
    assert_eq!(honest.final_wad, usd(BTC));
    assert_eq!(t.total_collateral_raw(ALICE), usd(600_000));
    assert!(
        t.total_collateral_raw(ALICE) < t.total_debt_raw(ALICE),
        "bad debt at the honest price"
    );
    assert!(t.health_factor_raw(ALICE) < WAD);
    t.assert_liquidatable(ALICE);
}

/// Mainnet SolvBTC bounds `[1.00, 1.05]`: the ratio cannot leave the factor
/// band (#230), and at the ceiling the blend is at most
/// `1.05 x (1 + 1.05) / 2 = 1.07625` of the honest value, under the 10% the
/// cap tolerates. The reachable range ratio `max/min x upper_ratio` is
/// 1.1025, inside the threat model's `u <= 11/9`, so with LT 80% the account
/// is never under water at the honest price. Nothing in the contract pins
/// these bounds; they are the listing's values.
#[test]
fn cr7_control_mainnet_factor_bounds_hold_the_walk_under_the_cap() {
    let (mut t, eth_key, adapter) = fixture();
    let max_factor = WAD * 105 / 100;
    try_configure(
        &t,
        &eth_key,
        solv_pair(
            &t,
            &adapter,
            &adapter,
            max_factor,
            allow_shared(&t.env, &adapter),
        ),
    )
    .expect("admitted");
    let stored = t.price_agg_client().oracle(&eth_key).expect("stored");
    let PriceSource::Scaled(scaled) = stored.sources.get_unchecked(0) else {
        panic!("first source is the scaled leg");
    };
    let upper = i128::from(stored.tolerance.upper_ratio_bps);
    assert!(
        scaled.max_factor_wad * upper * 9 <= scaled.min_factor_wad * 10_000 * 11,
        "range ratio max/min x upper_ratio within 11/9"
    );

    post(&t, &adapter, WAD * 19 / 10, usd(114_000));
    let refused = quote(&t, &eth_key);
    assert!(!refused.valid, "factor refused: {refused:?}");
    assert_eq!(refused.error_code, Some(FACTOR_OUT_OF_BOUNDS));
    assert_contract_error(try_price(&t, &eth_key), FACTOR_OUT_OF_BOUNDS);

    // Ratio at the ceiling, anchor exactly 5% above the scaled leg: legs
    // 63_000 and 66_150, midpoint 64_575.
    post(&t, &adapter, max_factor, usd(66_150));
    let ceiling = quote(&t, &eth_key);
    assert!(
        ceiling.valid && !ceiling.deviation,
        "ceiling blend accepted: {ceiling:?}"
    );
    assert_eq!(ceiling.final_wad, usd(64_575));
    assert!(ceiling.final_wad <= usd(TEN_PERCENT_CEILING));

    // 10 ETH at 64_575 gives 484_312.5 of LTV room; borrow 480_000.
    t.supply(ALICE, "ETH", 10.0);
    t.borrow(ALICE, "WBTC", 8.0);

    post(&t, &adapter, WAD, usd(BTC));
    assert_eq!(t.total_collateral_raw(ALICE), usd(600_000));
    assert!(
        t.total_collateral_raw(ALICE) > t.total_debt_raw(ALICE),
        "no bad debt at the honest price"
    );
}

/// A disjoint anchor is what the exemption assumes. With the anchor on a
/// second adapter, the first adapter's 1.9 ratio disagrees with the honest
/// anchor and the blend fails closed with #203; at 1.05 it is admitted and
/// moves the midpoint by the half-tolerance only.
#[test]
fn cr7_control_a_disjoint_anchor_holds_a_compromised_adapter_to_the_tolerance() {
    let (t, eth_key, adapter) = fixture();
    let honest_anchor = register_redstone_adapter(&t, &[(USD_FEED, usd(BTC))]);
    try_configure(
        &t,
        &eth_key,
        solv_pair(
            &t,
            &adapter,
            &honest_anchor,
            2 * WAD,
            IndependencePolicy::RequireDisjoint,
        ),
    )
    .expect("a disjoint pair is admitted under the wide band");

    post(&t, &adapter, WAD * 19 / 10, usd(114_000));
    let disagreeing = quote(&t, &eth_key);
    assert!(
        !disagreeing.valid && disagreeing.deviation,
        "legs disagree: {disagreeing:?}"
    );
    assert_eq!(disagreeing.error_code, Some(errors::UNSAFE_PRICE));
    assert_contract_error(try_price(&t, &eth_key), errors::UNSAFE_PRICE);

    post(&t, &adapter, WAD * 105 / 100, usd(114_000));
    let bounded = quote(&t, &eth_key);
    assert!(bounded.valid, "inside tolerance: {bounded:?}");
    assert_eq!(
        bounded.final_wad,
        usd(61_500),
        "midpoint of 63_000 and 60_000"
    );
}
