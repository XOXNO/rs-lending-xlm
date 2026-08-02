use super::*;
use crate::test_support::{in_contract, CountingRedStoneAdapter, CountingRedStoneAdapterClient};
use common::constants::WAD;
use common::types::{
    AssetOracle, FeedNature, FeedSource, IndependencePolicy, MultiFeedRef, OracleTolerance,
    ScaledSource,
};
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{Address, Symbol};

fn token(env: &Env) -> PriceKey {
    PriceKey::Token(Address::generate(env))
}

#[test]
fn test_price_memo_round_trips_per_key() {
    let env = Env::default();
    let mut session = Session::new(&env);
    let key = token(&env);

    assert!(session.cached_price(&key).is_none());
    session.store_price(
        &key,
        PriceFeedRaw {
            price_wad: 7,
            asset_decimals: 8,
            timestamp: 100,
        },
    );
    assert_eq!(session.cached_price(&key).unwrap().price_wad, 7);
}

#[test]
fn test_price_and_status_memos_are_separate() {
    let env = Env::default();
    let mut session = Session::new(&env);
    let key = token(&env);

    session.store_status(&key, PriceStatus::unusable());
    assert!(session.cached_status(&key).is_some());
    assert!(
        session.cached_price(&key).is_none(),
        "a stored status must never satisfy a price lookup"
    );
}

#[test]
fn test_cycle_stack_rejects_reentry() {
    let env = Env::default();
    let mut session = Session::new(&env);
    let key = token(&env);
    session.push_key(&key);

    assert!(session.is_resolving(&key));
    session.pop_key();
    assert!(!session.is_resolving(&key));
}

#[test]
fn test_distinct_keys_do_not_collide() {
    let env = Env::default();
    let mut session = Session::new(&env);
    let a = token(&env);
    let b = PriceKey::Ref(Symbol::new(&env, "BTC"));
    session.push_key(&a);
    assert!(!session.is_resolving(&b));
    session.pop_key();
}

const NOW: u64 = 1_000_000;
const CEILING: u64 = 3_600;

fn multi_feed(env: &Env, adapter: &Address, feed_id: &str) -> FeedSource {
    FeedSource {
        provider: ProviderRef::RedStone(MultiFeedRef {
            contract: adapter.clone(),
            feed_id: String::from_str(env, feed_id),
            nature: FeedNature::Fundamental,
        }),
        decimals: 8,
        max_stale_seconds: CEILING,
    }
}

fn oracle_of(env: &Env, source: PriceSource) -> AssetOracle {
    let mut sources = Vec::new(env);
    sources.push_back(source);
    AssetOracle {
        asset_decimals: 8,
        max_price_stale_seconds: CEILING,
        sources,
        tolerance: OracleTolerance {
            upper_ratio_bps: 10_500,
            lower_ratio_bps: 9_524,
        },
        independence: IndependencePolicy::RequireDisjoint,
        min_sanity_price_wad: WAD / 2,
        max_sanity_price_wad: 2 * WAD,
    }
}

fn counting_adapter<'a>(
    env: &'a Env,
    feeds: &[&str],
) -> (Address, CountingRedStoneAdapterClient<'a>) {
    let id = env.register(CountingRedStoneAdapter, ());
    let client = CountingRedStoneAdapterClient::new(env, &id);
    for feed in feeds {
        client.set_price(&String::from_str(env, feed), &WAD);
    }
    (id, client)
}

fn feed_key(env: &Env, adapter: &Address, feed_id: &str) -> PriceKey {
    let key = PriceKey::Token(Address::generate(env));
    crate::registry::store_oracle(
        env,
        &key,
        &oracle_of(env, PriceSource::Feed(multi_feed(env, adapter, feed_id))),
    );
    key
}

#[test]
fn test_one_bulk_read_serves_every_leg_and_no_lazy_read_follows() {
    let env = Env::default();
    env.ledger().set_timestamp(NOW);
    let (adapter, client) = counting_adapter(&env, &["A", "B"]);

    in_contract(&env, || {
        let first = feed_key(&env, &adapter, "A");
        let second = feed_key(&env, &adapter, "B");
        let keys = Vec::from_array(&env, [first.clone(), second.clone()]);

        let mut session = Session::new(&env);
        session.warm(&keys);
        assert_eq!(
            crate::engine::resolve(&mut session, &first, 0).price_wad,
            WAD
        );
        assert_eq!(
            crate::engine::resolve(&mut session, &second, 0).price_wad,
            WAD
        );
    });

    let (single, bulk, batch) = client.counts();
    assert_eq!(bulk, 1, "one bulk read for the whole batch");
    assert_eq!(batch, 2, "both feed ids in it");
    assert_eq!(single, 0, "and nothing read lazily afterwards");
}

#[test]
fn test_lp_underlyings_are_included_in_bulk_prefetch() {
    let env = Env::default();
    env.ledger().set_timestamp(NOW);
    let (adapter, client) = counting_adapter(&env, &["A", "B"]);

    in_contract(&env, || {
        let first = feed_key(&env, &adapter, "A");
        let second = feed_key(&env, &adapter, "B");
        let lp = PriceKey::Token(Address::generate(&env));
        crate::registry::store_oracle(
            &env,
            &lp,
            &oracle_of(
                &env,
                PriceSource::AquariusLp(common::types::AquariusLpSource {
                    pool: Address::generate(&env),
                    plane: Address::generate(&env),
                    token_a: Address::generate(&env),
                    token_b: Address::generate(&env),
                    key_a: first,
                    key_b: second,
                    reserve_a_decimals: 7,
                    reserve_b_decimals: 7,
                    min_pool_value_wad: WAD,
                }),
            ),
        );

        Session::new(&env).warm(&Vec::from_array(&env, [lp]));
    });

    let (single, bulk, batch) = client.counts();
    assert_eq!(bulk, 1);
    assert_eq!(batch, 2);
    assert_eq!(single, 0);
}

