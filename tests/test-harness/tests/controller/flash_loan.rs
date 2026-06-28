use common::math::fp::Bps;
use flash_loan_receiver::{FlashLoanMode, FlashLoanRequest};
use soroban_sdk::testutils::{Address as _, MockAuth, MockAuthInvoke};
use soroban_sdk::xdr::ToXdr;
use soroban_sdk::{token, Address, Bytes, IntoVal, Val, Vec};
use test_harness::{
    assert_contract_error, days, errors, eth_preset, hub_asset, usdc_preset, LendingTest, ALICE,
    BOB,
};

fn raw_units(t: &LendingTest, asset_name: &str, units: i128) -> i128 {
    units * 10i128.pow(t.resolve_market(asset_name).decimals)
}

fn flash_fee(t: &LendingTest, asset_name: &str, amount: i128) -> i128 {
    let config = t.get_asset_config(asset_name);
    Bps::from(config.flashloan_fee_bps).flash_loan_fee_on(&t.env, amount)
}

fn flash_guard_cleared(t: &LendingTest) -> bool {
    t.env.as_contract(&t.controller, || {
        !controller::test_support::is_flash_loan_ongoing(&t.env)
    })
}

fn pool_reserves(t: &LendingTest, asset_name: &str) -> i128 {
    let asset = t.resolve_asset(asset_name);
    t.pool_client(asset_name).get_reserves(&hub_asset(asset))
}

fn receiver_data(t: &LendingTest, mode: FlashLoanMode) -> Bytes {
    FlashLoanRequest { mode }.to_xdr(&t.env)
}

fn strict_flash_loan(
    t: &LendingTest,
    caller: &Address,
    asset: &Address,
    amount: i128,
    receiver: &Address,
    data: &Bytes,
) -> Result<(), std::string::String> {
    let args: Vec<Val> = (
        caller.clone(),
        asset.clone(),
        amount,
        receiver.clone(),
        data.clone(),
    )
        .into_val(&t.env);
    let invoke = MockAuthInvoke {
        contract: &t.controller,
        fn_name: "flash_loan",
        args,
        sub_invokes: &[],
    };
    let auths = [MockAuth {
        address: caller,
        invoke: &invoke,
    }];
    // Soroban `try_*` returns `Result<Result<T, ConversionError>, Result<Error, InvokeError>>`.
    match t
        .ctrl_client()
        .mock_auths(&auths)
        .try_flash_loan(caller, asset, &amount, receiver, data)
    {
        Ok(Ok(())) => Ok(()),
        Ok(Err(conv)) => Err(std::format!("conversion error: {conv:?}")),
        Err(Ok(contract_err)) => Err(std::format!("{contract_err:?}")),
        Err(Err(invoke)) => Err(std::format!("invoke error: {invoke:?}")),
    }
}

fn prefund_receiver_fee(t: &LendingTest, receiver: &Address, asset: &Address, fee: i128) {
    token::StellarAssetClient::new(&t.env, asset).mint(receiver, &fee);
}
// 1. test_flash_loan_success_under_non_root_auth
// Under the harness default auth mock, the receiver approves repayment and the
// pool pulls exactly amount + fee.

#[test]
fn test_flash_loan_success_under_non_root_auth() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Supply liquidity so the pool has funds.
    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    // Advance and sync to generate baseline revenue.
    t.advance_and_sync(days(30));

    let receiver = t.deploy_flash_loan_receiver();
    let result = t.try_flash_loan(BOB, "USDC", 10_000.0, &receiver);

    assert!(
        result.is_ok(),
        "flash loan with good receiver must succeed under non-root auth mock: {:?}",
        result
    );
    assert!(
        flash_guard_cleared(&t),
        "flash-loan guard must clear after a successful flash loan"
    );
}
// 2. test_flash_loan_rejects_bad_repayment

