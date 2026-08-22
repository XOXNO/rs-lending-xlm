//! Fee caps, path-split scale, and residual dust policy.

/// Path split weights sum to this value (parts per million).
pub(crate) const PPM_DENOMINATOR: i128 = 1_000_000;

/// Minimum residual dust allowed to accrue as admin fee after settlement.
pub(crate) const RESIDUAL_DUST_FLOOR: i128 = 1_000;

/// Residual allowance = `credited / RESIDUAL_PPM`, at least [`RESIDUAL_DUST_FLOOR`].
pub(crate) const RESIDUAL_PPM: i128 = 1_000_000;

/// Max static + referral fee combined, in basis points (10%).
pub(crate) const FEE_CAP: u32 = 1_000;

/// Returns the residual allowance for a token given `credited`: `credited / RESIDUAL_PPM`,
/// floored at [`RESIDUAL_DUST_FLOOR`].
pub(crate) fn residual_allowance(credited: i128) -> i128 {
    (credited / RESIDUAL_PPM).max(RESIDUAL_DUST_FLOOR)
}
