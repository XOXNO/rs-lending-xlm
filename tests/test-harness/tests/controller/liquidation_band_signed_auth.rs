//! A simulate-and-sign liquidator in the band `D <= C < D * (1 + base)`, under
//! ENFORCED authorization. The transfer amount is recorded at simulation and
//! interest accrues before execution.

use common::types::{HubAssetKey, SeizeMode};
use soroban_sdk::testutils::{AuthorizedFunction, MockAuth, MockAuthInvoke};
use soroban_sdk::xdr::ScErrorType;
use soroban_sdk::{symbol_short, vec, Address, IntoVal, TryFromVal, Vec};
use test_harness::{hub_asset, usd_cents, LendingTest, ALICE, LIQUIDATOR};

const DEBT: i128 = 30_000_000;
const OVER_OFFER: i128 = 40_000_000;
const PARTIAL: i128 = 10_000_000;

/// 10 000 USDC at `usdc_cents` against 3 ETH ($6 000). At 62 cents the
/// account sits in the band (cap 333 bps < 500 base); at 50 cents it is
/// insolvent.
fn account_at(usdc_cents: i128) -> (LendingTest, u64, Address) {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    t.set_price("USDC", usd_cents(usdc_cents));
    let liquidator = t.get_or_create_user(LIQUIDATOR);
    t.resolve_market("ETH")
        .token_admin
        .mint(&liquidator, &(2 * OVER_OFFER));
    let account_id = t.resolve_account_id(ALICE);
    (t, account_id, liquidator)
}

fn payments(t: &LendingTest, amount: i128) -> Vec<(HubAssetKey, i128)> {
    vec![&t.env, (hub_asset(t.resolve_asset("ETH")), amount)]
}

/// Runs `liquidate(offer)` in recording mode, as a simulation does, on a fresh
/// copy of the book and returns the `transfer` amount the liquidator signs.
fn simulated_transfer_amount(usdc_cents: i128, offer: i128) -> i128 {
    let (t, account_id, liquidator) = account_at(usdc_cents);
    t.env.mock_all_auths();
    t.ctrl_client().liquidate(
        &liquidator,
        &account_id,
        &payments(&t, offer),
        &SeizeMode::Transfer,
    );
    let recorded = t.env.auths();
    assert_eq!(recorded.len(), 1, "one signer: {recorded:#?}");
    let (signer, root) = &recorded[0];
    assert_eq!(signer, &liquidator);
    assert_eq!(root.sub_invocations.len(), 1, "one transfer: {root:#?}");
    let AuthorizedFunction::Contract((_, name, args)) = &root.sub_invocations[0].function else {
        panic!("the nested call is a contract call: {root:#?}");
    };
    assert_eq!(name, &symbol_short!("transfer"));
    i128::try_from_val(&t.env, &args.get(2).expect("transfer amount"))
        .expect("the amount is an i128")
}

/// `liquidate(offer)` signed by the liquidator with exactly one `transfer(signed)` beneath it.
fn liquidate_with_signed_tree(
    t: &LendingTest,
    liquidator: &Address,
    account_id: u64,
    offer: i128,
    signed: i128,
) -> Result<u64, soroban_sdk::Error> {
    let offered = payments(t, offer);
    let transfer = MockAuthInvoke {
        contract: &t.resolve_asset("ETH"),
        fn_name: "transfer",
        args: (
            liquidator.clone(),
            t.resolve_market("ETH").pool.clone(),
            signed,
        )
            .into_val(&t.env),
        sub_invokes: &[],
    };
    let liquidate = MockAuthInvoke {
        contract: &t.controller,
        fn_name: "liquidate",
        args: (
            liquidator.clone(),
            account_id,
            offered.clone(),
            SeizeMode::Transfer,
        )
            .into_val(&t.env),
        sub_invokes: core::slice::from_ref(&transfer),
    };
    let auths = [MockAuth {
        address: liquidator,
        invoke: &liquidate,
    }];
    match t.ctrl_client().mock_auths(&auths).try_liquidate(
        liquidator,
        &account_id,
        &offered,
        &SeizeMode::Transfer,
    ) {
        Ok(Ok(id)) => Ok(id),
        Ok(Err(e)) => panic!("conversion error: {e:?}"),
        Err(Ok(e)) => Err(e),
        Err(Err(e)) => panic!("invoke error: {e:?}"),
    }
}

