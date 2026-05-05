use common::constants::WAD;

// ---------------------------------------------------------------------------
// Price helpers (all return i128, WAD-scaled)
// ---------------------------------------------------------------------------

/// Whole-dollar price: usd(1) = 1 WAD, usd(2000) = 2000 WAD.
pub const fn usd(n: i128) -> i128 {
    n * WAD
}

/// Cent-precision price: usd_cents(50) = $0.50.
pub const fn usd_cents(n: i128) -> i128 {
    n * WAD / 100
}

/// Fractional price: usd_frac(3, 10) = $0.30.
pub const fn usd_frac(num: i128, den: i128) -> i128 {
    num * WAD / den
}

// ---------------------------------------------------------------------------
// Time helpers (all return u64 seconds)
// ---------------------------------------------------------------------------

pub const fn days(n: u64) -> u64 {
    n * 86_400
}

pub const fn hours(n: u64) -> u64 {
    n * 3_600
}

pub const fn minutes(n: u64) -> u64 {
    n * 60
}

pub const fn secs(n: u64) -> u64 {
    n
}

// ---------------------------------------------------------------------------
// Amount helpers
// ---------------------------------------------------------------------------

/// Convert a human-readable amount to on-chain representation.
/// tokens(1000, 7) = 1000_0000000.
pub fn tokens(n: i128, decimals: u32) -> i128 {
    n * 10i128.pow(decimals)
}

/// Identity function -- documents that a value is in basis points.
pub const fn bps(n: i128) -> i128 {
    n
}

/// Convert f64 amount to i128 using asset decimals.
/// f64_to_i128(1000.5, 7) = 10005000000.
pub fn f64_to_i128(amount: f64, decimals: u32) -> i128 {
    (amount * 10f64.powi(decimals as i32)) as i128
}

/// Convert i128 to f64 using asset decimals.
pub fn i128_to_f64(amount: i128, decimals: u32) -> f64 {
    amount as f64 / 10f64.powi(decimals as i32)
}

/// Convert WAD-scaled i128 to f64 (divide by 10^18).
pub fn wad_to_f64(amount: i128) -> f64 {
    amount as f64 / WAD as f64
}
