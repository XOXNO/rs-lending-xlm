//! Where the shortfall lands when an asset under-delivers on an outbound leg:
//! the recapitalize refund (`pool/src/ops/recapitalize.rs`) and the revenue
//! claim's forward to the accumulator (`controller/src/keepers.rs`, now measured
//! on both hops after F-8).

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
    // Captured before the balance reads below: `events().all()` is scoped to
    // the LAST contract invocation, and every `tok.balance` call is one.
    let claim_events = t.env.events().all();
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

    // The event must carry the MEASURED receipt, which is the whole reason it
    // is published from the controller rather than the pool: on this
    // fee-on-transfer market the pool's reported figure is strictly larger, so
    // an event emitted at the burn site would overstate lifetime revenue on
    // every claim. Indexers accumulate this number, so it has to be the one
    // that actually moved.
    let payloads = data_for_topic(&claim_events, "revenue", "claim");
    assert_eq!(payloads.len(), 1, "one claim, one revenue:claim event");
    assert_eq!(
        claim_event_amount(&payloads[0]),
        claimed,
        "revenue:claim must report the measured forward, not the pool's reported amount"
    );
}

/// A claim that finds nothing to sweep must stay silent. A keeper walking every
/// market on a timer would otherwise write a row per empty market forever, and
/// the indexer sums these into lifetime revenue.
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

/// The mirror of the fee-on-transfer case above, and the direction the suite
/// did not cover: an asset that OVER-delivers, crediting the recipient more
/// than was sent. This is the direction that could let a payer extract value
/// from `recapitalize`, because the refund basis is `received` -- the measured
/// inbound delta -- and not what the payer actually paid.
///
/// It does not. `transfer_amount_measured` measures the POOL's balance delta
/// (`common/src/token.rs:29-33`), so over the whole call the pool's balance
/// moves by `received - refund`, which is exactly `applied` -- the same figure
/// that lands in the cash book. The payer's windfall here comes from the token
/// inflating its own supply on each hop, not out of protocol custody.
///
/// That identity is why the unmeasured refund at
/// `contracts/pool/src/ops/recapitalize.rs:34` is not reachable as a drain
/// through the honest controller path: whatever the asset does on the way in,
/// the refund is capped by what the pool actually received.
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

    // The load-bearing identity: custody moves by exactly what the book moves
    // by, even though the asset over-delivered on both hops.
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
        custody_delta, book_delta,
        "book and custody must stay in step under an over-delivering asset"
    );
    assert_eq!(
        tok.balance(&controller),
        controller_before,
        "the controller must retain nothing"
    );

    // The payer does profit -- but from the token minting on each hop, not from
    // the pool. in: pool is credited 101% of `amount`. out: the pool sends that
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

/// The other way to inflate `received`: move tokens into the pool from inside
/// the measured window. `transfer_amount_measured` brackets only the single
/// `tok.transfer` call (`common/src/token.rs:29-31`), so anything the asset
/// does *during* that transfer lands between the two balance reads and is
/// counted as part of the payer's receipt.
///
/// The transfer-hook asset does exactly that: after every transfer it calls
/// `controller.supply` as `from`. If that re-entry succeeded during a
/// recapitalize it would credit the pool inside the window and inflate the
/// refund basis. It does not: the controller is already on the frame stack, and
/// the Soroban host runs cross-contract calls under
/// `ContractReentryMode::Prohibited`, so the hook's call is refused before
/// dispatch and the whole recapitalize reverts.
///
/// Note what is NOT holding this closed: `require_not_flash_loaning`
/// (`contracts/controller/src/keepers.rs`) only fires while a flash loan is in
/// flight, and there is none here. The defence is the host's, which is why it
/// is worth a test rather than an argument.
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
