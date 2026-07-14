//! Controller contract integration tests.

extern crate std;

mod account;
mod admin;
mod admin_config;
mod bad_debt_index;
mod borrow;
mod bulk_indexes;
mod decimal_diversity;
mod events;
mod flash_loan;
mod keeper;
mod limits;
mod liquidation;
mod liquidation_boundary;
mod liquidation_coverage;
mod liquidation_extreme;
mod liquidation_math;
mod liquidation_mixed_decimal;
mod liquidation_ratchet;
mod max_utilization;
mod min_borrow_collateral;
mod multi_hub;
mod ownership;
mod repay;
mod spoke;
mod spoke_liquidation_combo;

mod supply;
mod validation_admin;
mod views;
mod withdraw;
