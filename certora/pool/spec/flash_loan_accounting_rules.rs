use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume};
use soroban_sdk::{Address, Env};

use common::constants::{
    BPS, MAX_FLASHLOAN_FEE_BPS, MAX_SUPPLY_INDEX_RAY, RAY, SUPPLY_INDEX_FLOOR_RAW,
};
use common::math::fp::Ray;
use common::math::fp_core;

use super::fixture::{
    expected_protocol_fee_shares, hub, params, read_state, seed, state, ASSET_DECIMALS,
    MAX_FLOW_AMOUNT, ONE_TOKEN,
};
use crate::ops::flash::FlashTerms;

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

    let FlashTerms {
        fee,
        total_repayment: total,
        balance_after_payout: after_payout,
        balance_after_repayment: after_repayment,
    } = crate::ops::flash::terms(&e, amount, fee_bps, pre_balance);
    let rounded_fee = fp_core::mul_div_half_up(&e, amount, i128::from(fee_bps), BPS);
    let configured_fee = if fee_bps > 0 && rounded_fee == 0 {
        1
    } else {
        rounded_fee
    };

    cvlr_assert!(fee == configured_fee);
    cvlr_assert!(fee >= 0 && fee <= amount);
    cvlr_assert!(total == amount + fee);
    cvlr_assert!(after_payout == pre_balance - amount);
    cvlr_assert!(after_repayment == pre_balance + fee);
    cvlr_assert!(after_repayment - after_payout == total);
}

#[rule]
fn flash_fee_booking_is_exact(
    e: Env,
    admin: Address,
    asset: Address,
    fee: i128,
    supply_index: i128,
) {
    cvlr_assume!(fee >= 0 && fee <= MAX_FLOW_AMOUNT);
    cvlr_assume!(supply_index >= SUPPLY_INDEX_FLOOR_RAW && supply_index <= MAX_SUPPLY_INDEX_RAY);
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
    let expected_shares = expected_protocol_fee_shares(
        &e,
        Ray::from_asset(fee, ASSET_DECIMALS),
        Ray::from(supply_index),
        Ray::from(pre.supplied),
    );
    cvlr_assert!(
        expected_shares.mul_floor(&e, Ray::from(supply_index)).raw()
            <= Ray::from_asset(fee, ASSET_DECIMALS).raw()
    );
    let mut cache = crate::cache::Cache::load(&e, &hub(asset.clone()));
    crate::ops::flash::book_fee(&mut cache, fee);
    cache.commit();
    let post = read_state(&e, &asset);

    cvlr_assert!(post.revenue - pre.revenue == expected_shares.raw());
    cvlr_assert!(post.supplied - pre.supplied == expected_shares.raw());
    cvlr_assert!(post.cash - pre.cash == fee);
    cvlr_assert!(post.borrowed == pre.borrowed);
    cvlr_assert!(post.supply_index == pre.supply_index && post.borrow_index == pre.borrow_index);
}
