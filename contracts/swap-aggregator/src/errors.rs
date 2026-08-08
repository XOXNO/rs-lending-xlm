//! Contract error codes returned via `panic_with_error!`.

use soroban_sdk::contracterror;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    /// No paths and no LP burn/mint legs.
    EmptyBatch = 1,
    /// Path missing or hop list empty.
    EmptyPath = 2,
    /// Non-positive amount, overdraft, or spend mismatch.
    InvalidAmount = 3,
    /// Hop token chain or path endpoints do not line up.
    BrokenTokenChain = 4,
    /// Delivered output below `total_min_out`.
    SlippageExceeded = 5,
    /// Venue returned zero usable output.
    ZeroOutput = 7,
    /// Checked arithmetic overflow.
    IntegerOverflow = 9,
    /// Path `split_ppm` is zero.
    ZeroSplitPpm = 11,
    /// Per-token split weights do not sum to 1e6 (or exceed it).
    SplitPpmMismatch = 12,
    /// Strategy XDR failed to decode.
    InvalidRouteXdr = 13,
    /// Ownable owner missing when required.
    NotAdmin = 20,
    /// Fee bps above [`crate::constants::FEE_CAP`].
    FeeTooHigh = 21,
    /// Referral id not in storage.
    ReferralNotFound = 22,
    /// Input and output token are identical.
    SameToken = 25,
    /// Declared LP token is not the pool share token.
    LpTokenMismatch = 26,
    /// Mint delivered fewer shares than `mint_min_shares`.
    MinSharesNotMet = 27,
    /// Burn constituent below its min amount.
    MinAmountsNotMet = 28,
    /// Leftover vault balance exceeds residual allowance.
    ExcessiveResidual = 29,
}