#[test]
fn test_flash_loan_rejects_bad_repayment() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    t.supply(ALICE, "USDC", 100_000.0);

    let bad_receiver = t.deploy_bad_flash_loan_receiver();
    let result = t.try_flash_loan(BOB, "USDC", 10_000.0, &bad_receiver);
    // The bad receiver triggers a cross-contract failure that surfaces as
    // a host error, not a specific contract error code.
    assert!(
        result.is_err(),
        "flash loan should fail when receiver doesn't repay"
    );
}
// 3. test_flash_loan_rejects_disabled

#[test]
fn test_flash_loan_rejects_disabled() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    t.supply(ALICE, "USDC", 100_000.0);

    // Disable flash loans for USDC.
    t.edit_asset_config("USDC", |cfg| {
        cfg.is_flashloanable = false;
    });

    let receiver = t.deploy_flash_loan_receiver();
    let result = t.try_flash_loan(BOB, "USDC", 10_000.0, &receiver);
    assert_contract_error(result, errors::FLASHLOAN_NOT_ENABLED);
}
// 4. test_flash_loan_rejects_zero_amount

#[test]
fn test_flash_loan_rejects_zero_amount() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    t.supply(ALICE, "USDC", 100_000.0);

    let receiver = t.deploy_flash_loan_receiver();
    let result = t.try_flash_loan(BOB, "USDC", 0.0, &receiver);
    // Must reject with the precise AMOUNT_MUST_BE_POSITIVE (14).
    assert_contract_error(result, errors::AMOUNT_MUST_BE_POSITIVE);
}
// 5. test_flash_loan_reentrancy_blocks_supply

#[test]
fn test_flash_loan_reentrancy_blocks_supply() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.set_flash_loan_ongoing(true);

    let result = t.try_supply(BOB, "USDC", 1_000.0);
    assert_contract_error(result, errors::FLASH_LOAN_ONGOING);

    t.set_flash_loan_ongoing(false);
}
// 6. test_flash_loan_reentrancy_blocks_borrow

#[test]
fn test_flash_loan_reentrancy_blocks_borrow() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.set_flash_loan_ongoing(true);

    let result = t.try_borrow(ALICE, "ETH", 1.0);
    assert_contract_error(result, errors::FLASH_LOAN_ONGOING);

    t.set_flash_loan_ongoing(false);
}
// 7. test_flash_loan_reentrancy_blocks_withdraw

#[test]
fn test_flash_loan_reentrancy_blocks_withdraw() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.set_flash_loan_ongoing(true);

    let result = t.try_withdraw(ALICE, "USDC", 1_000.0);
    assert_contract_error(result, errors::FLASH_LOAN_ONGOING);

    t.set_flash_loan_ongoing(false);
}
// 8. test_flash_loan_reentrancy_blocks_repay

#[test]
fn test_flash_loan_reentrancy_blocks_repay() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);
    t.set_flash_loan_ongoing(true);

    let result = t.try_repay(ALICE, "ETH", 0.5);
    assert_contract_error(result, errors::FLASH_LOAN_ONGOING);

    t.set_flash_loan_ongoing(false);
}
// 9. test_flash_loan_reentrancy_blocks_liquidation

#[test]
fn test_flash_loan_reentrancy_blocks_liquidation() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    t.set_price("USDC", test_harness::usd_cents(50));
    t.assert_liquidatable(ALICE);

    t.set_flash_loan_ongoing(true);

    let result = t.try_liquidate(BOB, ALICE, "ETH", 1.0);
    assert_contract_error(result, errors::FLASH_LOAN_ONGOING);

    t.set_flash_loan_ongoing(false);
}
// 10. test_flash_loan_fee_config_matches_default_preset

#[test]
fn test_flash_loan_fee_config_matches_default_preset() {
    // Pin the default preset config values so any change to
    // `usdc_preset()` surfaces in CI. The end-to-end fee transfer runs in
    // the inline `test_flash_loan` in pool/src/lib.rs, which uses the admin
    // (covered by mock_all_auths) as receiver.
    let t = LendingTest::new().with_market(usdc_preset()).build();

    let config = t.get_asset_config("USDC");
    assert_eq!(
        config.flashloan_fee_bps, 9,
        "USDC preset flash-loan fee must be 9 BPS (0.09%)"
    );
    assert!(
        config.is_flashloanable,
        "USDC preset must have is_flashloanable = true"
    );
}
// 11. test_flash_loan_tiny_amount_charges_min_fee_when_bps_positive

