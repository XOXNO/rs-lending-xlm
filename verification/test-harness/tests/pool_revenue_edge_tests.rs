//! Edge-case coverage for revenue distribution paths in `pool/src/lib.rs`.
//!
//! Targets two branches that the broader revenue suite does not exercise:
//!
//! - `add_rewards` panics with `NoSuppliersToReward` when
//!   `cache.supplied == Ray::ZERO`. This case covers a market whose
//!   only supplier withdraws the entire position, returning `supplied_ray`
//!   to zero.
//!
//! - The zero-transfer branch of `claim_revenue` runs when
//!   `cache.revenue > 0` but `current_reserves == 0`, making
//!   `amount_to_transfer == 0`.
//!   In that case the pool must still emit `MarketUpdate` and persist
//!   state, but transfer nothing and burn no scaled revenue.

extern crate std;

use test_harness::{
    assert_contract_error, errors, eth_preset, usdc_preset, LendingTest, ALICE, BOB,
};
// 1. add_rewards on a market drained back to zero suppliers

/// Re-triggers `NoSuppliersToReward` after a funded market is fully
/// withdrawn. This exercises the zero-supply panic path after market activity,
/// distinct from the never-supplied case in
/// `rewards_rigorous_tests::test_add_rewards_rejects_when_no_supply`.
#[test]
#[should_panic(expected = "Error(Contract, #37)")]
fn test_add_rewards_rejects_after_full_withdrawal() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_dust_disabled_all_markets()
        .with_max_utilization_disabled_all_markets()
        .build();

    // Alice supplies, then withdraws her entire position. No borrows happen,
    // so all scaled supply belongs to Alice and `withdraw_all` returns the
    // pool's `cache.supplied` to `Ray::ZERO`.
    t.supply(ALICE, "USDC", 10_000.0);
    t.withdraw_all(ALICE, "USDC");

    // The pool is empty again. add_rewards must reject rather than silently
    // crediting the reserve pot.
    t.add_rewards("USDC", 500.0);
}
// 2. claim_revenue with revenue > 0 but reserves drained to exactly 0

/// Drives `claim_revenue` into the `else` branch where `amount_to_transfer`
/// is zero because the pool's on-chain token balance has been borrowed away,
/// even though scaled revenue is positive. The call must succeed, return 0,
/// and leave both the revenue accumulator and the scaled supplied total
/// untouched (no burn happens when nothing transfers).
#[test]
fn test_claim_revenue_else_branch_when_reserves_fully_drained() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_dust_disabled_all_markets()
        .with_max_utilization_disabled_all_markets()
        .build();

    // Set up the controller's accumulator so `claim_revenue` is permitted.
    let accumulator = t
        .env
        .register(test_harness::mock_reflector::MockReflector, ());
    t.set_accumulator(&accumulator);

    // SpotOnly bypasses the TWAP requirement during oracle reads triggered
    // by `claim_revenue` -> `update_market_with_price`.
    t.set_oracle_single_spot("USDC");

    // Generate USDC revenue: Alice supplies + borrows USDC against her own
    // collateral, then time advances so interest accrues.
    t.supply(ALICE, "USDC", 1_000.0);
    t.borrow(ALICE, "USDC", 700.0);
    t.advance_time(31_536_000); // 1 year
    t.update_indexes_for(&["USDC"]);

    let revenue_before_claim = t.snapshot_revenue("USDC");
    assert!(
        revenue_before_claim > 0,
        "fixture must accrue revenue before draining reserves"
    );

    // Drain the remaining USDC reserves to exactly zero. Bob supplies enough
    // ETH ($2M of collateral at the preset price) so he can borrow the full
    // ~$1M USDC reserve without tripping `InsufficientCollateral`.
    t.supply(BOB, "ETH", 1000.0);
    let res_raw = t.pool_client("USDC").reserves();
    assert!(
        res_raw > 0,
        "expected positive USDC reserves to drain; got {}",
        res_raw
    );
    t.borrow_raw(BOB, "USDC", res_raw);

    // Confirm the precondition for the else branch: revenue > 0, reserves = 0.
    let res_after_drain = t.pool_client("USDC").reserves();
    assert_eq!(
        res_after_drain, 0,
        "reserves must be zero to reach the else branch"
    );
    let revenue_pre = t.snapshot_revenue("USDC");
    assert!(
        revenue_pre > 0,
        "revenue must remain positive after the drain"
    );

    // Act: claim_revenue takes the else branch because reserves == 0 and
    // therefore amount_to_transfer = min(0, treasury_actual) = 0.
    let claimed = t.claim_revenue("USDC");

    // Pool returns zero, no token movement happened.
    assert_eq!(claimed, 0, "no reserves => no transfer");

    // Revenue must remain positive: the burn only fires when
    // amount_to_transfer is positive. The else branch only emits + saves.
    let revenue_post = t.snapshot_revenue("USDC");
    assert!(
        revenue_post >= revenue_pre,
        "revenue must not shrink when nothing transferred: pre={}, post={}",
        revenue_pre,
        revenue_post
    );

    // Reserves stay zero (no transfer happened).
    assert_eq!(
        t.pool_client("USDC").reserves(),
        0,
        "reserves remain zero after a no-op claim"
    );
}
// 3. claim_revenue blocked when burning the last supply would leave debt

/// `burn_claimable_revenue` decrements both `cache.revenue` and
/// `cache.supplied` by the revenue's scaled share. If the previous final
/// supplier has already withdrawn (leaving only the protocol-revenue
/// share as `supplied`), the burn would land at `supplied == 0` while
/// `borrowed > 0` — the same insolvent post-state the withdraw path
/// guards against. Pins that the symmetric guard in `claim_revenue`
/// rejects this state and the revenue stays parked until borrowers exit.
#[test]
fn test_claim_revenue_blocked_when_post_state_insolvent() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_dust_disabled_all_markets()
        .with_max_utilization_disabled_all_markets()
        .build();

    // Accumulator + spot-only oracle for the revenue claim path.
    let accumulator = t
        .env
        .register(test_harness::mock_reflector::MockReflector, ());
    t.set_accumulator(&accumulator);
    t.set_oracle_single_spot("USDC");

    // ALICE supplies USDC; BOB borrows USDC against ETH collateral so
    // `cache.borrowed` stays positive across ALICE's withdrawal.
    t.supply(ALICE, "USDC", 1_000.0);
    t.supply(BOB, "ETH", 10.0);
    t.borrow(BOB, "USDC", 500.0);

    // Accrue interest so revenue scales up to a non-trivial share of
    // `cache.supplied`.
    t.advance_time(31_536_000); // 1 year
    t.update_indexes_for(&["USDC"]);
    let revenue_pre = t.snapshot_revenue("USDC");
    assert!(revenue_pre > 0, "fixture must accrue revenue");

    // ALICE fully withdraws. Post-state: cache.supplied = revenue's
    // scaled share (positive, so the withdraw-side solvency guard
    // passes), borrowed > 0.
    t.withdraw_all(ALICE, "USDC");

    // claim_revenue would burn the remaining `supplied` (the revenue
    // share) and land at `(supplied = 0, borrowed > 0)`. The symmetric
    // solvency guard must reject this and leave the revenue parked
    // until borrowers exit.
    let result = t.try_claim_revenue("USDC");
    assert_contract_error(result, errors::POOL_INSOLVENT);
}
