#![cfg(feature = "reference-math")]

//! Test-only exact-arithmetic references.

pub mod liquidation;

pub use liquidation::{
    bigrational_to_i128_half_up, bigrational_to_i128_wad, compute_liquidation,
    float_to_bigrational, half_up_div, snapshot_collateral, snapshot_debt, RefCollateralPosition,
    RefDebtPosition, RefLiquidationResult,
};