#[test]
fn test_flash_loan_tiny_amount_charges_min_fee_when_bps_positive() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    t.supply(ALICE, "USDC", 100.0);

    let receiver = t.deploy_adversarial_flash_loan_receiver();
    let data = receiver_data(&t, FlashLoanMode::Success);

    let amount = 1i128;
    let fee = flash_fee(&t, "USDC", amount);
    let asset = t.resolve_asset("USDC");
    prefund_receiver_fee(&t, &receiver, &asset, fee);

    let reserves_before = pool_reserves(&t, "USDC");
    let caller = t.get_or_create_user(BOB);

    t.env.set_auths(&[]);
    let result = strict_flash_loan(&t, &caller, &asset, amount, &receiver, &data);

    assert!(
        result.is_ok(),
        "tiny flash loan should succeed: {:?}",
        result
    );
    assert_eq!(fee, 1, "positive flashloan_fee_bps must charge at least 1");
    assert_eq!(pool_reserves(&t, "USDC"), reserves_before + fee);
}
// 12. test_flash_loan_allows_zero_fee_when_configured_zero

#[test]
fn test_flash_loan_allows_zero_fee_when_configured_zero() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market_config("USDC", |config| config.flashloan_fee_bps = 0)
        .build();

    t.supply(ALICE, "USDC", 100.0);

    let receiver = t.deploy_adversarial_flash_loan_receiver();
    let data = receiver_data(&t, FlashLoanMode::Success);

    let amount = 1i128;
    let fee = flash_fee(&t, "USDC", amount);
    let asset = t.resolve_asset("USDC");
    let reserves_before = pool_reserves(&t, "USDC");
    let caller = t.get_or_create_user(BOB);

    t.env.set_auths(&[]);
    let result = strict_flash_loan(&t, &caller, &asset, amount, &receiver, &data);

    assert!(result.is_ok(), "zero-fee market should remain explicit");
    assert_eq!(fee, 0);
    assert_eq!(pool_reserves(&t, "USDC"), reserves_before);
}
// 13. test_flash_loan_strict_receiver_success_with_preauthorized_repayment

#[test]
fn test_flash_loan_strict_receiver_success_with_preauthorized_repayment() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    t.supply(ALICE, "USDC", 100_000.0);

    let receiver = t.deploy_adversarial_flash_loan_receiver();
    let data = receiver_data(&t, FlashLoanMode::Success);

    let amount = raw_units(&t, "USDC", 10_000);
    let fee = flash_fee(&t, "USDC", amount);
    let asset = t.resolve_asset("USDC");
    prefund_receiver_fee(&t, &receiver, &asset, fee);

    let reserves_before = pool_reserves(&t, "USDC");
    let caller = t.get_or_create_user(BOB);

    t.env.set_auths(&[]);
    let result = strict_flash_loan(&t, &caller, &asset, amount, &receiver, &data);

    assert!(
        result.is_ok(),
        "strict receiver must approve amount + fee for the pool: {:?}",
        result
    );
    assert!(flash_guard_cleared(&t), "flash-loan guard must clear");
    assert_eq!(pool_reserves(&t, "USDC"), reserves_before + fee);
}
// 14. test_flash_loan_strict_receiver_rejects_success_without_fee_prefund

