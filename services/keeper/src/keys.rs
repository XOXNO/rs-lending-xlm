//! XDR encoding for protocol storage keys used by the keeper.
//!
//! `#[contracttype]` enum keys serialize as `Vec[Symbol("Variant"), args...]`.
//! The keeper builds those XDR values directly instead of depending on
//! `soroban-sdk` in the host binary.

use anyhow::{anyhow, Result};
use stellar_xdr::curr::{
    ContractDataDurability, ContractId, Hash, LedgerKey, LedgerKeyContractData, ScAddress, ScMap,
    ScMapEntry, ScSymbol, ScVal, ScVec, StringM, VecM,
};

/// Protocol-wide controller persistent keys kept alive by the keeper.
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
            Self::Market(addr) => sc_enum("Market", &[sc_address_contract(addr)])?,
            Self::IsolatedDebt(addr) => sc_enum("IsolatedDebt", &[sc_address_contract(addr)])?,
            Self::EModeCategory(id) => sc_enum("EModeCategory", &[ScVal::U32(*id)])?,
        })
    }

    pub fn to_ledger_key(&self, controller_id: &[u8; 32]) -> Result<LedgerKey> {
        Ok(contract_data_key(
            controller_id,
            self.to_sc_val()?,
            ContractDataDurability::Persistent,
        ))
    }
}

/// Persistent role keys managed by `stellar_access`.
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
    /// `role -> admin_role`.
    RoleAdmin(String),
}

impl AccessControlPersistentKey {
    pub fn to_sc_val(&self) -> Result<ScVal> {
        Ok(match self {
            Self::ExistingRoles => sc_enum("ExistingRoles", &[])?,
            Self::RoleAccountsCount(role) => sc_enum("RoleAccountsCount", &[symbol_val(role)?])?,
            Self::RoleAccounts(role, index) => {
                sc_enum("RoleAccounts", &[role_account_key_map(role, *index)?])?
            }
            Self::HasRole(account, role) => sc_enum(
                "HasRole",
                &[ScVal::Address(account.clone()), symbol_val(role)?],
            )?,
            Self::RoleAdmin(role) => sc_enum("RoleAdmin", &[symbol_val(role)?])?,
        })
    }

    pub fn to_ledger_key(&self, controller_id: &[u8; 32]) -> Result<LedgerKey> {
        Ok(contract_data_key(
            controller_id,
            self.to_sc_val()?,
            ContractDataDurability::Persistent,
        ))
    }
}

/// Asset-keyed persistent keys of the central liquidity pool.
#[derive(Debug, Clone)]
pub enum PoolPersistentKey {
    Params([u8; 32]),
    State([u8; 32]),
}

impl PoolPersistentKey {
    pub fn to_sc_val(&self) -> Result<ScVal> {
        Ok(match self {
            Self::Params(asset) => sc_enum("Params", &[sc_address_contract(asset)])?,
            Self::State(asset) => sc_enum("State", &[sc_address_contract(asset)])?,
        })
    }

    pub fn to_ledger_key(&self, pool_id: &[u8; 32]) -> Result<LedgerKey> {
        Ok(contract_data_key(
            pool_id,
            self.to_sc_val()?,
            ContractDataDurability::Persistent,
        ))
    }
}

/// Controller instance-storage keys read from `ScContractInstance.storage`.
#[derive(Debug, Clone, Copy)]
pub enum ControllerInstanceKey {
    Pool,
    AccountNonce,
    LastEModeCategoryId,
}

impl ControllerInstanceKey {
    pub fn variant_name(&self) -> &'static str {
        match self {
            Self::Pool => "Pool",
            Self::AccountNonce => "AccountNonce",
            Self::LastEModeCategoryId => "LastEModeCategoryId",
        }
    }
}

/// Contract instance ledger key.
pub fn contract_instance_key(contract_id: &[u8; 32]) -> LedgerKey {
    contract_data_key(
        contract_id,
        ScVal::LedgerKeyContractInstance,
        ContractDataDurability::Persistent,
    )
}

