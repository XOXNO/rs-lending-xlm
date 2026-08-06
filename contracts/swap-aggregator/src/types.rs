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
