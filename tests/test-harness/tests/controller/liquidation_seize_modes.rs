//! End-to-end coverage for the pause split (`no_seize`) and share-credit liquidation
//! (`SeizeMode::Credit`).
//!
//! The scaled split arithmetic is unit-tested in
//! `contracts/controller/tests/positions/liquidation_seize_modes.rs`; everything here needs a
//! real pool, so it lives against the full harness.

use common::types::{
    AccountPositionRaw, ControllerKey, PoolStateRaw, PositionMode, SeizeMode, SpokeUsageRaw,
};
use soroban_sdk::testutils::{ContractEvents, Events};
use soroban_sdk::xdr::{ContractEventBody, ScVal};
use test_harness::{
    assert_contract_error, errors, eth_preset, hub_asset, usd_cents, usdc_preset, LendingTest,
    MarketPreset, ALICE, BOB, CAROL, HARNESS_SPOKE, LIQUIDATOR, STABLECOIN_SPOKE,
};

// --- inspection helpers --------------------------------------------------

fn pool_state(t: &LendingTest, asset_name: &str) -> PoolStateRaw {
    let asset = t.resolve_asset(asset_name);
    t.pool_client(asset_name)
        .get_sync_data(&hub_asset(asset))
        .state
}

fn scaled_supply(t: &LendingTest, account_id: u64, asset_name: &str) -> i128 {
    position(t, account_id, asset_name).map_or(0, |p| p.scaled_amount)
}

fn position(t: &LendingTest, account_id: u64, asset_name: &str) -> Option<AccountPositionRaw> {
    let asset = t.resolve_asset(asset_name);
    t.ctrl_client()
        .get_account_positions(&account_id)
        .0
        .get(hub_asset(asset))
}

fn spoke_supply_usage(t: &LendingTest, asset_name: &str) -> i128 {
    let asset = t.resolve_asset(asset_name);
    t.env.as_contract(&t.controller, || {
        t.env
            .storage()
            .persistent()
            .get::<_, SpokeUsageRaw>(&ControllerKey::SpokeUsage(HARNESS_SPOKE, hub_asset(asset)))
            .map(|u| u.supplied_scaled_ray)
            .unwrap_or(0)
    })
}

fn count_topic(events: &ContractEvents, first: &str, second: &str) -> usize {
    events
        .events()
        .iter()
        .filter(|event| {
            let ContractEventBody::V0(body) = &event.body;
            matches!(
                (body.topics.first(), body.topics.get(1)),
                (Some(ScVal::Symbol(a)), Some(ScVal::Symbol(b)))
                    if a.0.to_string() == first && b.0.to_string() == second
            )
        })
        .count()
}

// --- scenarios -----------------------------------------------------------

/// USDC market with no seeded free cash, so the only USDC the pool holds is what suppliers put
/// in and borrowers have not taken out. That is what makes a cash starve reachable at all: the
/// standard preset pre-mints a million units of unattributed cash.
fn dry_usdc_preset() -> MarketPreset {
    MarketPreset {
        initial_liquidity: 0.0,
        ..usdc_preset()
    }
}

