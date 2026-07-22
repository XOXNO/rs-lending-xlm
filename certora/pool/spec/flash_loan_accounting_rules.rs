//! Flash-loan arithmetic used by the production balance checks and fee booking.
//!
//! The SAC transfer, allowance, callback, and rollback semantics are external
//! host boundaries; these rules prove the exact targets and persisted pool math
//! that the production endpoint uses around those calls.

use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume};
use soroban_sdk::{Address, Env};

use common::constants::{MAX_FLASHLOAN_FEE_BPS, RAY};
use common::math::fp::{Bps, Ray};
use common::rates::protocol_fee_shares;

use super::fixture::{
    hub, params, read_state, seed, state, ASSET_DECIMALS, MAX_FLOW_AMOUNT, ONE_TOKEN,
};

/// The exact post-payout and post-repayment balances recover principal plus fee.
#[rule]
fn flash_repayment_terms_recover_principal_and_fee(
    e: Env,
    amount: i128,
    fee_bps: u32,
    pre_balance: i128,
) {
    cvlr_assume!(amount > 0 && amount <= MAX_FLOW_AMOUNT);
    cvlr_assume!(i128::from(fee_bps) <= MAX_FLASHLOAN_FEE_BPS);
    cvlr_assume!(pre_balance >= amount && pre_balance <= 1_000 * ONE_TOKEN);

    let (fee, total, after_payout, after_repayment) =
        crate::flash_repayment_terms(&e, amount, fee_bps, pre_balance);
    let configured_fee = Bps::from(i128::from(fee_bps)).flash_loan_fee_on(&e, amount);

    cvlr_assert!(fee == configured_fee);
    cvlr_assert!(fee >= 0 && fee <= amount);
    cvlr_assert!(total == amount + fee);
    cvlr_assert!(after_payout == pre_balance - amount);
    cvlr_assert!(after_repayment == pre_balance + fee);
    cvlr_assert!(after_repayment - after_payout == total);
}

/// On a successful flash loan, fee booking changes cash by the native fee and
/// mints the same scaled shares into protocol revenue and aggregate supply.
#[rule]
fn flash_fee_booking_is_exact(
    e: Env,
    admin: Address,
    asset: Address,
    fee: i128,
    supply_index: i128,
) {
    cvlr_assume!(fee >= 0 && fee <= MAX_FLOW_AMOUNT);
    cvlr_assume!(supply_index >= RAY && supply_index <= 10 * RAY);
    seed(
        &e,
        admin,
        asset.clone(),
        params(asset.clone(), 50, true),
        state(
            100 * RAY,
            20 * RAY,
            5 * RAY,
            RAY,
            supply_index,
            80 * ONE_TOKEN,
            e.ledger().timestamp(),
        ),
    );

    let pre = read_state(&e, &asset);
    let expected_shares = protocol_fee_shares(
        &e,
        Ray::from_asset(fee, ASSET_DECIMALS),
        Ray::from(supply_index),
        Ray::from(pre.supplied),
    );
    let mut cache = crate::cache::Cache::load(&e, &hub(asset.clone()));
    crate::book_flash_fee(&mut cache, fee);
    cache.save();
    let post = read_state(&e, &asset);

    cvlr_assert!(post.revenue - pre.revenue == expected_shares.raw());
    cvlr_assert!(post.supplied - pre.supplied == expected_shares.raw());
    cvlr_assert!(post.cash - pre.cash == fee);
    cvlr_assert!(post.borrowed == pre.borrowed);
    cvlr_assert!(post.supply_index == pre.supply_index && post.borrow_index == pre.borrow_index);
}
