use crate::shared::get_indexes;
use controller::constants::WAD;
use test_harness::{
    assert_contract_error, days, errors, hub_asset, usd, usd_cents, LendingTest, ALICE, BOB, CAROL,
    DAVE, LIQUIDATOR,
};

fn setup() -> LendingTest {
    LendingTest::new().standard_two_asset_dust_disabled()
}

#[test]
fn test_bad_debt_decreases_supply_index() {
    let mut t = setup();

    t.supply(BOB, "ETH", 100.0);

    t.supply(ALICE, "USDC", 10.0);
    t.borrow(ALICE, "ETH", 0.003);

    let (si_before, _) = get_indexes(&t, "ETH");

    t.set_price("USDC", usd_cents(10));
    t.assert_liquidatable(ALICE);

    t.liquidate(LIQUIDATOR, ALICE, "ETH", 0.001);

    let (si_after, _) = get_indexes(&t, "ETH");

    assert!(
        si_after < si_before,
        "supply index should DECREASE after bad debt: before={}, after={}",
        si_before,
        si_after
    );

    let decrease_ratio = si_after as f64 / si_before as f64;
    assert!(
        decrease_ratio > 0.99 && decrease_ratio < 1.0,
        "decrease should be small relative to total supply: ratio={:.6}",
        decrease_ratio
    );
}

#[test]
fn test_bad_debt_loss_distributed_proportionally() {
    let mut t = setup();

    t.supply(BOB, "ETH", 75.0);
    t.supply(CAROL, "ETH", 25.0);

    t.supply(ALICE, "USDC", 10.0);
    t.borrow(ALICE, "ETH", 0.003);

    let bob_before = t.supply_balance(BOB, "ETH");
    let carol_before = t.supply_balance(CAROL, "ETH");

    t.set_price("USDC", usd_cents(10));
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 0.001);

    let bob_after = t.supply_balance(BOB, "ETH");
    let carol_after = t.supply_balance(CAROL, "ETH");

    let bob_loss = bob_before - bob_after;
    let carol_loss = carol_before - carol_after;

    assert!(
        bob_loss > 0.0,
        "Bob should lose from bad debt: {:.6}",
        bob_loss
    );
    assert!(
        carol_loss > 0.0,
        "Carol should lose from bad debt: {:.6}",
        carol_loss
    );

    // The ratio check used to hide behind `if carol_loss > 0.0001`, so a
    // rounding change that shrank Carol's loss silently deleted it.
    assert!(
        carol_loss > 0.0001,
        "fixture must produce a measurable loss for the ratio to mean anything: {carol_loss:.6}"
    );
    let ratio = bob_loss / carol_loss;
    assert!(
        (ratio - 3.0).abs() < 0.3,
        "loss should be proportional (3:1): ratio={:.4}, bob_loss={:.6}, carol_loss={:.6}",
        ratio,
        bob_loss,
        carol_loss
    );
}

#[test]
fn test_bad_debt_index_floored_at_safety_floor() {
    let mut t = setup();

    t.supply(BOB, "ETH", 0.01);

    t.supply(ALICE, "USDC", 100.0);
    t.borrow(ALICE, "ETH", 0.005);

    t.set_price("USDC", usd_cents(1));

    t.liquidate(LIQUIDATOR, ALICE, "ETH", 0.001);

    let (si_after, _) = get_indexes(&t, "ETH");

    assert!(
        si_after >= controller::constants::SUPPLY_INDEX_FLOOR_RAW,
        "supply index should be floored at {}, got {}",
        controller::constants::SUPPLY_INDEX_FLOOR_RAW,
        si_after
    );
}

#[test]
fn test_supply_index_recovers_after_bad_debt() {
    let mut t = setup();

    t.supply(BOB, "ETH", 100.0);
    t.supply(ALICE, "USDC", 10.0);
    t.borrow(ALICE, "ETH", 0.003);

    t.set_price("USDC", usd_cents(10));
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 0.001);

    let (si_after_bad_debt, _) = get_indexes(&t, "ETH");

    t.set_price("USDC", usd(1));

    t.supply(DAVE, "USDC", 500_000.0);
    t.borrow(DAVE, "ETH", 30.0);

    t.advance_and_sync(days(365));

    let (si_recovered, _) = get_indexes(&t, "ETH");

    assert!(
        si_recovered > si_after_bad_debt,
        "supply index should recover with new interest: post_bad_debt={}, recovered={}",
        si_after_bad_debt,
        si_recovered
    );
}