/// Alice is liquidatable on a USDC market drained of cash by Bob's borrow.
///
/// Alice: 10_000 USDC collateral, 3 ETH debt. Bob: 50 ETH collateral, 9_400 USDC borrowed,
/// which leaves the USDC market with far less cash than a seizure would need. USDC then halves
/// in price, which pushes Alice under water and leaves Bob comfortably solvent.
fn cash_starved_usdc() -> LendingTest {
    let mut t = LendingTest::new()
        .with_market(dry_usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    t.supply(BOB, "ETH", 50.0);
    t.borrow(BOB, "USDC", 9_400.0);

    t.set_price("USDC", usd_cents(50));
    t.assert_liquidatable(ALICE);
    t
}

/// Alice is liquidatable in a market with plenty of cash, so both modes are available and can
/// be compared.
fn liquid_usdc() -> LendingTest {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    t.set_price("USDC", usd_cents(50));
    t.assert_liquidatable(ALICE);
    t
}

// --- the headline case ---------------------------------------------------

#[test]
fn cash_starved_market_blocks_transfer_but_not_credit() {
    let mut t = cash_starved_usdc();

    let cash_before = pool_state(&t, "USDC").cash;
    let transfer = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);
    assert_contract_error(transfer, errors::INSUFFICIENT_LIQUIDITY);
    assert_eq!(
        pool_state(&t, "USDC").cash,
        cash_before,
        "the failed transfer must not have moved cash"
    );

    // Same account, same repayment, same seizure — but the collateral is delivered as supply
    // shares, so the pool never has to find the underlying.
    let receiver = t
        .try_liquidate_with_mode(LIQUIDATOR, ALICE, "ETH", 1.0, SeizeMode::Credit(0))
        .expect("credit mode must clear a liquidation the market has no cash for");

    assert!(receiver > 0, "credit mode must return a receiving account");
    assert!(
        scaled_supply(&t, receiver, "USDC") > 0,
        "the liquidator must hold the seized collateral as shares"
    );
    assert!(
        t.borrow_balance(ALICE, "ETH") < 3.0,
        "the debt must actually have been repaid"
    );
    assert_eq!(
        pool_state(&t, "USDC").cash,
        cash_before,
        "credit mode must move no cash at all"
    );
}

// --- pool invariance -----------------------------------------------------

#[test]
fn credit_mode_leaves_supplied_and_cash_untouched_and_moves_only_revenue() {
    let mut t = liquid_usdc();
    let before = pool_state(&t, "USDC");

    let alice_id = t.resolve_account_id(ALICE);
    let alice_before = scaled_supply(&t, alice_id, "USDC");

    let receiver = t.liquidate_with_mode(LIQUIDATOR, ALICE, "ETH", 1.0, SeizeMode::Credit(0));

    let after = pool_state(&t, "USDC");
    assert_eq!(
        after.supplied, before.supplied,
        "a share transfer creates and destroys no supply"
    );
    assert_eq!(after.cash, before.cash, "a share transfer moves no cash");
    assert_eq!(
        after.supply_index, before.supply_index,
        "no interest should have accrued in this scenario"
    );

    // Conservation, end to end: everything the liquidated account lost is now either the
    // receiver's or the protocol's, to the share.
    let seized = alice_before - scaled_supply(&t, alice_id, "USDC");
    let credited = scaled_supply(&t, receiver, "USDC");
    let fee = after.revenue - before.revenue;
    assert!(seized > 0, "collateral must have been seized");
    assert_eq!(
        credited + fee,
        seized,
        "seized shares must equal credited shares plus the protocol fee, exactly"
    );
    assert!(fee > 0, "this fixture has a nonzero liquidation fee rate");
}

#[test]
fn credit_mode_moves_spoke_usage_by_exactly_the_protocol_fee() {
    let mut t = liquid_usdc();
    let usage_before = spoke_supply_usage(&t, "USDC");
    let revenue_before = pool_state(&t, "USDC").revenue;

    t.liquidate_with_mode(LIQUIDATOR, ALICE, "ETH", 1.0, SeizeMode::Credit(0));

    let fee = pool_state(&t, "USDC").revenue - revenue_before;
    // The account-to-account leg is a genuine no-op: both sides are the same spoke and the same
    // hub asset. The only value that leaves the account system is the protocol fee, which is
    // reclassified into revenue exactly as bad-debt cleanup reclassifies an absorbed position.
    assert_eq!(
        spoke_supply_usage(&t, "USDC"),
        usage_before - fee,
        "spoke usage must fall by the fee and by nothing else"
    );
}

// --- receiving-account rules --------------------------------------------

