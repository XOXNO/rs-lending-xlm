use common::types::HubAssetKey;
use soroban_sdk::testutils::{Ledger, LedgerInfo};
use soroban_sdk::{Address, Env};

/// Protocol version the simulated ledger reports to the host.
///
/// A version older than the protocol `soroban-sdk` targets fails every test with
/// `Error(Context, InternalError)` ("ledger protocol version too old for host").
/// Keep in step with the workspace `soroban-sdk` pin.
pub(crate) const LEDGER_PROTOCOL_VERSION: u32 = 28;

pub(crate) fn init_ledger(env: &Env) {
    env.ledger().set(LedgerInfo {
        timestamp: 1_000,
        protocol_version: LEDGER_PROTOCOL_VERSION,
        sequence_number: 100,
        network_id: Default::default(),
        base_reserve: 10,
        min_temp_entry_ttl: 10,
        min_persistent_entry_ttl: 10,
        max_entry_ttl: 3_110_400,
    });
}

pub(crate) fn hub(asset: &Address) -> HubAssetKey {
    HubAssetKey {
        hub_id: 0,
        asset: asset.clone(),
    }
}