#[test]
fn test_force_socialize_bad_debt_above_dust_threshold() {
    let mut t = setup();

    t.supply(BOB, "ETH", 100.0);
    t.supply(ALICE, "USDC", 100.0);
    t.borrow(ALICE, "ETH", 0.02);

    t.set_price("USDC", usd_cents(30));
    let account_id = t.resolve_account_id(ALICE);

    let collateral = t.total_collateral_raw(ALICE);
    let debt = t.total_debt_raw(ALICE);
    assert!(
        collateral > 5 * WAD,
        "fixture must sit strictly above the $5 dust gate: collateral_wad={collateral}"
    );
    assert!(
        debt > collateral,
        "fixture must be insolvent: debt_wad={debt} collateral_wad={collateral}"
    );

    let refused = t.try_clean_bad_debt_by_id(account_id);
    assert_contract_error(refused, errors::CANNOT_CLEAN_BAD_DEBT);

    let (si_before, _) = get_indexes(&t, "ETH");

    t.force_socialize_bad_debt_by_id(account_id);

    let (si_after, _) = get_indexes(&t, "ETH");
    assert!(
        si_after < si_before,
        "force-socialize must drop the ETH supply index: before={si_before}, after={si_after}"
    );
    t.assert_no_positions(ALICE);
}

#[test]
fn test_force_socialize_rejects_healthy_account() {
    let mut t = setup();

    t.supply(BOB, "ETH", 100.0);
    t.supply(ALICE, "USDC", 100.0);
    t.borrow(ALICE, "ETH", 0.01);

    let account_id = t.resolve_account_id(ALICE);
    let refused = t.try_force_socialize_bad_debt_by_id(account_id);
    assert_contract_error(refused, errors::CANNOT_CLEAN_BAD_DEBT);
}

#[test]
fn test_keeper_clean_bad_debt_decreases_supply_index() {
    let mut t = setup();

    t.supply(BOB, "ETH", 100.0);

    t.supply(ALICE, "USDC", 8.0);
    t.borrow(ALICE, "ETH", 0.002);

    let (si_before, _) = get_indexes(&t, "ETH");

    t.set_price("USDC", usd_cents(5));

    let account_id = t.resolve_account_id(ALICE);
    t.clean_bad_debt_by_id(account_id);

    let (si_after, _) = get_indexes(&t, "ETH");

    assert!(
        si_after < si_before,
        "keeper clean_bad_debt should decrease supply index: before={}, after={}",
        si_before,
        si_after
    );

    t.assert_no_positions(ALICE);
}

#[test]
fn test_bad_debt_does_not_affect_borrow_index() {
    let mut t = setup();

    t.supply(BOB, "ETH", 100.0);
    t.supply(ALICE, "USDC", 10.0);
    t.borrow(ALICE, "ETH", 0.003);

    t.advance_and_sync(days(1));
    let (_, bi_before) = get_indexes(&t, "ETH");

    t.set_price("USDC", usd_cents(10));
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 0.001);

    let (_, bi_after) = get_indexes(&t, "ETH");

    // No time passes between the two reads, so "does not affect" means equality.
    // `>=` was already guaranteed by INV-IDX-01 and would still pass if the
    // write-down were applied to the borrow index instead of the supply index.
    assert_eq!(
        bi_after, bi_before,
        "bad debt must leave the borrow index untouched: before={bi_before}, after={bi_after}"
    );
}

#[test]
fn test_bad_debt_reduction_matches_formula() {
    let mut t = setup();

    t.supply(BOB, "ETH", 1000.0);

    t.supply(ALICE, "USDC", 10.0);
    t.borrow(ALICE, "ETH", 0.003);

    let bob_balance_before = t.supply_balance(BOB, "ETH");
    let (si_before, _) = get_indexes(&t, "ETH");

    t.set_price("USDC", usd_cents(10));
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 0.001);

    let (si_after, _) = get_indexes(&t, "ETH");
    let bob_balance_after = t.supply_balance(BOB, "ETH");
    let bob_loss = bob_balance_before - bob_balance_after;

    // The formula this test is named for: every supplier's loss is his balance
    // times the supply-index write-down. `0.0 < loss < 0.01` was a band, and its
    // lower bound was the wrong direction to catch under-socialization.
    let expected_loss = bob_balance_before * (1.0 - si_after as f64 / si_before as f64);
    assert!(
        bob_loss > 0.0,
        "Bob must absorb part of the loss: {bob_loss:.9}"
    );
    assert!(
        (bob_loss - expected_loss).abs() * 10_000.0 <= expected_loss,
        "Bob's loss must equal balance x index write-down to within 1bp: \
         got={bob_loss:.9}, expected={expected_loss:.9}"
    );
}