#[test]
fn credit_zero_creates_a_usable_account_owned_by_the_liquidator_in_the_right_spoke() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Two identical borrowers, both opened while USDC is still worth a dollar.
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    t.supply(CAROL, "USDC", 10_000.0);
    t.borrow(CAROL, "ETH", 3.0);
    t.set_price("USDC", usd_cents(50));
    t.assert_liquidatable(ALICE);
    t.assert_liquidatable(CAROL);

    let liquidator = t.get_or_create_user(LIQUIDATOR);
    let alice_spoke = t.get_account_attributes(ALICE).spoke_id;

    let receiver = t.liquidate_with_mode(LIQUIDATOR, ALICE, "ETH", 1.0, SeizeMode::Credit(0));

    assert!(t.account_exists(receiver));
    assert_eq!(t.get_account_owner(receiver), liquidator);
    let attrs = t.ctrl_client().get_account_attributes(&receiver);
    assert_eq!(attrs.spoke_id, alice_spoke, "receiver must share the spoke");
    assert_eq!(attrs.mode, PositionMode::Normal);

    // The returned id is usable: a second credit-mode liquidation can target it directly and
    // adds to the position already there.
    let first_credit = scaled_supply(&t, receiver, "USDC");
    let same = t.liquidate_with_mode(LIQUIDATOR, CAROL, "ETH", 1.0, SeizeMode::Credit(receiver));
    assert_eq!(same, receiver);
    assert!(scaled_supply(&t, receiver, "USDC") > first_credit);
}

#[test]
fn credit_to_an_account_in_another_spoke_reverts() {
    // A second spoke listing the same asset, so only the binding differs between the two
    // candidate receivers.
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    t.set_price("USDC", usd_cents(50));
    t.assert_liquidatable(ALICE);

    let elsewhere = t.create_spoke_account(LIQUIDATOR, 2);
    let result =
        t.try_liquidate_with_mode(LIQUIDATOR, ALICE, "ETH", 1.0, SeizeMode::Credit(elsewhere));
    assert_contract_error(result, errors::SPOKE_MISMATCH);
}

#[test]
fn credit_to_an_account_the_liquidator_does_not_control_reverts() {
    let mut t = liquid_usdc();
    let bobs = t.create_account(BOB);

    let result = t.try_liquidate_with_mode(LIQUIDATOR, ALICE, "ETH", 1.0, SeizeMode::Credit(bobs));
    assert_contract_error(result, errors::NOT_AUTHORIZED);
}

#[test]
fn credit_back_into_the_liquidated_account_reverts() {
    let mut t = liquid_usdc();
    let alice_id = t.resolve_account_id(ALICE);

    let result =
        t.try_liquidate_with_mode(LIQUIDATOR, ALICE, "ETH", 1.0, SeizeMode::Credit(alice_id));
    assert_contract_error(result, errors::SELF_LIQUIDATION_NOT_ALLOWED);
}

#[test]
fn credit_to_a_strategy_mode_account_reverts() {
    let mut t = liquid_usdc();
    let multiply = t.create_account_full(LIQUIDATOR, HARNESS_SPOKE, PositionMode::Multiply);

    let result =
        t.try_liquidate_with_mode(LIQUIDATOR, ALICE, "ETH", 1.0, SeizeMode::Credit(multiply));
    assert_contract_error(result, errors::ACCOUNT_MODE_MISMATCH);
}

#[test]
fn credit_to_a_missing_account_reverts() {
    let mut t = liquid_usdc();
    let result = t.try_liquidate_with_mode(LIQUIDATOR, ALICE, "ETH", 1.0, SeizeMode::Credit(9_999));
    assert_contract_error(result, errors::ACCOUNT_NOT_FOUND);
}

// --- risk tuple on arrival ----------------------------------------------

