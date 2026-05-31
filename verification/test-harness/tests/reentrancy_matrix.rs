//! Exhaustive reentrancy guard regression.
//!
//! Soroban does not natively prevent reentrancy on cross-contract calls
//! (Halborn Normal Finance recommendation; STELLAR_AUDIT_FINDINGS.md §4.10).
//! Every state-changing controller entry must call
//! `validation::require_not_flash_loaning` before mutating storage. Because
//! the discipline is enforced by convention rather than a proc-macro, this
//! test exhaustively flips the `flash_loan_ongoing` flag and confirms each
//! public entry rejects with `FLASH_LOAN_ONGOING`.
//!
//! When adding a new state-changing public entry: add it here too.

extern crate std;

use test_harness::{
    assert_contract_error, errors, eth_preset, usdc_preset, LendingTest, ALICE, BOB, LIQUIDATOR,
};

fn setup() -> LendingTest {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

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
}
