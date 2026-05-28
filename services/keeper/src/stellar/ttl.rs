//! `ExtendFootprintTTL` op builder for wasm-code entries.
//!
//! Soroban auto-bumps storage entries on access and contracts can call
//! `extend_ttl` on their own instance/persistent/temporary entries. What a
//! contract **cannot** extend is its own `ContractCode` entry — the WASM
//! blob keyed by hash. Those entries (one per unique wasm hash deployed:
//! controller, pool template, flash-loan receiver) must be extended off
//! chain via the raw `ExtendFootprintTtlOp` operation. That's the only
//! place the keeper uses this op.

use anyhow::{anyhow, Result};
use stellar_xdr::curr::{
    ExtendFootprintTtlOp, ExtensionPoint, LedgerKey, Operation, OperationBody,
};

use crate::stellar::tx::{TxJob, TxKind};

/// Build an `ExtendFootprintTtl` operation that targets `read_only_keys` for
/// `extend_to` ledgers. Simulation later populates the actual
/// `SorobanTransactionData.footprint.read_only` field; here we only build the
/// op body — the footprint itself is inferred by `simulate_transaction`.
///
/// Practically, we still embed the read_only footprint in the
/// `SorobanTransactionData` we pass at simulate time so the RPC knows which
/// keys we intend to bump. That field is produced by [`build_soroban_data`]
/// and stitched in by `tx::build_envelope`'s caller.
pub fn extend_footprint_ttl(
    read_only_keys: &[LedgerKey],
    extend_to_ledgers: u32,
) -> Result<TxJob> {
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

/// Build a fully populated `SorobanTransactionData` for an extend job whose
/// footprint must be declared up front (the RPC needs it to estimate
/// resource fees). The tx pipeline replaces the initial empty soroban data
/// with this one before simulation.
pub fn build_extend_soroban_data(read_only_keys: &[LedgerKey]) -> Result<stellar_xdr::curr::SorobanTransactionData> {
    use stellar_xdr::curr::{LedgerFootprint, SorobanResources, SorobanTransactionDataExt, VecM};

    let read_only: VecM<LedgerKey> = read_only_keys
        .to_vec()
        .try_into()
        .map_err(|_| anyhow!("too many ExtendFootprintTtl read-only keys"))?;

    Ok(stellar_xdr::curr::SorobanTransactionData {
        ext: SorobanTransactionDataExt::V0,
        resources: SorobanResources {
            footprint: LedgerFootprint {
                read_only,
                read_write: VecM::default(),
            },
            instructions: 0,
            disk_read_bytes: 0,
            write_bytes: 0,
        },
        resource_fee: 0,
    })
}