#[test]
fn a_receiver_without_a_position_gets_the_current_listing_tuple_not_the_victims() {
    let mut t = liquid_usdc();
    let alice_id = t.resolve_account_id(ALICE);
    let alice_position = position(&t, alice_id, "USDC").expect("alice holds USDC");

    // Move the listing away from what Alice's position was stamped with, so importing her
    // stale tuple would be visible.
    t.edit_asset_config("USDC", |c| {
        c.loan_to_value = 4_000;
        c.liquidation_threshold = 5_000;
    });
    let listing = t.get_asset_config("USDC");
    assert_ne!(
        alice_position.loan_to_value, listing.loan_to_value,
        "the fixture must actually diverge for this test to mean anything"
    );

    let receiver = t.liquidate_with_mode(LIQUIDATOR, ALICE, "ETH", 1.0, SeizeMode::Credit(0));
    let credited = position(&t, receiver, "USDC").expect("receiver holds USDC");

    assert_eq!(credited.loan_to_value, listing.loan_to_value);
    assert_eq!(
        credited.liquidation_threshold,
        listing.liquidation_threshold
    );
    assert_ne!(
        credited.loan_to_value, alice_position.loan_to_value,
        "the liquidated account's stale tuple must never travel with the shares"
    );
}

#[test]
fn a_receiver_with_a_position_keeps_its_own_tuple_and_just_grows() {
    let mut t = liquid_usdc();
    // Give the liquidator a USDC position stamped under today's listing.
    t.supply(LIQUIDATOR, "USDC", 1_000.0);
    let receiver = t.resolve_account_id(LIQUIDATOR);
    let before = position(&t, receiver, "USDC").expect("liquidator holds USDC");

    // Now move the listing. An ordinary supply would not restamp an existing position either.
    t.edit_asset_config("USDC", |c| {
        c.loan_to_value = 4_000;
        c.liquidation_threshold = 5_000;
    });

    t.liquidate_with_mode(LIQUIDATOR, ALICE, "ETH", 1.0, SeizeMode::Credit(receiver));
    let after = position(&t, receiver, "USDC").expect("liquidator still holds USDC");

    assert_eq!(after.loan_to_value, before.loan_to_value);
    assert_eq!(after.liquidation_threshold, before.liquidation_threshold);
    assert_eq!(after.liquidation_bonus, before.liquidation_bonus);
    assert_eq!(after.liquidation_fees, before.liquidation_fees);
    assert!(
        after.scaled_amount > before.scaled_amount,
        "the credit must have added to the existing position"
    );
}

// --- entry gates that must NOT apply ------------------------------------

#[test]
fn a_non_collateralizable_asset_can_still_be_credited() {
    let mut t = liquid_usdc();
    // Turning collateral off blocks new supply, but a seizure is not a supply.
    t.edit_asset_config("USDC", |c| c.is_collateralizable = false);

    let receiver = t.liquidate_with_mode(LIQUIDATOR, ALICE, "ETH", 1.0, SeizeMode::Credit(0));
    assert!(
        scaled_supply(&t, receiver, "USDC") > 0,
        "seizure must not be gated on the supply entry rule"
    );
}

#[test]
fn a_spoke_at_its_supply_cap_can_still_be_credited() {
    let mut t = liquid_usdc();
    // Drop the supply cap far below current usage. A new supply would be rejected; a
    // liquidation must not be, or an account in a spoke sitting at its cap becomes
    // unliquidatable in credit mode.
    let cfg = t.get_asset_config("USDC");
    let asset = t.resolve_asset("USDC");
    t.ctrl_client()
        .edit_asset_in_spoke(&controller::types::SpokeAssetArgs {
            hub_id: test_harness::HARNESS_HUB,
            asset,
            spoke_id: HARNESS_SPOKE,
            can_collateral: cfg.is_collateralizable,
            can_borrow: cfg.is_borrowable,
            paused: false,
            frozen: false,
            no_seize: false,
            ltv: cfg.loan_to_value,
            threshold: cfg.liquidation_threshold,
            bonus: cfg.liquidation_bonus,
            liquidation_fees: cfg.liquidation_fees,
            supply_cap: 1,
            borrow_cap: cfg.borrow_cap,
        });

    let receiver = t.liquidate_with_mode(LIQUIDATOR, ALICE, "ETH", 1.0, SeizeMode::Credit(0));
    assert!(scaled_supply(&t, receiver, "USDC") > 0);
}

// --- position limits -----------------------------------------------------

