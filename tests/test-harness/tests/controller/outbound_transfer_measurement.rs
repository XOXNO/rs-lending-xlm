//! Where the shortfall lands when an asset under-delivers on an outbound leg:
//! the recapitalize refund (`pool/src/ops/recapitalize.rs`) and the revenue
//! claim's forward to the accumulator (`controller/src/keepers.rs`, now measured
//! on both hops after F-8).

use soroban_sdk::token;
use test_harness::{
    eth_preset, hub_asset, usdc_preset, weird_token::WeirdTokenClient, LendingTest, ALICE, BOB,
    CAROL,
};

/// A fee-on-transfer recapitalize into a healthy market strands nothing inside
/// the protocol: the whole receipt is refunded, the pool's balance and cash
/// book both come back to where they started, and the only loss is the token's
/// own haircut, taken twice, by the token contract.
///
/// This is the answer to "the refund is unmeasured, so where does the
/// difference go". `transfer_out` moves the declared `refund` out of the pool
/// regardless of what the payer receives, so the pool cannot retain it. The
/// haircut never enters protocol custody at all.
#[test]
fn recapitalize_refund_is_unmeasured_but_strands_nothing() {
    let mut t = LendingTest::new()
        .with_fee_on_transfer_market(usdc_preset(), 100)
        .with_market(eth_preset())
        .build();

    let payer = t.get_or_create_user(ALICE);
    let asset = t.resolve_asset("USDC");
    let pool = t.resolve_market("USDC").pool.clone();
    let controller = t.controller_address();
    let key = hub_asset(asset.clone());
    let tok = token::Client::new(&t.env, &asset);

    let amount = 10_000_000_000i128;
    WeirdTokenClient::new(&t.env, &asset).mint(&payer, &amount);

    let payer_before = tok.balance(&payer);
    let pool_before = tok.balance(&pool);
    let controller_before = tok.balance(&controller);
    let cash_before = t.pool_client("USDC").get_reserves(&key);

    let applied = t.ctrl_client().recapitalize(&payer, &key, &amount);

    // A healthy market has no backing shortfall, so nothing is applied and the
    // entire measured receipt is refunded.
    assert_eq!(applied, 0, "a backed market must apply nothing");

    assert_eq!(
        tok.balance(&pool),
        pool_before,
        "the pool must not retain the haircut"
    );
    assert_eq!(
        tok.balance(&controller),
        controller_before,
        "the controller must not retain the haircut"
    );
    assert_eq!(
        t.pool_client("USDC").get_reserves(&key),
        cash_before,
        "cash book must be untouched when nothing is applied"
    );

    // in: payer -> pool delivers 99%. out: pool -> payer delivers 99% of that.
    let received = amount - amount / 100;
    let refunded_to_payer = received - received / 100;
    assert_eq!(
        payer_before - tok.balance(&payer),
        amount - refunded_to_payer,
        "the payer absorbs exactly two token haircuts and nothing else"
    );
}

