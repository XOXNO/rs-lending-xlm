//! End-to-end coverage for the pause split (`no_seize`) and share-credit liquidation
//! (`SeizeMode::Credit`).
//!
//! The scaled split arithmetic is unit-tested in
//! `contracts/controller/tests/positions/liquidation_seize_modes.rs`; everything here needs a
//! real pool, so it lives against the full harness.

use crate::shared::count_topic;
use common::types::{
    AccountPositionRaw, ControllerKey, PoolStateRaw, PositionMode, SeizeMode, SpokeUsageRaw,
};
use soroban_sdk::testutils::{ContractEvents, Events};
use soroban_sdk::xdr::{ContractEventBody, ScVal};
use soroban_sdk::{Env, U256};
use test_harness::{
    assert_contract_error, errors, eth_preset, hub_asset, usd, usd_cents, usdc_preset, LendingTest,
    MarketPreset, ALICE, BOB, CAROL, DAVE, HARNESS_SPOKE, LIQUIDATOR, STABLECOIN_SPOKE,
};

// --- inspection helpers --------------------------------------------------

/// Returns one asset unit in supply shares at `supply_index`:
/// `10^(27-decimals) * RAY / supply_index`. This is the share gap bound between the two seize
/// modes under full delivery.
///
/// Transfer floors the seizure to asset units, then ceils the share burn (`resolve_withdrawal`).
/// Credit floors once in RAY share space. With `v` the seizure value, `K` one asset unit of
/// value, `c = 1 / index` and `r = v mod K`, transfer burns `ceil((v-r)*c)` and credit
/// `floor(v*c)`, so `credit - transfer < K*c` and `transfer - credit < 2` raw shares.
///
/// The bound follows the live index, so it must not be a constant. Bad-debt socialization
/// (`apply_bad_debt_to_supply_index`) can lower the index to `SUPPLY_INDEX_FLOOR_RAW`
/// (`RAY / 1_000`), where one asset unit is 1e23 shares, not 1e20.
///
/// Under-delivery adds a second floor (`scale_seizures_to_received`) and doubles the bound;
/// use `UNDER_DELIVERY_SLACK_UNITS` there. Returns f64 for messages and ratios only;
/// `share_gap_under_units` does the exact compare.
fn seize_mode_share_slack(supply_index: i128, decimals: u32) -> f64 {
    let shares_per_unit_at_ray = 10f64.powi(27 - decimals as i32);
    shares_per_unit_at_ray * common::constants::RAY as f64 / supply_index as f64
}

/// Exact form of `delta < units * seize_mode_share_slack(supply_index, decimals)`.
fn share_gap_under_units(
    env: &Env,
    delta: i128,
    supply_index: i128,
    decimals: u32,
    units: u32,
) -> bool {
    let wide = |v: i128| U256::from_u128(env, u128::try_from(v).expect("non-negative"));
    let one_unit_times_index = U256::from_u128(env, 10).pow(54 - decimals);
    wide(delta).mul(&wide(supply_index))
        < one_unit_times_index.mul(&U256::from_u128(env, units.into()))
}

#[test]
fn share_gap_under_units_is_exact_at_the_boundary() {
    let env = Env::default();
    let ray = common::constants::RAY;
    let unit = 10i128.pow(20);
    assert!(share_gap_under_units(&env, unit - 1, ray, 7, 1));
    assert!(!share_gap_under_units(&env, unit, ray, 7, 1));
    assert!(share_gap_under_units(&env, 2 * unit - 1, ray, 7, 2));
    assert!(!share_gap_under_units(&env, 2 * unit, ray, 7, 2));
    // At 3 RAY one unit is 33_333_333_333_333_333_333.3 shares.
    assert!(share_gap_under_units(
        &env,
        33_333_333_333_333_333_333,
        3 * ray,
        7,
        1
    ));
    assert!(!share_gap_under_units(
        &env,
        33_333_333_333_333_333_334,
        3 * ray,
        7,
        1
    ));
    // At the supply-index floor (RAY / 1000) one unit is 1e23 shares.
    assert!(share_gap_under_units(
        &env,
        1000 * unit - 1,
        ray / 1000,
        7,
        1
    ));
    assert!(!share_gap_under_units(&env, 1000 * unit, ray / 1000, 7, 1));
}

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
    let mut t = LendingTest::new().standard_two_asset().build();

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
    // Both accounts share the spoke and hub asset, so only the protocol fee leaves spoke usage.
    // The pool reclassifies it as revenue, as bad-debt cleanup does for an absorbed position.
    assert_eq!(
        spoke_supply_usage(&t, "USDC"),
        usage_before - fee,
        "spoke usage must fall by the fee and by nothing else"
    );
}

// --- receiving-account rules --------------------------------------------

