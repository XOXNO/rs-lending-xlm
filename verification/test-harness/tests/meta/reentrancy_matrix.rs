use common::types::PositionMode;
use soroban_sdk::Bytes;
use test_harness::{
    assert_contract_error, build_aggregator_swap, errors, LendingTest, ALICE, BOB, LIQUIDATOR,
};

fn setup() -> LendingTest {
    let mut t = LendingTest::new().standard_two_asset().build();

    // Seed positions so the entries actually have something to act on; the
    // reentrancy guard fires before any business logic, so the assertions
    // pass regardless of input shape — but we want each entry to make it
    // past `caller.require_auth` so the flag check is what reverts.
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 1.0);
    t.supply(BOB, "USDC", 5_000.0);
    t.supply(LIQUIDATOR, "USDC", 5_000.0);
    t
}

#[test]
fn test_all_state_changing_entries_reject_under_flash_loan_ongoing() {
    let mut t = setup();
    let alice_id = t.resolve_account_id(ALICE);
    t.set_flash_loan_ongoing(true);

    assert_contract_error(t.try_supply(BOB, "USDC", 1.0), errors::FLASH_LOAN_ONGOING);
    assert_contract_error(t.try_borrow(ALICE, "ETH", 0.01), errors::FLASH_LOAN_ONGOING);
    assert_contract_error(t.try_repay(ALICE, "ETH", 0.01), errors::FLASH_LOAN_ONGOING);
    assert_contract_error(
        t.try_withdraw(ALICE, "USDC", 1.0),
        errors::FLASH_LOAN_ONGOING,
    );
    assert_contract_error(
        t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 0.01),
        errors::FLASH_LOAN_ONGOING,
    );

    // Router / keeper / revenue (ADR-0006 + router.rs).
    assert_contract_error(
        t.try_update_indexes_for(&["USDC", "ETH"]),
        errors::FLASH_LOAN_ONGOING,
    );
    assert_contract_error(t.try_claim_revenue("USDC"), errors::FLASH_LOAN_ONGOING);
    assert_contract_error(t.try_add_rewards("USDC", 1.0), errors::FLASH_LOAN_ONGOING);
    assert_contract_error(
        t.try_update_account_threshold("USDC", false, &[alice_id]),
        errors::FLASH_LOAN_ONGOING,
    );
    assert_contract_error(
        t.try_clean_bad_debt_by_id(alice_id),
        errors::FLASH_LOAN_ONGOING,
    );

    // Strategy entrypoints (each calls `require_not_flash_loaning` at the top).
    let empty_swap = Bytes::new(&t.env);
    assert_contract_error(
        t.try_multiply(ALICE, "USDC", 1.0, "ETH", PositionMode::Multiply, &empty_swap),
        errors::FLASH_LOAN_ONGOING,
    );
    assert_contract_error(
        t.try_swap_debt(ALICE, "ETH", 0.01, "USDC", &empty_swap),
        errors::FLASH_LOAN_ONGOING,
    );
    assert_contract_error(
        t.try_swap_collateral(ALICE, "USDC", 1.0, "ETH", &empty_swap),
        errors::FLASH_LOAN_ONGOING,
    );
    assert_contract_error(
        t.try_repay_debt_with_collateral(ALICE, "USDC", 1.0, "ETH", &empty_swap, false),
        errors::FLASH_LOAN_ONGOING,
    );

    // Nested flash loan while the guard is already held.
    let receiver = t.deploy_flash_loan_receiver();
    assert_contract_error(
        t.try_flash_loan(BOB, "USDC", 1.0, &receiver),
        errors::FLASH_LOAN_ONGOING,
    );

    t.set_flash_loan_ongoing(false);

    // Sanity: a real multiply still works once the guard is cleared.
    t.fund_router("USDC", 3_000.0);
    let steps = build_aggregator_swap(
        &t,
        "ETH",
        "USDC",
        test_harness::apply_flash_fee(10_000_000),
        3000_0000000,
    );
    let account_id = t
        .try_multiply(ALICE, "USDC", 1.0, "ETH", PositionMode::Multiply, &steps)
        .expect("multiply should succeed after guard cleared");
    assert!(account_id > 0);
}