use controller::constants::WAD;

/// Returns `n` US dollars in WAD.
pub const fn usd(n: i128) -> i128 {
    n * WAD
}

/// Returns `n` US cents in WAD.
pub const fn usd_cents(n: i128) -> i128 {
    n * WAD / 100
}

/// Returns `num / den` US dollars in WAD, rounded toward zero.
pub const fn usd_frac(num: i128, den: i128) -> i128 {
    num * WAD / den
}

pub const fn days(n: u64) -> u64 {
    n * 86_400
}

/// Converts a float token amount to raw units at `decimals`, truncating toward zero.
pub fn f64_to_i128(amount: f64, decimals: u32) -> i128 {
    (amount * 10f64.powi(decimals as i32)) as i128
}

pub fn i128_to_f64(amount: i128, decimals: u32) -> f64 {
    amount as f64 / 10f64.powi(decimals as i32)
}

pub fn wad_to_f64(amount: i128) -> f64 {
    amount as f64 / WAD as f64
}