#[test]
fn credit_zero_creates_a_usable_account_owned_by_the_liquidator_in_the_right_spoke() {
    let mut t = LendingTest::new().standard_two_asset().build();

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
        .standard_two_asset()
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

    // Move the listing. An ordinary supply restamps an existing position; a credit does not.
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
        .standard_two_asset()
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
    // A dust-sized borrower whose collateral falls far enough that the residual debt clears
    // the socialization gate.
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
    // Seizure is pro-rata across every collateral, so a paused USDC listing must not block it.
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
    let mut t = LendingTest::new().standard_two_asset().build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    t.set_price("USDC", usd_cents(50));
    t.assert_liquidatable(ALICE);

    t.set_spoke_asset_flags("ETH", true, false, false);
    // The liquidator chooses the debt leg, so naming a paused debt asset reverts.
    assert_contract_error(
        t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 1.0),
        errors::SPOKE_ASSET_PAUSED,
    );
}

#[test]
fn no_seize_does_not_block_ordinary_withdrawal() {
    let mut t = LendingTest::new().standard_two_asset().build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.set_spoke_asset_flags("USDC", false, false, true);

    // `no_seize` governs the seizure leg only; users keep their exits.
    t.withdraw(ALICE, "USDC", 1_000.0);
    assert!(t.supply_balance(ALICE, "USDC") < 10_000.0);
}

// --- mode parity ---------------------------------------------------------

/// At the same ledger, transfer and credit debit identical accounts by share amounts less
/// than one asset unit apart.
///
/// The modes convert the seizure differently, so they cannot match bit for bit:
///
/// - Transfer: value -> asset units (floor) -> shares (ceil), in `resolve_withdrawal`.
/// - Credit: value -> shares (floor).
#[test]
fn transfer_and_credit_seize_the_same_value_at_the_same_ledger() {
    let mut t = LendingTest::new().standard_two_asset().build();

    // Two borrowers built identically, so the only difference at liquidation time
    // is the seize mode.
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    t.supply(CAROL, "USDC", 10_000.0);
    t.borrow(CAROL, "ETH", 3.0);
    t.set_price("USDC", usd_cents(50));
    t.assert_liquidatable(ALICE);
    t.assert_liquidatable(CAROL);

    let alice_id = t.resolve_account_id(ALICE);
    let carol_id = t.resolve_account_id(CAROL);
    let alice_before = scaled_supply(&t, alice_id, "USDC");
    let carol_before = scaled_supply(&t, carol_id, "USDC");
    assert_eq!(
        alice_before, carol_before,
        "fixture broken: the two borrowers must start identical"
    );

    t.liquidate_with_mode(LIQUIDATOR, ALICE, "ETH", 1.0, SeizeMode::Transfer);
    let receiver = t.liquidate_with_mode(LIQUIDATOR, CAROL, "ETH", 1.0, SeizeMode::Credit(0));

    let transfer_seized = alice_before - scaled_supply(&t, alice_id, "USDC");
    let credit_seized = carol_before - scaled_supply(&t, carol_id, "USDC");
    assert!(transfer_seized > 0 && credit_seized > 0, "both must seize");

    // The liquidated accounts' share debits differ by less than one asset unit.
    let delta = (transfer_seized - credit_seized).abs();
    let index = pool_state(&t, "USDC").supply_index;
    let decimals = t.resolve_market("USDC").decimals;
    let slack = seize_mode_share_slack(index, decimals);
    assert!(
        share_gap_under_units(&t.env, delta, index, decimals, 1),
        "modes disagree on shares seized by {delta} (transfer={transfer_seized} \
         credit={credit_seized}); the two conversion routes should not diverge \
         beyond {slack}"
    );

    let credited = scaled_supply(&t, receiver, "USDC");
    assert!(
        credited > 0 && credited <= credit_seized,
        "credited {credited} must be positive and no more than seized {credit_seized}"
    );
}

/// On a full close, transfer and credit debit exactly the same shares.
///
/// `resolve_withdrawal` returns `pos_scaled` when the request covers the position, and
/// credit takes `position.scaled_amount` when `capped_ray == actual_ray`. Neither path
/// derives the shares from an asset amount.
#[test]
fn transfer_and_credit_agree_exactly_on_a_full_close() {
    let mut t = LendingTest::new().standard_two_asset().build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    t.supply(CAROL, "USDC", 10_000.0);
    t.borrow(CAROL, "ETH", 3.0);
    // Deep enough underwater that the seizure exhausts the collateral position.
    t.set_price("USDC", usd_cents(20));
    t.assert_liquidatable(ALICE);
    t.assert_liquidatable(CAROL);

    let alice_id = t.resolve_account_id(ALICE);
    let carol_id = t.resolve_account_id(CAROL);
    let alice_before = scaled_supply(&t, alice_id, "USDC");
    let carol_before = scaled_supply(&t, carol_id, "USDC");

    t.liquidate_with_mode(LIQUIDATOR, ALICE, "ETH", 3.0, SeizeMode::Transfer);
    t.liquidate_with_mode(LIQUIDATOR, CAROL, "ETH", 3.0, SeizeMode::Credit(0));

    let transfer_seized = alice_before - scaled_supply(&t, alice_id, "USDC");
    let credit_seized = carol_before - scaled_supply(&t, carol_id, "USDC");
    assert_eq!(
        transfer_seized, credit_seized,
        "a full close takes the whole stored position in both modes, so the two \
         must agree to the share"
    );
}

