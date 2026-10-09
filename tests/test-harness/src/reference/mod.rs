#![cfg(feature = "reference-math")]

mod liquidation;

pub use liquidation::{
    bigrational_to_i128_half_up, bigrational_to_i128_wad, bonus_scale_exact,
    calculate_linear_bonus_with_target_exact, compute_liquidation, compute_liquidation_with_curve,
    estimate_liquidation_amount_exact, exact_totals, float_to_bigrational, half_up_div,
    max_bonus_for_threshold_exact, max_hf_preserving_bonus_bps_exact, plan_liquidation_exact,
    snapshot_collateral, snapshot_debt, snapshot_exact, ExactBook, ExactCollateral, ExactDebt,
    ExactPlan, ExactSeizure, ExactTotals, RefCollateralPosition, RefCurve, RefDebtPosition,
    RefLiquidationResult,
};
