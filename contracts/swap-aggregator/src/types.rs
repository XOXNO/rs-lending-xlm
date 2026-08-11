//! Wire types, internal hop shape, referral config, and storage keys.
//!
//! The wire payload holds two registries plus one packed instruction stream.
//! See [`crate::program`] for the byte layout.

use soroban_sdk::{contracttype, Address, Bytes, Vec};

/// DEX venue selected for a single hop.
///
/// Not a wire type — venues travel as the opcode byte of an instruction and are
/// resolved by [`crate::program::Opcode`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwapVenue {
    Soroswap,
    Aquarius,
    Phoenix,
    Sushi,
    CometDex,
}

/// One pool hop: pull `token_in`, push `token_out` via `venue`.
///
/// Built per instruction from registry indices; venue adapters consume this.
#[derive(Clone, Debug)]
pub struct SwapHop {
    pub pool: Address,
    pub token_in: Address,
    pub token_out: Address,
    pub venue: SwapVenue,
}

/// Full strategy decoded from `execute_strategy` XDR.
///
/// Instructions reference `assets` and `amounts` by `u8` index, so an address
/// or amount used by several hops is carried exactly once.
#[contracttype]
#[derive(Clone, Debug)]
pub struct StrategyPayload {
    /// Amount registry: min-out, fixed inputs, burn floors, mint min-shares.
    pub amounts: Vec<i128>,
    /// Address registry: tokens, pools, and LP share tokens.
    pub assets: Vec<Address>,
    /// Packed program: header, instruction records, split weights.
    pub ops: Bytes,
}

/// Stored referral account configuration, re-exported from
/// `swap_aggregator_interface`.
pub use swap_aggregator_interface::ReferralConfig;

/// Instance and persistent storage keys.
#[contracttype]
#[derive(Clone, Debug)]
pub enum DataKey {
    StaticFeeBps,
    ReferralCounter,
    Referral(u64),
    WhitelistedTokens,
    AdminFee(Address),
    ReferralFee(u64, Address),
}
