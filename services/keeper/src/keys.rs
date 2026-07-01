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
    AssetOracle([u8; 32]),
    Spoke(u32),
}

impl ControllerPersistentKey {
    pub fn to_sc_val(&self) -> Result<ScVal> {
        Ok(match self {
            Self::AssetOracle(addr) => sc_enum("AssetOracle", &[sc_address_contract(addr)])?,
            Self::Spoke(id) => sc_enum("Spoke", &[ScVal::U32(*id)])?,
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

/// Controller per-user persistent keys, keyed by the `u64` account id.
///
/// The contract splits each account across three persistent entries:
/// `AccountMeta(id)`, `SupplyPositions(id)`, and `BorrowPositions(id)`.
#[derive(Debug, Clone)]
pub enum ControllerUserKey {
    AccountMeta(u64),
    SupplyPositions(u64),
    BorrowPositions(u64),
}

impl ControllerUserKey {
    pub fn to_sc_val(&self) -> Result<ScVal> {
        Ok(match self {
            Self::AccountMeta(id) => sc_enum("AccountMeta", &[ScVal::U64(*id)])?,
            Self::SupplyPositions(id) => sc_enum("SupplyPositions", &[ScVal::U64(*id)])?,
            Self::BorrowPositions(id) => sc_enum("BorrowPositions", &[ScVal::U64(*id)])?,
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

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct HubAssetKey {
    pub hub_id: u32,
    pub asset: [u8; 32],
}

/// Hub-asset-keyed persistent keys for the central liquidity pool.
#[derive(Debug, Clone)]
pub enum PoolPersistentKey {
    Params(HubAssetKey),
    State(HubAssetKey),
}

impl PoolPersistentKey {
    pub fn to_sc_val(&self) -> Result<ScVal> {
        Ok(match self {
            Self::Params(hub_asset) => sc_enum("Params", &[hub_asset_key_sc_val(hub_asset)?])?,
            Self::State(hub_asset) => sc_enum("State", &[hub_asset_key_sc_val(hub_asset)?])?,
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

// Governance storage needs no enum here: the contract keeps almost everything
// in INSTANCE storage, so a single instance bump covers `GovernanceKey::
// Controller`, ownable `Owner`, access_control `Admin` + per-role `RoleAdmin`,
// and the timelock `MinDelay`.
//
// `MinDelay` is INSTANCE, not Persistent: `stellar_governance::timelock`
// reads/writes it via `e.storage().instance()` (verified against
// stellar-governance 0.7.2 `src/timelock/storage.rs`: `get_min_delay` /
// `set_min_delay` both use `instance()`). A *persistent* `MinDelay` key would
// silently resolve to nothing, so the keeper relies on the instance bump.
//
// The access_control role-holder keys (`ExistingRoles`, `RoleAccountsCount`,
// `RoleAccounts`, `HasRole`) ARE persistent and are discovered by reusing
// `discover_role_keys` against the governance contract id — the same encoding
// as the controller's role keys (`AccessControlPersistentKey`).
//
// The timelock per-operation key `OperationLedger(BytesN<32>)` is persistent
// but NOT enumerable on-chain (the op id is a keccak256 hash known only from
// the schedule event). It is transient — resolved within `min_delay` ledgers,
// far inside any TTL window — so it is documented and intentionally skipped.

/// Controller instance-storage keys read from `ScContractInstance.storage`.
#[derive(Debug, Clone, Copy)]
pub enum ControllerInstanceKey {
    Pool,
    AccountNonce,
    LastSpokeId,
}

impl ControllerInstanceKey {
    pub fn variant_name(&self) -> &'static str {
        match self {
            Self::Pool => "Pool",
            Self::AccountNonce => "AccountNonce",
            Self::LastSpokeId => "LastSpokeId",
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
pub fn hub_asset_key_sc_val(hub_asset: &HubAssetKey) -> Result<ScVal> {
    let entries = vec![
        ScMapEntry {
            key: symbol_val("asset")?,
            val: sc_address_contract(&hub_asset.asset),
        },
        ScMapEntry {
            key: symbol_val("hub_id")?,
            val: ScVal::U32(hub_asset.hub_id),
        },
    ];
    let map: VecM<ScMapEntry> = entries
        .try_into()
        .map_err(|_| anyhow!("hub asset map convert"))?;
    Ok(ScVal::Map(Some(ScMap(map))))
}

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
    fn tuple_variant_carries_args_in_order() {
        let sv = ControllerPersistentKey::Spoke(99).to_sc_val().unwrap();
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
    fn pool_params_key_carries_hub_asset_map() {
        let hub_asset = HubAssetKey {
            hub_id: 7,
            asset: [9u8; 32],
        };
        let sv = PoolPersistentKey::Params(hub_asset).to_sc_val().unwrap();
        let ScVal::Vec(Some(ScVec(items))) = sv else {
            panic!("expected Vec");
        };
        assert_eq!(items.len(), 2);
        assert_eq!(sym_text(&items[0]), "Params");
        assert_hub_asset(&items[1], hub_asset);
    }

    #[test]
    fn pool_state_key_is_persistent_data_on_pool_contract() {
        let key = PoolPersistentKey::State(HubAssetKey {
            hub_id: 3,
            asset: [4u8; 32],
        })
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

    fn assert_hub_asset(v: &ScVal, expected: HubAssetKey) {
        let ScVal::Map(Some(ScMap(entries))) = v else {
            panic!("expected HubAssetKey map");
        };
        assert_eq!(entries.len(), 2);
        assert_eq!(sym_text(&entries[0].key), "asset");
        assert!(matches!(
            &entries[0].val,
            ScVal::Address(ScAddress::Contract(ContractId(Hash(b)))) if *b == expected.asset
        ));
        assert_eq!(sym_text(&entries[1].key), "hub_id");
        assert!(matches!(entries[1].val, ScVal::U32(id) if id == expected.hub_id));
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

    #[test]
    fn account_meta_user_key_carries_u64_id() {
        let sv = ControllerUserKey::AccountMeta(7).to_sc_val().unwrap();
        let ScVal::Vec(Some(ScVec(items))) = sv else {
            panic!("expected Vec");
        };
        assert_eq!(items.len(), 2);
        assert_eq!(sym_text(&items[0]), "AccountMeta");
        assert!(matches!(items[1], ScVal::U64(7)));
    }

    #[test]
    fn supply_positions_user_key_carries_u64_id() {
        let sv = ControllerUserKey::SupplyPositions(42).to_sc_val().unwrap();
        let ScVal::Vec(Some(ScVec(items))) = sv else {
            panic!("expected Vec");
        };
        assert_eq!(items.len(), 2);
        assert_eq!(sym_text(&items[0]), "SupplyPositions");
        assert!(matches!(items[1], ScVal::U64(42)));
    }

    #[test]
    fn borrow_positions_user_key_carries_u64_id() {
        let sv = ControllerUserKey::BorrowPositions(1).to_sc_val().unwrap();
        let ScVal::Vec(Some(ScVec(items))) = sv else {
            panic!("expected Vec");
        };
        assert_eq!(items.len(), 2);
        assert_eq!(sym_text(&items[0]), "BorrowPositions");
        assert!(matches!(items[1], ScVal::U64(1)));
    }

    #[test]
    fn user_key_is_persistent_contract_data_on_controller() {
        let key = ControllerUserKey::AccountMeta(3)
            .to_ledger_key(&[6u8; 32])
            .unwrap();
        let LedgerKey::ContractData(cd) = key else {
            panic!("expected ContractData");
        };
        assert!(matches!(cd.durability, ContractDataDurability::Persistent));
        assert!(matches!(
            cd.contract,
            ScAddress::Contract(ContractId(Hash(b))) if b == [6u8; 32]
        ));
    }
}
