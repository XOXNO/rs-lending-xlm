//! Contract error codes returned via `panic_with_error!`.
//!
//! The numbering is not contiguous (2, 6, 8, 10, 14-19, 23, 24 are unused).
//! Whatever the reason for any individual gap, codes are part of the contract's
//! observable interface: never backfill one, always append.

use soroban_sdk::contracterror;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    /// Instruction count is zero or above the program cap.
    EmptyBatch = 1,
    /// Non-positive amount, overdraft, or spend mismatch.
    InvalidAmount = 3,
    /// `Prev` has no predecessor output, or it names a different token.
    BrokenTokenChain = 4,
    /// Delivered output below the declared minimum.
    SlippageExceeded = 5,
    /// Venue returned zero usable output.
    ZeroOutput = 7,
    /// Checked arithmetic overflow.
    IntegerOverflow = 9,
    /// A split weight is zero.
    ZeroSplitPpm = 11,
    /// A split weight exceeds 1e6.
    SplitPpmMismatch = 12,
    /// Strategy XDR or packed program failed to decode or validate.
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
    /// A contract-internal invariant no longer holds; the call fails closed
    /// rather than acting on state it can no longer trust. One occurrence is
    /// expected: fee claims on an instance upgraded from a build that
    /// predates `ReservedTotal` fail with this code until
    /// [`crate::Router::migrate_reserved_totals`] runs for the token. Any
    /// other sighting is a bug.
    InternalInvariant = 30,
}
