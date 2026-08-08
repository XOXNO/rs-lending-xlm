//! XDR wire types, referral config, and storage keys.

use soroban_sdk::{contracttype, Address, Vec};

/// DEX venue selected for a single hop.
#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwapVenue {
    Soroswap,
    Aquarius,
    Phoenix,
    Sushi,
    CometDex,
}

/// One pool hop: pull `token_in`, push `token_out` via `venue`.
#[contracttype]
#[derive(Clone, Debug)]
pub struct SwapHop {
    pub pool: Address,
    pub token_in: Address,
    pub token_out: Address,
    pub venue: SwapVenue,
}

/// Ordered hops plus a share of the token-group input (`split_ppm` / 1e6).
#[contracttype]
#[derive(Clone, Debug)]
pub struct SwapPath {
    pub hops: Vec<SwapHop>,
    pub split_ppm: u32,
}

/// Full strategy decoded from `execute_strategy` XDR.
#[contracttype]
#[derive(Clone, Debug)]
pub struct StrategyPayload {
    /// Optional Aquarius pool to burn LP (`token_in`) into constituents.
    pub burn_pool: Option<Address>,
    /// Per-constituent mins for the burn leg (same order as pool tokens).
    pub burn_min_amounts: Vec<i128>,
    /// Optional Aquarius pool to mint LP as `token_out`.
    pub mint_pool: Option<Address>,
    /// Minimum LP shares for the mint leg.
    pub mint_min_shares: i128,
    /// Swap paths between optional burn and mint.
    pub paths: Vec<SwapPath>,
    /// Off-chain sized pre-swap amount for mint balancing; `<= 0` skips it.
    pub pre_swap_amount: i128,
    /// Pre-swap direction: true = token A → B in the pool token list.
    pub pre_swap_from_a: bool,
    /// Referral id for fee routing; `0` disables referral fees.
    pub referral_id: u64,
    pub token_in: Address,
    pub token_out: Address,
    /// Aggregate minimum output delivered to the sender.
    pub total_min_out: i128,
}

/// Stored referral account.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct ReferralConfig {
    pub owner: Address,
    pub fee_bps: u32,
    pub active: bool,
}

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
