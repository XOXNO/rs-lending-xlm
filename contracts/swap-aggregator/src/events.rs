//! Contract events published by the router.

use soroban_sdk::{contractevent, Address};

/// Published once per successful swap, after the payout.
///
/// Raw token units. `amount_in` is measured; `amount_out` is after fees.
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
    /// Protocol revenue in `fee_token`, including any fixed-output surplus.
    pub static_fee: i128,
    pub referral_fee: i128,
}