/// Full committed market state for `asset`, read straight off its pool.
fn market_state(t: &LendingTest, asset: &str) -> controller::types::PoolStateRaw {
    let asset_addr = t.resolve_asset(asset);
    t.pool_client(asset)
        .get_sync_data(&hub_asset(asset_addr))
        .state
}

/// Asserts every field of a market's committed state is bit-identical.
fn assert_market_state_unchanged(
    asset: &str,
    before: &controller::types::PoolStateRaw,
    after: &controller::types::PoolStateRaw,
) {
    assert_eq!(
        before.supply_index, after.supply_index,
        "{asset} supply index moved: before={} after={}",
        before.supply_index, after.supply_index
    );
    assert_eq!(
        before.borrow_index, after.borrow_index,
        "{asset} borrow index moved"
    );
    assert_eq!(before.supplied, after.supplied, "{asset} supplied moved");
    assert_eq!(before.borrowed, after.borrowed, "{asset} borrowed moved");
    assert_eq!(before.revenue, after.revenue, "{asset} revenue moved");
    assert_eq!(before.cash, after.cash, "{asset} cash moved");
    assert_eq!(
        before.last_timestamp, after.last_timestamp,
        "{asset} last_timestamp moved"
    );
}

/// Bad-debt socialization writes down exactly one market's supply index.
///
/// ETH carries the socialized loss; WBTC is a live market the insolvent
/// account never touched, so every field of its committed state must survive
/// the cleanup bit-identical. Both markets are driven to distinct, non-unit
/// indexes first, so a cross-market write would be observable rather than
/// masked by two equal values.
#[test]
fn test_socialization_leaves_an_untouched_market_bit_identical() {
    let mut t = LendingTest::new()
        .three_asset_usdc_eth_wbtc()
        .with_dust_disabled_all_markets()
        .build();

    t.supply(BOB, "ETH", 100.0);
    t.supply(CAROL, "WBTC", 2.0);
    t.supply(DAVE, "USDC", 200_000.0);
    t.borrow(DAVE, "WBTC", 1.0);

    // Move WBTC off its genesis index so an accidental write is detectable.
    t.advance_and_sync(days(90));

    t.supply(ALICE, "USDC", 8.0);
    t.borrow(ALICE, "ETH", 0.002);

    let wbtc_before = market_state(&t, "WBTC");
    let (eth_si_before, _) = get_indexes(&t, "ETH");

    assert!(
        wbtc_before.supply_index > controller::constants::RAY,
        "fixture must accrue WBTC interest first: supply_index={}",
        wbtc_before.supply_index
    );
    assert_ne!(
        wbtc_before.supply_index, eth_si_before,
        "fixture must keep the two supply indexes distinct, else the assertion is vacuous"
    );

    t.set_price("USDC", usd_cents(5));
    let account_id = t.resolve_account_id(ALICE);
    t.clean_bad_debt_by_id(account_id);

    let (eth_si_after, _) = get_indexes(&t, "ETH");
    let wbtc_after = market_state(&t, "WBTC");

    assert!(
        eth_si_after < eth_si_before,
        "the socialized market must be written down, else this test proves nothing: before={eth_si_before} after={eth_si_after}"
    );
    assert_market_state_unchanged("WBTC", &wbtc_before, &wbtc_after);
}

/// The same scoping under `force_socialize_bad_debt`, the owner-only path that
/// bypasses the dust gate and takes a different controller entry point.
#[test]
fn test_force_socialize_leaves_an_untouched_market_bit_identical() {
    let mut t = LendingTest::new()
        .three_asset_usdc_eth_wbtc()
        .with_dust_disabled_all_markets()
        .build();

    t.supply(BOB, "ETH", 100.0);
    t.supply(CAROL, "WBTC", 2.0);
    t.supply(DAVE, "USDC", 200_000.0);
    t.borrow(DAVE, "WBTC", 1.0);

    t.advance_and_sync(days(90));

    t.supply(ALICE, "USDC", 100.0);
    t.borrow(ALICE, "ETH", 0.02);

    let wbtc_before = market_state(&t, "WBTC");
    let (eth_si_before, _) = get_indexes(&t, "ETH");

    t.set_price("USDC", usd_cents(30));
    let account_id = t.resolve_account_id(ALICE);
    t.force_socialize_bad_debt_by_id(account_id);

    let (eth_si_after, _) = get_indexes(&t, "ETH");
    let wbtc_after = market_state(&t, "WBTC");

    assert!(
        eth_si_after < eth_si_before,
        "force-socialize must write down ETH: before={eth_si_before} after={eth_si_after}"
    );
    assert_market_state_unchanged("WBTC", &wbtc_before, &wbtc_after);
}

