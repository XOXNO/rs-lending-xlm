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
    // Bounds the clock so the checked `timestamp * 1_000` in `fixture::state` and
    // `time::now_ms` cannot overflow. Sunbeam prunes that panic path silently.
    cvlr_assume!(e.ledger().timestamp() <= u64::MAX / 1_000);
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
        Ray::from_asset(&e, fee, ASSET_DECIMALS),
        Ray::from(supply_index),
        Ray::from(pre.supplied),
    );
    cvlr_assert!(
        expected_shares.mul_floor(&e, Ray::from(supply_index)).raw()
            <= Ray::from_asset(&e, fee, ASSET_DECIMALS).raw()
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

/// Proves the successful-path accounting of `flash::apply`: `prepare_with_balance`
/// (gates and terms), then `book_fee` and `commit`, which is `finalize` without its
/// event. The token transfers and the receiver callback are not modeled.
///
/// The fee enters the cash book; the principal does not.
#[rule]
fn flash_apply_accounting_books_fee_without_principal_cash(
    e: Env,
    admin: Address,
    asset: Address,
    amount: i128,
    fee_bps: u32,
    pre_balance: i128,
    supply_index: i128,
) {
    cvlr_assume!(e.ledger().timestamp() <= u64::MAX / 1_000);
    cvlr_assume!(amount > 0 && amount <= MAX_FLOW_AMOUNT);
    cvlr_assume!(i128::from(fee_bps) <= MAX_FLASHLOAN_FEE_BPS);
    cvlr_assume!(pre_balance >= amount && pre_balance <= 1_000 * ONE_TOKEN);
    cvlr_assume!(supply_index >= SUPPLY_INDEX_FLOOR_RAW && supply_index <= MAX_SUPPLY_INDEX_RAY);

    // `prepare` calls `require_reserves`: cash must cover the principal.
    let cash = 200 * ONE_TOKEN;
    cvlr_assume!(cash >= amount);

    seed(
        &e,
        admin,
        asset.clone(),
        params(asset.clone(), fee_bps, true),
        state(
            100 * RAY,
            20 * RAY,
            5 * RAY,
            RAY,
            supply_index,
            cash,
            e.ledger().timestamp(),
        ),
    );

    let pre = read_state(&e, &asset);
    let (mut cache, terms) =
        crate::ops::flash::prepare_with_balance(&e, hub(asset.clone()), amount, pre_balance);

    let rounded_fee = fp_core::mul_div_half_up(&e, amount, i128::from(fee_bps), BPS);
    let configured_fee = if fee_bps > 0 && rounded_fee == 0 {
        1
    } else {
        rounded_fee
    };
    cvlr_assert!(terms.fee == configured_fee);
    cvlr_assert!(terms.total_repayment == amount + terms.fee);
    cvlr_assert!(terms.balance_after_payout == pre_balance - amount);
    cvlr_assert!(terms.balance_after_repayment == pre_balance + terms.fee);

    let expected_shares = expected_protocol_fee_shares(
        &e,
        Ray::from_asset(&e, terms.fee, ASSET_DECIMALS),
        Ray::from(supply_index),
        Ray::from(pre.supplied),
    );

    crate::ops::flash::book_fee(&mut cache, terms.fee);
    cache.commit();
    let post = read_state(&e, &asset);

    cvlr_assert!(post.cash - pre.cash == terms.fee);
    cvlr_assert!(post.revenue - pre.revenue == expected_shares.raw());
    cvlr_assert!(post.supplied - pre.supplied == expected_shares.raw());
    cvlr_assert!(post.borrowed == pre.borrowed);
    cvlr_assert!(post.supply_index == pre.supply_index);
    cvlr_assert!(post.borrow_index == pre.borrow_index);
    // The fixture stamps `last_timestamp` at the current time, so the
    // `global_sync` in `prepare` does not accrue.
    cvlr_assert!(post.last_timestamp == pre.last_timestamp);
}

/// Proves that a flash loan on a flash-enabled market with `flashloan_fee == 0`
/// charges no fee, requires a repayment equal to the principal, and leaves cash,
/// revenue, supplied and borrowed unchanged after `book_fee` and `commit`.
#[rule]
fn flash_apply_accounting_zero_fee_is_cash_noop(
    e: Env,
    admin: Address,
    asset: Address,
    amount: i128,
    pre_balance: i128,
) {
    cvlr_assume!(e.ledger().timestamp() <= u64::MAX / 1_000);
    cvlr_assume!(amount > 0 && amount <= MAX_FLOW_AMOUNT);
    cvlr_assume!(pre_balance >= amount && pre_balance <= 1_000 * ONE_TOKEN);
    let cash = 200 * ONE_TOKEN;
    cvlr_assume!(cash >= amount);

    seed(
        &e,
        admin,
        asset.clone(),
        params(asset.clone(), 0, true),
        state(
            100 * RAY,
            20 * RAY,
            5 * RAY,
            RAY,
            RAY,
            cash,
            e.ledger().timestamp(),
        ),
    );

    let pre = read_state(&e, &asset);
    let (mut cache, terms) =
        crate::ops::flash::prepare_with_balance(&e, hub(asset.clone()), amount, pre_balance);
    cvlr_assert!(terms.fee == 0);
    cvlr_assert!(terms.total_repayment == amount);
    cvlr_assert!(terms.balance_after_repayment == pre_balance);

    crate::ops::flash::book_fee(&mut cache, terms.fee);
    cache.commit();
    let post = read_state(&e, &asset);

    cvlr_assert!(post.cash == pre.cash);
    cvlr_assert!(post.revenue == pre.revenue);
    cvlr_assert!(post.supplied == pre.supplied);
    cvlr_assert!(post.borrowed == pre.borrowed);
}
