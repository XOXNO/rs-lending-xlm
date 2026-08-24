use crate::shared::{flash_fee, flash_guard_cleared, raw_units, receiver_data, strict_flash_loan};
use flash_loan_receiver::FlashLoanMode;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{token, Address};
use test_harness::{
    assert_contract_error, errors, hub_asset, usdc_preset, LendingTest, ALICE, BOB, HARNESS_SPOKE,
};

fn pool_reserves_raw(t: &LendingTest, asset_name: &str) -> i128 {
    let asset = t.resolve_asset(asset_name);
    t.pool_client(asset_name).get_reserves(&hub_asset(asset))
}

fn prefund_receiver_fee(t: &LendingTest, receiver: &Address, asset: &Address, fee: i128) {
    if fee > 0 {
        token::StellarAssetClient::new(&t.env, asset).mint(receiver, &fee);
    }
}

fn mint_weird(t: &LendingTest, asset_name: &str, to: &Address, amount: i128) {
    let asset = t.resolve_asset(asset_name);
    test_harness::weird_token::WeirdTokenClient::new(&t.env, &asset).mint(to, &amount);
}

fn assert_reentry_fails(t: &mut LendingTest, mode: FlashLoanMode) {
    t.supply(ALICE, "USDC", 100_000.0);
    let receiver = t.deploy_adversarial_flash_loan_receiver();
    let data = receiver_data(t, mode);
    let amount = raw_units(t, "USDC", 10_000);
    let fee = flash_fee(t, "USDC", amount);
    let asset = t.resolve_asset("USDC");
    prefund_receiver_fee(t, &receiver, &asset, fee);
    let reserves_before = pool_reserves_raw(t, "USDC");
    let caller = t.get_or_create_user(BOB);
    t.env.set_auths(&[]);
    let result = strict_flash_loan(
        t,
        &caller,
        &hub_asset(asset.clone()),
        amount,
        &receiver,
        &data,
    );
    assert!(
        result.is_err(),
        "malicious flash-loan reentry {mode:?} must revert: {result:?}"
    );
    assert!(
        flash_guard_cleared(t),
        "flash guard must clear after {mode:?} rollback"
    );
    assert_eq!(
        pool_reserves_raw(t, "USDC"),
        reserves_before,
        "pool reserves must roll back after {mode:?}"
    );
}

#[test]
fn test_flash_loan_reenter_borrow_rejects() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    assert_reentry_fails(&mut t, FlashLoanMode::ReenterControllerBorrow);
}

#[test]
fn test_flash_loan_reenter_withdraw_rejects() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    assert_reentry_fails(&mut t, FlashLoanMode::ReenterControllerWithdraw);
}

#[test]
fn test_flash_loan_reenter_repay_rejects() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    assert_reentry_fails(&mut t, FlashLoanMode::ReenterControllerRepay);
}

#[test]
fn test_flash_loan_reenter_flash_loan_rejects() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    assert_reentry_fails(&mut t, FlashLoanMode::ReenterControllerFlashLoan);
}

#[test]
fn test_flash_loan_reenter_flash_position_rejects() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    assert_reentry_fails(&mut t, FlashLoanMode::ReenterControllerFlashPosition);
}

#[test]
fn test_flash_loan_reenter_multiply_rejects() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    assert_reentry_fails(&mut t, FlashLoanMode::ReenterControllerMultiply);
}

#[test]
fn test_flash_loan_reenter_swap_debt_rejects() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    assert_reentry_fails(&mut t, FlashLoanMode::ReenterControllerSwapDebt);
}

#[test]
fn test_flash_loan_reenter_swap_collateral_rejects() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    assert_reentry_fails(&mut t, FlashLoanMode::ReenterControllerSwapCollateral);
}

#[test]
fn test_flash_loan_reenter_rdwc_rejects() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    assert_reentry_fails(&mut t, FlashLoanMode::ReenterControllerRdwc);
}

#[test]
fn test_flash_loan_reenter_liquidate_rejects() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    assert_reentry_fails(&mut t, FlashLoanMode::ReenterControllerLiquidate);
}

#[test]
fn test_flash_loan_reenter_migrate_blend_rejects() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    assert_reentry_fails(&mut t, FlashLoanMode::ReenterMigrateBlend);
}

#[test]
fn test_flash_loan_reenter_supply_against_live_controller_rejects() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    assert_reentry_fails(&mut t, FlashLoanMode::ReenterControllerSupply);
}