/// A4-econ: bad-debt socialization is recognised only when a keeper calls the
/// write-down, and `backing_shortfall` still counts the unrecoverable debt at
/// face value until then. A supplier who watches the chain can therefore exit
/// at the pre-write-down index and leave the whole loss on the suppliers who
/// stayed. Runs the same crash twice, once with Bob passive and once with Bob
/// exiting first, and compares Carol's realised loss.
#[test]
fn supplier_can_exit_ahead_of_bad_debt_writedown() {
    // Scenario A: nobody dodges. Bob 75%, Carol 25% of the ETH supply.
    let mut a = setup();
    a.supply(BOB, "ETH", 75.0);
    a.supply(CAROL, "ETH", 25.0);
    a.supply(ALICE, "USDC", 10.0);
    a.borrow(ALICE, "ETH", 0.003);

    let carol_before_a = a.supply_balance(CAROL, "ETH");
    a.set_price("USDC", usd_cents(10));
    a.liquidate(LIQUIDATOR, ALICE, "ETH", 0.001);
    let carol_loss_a = carol_before_a - a.supply_balance(CAROL, "ETH");

    // Scenario B: identical state, but Bob withdraws before the write-down.
    let mut b = setup();
    b.supply(BOB, "ETH", 75.0);
    b.supply(CAROL, "ETH", 25.0);
    b.supply(ALICE, "USDC", 10.0);
    b.borrow(ALICE, "ETH", 0.003);

    let carol_before_b = b.supply_balance(CAROL, "ETH");
    let bob_before_b = b.supply_balance(BOB, "ETH");
    let bob_wallet_before = b.token_balance(BOB, "ETH");

    // The crash is public state. Alice is insolvent from here on, but no
    // write-down has been applied yet.
    b.set_price("USDC", usd_cents(10));
    b.assert_liquidatable(ALICE);

    // Bob exits at the un-written-down index. No gate stops him: the
    // liquidation buffer only guards borrow draws, and `backing_shortfall`
    // still values Alice's uncollateralised debt at face.
    b.withdraw_all(BOB, "ETH");
    let bob_recovered = b.token_balance(BOB, "ETH") - bob_wallet_before;

    b.liquidate(LIQUIDATOR, ALICE, "ETH", 0.001);
    let carol_loss_b = carol_before_b - b.supply_balance(CAROL, "ETH");

    assert!(
        bob_recovered >= bob_before_b,
        "Bob exits whole: supplied={:.9} recovered={:.9}",
        bob_before_b,
        bob_recovered
    );
    assert!(
        carol_loss_b > carol_loss_a,
        "dodging must push loss onto Carol: A={:.9} B={:.9}",
        carol_loss_a,
        carol_loss_b
    );

    // Carol holds 25% of supply, so passing the whole loss to her is ~4x.
    let amplification = carol_loss_b / carol_loss_a;
    assert!(
        amplification > 3.0,
        "expected ~4x concentration onto the remaining supplier, got {:.3}x \
         (A={:.9} B={:.9})",
        amplification,
        carol_loss_a,
        carol_loss_b
    );

    std::println!(
        "A4-econ dodge: bob_supplied={:.9} bob_recovered={:.9} \
         carol_loss_passive={:.9} carol_loss_after_dodge={:.9} amplification={:.3}x",
        bob_before_b,
        bob_recovered,
        carol_loss_a,
        carol_loss_b,
        amplification
    );
}

