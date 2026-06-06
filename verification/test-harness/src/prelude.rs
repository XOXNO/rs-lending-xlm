//! Convenient re-exports for integration tests.

pub use crate::assert::assert_contract_error;
pub use crate::core::{AccountEntry, LendingTest, MarketState, UserState};
/// Stable `u32` error constants (`INSUFFICIENT_COLLATERAL`, etc.).
pub use crate::errors;
pub use crate::fixtures::{
    liquidatable_usdc_eth, seed_fuzz_conservation_book, seed_liquidatable_usdc_eth,
    seed_liquidator_usdc, seed_standard_liquidity,
};
pub use crate::helpers::{self, units::*};
pub use crate::ops::internal::{amount_raw, asset_payment_vec};
pub use crate::oracle::config::*;
pub use crate::presets::*;
pub use crate::setup::LendingTestBuilder;
pub use crate::strategy::{
    apply_flash_fee, build_aggregator_swap, mock_swap_payload_xdr, MockSwapPayload,
    DEFAULT_FLASHLOAN_FEE_BPS,
};
pub use crate::view::PositionType;