#[test]
fn test_flash_loan_strict_receiver_rejects_success_without_fee_prefund() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    t.supply(ALICE, "USDC", 100_000.0);

    let receiver = t.deploy_adversarial_flash_loan_receiver();
    let data = receiver_data(&t, FlashLoanMode::Success);

    let amount = raw_units(&t, "USDC", 10_000);
    let asset = t.resolve_asset("USDC");
    let reserves_before = pool_reserves(&t, "USDC");
    let caller = t.get_or_create_user(BOB);

    t.env.set_auths(&[]);
    let result = strict_flash_loan(&t, &caller, &asset, amount, &receiver, &data);

    assert!(
        result.is_err(),
        "receiver cannot repay fee it was not prefunded with"
    );
    assert!(flash_guard_cleared(&t), "flash-loan guard must roll back");
    assert_eq!(pool_reserves(&t, "USDC"), reserves_before);
}
// 15. test_flash_loan_adversarial_receiver_no_repay_rejects

#[test]
fn test_flash_loan_adversarial_receiver_no_repay_rejects() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    t.supply(ALICE, "USDC", 100_000.0);

    let receiver = t.deploy_adversarial_flash_loan_receiver();
    let data = receiver_data(&t, FlashLoanMode::NoRepay);

    let amount = raw_units(&t, "USDC", 10_000);
    let asset = t.resolve_asset("USDC");
    let reserves_before = pool_reserves(&t, "USDC");
    let caller = t.get_or_create_user(BOB);

    t.env.set_auths(&[]);
    let result = strict_flash_loan(&t, &caller, &asset, amount, &receiver, &data);

    assert!(result.is_err(), "no-repay receiver must fail");
    assert!(flash_guard_cleared(&t), "flash-loan guard must roll back");
    assert_eq!(pool_reserves(&t, "USDC"), reserves_before);
}
// 16. test_flash_loan_adversarial_receiver_under_repay_rejects

#[test]
fn test_flash_loan_adversarial_receiver_under_repay_rejects() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    t.supply(ALICE, "USDC", 100_000.0);

    let receiver = t.deploy_adversarial_flash_loan_receiver();
    let data = receiver_data(&t, FlashLoanMode::UnderRepay);

    let amount = raw_units(&t, "USDC", 10_000);
    let fee = flash_fee(&t, "USDC", amount);
    let asset = t.resolve_asset("USDC");
    prefund_receiver_fee(&t, &receiver, &asset, fee);

    let reserves_before = pool_reserves(&t, "USDC");

    // Default harness auth mock lets the receiver approve repayment; pool CEI must
    // still reject the under-repay with InvalidFlashloanRepay (#402).
    let result = t.try_flash_loan_with_data(BOB, "USDC", amount, &receiver, &data);
    assert_contract_error(result, errors::INVALID_FLASHLOAN_REPAY);
    assert!(flash_guard_cleared(&t), "flash-loan guard must roll back");
    assert_eq!(pool_reserves(&t, "USDC"), reserves_before);
}
// 17. test_flash_loan_adversarial_receiver_reenter_pool_flash_loan_rejects

#[test]
fn test_flash_loan_adversarial_receiver_reenter_pool_flash_loan_rejects() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    t.supply(ALICE, "USDC", 100_000.0);

    let receiver = t.deploy_adversarial_flash_loan_receiver();
    let data = receiver_data(&t, FlashLoanMode::ReenterPoolFlashLoan);

    let amount = raw_units(&t, "USDC", 10_000);
    let fee = flash_fee(&t, "USDC", amount);
    let asset = t.resolve_asset("USDC");
    prefund_receiver_fee(&t, &receiver, &asset, fee);

    let reserves_before = pool_reserves(&t, "USDC");
    let caller = t.get_or_create_user(BOB);

    t.env.set_auths(&[]);
    let result = strict_flash_loan(&t, &caller, &asset, amount, &receiver, &data);

    assert!(result.is_err(), "receiver pool reentry must fail");
    assert!(flash_guard_cleared(&t), "flash-loan guard must roll back");
    assert_eq!(pool_reserves(&t, "USDC"), reserves_before);
}
// 18. test_flash_loan_adversarial_receiver_callback_panic_rolls_back

