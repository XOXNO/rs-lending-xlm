//! On-chain integer → Prometheus `f64` gauge math.
//!
//! APY: per-ms RAY rate, daily-compounded over 365 days (matches api-v2 accrual).

pub const RAY_F64: f64 = 1e27;
pub const WAD_F64: f64 = 1e18;
pub const BPS_F64: f64 = 1e4;
/// ms/day — per-ms rate compounds over this to a daily rate.
pub const MS_PER_DAY: f64 = 86_400_000.0;
pub const DAYS_PER_YEAR: i32 = 365;

pub fn ray_to_f64(value: i128) -> f64 {
    value as f64 / RAY_F64
}

pub fn wad_to_f64(value: i128) -> f64 {
    value as f64 / WAD_F64
}

/// 200 bps → 0.02.
pub fn bps_to_ratio(value: u32) -> f64 {
    f64::from(value) / BPS_F64
}

pub fn token_to_f64(base_units: i128, decimals: u32) -> f64 {
    base_units as f64 / 10f64.powi(decimals as i32)
}

pub fn token_usd(base_units: i128, decimals: u32, price_wad: i128) -> f64 {
    token_to_f64(base_units, decimals) * wad_to_f64(price_wad)
}

/// `(1 + rate_per_ms/RAY * ms_per_day)^365 - 1`.
pub fn apy_from_per_ms_ray(rate_per_ms_ray: i128) -> f64 {
    let daily = (rate_per_ms_ray as f64 / RAY_F64) * MS_PER_DAY;
    (1.0 + daily).powi(DAYS_PER_YEAR) - 1.0
}

/// |primary−anchor| in bps; `None` if anchor is 0.
pub fn deviation_bps(primary_wad: i128, anchor_wad: i128) -> Option<f64> {
    if anchor_wad == 0 {
        return None;
    }
    let dev = (primary_wad as f64 - anchor_wad as f64).abs() / anchor_wad as f64;
    Some(dev * BPS_F64)
}

/// Whole tokens from RAY-scaled share × live index.
///
/// On-chain: `scaled * index / 1e(54-dec)` base units; divide by `10^dec` cancels
/// decimals → `ray(scaled) * ray(index)`.
pub fn scaled_usage_to_token(scaled_ray: i128, index_ray: i128) -> f64 {
    ray_to_f64(scaled_ray) * ray_to_f64(index_ray)
}

/// Headroom until stale: `max_stale - (now - feed_ts)`. Negative if already stale.
/// Matches on-chain `stale iff now > feed_ts && (now - feed_ts) > max_stale`.
pub fn seconds_until_stale(now_secs: i64, feed_ts_secs: u64, max_stale_secs: u64) -> f64 {
    let age = now_secs - feed_ts_secs as i64;
    max_stale_secs as f64 - age as f64
}

/// `usage / cap`; `None` when uncapped (`cap <= 0`).
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
        assert_eq!(token_to_f64(100_000_000, 7), 10.0);
        assert_eq!(token_usd(100_000_000, 7, 2 * WAD_F64 as i128), 20.0);
    }

    #[test]
    fn apy_from_zero_rate_is_zero() {
        assert_eq!(apy_from_per_ms_ray(0), 0.0);
    }

    #[test]
    fn apy_is_positive_and_reasonable_for_a_small_rate() {
        let ms_per_year = 31_556_926_000.0_f64;
        let rate_per_ms_ray = ((0.05 / ms_per_year) * RAY_F64) as i128;
        let apy = apy_from_per_ms_ray(rate_per_ms_ray);
        assert!(apy > 0.049 && apy < 0.052, "apy={apy}");
    }

    #[test]
    fn deviation_bps_matches_hand_calc() {
        let dev = deviation_bps(101 * WAD_F64 as i128, 100 * WAD_F64 as i128).unwrap();
        assert!((dev - 100.0).abs() < 1e-6, "dev={dev}");
        assert_eq!(deviation_bps(1, 0), None);
    }

    #[test]
    fn staleness_countdown_and_negative_when_stale() {
        assert_eq!(seconds_until_stale(1000, 940, 120), 60.0);
        assert_eq!(seconds_until_stale(1000, 800, 120), -80.0);
    }

    #[test]
    fn cap_utilization_guards_uncapped() {
        assert_eq!(cap_utilization(5.0, 0, 7), None);
        assert_eq!(cap_utilization(5.0, 100_000_000, 7), Some(0.5));
    }

    #[test]
    fn scaled_usage_is_share_times_index_no_decimals() {
        let share = 50 * RAY_F64 as i128;
        let index = RAY_F64 as i128;
        assert!((scaled_usage_to_token(share, index) - 50.0).abs() < 1e-6);
        let index_12 = (1.2 * RAY_F64) as i128;
        assert!((scaled_usage_to_token(share, index_12) - 60.0).abs() < 1e-3);
    }
}
