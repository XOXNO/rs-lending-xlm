//! Contract events published by the router.

use soroban_sdk::{contractevent, Address, Map};

/// Published once per successful swap, after the payout.
///
/// Raw token units. `amount_in` is measured; `amount_out` is after fees.
/// `residuals` maps each leftover token swept to admin revenue to its amount.
#[contractevent(topics = ["router", "swap"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SwapEvent {
    pub sender: Address,
    pub token_in: Address,
    pub token_out: Address,
    pub amount_in: i128,
    pub amount_out: i128,
    pub referral_id: u64,
    pub fee_token: Address,
    pub static_fee: i128,
    pub referral_fee: i128,
    pub residuals: Map<Address, i128>,
}