/// Transfer and credit debit within one asset unit of shares on amounts and a price that
/// do not divide evenly, where floor and ceil can differ.
#[test]
fn transfer_and_credit_agree_on_values_that_do_not_divide_evenly() {
    let mut t = LendingTest::new().standard_two_asset().build();

    t.supply(ALICE, "USDC", 7_333.37);
    t.borrow(ALICE, "ETH", 2.19);
    t.supply(CAROL, "USDC", 7_333.37);
    t.borrow(CAROL, "ETH", 2.19);
    t.set_price("USDC", usd_cents(51));
    t.assert_liquidatable(ALICE);
    t.assert_liquidatable(CAROL);

    let alice_id = t.resolve_account_id(ALICE);
    let carol_id = t.resolve_account_id(CAROL);
    let alice_before = scaled_supply(&t, alice_id, "USDC");
    let carol_before = scaled_supply(&t, carol_id, "USDC");

    t.liquidate_with_mode(LIQUIDATOR, ALICE, "ETH", 0.73, SeizeMode::Transfer);
    t.liquidate_with_mode(LIQUIDATOR, CAROL, "ETH", 0.73, SeizeMode::Credit(0));

    let transfer_seized = alice_before - scaled_supply(&t, alice_id, "USDC");
    let credit_seized = carol_before - scaled_supply(&t, carol_id, "USDC");
    let delta = (transfer_seized - credit_seized).abs();
    let index = pool_state(&t, "USDC").supply_index;
    let decimals = t.resolve_market("USDC").decimals;
    let slack = seize_mode_share_slack(index, decimals);
    assert!(
        share_gap_under_units(&t.env, delta, index, decimals, 1),
        "modes diverged by {delta} shares on non-dividing values \
         (transfer={transfer_seized} credit={credit_seized}, slack={slack})"
    );

    // Exact in asset units only because `supply_index == RAY` here. At other indexes the
    // bound is one unit; see
    // `transfer_and_credit_agree_on_values_that_do_not_divide_evenly_after_accrual`.
    assert_eq!(
        t.supply_balance_raw(ALICE, "USDC"),
        t.supply_balance_raw(CAROL, "USDC"),
        "the two modes must leave the liquidated accounts with the same asset \
         value; a difference here is a real over- or under-seize, not rounding"
    );
}

/// Credit followed by a same-ledger withdraw-all pays the liquidator at most one unit less
/// than transfer.
///
/// Credit rounds twice (value to shares, then shares to asset units at withdraw); transfer
/// rounds once. The withdraw uses the `0` withdraw-all sentinel, so no caller amount adds a
/// rounding step.
#[test]
fn withdrawing_the_credit_in_the_same_ledger_matches_the_transfer_payout() {
    let mut t = LendingTest::new().standard_two_asset().build();

    t.supply(ALICE, "USDC", 7_333.37);
    t.borrow(ALICE, "ETH", 2.19);
    t.supply(CAROL, "USDC", 7_333.37);
    t.borrow(CAROL, "ETH", 2.19);
    t.set_price("USDC", usd_cents(51));
    t.assert_liquidatable(ALICE);
    t.assert_liquidatable(CAROL);

    // Register the liquidator before reading balances: the harness creates users
    // lazily, and the first read would otherwise precede their existence.
    let liquidator_addr = t.get_or_create_user(LIQUIDATOR);

    // Leg 1: Transfer pays the liquidator in underlying immediately.
    let before_transfer = t.token_balance_raw(LIQUIDATOR, "USDC");
    t.liquidate_with_mode(LIQUIDATOR, ALICE, "ETH", 0.73, SeizeMode::Transfer);
    let transfer_payout = t.token_balance_raw(LIQUIDATOR, "USDC") - before_transfer;

    // Leg 2: Credit, then drain the receiving account in the same ledger.
    let before_credit = t.token_balance_raw(LIQUIDATOR, "USDC");
    let receiver = t.liquidate_with_mode(LIQUIDATOR, CAROL, "ETH", 0.73, SeizeMode::Credit(0));

    let usdc = t.resolve_asset("USDC");
    let withdrawals = soroban_sdk::vec![&t.env, (hub_asset(usdc), 0i128)]; // 0 = withdraw all
    t.ctrl_client()
        .withdraw(&liquidator_addr, &receiver, &withdrawals, &None);
    let credit_payout = t.token_balance_raw(LIQUIDATOR, "USDC") - before_credit;

    assert!(
        transfer_payout > 0 && credit_payout > 0,
        "both flows must pay the liquidator something"
    );
    // The ordering holds only above the dust threshold; on a dust leg it inverts
    // (`a_dust_leg_inverts_the_payout_ordering_between_the_modes`). The one-unit magnitude
    // holds at every leg size
    // (`the_two_modes_never_differ_by_more_than_one_stroop_at_any_leg_size`).
    assert!(
        credit_payout <= transfer_payout,
        "credit-then-withdraw paid MORE than transfer ({credit_payout} vs \
         {transfer_payout}); above the dust threshold the ordering must hold"
    );
    assert!(
        transfer_payout - credit_payout <= 1,
        "credit-then-withdraw lost {} units versus transfer; the two flows differ \
         by at most one floor step",
        transfer_payout - credit_payout
    );
}

