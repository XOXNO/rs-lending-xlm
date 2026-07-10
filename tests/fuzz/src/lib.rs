//! Shared helpers for contract-level libFuzzer targets.

pub mod context;
pub mod decode;
pub mod invariants;

pub use context::{build_min_context, build_wide_context, LendingTest};
pub use decode::{
    amount_for_value, arb_amount, asset_price_usd, fraction, scaled_amount, HF_WAD_FLOOR,
};
pub use invariants::{
    assert_flash_guard_cleared, assert_pool_accounting, assert_state_preserved_on_failure,
    assert_user_health, snapshot, StateSnapshot,
};
pub use test_harness::{ALICE, BOB, LIQUIDATOR};
