//! XDR encoding of the controller's storage keys, mirroring the on-chain
//! `ControllerKey` enum from `common::types::controller`.
//!
//! soroban-sdk serializes `#[contracttype]` enums as `ScVal::Vec`:
//! unit variants → `[Symbol("Variant")]`, tuple variants →
//! `[Symbol("Variant"), arg1_scval, …]`. We re-create those values directly
//! against `stellar-xdr` so the keeper doesn't pull soroban-sdk into the host
//! binary. A startup self-check (see `discovery::self_check`) reads
//! `PoolsList` from the live controller and fails fast if encoding ever
//! drifts.
//!
//! Tier policy (matches `contracts/controller/src/storage/`):
//! - `Market`, `EModeCategory`, `PoolsList`, `IsolatedDebt` → Persistent.
//! - `PoolTemplate`, `Aggregator`, `Accumulator`, `AccountNonce`,
//!   `PositionLimits`, `LastEModeCategoryId`, `AppVersion` → Instance
//!   (read via `get_contract_instance` instead of `get_ledger_entries`).
//!
//! The per-user persistent keys (`AccountMeta`, `SupplyPositions`,
//! `BorrowPositions`) are deliberately not modelled here: a user's own
//! protocol interactions auto-bump those three entries, so the keeper leaves
//! them out of its keep-alive set.

use anyhow::{anyhow, Result};
use stellar_xdr::curr::{
    ContractDataDurability, ContractId, Hash, LedgerKey, LedgerKeyContractData, ScAddress,
    ScSymbol, ScVal, ScVec, StringM, VecM,
};

/// Persistent controller storage keys that the keeper keeps alive (the
/// protocol-wide and per-asset entries — never the per-user triplets).
#[derive(Debug, Clone)]
pub enum ControllerPersistentKey {
    PoolsList,
    Market([u8; 32]),
    IsolatedDebt([u8; 32]),
    EModeCategory(u32),
}

impl ControllerPersistentKey {
    pub fn to_sc_val(&self) -> Result<ScVal> {
        Ok(match self {
            Self::PoolsList => sc_enum("PoolsList", &[])?,
            Self::Market(addr) => sc_enum("Market", &[sc_address_contract(addr)?])?,
            Self::IsolatedDebt(addr) => sc_enum("IsolatedDebt", &[sc_address_contract(addr)?])?,
            Self::EModeCategory(id) => sc_enum("EModeCategory", &[ScVal::U32(*id)])?,
        })
    }

    pub fn to_ledger_key(&self, controller_id: &[u8; 32]) -> Result<LedgerKey> {
        Ok(LedgerKey::ContractData(LedgerKeyContractData {
            contract: ScAddress::Contract(ContractId(Hash(*controller_id))),
            key: self.to_sc_val()?,
            durability: ContractDataDurability::Persistent,
        }))
    }
}

/// Instance-storage symbol used to look up a value inside the controller's
/// `ScContractInstance.storage` map.
#[derive(Debug, Clone, Copy)]
pub enum ControllerInstanceKey {
    PoolTemplate,
    Aggregator,
    Accumulator,
    AccountNonce,
    PositionLimits,
    LastEModeCategoryId,
    AppVersion,
}

impl ControllerInstanceKey {
    pub fn variant_name(&self) -> &'static str {
        match self {
            Self::PoolTemplate => "PoolTemplate",
            Self::Aggregator => "Aggregator",
            Self::Accumulator => "Accumulator",
            Self::AccountNonce => "AccountNonce",
            Self::PositionLimits => "PositionLimits",
            Self::LastEModeCategoryId => "LastEModeCategoryId",
            Self::AppVersion => "AppVersion",
        }
    }
}

/// `LedgerKey::ContractInstance` for the controller (or any contract).
pub fn contract_instance_key(contract_id: &[u8; 32]) -> LedgerKey {
    LedgerKey::ContractData(LedgerKeyContractData {
        contract: ScAddress::Contract(ContractId(Hash(*contract_id))),
        key: ScVal::LedgerKeyContractInstance,
        durability: ContractDataDurability::Persistent,
    })
}

/// `LedgerKey::ContractCode` for a wasm-hash entry.
pub fn contract_code_key(wasm_hash: &[u8; 32]) -> LedgerKey {
    LedgerKey::ContractCode(stellar_xdr::curr::LedgerKeyContractCode {
        hash: Hash(*wasm_hash),
    })
}

// -- helpers --------------------------------------------------------------

fn sc_enum(variant: &str, args: &[ScVal]) -> Result<ScVal> {
    let mut elems: Vec<ScVal> = Vec::with_capacity(1 + args.len());
    elems.push(ScVal::Symbol(symbol(variant)?));
    elems.extend_from_slice(args);
    let vec_m: VecM<ScVal> = elems
        .try_into()
        .map_err(|_| anyhow!("variant {variant} too many args for ScVec"))?;
    Ok(ScVal::Vec(Some(ScVec(vec_m))))
}

fn symbol(text: &str) -> Result<ScSymbol> {
    let string_m: StringM<32> = text
        .try_into()
        .map_err(|_| anyhow!("symbol '{text}' exceeds 32 bytes"))?;
    Ok(ScSymbol(string_m))
}

fn sc_address_contract(contract: &[u8; 32]) -> Result<ScVal> {
    Ok(ScVal::Address(ScAddress::Contract(ContractId(Hash(
        *contract,
    )))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_variant_serializes_as_symbol_vec() {
        let sv = ControllerPersistentKey::PoolsList.to_sc_val().unwrap();
        match sv {
            ScVal::Vec(Some(ScVec(items))) => {
                assert_eq!(items.len(), 1);
                match &items[0] {
                    ScVal::Symbol(ScSymbol(s)) => assert_eq!(s.to_utf8_string_lossy(), "PoolsList"),
                    other => panic!("expected Symbol, got {other:?}"),
                }
            }
            other => panic!("expected ScVal::Vec, got {other:?}"),
        }
    }

    #[test]
    fn tuple_variant_carries_args_in_order() {
        let sv = ControllerPersistentKey::EModeCategory(99).to_sc_val().unwrap();
        match sv {
            ScVal::Vec(Some(ScVec(items))) => {
                assert_eq!(items.len(), 2);
                assert!(matches!(items[0], ScVal::Symbol(_)));
                assert!(matches!(items[1], ScVal::U32(99)));
            }
            _ => panic!("expected Vec"),
        }
    }
}