// --- fee base ------------------------------------------------------------

/// The protocol fee is charged on the bonus, not on the gross seizure.
///
/// Repay 100 and seize 105: the fee is a share of the 5, and the liquidator keeps
/// `105 - fee`, not `105 * (1 - fee_rate)`.
#[test]
fn the_protocol_fee_is_charged_on_the_bonus_not_the_gross_seizure() {
    let mut t = LendingTest::new().standard_two_asset().build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    t.set_price("USDC", usd_cents(50));
    t.assert_liquidatable(ALICE);

    let account_id = t.resolve_account_id(ALICE);
    let payments =
        soroban_sdk::Vec::from_array(&t.env, [(hub_asset(t.resolve_asset("ETH")), 1_0000000)]);
    let estimate =
        t.ctrl_client()
            .get_liquidation_estimate(&account_id, &payments, &SeizeMode::Transfer);

    let seized = estimate.seized_collaterals.get_unchecked(0).amount;
    let fee = estimate.protocol_fees.get_unchecked(0).amount;
    let bonus_bps = estimate.bonus_rate_bps;
    assert!(
        seized > 0 && fee > 0 && bonus_bps > 0,
        "estimate must be live"
    );

    // DEFAULT_ASSET_CONFIG.liquidation_fees
    const FEE_BPS: i128 = 1_200;
    const BPS: i128 = 10_000;

    // If the base were the gross seizure, this would hold with equality.
    let fee_if_charged_on_gross = seized * FEE_BPS / BPS;
    assert!(
        fee < fee_if_charged_on_gross,
        "fee {fee} matches a charge on the gross seizure ({fee_if_charged_on_gross}); \
         it must be charged on the bonus only"
    );

    // And it must match a charge on the bonus. seized = principal * (1 + b), so
    // the bonus portion is seized * b / (1 + b).
    let bonus_portion = seized * bonus_bps / (BPS + bonus_bps);
    let fee_if_charged_on_bonus = bonus_portion * FEE_BPS / BPS;
    let drift = (fee - fee_if_charged_on_bonus).abs();
    assert!(
        drift <= 1,
        "fee {fee} does not match a charge on the bonus ({fee_if_charged_on_bonus}, \
         bonus_bps={bonus_bps}, seized={seized}); drift {drift}"
    );

    let liquidator_take = seized - fee;
    assert!(
        liquidator_take > seized * (BPS - FEE_BPS) / BPS,
        "liquidator take {liquidator_take} looks like the gross scaled by the fee \
         rate rather than the gross minus a bonus-based fee"
    );
}

// --- dust legs: where the two modes stop agreeing ------------------------

/// Returns `(transfer_payout, credit_then_withdraw_payout)` in WBTC stroops for two identical
/// liquidatable accounts that each supply `wbtc_amt` WBTC beside USDC, for the same repayment.
///
/// Seizure is pro-rata across every collateral, so a small WBTC position yields a dust leg.
fn wbtc_leg_payouts(wbtc_amt: f64) -> (i128, i128) {
    let mut t = LendingTest::new().three_asset_usdc_eth_wbtc().build();
    for user in [ALICE, CAROL] {
        t.supply(user, "USDC", 10_000.0);
        t.supply(user, "WBTC", wbtc_amt);
        t.borrow(user, "ETH", 3.0);
    }
    t.set_price("USDC", usd_cents(50));
    t.assert_liquidatable(ALICE);
    t.assert_liquidatable(CAROL);

    let liquidator_addr = t.get_or_create_user(LIQUIDATOR);

    let before_transfer = t.token_balance_raw(LIQUIDATOR, "WBTC");
    t.liquidate_with_mode(LIQUIDATOR, ALICE, "ETH", 1.0, SeizeMode::Transfer);
    let transfer_payout = t.token_balance_raw(LIQUIDATOR, "WBTC") - before_transfer;

    let before_credit = t.token_balance_raw(LIQUIDATOR, "WBTC");
    let receiver = t.liquidate_with_mode(LIQUIDATOR, CAROL, "ETH", 1.0, SeizeMode::Credit(0));
    let withdrawals = soroban_sdk::vec![&t.env, (hub_asset(t.resolve_asset("WBTC")), 0i128)];
    t.ctrl_client()
        .withdraw(&liquidator_addr, &receiver, &withdrawals, &None);
    let credit_payout = t.token_balance_raw(LIQUIDATOR, "WBTC") - before_credit;

    (transfer_payout, credit_payout)
}

