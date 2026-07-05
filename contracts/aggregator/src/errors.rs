//! Contract error codes.

use soroban_sdk::contracterror;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    /// The swap payload contained zero paths.
    EmptyBatch = 1,
    /// A path contained zero hops.
    EmptyPath = 2,
    /// `total_in <= 0` or a per-path allocation computed to zero.
    InvalidAmount = 3,
    /// Token chain broken — `hops[i].token_out != hops[i+1].token_in`,
    /// or two paths disagree on first-hop `token_in` /
    /// last-hop `token_out`.
    BrokenTokenChain = 4,
    /// `total_min_out <= 0`, or aggregate output across all paths was
    /// less than `total_min_out`.
    SlippageExceeded = 5,
    /// A venue returned zero output — treated as drained pool.
    ZeroOutput = 7,
    /// Checked arithmetic overflowed, or an integer conversion (e.g.
    /// `i128` <-> `u128`) was out of range.
    IntegerOverflow = 9,
    /// `path.split_ppm == 0`.
    ZeroSplitPpm = 11,
    /// Sum of `path.split_ppm` across all paths must equal `1_000_000`.
    SplitPpmMismatch = 12,
    /// Swap XDR did not decode into the router-owned payload type.
    InvalidRouteXdr = 13,
    /// No admin is set in storage (contract not yet constructed).
    /// Caller-vs-admin mismatches instead trap via `Address::require_auth`.
    NotAdmin = 20,
    /// Fee config exceeds the per-side cap.
    FeeTooHigh = 21,
    /// Referral ID does not exist.
    ReferralNotFound = 22,
    /// Already initialised.
    AlreadyInitialised = 24,
    /// Batch input and output token are identical.
    SameToken = 25,
}
