//! Input decoding helpers for libFuzzer byte streams.

/// Decode a `u32` fuzz byte into a bounded `f64` amount in `[lo, hi]`.
#[inline]
pub fn arb_amount(raw: u32, lo: f64, hi: f64) -> f64 {
    debug_assert!(hi > lo);
    let span = (hi - lo).max(1.0);
    lo + (raw as f64 % span)
}

/// Required health-factor floor after a risk-increasing operation.
pub const HF_WAD_FLOOR: f64 = 1.0;