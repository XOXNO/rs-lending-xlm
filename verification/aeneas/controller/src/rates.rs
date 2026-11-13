//! The production rate formulas, verbatim, with the re-exports of
//! `common/src/rates/mod.rs`. Only `scaling.rs` and `value.rs` are included;
//! the controller liquidation math calls nothing else in `rates`.

#[path = "../../../../common/src/rates/scaling.rs"]
pub mod scaling;
#[path = "../../../../common/src/rates/value.rs"]
pub mod value;

pub use scaling::{
    calculate_scaled_borrow, calculate_scaled_borrow_floor, calculate_scaled_cap,
    calculate_scaled_supply, calculate_scaled_supply_ceil, resolve_net_settle, resolve_repay,
    resolve_withdrawal, scaled_to_original, unscale_borrow, unscale_borrow_ceil, unscale_supply,
    unscale_supply_floor,
};
pub use value::{position_value, position_value_ceil, position_value_floor};
