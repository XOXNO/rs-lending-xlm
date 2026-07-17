//! Router error codes.

use soroban_sdk::contracterror;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    EmptyBatch = 1,
    EmptyPath = 2,
    InvalidAmount = 3,
    BrokenTokenChain = 4,
    SlippageExceeded = 5,
    ZeroOutput = 7,
    IntegerOverflow = 9,
    ZeroSplitPpm = 11,
    SplitPpmMismatch = 12,
    InvalidRouteXdr = 13,
    NotAdmin = 20,
    FeeTooHigh = 21,
    ReferralNotFound = 22,
    SameToken = 25,
}
