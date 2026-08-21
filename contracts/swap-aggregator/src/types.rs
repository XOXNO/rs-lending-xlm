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
///
/// A `#[contracttype]` enum with data-carrying variants is encoded as
/// `ScVal::Vec([Symbol("<variant name>"), ..fields])`, so a key's on-ledger
/// bytes depend on the variant *name*, never on its position. New variants may
/// therefore be appended without disturbing any existing entry — but a rename
/// silently orphans every entry already written under the old name.
#[contracttype]
#[derive(Clone, Debug)]
pub enum DataKey {
    StaticFeeBps,
    ReferralCounter,
    Referral(u64),
    WhitelistedTokens,
    AdminFee(Address),
    ReferralFee(u64, Address),
    /// Sum of every fee bucket denominated in this token: the admin bucket plus
    /// each referral bucket. Only `storage::accumulate_fee`,
    /// `storage::accumulate_swap_fees`, `storage::take_fee_bucket` and
    /// `storage::rebuild_reserved_total` maintain this counter, so
    /// `sweep_balance` never has to walk the referral space.
    ReservedTotal(Address),
}