/// F-8 fixed: the controller measures what it receives from the pool and
/// forwards exactly that (`keepers.rs`), so an under-delivering asset no longer
/// makes it raid a stranded balance. The accumulator is short only by the
/// forward transfer's own haircut, which is inherent to the token.
#[test]
fn claim_revenue_forwards_the_measured_amount_and_leaves_controller_dust_intact() {
    let mut t = LendingTest::new()
        .with_fee_on_transfer_market(usdc_preset(), 100)
        .with_market(eth_preset())
        .with_dust_disabled_all_markets()
        .with_max_utilization_disabled_all_markets()
        .build();

    let accumulator = t
        .env
        .register(test_harness::mock_reflector::MockReflector, ());
    t.set_accumulator(&accumulator);
    t.set_oracle_single_spot("USDC");

    let asset = t.resolve_asset("USDC");
    let controller = t.controller_address();
    let tok = token::Client::new(&t.env, &asset);
    let weird = WeirdTokenClient::new(&t.env, &asset);

    t.supply(ALICE, "USDC", 1_000.0);
    t.supply(CAROL, "USDC", 300.0);
    t.supply(BOB, "ETH", 50.0);
    t.borrow(BOB, "USDC", 700.0);
    t.advance_time(31_536_000);
    t.update_indexes_for(&["USDC"]);

    let revenue = t.snapshot_revenue("USDC");
    assert!(revenue > 0, "fixture must accrue revenue");

    // Stranded dust at the controller, of the kind a receiver callback can
    // leave behind. Large enough to cover the forward's shortfall.
    let dust = 100_000_000_000i128;
    weird.mint(&controller, &dust);
    let controller_before = tok.balance(&controller);

    let claimed = t.claim_revenue("USDC");
    assert!(
        claimed > 0,
        "a positive claim is what makes this observable"
    );

    let accumulator_got = tok.balance(&accumulator);
    let controller_after = tok.balance(&controller);

    // `claimed` is now the measured receipt, and the accumulator is short only
    // by the inherent haircut on the forward transfer itself (unavoidable for a
    // fee-on-transfer token).
    assert_eq!(
        accumulator_got,
        claimed - claimed / 100,
        "accumulator receives one forward-hop haircut less than the measured claim"
    );

    // F-8 fixed: the controller forwards exactly what it received and never
    // raids its pre-existing dust.
    assert_eq!(
        controller_after, controller_before,
        "controller dust must be untouched, before={controller_before} after={controller_after}"
    );
}

/// Reconciles two trace statements that look contradictory: "on repay only
/// `net_repay` is credited to cash, the overpayment is UNCREDITED" and "the
/// overpayment routes controller -> caller".
///
/// Both are true and they describe different layers. The overpayment is
/// uncredited *to the cash book* on purpose — `pool/src/ops/repay.rs:58`
/// credits `net_repay`, which deliberately excludes it — and it is refunded by
/// `transfer_out(payer, overpayment)` at `repay.rs:33`.
///
/// Which address `payer` is depends on the caller:
/// - plain `repay`: `positions/debt.rs:195` passes the user's own address, so
///   the pool refunds the user directly. That is what this test pins.
/// - strategy legs: `execute_repayment` passes `EventContext.counterparty`,
///   which `legs.rs:38-43` sets to the controller, so the pool refunds the
///   controller and `legs.rs:83` `refund_controller_balance_delta` forwards the
///   measured delta on to `caller`.
///
/// Either way the overpayment reaches a real party and the controller keeps
/// nothing.
#[test]
fn repay_overpayment_is_refunded_to_the_payer_not_stranded() {
    let mut t = LendingTest::new().standard_two_asset().build();

    t.set_oracle_single_spot("USDC");
    t.supply(ALICE, "USDC", 10_000.0);
    t.supply(BOB, "ETH", 50.0);
    t.borrow(BOB, "USDC", 1_000.0);

    let asset = t.resolve_asset("USDC");
    let controller = t.controller_address();
    let tok = token::Client::new(&t.env, &asset);
    let bob = t.get_or_create_user(BOB);

    let controller_before = tok.balance(&controller);
    let bob_before = tok.balance(&bob);

    // `repay_raw` mints exactly `overpay` to BOB and then repays all of it, so
    // BOB's net balance change is the refund.
    let overpay = 30_000_000_000i128; // 3000 USDC at 7dp, well above the 1000 debt
    t.repay_raw(BOB, "USDC", overpay);

    let bob_after = tok.balance(&bob);
    let debt_after = t.borrow_balance_for(BOB, t.resolve_account_id(BOB), "USDC");

    assert!(
        bob_after > bob_before,
        "the overpayment must come back to the payer: before={bob_before} after={bob_after}"
    );
    assert!(
        debt_after < 1.0,
        "the repay must have cleared the debt for the overpayment to exist, got {debt_after}"
    );
    assert_eq!(
        tok.balance(&controller),
        controller_before,
        "the controller must not retain any part of a plain-repay overpayment"
    );
}
