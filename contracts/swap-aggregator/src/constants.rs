//! Fee caps, path-split scale, and residual dust policy.

/// Path split weights sum to this value (parts per million).
pub(crate) const PPM_DENOMINATOR: i128 = 1_000_000;

/// Minimum residual dust allowed to accrue as admin fee after settlement.
pub(crate) const RESIDUAL_DUST_FLOOR: i128 = 1_000;

/// Residual allowance = `credited / RESIDUAL_PPM`, at least [`RESIDUAL_DUST_FLOOR`].
pub(crate) const RESIDUAL_PPM: i128 = 1_000_000;

/// Max static + referral fee combined, in basis points (10%).
pub(crate) const FEE_CAP: u32 = 1_000;

/// Max leftover vault amount for a token after settlement, given how much was credited.
pub(crate) fn residual_allowance(credited: i128) -> i128 {
    let proportional = credited / RESIDUAL_PPM;
    if proportional > RESIDUAL_DUST_FLOOR {
        proportional
    } else {
        RESIDUAL_DUST_FLOOR
    }
}
