use super::*;
use crate::admin;
use crate::session::Session;
use crate::test_support::{in_contract, register_redstone_feed};
use common::constants::WAD;
use common::types::{
    AssetOracle, FeedNature, FeedSource, IndependencePolicy, LpShareSource, MultiFeedRef,
    OracleTolerance, PoolKind, ProviderKind, ProviderRef, ScaledSource,
};
use mock_redstone::MockRedStonePriceFeedClient;
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{Address, String, Symbol, Vec};

const NOW: u64 = 1_000_000;
const ASSET_CEILING: u64 = 3_600;
const RATIO_BOUND: u64 = 86_400;

fn at_now(env: &Env) {
    env.ledger().set_timestamp(NOW);
}

fn multi_feed(env: &Env, adapter: &Address, feed: &str, max_stale: u64) -> FeedSource {
    FeedSource {
        provider: ProviderRef::MultiFeed(MultiFeedRef {
            contract: adapter.clone(),
            feed_id: String::from_str(env, feed),
            kind: ProviderKind::RedStone,
            nature: FeedNature::Fundamental,
        }),
        decimals: 8,
        max_stale_seconds: max_stale,
    }
}

fn oracle(
    env: &Env,
    sources: Vec<PriceSource>,
    ceiling: u64,
    min_wad: i128,
    max_wad: i128,
) -> AssetOracle {
    let _ = env;
    AssetOracle {
        asset_decimals: 8,
        max_price_stale_seconds: ceiling,
        sources,
        tolerance: OracleTolerance {
            upper_ratio_bps: 10_500,
            lower_ratio_bps: 9_524,
        },
        independence: IndependencePolicy::RequireDisjoint,
        min_sanity_price_wad: min_wad,
        max_sanity_price_wad: max_wad,
    }
}

fn sources(env: &Env, items: &[PriceSource]) -> Vec<PriceSource> {
    let mut out = Vec::new(env);
    for item in items {
        out.push_back(item.clone());
    }
    out
}

/// Publishes `feed_id` at `price`, dated `age` seconds before the ledger clock.
fn publish(client: &MockRedStonePriceFeedClient, env: &Env, feed_id: &str, price: i128, age: u64) {
    let ts_ms = (NOW - age) * 1_000;
    client.set_price_data(&String::from_str(env, feed_id), &price, &ts_ms, &ts_ms);
}

fn single_feed_key(env: &Env, adapter: &Address, feed: &str, ceiling: u64) -> PriceKey {
    let key = PriceKey::Token(Address::generate(env));
    admin::store_oracle(
        env,
        &key,
        &oracle(
            env,
            sources(
                env,
                &[PriceSource::Feed(multi_feed(env, adapter, feed, ceiling))],
            ),
            ceiling,
            WAD / 2,
            2 * WAD,
        ),
    );
    key
}

// ---------------------------------------------------------------------------
// Single source
// ---------------------------------------------------------------------------

#[test]
fn test_single_feed_source_resolves() {
    let env = Env::default();
    at_now(&env);
    let (adapter, client) = register_redstone_feed(&env);
    publish(&client, &env, "USST", WAD, 0);

    in_contract(&env, || {
        let key = single_feed_key(&env, &adapter, "USST", ASSET_CEILING);
        let mut cache = Session::new(&env);
        let feed = resolve(&mut cache, &key, 0);
        assert_eq!(feed.price_wad, WAD);
        assert_eq!(feed.asset_decimals, 8);
    });
}

#[test]
#[should_panic]
fn test_a_feed_past_its_own_bound_reverts() {
    let env = Env::default();
    at_now(&env);
    let (adapter, client) = register_redstone_feed(&env);
    publish(&client, &env, "USST", WAD, ASSET_CEILING + 1);

    in_contract(&env, || {
        let key = single_feed_key(&env, &adapter, "USST", ASSET_CEILING);
        let mut cache = Session::new(&env);
        let _ = resolve(&mut cache, &key, 0);
    });
}

