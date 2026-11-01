#[inline]
pub fn scaled_amount(raw: u8, lo: f64, hi: f64) -> f64 {
    debug_assert!(hi >= lo);
    lo + (hi - lo) * (raw as f64 / u8::MAX as f64)
}

#[inline]
pub fn fraction(raw: u8) -> f64 {
    ((raw as f64) + 1.0) / 256.0
}

#[inline]
pub fn asset_price_usd(asset: &str) -> f64 {
    match asset {
        "ETH" => 2_000.0,
        "XLM" => 0.10,
        _ => 1.0,
    }
}

#[inline]
pub fn log_scaled_amount(raw: u8, lo: f64, hi: f64) -> f64 {
    debug_assert!(lo > 0.0 && hi >= lo);
    let t = raw as f64 / u8::MAX as f64;
    (lo.ln() + (hi.ln() - lo.ln()) * t).exp()
}

#[inline]
pub fn amount_for_value(raw: u8, asset: &str, min_usd: f64, max_usd: f64) -> f64 {
    log_scaled_amount(raw, min_usd, max_usd) / asset_price_usd(asset)
}

fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// Maps fuzzer entropy to an integer in `[lo, hi]`, favouring the bounds, `hi - 1`,
/// powers of ten, and log-uniform magnitudes.
///
/// A linear mapping over a 128-bit span almost never lands on a bit-width or a bound,
/// which is where fixed-point code breaks.
pub fn edge_biased_amount(entropy: u64, lo: i128, hi: i128) -> i128 {
    debug_assert!(0 <= lo && lo < hi);
    let e = splitmix64(entropy);
    match e & 7 {
        0 => lo,
        1 => hi,
        2 => hi - 1,
        3 => 10i128
            .checked_pow(((e >> 3) % 39) as u32)
            .unwrap_or(i128::MAX)
            .clamp(lo, hi),
        _ => {
            let bits = 128 - (hi as u128).leading_zeros();
            let m = ((e >> 3) % bits as u64) as u32;
            let r = ((splitmix64(e) as u128) << 64) | splitmix64(e.rotate_left(32)) as u128;
            ((r & ((1u128 << m) - 1)) as i128).clamp(lo, hi)
        }
    }
}

pub const HF_WAD_FLOOR: f64 = 1.0;