/// A4-econ: `force_socialize_bad_debt` applies the **full** outstanding debt to
/// the debt market's supply index (`interest::apply_bad_debt_to_supply_index`)
/// while the account's collateral is reclassified as protocol revenue in its own
/// market (`Cache::absorb_supply_as_revenue`). The two sides live in different
/// markets, so the recovery never offsets the loss: debt-asset suppliers absorb
/// 100% of the write-down and the collateral asset's treasury keeps the whole
/// recovery. Unlike `clean_bad_debt`, the `Insolvent` gate carries no dust cap,
/// so the un-netted collateral is unbounded.
#[test]
fn force_socialize_does_not_net_collateral_against_debt() {
    let mut t = setup();

    t.supply(BOB, "ETH", 100.0);
    t.supply(ALICE, "USDC", 10.0);
    t.borrow(ALICE, "ETH", 0.003);

    let alice_id = t.resolve_account_id(ALICE);
    let bob_before = t.supply_balance(BOB, "ETH");
    let alice_collateral_before = t.supply_balance(ALICE, "USDC");
    let usdc_rev_before = t.snapshot_revenue("USDC");

    // Crash the collateral so debt > collateral, satisfying the Insolvent gate.
    // Alice keeps real collateral: no liquidator has taken it.
    t.set_price("USDC", usd_cents(10));
    t.assert_liquidatable(ALICE);

    t.force_socialize_bad_debt_by_id(alice_id);

    let bob_loss = bob_before - t.supply_balance(BOB, "ETH");
    let usdc_rev_gain = t.snapshot_revenue("USDC") - usdc_rev_before;

    // Alice's untouched collateral becomes USDC protocol revenue...
    assert!(
        usdc_rev_gain > 0,
        "collateral should become USDC protocol revenue, gain={}",
        usdc_rev_gain
    );
    // ...while ETH suppliers absorb the debt with no credit for that recovery.
    assert!(
        bob_loss > 0.0,
        "ETH supplier must absorb the socialised debt, loss={:.9}",
        bob_loss
    );

    std::println!(
        "A4-econ non-netting: alice_collateral={:.6} USDC  usdc_revenue_gain={} (raw)  \
eth_supplier_loss={:.9} ETH",
        alice_collateral_before,
        usdc_rev_gain,
        bob_loss
    );
}

/// Raw scaled (supply, borrow) position of `user` in `asset`, straight off the
/// controller's account maps.
fn scaled_positions(t: &LendingTest, user: &str, asset: &str) -> (i128, i128) {
    let account_id = t.resolve_account_id(user);
    let key = hub_asset(t.resolve_asset(asset));
    let (supplies, borrows) = t.ctrl_client().get_account_positions(&account_id);
    (
        supplies
            .get(key.clone())
            .map(|p| p.scaled_amount)
            .unwrap_or(0),
        borrows.get(key).map(|p| p.scaled_amount).unwrap_or(0),
    )
}

/// Total supplied underlying for `asset`, in asset units.
fn supplied_amount(t: &LendingTest, asset: &str) -> i128 {
    let asset_addr = t.resolve_asset(asset);
    t.pool_client(asset)
        .get_supplied_amount(&hub_asset(asset_addr))
}

