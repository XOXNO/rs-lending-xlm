use soroban_sdk::Vec;
use test_harness::{
    assert_contract_error, errors, eth_preset, usdc_preset, FlashPositionMode,
    FlashPositionRequest, LendingTest, ALICE, HARNESS_SPOKE,
};

use crate::helpers::{collaterals, data, usdc_raw, AliceOps};

fn request(
    t: &LendingTest,
    mode: FlashPositionMode,
    collateral_amount: f64,
) -> FlashPositionRequest {
    FlashPositionRequest {
        mode,
        collateral: t.resolve_asset("USDC"),
        collateral_amount: usdc_raw(t, collateral_amount),
        extra_asset: t.resolve_asset("ETH"),
        extra_amount: 0,
        reenter_spoke_id: HARNESS_SPOKE,
        reenter_account_id: 0,
    }
}

fn assert_reentry_fails(t: &mut LendingTest, mode: FlashPositionMode) {
    let receiver = t.deploy_flash_position_receiver();
    let payload = data(t, request(t, mode, 4_000.0));
    let mins = collaterals(t, &[("USDC", 4_000.0)]);
    let result = t.try_alice_eth_flash(&receiver, &payload, &mins, &Vec::new(&t.env));
    assert!(
        result.is_err(),
        "malicious reentry {mode:?} must revert: {result:?}"
    );
    t.env.as_contract(&t.controller, || {
        assert!(
            !controller::test_support::is_flash_loan_ongoing(&t.env),
            "flash guard must clear after {mode:?} rollback"
        );
    });
}

#[test]
fn test_flash_position_reenter_flash_loan_rejects() {
    let mut t = LendingTest::new().standard_two_asset().build();
    assert_reentry_fails(&mut t, FlashPositionMode::ReenterFlashLoan);
}

#[test]
fn test_flash_position_reenter_flash_position_rejects() {
    let mut t = LendingTest::new().standard_two_asset().build();
    assert_reentry_fails(&mut t, FlashPositionMode::ReenterFlashPosition);
}

#[test]
fn test_flash_position_reenter_borrow_rejects() {
    let mut t = LendingTest::new().standard_two_asset().build();
    assert_reentry_fails(&mut t, FlashPositionMode::ReenterBorrow);
}

#[test]
fn test_flash_position_reenter_withdraw_rejects() {
    let mut t = LendingTest::new().standard_two_asset().build();
    assert_reentry_fails(&mut t, FlashPositionMode::ReenterWithdraw);
}

#[test]
fn test_flash_position_reenter_repay_rejects() {
    let mut t = LendingTest::new().standard_two_asset().build();
    assert_reentry_fails(&mut t, FlashPositionMode::ReenterRepay);
}

#[test]
fn test_flash_position_fee_on_transfer_debt_fails_closed() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_fee_on_transfer_market(eth_preset(), 100)
        .build();
    let receiver = t.deploy_flash_position_receiver();
    let payload = data(&t, request(&t, FlashPositionMode::Success, 4_000.0));
    let mins = collaterals(&t, &[("USDC", 4_000.0)]);
    let result = t.try_alice_eth_flash(&receiver, &payload, &mins, &Vec::new(&t.env));
    assert_contract_error(result, errors::INTERNAL_ERROR);
}

#[test]
fn test_flash_position_fee_on_transfer_collateral_misses_min() {
    let mut t = LendingTest::new()
        .with_fee_on_transfer_market(usdc_preset(), 100)
        .with_market(eth_preset())
        .build();
    let receiver = t.deploy_flash_position_receiver();
    let payload = data(&t, request(&t, FlashPositionMode::Success, 4_000.0));
    let mins = collaterals(&t, &[("USDC", 4_000.0)]);
    let result = t.try_alice_eth_flash(&receiver, &payload, &mins, &Vec::new(&t.env));
    assert_contract_error(result, errors::COLLATERAL_MINIMUM_NOT_MET);
}

