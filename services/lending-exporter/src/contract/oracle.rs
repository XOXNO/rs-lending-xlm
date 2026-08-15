use anyhow::{anyhow, Result};
use stellar_xdr::curr::{ScString, ScVal, StringM};

use crate::scval::{
    address_strkey, enum_variant, field_i128, field_u32, field_u64, map_field, string_text,
    vec_items,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OracleKind {
    Reflector,
    RedStone,
    Xoxno,
}

#[derive(Debug, Clone)]
pub struct OracleSource {
    pub kind: OracleKind,
    pub contract: String,

    pub asset_ref: Option<ScVal>,

    pub feed_id: Option<String>,
    pub max_stale_seconds: u64,
}

#[derive(Debug, Clone)]
pub struct OracleConfig {
    pub max_price_stale_seconds: u64,
    pub tolerance_upper_bps: u32,
    pub tolerance_lower_bps: u32,

    pub source_count: u32,
    pub min_sanity_price_wad: i128,
    pub max_sanity_price_wad: i128,

    pub sources: Vec<OracleSource>,
}

#[derive(Debug, Clone, Copy)]
pub struct PriceObservation {
    pub feed_ts_secs: u64,
}

pub fn decode_oracle_config(value: &ScVal) -> Result<OracleConfig> {
    let max_price_stale_seconds = field_u64(value, "max_price_stale_seconds")
        .ok_or_else(|| anyhow!("max_price_stale_seconds missing"))?;

    let tolerance = map_field(value, "tolerance").ok_or_else(|| anyhow!("tolerance missing"))?;
    let tolerance_upper_bps = field_u32(tolerance, "upper_ratio_bps").unwrap_or(0);
    let tolerance_lower_bps = field_u32(tolerance, "lower_ratio_bps").unwrap_or(0);

    let min_sanity_price_wad = field_i128(value, "min_sanity_price_wad").unwrap_or(0);
    let max_sanity_price_wad = field_i128(value, "max_sanity_price_wad").unwrap_or(0);

    let raw_sources =
        vec_items(map_field(value, "sources").ok_or_else(|| anyhow!("sources missing"))?)
            .ok_or_else(|| anyhow!("sources not a Vec"))?;
    let source_count = raw_sources.len() as u32;

    let mut sources = Vec::with_capacity(raw_sources.len());
    for raw in raw_sources {
        if let Some(source) = decode_price_source(raw, max_price_stale_seconds)? {
            sources.push(source);
        }
    }

    Ok(OracleConfig {
        max_price_stale_seconds,
        tolerance_upper_bps,
        tolerance_lower_bps,
        source_count,
        min_sanity_price_wad,
        max_sanity_price_wad,
        sources,
    })
}

fn decode_price_source(
    value: &ScVal,
    market_default_max_stale: u64,
) -> Result<Option<OracleSource>> {
    let (tag, payload) =
        enum_variant(value).ok_or_else(|| anyhow!("price source not enum-tagged"))?;
    let inner = payload
        .first()
        .ok_or_else(|| anyhow!("price source has no payload"))?;

    match tag.as_str() {
        "Feed" => decode_feed_source(inner, market_default_max_stale).map(Some),
        "Scaled" => {
            let factor = map_field(inner, "factor")
                .ok_or_else(|| anyhow!("scaled source missing factor"))?;
            decode_feed_source(factor, market_default_max_stale).map(Some)
        }
        "AquariusLp" => Ok(None),
        other => Err(anyhow!("unknown price source variant {other}")),
    }
}

fn decode_feed_source(value: &ScVal, market_default_max_stale: u64) -> Result<OracleSource> {
    let provider =
        map_field(value, "provider").ok_or_else(|| anyhow!("feed source missing provider"))?;
    let max_stale_seconds =
        field_u64(value, "max_stale_seconds").unwrap_or(market_default_max_stale);

    let (tag, payload) =
        enum_variant(provider).ok_or_else(|| anyhow!("provider not enum-tagged"))?;
    let inner = payload
        .first()
        .ok_or_else(|| anyhow!("provider has no payload"))?;
    let contract = map_field(inner, "contract")
        .and_then(address_strkey)
        .ok_or_else(|| anyhow!("provider contract missing"))?;

    match tag.as_str() {
        "Reflector" => Ok(OracleSource {
            kind: OracleKind::Reflector,
            contract,
            asset_ref: map_field(inner, "asset").cloned(),
            feed_id: None,
            max_stale_seconds,
        }),
        "RedStone" | "Xoxno" => {
            let kind = if tag == "RedStone" {
                OracleKind::RedStone
            } else {
                OracleKind::Xoxno
            };
            Ok(OracleSource {
                kind,
                contract,
                asset_ref: None,
                feed_id: map_field(inner, "feed_id").and_then(string_text),
                max_stale_seconds,
            })
        }
        other => Err(anyhow!("unknown provider variant {other}")),
    }
}

pub fn oracle_asset_ref_to_reflector_arg(asset_ref: &ScVal) -> Result<ScVal> {
    let (tag, payload) =
        enum_variant(asset_ref).ok_or_else(|| anyhow!("asset ref not enum-tagged"))?;
    match tag.as_str() {
        "Stellar" => Ok(asset_ref.clone()),
        "Symbol" => {
            let sym = payload
                .first()
                .cloned()
                .ok_or_else(|| anyhow!("Symbol asset ref empty"))?;
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
    let s: StringM = feed_id
        .try_into()
        .map_err(|_| anyhow!("feed id too long"))?;
    Ok(ScVal::String(ScString(s)))
}

pub fn decode_reflector_price(value: &ScVal) -> Result<Option<PriceObservation>> {
    if matches!(value, ScVal::Void) {
        return Ok(None);
    }
    let ts = field_u64(value, "timestamp")
        .ok_or_else(|| anyhow!("ReflectorPriceData.timestamp missing"))?;
    Ok(Some(PriceObservation { feed_ts_secs: ts }))
}

pub fn decode_redstone_price(value: &ScVal) -> Result<PriceObservation> {
    let package_ms = field_u64(value, "package_timestamp")
        .ok_or_else(|| anyhow!("RedStonePriceData.package_timestamp missing"))?;
    let write_ms = field_u64(value, "write_timestamp")
        .ok_or_else(|| anyhow!("RedStonePriceData.write_timestamp missing"))?;
    Ok(PriceObservation {
        feed_ts_secs: package_ms.min(write_ms) / 1000,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use stellar_xdr::curr::{Int128Parts, ScMap, ScMapEntry, ScVec, VecM};

    fn sym(t: &str) -> ScVal {
        ScVal::Symbol(crate::keys::symbol(t).unwrap())
    }
    fn i128v(v: i128) -> ScVal {
        ScVal::I128(Int128Parts {
            hi: (v >> 64) as i64,
            lo: v as u64,
        })
    }
    fn map(entries: Vec<(&str, ScVal)>) -> ScVal {
        ScVal::Map(Some(ScMap(
            entries
                .into_iter()
                .map(|(k, v)| ScMapEntry {
                    key: sym(k),
                    val: v,
                })
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
    fn vec_val(items: Vec<ScVal>) -> ScVal {
        let v: VecM<ScVal> = items.try_into().unwrap();
        ScVal::Vec(Some(ScVec(v)))
    }
    fn addr(byte: u8) -> ScVal {
        crate::keys::sc_address_contract(&[byte; 32])
    }

    fn reflector_feed(contract_byte: u8) -> ScVal {
        map(vec![
            (
                "provider",
                enum_val(
                    "Reflector",
                    vec![map(vec![
                        ("contract", addr(contract_byte)),
                        ("asset", enum_val("Stellar", vec![addr(contract_byte)])),
                        ("read_mode", enum_val("Twap", vec![ScVal::U32(3)])),
                    ])],
                ),
            ),
            ("decimals", ScVal::U32(14)),
            ("max_stale_seconds", ScVal::U64(3600)),
        ])
    }

    fn multi_feed(contract_byte: u8, provider: &str, feed_id: &str, max_stale: u64) -> ScVal {
        map(vec![
            (
                "provider",
                enum_val(
                    provider,
                    vec![map(vec![
                        ("contract", addr(contract_byte)),
                        (
                            "feed_id",
                            ScVal::String(ScString(feed_id.try_into().unwrap())),
                        ),
                        ("nature", sym("Fundamental")),
                    ])],
                ),
            ),
            ("decimals", ScVal::U32(8)),
            ("max_stale_seconds", ScVal::U64(max_stale)),
        ])
    }

    fn asset_oracle(sources: Vec<ScVal>) -> ScVal {
        map(vec![
            ("asset_decimals", ScVal::U32(7)),
            ("max_price_stale_seconds", ScVal::U64(43200)),
            ("sources", vec_val(sources)),
            (
                "tolerance",
                map(vec![
                    ("upper_ratio_bps", ScVal::U32(11000)),
                    ("lower_ratio_bps", ScVal::U32(9091)),
                ]),
            ),
            ("independence", sym("RequireDisjoint")),
            ("min_sanity_price_wad", i128v(0)),
            ("max_sanity_price_wad", i128v(0)),
        ])
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
        assert_eq!(
            decode_reflector_price(&m).unwrap().unwrap().feed_ts_secs,
            42
        );
    }

    #[test]
    fn decodes_dual_source_reflector_and_redstone_asset_oracle() {
        let oracle = asset_oracle(vec![
            enum_val("Feed", vec![reflector_feed(9)]),
            enum_val("Feed", vec![multi_feed(4, "RedStone", "XLM", 43200)]),
        ]);
        let cfg = decode_oracle_config(&oracle).unwrap();

        assert_eq!(cfg.max_price_stale_seconds, 43200);
        assert_eq!(cfg.tolerance_upper_bps, 11000);
        assert_eq!(cfg.tolerance_lower_bps, 9091);
        assert_eq!(cfg.source_count, 2);
        assert_eq!(cfg.sources.len(), 2);

        assert_eq!(cfg.sources[0].kind, OracleKind::Reflector);
        assert_eq!(cfg.sources[0].max_stale_seconds, 3600);
        assert!(cfg.sources[0].asset_ref.is_some());

        assert_eq!(cfg.sources[1].kind, OracleKind::RedStone);
        assert_eq!(cfg.sources[1].max_stale_seconds, 43200);
        assert_eq!(cfg.sources[1].feed_id.as_deref(), Some("XLM"));
    }

    #[test]
    fn decodes_single_source_asset_oracle() {
        let oracle = asset_oracle(vec![enum_val(
            "Feed",
            vec![multi_feed(4, "Xoxno", "BTC/USD", 600)],
        )]);
        let cfg = decode_oracle_config(&oracle).unwrap();

        assert_eq!(cfg.source_count, 1);
        assert_eq!(cfg.sources.len(), 1);
        assert_eq!(cfg.sources[0].kind, OracleKind::Xoxno);
        assert_eq!(cfg.sources[0].max_stale_seconds, 600);
    }

    #[test]
    fn scaled_source_contributes_its_factor_leg() {
        let scaled = enum_val(
            "Scaled",
            vec![map(vec![
                ("factor", multi_feed(4, "RedStone", "SolvBTC/BTC", 900)),
                ("quote", enum_val("Ref", vec![sym("BTC")])),
                ("min_factor_wad", i128v(1)),
                ("max_factor_wad", i128v(2)),
            ])],
        );
        let oracle = asset_oracle(vec![scaled]);
        let cfg = decode_oracle_config(&oracle).unwrap();

        assert_eq!(cfg.source_count, 1);
        assert_eq!(cfg.sources.len(), 1);
        assert_eq!(cfg.sources[0].kind, OracleKind::RedStone);
        assert_eq!(cfg.sources[0].feed_id.as_deref(), Some("SolvBTC/BTC"));
    }

    #[test]
    fn aquarius_lp_source_contributes_no_pollable_feed() {
        let lp_share = enum_val(
            "AquariusLp",
            vec![map(vec![
                ("pool", addr(1)),
                ("token_a", addr(2)),
                ("token_b", addr(3)),
                ("key_a", enum_val("Token", vec![addr(2)])),
                ("key_b", enum_val("Token", vec![addr(3)])),
                ("reserve_a_decimals", ScVal::U32(7)),
                ("reserve_b_decimals", ScVal::U32(7)),
                ("min_pool_value_wad", i128v(1)),
            ])],
        );
        let oracle = asset_oracle(vec![lp_share]);
        let cfg = decode_oracle_config(&oracle).unwrap();

        assert_eq!(cfg.source_count, 1);
        assert!(cfg.sources.is_empty());
    }

    #[test]
    fn reflector_asset_ref_symbol_retags_to_other() {
        let symref = enum_val("Symbol", vec![sym("XLM")]);
        let arg = oracle_asset_ref_to_reflector_arg(&symref).unwrap();
        let (tag, _) = enum_variant(&arg).unwrap();
        assert_eq!(tag, "Other");
    }
}