/// LEAD B — the deposit-side absorb runs before the borrow-side writedown on
/// the same market (`bad_debt.rs:23-48` pushes Deposit entries first). This
/// pins that the ordering conserves exactly: it neither double-charges the
/// outside suppliers nor loses part of the loss.
///
/// `absorb_supply_as_revenue` moves shares into `revenue` while leaving
/// `supplied` untouched, and `apply_bad_debt_to_supply_index` reads only
/// `supplied` and `supply_index`. The two writes therefore commute, and the
/// seized collateral still absorbs its own pro-rata slice of the loss.
#[test]
fn test_same_market_absorb_before_writedown_conserves_exactly() {
    let mut t = setup();

    // Outside supplier, who must eat only their pro-rata slice.
    t.supply(BOB, "ETH", 100.0);

    // The doomed account holds BOTH a supply and a debt position in ETH — the
    // only shape where the Deposit-then-Borrow entry order can matter.
    t.supply(ALICE, "USDC", 200.0);
    t.supply(ALICE, "ETH", 0.01);
    t.borrow(ALICE, "ETH", 0.05);

    let eth_before = market_state(&t, "ETH");
    let bob_before = t.supply_balance_raw(BOB, "ETH");
    let (alice_supply_scaled, alice_debt_scaled) = scaled_positions(&t, ALICE, "ETH");
    let alice_supply_value = t.supply_balance_raw(ALICE, "ETH");
    let alice_debt_value = t.borrow_balance_raw(ALICE, "ETH");
    let supplied_value_before = supplied_amount(&t, "ETH");

    assert!(
        alice_supply_scaled > 0 && alice_debt_scaled > 0,
        "fixture needs a same-market supply AND debt: supply={alice_supply_scaled} debt={alice_debt_scaled}"
    );

    t.set_price("USDC", usd_cents(1));
    let account_id = t.resolve_account_id(ALICE);
    t.force_socialize_bad_debt_by_id(account_id);

    let eth_after = market_state(&t, "ETH");
    let bob_after = t.supply_balance_raw(BOB, "ETH");

    // Deposit side: shares are reclassified, never created or destroyed.
    assert_eq!(
        eth_after.supplied, eth_before.supplied,
        "absorb must not change total supply shares"
    );
    assert_eq!(
        eth_after.revenue - eth_before.revenue,
        alice_supply_scaled,
        "revenue must rise by exactly the seized scaled supply"
    );
    assert_eq!(
        eth_after.cash, eth_before.cash,
        "seize must never touch the cash book (INV-ACCT-02)"
    );

    // Borrow side: the debt shares are burned in full.
    assert_eq!(
        eth_before.borrowed - eth_after.borrowed,
        alice_debt_scaled,
        "borrowed must fall by exactly the seized scaled debt"
    );

    // INV-ACCT-09: seize cannot produce debt with no supply.
    assert!(
        !(eth_after.supplied == 0 && eth_after.borrowed != 0),
        "seize left debt with zero supply"
    );

    // Conservation. The loss lands once, spread over EVERY live supply share,
    // including the slice just reclassified to the treasury.
    let bob_loss = bob_before - bob_after;
    let expected_bob_loss = alice_debt_value * bob_before / supplied_value_before;

    assert!(bob_loss > 0, "Bob must absorb part of the loss: {bob_loss}");
    let deviation = (bob_loss - expected_bob_loss).abs();
    assert!(
        deviation * 10_000 <= expected_bob_loss,
        "Bob's loss must equal his pro-rata slice of the burned debt to within 1bp: \
         got={bob_loss} expected={expected_bob_loss} supplied_value={supplied_value_before}"
    );

    // The double-count shape is strictly excluded: Bob's slice must stay below
    // the loss he would carry had the seized collateral been burned instead of
    // reclassified into revenue.
    let double_counted =
        alice_debt_value * bob_before / (supplied_value_before - alice_supply_value);
    assert!(
        bob_loss < double_counted,
        "Bob's loss {bob_loss} reached the burn-the-collateral figure {double_counted}"
    );
}

/// LEAD A — INV-LIQ-04 says socialization is "total", and `ops/seize.rs`
/// commits with no `guards::` assertion. This pins what the missing guard would
/// have checked: after a large socialization the market is still fully backed
/// (`require_backed_market` admits new supply) and still holds supply against
/// its debt (INV-ACCT-09), with the cash book untouched (INV-ACCT-02).
///
/// It also records the reachability bound on the one real deviation. The
/// `SUPPLY_INDEX_FLOOR_RAW` clamp (`interest.rs:90`) makes a wipeout partial,
/// but `require_utilization_below_max` caps debt at 95% of supply value, so a
/// single ordinary liquidation cannot drive the index anywhere near the floor.
#[test]
fn test_socialization_leaves_the_market_backed_and_open() {
    let mut t = setup();

    // Push the ETH market as close to its utilization ceiling as the fixture
    // allows, so the write-down is as violent as one liquidation can make it.
    t.supply(BOB, "ETH", 0.01);
    t.supply(ALICE, "USDC", 100.0);
    t.borrow(ALICE, "ETH", 0.005);

    let eth_before = market_state(&t, "ETH");

    t.set_price("USDC", usd_cents(1));
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 0.001);

    let eth_after = market_state(&t, "ETH");
    let (si_after, _) = get_indexes(&t, "ETH");

    assert!(
        si_after < eth_before.supply_index,
        "fixture must actually socialize: before={} after={si_after}",
        eth_before.supply_index
    );

    // The utilization ceiling keeps a single liquidation far away from the
    // index floor, which is what bounds the INV-LIQ-04 "total" deviation.
    assert!(
        si_after > controller::constants::SUPPLY_INDEX_FLOOR_RAW * 100,
        "one liquidation should not approach the index floor: si={si_after} floor={}",
        controller::constants::SUPPLY_INDEX_FLOOR_RAW
    );

    // INV-ACCT-09 — debt never outlives supply on the seize path.
    assert!(
        !(eth_after.supplied == 0 && eth_after.borrowed != 0),
        "seize left debt with zero supply"
    );

    // INV-ACCT-04 — the market is still backed, so it is open to new supply.
    // This is the post-condition `ops/seize.rs` never asserts.
    t.supply(DAVE, "ETH", 1.0);
    assert!(
        t.supply_balance(DAVE, "ETH") > 0.0,
        "a solvent post-socialization market must still accept supply"
    );
}
