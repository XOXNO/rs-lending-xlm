//! `contracts/controller/src/risk/`: `totals.rs` (generated from production
//! by `../../gen_totals.py`: verbatim except the `fn` pointer of
//! `sum_debt_usd_loaded`), with the re-exports of the production
//! `risk/mod.rs` that the included files use. `validation.rs` is a
//! signature-only stand-in; `params.rs` is not included.

pub(crate) mod totals;
pub(crate) mod validation;

pub(crate) use totals::{
    account_price_assets, calculate_account_risk_totals, calculate_ltv_collateral_wad,
    position_value, sum_debt_usd, AccountRiskTotals,
};