/// On a dust leg, credit-then-withdraw pays exactly one stroop more than transfer.
///
/// When the transfer fee is positive but floors to zero asset units,
/// `calculate_seized_collateral` charges one unit. Credit ceils the fee in RAY share space
/// (`split_seized_shares`), so it charges the sub-stroop fee. The difference is at most one
/// stroop of fee per dust leg; `PositionLimits.max_supply_positions` caps the leg count.
#[test]
fn a_dust_leg_inverts_the_payout_ordering_between_the_modes() {
    for wbtc_amt in [0.000001_f64, 0.000005, 0.00001, 0.00002] {
        let (transfer_payout, credit_payout) = wbtc_leg_payouts(wbtc_amt);
        assert!(
            transfer_payout > 0 && credit_payout > 0,
            "wbtc={wbtc_amt}: both flows must pay something"
        );
        assert_eq!(
            credit_payout - transfer_payout,
            1,
            "wbtc={wbtc_amt}: on a dust leg credit must pay exactly one stroop MORE \
             than transfer (transfer={transfer_payout} credit={credit_payout}); a \
             change here means the fee bump or the credit fee rule moved"
        );
    }
}

/// Above the dust threshold, transfer pays at least as much as credit-then-withdraw and at
/// most one stroop more. With the dust-leg test above, this brackets the crossover.
#[test]
fn above_the_dust_threshold_transfer_pays_at_least_as_much_as_credit() {
    for wbtc_amt in [0.0001_f64, 0.001, 0.01] {
        let (transfer_payout, credit_payout) = wbtc_leg_payouts(wbtc_amt);
        assert!(
            credit_payout <= transfer_payout,
            "wbtc={wbtc_amt}: above the dust threshold credit must not pay more \
             (transfer={transfer_payout} credit={credit_payout})"
        );
        assert!(
            transfer_payout - credit_payout <= 1,
            "wbtc={wbtc_amt}: the two flows differ by at most one floor step \
             (transfer={transfer_payout} credit={credit_payout})"
        );
    }
}

/// At every WBTC leg size, the two payouts differ by at most one stroop, in either direction.
///
/// Full delivery only: all three markets use standard SACs. On an under-delivering asset the
/// bound is two units; see `under_delivery_doubles_the_gap_but_keeps_it_bounded`.
///
/// The sweep stops at 0.01 WBTC ($600). A WBTC leg above $2,500 keeps the account healthy
/// after the USDC price drop, so no liquidation runs.
#[test]
fn the_two_modes_never_differ_by_more_than_one_stroop_at_any_leg_size() {
    for wbtc_amt in [
        0.000001_f64,
        0.000005,
        0.00001,
        0.00002,
        0.00005,
        0.0001,
        0.0005,
        0.001,
        0.01,
    ] {
        let (transfer_payout, credit_payout) = wbtc_leg_payouts(wbtc_amt);
        let delta = (credit_payout - transfer_payout).abs();
        assert!(
            delta <= 1,
            "wbtc={wbtc_amt}: modes diverged by {delta} stroops \
             (transfer={transfer_payout} credit={credit_payout})"
        );
    }
}

// --- accrued index and fee parity ----------------------------------------

/// Transfer and credit stay within one asset unit on values that do not divide evenly after
/// one year of accrual moves the USDC supply index above RAY.
#[test]
fn transfer_and_credit_agree_on_values_that_do_not_divide_evenly_after_accrual() {
    let mut t = LendingTest::new().standard_two_asset().build();

    for user in [ALICE, CAROL] {
        t.supply(user, "USDC", 7_333.37);
        t.borrow(user, "ETH", 2.19);
    }
    // A USDC borrow is necessary: with no USDC borrowed, the supply index stays at RAY.
    t.supply(BOB, "ETH", 50.0);
    t.borrow(BOB, "USDC", 9_000.0);
    t.advance_and_sync(365 * 24 * 60 * 60);

    let index_before = pool_state(&t, "USDC").supply_index;
    assert!(
        index_before > common::constants::RAY,
        "fixture broken: the USDC supply index must have accrued past RAY, got \
         {index_before}"
    );

    t.set_price("USDC", usd_cents(51));
    t.assert_liquidatable(ALICE);
    t.assert_liquidatable(CAROL);

    let alice_id = t.resolve_account_id(ALICE);
    let carol_id = t.resolve_account_id(CAROL);
    let alice_before = scaled_supply(&t, alice_id, "USDC");
    let carol_before = scaled_supply(&t, carol_id, "USDC");

    t.liquidate_with_mode(LIQUIDATOR, ALICE, "ETH", 0.73, SeizeMode::Transfer);
    t.liquidate_with_mode(LIQUIDATOR, CAROL, "ETH", 0.73, SeizeMode::Credit(0));

    let transfer_seized = alice_before - scaled_supply(&t, alice_id, "USDC");
    let credit_seized = carol_before - scaled_supply(&t, carol_id, "USDC");
    let delta = (transfer_seized - credit_seized).abs();
    let index = pool_state(&t, "USDC").supply_index;
    let decimals = t.resolve_market("USDC").decimals;
    let slack = seize_mode_share_slack(index, decimals);
    assert!(
        share_gap_under_units(&t.env, delta, index, decimals, 1),
        "with an accrued index the modes diverged by {delta} shares \
         (transfer={transfer_seized} credit={credit_seized}); the bound is one \
         asset unit in share space at THIS index, {slack}"
    );

    // Off RAY the two accounts can end one asset unit apart, in either direction.
    let alice_left = t.supply_balance_raw(ALICE, "USDC");
    let carol_left = t.supply_balance_raw(CAROL, "USDC");
    let asset_delta = (alice_left - carol_left).abs();
    assert!(
        asset_delta <= 1,
        "with an accrued index the modes left the victims {asset_delta} asset units \
         apart (transfer={alice_left} credit={carol_left}); the gap must stay under \
         one unit whatever the index"
    );
}

