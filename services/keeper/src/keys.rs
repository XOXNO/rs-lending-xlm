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
    ContractDataDurability, ContractId, Hash, LedgerKey, LedgerKeyContractData, ScAddress, ScMap,
    ScMapEntry, ScSymbol, ScVal, ScVec, StringM, VecM,
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

/// Controller access-control persistent keys, managed by the vendored
/// OpenZeppelin `stellar_access` crate (`AccessControlStorageKey`). These hold
/// the operational-role assignments (`KEEPER` / `REVENUE` / `ORACLE`) and
/// self-extend only when a role-gated call reads them — so the keeper must keep
/// them alive itself. `Admin`/owner are instance-tier and ride the
/// controller-instance bump, so they are not modelled here.
#[derive(Debug, Clone)]
pub enum AccessControlPersistentKey {
    /// `Vec<Symbol>` of every existing role name.
    ExistingRoles,
    /// `role -> u32` count of accounts holding the role.
    RoleAccountsCount(String),
    /// `(role, index) -> Address` of the account at that enumeration slot.
    RoleAccounts(String, u32),
    /// `(account, role) -> u32` enumeration index; absence means "no role".
    HasRole(ScAddress, String),
}

impl AccessControlPersistentKey {
    pub fn to_sc_val(&self) -> Result<ScVal> {
        Ok(match self {
            Self::ExistingRoles => sc_enum("ExistingRoles", &[])?,
            Self::RoleAccountsCount(role) => sc_enum("RoleAccountsCount", &[symbol_val(role)?])?,
            Self::RoleAccounts(role, index) => {
                sc_enum("RoleAccounts", &[role_account_key_map(role, *index)?])?
            }
            Self::HasRole(account, role) => {
                sc_enum("HasRole", &[ScVal::Address(account.clone()), symbol_val(role)?])?
            }
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

fn symbol_val(text: &str) -> Result<ScVal> {
    Ok(ScVal::Symbol(symbol(text)?))
}

/// Encode `RoleAccountKey { role, index }` as soroban does — a `Map` whose
/// entries are sorted by field symbol, so `index` precedes `role`.
fn role_account_key_map(role: &str, index: u32) -> Result<ScVal> {
    let entries = vec![
        ScMapEntry { key: symbol_val("index")?, val: ScVal::U32(index) },
        ScMapEntry { key: symbol_val("role")?, val: symbol_val(role)? },
    ];
    let map: VecM<ScMapEntry> = entries
        .try_into()
        .map_err(|_| anyhow!("role-account map convert"))?;
    Ok(ScVal::Map(Some(ScMap(map))))
}

fn sc_address_contract(contract: &[u8; 32]) -> Result<ScVal> {
    Ok(ScVal::Address(ScAddress::Contract(ContractId(Hash(
        *contract,
    )))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use stellar_xdr::curr::{AccountId, PublicKey, Uint256};

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

    fn sym_text(v: &ScVal) -> String {
        match v {
            ScVal::Symbol(ScSymbol(s)) => s.to_utf8_string_lossy(),
            other => panic!("expected Symbol, got {other:?}"),
        }
    }

    #[test]
    fn existing_roles_encodes_as_symbol_vec() {
        let sv = AccessControlPersistentKey::ExistingRoles.to_sc_val().unwrap();
        match sv {
            ScVal::Vec(Some(ScVec(items))) => {
                assert_eq!(items.len(), 1);
                assert_eq!(sym_text(&items[0]), "ExistingRoles");
            }
            other => panic!("expected Vec, got {other:?}"),
        }
    }

    #[test]
    fn role_accounts_count_carries_role_symbol() {
        let sv = AccessControlPersistentKey::RoleAccountsCount("KEEPER".into())
            .to_sc_val()
            .unwrap();
        match sv {
            ScVal::Vec(Some(ScVec(items))) => {
                assert_eq!(items.len(), 2);
                assert_eq!(sym_text(&items[0]), "RoleAccountsCount");
                assert_eq!(sym_text(&items[1]), "KEEPER");
            }
            other => panic!("expected Vec, got {other:?}"),
        }
    }

    #[test]
    fn role_accounts_struct_arg_is_map_sorted_index_then_role() {
        let sv = AccessControlPersistentKey::RoleAccounts("ORACLE".into(), 0)
            .to_sc_val()
            .unwrap();
        let ScVal::Vec(Some(ScVec(items))) = sv else {
            panic!("expected Vec");
        };
        assert_eq!(items.len(), 2);
        assert_eq!(sym_text(&items[0]), "RoleAccounts");
        // soroban serializes #[contracttype] struct fields as a Map sorted by
        // field symbol: "index" precedes "role".
        let ScVal::Map(Some(map)) = &items[1] else {
            panic!("expected Map arg, got {:?}", items[1]);
        };
        assert_eq!(map.0.len(), 2);
        assert_eq!(sym_text(&map.0[0].key), "index");
        assert!(matches!(map.0[0].val, ScVal::U32(0)));
        assert_eq!(sym_text(&map.0[1].key), "role");
        assert_eq!(sym_text(&map.0[1].val), "ORACLE");
    }

    #[test]
    fn has_role_carries_address_then_role() {
        let addr = ScAddress::Account(AccountId(PublicKey::PublicKeyTypeEd25519(Uint256(
            [7u8; 32],
        ))));
        let sv = AccessControlPersistentKey::HasRole(addr, "REVENUE".into())
            .to_sc_val()
            .unwrap();
        let ScVal::Vec(Some(ScVec(items))) = sv else {
            panic!("expected Vec");
        };
        assert_eq!(items.len(), 3);
        assert_eq!(sym_text(&items[0]), "HasRole");
        assert!(matches!(items[1], ScVal::Address(ScAddress::Account(_))));
        assert_eq!(sym_text(&items[2]), "REVENUE");
    }

    #[test]
    fn access_control_key_is_persistent_contract_data() {
        let key = AccessControlPersistentKey::ExistingRoles
            .to_ledger_key(&[3u8; 32])
            .unwrap();
        match key {
            LedgerKey::ContractData(cd) => {
                assert!(matches!(cd.durability, ContractDataDurability::Persistent));
                assert!(matches!(cd.contract, ScAddress::Contract(_)));
            }
            other => panic!("expected ContractData, got {other:?}"),
        }
    }
}
