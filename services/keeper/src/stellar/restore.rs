//! `RestoreFootprint` operation builder.

use anyhow::{anyhow, Result};
use stellar_xdr::curr::{
    ExtensionPoint, LedgerKey, Operation, OperationBody, RestoreFootprintOp,
    SorobanTransactionData, VecM,
};

use crate::stellar::tx::{empty_soroban_data, TxJob, TxKind};

pub fn restore_footprint(read_write_keys: &[LedgerKey]) -> Result<TxJob> {
    if read_write_keys.is_empty() {
        return Err(anyhow!(
            "RestoreFootprint needs at least one read-write key"
        ));
    }
    Ok(TxJob {
        kind: TxKind::RestoreFootprint,
        op: Operation {
            source_account: None,
            body: OperationBody::RestoreFootprint(RestoreFootprintOp {
                ext: ExtensionPoint::V0,
            }),
        },
        initial_soroban_data: Some(build_restore_soroban_data(read_write_keys)?),
    })
}

fn build_restore_soroban_data(read_write_keys: &[LedgerKey]) -> Result<SorobanTransactionData> {
    let read_write: VecM<LedgerKey> = read_write_keys
        .try_into()
        .map_err(|_| anyhow!("too many RestoreFootprint read-write keys"))?;

    let mut data = empty_soroban_data();
    data.resources.footprint.read_write = read_write;
    Ok(data)
}

#[cfg(test)]
mod tests {
    use crate::stellar::restore::restore_footprint;
    use crate::stellar::tx::TxKind;
    use stellar_xdr::curr::{
        ContractDataDurability, ContractId, Hash, LedgerKey, LedgerKeyContractData, OperationBody,
        ScAddress, ScVal,
    };

    fn key(n: u8) -> LedgerKey {
        LedgerKey::ContractData(LedgerKeyContractData {
            contract: ScAddress::Contract(ContractId(Hash([n; 32]))),
            key: ScVal::LedgerKeyContractInstance,
            durability: ContractDataDurability::Persistent,
        })
    }

    #[test]
    fn builds_restore_op_with_keys_in_read_write_footprint() {
        let job = restore_footprint(&[key(1), key(2)]).unwrap();

        assert_eq!(job.kind, TxKind::RestoreFootprint);
        assert!(matches!(job.op.body, OperationBody::RestoreFootprint(_)));

        let data = job
            .initial_soroban_data
            .expect("restore seeds soroban data");
        assert_eq!(data.resources.footprint.read_write.len(), 2);
        assert_eq!(
            data.resources.footprint.read_only.len(),
            0,
            "restore targets belong in read_write, not read_only"
        );
    }

    #[test]
    fn rejects_empty_key_set() {
        assert!(restore_footprint(&[]).is_err());
    }
}
