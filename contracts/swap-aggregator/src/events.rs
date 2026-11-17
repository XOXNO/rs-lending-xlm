//! Contract events published by the router.

use soroban_sdk::{contractevent, Address};

/// Event published once per successful `execute_strategy`, after the payout.
///
/// Indexers use it for per-sender and per-router swap volume. Amounts are raw
/// token units. `amount_in` is the measured input the router received;
/// `amount_out` is what the router sent to `sender`, after fees. Fees are
/// charged on `fee_token`, which is `token_in` or `token_out`; both fee fields
/// are zero when the swap pays no fee.
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
}
