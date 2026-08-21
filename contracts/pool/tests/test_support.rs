use common::types::HubAssetKey;
use soroban_sdk::testutils::{Ledger, LedgerInfo};
use soroban_sdk::{Address, Env};

/// Protocol version the simulated ledger reports to the host.
///
/// The host rejects a ledger older than the protocol `soroban-sdk` was built
/// for, raising `Error(Context, InternalError)` with "ledger protocol version
/// too old for host" -- which reads as an unexplained mass failure rather than
/// a version mismatch. Keep in step with the workspace `soroban-sdk` pin.
pub(crate) const LEDGER_PROTOCOL_VERSION: u32 = 27;

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
