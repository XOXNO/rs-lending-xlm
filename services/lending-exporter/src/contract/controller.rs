//! Typed decoders for the controller's view return values.

use anyhow::{anyhow, Result};
use stellar_xdr::curr::ScVal;

use crate::scval::{field_bool, field_i128, field_u32, map_field, vec_items};

/// One `get_market_indexes_detailed` row: RAY indexes + WAD prices.
#[derive(Debug, Clone)]
pub struct MarketIndexView {
    pub supply_index_ray: i128,
    pub borrow_index_ray: i128,
    pub final_price_wad: i128,
    pub primary_price_wad: i128,
    pub anchor_price_wad: i128,
}

#[derive(Debug, Clone)]
pub struct SpokeConfig {
    pub is_deprecated: bool,
    pub liquidation_target_hf_wad: i128,
    pub hf_for_max_bonus_wad: i128,
    pub liquidation_bonus_factor_bps: u32,
}

/// Spoke-asset listing: flags, risk ratios, caps (`oracle_override` unused).
#[derive(Debug, Clone)]
pub struct SpokeAssetConfig {
    pub is_collateralizable: bool,
    pub is_borrowable: bool,
    pub paused: bool,
    pub frozen: bool,
    pub loan_to_value_bps: u32,
    pub liquidation_threshold_bps: u32,
    pub liquidation_bonus_bps: u32,
    pub liquidation_fees_bps: u32,
    pub supply_cap: i128,
    pub borrow_cap: i128,
}

#[derive(Debug, Clone, Default)]
pub struct SpokeUsage {
    pub supplied_scaled_ray: i128,
    pub borrowed_scaled_ray: i128,
}

/// `get_market_indexes_detailed` rows, index-aligned to the request.
pub fn decode_market_indexes(value: &ScVal) -> Result<Vec<MarketIndexView>> {
    let items = vec_items(value).ok_or_else(|| anyhow!("expected Vec<MarketIndexView>"))?;
    items.iter().map(decode_market_index_row).collect()
}

fn decode_market_index_row(value: &ScVal) -> Result<MarketIndexView> {
    Ok(MarketIndexView {
        supply_index_ray: field_i128(value, "supply_index")
            .ok_or_else(|| anyhow!("supply_index missing"))?,
        borrow_index_ray: field_i128(value, "borrow_index")
            .ok_or_else(|| anyhow!("borrow_index missing"))?,
        final_price_wad: field_i128(value, "price_wad").ok_or_else(|| anyhow!("price_wad missing"))?,
        primary_price_wad: field_i128(value, "safe_price_wad")
            .ok_or_else(|| anyhow!("safe_price_wad missing"))?,
        anchor_price_wad: field_i128(value, "aggregator_price_wad")
            .ok_or_else(|| anyhow!("aggregator_price_wad missing"))?,
    })
}

pub fn decode_spoke(value: &ScVal) -> Result<SpokeConfig> {
    Ok(SpokeConfig {
        is_deprecated: field_bool(value, "is_deprecated").unwrap_or(false),
        liquidation_target_hf_wad: field_i128(value, "liquidation_target_hf_wad").unwrap_or(0),
        hf_for_max_bonus_wad: field_i128(value, "hf_for_max_bonus_wad").unwrap_or(0),
        liquidation_bonus_factor_bps: field_u32(value, "liquidation_bonus_factor_bps").unwrap_or(0),
    })
}

pub fn decode_spoke_asset(value: &ScVal) -> Result<SpokeAssetConfig> {
    Ok(SpokeAssetConfig {
        is_collateralizable: field_bool(value, "is_collateralizable").unwrap_or(false),
        is_borrowable: field_bool(value, "is_borrowable").unwrap_or(false),
        paused: field_bool(value, "paused").unwrap_or(false),
        frozen: field_bool(value, "frozen").unwrap_or(false),
        loan_to_value_bps: field_u32(value, "loan_to_value").unwrap_or(0),
        liquidation_threshold_bps: field_u32(value, "liquidation_threshold").unwrap_or(0),
        liquidation_bonus_bps: field_u32(value, "liquidation_bonus").unwrap_or(0),
        liquidation_fees_bps: field_u32(value, "liquidation_fees").unwrap_or(0),
        supply_cap: field_i128(value, "supply_cap").unwrap_or(0),
        borrow_cap: field_i128(value, "borrow_cap").unwrap_or(0),
    })
}

pub fn decode_spoke_usage(value: &ScVal) -> Result<SpokeUsage> {
    // Missing fields → 0 (contract zero-default when no usage row).
    if !matches!(value, ScVal::Map(_)) {
        return Err(anyhow!("expected SpokeUsageRaw map"));
    }
    Ok(SpokeUsage {
        supplied_scaled_ray: field_i128(value, "supplied_scaled_ray").unwrap_or(0),
        borrowed_scaled_ray: field_i128(value, "borrowed_scaled_ray").unwrap_or(0),
    })
}

pub fn spoke_asset_has_oracle_override(value: &ScVal) -> bool {
    match map_field(value, "oracle_override").and_then(crate::scval::enum_variant) {
        Some((tag, _)) => tag == "Some",
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stellar_xdr::curr::{Int128Parts, ScMap, ScMapEntry, ScVec};

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

    #[test]
    fn decodes_market_index_vec_in_order() {
        let row = map(vec![
            ("asset", i128v(0)), // unused by decoder
            ("supply_index", i128v(1_000_000)),
            ("borrow_index", i128v(2_000_000)),
            ("price_wad", i128v(100)),
            ("safe_price_wad", i128v(101)),
            ("aggregator_price_wad", i128v(99)),
        ]);
        let vec = ScVal::Vec(Some(ScVec(vec![row].try_into().unwrap())));
        let decoded = decode_market_indexes(&vec).unwrap();
        assert_eq!(decoded.len(), 1);
        assert_eq!(decoded[0].primary_price_wad, 101);
        assert_eq!(decoded[0].anchor_price_wad, 99);
    }

    #[test]
    fn decodes_spoke_asset_flags_and_caps() {
        let m = map(vec![
            ("is_collateralizable", ScVal::Bool(true)),
            ("is_borrowable", ScVal::Bool(false)),
            ("paused", ScVal::Bool(true)),
            ("frozen", ScVal::Bool(false)),
            ("loan_to_value", ScVal::U32(7500)),
            ("liquidation_threshold", ScVal::U32(8000)),
            ("liquidation_bonus", ScVal::U32(500)),
            ("liquidation_fees", ScVal::U32(100)),
            ("supply_cap", i128v(1_000_000)),
            ("borrow_cap", i128v(0)),
        ]);
        let cfg = decode_spoke_asset(&m).unwrap();
        assert!(cfg.is_collateralizable);
        assert!(cfg.paused);
        assert_eq!(cfg.loan_to_value_bps, 7500);
        assert_eq!(cfg.borrow_cap, 0);
    }

    #[test]
    fn spoke_usage_missing_fields_default_zero() {
        let usage = decode_spoke_usage(&map(vec![])).unwrap();
        assert_eq!(usage.supplied_scaled_ray, 0);
        assert_eq!(usage.borrowed_scaled_ray, 0);
    }
}