#[test]
fn a_receiver_at_the_supply_position_limit_reverts() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_position_limits(1, 4)
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    // The liquidator's account already holds its one permitted supply position, in a different
    // asset than the one about to be seized.
    t.supply(LIQUIDATOR, "ETH", 5.0);
    let receiver = t.resolve_account_id(LIQUIDATOR);

    t.set_price("USDC", usd_cents(50));
    t.assert_liquidatable(ALICE);

    let result =
        t.try_liquidate_with_mode(LIQUIDATOR, ALICE, "ETH", 1.0, SeizeMode::Credit(receiver));
    assert_contract_error(result, errors::POSITION_LIMIT_EXCEEDED);

    // The revert is actionable: a fresh account has room.
    let fresh = t.liquidate_with_mode(LIQUIDATOR, ALICE, "ETH", 1.0, SeizeMode::Credit(0));
    assert!(scaled_supply(&t, fresh, "USDC") > 0);
}

// --- events --------------------------------------------------------------

#[test]
fn credit_mode_emits_two_position_batches_liquidated_account_first() {
    let mut t = liquid_usdc();
    let alice_id = t.resolve_account_id(ALICE);

    let receiver = t.liquidate_with_mode(LIQUIDATOR, ALICE, "ETH", 1.0, SeizeMode::Credit(0));

    let events = t.env.events().all();
    assert_eq!(
        count_topic(&events, "position", "batch_update"),
        2,
        "credit mode writes two accounts, so it must publish two position batches"
    );

    let ids = batch_account_ids(&events);
    assert_eq!(
        ids,
        std::vec![alice_id, receiver],
        "the liquidated account's batch must come first"
    );
}

#[test]
fn transfer_mode_still_emits_a_single_position_batch() {
    let mut t = liquid_usdc();
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);
    assert_eq!(
        count_topic(&t.env.events().all(), "position", "batch_update"),
        1
    );
}

/// Account ids from each `UpdatePositionBatchEvent`, in emission order.
fn batch_account_ids(events: &ContractEvents) -> std::vec::Vec<u64> {
    events
        .events()
        .iter()
        .filter_map(|event| {
            let ContractEventBody::V0(body) = &event.body;
            match (body.topics.first(), body.topics.get(1)) {
                (Some(ScVal::Symbol(a)), Some(ScVal::Symbol(b)))
                    if a.0.to_string() == "position" && b.0.to_string() == "batch_update" =>
                {
                    match &body.data {
                        ScVal::Vec(Some(entries)) => match entries.0.first() {
                            Some(ScVal::U64(id)) => Some(*id),
                            other => panic!("expected a u64 account id, got {:?}", other),
                        },
                        other => panic!("expected ScVal::Vec, got {:?}", other),
                    }
                }
                _ => None,
            }
        })
        .collect()
}

// --- estimate view -------------------------------------------------------

#[test]
fn the_estimate_reports_the_units_the_chosen_mode_moves() {
    let mut t = liquid_usdc();
    let alice_id = t.resolve_account_id(ALICE);
    let payments = test_harness::asset_payment_vec(
        &t.env,
        t.resolve_asset("ETH"),
        test_harness::amount_raw(1.0, t.resolve_market("ETH").decimals),
    );

    let transfer =
        t.ctrl_client()
            .get_liquidation_estimate(&alice_id, &payments, &SeizeMode::Transfer);
    let credit =
        t.ctrl_client()
            .get_liquidation_estimate(&alice_id, &payments, &SeizeMode::Credit(0));

    let transfer_amount = transfer.seized_collaterals.get(0).unwrap().amount;
    let credit_amount = credit.seized_collaterals.get(0).unwrap().amount;
    assert!(transfer_amount > 0 && credit_amount > 0);
    assert!(
        credit_amount > transfer_amount,
        "credit mode reports RAY-scaled shares, which are far larger than 7-decimal asset units"
    );

    // And the credit estimate matches what execution actually moves.
    let alice_before = scaled_supply(&t, alice_id, "USDC");
    let receiver = t.liquidate_with_mode(LIQUIDATOR, ALICE, "ETH", 1.0, SeizeMode::Credit(0));
    let moved = alice_before - scaled_supply(&t, alice_id, "USDC");
    assert_eq!(moved, credit_amount, "estimate must match execution");
    assert_eq!(
        scaled_supply(&t, receiver, "USDC"),
        credit_amount - credit.protocol_fees.get(0).unwrap().amount,
        "the liquidator receives the seizure minus the reported fee"
    );
}