#[test]
#[should_panic]
fn test_a_price_outside_the_sanity_band_reverts() {
    let env = Env::default();
    at_now(&env);
    let (adapter, client) = register_redstone_feed(&env);
    publish(&client, &env, "USST", 5 * WAD, 0);

    in_contract(&env, || {
        let key = single_feed_key(&env, &adapter, "USST", ASSET_CEILING);
        let mut cache = Session::new(&env);
        let _ = resolve(&mut cache, &key, 0);
    });
}

// ---------------------------------------------------------------------------
// Two sources: symmetry is the point of the redesign
// ---------------------------------------------------------------------------

fn dual_key(env: &Env, adapter: &Address, reversed: bool) -> PriceKey {
    let key = PriceKey::Token(Address::generate(env));
    let a = PriceSource::Feed(multi_feed(env, adapter, "A", ASSET_CEILING));
    let b = PriceSource::Feed(multi_feed(env, adapter, "B", ASSET_CEILING));
    let ordered = if reversed { [b, a] } else { [a, b] };
    admin::store_oracle(
        env,
        &key,
        &oracle(env, sources(env, &ordered), ASSET_CEILING, WAD / 2, 2 * WAD),
    );
    key
}

#[test]
fn test_two_agreeing_sources_yield_their_midpoint() {
    let env = Env::default();
    at_now(&env);
    let (adapter, client) = register_redstone_feed(&env);
    publish(&client, &env, "A", WAD, 0);
    publish(&client, &env, "B", WAD + WAD / 100, 0);

    in_contract(&env, || {
        let key = dual_key(&env, &adapter, false);
        let mut cache = Session::new(&env);
        assert_eq!(resolve(&mut cache, &key, 0).price_wad, WAD + WAD / 200);
    });
}

#[test]
fn test_source_order_changes_neither_price_nor_outcome() {
    // The invariant the symmetric model rests on: there is no primary and no
    // anchor, so swapping the two must be unobservable.
    let env = Env::default();
    at_now(&env);
    let (adapter, client) = register_redstone_feed(&env);
    publish(&client, &env, "A", WAD, 0);
    publish(&client, &env, "B", WAD + WAD / 100, 0);

    let (forward, backward) = in_contract(&env, || {
        let forward_key = dual_key(&env, &adapter, false);
        let backward_key = dual_key(&env, &adapter, true);
        let mut cache = Session::new(&env);
        (
            resolve(&mut cache, &forward_key, 0).price_wad,
            resolve(&mut cache, &backward_key, 0).price_wad,
        )
    });
    assert_eq!(forward, backward);
}

#[test]
#[should_panic]
fn test_two_disagreeing_sources_revert() {
    let env = Env::default();
    at_now(&env);
    let (adapter, client) = register_redstone_feed(&env);
    publish(&client, &env, "A", WAD, 0);
    publish(&client, &env, "B", WAD + WAD / 2, 0);

    in_contract(&env, || {
        let key = dual_key(&env, &adapter, false);
        let mut cache = Session::new(&env);
        let _ = resolve(&mut cache, &key, 0);
    });
}

// ---------------------------------------------------------------------------
// Scaled: the SolvBTC shape
// ---------------------------------------------------------------------------

/// `Ref("BTC")` priced by one feed, and a token scaled onto it.
fn scaled_setup(
    env: &Env,
    adapter: &Address,
    ceiling: u64,
    min_factor: i128,
    max_factor: i128,
) -> PriceKey {
    let btc = PriceKey::Ref(Symbol::new(env, "BTC"));
    admin::store_oracle(
        env,
        &btc,
        &oracle(
            env,
            sources(
                env,
                &[PriceSource::Feed(multi_feed(env, adapter, "BTC", 3_600))],
            ),
            3_600,
            WAD / 2,
            1_000_000 * WAD,
        ),
    );

    let token = PriceKey::Token(Address::generate(env));
    admin::store_oracle(
        env,
        &token,
        &oracle(
            env,
            sources(
                env,
                &[PriceSource::Scaled(ScaledSource {
                    factor: multi_feed(env, adapter, "RATIO", RATIO_BOUND),
                    quote: btc,
                    min_factor_wad: min_factor,
                    max_factor_wad: max_factor,
                })],
            ),
            ceiling,
            WAD / 2,
            1_000_000 * WAD,
        ),
    );
    token
}

