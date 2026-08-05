use soroban_sdk::{contracttype, Address, Vec};

#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwapVenue {
    Soroswap,
    Aquarius,
    Phoenix,
    Sushi,
    CometDex,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct SwapHop {
    pub pool: Address,
    pub token_in: Address,
    pub token_out: Address,
    pub venue: SwapVenue,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct SwapPath {
    pub hops: Vec<SwapHop>,
    pub split_ppm: u32,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct StrategyPayload {
    /// Aquarius pool whose LP shares are burned before routing. `token_in` is
    /// the share token, and is checked against the pool's own `share_id`.
    pub burn_pool: Option<Address>,
    /// Per-constituent floor for the burn, in the pool's token order.
    pub burn_min_amounts: Vec<i128>,
    /// Aquarius pool that mints `token_out` from the routed constituents after
    /// routing. `token_out` is checked against the pool's own `share_id`.
    pub mint_pool: Option<Address>,
    pub mint_min_shares: i128,
    pub paths: Vec<SwapPath>,
    /// Swap fee of the mint pool, in bps, when a constant-product deposit
    /// should be pre-balanced on-chain; `0` skips pre-balancing (stable
    /// pools, plain swaps). Carried in the payload because reading the kind
    /// and fee from the pool costs two extra cross-contract calls, and the
    /// per-call VM-instantiation memory wall (measured: the 8th call into
    /// the pool trips `Budget(ExceededLimit)`) leaves room for exactly one —
    /// the pre-swap itself. A wrong hint only shifts the bisection target:
    /// settlement stays measured-delta and floor-guarded.
    pub pre_balance_fee_bps: u32,
    pub referral_id: u64,
    pub token_in: Address,
    pub token_out: Address,
    pub total_min_out: i128,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct ReferralConfig {
    pub owner: Address,
    pub fee_bps: u32,
    pub active: bool,
}

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