#[test]
fn test_flash_position_fee_on_transfer_collateral_credits_net() {
    let mut t = LendingTest::new()
        .with_fee_on_transfer_market(usdc_preset(), 100)
        .with_market(eth_preset())
        .build();
    let receiver = t.deploy_flash_position_receiver();
    let payload = data(&t, request(&t, FlashPositionMode::Success, 4_000.0));
    // 1% shortfall on 4000 → 3960 delivered.
    let mins = collaterals(&t, &[("USDC", 3_960.0)]);
    let account_id = t
        .try_alice_eth_flash(&receiver, &payload, &mins, &Vec::new(&t.env))
        .expect("net receipt meets the lowered min");
    let supply = t.supply_balance_for(ALICE, account_id, "USDC");
    // Controller snapshot sees 4000*0.99; the subsequent pool transfer
    // applies the same shortfall again, so shares track ~4000*0.99².
    assert!(
        (3_919.0..=3_922.0).contains(&supply),
        "measured net USDC supply got {supply}"
    );
}

#[test]
fn test_flash_position_extra_credit_is_measured_not_pool_theft() {
    let mut t = LendingTest::new()
        .with_extra_credit_market(usdc_preset(), 100)
        .with_market(eth_preset())
        .build();
    let eth_cash_before = t.pool_reserves("ETH");
    let receiver = t.deploy_flash_position_receiver();
    let payload = data(&t, request(&t, FlashPositionMode::Success, 4_000.0));
    let mins = collaterals(&t, &[("USDC", 4_000.0)]);
    let account_id = t
        .try_alice_eth_flash(&receiver, &payload, &mins, &Vec::new(&t.env))
        .expect("extra credit still meets min");
    let supply = t.supply_balance_for(ALICE, account_id, "USDC");
    // Extra bps apply on the push *and* the pool deposit hop (~4000*1.01²).
    assert!(
        (4_079.0..=4_082.0).contains(&supply),
        "1% extra credit must be measured in, got {supply}"
    );
    let eth_cash_after = t.pool_reserves("ETH");
    let borrowed = t.borrow_balance_for(ALICE, account_id, "ETH");
    assert!((0.99..=1.01).contains(&borrowed));
    assert!(
        eth_cash_before - eth_cash_after > 0.0,
        "ETH cash must fall by the borrowed amount, not by the extra USDC air"
    );
}

#[test]
fn test_flash_position_transfer_hook_cannot_reenter() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_transfer_hook_market(eth_preset())
        .build();
    let receiver = t.deploy_flash_position_receiver();
    let payload = data(&t, request(&t, FlashPositionMode::Success, 4_000.0));
    let mins = collaterals(&t, &[("USDC", 4_000.0)]);
    let result = t.try_alice_eth_flash(&receiver, &payload, &mins, &Vec::new(&t.env));
    assert!(
        result.is_err(),
        "debt-token transfer hook must not reenter: {result:?}"
    );
    t.env.as_contract(&t.controller, || {
        assert!(
            !controller::test_support::is_flash_loan_ongoing(&t.env),
            "flash guard must clear after hook rollback"
        );
    });
}

/// `refund_assets` is caller-supplied and reaches `token::Client::balance` and
/// `transfer` after `with_flash_guard` has closed, while the invocation still
/// holds an unpersisted spoke-usage snapshot that `strategy_finalize` writes
/// back absolutely. An unlisted address there is an arbitrary contract the
/// controller invokes with reentrancy protection off. Only listed assets may
/// appear. A WeirdToken is used rather than an arbitrary contract because a
/// non-token merely errors on the missing `balance`, which is an accident of
/// shape, not a check.
#[test]
fn test_flash_position_rejects_unlisted_refund_asset() {
    let mut t = LendingTest::new().standard_two_asset().build();
    let receiver = t.deploy_flash_position_receiver();
    let payload = data(&t, request(&t, FlashPositionMode::Success, 4_000.0));
    let mins = collaterals(&t, &[("USDC", 4_000.0)]);

    // A real token contract, listed on no spoke: what an attacker supplies.
    let rogue = t.env.register(test_harness::weird_token::WeirdToken, ());
    let mut refunds = Vec::new(&t.env);
    refunds.push_back(rogue);

    let result = t.try_alice_eth_flash(&receiver, &payload, &mins, &refunds);

    assert_contract_error(result, errors::ASSET_NOT_IN_SPOKE);
}
