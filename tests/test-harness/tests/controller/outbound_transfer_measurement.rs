//! Where the difference lands when an asset delivers an inexact amount on an
//! outbound leg: the recapitalize refund (`pool/src/ops/recapitalize.rs`), the
//! revenue claim's forward to the accumulator (`controller/src/markets.rs`,
//! measured on both hops), and the repay overpayment refund.

use crate::shared::{count_topic, data_for_topic};
use soroban_sdk::{testutils::Events, token, xdr::ScVal};
use test_harness::{
    eth_preset, hub_asset, usdc_preset, weird_token::WeirdTokenClient, LendingTest, ALICE, BOB,
    CAROL,
};

/// Reads the `amount` field (an `i128`) out of a `revenue:claim` event payload.
fn claim_event_amount(data: &ScVal) -> i128 {
    let ScVal::Map(Some(entries)) = data else {
        panic!("expected ScVal::Map for revenue:claim, got {data:?}");
    };
    for entry in entries.0.iter() {
        if let ScVal::Symbol(key) = &entry.key {
            if key.to_string() == "amount" {
                let ScVal::I128(parts) = &entry.val else {
                    panic!("revenue:claim amount must be i128, got {:?}", entry.val);
                };
                return ((parts.hi as i128) << 64) | (parts.lo as i128);
            }
        }
    }
    panic!("revenue:claim payload has no `amount` field");
}

/// A fee-on-transfer recapitalize into a healthy market strands nothing in the
/// protocol. The pool refunds the whole receipt, so its balance and cash book
/// return to their start values. The payer loses only the token's haircut on
/// each of the two hops.
///
/// `transfer_out` moves the declared `refund` out of the pool whatever the payer
/// receives, so the pool cannot retain the difference.
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

/// The controller measures what it receives from the pool and forwards exactly
/// that (`claim_revenue_for_asset`), so an under-delivering asset cannot drain a
/// stranded controller balance. The accumulator is short only by the forward
/// transfer's own haircut.
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
    // Captured before the balance reads below: `events().all()` is scoped to
    // the last contract invocation, and every `tok.balance` call is one.
    let claim_events = t.env.events().all();
    assert!(
        claimed > 0,
        "a positive claim is what makes this observable"
    );

    let accumulator_got = tok.balance(&accumulator);
    let controller_after = tok.balance(&controller);

    assert_eq!(
        accumulator_got,
        claimed - claimed / 100,
        "accumulator receives one forward-hop haircut less than the measured claim"
    );

    assert_eq!(
        controller_after, controller_before,
        "controller dust must be untouched, before={controller_before} after={controller_after}"
    );

    // The controller publishes the measured receipt, not the pool's larger
    // reported figure: indexers sum this number into lifetime revenue.
    let payloads = data_for_topic(&claim_events, "revenue", "claim");
    assert_eq!(payloads.len(), 1, "one claim, one revenue:claim event");
    assert_eq!(
        claim_event_amount(&payloads[0]),
        claimed,
        "revenue:claim must report the measured forward, not the pool's reported amount"
    );
}

/// A claim with no revenue emits no `revenue:claim` event, so a keeper that
/// claims every market on a timer adds no indexer rows for empty markets.
#[test]
fn claim_revenue_emits_nothing_when_there_is_no_revenue() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let accumulator = t
        .env
        .register(test_harness::mock_reflector::MockReflector, ());
    t.set_accumulator(&accumulator);

    assert_eq!(t.claim_revenue("USDC"), 0, "fixture must accrue nothing");
    // Captured immediately: `events().all()` is scoped to the last invocation.
    let events = t.env.events().all();
    assert!(
        count_topic(&events, "market", "batch_state_update") > 0,
        "guard: the claim's own events must be in this window, or the \
         assertion below passes vacuously"
    );
    assert_eq!(
        count_topic(&events, "revenue", "claim"),
        0,
        "a zero claim must not emit revenue:claim"
    );
}

