//! Coverage for the dependency-graph half of `admin.rs`: the cascade that
//! revalidates composed oracles when a key they are built on changes, the
//! cycle guard that keeps that walk from recursing forever, and the LP
//! source-count rule.
use super::*;

use crate::registry;
use crate::test_support::{in_contract, register_redstone_feed};
use common::constants::WAD;
use common::types::{
    AquariusLpSource, FeedNature, IndependencePolicy, MultiFeedRef, OracleTolerance, ScaledSource,
};
use mock_redstone::MockRedStonePriceFeedClient;
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{Address, String, Symbol};

const NOW: u64 = 1_000_000;
const CEILING: u64 = 3_600;

fn at_now(env: &Env) {
    env.ledger().set_timestamp(NOW);
}

fn feed(env: &Env, adapter: &Address, id: &str, max_stale: u64) -> FeedSource {
    FeedSource {
        provider: ProviderRef::RedStone(MultiFeedRef {
            contract: adapter.clone(),
            feed_id: String::from_str(env, id),
            nature: FeedNature::Fundamental,
        }),
        decimals: 8,
        max_stale_seconds: max_stale,
    }
}

fn one(env: &Env, source: PriceSource) -> Vec<PriceSource> {
    let mut out = Vec::new(env);
    out.push_back(source);
    out
}

fn oracle_of(env: &Env, sources: Vec<PriceSource>) -> AssetOracle {
    let _ = env;
    AssetOracle {
        asset_decimals: 8,
        max_price_stale_seconds: CEILING,
        sources,
        tolerance: OracleTolerance {
            upper_ratio_bps: 10_500,
            lower_ratio_bps: 9_524,
        },
        independence: IndependencePolicy::RequireDisjoint,
        // Every oracle here resolves to ~100 WAD, and a single-source band is
        // capped at MAX_SINGLE_SOURCE_SANITY_BAND_BPS (1000). 95..105 is 500 bps.
        min_sanity_price_wad: 95 * WAD,
        max_sanity_price_wad: 105 * WAD,
    }
}

fn publish(client: &MockRedStonePriceFeedClient, env: &Env, id: &str, price: i128) {
    let ts_ms = NOW * 1_000;
    client.set_price_data(&String::from_str(env, id), &price, &ts_ms, &ts_ms);
}

fn scaled_onto(env: &Env, adapter: &Address, quote: PriceKey) -> PriceSource {
    PriceSource::Scaled(ScaledSource {
        factor: feed(env, adapter, "RATIO", CEILING),
        quote,
        // `validation::factor_bounds` caps the max at `MAX_REASONABLE_PRICE_WAD`.
        // The published RATIO (1 WAD) sits inside this band.
        min_factor_wad: WAD / 2,
        max_factor_wad: 2 * WAD,
    })
}

/// Registers `base <- mid <- leaf`, each quoting the one before it, writing
/// straight to the registry so no validation or probing runs. Returns the three
/// keys in that order.
fn chain(env: &Env, adapter: &Address) -> (PriceKey, PriceKey, PriceKey) {
    // A Token key, not a Ref: validation::asset_decimals requires a Ref to
    // declare 0 decimals, and set_oracle validates the key it is handed.
    let base = PriceKey::Token(Address::generate(env));
    let mid = PriceKey::Token(Address::generate(env));
    let leaf = PriceKey::Token(Address::generate(env));

    registry::store_oracle(
        env,
        &base,
        &oracle_of(
            env,
            one(env, PriceSource::Feed(feed(env, adapter, "BASE", CEILING))),
        ),
    );
    registry::store_oracle(
        env,
        &mid,
        &oracle_of(env, one(env, scaled_onto(env, adapter, base.clone()))),
    );
    registry::store_oracle(
        env,
        &leaf,
        &oracle_of(env, one(env, scaled_onto(env, adapter, mid.clone()))),
    );
    (base, mid, leaf)
}

#[test]
fn depends_on_follows_the_chain_past_its_direct_quote() {
    let env = Env::default();
    let adapter = Address::generate(&env);
    in_contract(&env, || {
        let (base, mid, leaf) = chain(&env, &adapter);

        // The direct edge, which is all a non-recursive walk would find.
        assert!(depends_on(&env, &mid, &base, &mut Vec::new(&env)));
        // The transitive one: leaf quotes mid, and only mid quotes base.
        assert!(depends_on(&env, &leaf, &base, &mut Vec::new(&env)));
        // Direction matters -- the base does not depend on what is built on it.
        assert!(!depends_on(&env, &base, &leaf, &mut Vec::new(&env)));
    });
}

