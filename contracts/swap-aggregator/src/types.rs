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
    pub burn_pool: Option<Address>,

    pub burn_min_amounts: Vec<i128>,

    pub mint_pool: Option<Address>,
    pub mint_min_shares: i128,
    pub paths: Vec<SwapPath>,

    /// Caller-computed pre-swap that balances a lopsided mint against the
    /// pool's ratio. `pre_swap_amount <= 0` skips it. Solving for the optimal
    /// amount is the router's job off-chain; the contract only executes it and
    /// still enforces `mint_min_shares` and the residual allowance afterwards.
    pub pre_swap_amount: i128,
    pub pre_swap_from_a: bool,
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
