//! Reference implementations of protocol math using exact rational arithmetic
//! (`num_rational::BigRational`) for differential testing against the
//! production i128-backed half-up implementations.
//!
//! These modules are test-only; the production path never calls into here.

pub mod liquidation;

pub use liquidation::{
    bigrational_to_i128_half_up, bigrational_to_i128_wad, compute_liquidation,
    float_to_bigrational, half_up_div, snapshot_collateral, snapshot_debt, RefCollateralPosition,
    RefDebtPosition, RefLiquidationResult,
};
