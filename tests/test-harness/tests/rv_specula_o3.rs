//! RV second opinion on Specula Oracle CR-6: `set_oracle` revalidation walks
//! the whole registry (`admin::revalidate_dependents`), so the ledger footprint
//! of one `set_oracle` grows by one entry per registered key and crosses the
//! per-transaction footprint limit at 383 registered keys.
//!
//! The harness builds its `Env` with resource limits disabled and the budget
//! unlimited, so the footprint is measured through
//! `cost_estimate().resources()` and compared with the SDK's snapshot of the
//! mainnet limit. Re-arming `enforce_resource_limits` is deliberately not
//! used: the host snapshots storage resources under a metered shadow budget
//! and truncates the count for footprints this large, so the limit check sees
//! a partial number and never fires.

use controller::types::{AssetOracle, PriceKey};
use governance::op::{AdminOperation, ConfigureAssetOracleArgs};
use soroban_sdk::Symbol;
use test_harness::LendingTest;

/// `InvocationResourceLimits::mainnet().ledger_entries` in soroban-sdk 28.0.0,
/// the SDK's 2026-07-10 snapshot of the mainnet per-transaction footprint
/// limit (disk reads + in-memory reads + writes). The SDK does not re-export
/// the struct, so the number is pinned here.
const MAINNET_FOOTPRINT_ENTRIES: u32 = 400;

/// `Ref` keys registered on top of the two market keys the fixture seeds.
/// With 383 keys registered, the governed `set_oracle` adding the 384th is
/// the first one over the limit (382 registered lands exactly on 400, which
/// the host still accepts).
const FILL: u32 = 381;

/// Entries one `set_oracle` touches beyond the registry: the aggregator
/// instance and code, the governance instance and code, the owner entry, the
/// mock Reflector instance, code and price entries, and the new oracle entry.
/// Measured, not derived; the assertions below bound it rather than pin it.
const BASELINE_UPPER_BOUND: u32 = 24;

/// A valid single-source oracle for a `Ref` key: the USDC market oracle with
/// `asset_decimals` forced to `0`, which `validation::asset_decimals` demands
/// for `Ref` keys. Every probe reads the same mock Reflector price, so the
/// provider-side footprint is constant across the registry.
fn ref_template(t: &LendingTest) -> AssetOracle {
    let usdc = t.resolve_asset("USDC");
    let mut oracle = t
        .price_agg_client()
        .oracle(&PriceKey::Token(usdc))
        .expect("USDC oracle seeded by the fixture");
    oracle.asset_decimals = 0;
    oracle
}

fn ref_key(t: &LendingTest, i: u32) -> PriceKey {
    PriceKey::Ref(Symbol::new(&t.env, &format!("RK{i}")))
}

/// Total footprint entries of the last invocation, the quantity the host
/// compares with `ledger_entries` when it enforces limits.
fn footprint_entries(t: &LendingTest) -> u32 {
    let r = t.env.cost_estimate().resources();
    r.disk_read_entries + r.memory_read_entries + r.write_entries
}

fn register_ref(t: &LendingTest, i: u32, template: &AssetOracle) {
    t.price_agg_client().set_oracle(&ref_key(t, i), template);
}

#[test]
fn set_oracle_footprint_grows_one_entry_per_registered_key_until_the_limit() {
    let t = LendingTest::new().standard_two_asset().build();
    let template = ref_template(&t);

    let mut at_100 = 0;
    let mut at_200 = 0;
    let mut at_350 = 0;
    for i in 0..FILL {
        register_ref(&t, i, &template);
        match i + 1 {
            100 => at_100 = footprint_entries(&t),
            200 => at_200 = footprint_entries(&t),
            350 => at_350 = footprint_entries(&t),
            _ => {}
        }
    }

    // revalidate_dependents reads every registered oracle entry once, so 100
    // more keys cost exactly 100 more footprint entries.
    assert_eq!(
        at_200 - at_100,
        100,
        "footprint should grow by one entry per registered key: {at_100} -> {at_200}"
    );
    // The direct path at 352 registered keys still fits: this is a threshold,
    // not a constant failure.
    assert!(
        at_350 < MAINNET_FOOTPRINT_ENTRIES,
        "direct set_oracle at 352 keys touches {at_350} entries"
    );
    let baseline_direct = at_350 - (350 + 2 + 1);
    assert!(
        baseline_direct <= BASELINE_UPPER_BOUND,
        "direct non-registry baseline is {baseline_direct} entries"
    );

    // The production path: governance executes ConfigureAssetOracle, which adds
    // its own entries on top of the aggregator's walk. With 383 keys registered
    // and one more being added, the transaction no longer fits.
    let admin = t.admin();
    t.gov_client().execute_immediate(
        &admin,
        &AdminOperation::ConfigureAssetOracle(ConfigureAssetOracleArgs {
            key: ref_key(&t, FILL),
            oracle: template.clone(),
        }),
    );
    let via_governance = footprint_entries(&t);
    assert!(
        via_governance > MAINNET_FOOTPRINT_ENTRIES,
        "with {} registered keys one governed set_oracle touches {via_governance} entries, mainnet allows {MAINNET_FOOTPRINT_ENTRIES}",
        FILL + 2
    );
    let baseline_governed = via_governance - (FILL + 2 + 1);
    assert!(
        baseline_governed <= BASELINE_UPPER_BOUND,
        "governed non-registry baseline is {baseline_governed} entries; the key threshold is about {}",
        MAINNET_FOOTPRINT_ENTRIES - baseline_governed
    );
}
