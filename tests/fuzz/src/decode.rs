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
pub fn amount_for_value(raw: u8, asset: &str, min_usd: f64, max_usd: f64) -> f64 {
    scaled_amount(raw, min_usd, max_usd) / asset_price_usd(asset)
}

pub const HF_WAD_FLOOR: f64 = 1.0;