#[test]
fn test_a_lone_feed_is_read_lazily_rather_than_bulked() {
    let env = Env::default();
    env.ledger().set_timestamp(NOW);
    let (adapter, client) = counting_adapter(&env, &["A"]);

    in_contract(&env, || {
        let only = feed_key(&env, &adapter, "A");
        let mut session = Session::new(&env);
        session.warm(&Vec::from_array(&env, [only.clone()]));
        assert_eq!(
            crate::engine::resolve(&mut session, &only, 0).price_wad,
            WAD
        );
    });

    let (single, bulk, _) = client.counts();
    assert_eq!(bulk, 0, "one feed does not earn a bulk call");
    assert_eq!(single, 1);
}

#[test]
fn test_the_prefetch_walk_stops_at_the_composition_cap() {
    let env = Env::default();
    env.ledger().set_timestamp(NOW);
    let (adapter, client) = counting_adapter(&env, &["F1", "F2", "F3", "F4", "LEAF"]);

    in_contract(&env, || {
        let mut current = PriceKey::Ref(Symbol::new(&env, "LEAF"));
        crate::registry::store_oracle(
            &env,
            &current,
            &oracle_of(&env, PriceSource::Feed(multi_feed(&env, &adapter, "LEAF"))),
        );

        for (name, factor) in [("L4", "F4"), ("L3", "F3"), ("L2", "F2"), ("L1", "F1")] {
            let key = PriceKey::Ref(Symbol::new(&env, name));
            crate::registry::store_oracle(
                &env,
                &key,
                &oracle_of(
                    &env,
                    PriceSource::Scaled(ScaledSource {
                        factor: multi_feed(&env, &adapter, factor),
                        quote: current.clone(),
                        min_factor_wad: WAD,
                        max_factor_wad: WAD,
                    }),
                ),
            );
            current = key;
        }

        let mut session = Session::new(&env);
        session.warm(&Vec::from_array(&env, [current.clone()]));
    });

    let (_, bulk, batch) = client.counts();
    assert_eq!(bulk, 1);
    assert_eq!(
        batch,
        MAX_RESOLUTION_DEPTH + 1,
        "one factor leg per level the cap admits, and the leaf past it left out"
    );
}

// An LP's underlyings sit one level below the LP itself, exactly like a scaled
// source's quote. An LP parked at the cap therefore contributes nothing to the
// batch: its legs are past the depth the resolver will walk at read time, so
// prefetching them would buy feeds no read can use.
#[test]
fn test_the_prefetch_walk_leaves_out_the_legs_of_an_lp_sitting_at_the_cap() {
    let env = Env::default();
    env.ledger().set_timestamp(NOW);
    let (adapter, client) = counting_adapter(&env, &["F1", "F2", "F3", "LA", "LB"]);

    in_contract(&env, || {
        let mut current = PriceKey::Ref(Symbol::new(&env, "LP"));
        crate::registry::store_oracle(
            &env,
            &current,
            &oracle_of(
                &env,
                PriceSource::AquariusLp(common::types::AquariusLpSource {
                    pool: Address::generate(&env),
                    plane: Address::generate(&env),
                    token_a: Address::generate(&env),
                    token_b: Address::generate(&env),
                    key_a: feed_key(&env, &adapter, "LA"),
                    key_b: feed_key(&env, &adapter, "LB"),
                    reserve_a_decimals: 7,
                    reserve_b_decimals: 7,
                    min_pool_value_wad: WAD,
                }),
            ),
        );

        for (name, factor) in [("L3", "F3"), ("L2", "F2"), ("L1", "F1")] {
            let key = PriceKey::Ref(Symbol::new(&env, name));
            crate::registry::store_oracle(
                &env,
                &key,
                &oracle_of(
                    &env,
                    PriceSource::Scaled(ScaledSource {
                        factor: multi_feed(&env, &adapter, factor),
                        quote: current.clone(),
                        min_factor_wad: WAD,
                        max_factor_wad: WAD,
                    }),
                ),
            );
            current = key;
        }

        Session::new(&env).warm(&Vec::from_array(&env, [current]));
    });

    let (_, bulk, batch) = client.counts();
    assert_eq!(bulk, 1);
    assert_eq!(
        batch, MAX_RESOLUTION_DEPTH,
        "the three factor legs the cap admits, and neither LP underlying"
    );
}

#[test]
fn test_a_short_bulk_response_is_discarded_rather_than_misaligned() {
    let env = Env::default();
    env.ledger().set_timestamp(NOW);
    let (adapter, client) = counting_adapter(&env, &["A", "B"]);
    client.set_short(&true);

    in_contract(&env, || {
        let first = feed_key(&env, &adapter, "A");
        let second = feed_key(&env, &adapter, "B");
        let keys = Vec::from_array(&env, [first.clone(), second.clone()]);

        let mut session = Session::new(&env);
        session.warm(&keys);
        assert_eq!(
            crate::engine::resolve(&mut session, &first, 0).price_wad,
            WAD
        );
        assert_eq!(
            crate::engine::resolve(&mut session, &second, 0).price_wad,
            WAD
        );
    });

    let (single, bulk, _) = client.counts();
    assert_eq!(bulk, 1, "the batch was attempted");
    assert_eq!(single, 2, "and then thrown away, both legs read singly");
}
