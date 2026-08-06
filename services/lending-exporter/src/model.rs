
pub const RAY_F64: f64 = 1e27;
pub const WAD_F64: f64 = 1e18;
pub const BPS_F64: f64 = 1e4;

pub const MS_PER_DAY: f64 = 86_400_000.0;
pub const DAYS_PER_YEAR: i32 = 365;

pub fn ray_to_f64(value: i128) -> f64 {
    value as f64 / RAY_F64
}

pub fn wad_to_f64(value: i128) -> f64 {
    value as f64 / WAD_F64
}

pub fn bps_to_ratio(value: u32) -> f64 {
    f64::from(value) / BPS_F64
}

pub fn token_to_f64(base_units: i128, decimals: u32) -> f64 {
    base_units as f64 / 10f64.powi(decimals as i32)
}

pub fn token_usd(base_units: i128, decimals: u32, price_wad: i128) -> f64 {
    token_to_f64(base_units, decimals) * wad_to_f64(price_wad)
}

pub fn apy_from_per_ms_ray(rate_per_ms_ray: i128) -> f64 {
    let daily = (rate_per_ms_ray as f64 / RAY_F64) * MS_PER_DAY;
    (1.0 + daily).powi(DAYS_PER_YEAR) - 1.0
}

pub fn deviation_bps(primary_wad: i128, anchor_wad: i128) -> Option<f64> {
    if anchor_wad == 0 {
        return None;
    }
    let dev = (primary_wad as f64 - anchor_wad as f64).abs() / anchor_wad as f64;
    Some(dev * BPS_F64)
}

pub fn scaled_usage_to_token(scaled_ray: i128, index_ray: i128) -> f64 {
    ray_to_f64(scaled_ray) * ray_to_f64(index_ray)
}

pub fn seconds_until_stale(now_secs: i64, feed_ts_secs: u64, max_stale_secs: u64) -> f64 {
    let age = now_secs - feed_ts_secs as i64;
    max_stale_secs as f64 - age as f64
}

/// Usage / cap as a ratio, or `None` when the ratio has no arithmetically
/// correct value.
///
/// A cap is always an enforced ceiling in asset units; there is no "unlimited"
/// sentinel. `cap_base_units == 0` therefore means the side is **closed** — it
/// accepts nothing — so usage/cap is 0/0 and undefined. Publish
/// [`market_closed`] alongside this so a dashboard can tell a closed market from
/// an absent scrape.
pub fn cap_utilization(usage_token: f64, cap_base_units: i128, decimals: u32) -> Option<f64> {
    if cap_base_units <= 0 {
        return None;
    }
    let cap = token_to_f64(cap_base_units, decimals);
    if cap == 0.0 {
        // A non-zero cap that underflows f64 (absurd `decimals`) is still not a
        // usable divisor.
        return None;
    }
    Some(usage_token / cap)
}

/// `1.0` when a cap of `0` closes that side of the market, `0.0` otherwise.
///
/// Caps and the `can_be_collateral` / `can_be_borrowed` flags are orthogonal by
/// design, so `cap == 0` on a side whose flag is enabled is a legitimate soft
/// wind-down: the listing reads as live everywhere else. This gauge is the
/// signal that says otherwise, and it needs no `decimals`, so it stays
/// publishable even when the asset's decimals could not be read.
///
/// A negative cap is not reachable on-chain; it is reported as closed to match
/// [`cap_utilization`]'s guard rather than silently reading as open.
pub fn market_closed(cap_base_units: i128) -> f64 {
    if cap_base_units <= 0 {
        1.0
    } else {
        0.0
    }
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
    fn cap_utilization_is_undefined_for_a_closed_market() {
        // cap == 0 closes the side: 0/0 has no correct ratio, so callers must
        // read `market_closed` instead of inferring anything from the gap.
        assert_eq!(cap_utilization(5.0, 0, 7), None);
        assert_eq!(cap_utilization(0.0, 0, 7), None);
        assert_eq!(cap_utilization(5.0, 100_000_000, 7), Some(0.5));
    }

    #[test]
    fn cap_utilization_reports_a_saturated_open_market() {
        assert_eq!(cap_utilization(10.0, 100_000_000, 7), Some(1.0));
        assert_eq!(cap_utilization(1e-7, 1, 7), Some(1.0));
    }

    #[test]
    fn market_closed_flags_only_a_zero_cap() {
        assert_eq!(market_closed(0), 1.0);
        assert_eq!(market_closed(1), 0.0);
        assert_eq!(market_closed(i128::MAX), 0.0);
    }

    #[test]
    fn market_closed_treats_a_negative_cap_as_closed() {
        assert_eq!(market_closed(-1), 1.0);
        assert_eq!(market_closed(i128::MIN), 1.0);
    }

    #[test]
    fn closed_market_is_exactly_the_case_cap_utilization_drops() {
        // The two signals must stay complementary: whenever the closed gauge is
        // 1 the utilization series is absent, and whenever it is 0 the series is
        // present. Otherwise a dashboard cannot tell "closed" from "no scrape".
        for cap in [i128::MIN, -1, 0, 1, 100_000_000, i128::MAX] {
            let closed = market_closed(cap) == 1.0;
            assert_eq!(closed, cap_utilization(5.0, cap, 7).is_none(), "cap={cap}");
        }
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