// --- bad debt ------------------------------------------------------------

#[test]
fn bad_debt_promotion_still_fires_after_a_credit_mode_liquidation() {
    // Same shape the existing bad-debt suite uses: a dust-sized borrower whose collateral
    // collapses far enough that the residual clears the socialization gate.
    let mut t = LendingTest::new().standard_two_asset_dust_disabled();
    t.supply(BOB, "ETH", 100.0);
    t.supply(ALICE, "USDC", 10.0);
    t.borrow(ALICE, "ETH", 0.003);

    t.set_price("USDC", usd_cents(10));
    t.assert_liquidatable(ALICE);

    let receiver = t.liquidate_with_mode(LIQUIDATOR, ALICE, "ETH", 0.001, SeizeMode::Credit(0));
    // Read the ledger's events before any view call: a later invocation replaces the buffer.
    let events = t.env.events().all();

    assert_eq!(
        count_topic(&events, "debt", "bad_debt"),
        1,
        "bad-debt cleanup must still publish"
    );
    assert!(
        scaled_supply(&t, receiver, "USDC") > 0,
        "the liquidator still takes the collateral that was there"
    );
    assert!(
        t.find_account_id(ALICE).is_none(),
        "the insolvent account must have been socialized and removed"
    );
}

// --- pause split ---------------------------------------------------------

#[test]
fn a_paused_collateral_can_still_be_seized() {
    let mut t = liquid_usdc();
    // Pausing USDC used to halt liquidation of every account holding it, because seizure is
    // pro-rata across the whole collateral set.
    t.set_spoke_asset_flags("USDC", true, false, false);

    let coll_before = t.supply_balance(ALICE, "USDC");
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);
    assert!(
        t.supply_balance(ALICE, "USDC") < coll_before,
        "a paused listing must not block the seizure leg"
    );
}

#[test]
fn a_paused_collateral_can_still_be_seized_into_credit_mode() {
    let mut t = liquid_usdc();
    t.set_spoke_asset_flags("USDC", true, false, false);

    let receiver = t.liquidate_with_mode(LIQUIDATOR, ALICE, "ETH", 1.0, SeizeMode::Credit(0));
    assert!(scaled_supply(&t, receiver, "USDC") > 0);
}

#[test]
fn no_seize_blocks_the_seizure_leg_in_both_modes() {
    let mut t = liquid_usdc();
    t.set_spoke_asset_flags("USDC", false, false, true);

    assert_contract_error(
        t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 1.0),
        errors::SPOKE_ASSET_SEIZURE_HALTED,
    );
    assert_contract_error(
        t.try_liquidate_with_mode(LIQUIDATOR, ALICE, "ETH", 1.0, SeizeMode::Credit(0))
            .map(|_| ()),
        errors::SPOKE_ASSET_SEIZURE_HALTED,
    );
}

#[test]
fn a_paused_debt_asset_is_opt_in_and_only_blocks_when_named() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Two debts; only one of them gets paused.
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    t.set_price("USDC", usd_cents(50));
    t.assert_liquidatable(ALICE);

    t.set_spoke_asset_flags("ETH", true, false, false);
    // The debt side is chosen by the liquidator, so naming a paused asset reverts...
    assert_contract_error(
        t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 1.0),
        errors::SPOKE_ASSET_PAUSED,
    );
}

#[test]
fn no_seize_does_not_block_ordinary_withdrawal() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.set_spoke_asset_flags("USDC", false, false, true);

    // `no_seize` governs the seizure leg only; users keep their exits.
    t.withdraw(ALICE, "USDC", 1_000.0);
    assert!(t.supply_balance(ALICE, "USDC") < 10_000.0);
}