#[test]
fn test_flash_loan_over_repay_still_charges_exact_fee() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    t.supply(ALICE, "USDC", 100_000.0);

    let receiver = t.deploy_adversarial_flash_loan_receiver();
    let data = receiver_data(&t, FlashLoanMode::OverRepay);
    let amount = raw_units(&t, "USDC", 10_000);
    let fee = flash_fee(&t, "USDC", amount);
    let asset = t.resolve_asset("USDC");
    prefund_receiver_fee(&t, &receiver, &asset, fee);

    let reserves_before = pool_reserves_raw(&t, "USDC");
    let revenue_before = t.snapshot_revenue("USDC");
    t.try_flash_loan_with_data(BOB, "USDC", amount, &receiver, &data)
        .expect("approving more than amount+fee must still succeed");
    assert!(flash_guard_cleared(&t));
    assert_eq!(pool_reserves_raw(&t, "USDC"), reserves_before + fee);
    assert_eq!(t.snapshot_revenue("USDC"), revenue_before + fee);
}

#[test]
fn test_flash_loan_push_to_pool_fails_closed() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    t.supply(ALICE, "USDC", 100_000.0);

    let receiver = t.deploy_adversarial_flash_loan_receiver();
    let data = receiver_data(&t, FlashLoanMode::PushToPool);
    let amount = raw_units(&t, "USDC", 10_000);
    let reserves_before = pool_reserves_raw(&t, "USDC");
    let result = t.try_flash_loan_with_data(BOB, "USDC", amount, &receiver, &data);
    assert_contract_error(result, errors::INVALID_FLASHLOAN_REPAY);
    assert!(flash_guard_cleared(&t));
    assert_eq!(pool_reserves_raw(&t, "USDC"), reserves_before);
}

#[test]
fn test_flash_loan_controller_receiver_rejects() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    t.supply(ALICE, "USDC", 100_000.0);
    let data = receiver_data(&t, FlashLoanMode::Success);
    let amount = raw_units(&t, "USDC", 1_000);
    let controller = t.controller.clone();
    let result = t.try_flash_loan_with_data(BOB, "USDC", amount, &controller, &data);
    assert!(
        result.is_err(),
        "controller cannot be a flash-loan receiver: {result:?}"
    );
    assert!(flash_guard_cleared(&t));
}

#[test]
fn test_flash_loan_pool_receiver_rejects() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    t.supply(ALICE, "USDC", 100_000.0);
    let data = receiver_data(&t, FlashLoanMode::Success);
    let amount = raw_units(&t, "USDC", 1_000);
    let pool = t.resolve_market("USDC").pool.clone();
    let result = t.try_flash_loan_with_data(BOB, "USDC", amount, &pool, &data);
    assert!(
        result.is_err(),
        "pool cannot be a flash-loan receiver: {result:?}"
    );
    assert!(flash_guard_cleared(&t));
}

#[test]
fn test_flash_loan_insufficient_liquidity_rejects() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    let receiver = t.deploy_adversarial_flash_loan_receiver();
    let data = receiver_data(&t, FlashLoanMode::Success);
    let amount = raw_units(&t, "USDC", 2_000_000);
    let result = t.try_flash_loan_with_data(BOB, "USDC", amount, &receiver, &data);
    assert_contract_error(result, errors::INSUFFICIENT_LIQUIDITY);
    assert!(flash_guard_cleared(&t));
}

#[test]
fn test_flash_loan_rejects_when_paused() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    t.supply(ALICE, "USDC", 100_000.0);
    t.pause();
    let receiver = t.deploy_flash_loan_receiver();
    let result = t.try_flash_loan(BOB, "USDC", 1_000.0, &receiver);
    assert_contract_error(result, errors::CONTRACT_PAUSED);
    assert!(flash_guard_cleared(&t));
}

#[test]
fn test_flash_loan_fee_on_transfer_fails_closed() {
    let mut t = LendingTest::new()
        .with_fee_on_transfer_market(usdc_preset(), 100)
        .build();

    let receiver = t.deploy_adversarial_flash_loan_receiver();
    let data = receiver_data(&t, FlashLoanMode::Success);
    let amount = raw_units(&t, "USDC", 10_000);
    let fee = flash_fee(&t, "USDC", amount);
    let asset = t.resolve_asset("USDC");
    mint_weird(&t, "USDC", &receiver, fee);
    let reserves_before = pool_reserves_raw(&t, "USDC");
    let caller = t.get_or_create_user(BOB);
    t.env.set_auths(&[]);
    let result = strict_flash_loan(&t, &caller, &hub_asset(asset), amount, &receiver, &data);
    assert!(
        result.is_err(),
        "fee-on-transfer flash loan must fail closed: {result:?}"
    );
    assert!(flash_guard_cleared(&t));
    assert_eq!(pool_reserves_raw(&t, "USDC"), reserves_before);
}

