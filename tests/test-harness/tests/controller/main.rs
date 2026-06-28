//! Controller contract integration tests.

extern crate std;

mod account;
mod admin;
mod admin_config;
mod bad_debt_index;
mod borrow;
mod bulk_indexes;
mod decimal_diversity;
mod emode;
mod emode_liquidation_combo;
mod events;
mod flash_loan;
mod keeper;
mod limits;
mod liquidation;
mod liquidation_boundary;
mod liquidation_coverage;
mod liquidation_math;
mod liquidation_mixed_decimal;
mod liquidation_ratchet;
mod max_utilization;
mod min_borrow_collateral;
mod multi_hub;
mod ownership;
mod repay;

mod supply;
mod validation_admin;
mod views;
mod withdraw;