/// Transfer and credit charge the same protocol fee within one asset unit.
///
/// Transfer rates the bonus half-up in RAY asset value, then floors to asset units with a
/// one-unit minimum (`calculate_seized_collateral`). Credit ceils the fee on a floored share
/// bonus (`split_seized_shares`). The test converts the credit fee to asset units to compare.
#[test]
fn the_two_modes_charge_the_same_protocol_fee_within_one_unit() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(ALICE, "USDC", 7_333.37);
    t.borrow(ALICE, "ETH", 2.19);
    t.set_price("USDC", usd_cents(51));
    t.assert_liquidatable(ALICE);

    let alice_id = t.resolve_account_id(ALICE);
    let payments = test_harness::asset_payment_vec(
        &t.env,
        t.resolve_asset("ETH"),
        test_harness::amount_raw(0.73, t.resolve_market("ETH").decimals),
    );

    let transfer =
        t.ctrl_client()
            .get_liquidation_estimate(&alice_id, &payments, &SeizeMode::Transfer);
    let credit =
        t.ctrl_client()
            .get_liquidation_estimate(&alice_id, &payments, &SeizeMode::Credit(0));

    let transfer_fee = transfer.protocol_fees.get_unchecked(0).amount;
    let credit_fee_shares = credit.protocol_fees.get_unchecked(0).amount;
    assert!(
        transfer_fee > 0 && credit_fee_shares > 0,
        "both estimates must report a live fee"
    );

    // No accrual or bad debt has touched USDC, so the index is RAY and shares convert
    // to asset units by the decimal factor alone.
    let supply_index = pool_state(&t, "USDC").supply_index;
    assert_eq!(supply_index, common::constants::RAY);
    let decimals = t.resolve_market("USDC").decimals;
    let credit_fee_assets = credit_fee_shares / 10i128.pow(27 - decimals);

    let delta = (transfer_fee - credit_fee_assets).abs();
    assert!(
        delta <= 1,
        "the two fee rules disagree by {delta} asset units (transfer={transfer_fee} \
         credit={credit_fee_assets} from {credit_fee_shares} shares at index \
         {supply_index}); they rate the same bonus and must land within one unit"
    );
}

/// Across a sweep of accrual durations, USDC borrows and repay fractions, transfer and credit
/// stay within one asset unit at each resulting supply index.
///
/// Each point checks two bounds:
///
/// 1. Share space: the gap is under one asset unit at that index (`seize_mode_share_slack`).
/// 2. Asset space: the two accounts end within one unit of each other.
///
/// The supremum is one asset unit: the transfer asset floor can shed almost a whole unit
/// before the share ceil. Full delivery only (a standard SAC repayment); the under-delivery
/// bound is in `under_delivery_doubles_the_gap_but_keeps_it_bounded`.
#[test]
fn the_share_gap_stays_under_one_asset_unit_across_a_supply_index_sweep() {
    let mut ran = 0usize;
    let mut worst = 0.0f64;

    for secs in [1u64, 3_607, 86_399, 2_592_001, 31_536_007] {
        for bob_borrow in [700.13f64, 4_999.37, 11_777.91] {
            for repay_bps in [2_903u32, 6_113, 9_337] {
                let mut t = LendingTest::new().standard_two_asset().build();
                let (supply, borrow) = (9_876.54f64, 2.19f64);
                for user in [ALICE, CAROL] {
                    t.supply(user, "USDC", supply);
                    t.borrow(user, "ETH", borrow);
                }
                t.supply(BOB, "ETH", 300.0);
                if t.try_borrow(BOB, "USDC", bob_borrow).is_err() {
                    continue;
                }
                t.advance_and_sync(secs);
                t.set_price("USDC", usd_cents(51));
                if !t.can_be_liquidated(ALICE) || !t.can_be_liquidated(CAROL) {
                    continue;
                }

                let index = pool_state(&t, "USDC").supply_index;
                let repay = borrow * f64::from(repay_bps) / 10_000.0;
                let a = t.resolve_account_id(ALICE);
                let c = t.resolve_account_id(CAROL);
                let a_before = scaled_supply(&t, a, "USDC");
                let c_before = scaled_supply(&t, c, "USDC");

                if t.try_liquidate_with_mode(LIQUIDATOR, ALICE, "ETH", repay, SeizeMode::Transfer)
                    .is_err()
                {
                    continue;
                }
                if t.try_liquidate_with_mode(LIQUIDATOR, CAROL, "ETH", repay, SeizeMode::Credit(0))
                    .is_err()
                {
                    continue;
                }
                ran += 1;

                let transfer_seized = a_before - scaled_supply(&t, a, "USDC");
                let credit_seized = c_before - scaled_supply(&t, c, "USDC");
                let delta = (transfer_seized - credit_seized).abs();
                let decimals = t.resolve_market("USDC").decimals;
                let ratio = delta as f64 / seize_mode_share_slack(index, decimals);
                if ratio > worst {
                    worst = ratio;
                }
                assert!(
                    share_gap_under_units(&t.env, delta, index, decimals, 1),
                    "secs={secs} bob={bob_borrow} repay_bps={repay_bps} index={index}: \
                     share gap {delta} reached {ratio:.4} of one asset unit \
                     (transfer={transfer_seized} credit={credit_seized})"
                );

                let alice_left = t.supply_balance_raw(ALICE, "USDC");
                let carol_left = t.supply_balance_raw(CAROL, "USDC");
                assert!(
                    (alice_left - carol_left).abs() <= 1,
                    "secs={secs} bob={bob_borrow} repay_bps={repay_bps} index={index}: \
                     victims left {alice_left} vs {carol_left}, more than one unit apart"
                );
            }
        }
    }

    assert!(
        ran >= 20,
        "sweep degenerated: only {ran} points actually liquidated, so the bound \
         was barely exercised"
    );
    std::println!("index sweep: {ran} points, worst ratio {worst:.4} of one asset unit");
}

