use controller::constants::WAD;

pub const fn usd(n: i128) -> i128 {
    n * WAD
}

pub const fn usd_cents(n: i128) -> i128 {
    n * WAD / 100
}

pub const fn usd_frac(num: i128, den: i128) -> i128 {
    num * WAD / den
}

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

pub fn tokens(n: i128, decimals: u32) -> i128 {
    n * 10i128.pow(decimals)
}

pub const fn bps(n: i128) -> i128 {
    n
}

pub fn f64_to_i128(amount: f64, decimals: u32) -> i128 {
    (amount * 10f64.powi(decimals as i32)) as i128
}

pub fn i128_to_f64(amount: i128, decimals: u32) -> f64 {
    amount as f64 / 10f64.powi(decimals as i32)
}

pub fn wad_to_f64(amount: i128) -> f64 {
    amount as f64 / WAD as f64
}