/// A plain `repay` overpayment is refunded to the payer, and the controller
/// keeps nothing.
///
/// The pool credits only `net_repay` to cash and refunds the overpayment with
/// `transfer_out(payer, overpayment)` (`pool/src/ops/repay.rs`). For plain
/// `repay`, `payer` is the user; this test pins that path. Strategy legs pass
/// the controller as `refund_to` (`repay_debt_from_controller`), and
/// `refund_controller_balance_delta` forwards the measured delta to `caller`.
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

/// A recapitalize into an over-delivering market moves pool custody and the cash
/// book by the same `applied` amount.
///
/// The refund basis is `received`, the pool's measured balance delta
/// (`transfer_amount_measured`), so the refund never exceeds what the pool
/// received. The payer's gain comes from the token minting on each hop, not
/// from protocol custody.
#[test]
fn recapitalize_into_an_over_delivering_market_keeps_book_and_custody_in_step() {
    let mut t = LendingTest::new()
        .with_extra_credit_market(usdc_preset(), 100)
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

    assert_eq!(applied, 0, "a backed market must apply nothing");

    let custody_delta = tok.balance(&pool) - pool_before;
    let book_delta = t.pool_client("USDC").get_reserves(&key) - cash_before;
    assert_eq!(
        custody_delta, applied,
        "pool custody must move by exactly `applied`, got {custody_delta}"
    );
    assert_eq!(
        book_delta, applied,
        "the cash book must move by exactly `applied`, got {book_delta}"
    );
    assert_eq!(
        tok.balance(&controller),
        controller_before,
        "the controller must retain nothing"
    );

    // in: the pool is credited 101% of `amount`. out: the pool sends that
    // measured receipt and the payer is credited 101% of it.
    let received = amount + amount / 100;
    let refunded_to_payer = received + received / 100;
    assert_eq!(
        tok.balance(&payer) - payer_before,
        refunded_to_payer - amount,
        "the payer's gain is exactly the token's own two-hop inflation"
    );
    assert!(
        refunded_to_payer > amount,
        "guard: the fixture must actually over-deliver, or this proves nothing"
    );
}

/// A token that re-enters the controller during the measured transfer cannot
/// inflate `received`, and the recapitalize reverts.
///
/// `transfer_amount_measured` reads the pool balance before and after one
/// `transfer` call, so a pool credit made inside that call would count as
/// receipt. The transfer-hook asset calls `controller.supply` as `from` after
/// every transfer. The host refuses that call because the controller is already
/// on the call stack (`ContractReentryMode::Prohibited`). The flash-loan guard
/// (`require_not_flash_loaning`) does not apply: no flash loan is in flight.
#[test]
fn recapitalize_fails_closed_when_the_asset_reenters_during_the_measured_window() {
    let mut t = LendingTest::new()
        .with_transfer_hook_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let payer = t.get_or_create_user(ALICE);
    let asset = t.resolve_asset("USDC");
    let pool = t.resolve_market("USDC").pool.clone();
    let key = hub_asset(asset.clone());
    let tok = token::Client::new(&t.env, &asset);

    let amount = 10_000_000_000i128;
    WeirdTokenClient::new(&t.env, &asset).mint(&payer, &amount);

    let payer_before = tok.balance(&payer);
    let pool_before = tok.balance(&pool);
    let cash_before = t.pool_client("USDC").get_reserves(&key);

    let outcome = t.ctrl_client().try_recapitalize(&payer, &key, &amount);

    assert!(
        outcome.is_err(),
        "a re-entrant asset must not be able to settle a recapitalize"
    );
    assert_eq!(
        tok.balance(&payer),
        payer_before,
        "the reverted call must leave the payer whole"
    );
    assert_eq!(
        tok.balance(&pool),
        pool_before,
        "the reverted call must leave pool custody untouched"
    );
    assert_eq!(
        t.pool_client("USDC").get_reserves(&key),
        cash_before,
        "the reverted call must leave the cash book untouched"
    );
}