/// Transfer and credit stay within one asset unit after bad-debt socialization of a USDC
/// borrower drops the USDC supply index below RAY.
///
/// At an index of `x` RAY, one asset unit is `10^(27-decimals) / x` shares.
#[test]
fn the_share_gap_stays_under_one_asset_unit_with_the_supply_index_below_ray() {
    let mut ran = 0usize;
    let mut lowest_index = i128::MAX;

    for wiped_usdc in [3_000.0f64, 9_000.0, 15_000.0, 18_700.0] {
        for repay_bps in [2_903u32, 6_113, 9_337] {
            let mut t = LendingTest::new().standard_two_asset().build();
            for user in [ALICE, CAROL] {
                t.supply(user, "USDC", 9_876.54);
            }
            t.supply(BOB, "ETH", 300.0);
            t.supply(DAVE, "ETH", 20.0);
            t.borrow(DAVE, "USDC", wiped_usdc);

            // Dave's collateral collapses; his USDC debt is written off against
            // the USDC supply index.
            t.set_price("ETH", usd_cents(100));
            t.force_socialize_bad_debt_by_id(t.resolve_account_id(DAVE));
            t.set_price("ETH", usd(2_000));
            let index = pool_state(&t, "USDC").supply_index;
            assert!(
                index < common::constants::RAY,
                "wiped {wiped_usdc}: index {index}"
            );

            // Borrow to 95% of the LTV left after the write-down.
            let borrow = t.total_collateral_raw(ALICE) as f64 / 1e18 * 0.75 * 0.95 / 2_000.0;
            if [ALICE, CAROL]
                .iter()
                .any(|user| t.try_borrow(user, "ETH", borrow).is_err())
            {
                continue;
            }
            // Walk USDC down until both are liquidatable while still solvent.
            let mut cents = 100;
            while cents > 0 && !(t.can_be_liquidated(ALICE) && t.can_be_liquidated(CAROL)) {
                cents -= 1;
                t.set_price("USDC", usd_cents(cents));
            }
            if cents == 0 || t.total_debt_raw(ALICE) >= t.total_collateral_raw(ALICE) {
                continue;
            }

            let repay = borrow * f64::from(repay_bps) / 10_000.0;
            let a = t.resolve_account_id(ALICE);
            let c = t.resolve_account_id(CAROL);
            let a_before = scaled_supply(&t, a, "USDC");
            let c_before = scaled_supply(&t, c, "USDC");
            if t.try_liquidate_with_mode(LIQUIDATOR, ALICE, "ETH", repay, SeizeMode::Transfer)
                .is_err()
                || t.try_liquidate_with_mode(LIQUIDATOR, CAROL, "ETH", repay, SeizeMode::Credit(0))
                    .is_err()
            {
                continue;
            }
            ran += 1;
            lowest_index = lowest_index.min(index);

            let transfer_seized = a_before - scaled_supply(&t, a, "USDC");
            let credit_seized = c_before - scaled_supply(&t, c, "USDC");
            let delta = (transfer_seized - credit_seized).abs();
            let decimals = t.resolve_market("USDC").decimals;
            let slack = seize_mode_share_slack(index, decimals);
            assert!(
                share_gap_under_units(&t.env, delta, index, decimals, 1),
                "wiped={wiped_usdc} repay_bps={repay_bps} index={index}: share gap {delta} \
                 is not under one asset unit ({slack:.0} shares)"
            );
            let alice_left = t.supply_balance_raw(ALICE, "USDC");
            let carol_left = t.supply_balance_raw(CAROL, "USDC");
            assert!(
                (alice_left - carol_left).abs() <= 1,
                "wiped={wiped_usdc} repay_bps={repay_bps} index={index}: victims left \
                 {alice_left} vs {carol_left}"
            );
        }
    }

    assert!(ran >= 6, "only {ran} points liquidated");
    assert!(
        lowest_index < common::constants::RAY / 10,
        "the sweep never reached a deep write-down: lowest index {lowest_index}"
    );
    std::println!("below-RAY sweep: {ran} points, lowest index {lowest_index}");
}

