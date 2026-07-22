//! `AssetOracleConfig` + provider price payloads for staleness/deviation metrics.

use anyhow::{anyhow, Result};
use stellar_xdr::curr::{ScString, ScVal, StringM};

use crate::scval::{
    address_strkey, enum_variant, field_i128, field_u32, field_u64, map_field, string_text,
    symbol_text, vec_items,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OracleKind {
    Reflector,
    RedStone,
    Xoxno,
}

/// Resolved source for polling a price timestamp.
#[derive(Debug, Clone)]
pub struct OracleSource {
    pub kind: OracleKind,
    pub contract: String,
    /// Reflector: raw `OracleAssetRef` for `lastprice`.
    pub asset_ref: Option<ScVal>,
    /// RedStone/Xoxno: feed id for `read_price_data_for_feed`.
    pub feed_id: Option<String>,
    pub max_stale_seconds: u64,
}

#[derive(Debug, Clone)]
pub struct OracleConfig {
    pub max_price_stale_seconds: u64,
    pub tolerance_upper_bps: u32,
    pub tolerance_lower_bps: u32,
    /// 0 = single source, 1 = primary+anchor.
    pub strategy: u32,
    pub min_sanity_price_wad: i128,
    pub max_sanity_price_wad: i128,
    pub primary: OracleSource,
    pub anchor: Option<OracleSource>,
}

#[derive(Debug, Clone, Copy)]
pub struct PriceObservation {
    pub feed_ts_secs: u64,
}

/// Decode `AssetOracleConfig` from the `AssetOracle` ledger entry.
pub fn decode_oracle_config(value: &ScVal) -> Result<OracleConfig> {
    let max_price_stale_seconds =
        field_u64(value, "max_price_stale_seconds").ok_or_else(|| anyhow!("max_price_stale_seconds missing"))?;

    let tolerance = map_field(value, "tolerance").ok_or_else(|| anyhow!("tolerance missing"))?;
    let tolerance_upper_bps = field_u32(tolerance, "upper_ratio_bps").unwrap_or(0);
    let tolerance_lower_bps = field_u32(tolerance, "lower_ratio_bps").unwrap_or(0);

    let strategy = decode_strategy(map_field(value, "strategy"));
    let min_sanity_price_wad = field_i128(value, "min_sanity_price_wad").unwrap_or(0);
    let max_sanity_price_wad = field_i128(value, "max_sanity_price_wad").unwrap_or(0);

    let primary_val = map_field(value, "primary").ok_or_else(|| anyhow!("primary missing"))?;
    let primary = decode_source(primary_val, max_price_stale_seconds)?;

    let anchor = match map_field(value, "anchor").and_then(enum_variant) {
        Some((tag, payload)) if tag == "Some" => payload
            .first()
            .map(|inner| decode_source(inner, max_price_stale_seconds))
            .transpose()?,
        _ => None,
    };

    Ok(OracleConfig {
        max_price_stale_seconds,
        tolerance_upper_bps,
        tolerance_lower_bps,
        strategy,
        min_sanity_price_wad,
        max_sanity_price_wad,
        primary,
        anchor,
    })
}

fn decode_strategy(value: Option<&ScVal>) -> u32 {
    // Prefer U32; tolerate symbol-tagged encoding.
    match value {
        Some(v) => crate::scval::as_u32(v).unwrap_or_else(|| match symbol_text(v).as_deref() {
            Some("PrimaryWithAnchor") => 1,
            _ => 0,
        }),
        None => 0,
    }
}

fn decode_source(value: &ScVal, market_default_max_stale: u64) -> Result<OracleSource> {
    let (tag, payload) = enum_variant(value).ok_or_else(|| anyhow!("oracle source not enum-tagged"))?;
    let inner = payload.first().ok_or_else(|| anyhow!("oracle source has no payload"))?;
    let contract = map_field(inner, "contract")
        .and_then(address_strkey)
        .ok_or_else(|| anyhow!("oracle source contract missing"))?;

    match tag.as_str() {
        "Reflector" => Ok(OracleSource {
            kind: OracleKind::Reflector,
            contract,
            asset_ref: map_field(inner, "asset").cloned(),
            feed_id: None,
            max_stale_seconds: market_default_max_stale,
        }),
        "RedStone" | "Xoxno" => Ok(OracleSource {
            kind: if tag == "Xoxno" { OracleKind::Xoxno } else { OracleKind::RedStone },
            contract,
            asset_ref: None,
            feed_id: map_field(inner, "feed_id").and_then(string_text),
            max_stale_seconds: field_u64(inner, "max_stale_seconds").unwrap_or(market_default_max_stale),
        }),
        other => Err(anyhow!("unknown oracle source variant {other}")),
    }
}

/// `OracleAssetRef` → Reflector `lastprice` arg. `Stellar` wire-identical;
/// `Symbol` → `Other`; `String` unsupported.
pub fn oracle_asset_ref_to_reflector_arg(asset_ref: &ScVal) -> Result<ScVal> {
    let (tag, payload) = enum_variant(asset_ref).ok_or_else(|| anyhow!("asset ref not enum-tagged"))?;
    match tag.as_str() {
        "Stellar" => Ok(asset_ref.clone()),
        "Symbol" => {
            let sym = payload.first().cloned().ok_or_else(|| anyhow!("Symbol asset ref empty"))?;
            Ok(retag_enum("Other", sym)?)
        }
        other => Err(anyhow!("unsupported oracle asset ref variant {other}")),
    }
}

fn retag_enum(variant: &str, payload: ScVal) -> Result<ScVal> {
    use stellar_xdr::curr::{ScVec, VecM};
    let items: VecM<ScVal> = vec![ScVal::Symbol(crate::keys::symbol(variant)?), payload]
        .try_into()
        .map_err(|_| anyhow!("retag vec"))?;
    Ok(ScVal::Vec(Some(ScVec(items))))
}

pub fn feed_id_arg(feed_id: &str) -> Result<ScVal> {
    let s: StringM = feed_id.try_into().map_err(|_| anyhow!("feed id too long"))?;
    Ok(ScVal::String(ScString(s)))
}

/// Reflector `lastprice` → `Option`; Void is no observation.
pub fn decode_reflector_price(value: &ScVal) -> Result<Option<PriceObservation>> {
    if matches!(value, ScVal::Void) {
        return Ok(None);
    }
    let ts = field_u64(value, "timestamp").ok_or_else(|| anyhow!("ReflectorPriceData.timestamp missing"))?;
    Ok(Some(PriceObservation { feed_ts_secs: ts }))
}

/// RedStone/Xoxno price payload; freshness = `min(package_ms, write_ms) / 1000`.
pub fn decode_redstone_price(value: &ScVal) -> Result<PriceObservation> {
    let package_ms = field_u64(value, "package_timestamp")
        .ok_or_else(|| anyhow!("RedStonePriceData.package_timestamp missing"))?;
    let write_ms = field_u64(value, "write_timestamp")
        .ok_or_else(|| anyhow!("RedStonePriceData.write_timestamp missing"))?;
    Ok(PriceObservation {
        feed_ts_secs: package_ms.min(write_ms) / 1000,
    })
}

pub fn is_nonempty_vec(value: &ScVal) -> bool {
    vec_items(value).map(|v| !v.is_empty()).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use stellar_xdr::curr::{Int128Parts, ScMap, ScMapEntry, ScVec, VecM};

    fn sym(t: &str) -> ScVal {
        ScVal::Symbol(crate::keys::symbol(t).unwrap())
    }
    fn i128v(v: i128) -> ScVal {
        ScVal::I128(Int128Parts { hi: (v >> 64) as i64, lo: v as u64 })
    }
    fn map(entries: Vec<(&str, ScVal)>) -> ScVal {
        ScVal::Map(Some(ScMap(
            entries
                .into_iter()
                .map(|(k, v)| ScMapEntry { key: sym(k), val: v })
                .collect::<Vec<_>>()
                .try_into()
                .unwrap(),
        )))
    }
    fn enum_val(tag: &str, payload: Vec<ScVal>) -> ScVal {
        let mut items = vec![sym(tag)];
        items.extend(payload);
        let v: VecM<ScVal> = items.try_into().unwrap();
        ScVal::Vec(Some(ScVec(v)))
    }
    fn addr(byte: u8) -> ScVal {
        crate::keys::sc_address_contract(&[byte; 32])
    }

    #[test]
    fn decodes_redstone_min_of_timestamps_to_seconds() {
        let m = map(vec![
            ("package_timestamp", ScVal::U64(1_700_000_500_000)),
            ("write_timestamp", ScVal::U64(1_700_000_000_000)),
        ]);
        let obs = decode_redstone_price(&m).unwrap();
        assert_eq!(obs.feed_ts_secs, 1_700_000_000);
    }

    #[test]
    fn reflector_none_is_ok_none() {
        assert!(decode_reflector_price(&ScVal::Void).unwrap().is_none());
        let m = map(vec![("price", i128v(1)), ("timestamp", ScVal::U64(42))]);
        assert_eq!(decode_reflector_price(&m).unwrap().unwrap().feed_ts_secs, 42);
    }

    #[test]
    fn decodes_market_oracle_config_reflector_primary() {
        let reflector = enum_val(
            "Reflector",
            vec![map(vec![
                ("contract", addr(9)),
                ("asset", enum_val("Stellar", vec![addr(9)])),
                ("read_mode", enum_val("Spot", vec![])),
                ("decimals", ScVal::U32(14)),
                ("resolution_seconds", ScVal::U32(300)),
                ("base", enum_val("Usd", vec![])),
            ])],
        );
        let cfg = decode_oracle_config(&map(vec![
            ("asset_decimals", ScVal::U32(7)),
            ("max_price_stale_seconds", ScVal::U64(900)),
            (
                "tolerance",
                map(vec![("upper_ratio_bps", ScVal::U32(200)), ("lower_ratio_bps", ScVal::U32(196))]),
            ),
            ("strategy", ScVal::U32(1)),
            ("primary", reflector),
            ("anchor", enum_val("None", vec![])),
            ("min_sanity_price_wad", i128v(0)),
            ("max_sanity_price_wad", i128v(0)),
        ]))
        .unwrap();

        assert_eq!(cfg.max_price_stale_seconds, 900);
        assert_eq!(cfg.tolerance_upper_bps, 200);
        assert_eq!(cfg.strategy, 1);
        assert_eq!(cfg.primary.kind, OracleKind::Reflector);
        assert_eq!(cfg.primary.max_stale_seconds, 900);
        assert!(cfg.primary.asset_ref.is_some());
        assert!(cfg.anchor.is_none());
    }

    #[test]
    fn redstone_source_uses_own_max_stale() {
        let redstone = enum_val(
            "RedStone",
            vec![map(vec![
                ("contract", addr(4)),
                ("feed_id", ScVal::String(ScString("BTC/USD".try_into().unwrap()))),
                ("decimals", ScVal::U32(8)),
                ("max_stale_seconds", ScVal::U64(600)),
            ])],
        );
        let src = decode_source(&redstone, 900).unwrap();
        assert_eq!(src.kind, OracleKind::RedStone);
        assert_eq!(src.max_stale_seconds, 600);
        assert_eq!(src.feed_id.as_deref(), Some("BTC/USD"));
    }

    #[test]
    fn reflector_asset_ref_symbol_retags_to_other() {
        let symref = enum_val("Symbol", vec![sym("XLM")]);
        let arg = oracle_asset_ref_to_reflector_arg(&symref).unwrap();
        let (tag, _) = enum_variant(&arg).unwrap();
        assert_eq!(tag, "Other");
    }
}
