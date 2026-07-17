//! Typed decoders for the central pool's view return values.

use anyhow::{anyhow, Result};
use stellar_xdr::curr::ScVal;

use crate::scval::{field_bool, field_i128, field_u32, field_u64, map_field};

/// IRM params (RAY rates/slopes/util + BPS fees) and asset decimals.
#[derive(Debug, Clone)]
pub struct MarketParams {
    pub asset_decimals: u32,
    pub reserve_factor_bps: u32,
    pub is_flashloanable: bool,
    pub flashloan_fee_bps: u32,
    pub max_borrow_rate_ray: i128,
    pub base_borrow_rate_ray: i128,
    pub slope1_ray: i128,
    pub slope2_ray: i128,
    pub slope3_ray: i128,
    pub mid_utilization_ray: i128,
    pub optimal_utilization_ray: i128,
    pub max_utilization_ray: i128,
}

#[derive(Debug, Clone)]
pub struct MarketSync {
    pub params: MarketParams,
    pub last_timestamp: u64,
}

pub fn decode_i128(value: &ScVal) -> Result<i128> {
    crate::scval::as_i128(value).ok_or_else(|| anyhow!("expected i128, got {value:?}"))
}

pub fn decode_sync_data(value: &ScVal) -> Result<MarketSync> {
    let params_val = map_field(value, "params").ok_or_else(|| anyhow!("PoolSyncData.params missing"))?;
    let state_val = map_field(value, "state").ok_or_else(|| anyhow!("PoolSyncData.state missing"))?;

    let params = MarketParams {
        asset_decimals: field_u32(params_val, "asset_decimals")
            .ok_or_else(|| anyhow!("params.asset_decimals missing"))?,
        reserve_factor_bps: field_u32(params_val, "reserve_factor")
            .ok_or_else(|| anyhow!("params.reserve_factor missing"))?,
        is_flashloanable: field_bool(params_val, "is_flashloanable").unwrap_or(false),
        flashloan_fee_bps: field_u32(params_val, "flashloan_fee").unwrap_or(0),
        max_borrow_rate_ray: field_i128(params_val, "max_borrow_rate").unwrap_or(0),
        base_borrow_rate_ray: field_i128(params_val, "base_borrow_rate").unwrap_or(0),
        slope1_ray: field_i128(params_val, "slope1").unwrap_or(0),
        slope2_ray: field_i128(params_val, "slope2").unwrap_or(0),
        slope3_ray: field_i128(params_val, "slope3").unwrap_or(0),
        mid_utilization_ray: field_i128(params_val, "mid_utilization").unwrap_or(0),
        optimal_utilization_ray: field_i128(params_val, "optimal_utilization").unwrap_or(0),
        max_utilization_ray: field_i128(params_val, "max_utilization").unwrap_or(0),
    };
    let last_timestamp = field_u64(state_val, "last_timestamp").unwrap_or(0);
    Ok(MarketSync { params, last_timestamp })
}

#[cfg(test)]
mod tests {
    use super::*;
    use stellar_xdr::curr::{Int128Parts, ScMap, ScMapEntry};

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
    fn decodes_sync_data_params_and_timestamp() {
        let params = map(vec![
            ("asset_decimals", ScVal::U32(7)),
            ("reserve_factor", ScVal::U32(1000)),
            ("is_flashloanable", ScVal::Bool(true)),
            ("flashloan_fee", ScVal::U32(9)),
            ("max_borrow_rate", i128v(123)),
            ("base_borrow_rate", i128v(1)),
            ("slope1", i128v(2)),
            ("slope2", i128v(3)),
            ("slope3", i128v(4)),
            ("mid_utilization", i128v(5)),
            ("optimal_utilization", i128v(6)),
            ("max_utilization", i128v(7)),
        ]);
        let state = map(vec![("last_timestamp", ScVal::U64(1_700_000_000))]);
        let sync = decode_sync_data(&map(vec![("params", params), ("state", state)])).unwrap();
        assert_eq!(sync.params.asset_decimals, 7);
        assert_eq!(sync.params.reserve_factor_bps, 1000);
        assert!(sync.params.is_flashloanable);
        assert_eq!(sync.last_timestamp, 1_700_000_000);
    }
}