/// Contract code ledger key.
pub fn contract_code_key(wasm_hash: &[u8; 32]) -> LedgerKey {
    LedgerKey::ContractCode(stellar_xdr::curr::LedgerKeyContractCode {
        hash: Hash(*wasm_hash),
    })
}

// -- helpers --------------------------------------------------------------

fn contract_data_key(
    contract_id: &[u8; 32],
    key: ScVal,
    durability: ContractDataDurability,
) -> LedgerKey {
    LedgerKey::ContractData(LedgerKeyContractData {
        contract: ScAddress::Contract(ContractId(Hash(*contract_id))),
        key,
        durability,
    })
}

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

/// Encode `RoleAccountKey { role, index }` with fields sorted by symbol.
fn role_account_key_map(role: &str, index: u32) -> Result<ScVal> {
    let entries = vec![
        ScMapEntry {
            key: symbol_val("index")?,
            val: ScVal::U32(index),
        },
        ScMapEntry {
            key: symbol_val("role")?,
            val: symbol_val(role)?,
        },
    ];
    let map: VecM<ScMapEntry> = entries
        .try_into()
        .map_err(|_| anyhow!("role-account map convert"))?;
    Ok(ScVal::Map(Some(ScMap(map))))
}

fn sc_address_contract(contract: &[u8; 32]) -> ScVal {
    ScVal::Address(ScAddress::Contract(ContractId(Hash(*contract))))
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
        let sv = ControllerPersistentKey::EModeCategory(99)
            .to_sc_val()
            .unwrap();
        match sv {
            ScVal::Vec(Some(ScVec(items))) => {
                assert_eq!(items.len(), 2);
                assert!(matches!(items[0], ScVal::Symbol(_)));
                assert!(matches!(items[1], ScVal::U32(99)));
            }
            _ => panic!("expected Vec"),
        }
    }

    #[test]
    fn pool_params_key_carries_asset_contract_address() {
        let sv = PoolPersistentKey::Params([9u8; 32]).to_sc_val().unwrap();
        let ScVal::Vec(Some(ScVec(items))) = sv else {
            panic!("expected Vec");
        };
        assert_eq!(items.len(), 2);
        assert_eq!(sym_text(&items[0]), "Params");
        assert!(matches!(
            &items[1],
            ScVal::Address(ScAddress::Contract(ContractId(Hash(b)))) if *b == [9u8; 32]
        ));
    }

    #[test]
    fn pool_state_key_is_persistent_data_on_pool_contract() {
        let key = PoolPersistentKey::State([4u8; 32])
            .to_ledger_key(&[8u8; 32])
            .unwrap();
        let LedgerKey::ContractData(cd) = key else {
            panic!("expected ContractData");
        };
        assert!(matches!(cd.durability, ContractDataDurability::Persistent));
        assert!(matches!(
            cd.contract,
            ScAddress::Contract(ContractId(Hash(b))) if b == [8u8; 32]
        ));
        let ScVal::Vec(Some(ScVec(items))) = cd.key else {
            panic!("expected Vec key");
        };
        assert_eq!(sym_text(&items[0]), "State");
    }

    fn sym_text(v: &ScVal) -> String {
        match v {
            ScVal::Symbol(ScSymbol(s)) => s.to_utf8_string_lossy(),
            other => panic!("expected Symbol, got {other:?}"),
        }
    }

    #[test]
    fn existing_roles_encodes_as_symbol_vec() {
        let sv = AccessControlPersistentKey::ExistingRoles
            .to_sc_val()
            .unwrap();
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
    fn role_admin_carries_role_symbol() {
        let sv = AccessControlPersistentKey::RoleAdmin("KEEPER".into())
            .to_sc_val()
            .unwrap();
        let ScVal::Vec(Some(ScVec(items))) = sv else {
            panic!("expected Vec");
        };
        assert_eq!(items.len(), 2);
        assert_eq!(sym_text(&items[0]), "RoleAdmin");
        assert_eq!(sym_text(&items[1]), "KEEPER");
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
