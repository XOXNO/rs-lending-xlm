//! Shared test fixtures. Gated `#[cfg(test)]` at the module level in `lib.rs`.

use soroban_sdk::testutils::{Ledger, LedgerInfo};
use soroban_sdk::Env;

/// Sets the canonical ledger snapshot used by all pool unit tests.
/// Timestamp is in **seconds**; pool code multiplies by 1000 internally to
/// reach the milliseconds it stores in `last_timestamp`.
pub(crate) fn init_ledger(env: &Env) {
    env.ledger().set(LedgerInfo {
        timestamp: 1_000,
        protocol_version: 26,
        sequence_number: 100,
        network_id: Default::default(),
        base_reserve: 10,
        min_temp_entry_ttl: 10,
        min_persistent_entry_ttl: 10,
        max_entry_ttl: 3_110_400,
    });
}
