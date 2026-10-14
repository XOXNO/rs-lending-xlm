//! RV second opinion on Specula Oracle CR-6: `set_oracle` revalidation walks
//! the whole registry (`admin::revalidate_dependents`), so the ledger footprint
//! of one `set_oracle` grows by one entry per registered key. Before PR #196
//! the registry was unbounded and a governed `set_oracle` crossed the
//! per-transaction footprint limit at 383 registered keys. PR #196 (2d68305)
//! caps the registry at `MAX_ORACLE_KEYS` and refuses a new key past the cap
//! with `OracleRegistryFull` (233), so the walk stays under the limit.
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
use test_harness::errors::OracleError;
use test_harness::{assert_contract_error, LendingTest};

/// `InvocationResourceLimits::mainnet().ledger_entries` in soroban-sdk 28.0.0,
/// the SDK's 2026-07-10 snapshot of the mainnet per-transaction footprint
/// limit (disk reads + in-memory reads + writes). The SDK does not re-export
/// the struct, so the number is pinned here.
const MAINNET_FOOTPRINT_ENTRIES: u32 = 400;

/// Upper bound on the `Ref` keys the probe tries to register; the cap must
/// bind before it (`MAX_ORACLE_KEYS` is crate-private).
const PROBE_LIMIT: u32 = 400;

/// Entries one `set_oracle` touches beyond the registry: the aggregator
/// instance and code, the governance instance and code, the owner entry, the
/// mock Reflector instance, code and price entries, and the new oracle entry.
/// Measured, not derived; the assertions below bound it rather than pin it.
const BASELINE_UPPER_BOUND: u32 = 24;

const ORACLE_REGISTRY_FULL: u32 = OracleError::OracleRegistryFull as u32;

fn flat<T, E: Into<soroban_sdk::Error>>(
    result: Result<Result<T, E>, Result<soroban_sdk::Error, soroban_sdk::InvokeError>>,
) -> Result<(), soroban_sdk::Error> {
    match result {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(err)) => Err(err.into()),
        Err(Ok(err)) => Err(err),
        Err(Err(_)) => panic!("expected a contract error, got an InvokeError"),
    }
}
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

fn try_register_ref(
    t: &LendingTest,
    i: u32,
    template: &AssetOracle,
) -> Result<(), soroban_sdk::Error> {
    flat(
        t.price_agg_client()
            .try_set_oracle(&ref_key(t, i), template),
    )
}

fn configure_ref(
    t: &LendingTest,
    i: u32,
    template: &AssetOracle,
) -> Result<(), soroban_sdk::Error> {
    flat(t.gov_client().try_execute_immediate(
        &t.admin(),
        &AdminOperation::ConfigureAssetOracle(ConfigureAssetOracleArgs {
            key: ref_key(t, i),
            oracle: template.clone(),
        }),
    ))
}

/// Fills the registry until the cap refuses a new key, measuring the
/// footprint on the way. The walk still costs one entry per registered key,
/// but the cap holds it, and a governed reconfiguration of a key in the full
/// registry, the widest walk governance can trigger, stays under the limit.
#[test]
fn set_oracle_footprint_grows_one_entry_per_registered_key_up_to_the_registry_cap() {
    let t = LendingTest::new().standard_two_asset().build();
    let template = ref_template(&t);

    let mut at_25 = 0;
    let mut at_50 = 0;
    let mut last_admitted = 0;
    let mut admitted = 0u32;
    loop {
        assert!(
            admitted < PROBE_LIMIT,
            "no registry cap below {PROBE_LIMIT} keys"
        );
        match try_register_ref(&t, admitted, &template) {
            Ok(()) => {
                admitted += 1;
                last_admitted = footprint_entries(&t);
                match admitted {
                    25 => at_25 = last_admitted,
                    50 => at_50 = last_admitted,
                    _ => {}
                }
            }
            Err(err) => {
                assert_eq!(
                    err,
                    soroban_sdk::Error::from_contract_error(ORACLE_REGISTRY_FULL),
                    "the cap refuses the key past it"
                );
                break;
            }
        }
    }
    let registered = admitted + 2;
    std::println!(
        "registry cap: {registered} keys; last direct set_oracle {last_admitted} entries"
    );
    assert!(admitted >= 50, "the cap admits at least 50 Ref keys");

    // revalidate_dependents reads every registered oracle entry once, so 25
    // more keys cost exactly 25 more footprint entries.
    assert_eq!(
        at_50 - at_25,
        25,
        "footprint should grow by one entry per registered key: {at_25} -> {at_50}"
    );
    assert!(
        last_admitted < MAINNET_FOOTPRINT_ENTRIES,
        "the last admitted direct set_oracle touches {last_admitted} entries"
    );
    assert!(
        last_admitted - registered <= BASELINE_UPPER_BOUND,
        "direct non-registry baseline is {} entries",
        last_admitted - registered
    );

    // The production path refuses a new key the same way, and reconfiguring a
    // key in the full registry walks it all and still fits.
    assert_contract_error(configure_ref(&t, admitted, &template), ORACLE_REGISTRY_FULL);
    configure_ref(&t, 0, &template).expect("reconfiguring a registered key is admitted");
    let via_governance = footprint_entries(&t);
    assert!(
        via_governance < MAINNET_FOOTPRINT_ENTRIES,
        "one governed set_oracle over the full registry touches {via_governance} entries, mainnet allows {MAINNET_FOOTPRINT_ENTRIES}"
    );
    assert!(
        via_governance - registered <= BASELINE_UPPER_BOUND,
        "governed non-registry baseline is {} entries",
        via_governance - registered
    );
}
