use soroban_sdk::contractevent;

#[contractevent(topics = ["debt", "bad_debt"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CleanBadDebtEvent {
    pub account_id: u64,

    pub total_borrow_usd_wad: i128,

    pub total_collateral_usd_wad: i128,
}