#[test]
fn depends_on_reports_no_match_for_a_key_outside_the_graph() {
    let env = Env::default();
    let adapter = Address::generate(&env);
    in_contract(&env, || {
        let (_base, _mid, leaf) = chain(&env, &adapter);
        let stranger = PriceKey::Ref(Symbol::new(&env, "OTHER"));
        assert!(!depends_on(&env, &leaf, &stranger, &mut Vec::new(&env)));
    });
}

#[test]
fn depends_on_terminates_on_a_cycle_rather_than_recursing_forever() {
    let env = Env::default();
    let adapter = Address::generate(&env);
    in_contract(&env, || {
        // A quotes B and B quotes A. `set_oracle` rejects this cycle, so the
        // registry is written directly.
        let a = PriceKey::Token(Address::generate(&env));
        let b = PriceKey::Token(Address::generate(&env));
        registry::store_oracle(
            &env,
            &a,
            &oracle_of(&env, one(&env, scaled_onto(&env, &adapter, b.clone()))),
        );
        registry::store_oracle(
            &env,
            &b,
            &oracle_of(&env, one(&env, scaled_onto(&env, &adapter, a.clone()))),
        );

        // Without the `visiting` guard this walk recurses until the host traps.
        let stranger = PriceKey::Ref(Symbol::new(&env, "OTHER"));
        assert!(!depends_on(&env, &a, &stranger, &mut Vec::new(&env)));
        // The direct edge from `a` to `b` still matches.
        assert!(depends_on(&env, &a, &b, &mut Vec::new(&env)));
    });
}

