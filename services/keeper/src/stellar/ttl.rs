//! `ExtendFootprintTtl` op builder.
//!
//! Soroban auto-bumps storage entries on access, and a contract can call
//! `extend_ttl` on its own instance/persistent/temporary entries. What a
//! contract **cannot** extend is any entry outside the transaction's own
//! footprint — most importantly its `ContractCode` (WASM blob, keyed by hash).
//! The keeper bumps those entries (and the controller's own storage, since the
//! keeper is not the controller) off chain via the raw `ExtendFootprintTtlOp`.

use anyhow::{anyhow, Result};
use stellar_xdr::curr::{
    ExtendFootprintTtlOp, ExtensionPoint, LedgerKey, Operation, OperationBody,
    SorobanTransactionData, VecM,
};

use crate::stellar::tx::{empty_soroban_data, TxJob, TxKind};

/// Ledgers-from-now to extend entries to.
///
/// `ExtendFootprintTtlOp.extend_to` is a count of ledgers from the current
/// ledger (not an absolute sequence number), and `ExtendFootprintTtl` ops are
/// capped at `MAX_LEDGERS_TO_EXTEND` in stellar-core: **535,679** (≈ 31 days).
/// Any value above that is rejected at submission with
/// `OpInner(ExtendFootprintTtl(Malformed))`. With a tick cadence well under
/// this window, extending to the cap every tick keeps entries perpetually
/// fresh.
pub const MAX_LEDGERS_TO_EXTEND: u32 = 535_679;

/// Build an `ExtendFootprintTtl` operation that targets `read_only_keys` for
/// `extend_to_ledgers` ledgers. The read-only footprint is declared up front in
/// the seed `SorobanTransactionData` so the RPC can estimate resource fees; the
/// tx pipeline swaps in the simulator's refined data before signing.
pub fn extend_footprint_ttl(read_only_keys: &[LedgerKey], extend_to_ledgers: u32) -> Result<TxJob> {
    if extend_to_ledgers == 0 {
        return Err(anyhow!("extend_to_ledgers must be > 0"));
    }
    if read_only_keys.is_empty() {
        return Err(anyhow!("ExtendFootprintTtl needs at least one read-only key"));
    }
    Ok(TxJob {
        kind: TxKind::ExtendFootprintTtl,
        op: Operation {
            source_account: None,
            body: OperationBody::ExtendFootprintTtl(ExtendFootprintTtlOp {
                ext: ExtensionPoint::V0,
                extend_to: extend_to_ledgers,
            }),
        },
        initial_soroban_data: Some(build_extend_soroban_data(read_only_keys)?),
    })
}

/// Build the seed `SorobanTransactionData` whose read-only footprint names the
/// keys to bump. The simulator refines the resource estimates before signing.
fn build_extend_soroban_data(read_only_keys: &[LedgerKey]) -> Result<SorobanTransactionData> {
    let read_only: VecM<LedgerKey> = read_only_keys
        .try_into()
        .map_err(|_| anyhow!("too many ExtendFootprintTtl read-only keys"))?;

    let mut data = empty_soroban_data();
    data.resources.footprint.read_only = read_only;
    Ok(data)
}