#[test]
fn test_flash_loan_adversarial_receiver_callback_panic_rolls_back() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    t.supply(ALICE, "USDC", 100_000.0);

    let receiver = t.deploy_adversarial_flash_loan_receiver();
    let data = receiver_data(&t, FlashLoanMode::Panic);

    let amount = raw_units(&t, "USDC", 10_000);
    let asset = t.resolve_asset("USDC");
    let reserves_before = pool_reserves(&t, "USDC");
    let caller = t.get_or_create_user(BOB);

    t.env.set_auths(&[]);
    let result = strict_flash_loan(&t, &caller, &asset, amount, &receiver, &data);

    assert!(result.is_err(), "callback panic must fail");
    assert!(flash_guard_cleared(&t), "flash-loan guard must roll back");
    assert_eq!(pool_reserves(&t, "USDC"), reserves_before);
}
// 19. test_flash_loan_non_contract_receiver_rejects_and_rolls_back

#[test]
fn test_flash_loan_non_contract_receiver_rejects_and_rolls_back() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    t.supply(ALICE, "USDC", 100_000.0);

    let amount = raw_units(&t, "USDC", 10_000);
    let asset = t.resolve_asset("USDC");
    let non_contract_receiver = Address::generate(&t.env);
    let reserves_before = pool_reserves(&t, "USDC");
    let caller = t.get_or_create_user(BOB);
    let data = receiver_data(&t, FlashLoanMode::Success);

    t.env.set_auths(&[]);
    let result = strict_flash_loan(&t, &caller, &asset, amount, &non_contract_receiver, &data);

    assert!(
        result.is_err(),
        "non-contract receiver cannot handle execute_flash_loan"
    );
    assert!(flash_guard_cleared(&t), "flash-loan guard must roll back");
    assert_eq!(pool_reserves(&t, "USDC"), reserves_before);
}
// 20. test_flash_loan_adversarial_receiver_rejects_invalid_data

#[test]
fn test_flash_loan_adversarial_receiver_rejects_invalid_data() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    t.supply(ALICE, "USDC", 100_000.0);

    let receiver = t.deploy_adversarial_flash_loan_receiver();
    let amount = raw_units(&t, "USDC", 10_000);
    let asset = t.resolve_asset("USDC");
    let reserves_before = pool_reserves(&t, "USDC");
    let caller = t.get_or_create_user(BOB);
    let malformed_data = Bytes::new(&t.env);

    t.env.set_auths(&[]);
    let result = strict_flash_loan(&t, &caller, &asset, amount, &receiver, &malformed_data);

    assert!(result.is_err(), "malformed receiver data must fail");
    assert!(flash_guard_cleared(&t), "flash-loan guard must roll back");
    assert_eq!(pool_reserves(&t, "USDC"), reserves_before);
}
// 21. test_flash_loan_adversarial_receiver_reenter_controller_supply_rejects

// Exercises `FlashLoanMode::ReenterControllerSupply` — the reference
// receiver tries to call `controller.supply` from inside the callback. The
// controller's `require_not_flash_loaning` guard must reject this and roll
// the loan back. Covers flash-loan-receiver lines 99-101 + 183-210.
#[test]
fn test_flash_loan_adversarial_receiver_reenter_controller_supply_rejects() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    t.supply(ALICE, "USDC", 100_000.0);

    let receiver = t.deploy_adversarial_flash_loan_receiver();
    let data = receiver_data(&t, FlashLoanMode::ReenterControllerSupply);

    let amount = raw_units(&t, "USDC", 10_000);
    let fee = flash_fee(&t, "USDC", amount);
    let asset = t.resolve_asset("USDC");
    prefund_receiver_fee(&t, &receiver, &asset, fee);

    let reserves_before = pool_reserves(&t, "USDC");
    let caller = t.get_or_create_user(BOB);

    t.env.set_auths(&[]);
    let result = strict_flash_loan(&t, &caller, &asset, amount, &receiver, &data);

    assert!(
        result.is_err(),
        "receiver controller-reentry must fail under flash-loan guard"
    );
    assert!(flash_guard_cleared(&t), "flash-loan guard must roll back");
    assert_eq!(pool_reserves(&t, "USDC"), reserves_before);
}