#[test]
fn test_scaled_source_multiplies_ratio_by_quote() {
    let env = Env::default();
    at_now(&env);
    let (adapter, client) = register_redstone_feed(&env);
    publish(&client, &env, "BTC", 100 * WAD, 0);
    publish(&client, &env, "RATIO", WAD + WAD / 100, 0);

    in_contract(&env, || {
        let token = scaled_setup(&env, &adapter, RATIO_BOUND, WAD, 2 * WAD);
        let mut cache = Session::new(&env);
        // 1.01 x 100 = 101
        assert_eq!(resolve(&mut cache, &token, 0).price_wad, 101 * WAD);
    });
}

#[test]
#[should_panic]
fn test_a_ratio_outside_its_bounds_reverts() {
    // A compromised ratio feed would otherwise reprice the asset arbitrarily
    // inside a sanity band sized for the quote's volatility.
    let env = Env::default();
    at_now(&env);
    let (adapter, client) = register_redstone_feed(&env);
    publish(&client, &env, "BTC", 100 * WAD, 0);
    publish(&client, &env, "RATIO", WAD / 2, 0);

    in_contract(&env, || {
        let token = scaled_setup(&env, &adapter, RATIO_BOUND, WAD, 2 * WAD);
        let mut cache = Session::new(&env);
        let _ = resolve(&mut cache, &token, 0);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #217)")]
fn test_scaled_product_overflow_is_typed_invalid_price_not_host_trap() {
    // Bypass write-time factor caps via store_oracle. Multi-feed raw is 8-dec;
    // upscale by 1e10, then factor×quote / WAD must not fit i128 → InvalidPrice
    // (#217) via try_mul, not a host MathOverflow trap.
    let env = Env::default();
    at_now(&env);
    let (adapter, client) = register_redstone_feed(&env);
    // Largest raw that still normalizes: ≈ i128::MAX / 1e10 → ~1e28 WAD each.
    let huge_raw = i128::MAX / 10i128.pow(10);
    publish(&client, &env, "BTC", huge_raw, 0);
    publish(&client, &env, "RATIO", huge_raw, 0);

    in_contract(&env, || {
        let btc = PriceKey::Ref(Symbol::new(&env, "BTC"));
        admin::store_oracle(
            &env,
            &btc,
            &oracle(
                &env,
                sources(
                    &env,
                    &[PriceSource::Feed(multi_feed(&env, &adapter, "BTC", 3_600))],
                ),
                3_600,
                1,
                i128::MAX,
            ),
        );
        let token = PriceKey::Token(Address::generate(&env));
        admin::store_oracle(
            &env,
            &token,
            &oracle(
                &env,
                sources(
                    &env,
                    &[PriceSource::Scaled(ScaledSource {
                        factor: multi_feed(&env, &adapter, "RATIO", RATIO_BOUND),
                        quote: btc,
                        min_factor_wad: 1,
                        max_factor_wad: i128::MAX,
                    })],
                ),
                RATIO_BOUND,
                1,
                i128::MAX,
            ),
        );
        let mut cache = Session::new(&env);
        let _ = resolve(&mut cache, &token, 0);
    });
}

#[test]
#[should_panic]
fn test_a_frozen_slow_leg_cannot_ride_under_a_live_fast_one() {
    // The regression that per-leg-only gating would reintroduce.
    //
    // The ratio publisher goes silent - plausibly *because* a depeg started -
    // while the quote keeps updating. Every component still passes its own
    // bound: the ratio's 86400 window is nowhere near expired. Without the
    // composite gate at the asset ceiling, a live quote multiplied by a frozen
    // ratio would keep printing a fresh-looking price at par.
    let env = Env::default();
    at_now(&env);
    let (adapter, client) = register_redstone_feed(&env);
    publish(&client, &env, "BTC", 100 * WAD, 0);
    publish(&client, &env, "RATIO", WAD, ASSET_CEILING * 2);

    in_contract(&env, || {
        let token = scaled_setup(&env, &adapter, ASSET_CEILING, WAD / 2, 2 * WAD);
        let mut cache = Session::new(&env);
        let _ = resolve(&mut cache, &token, 0);
    });
}

#[test]
fn test_the_same_frozen_leg_prices_under_a_ceiling_that_allows_it() {
    // Mirror of the test above: the gate is the asset ceiling, not an arbitrary
    // tightening, so a config that genuinely tolerates a 12h heartbeat still
    // prices. Gating a composite at its *tightest* leg would fail this.
    let env = Env::default();
    at_now(&env);
    let (adapter, client) = register_redstone_feed(&env);
    publish(&client, &env, "BTC", 100 * WAD, 0);
    publish(&client, &env, "RATIO", WAD, ASSET_CEILING * 2);

    in_contract(&env, || {
        let token = scaled_setup(&env, &adapter, RATIO_BOUND, WAD / 2, 2 * WAD);
        let mut cache = Session::new(&env);
        assert_eq!(resolve(&mut cache, &token, 0).price_wad, 100 * WAD);
    });
}

#[test]
fn test_a_composite_reports_the_freshness_of_its_weaker_leg() {
    let env = Env::default();
    at_now(&env);
    let (adapter, client) = register_redstone_feed(&env);
    publish(&client, &env, "BTC", 100 * WAD, 0);
    publish(&client, &env, "RATIO", WAD, 600);

    in_contract(&env, || {
        let token = scaled_setup(&env, &adapter, RATIO_BOUND, WAD / 2, 2 * WAD);
        let mut cache = Session::new(&env);
        assert_eq!(resolve(&mut cache, &token, 0).timestamp, NOW - 600);
    });
}

// ---------------------------------------------------------------------------
// Bounds and unsupported shapes
// ---------------------------------------------------------------------------

#[test]
#[should_panic]
fn test_an_unconfigured_key_reverts_rather_than_pricing_zero() {
    let env = Env::default();
    at_now(&env);
    in_contract(&env, || {
        let mut cache = Session::new(&env);
        let _ = resolve(&mut cache, &PriceKey::Ref(Symbol::new(&env, "NOPE")), 0);
    });
}

#[test]
#[should_panic]
fn test_lp_shares_are_not_priceable_yet() {
    let env = Env::default();
    at_now(&env);
    in_contract(&env, || {
        let key = PriceKey::Token(Address::generate(&env));
        admin::store_oracle(
            &env,
            &key,
            &oracle(
                &env,
                sources(
                    &env,
                    &[PriceSource::LpShare(LpShareSource {
                        pool: Address::generate(&env),
                        kind: PoolKind::ConstantProduct,
                        key_a: PriceKey::Ref(Symbol::new(&env, "A")),
                        key_b: PriceKey::Ref(Symbol::new(&env, "B")),
                        reserve_a_decimals: 7,
                        reserve_b_decimals: 7,
                        share_decimals: 7,
                    })],
                ),
                ASSET_CEILING,
                1,
                i128::MAX / 2,
            ),
        );
        let mut cache = Session::new(&env);
        let _ = resolve(&mut cache, &key, 0);
    });
}

#[test]
#[should_panic]
fn test_a_scaled_cycle_reverts_at_read_time_too() {
    // Config-time validation walks the graph as it was then; a dependency
    // re-pointed since could reintroduce a cycle, so the read path must not
    // trust that earlier promise.
    let env = Env::default();
    at_now(&env);
    let (adapter, _client) = register_redstone_feed(&env);

    in_contract(&env, || {
        let key = PriceKey::Ref(Symbol::new(&env, "LOOP"));
        admin::store_oracle(
            &env,
            &key,
            &oracle(
                &env,
                sources(
                    &env,
                    &[PriceSource::Scaled(ScaledSource {
                        factor: multi_feed(&env, &adapter, "RATIO", RATIO_BOUND),
                        quote: key.clone(),
                        min_factor_wad: 1,
                        max_factor_wad: i128::MAX,
                    })],
                ),
                ASSET_CEILING,
                1,
                i128::MAX / 2,
            ),
        );
        let mut cache = Session::new(&env);
        let _ = resolve(&mut cache, &key, 0);
    });
}
