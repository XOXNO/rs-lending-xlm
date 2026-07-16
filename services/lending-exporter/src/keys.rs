//! XDR encoding for protocol view arguments and storage keys.
//!
//! `#[contracttype]` struct values serialize as an `ScMap` whose entries are
//! sorted by field-name symbol; enum keys serialize as `Vec[Symbol("Variant"),
//! args...]`. The exporter builds these directly instead of depending on
//! `soroban-sdk` in a std binary. Mirrors `services/keeper/src/keys.rs`.

use anyhow::{anyhow, Result};
use stellar_xdr::curr::{
    ContractDataDurability, ContractId, Hash, LedgerKey, LedgerKeyContractData, ScAddress, ScMap,
    ScMapEntry, ScSymbol, ScVal, ScVec, StringM, VecM,
};

/// A hub-scoped asset coordinate, using the raw 32-byte asset id.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct HubAssetKey {
    pub hub_id: u32,
    pub asset: [u8; 32],
}

/// Encodes `HubAssetKey { hub_id, asset }` as an `ScMap` with fields sorted by
/// symbol (`asset` before `hub_id`) — the soroban `#[contracttype]` layout.
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

/// `ScVal::Vec` of hub-asset keys, the argument shape for the controller's
/// bulk `get_market_indexes_detailed(Vec<HubAssetKey>)` view.
pub fn hub_asset_vec_sc_val(keys: &[HubAssetKey]) -> Result<ScVal> {
    let items: Vec<ScVal> = keys
        .iter()
        .map(hub_asset_key_sc_val)
        .collect::<Result<Vec<_>>>()?;
    let vec_m: VecM<ScVal> = items
        .try_into()
        .map_err(|_| anyhow!("hub asset vec exceeds ScVec capacity"))?;
    Ok(ScVal::Vec(Some(ScVec(vec_m))))
}

/// Persistent `ControllerKey::AssetOracle(asset)` ledger key holding the
/// resolved `MarketOracleConfig` for one asset.
pub fn asset_oracle_ledger_key(controller_id: &[u8; 32], asset_id: &[u8; 32]) -> Result<LedgerKey> {
    let key = sc_enum("AssetOracle", &[sc_address_contract(asset_id)])?;
    Ok(contract_data_key(
        controller_id,
        key,
        ContractDataDurability::Persistent,
    ))
}

/// Decode a contract strkey (`C...`) into the raw 32-byte contract id.
pub fn contract_id_from_strkey(c_strkey: &str) -> Result<[u8; 32]> {
    let c = stellar_strkey::Contract::from_string(c_strkey.trim())
        .map_err(|e| anyhow!("invalid C... contract id {c_strkey}: {e}"))?;
    Ok(c.0)
}

/// Render a raw 32-byte contract id back to its `C...` strkey.
pub fn contract_strkey(contract_id: &[u8; 32]) -> String {
    // `Display`, not the inherent `to_string` (which returns `heapless::String`).
    format!("{}", stellar_strkey::Contract(*contract_id))
}

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

pub(crate) fn symbol(text: &str) -> Result<ScSymbol> {
    let string_m: StringM<32> = text
        .try_into()
        .map_err(|_| anyhow!("symbol '{text}' exceeds 32 bytes"))?;
    Ok(ScSymbol(string_m))
}

fn symbol_val(text: &str) -> Result<ScVal> {
    Ok(ScVal::Symbol(symbol(text)?))
}

pub(crate) fn sc_address_contract(contract: &[u8; 32]) -> ScVal {
    ScVal::Address(ScAddress::Contract(ContractId(Hash(*contract))))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sym_text(v: &ScVal) -> String {
        match v {
            ScVal::Symbol(ScSymbol(s)) => s.to_utf8_string_lossy(),
            other => panic!("expected Symbol, got {other:?}"),
        }
    }

    #[test]
    fn hub_asset_key_map_is_sorted_asset_then_hub_id() {
        let key = HubAssetKey {
            hub_id: 7,
            asset: [9u8; 32],
        };
        let ScVal::Map(Some(ScMap(entries))) = hub_asset_key_sc_val(&key).unwrap() else {
            panic!("expected Map");
        };
        assert_eq!(entries.len(), 2);
        assert_eq!(sym_text(&entries[0].key), "asset");
        assert_eq!(sym_text(&entries[1].key), "hub_id");
        assert!(matches!(entries[1].val, ScVal::U32(7)));
    }

    #[test]
    fn asset_oracle_key_is_persistent_vec_tagged() {
        let key = asset_oracle_ledger_key(&[8u8; 32], &[3u8; 32]).unwrap();
        let LedgerKey::ContractData(cd) = key else {
            panic!("expected ContractData");
        };
        assert!(matches!(cd.durability, ContractDataDurability::Persistent));
        let ScVal::Vec(Some(ScVec(items))) = cd.key else {
            panic!("expected Vec key");
        };
        assert_eq!(sym_text(&items[0]), "AssetOracle");
        assert!(matches!(
            items[1],
            ScVal::Address(ScAddress::Contract(ContractId(Hash(b)))) if b == [3u8; 32]
        ));
    }

    #[test]
    fn contract_strkey_roundtrips() {
        let id = [5u8; 32];
        let s = contract_strkey(&id);
        assert_eq!(contract_id_from_strkey(&s).unwrap(), id);
    }
}
