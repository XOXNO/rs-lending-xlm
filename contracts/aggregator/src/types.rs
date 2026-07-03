//! Contract ABI types.
//!
//! These mirror `stellar-indexer/src/transaction/abi.rs` exactly. The
//! `#[contracttype]` macro emits alphabetical `ScMap` key ordering, which is
//! what the off-chain builder also produces — so the bytes are bit-for-bit
//! compatible across the two crates.

use soroban_sdk::{contracttype, Address, Vec};

/// Which DEX/venue routes a given hop. Tag-only enum.
#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwapVenue {
    Soroswap,
    Aquarius,
    Phoenix,
    Sushi,
    CometDex,
}

/// Single hop in a path.
#[contracttype]
#[derive(Clone, Debug)]
pub struct SwapHop {
    /// Off-chain quoted output for this hop, informational only. Every venue —
    /// including Soroswap, whose pair requires the caller to name exact output
    /// amounts — derives the honored output on-chain at execution from live
    /// reserves/pool math, so this quoted value never drives the swap.
    pub amount_out: i128,
    pub pool: Address,
    pub token_in: Address,
    pub token_out: Address,
    pub venue: SwapVenue,
}

/// One path. `split_ppm` is parts-per-million of the amount entering path
/// allocation (`total_in` minus any input-side fee) allocated to this path;
/// the LAST path absorbs PPM rounding so the entire amount is consumed.
/// Hops chain output→input automatically.
#[contracttype]
#[derive(Clone, Debug)]
pub struct SwapPath {
    pub hops: Vec<SwapHop>,
    pub split_ppm: u32,
}

/// Complete opaque swap payload. Encoded as ScVal XDR bytes for
/// `execute_strategy` so callers do not need the router's hop/path/venue
/// types at their ABI boundary.
#[contracttype]
#[derive(Clone, Debug)]
pub struct StrategyPayload {
    pub paths: Vec<SwapPath>,
    /// Referral ID for fee attribution. `0` = no fee. `> 0` = static
    /// fee (admin) + referral fee (referral owner) charged on the output
    /// token only when it's whitelisted and the input token isn't;
    /// otherwise charged on input (see `is_whitelisted` and
    /// `apply_fees_on_token`).
    pub referral_id: u64,
    pub token_in: Address,
    pub token_out: Address,
    pub total_min_out: i128,
}

/// Referral metadata. `fee_bps` is the slice owed to the referral
/// owner; the contract's static fee (admin slice) is applied
/// independently and is read from instance storage.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct ReferralConfig {
    pub owner: Address,
    pub fee_bps: u32,
    pub active: bool,
}

/// Storage namespace for the router's persistent + instance keys.
#[contracttype]
#[derive(Clone, Debug)]
pub enum DataKey {
    /// Contract admin (instance). Set in the constructor.
    Admin,
    /// Static admin-side fee in basis points (instance).
    /// Applied alongside the referral fee whenever `referral_id > 0`
    /// resolves to an active referral.
    StaticFeeBps,
    /// Monotonic counter for assigning new referral IDs (instance).
    ReferralCounter,
    /// Per-referral configuration (persistent).
    Referral(u64),
    /// Whitelist of tokens (instance). When the swap's output token is
    /// whitelisted but the input token isn't, fees come out of the
    /// output side; otherwise fees come out of the input side. Stored
    /// as a single instance-storage `Vec<Address>` so the fee path
    /// reads ONE entry instead of two persistent entries per swap.
    /// Practical cap: a few dozen entries (instance storage is bounded).
    WhitelistedTokens,
    /// Accumulated admin fees per token (persistent). Claimable by admin.
    AdminFee(Address),
    /// Accumulated referral fees per (referral_id, token) (persistent).
    /// Claimed via `claim_referral_fees`; transferred to the
    /// `ReferralConfig.owner` at the time of claim.
    ReferralFee(u64, Address),
}
