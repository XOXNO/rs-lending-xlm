use soroban_sdk::{contracttype, Address, Vec};

#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwapVenue {
    Soroswap,
    Aquarius,
    Phoenix,
    NativeAmm,
    StaticBridge,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct SwapHop {
    pub fee_bps: u32,
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
pub struct AggregatorSwap {
    pub paths: Vec<SwapPath>,
    pub total_min_out: i128,
}

pub type SwapSteps = AggregatorSwap;

#[contracttype]
#[derive(Clone, Debug)]
pub struct BatchSwap {
    pub paths: Vec<SwapPath>,
    pub referral_id: u64,
    pub sender: Address,
    pub total_in: i128,
    pub total_min_out: i128,
}