#[test]
fn test_flash_loan_extra_credit_is_not_pool_theft() {
    let mut t = LendingTest::new()
        .with_extra_credit_market(usdc_preset(), 100)
        .build();

    let receiver = t.deploy_adversarial_flash_loan_receiver();
    let data = receiver_data(&t, FlashLoanMode::Success);
    let amount = raw_units(&t, "USDC", 10_000);
    let fee = flash_fee(&t, "USDC", amount);
    let asset = t.resolve_asset("USDC");
    mint_weird(&t, "USDC", &receiver, fee);
    let reserves_before = pool_reserves_raw(&t, "USDC");
    let caller = t.get_or_create_user(BOB);
    t.env.set_auths(&[]);
    let result = strict_flash_loan(
        &t,
        &caller,
        &hub_asset(asset.clone()),
        amount,
        &receiver,
        &data,
    );
    // Extra bps also apply on the pool's `transfer_from` repay hop, so the
    // exact post-repay SAC bracket (`pre + fee`) is violated and the loan
    // fails closed. The extra units are not booked as protocol cash.
    assert!(
        result.is_err(),
        "extra-credit repay hop must fail the exact-balance check: {result:?}"
    );
    assert!(flash_guard_cleared(&t));
    assert_eq!(
        pool_reserves_raw(&t, "USDC"),
        reserves_before,
        "pool reserves must roll back; extra credit is not protocol revenue"
    );
}

#[test]
fn test_flash_loan_transfer_hook_cannot_reenter() {
    let mut t = LendingTest::new()
        .with_transfer_hook_market(usdc_preset())
        .build();

    let receiver = t.deploy_adversarial_flash_loan_receiver();
    let data = receiver_data(&t, FlashLoanMode::Success);
    let amount = raw_units(&t, "USDC", 10_000);
    let fee = flash_fee(&t, "USDC", amount);
    mint_weird(&t, "USDC", &receiver, fee);
    let asset = t.resolve_asset("USDC");
    let reserves_before = pool_reserves_raw(&t, "USDC");
    let caller = t.get_or_create_user(BOB);
    t.env.set_auths(&[]);
    let result = strict_flash_loan(&t, &caller, &hub_asset(asset), amount, &receiver, &data);
    assert!(
        result.is_err(),
        "token transfer hook must not reenter: {result:?}"
    );
    assert!(flash_guard_cleared(&t));
    assert_eq!(pool_reserves_raw(&t, "USDC"), reserves_before);
}

#[test]
fn test_flash_loan_eoa_receiver_still_rejected() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    t.supply(ALICE, "USDC", 100_000.0);
    let eoa = Address::generate(&t.env);
    let data = receiver_data(&t, FlashLoanMode::Success);
    let amount = raw_units(&t, "USDC", 1_000);
    let result = t.try_flash_loan_with_data(BOB, "USDC", amount, &eoa, &data);
    assert_contract_error(result, errors::INVALID_FLASHLOAN_RECEIVER);
    assert!(flash_guard_cleared(&t));
}

#[test]
fn test_flash_loan_plan_targets_existing_account() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);
    let alice_id = t.resolve_account_id(ALICE);
    let receiver = t.deploy_adversarial_flash_loan_receiver();
    t.set_flash_loan_receiver_plan(&receiver, HARNESS_SPOKE, alice_id);
    let data = receiver_data(&t, FlashLoanMode::ReenterControllerBorrow);
    let amount = raw_units(&t, "USDC", 1_000);
    let fee = flash_fee(&t, "USDC", amount);
    let asset = t.resolve_asset("USDC");
    prefund_receiver_fee(&t, &receiver, &asset, fee);
    let result = t.try_flash_loan_with_data(BOB, "USDC", amount, &receiver, &data);
    assert!(
        result.is_err(),
        "reenter-borrow against an existing account must still fail: {result:?}"
    );
    assert!(flash_guard_cleared(&t));
}
