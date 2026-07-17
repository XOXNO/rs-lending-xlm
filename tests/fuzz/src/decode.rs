//! Input decoding helpers for libFuzzer byte streams.

#[inline]
pub fn arb_amount(raw: u32, lo: f64, hi: f64) -> f64 {
    debug_assert!(hi > lo);
    let span = (hi - lo).max(1.0);
    lo + (raw as f64 % span)
}

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

/// Decode a byte into an asset amount via a USD value range.
#[inline]
pub fn amount_for_value(raw: u8, asset: &str, min_usd: f64, max_usd: f64) -> f64 {
    scaled_amount(raw, min_usd, max_usd) / asset_price_usd(asset)
}

/// Required health-factor floor after a risk-increasing operation.
pub const HF_WAD_FLOOR: f64 = 1.0;
