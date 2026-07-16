//! Scaling and rate math for turning raw on-chain integers into gauge values.
//!
//! All outputs are `f64` (Prometheus gauges are `f64`); for a dashboard the
//! mantissa precision is ample. The APY formula mirrors api-v2's
//! `stellar-lending.accrual.ts`: a per-millisecond RAY rate compounds over 365
//! daily periods.

/// RAY fixed-point scale (1e27).
pub const RAY_F64: f64 = 1e27;
/// WAD fixed-point scale (1e18).
pub const WAD_F64: f64 = 1e18;
/// Basis-point scale (1e4).
pub const BPS_F64: f64 = 1e4;
/// Milliseconds per day — the per-ms rate accrues over this to a daily rate.
pub const MS_PER_DAY: f64 = 86_400_000.0;
/// Daily compounding periods per year, matching the indexer's APY convention.
pub const DAYS_PER_YEAR: i32 = 365;

/// RAY-scaled integer to a plain ratio (e.g. an index or utilization).
pub fn ray_to_f64(value: i128) -> f64 {
    value as f64 / RAY_F64
}

/// WAD-scaled integer (USD price) to a plain number.
pub fn wad_to_f64(value: i128) -> f64 {
    value as f64 / WAD_F64
}

/// Basis points to a ratio (200 bps -> 0.02).
pub fn bps_to_ratio(value: u32) -> f64 {
    f64::from(value) / BPS_F64
}

/// Base-unit token amount to a whole-token number by its decimals.
pub fn token_to_f64(base_units: i128, decimals: u32) -> f64 {
    base_units as f64 / 10f64.powi(decimals as i32)
}

/// USD value of a base-unit token amount at a WAD price.
pub fn token_usd(base_units: i128, decimals: u32, price_wad: i128) -> f64 {
    token_to_f64(base_units, decimals) * wad_to_f64(price_wad)
}

/// Annual APY from a per-millisecond RAY rate, daily-compounded over 365 days —
/// `(1 + rate_per_ms/RAY * ms_per_day)^365 - 1`.
pub fn apy_from_per_ms_ray(rate_per_ms_ray: i128) -> f64 {
    let daily = (rate_per_ms_ray as f64 / RAY_F64) * MS_PER_DAY;
    (1.0 + daily).powi(DAYS_PER_YEAR) - 1.0
}

/// Absolute primary/anchor deviation in basis points, `None` when anchor is 0.
pub fn deviation_bps(primary_wad: i128, anchor_wad: i128) -> Option<f64> {
    if anchor_wad == 0 {
        return None;
    }
    let dev = (primary_wad as f64 - anchor_wad as f64).abs() / anchor_wad as f64;
    Some(dev * BPS_F64)
}

/// Whole-token usage of a RAY-scaled share at a live index.
///
/// On-chain the token amount is `Ray::from(scaled).mul(index).to_asset(dec)`,
/// i.e. base units = `scaled * index / 1e(54-dec)`; whole tokens divide that by
/// `10^dec`, so the decimals cancel and whole tokens = `scaled * index / 1e54`
/// = `ray(scaled) * ray(index)`. No decimals term.
pub fn scaled_usage_to_token(scaled_ray: i128, index_ray: i128) -> f64 {
    ray_to_f64(scaled_ray) * ray_to_f64(index_ray)
}

/// Seconds until a price feed is considered stale, matching the on-chain
/// predicate `stale iff now > feed_ts && (now - feed_ts) > max_stale`. Negative
/// once the feed is already stale.
pub fn seconds_until_stale(now_secs: i64, feed_ts_secs: u64, max_stale_secs: u64) -> f64 {
    let age = now_secs - feed_ts_secs as i64;
    max_stale_secs as f64 - age as f64
}

/// Cap utilization ratio (usage / cap), `None` when uncapped (`cap <= 0`).
pub fn cap_utilization(usage_token: f64, cap_base_units: i128, decimals: u32) -> Option<f64> {
    if cap_base_units <= 0 {
        return None;
    }
    let cap = token_to_f64(cap_base_units, decimals);
    if cap == 0.0 {
        return None;
    }
    Some(usage_token / cap)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ray_and_wad_scale() {
        assert_eq!(ray_to_f64(RAY_F64 as i128), 1.0);
        assert_eq!(wad_to_f64(2 * WAD_F64 as i128), 2.0);
        assert_eq!(bps_to_ratio(200), 0.02);
    }

    #[test]
    fn token_denomination_and_usd() {
        // 10 tokens at 7 decimals = 1e8 base units.
        assert_eq!(token_to_f64(100_000_000, 7), 10.0);
        // 10 tokens @ $2 (2 * WAD) = $20.
        assert_eq!(token_usd(100_000_000, 7, 2 * WAD_F64 as i128), 20.0);
    }

    #[test]
    fn apy_from_zero_rate_is_zero() {
        assert_eq!(apy_from_per_ms_ray(0), 0.0);
    }

    #[test]
    fn apy_is_positive_and_reasonable_for_a_small_rate() {
        // ~5% APR expressed per-ms: 0.05 / ms_per_year, in RAY.
        let ms_per_year = 31_556_926_000.0_f64;
        let rate_per_ms_ray = ((0.05 / ms_per_year) * RAY_F64) as i128;
        let apy = apy_from_per_ms_ray(rate_per_ms_ray);
        assert!(apy > 0.049 && apy < 0.052, "apy={apy}");
    }

    #[test]
    fn deviation_bps_matches_hand_calc() {
        // primary 101, anchor 100 -> 1% -> 100 bps.
        let dev = deviation_bps(101 * WAD_F64 as i128, 100 * WAD_F64 as i128).unwrap();
        assert!((dev - 100.0).abs() < 1e-6, "dev={dev}");
        assert_eq!(deviation_bps(1, 0), None);
    }

    #[test]
    fn staleness_countdown_and_negative_when_stale() {
        // now=1000, feed=940, max=120 -> 60s left.
        assert_eq!(seconds_until_stale(1000, 940, 120), 60.0);
        // now=1000, feed=800, max=120 -> -80s (stale).
        assert_eq!(seconds_until_stale(1000, 800, 120), -80.0);
    }

    #[test]
    fn cap_utilization_guards_uncapped() {
        assert_eq!(cap_utilization(5.0, 0, 7), None);
        assert_eq!(cap_utilization(5.0, 100_000_000, 7), Some(0.5));
    }

    #[test]
    fn scaled_usage_is_share_times_index_no_decimals() {
        // 50 whole-token share (RAY) at index 1.0 (RAY) -> 50 tokens.
        let share = 50 * RAY_F64 as i128;
        let index = RAY_F64 as i128;
        assert!((scaled_usage_to_token(share, index) - 50.0).abs() < 1e-6);
        // At index 1.2, the same share is worth 60 tokens.
        let index_12 = (1.2 * RAY_F64) as i128;
        assert!((scaled_usage_to_token(share, index_12) - 60.0).abs() < 1e-3);
    }
}
