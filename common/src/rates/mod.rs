//! Interest-rate curve, compounding, index-update, and unit-scaling
//! primitives used to accrue and account for pool interest.

#[cfg(test)]
#[path = "../../tests/rates/support.rs"]
pub(crate) mod test_support;

pub mod compound;
pub mod curve;
pub mod index;
pub mod scaling;
pub mod simulate;
pub mod value;

pub use compound::{compound_interest, MAX_COMPOUND_DELTA_MS};
pub use curve::{
    calculate_annual_borrow_rate, calculate_borrow_rate, calculate_deposit_rate, utilization,
};
pub use index::{
    calculate_supplier_rewards, protocol_fee_shares, supply_index_reward_shortfall,
    update_borrow_index, update_supply_index,
};
pub use scaling::{
    calculate_scaled_borrow, calculate_scaled_borrow_floor, calculate_scaled_cap,
    calculate_scaled_supply, calculate_scaled_supply_ceil, resolve_net_settle, resolve_repay,
    resolve_withdrawal, scaled_to_original, unscale_borrow, unscale_borrow_ceil,
    unscale_borrow_ceil_ray, unscale_supply, unscale_supply_floor,
};
pub use simulate::simulate_update_indexes;
pub use value::{position_value, position_value_ceil, position_value_floor};

#[cfg(feature = "certora")]
pub(crate) use simulate::simulate_update_indexes_body;