// --- under-delivery: the one regime where one unit is not the bound ---------

/// Cross-mode gap bound, in asset units, when the debt token under-delivers. One extra floor
/// per representation doubles the full-delivery bound.
const UNDER_DELIVERY_SLACK_UNITS: u32 = 2;

/// A fee-on-transfer debt token doubles the gap between the two seize modes to under two
/// asset units in share space and at most two units in asset space.
///
/// When `received < planned`, `scale_seizures_to_received` floor-scales `amount` and
/// `scaled_amount` separately by `received / planned`. That adds one floor to each
/// representation.
#[test]
fn under_delivery_doubles_the_gap_but_keeps_it_bounded() {
    let mut ran = 0usize;
    let mut worst_ratio = 0.0f64;
    let mut worst_asset = 0i128;
    let mut exceeded_single_unit = false;

    for bps in [1i128, 7, 50, 137, 500] {
        for repay_bps in [2_903u32, 6_113, 9_337] {
            let mut t = LendingTest::new()
                .with_market(usdc_preset())
                .with_fee_on_transfer_market(eth_preset(), bps)
                .build();
            let (supply, borrow) = (9_876.54f64, 2.19f64);
            let mut fixture_ok = true;
            for user in [ALICE, CAROL] {
                t.supply(user, "USDC", supply);
                if t.try_borrow(user, "ETH", borrow).is_err() {
                    fixture_ok = false;
                }
            }
            if !fixture_ok {
                continue;
            }
            t.set_price("USDC", usd_cents(51));
            if !t.can_be_liquidated(ALICE) || !t.can_be_liquidated(CAROL) {
                continue;
            }

            let index = pool_state(&t, "USDC").supply_index;
            let repay = borrow * f64::from(repay_bps) / 10_000.0;
            let a = t.resolve_account_id(ALICE);
            let c = t.resolve_account_id(CAROL);
            let a_before = scaled_supply(&t, a, "USDC");
            let c_before = scaled_supply(&t, c, "USDC");

            if t.try_liquidate_with_mode(LIQUIDATOR, ALICE, "ETH", repay, SeizeMode::Transfer)
                .is_err()
            {
                continue;
            }
            if t.try_liquidate_with_mode(LIQUIDATOR, CAROL, "ETH", repay, SeizeMode::Credit(0))
                .is_err()
            {
                continue;
            }
            ran += 1;

            let transfer_seized = a_before - scaled_supply(&t, a, "USDC");
            let credit_seized = c_before - scaled_supply(&t, c, "USDC");
            let delta = (transfer_seized - credit_seized).abs();
            let decimals = t.resolve_market("USDC").decimals;
            let ratio = delta as f64 / seize_mode_share_slack(index, decimals);
            if ratio > worst_ratio {
                worst_ratio = ratio;
            }
            if !share_gap_under_units(&t.env, delta, index, decimals, 1) {
                exceeded_single_unit = true;
            }
            assert!(
                share_gap_under_units(&t.env, delta, index, decimals, UNDER_DELIVERY_SLACK_UNITS),
                "bps={bps} repay_bps={repay_bps}: under-delivery gap {delta} reached \
                 {ratio:.4} asset units, past the doubled bound \
                 (transfer={transfer_seized} credit={credit_seized})"
            );

            let alice_left = t.supply_balance_raw(ALICE, "USDC");
            let carol_left = t.supply_balance_raw(CAROL, "USDC");
            let asset_delta = (alice_left - carol_left).abs();
            if asset_delta > worst_asset {
                worst_asset = asset_delta;
            }
            assert!(
                asset_delta <= 2,
                "bps={bps} repay_bps={repay_bps}: victims left {alice_left} vs \
                 {carol_left}, more than two units apart"
            );
        }
    }

    assert!(
        ran >= 10,
        "sweep degenerated: only {ran} points liquidated under under-delivery"
    );
    // At least one point must exceed the single-unit bound; otherwise the token does not
    // under-deliver and the test exercises nothing.
    assert!(
        exceeded_single_unit,
        "no point exceeded one asset unit across {ran} runs, so under-delivery \
         was not actually exercised; worst ratio was {worst_ratio:.4}"
    );
    std::println!(
        "under-delivery: {ran} points, worst ratio {worst_ratio:.4}, worst asset gap {worst_asset}"
    );
}
