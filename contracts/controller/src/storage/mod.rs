//! Typed account, hub, protocol, and spoke storage accessors.
//! Generic key-value helpers stay private to this module. Certora adds
//! verification accessors through `verification_storage`.

mod account;
mod hub;
mod protocol;
mod spoke;
#[cfg(feature = "certora")]
#[path = "../../../../certora/controller/harness/storage.rs"]
mod verification_storage;

pub(crate) use account::{
    account_from_parts, account_owner, add_delegate, get_account, get_account_borrow_only,
    get_account_meta, get_debt_positions, get_delegates, get_supply_positions, iter_debt_positions,
    iter_typed_positions, remove_account_entry, remove_delegate, renew_user_account,
    set_account_meta, set_debt_positions, set_supply_positions, try_account_owner, try_get_account,
    try_get_account_meta, try_get_debt_position, try_get_supply_position,
};
pub(crate) use account::{is_flash_loan_ongoing, with_flash_guard};
pub(crate) use hub::{get_hub, increment_hub_id, set_hub};
pub(crate) use protocol::renew_controller_instance;
pub(crate) use protocol::{
    get_min_borrow_collateral_usd_wad, get_pool, get_position_limits, get_position_manager,
    get_position_nft, get_price_aggregator, get_swap_aggregator, is_blend_pool_approved,
    set_accumulator, set_blend_pool_approved, set_min_borrow_collateral_usd_wad, set_pool,
    set_position_limits, set_position_manager, set_position_nft, set_price_aggregator,
    set_swap_aggregator, try_get_accumulator, try_get_pool, try_get_position_nft,
};
pub(crate) use spoke::{
    get_spoke, get_spoke_asset, get_spoke_usage, increment_spoke_id, remove_spoke_asset, set_spoke,
    set_spoke_asset, set_spoke_usage,
};

#[cfg(any(test, feature = "testing", feature = "certora"))]
pub(crate) use account::set_flash_loan_ongoing;
#[cfg(feature = "certora")]
pub(crate) use spoke::try_get_spoke;
#[cfg(feature = "certora")]
pub(crate) use verification_storage::*;