#[test]
fn set_oracle_revalidates_the_oracles_stacked_on_the_changed_key() {
    let env = Env::default();
    at_now(&env);
    let (adapter, client) = register_redstone_feed(&env);
    publish(&client, &env, "BASE", 100 * WAD);
    publish(&client, &env, "RATIO", WAD);

    in_contract(&env, || {
        let (base, mid, leaf) = chain(&env, &adapter);

        // Rewriting the base drives revalidate_dependents over mid and leaf:
        // each is re-fetched and re-validated under the new state.
        set_oracle(
            &env,
            base.clone(),
            oracle_of(
                &env,
                one(
                    &env,
                    PriceSource::Feed(feed(&env, &adapter, "BASE", CEILING)),
                ),
            ),
        );

        // The dependents survived the cascade intact rather than being dropped
        // or rewritten by it.
        assert!(registry::get_oracle(&env, &mid).is_some());
        assert!(registry::get_oracle(&env, &leaf).is_some());
        assert!(registry::get_oracle(&env, &base).is_some());
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #231)")]
fn an_lp_source_may_not_share_its_oracle_with_a_second_source() {
    let env = Env::default();
    let adapter = Address::generate(&env);
    in_contract(&env, || {
        // The LP's two legs are resolved through the registry while the derived
        // properties are built, which happens before the source-count rule, so
        // they have to exist or the test stops on OracleNotConfigured instead.
        let key_a = PriceKey::Ref(Symbol::new(&env, "A"));
        let key_b = PriceKey::Ref(Symbol::new(&env, "B"));
        for k in [&key_a, &key_b] {
            registry::store_oracle(
                &env,
                k,
                &oracle_of(
                    &env,
                    one(
                        &env,
                        PriceSource::Feed(feed(&env, &adapter, "BASE", CEILING)),
                    ),
                ),
            );
        }
        let lp = PriceSource::AquariusLp(AquariusLpSource {
            pool: Address::generate(&env),
            token_a: Address::generate(&env),
            token_b: Address::generate(&env),
            key_a,
            key_b,
            reserve_a_decimals: 7,
            reserve_b_decimals: 7,
            min_pool_value_wad: 1,
        });
        let mut sources = Vec::new(&env);
        sources.push_back(lp);
        sources.push_back(PriceSource::Feed(feed(&env, &adapter, "BASE", CEILING)));

        // An LP share price is not a quote that can be averaged against a feed,
        // so it has to be the sole source on its oracle.
        let key = PriceKey::Token(Address::generate(&env));
        validate_asset_oracle(&env, &key, &oracle_of(&env, sources));
    });
}

/// A RedStone-shaped feed that counts its reads in its own storage, so a test
/// can count how many times the cascade crosses the contract boundary.
#[soroban_sdk::contract]
pub(crate) struct CountingRedStoneFeed;

#[soroban_sdk::contractimpl]
impl CountingRedStoneFeed {
    pub fn set_price_data(
        env: Env,
        feed_id: String,
        price_wad: i128,
        package_timestamp: u64,
        write_timestamp: u64,
    ) {
        let price_8 = (price_wad / 10_000_000_000) as u128;
        env.storage().persistent().set(
            &feed_id,
            &common::oracle::providers::redstone::RedStonePriceData {
                price: soroban_sdk::U256::from_u128(&env, price_8),
                package_timestamp,
                write_timestamp,
            },
        );
    }

    pub fn reads(env: Env) -> u32 {
        env.storage()
            .persistent()
            .get(&Symbol::new(&env, "reads"))
            .unwrap_or(0)
    }

    pub fn read_price_data_for_feed(
        env: Env,
        feed_id: String,
    ) -> Option<common::oracle::providers::redstone::RedStonePriceData> {
        let key = Symbol::new(&env, "reads");
        let n: u32 = env.storage().persistent().get(&key).unwrap_or(0);
        env.storage().persistent().set(&key, &(n + 1));
        env.storage().persistent().get(&feed_id)
    }
}

/// Rewriting `base` in `base <- mid <- leaf` revalidates `mid` and `leaf` from
/// the registry alone. The only provider read is the probe of `base` itself.
/// Each live provider read instantiates a callee VM, which the transaction
/// memory budget pays for.
#[test]
fn revalidating_dependents_crosses_no_contract_boundary() {
    let env = Env::default();
    at_now(&env);
    let adapter = env.register(CountingRedStoneFeed, ());
    let client = CountingRedStoneFeedClient::new(&env, &adapter);
    let ts_ms = NOW * 1_000;
    client.set_price_data(
        &String::from_str(&env, "BASE"),
        &(100 * WAD),
        &ts_ms,
        &ts_ms,
    );
    client.set_price_data(&String::from_str(&env, "RATIO"), &WAD, &ts_ms, &ts_ms);

    in_contract(&env, || {
        let (base, _mid, _leaf) = chain(&env, &adapter);
        set_oracle(
            &env,
            base.clone(),
            oracle_of(
                &env,
                one(
                    &env,
                    PriceSource::Feed(feed(&env, &adapter, "BASE", CEILING)),
                ),
            ),
        );
    });

    // Probing `base` reads BASE once; revalidating `mid` and `leaf` adds no read.
    let reads = client.reads();
    assert_eq!(
        reads, 1,
        "revalidating dependents must not cross a contract boundary, saw {reads} reads"
    );
}

/// The structural check on dependents fires without a live probe. The chain
/// `base <- mid <- leaf` sits at depth 2 from `leaf`. Re-pointing
/// `base` onto a two-deep stack pushes `leaf` to depth 4, past
/// `MAX_RESOLUTION_DEPTH`, and the edit to `base` must be refused even though
/// `base` itself validates.
#[test]
#[should_panic(expected = "Error(Contract, #229)")]
fn a_base_edit_that_pushes_a_dependent_past_the_depth_limit_is_rejected() {
    let env = Env::default();
    at_now(&env);
    let (adapter, client) = register_redstone_feed(&env);
    publish(&client, &env, "BASE", 100 * WAD);
    publish(&client, &env, "RATIO", WAD);

    in_contract(&env, || {
        let (base, _mid, _leaf) = chain(&env, &adapter);

        let deeper = PriceKey::Token(Address::generate(&env));
        let deep = PriceKey::Token(Address::generate(&env));
        registry::store_oracle(
            &env,
            &deeper,
            &oracle_of(
                &env,
                one(
                    &env,
                    PriceSource::Feed(feed(&env, &adapter, "BASE", CEILING)),
                ),
            ),
        );
        registry::store_oracle(
            &env,
            &deep,
            &oracle_of(&env, one(&env, scaled_onto(&env, &adapter, deeper))),
        );

        // deeper <- deep <- base is depth 2 from base and passes on its own;
        // deeper <- deep <- base <- mid <- leaf is depth 4 and must not.
        set_oracle(
            &env,
            base,
            oracle_of(&env, one(&env, scaled_onto(&env, &adapter, deep))),
        );
    });
}