/// Execution-time debt: the offer minus the refund the estimate reports.
fn debt_now(t: &LendingTest, account_id: u64) -> i128 {
    let estimate = t.ctrl_client().get_liquidation_estimate(
        &account_id,
        &payments(t, OVER_OFFER),
        &SeizeMode::Transfer,
    );
    OVER_OFFER
        - estimate
            .refunds
            .get(0)
            .expect("over-offer is refunded")
            .amount
}

#[test]
fn a_signed_band_over_offer_closes_after_interest_and_refunds_the_excess() {
    let signed = simulated_transfer_amount(62, OVER_OFFER);
    assert_eq!(signed, OVER_OFFER, "a full-close plan pulls the offer");

    let (mut t, account_id, liquidator) = account_at(62);
    t.advance_time(5);
    let debt = debt_now(&t, account_id);
    assert!(debt > DEBT, "interest accrued after the simulation");

    let before = t.token_balance_raw(LIQUIDATOR, "ETH");
    liquidate_with_signed_tree(&t, &liquidator, account_id, OVER_OFFER, signed)
        .expect("the signed transfer still matches");
    assert_eq!(t.borrow_balance_raw(ALICE, "ETH"), 0);
    assert_eq!(
        before - t.token_balance_raw(LIQUIDATOR, "ETH"),
        debt,
        "the pool refunds everything above the execution-time debt"
    );
}

#[test]
fn a_signed_exact_debt_offer_in_the_band_repays_all_but_the_accrued_interest() {
    let signed = simulated_transfer_amount(62, DEBT);
    assert_eq!(signed, DEBT);

    let (mut t, account_id, liquidator) = account_at(62);
    t.advance_time(5);
    let debt = debt_now(&t, account_id);

    let before = t.token_balance_raw(LIQUIDATOR, "ETH");
    liquidate_with_signed_tree(&t, &liquidator, account_id, DEBT, signed)
        .expect("a partial in the band is accepted");
    assert_eq!(before - t.token_balance_raw(LIQUIDATOR, "ETH"), DEBT);
    assert_eq!(debt - DEBT, 1, "five seconds of interest ceil to one unit");
    assert!(
        t.borrow_balance_raw(ALICE, "ETH") <= 1,
        "only the accrued interest is left"
    );
}

#[test]
fn a_signed_band_partial_pulls_exactly_the_signed_amount_after_interest() {
    let signed = simulated_transfer_amount(62, PARTIAL);
    assert_eq!(signed, PARTIAL);

    let (mut t, account_id, liquidator) = account_at(62);
    t.advance_time(5);
    let coverage = t.total_collateral_raw(ALICE) as f64 / t.total_debt_raw(ALICE) as f64;

    let before = t.token_balance_raw(LIQUIDATOR, "ETH");
    let usdc_before = t.token_balance_raw(LIQUIDATOR, "USDC");
    liquidate_with_signed_tree(&t, &liquidator, account_id, PARTIAL, signed)
        .expect("a partial in the band is accepted");
    assert_eq!(before - t.token_balance_raw(LIQUIDATOR, "ETH"), PARTIAL);
    assert!(
        t.token_balance_raw(LIQUIDATOR, "USDC") > usdc_before,
        "the partial seizes collateral"
    );
    let coverage_after = t.total_collateral_raw(ALICE) as f64 / t.total_debt_raw(ALICE) as f64;
    assert!(
        coverage_after >= coverage,
        "a band partial must not lower coverage: {coverage} -> {coverage_after}"
    );
}

/// On an insolvent account the plan caps an over-offer at what the collateral
/// backs, so the pull differs from the signed amount and the host rejects it:
/// a racing liquidator reverts instead of paying the whole debt for less.
#[test]
fn a_signed_insolvent_over_offer_fails_authorization_instead_of_overpaying() {
    let (t, account_id, liquidator) = account_at(50);
    let debt_before = t.borrow_balance_raw(ALICE, "ETH");
    let err = liquidate_with_signed_tree(&t, &liquidator, account_id, DEBT, DEBT)
        .expect_err("the capped pull no longer matches the signed transfer");
    assert!(
        err.is_type(ScErrorType::Auth) || err.is_type(ScErrorType::Context),
        "expected a host auth failure, got {err:?}"
    );
    let log = std::format!("{:?}", t.env.host().get_events().unwrap().0);
    assert!(
        log.contains("Unauthorized function call for address"),
        "the failure must be the unauthorized token transfer"
    );
    assert_eq!(t.borrow_balance_raw(ALICE, "ETH"), debt_before);
}
