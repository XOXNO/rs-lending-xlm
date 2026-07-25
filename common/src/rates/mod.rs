//! Interest-rate and scaled-share math shared by the pool and the controller.
//!
//! Split by concern, and by consumer:
//!
//! - [`curve`], [`compound`], [`index`], [`simulate`] are the accrual model.
//!   Only the pool calls these in production; they live here so the `common`
//!   Certora artifact can prove them without linking a contract.
//! - [`scaling`] is a cross-contract surface. The controller's liquidation math
//!   and limit views call into it, so a change there moves both contracts at
//!   once — which is the point: a private copy on either side would let the
//!   position map and the pool's books drift apart.
//!
//! Rates and indexes are RAY (`1e27`); reserve factor is BPS. Accrual chunks at
//! most one year (`MAX_COMPOUND_DELTA_MS`). See `docs/reference/invariants.md`.

#[cfg(test)]
#[path = "../../tests/rates/support.rs"]
pub(crate) mod test_support;

pub mod compound;
pub mod curve;
pub mod index;
pub mod scaling;
pub mod simulate;

pub use compound::{compound_interest, MAX_COMPOUND_DELTA_MS};
pub use curve::{calculate_borrow_rate, calculate_deposit_rate, utilization};
pub use index::{
    calculate_supplier_rewards, protocol_fee_shares, supply_index_reward_shortfall,
    update_borrow_index, update_supply_index,
};
pub use scaling::{
    calculate_scaled_borrow, calculate_scaled_borrow_floor, calculate_scaled_supply,
    calculate_scaled_supply_ceil, resolve_repay, resolve_withdrawal, scaled_to_original,
    unscale_borrow, unscale_borrow_ceil, unscale_borrow_ceil_ray, unscale_supply,
    unscale_supply_floor,
};
pub use simulate::simulate_update_indexes;

#[cfg(feature = "certora")]
pub(crate) use simulate::simulate_update_indexes_body;
