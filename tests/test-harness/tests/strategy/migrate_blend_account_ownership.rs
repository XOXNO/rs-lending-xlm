//! `migrate_from_blend` borrows `debt_caps` on the target account to repay the
//! CALLER's Blend liability. The `AccountGuard::Migrate` owner check
//! (`controller/src/account.rs:101-102`) is the only thing that stops a stranger
//! from pointing it at someone else's funded account.

use test_harness::mock_blend::{KIND_COLLATERAL, KIND_LIABILITY};
use test_harness::{assert_contract_error, errors, LendingTest, ALICE, BOB};

/// Alice (victim): 10 000 USDC collateral, 0.5 ETH debt, lots of headroom.
/// Bob (stranger): a Blend position with 10 000 USDC collateral and 1 ETH liability.
fn victim_and_stranger() -> (LendingTest, u64) {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 0.5);
    let victim_account = t.resolve_account_id(ALICE);
    t.seed_blend(BOB, "USDC", KIND_COLLATERAL, 10_000.0);
    t.seed_blend(BOB, "ETH", KIND_LIABILITY, 1.0);
    (t, victim_account)
}

#[test]
fn migrate_into_an_account_the_caller_does_not_own_is_not_authorized() {
    let (mut t, victim_account) = victim_and_stranger();
    assert_eq!(errors::NOT_AUTHORIZED, 44);
    let debt_before = t.borrow_balance_raw(ALICE, "ETH");
    let supply_before = t.supply_balance_raw(ALICE, "USDC");

    // Pure theft shape: borrow on the victim, repay the stranger's Blend debt.
    let debt_only = t.try_migrate_from_blend(BOB, victim_account, &[], &[], &[("ETH", 1.0)]);
    assert_contract_error(debt_only, errors::NOT_AUTHORIZED);

    // Same, with the stranger's own collateral moved in to look benign.
    let with_collateral =
        t.try_migrate_from_blend(BOB, victim_account, &["USDC"], &[], &[("ETH", 1.0)]);
    assert_contract_error(with_collateral, errors::NOT_AUTHORIZED);

    assert_eq!(t.borrow_balance_raw(ALICE, "ETH"), debt_before);
    assert_eq!(t.supply_balance_raw(ALICE, "USDC"), supply_before);
    assert_eq!(t.blend_position(BOB, "ETH", KIND_LIABILITY), 10_000_000);
    assert_eq!(
        t.blend_position(BOB, "USDC", KIND_COLLATERAL),
        100_000_000_000
    );

    // Control: the identical arguments succeed on an account the caller owns, so
    // the rejection above is the ownership check and nothing else.
    let own = t
        .try_migrate_from_blend(BOB, 0, &["USDC"], &[], &[("ETH", 1.0)])
        .expect("the stranger can migrate into their own new account");
    assert_ne!(own, victim_account);
    assert_eq!(t.blend_position(BOB, "ETH", KIND_LIABILITY), 0);
    assert_eq!(t.borrow_balance_raw(ALICE, "ETH"), debt_before);
}
