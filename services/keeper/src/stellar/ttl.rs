//! `ExtendFootprintTtl` operation builder.

use anyhow::{anyhow, Result};
use stellar_xdr::curr::{
    ExtendFootprintTtlOp, ExtensionPoint, LedgerKey, Operation, OperationBody,
    SorobanTransactionData, VecM,
};

use crate::stellar::tx::{empty_soroban_data, TxJob, TxKind};

/// Stellar-core cap for `ExtendFootprintTtlOp.extend_to`.
pub const MAX_LEDGERS_TO_EXTEND: u32 = 535_679;

/// Builds an extend op for read-only footprint keys.
pub fn extend_footprint_ttl(read_only_keys: &[LedgerKey], extend_to_ledgers: u32) -> Result<TxJob> {
    if extend_to_ledgers == 0 {
        return Err(anyhow!("extend_to_ledgers must be > 0"));
    }
    if read_only_keys.is_empty() {
        return Err(anyhow!(
            "ExtendFootprintTtl needs at least one read-only key"
        ));
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

/// Builds seed Soroban data with read-only footprint keys.
fn build_extend_soroban_data(read_only_keys: &[LedgerKey]) -> Result<SorobanTransactionData> {
    let read_only: VecM<LedgerKey> = read_only_keys
        .try_into()
        .map_err(|_| anyhow!("too many ExtendFootprintTtl read-only keys"))?;

    let mut data = empty_soroban_data();
    data.resources.footprint.read_only = read_only;
    Ok(data)
}
