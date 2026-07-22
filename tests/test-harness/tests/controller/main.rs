//! Controller contract integration tests.

extern crate std;

mod account;
mod admin;
mod admin_config;
mod audit_borrow_withdraw_liquidate_stale_anchor_blend;
mod audit_liquidate_and_clean_stale_leg;
mod audit_liquidate_dust_fee_dos;
mod audit_liquidate_subunit_leg_brick;
mod audit_supply_stale_shield;
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

mod security_audit;
mod security_audit_extended;
mod supply;
mod validation_admin;
mod views;
mod withdraw;
