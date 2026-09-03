use controller::constants::RAY_DECIMALS;
use controller::types::{ControllerKey, SpokeAssetArgs, SpokeUsageRaw};
use soroban_sdk::{vec, Vec};
use test_harness::{
    assert_contract_error, errors, hub_asset, map_try_ok_unit, map_try_ok_value, usd_cents,
    usdc_preset, usdt_stable_preset, HubAssetKey, LendingTest, ALICE, BOB, HARNESS_HUB,
    HARNESS_SPOKE, LIQUIDATOR, STABLECOIN_SPOKE, UNCONSTRAINED_TEST_CAP,
};

const UNIT: i128 = 10_000_000;

fn spoke_cap_args(
    t: &LendingTest,
    spoke_id: u32,
    asset_name: &str,
    supply_cap: i128,
    borrow_cap: i128,
) -> SpokeAssetArgs {
    let asset = t.resolve_asset(asset_name);
    let cfg = t
        .ctrl_client()
        .get_spoke_asset(&spoke_id, &hub_asset(asset.clone()));
    SpokeAssetArgs {
        hub_id: HARNESS_HUB,
        asset,
        spoke_id,
        can_collateral: cfg.is_collateralizable,
        can_borrow: cfg.is_borrowable,
        paused: cfg.paused,
        frozen: cfg.frozen,
        no_seize: false,
        ltv: cfg.loan_to_value,
        threshold: cfg.liquidation_threshold,
        bonus: cfg.liquidation_bonus,
        liquidation_fees: cfg.liquidation_fees,
        supply_cap,
        borrow_cap,
    }
}

fn set_spoke_caps(
    t: &LendingTest,
    spoke_id: u32,
    asset_name: &str,
    supply_cap: i128,
    borrow_cap: i128,
) {
    t.ctrl_client().edit_asset_in_spoke(&spoke_cap_args(
        t, spoke_id, asset_name, supply_cap, borrow_cap,
    ));
}

fn try_set_spoke_caps(
    t: &LendingTest,
    spoke_id: u32,
    asset_name: &str,
    supply_cap: i128,
    borrow_cap: i128,
) -> Result<(), soroban_sdk::Error> {
    let args = spoke_cap_args(t, spoke_id, asset_name, supply_cap, borrow_cap);
    map_try_ok_unit(t.ctrl_client().try_edit_asset_in_spoke(&args))
}

fn spoke_usage(t: &LendingTest, spoke_id: u32, asset_name: &str) -> SpokeUsageRaw {
    let asset = t.resolve_asset(asset_name);
    t.env.as_contract(&t.controller, || {
        t.env
            .storage()
            .persistent()
            .get::<_, SpokeUsageRaw>(&ControllerKey::SpokeUsage(spoke_id, hub_asset(asset)))
            .unwrap_or_default()
    })
}

/// A080 plant: drop the durable usage row while account positions stay live.
fn clear_spoke_usage(t: &LendingTest, spoke_id: u32, asset_name: &str) {
    let asset = t.resolve_asset(asset_name);
    t.env.as_contract(&t.controller, || {
        t.env
            .storage()
            .persistent()
            .remove(&ControllerKey::SpokeUsage(spoke_id, hub_asset(asset)));
    });
}

fn spoke_usage_row_present(t: &LendingTest, spoke_id: u32, asset_name: &str) -> bool {
    let asset = t.resolve_asset(asset_name);
    t.env.as_contract(&t.controller, || {
        t.env
            .storage()
            .persistent()
            .has(&ControllerKey::SpokeUsage(spoke_id, hub_asset(asset)))
    })
}

fn try_supply_raw(
    t: &mut LendingTest,
    user: &str,
    asset_name: &str,
    amount: i128,
) -> Result<u64, soroban_sdk::Error> {
    let addr = t.get_or_create_user(user);
    let market = t.resolve_market(asset_name);
    let asset_addr = market.asset.clone();
    market.token_admin.mint(&addr, &amount);

    let account_id = t.default_account_id_or_zero(user);
    let spoke_id = t.ctrl_client().get_account_attributes(&account_id).spoke_id;

    let assets: Vec<(HubAssetKey, i128)> = vec![&t.env, (hub_asset(asset_addr), amount)];
    map_try_ok_value(
        t.ctrl_client()
            .try_supply(&addr, &account_id, &spoke_id, &assets),
    )
}

fn try_borrow_raw(
    t: &mut LendingTest,
    user: &str,
    asset_name: &str,
    amount: i128,
) -> Result<(), soroban_sdk::Error> {
    let account_id = t.resolve_account_id(user);
    let addr = t.get_or_create_user(user);
    let asset_addr = t.resolve_asset(asset_name);

    let borrows: Vec<(HubAssetKey, i128)> = vec![&t.env, (hub_asset(asset_addr), amount)];
    map_try_ok_unit(
        t.ctrl_client()
            .try_borrow(&addr, &account_id, &borrows, &None),
    )
}

fn usdc_spoke_market() -> LendingTest {
    LendingTest::new()
        .with_market(usdc_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .build()
}

fn usdc_usdt_spoke_market() -> LendingTest {
    LendingTest::new().stablecoin_spoke_two_asset().build()
}

#[test]
fn test_zero_supply_cap_rejects_every_supply() {
    let mut t = usdc_spoke_market();
    set_spoke_caps(&t, 2, "USDC", 0, UNCONSTRAINED_TEST_CAP);
    t.create_spoke_account(ALICE, 2);

    assert_contract_error(
        t.try_supply(ALICE, "USDC", 1_000.0),
        errors::SPOKE_SUPPLY_CAP_REACHED,
    );
    assert_contract_error(
        try_supply_raw(&mut t, ALICE, "USDC", 1),
        errors::SPOKE_SUPPLY_CAP_REACHED,
    );

    assert_eq!(
        spoke_usage(&t, 2, "USDC").supplied_scaled_ray,
        0,
        "a rejected supply must not record spoke usage"
    );
}

#[test]
fn test_zero_borrow_cap_rejects_every_borrow() {
    let mut t = usdc_usdt_spoke_market();
    set_spoke_caps(&t, 2, "USDT", UNCONSTRAINED_TEST_CAP, 0);
    t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 10_000.0);

    assert_contract_error(
        t.try_borrow(ALICE, "USDT", 1_000.0),
        errors::SPOKE_BORROW_CAP_REACHED,
    );
    assert_contract_error(
        try_borrow_raw(&mut t, ALICE, "USDT", 1),
        errors::SPOKE_BORROW_CAP_REACHED,
    );

    assert_eq!(
        spoke_usage(&t, 2, "USDT").borrowed_scaled_ray,
        0,
        "a rejected borrow must not record spoke usage"
    );
}

#[test]
fn test_closed_market_still_allows_full_repay_and_withdraw() {
    let mut t = usdc_usdt_spoke_market();
    t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "USDT", 2_000.0);

    set_spoke_caps(&t, 2, "USDC", 0, 0);
    set_spoke_caps(&t, 2, "USDT", 0, 0);

    assert_contract_error(
        t.try_supply(ALICE, "USDC", 1.0),
        errors::SPOKE_SUPPLY_CAP_REACHED,
    );
    assert_contract_error(
        t.try_borrow(ALICE, "USDT", 1.0),
        errors::SPOKE_BORROW_CAP_REACHED,
    );

    let debt = t.borrow_balance(ALICE, "USDT");
    assert!(debt > 0.0, "the position under test must carry debt");
    t.repay(ALICE, "USDT", debt * 1.01);
    t.assert_borrow_count(ALICE, 0);

    let account_id = t.resolve_account_id(ALICE);
    t.withdraw_all(ALICE, "USDC");
    assert_eq!(
        t.supply_balance_for(ALICE, account_id, "USDC"),
        0.0,
        "a closed market must still let the whole collateral position out"
    );
    t.assert_no_positions_for(ALICE, account_id);

    assert_eq!(
        spoke_usage(&t, 2, "USDC").supplied_scaled_ray,
        0,
        "withdrawing from a closed market must drain supply usage"
    );
    assert_eq!(
        spoke_usage(&t, 2, "USDT").borrowed_scaled_ray,
        0,
        "repaying into a closed market must drain borrow usage"
    );
}

#[test]
fn test_closed_market_still_allows_bad_debt_cleanup() {
    let mut t = LendingTest::new().standard_two_asset_dust_disabled();

    t.supply(ALICE, "USDC", 10.0);
    t.borrow(ALICE, "ETH", 0.003);

    set_spoke_caps(&t, HARNESS_SPOKE, "USDC", 0, 0);
    set_spoke_caps(&t, HARNESS_SPOKE, "ETH", 0, 0);
    assert_contract_error(
        t.try_supply(ALICE, "USDC", 1.0),
        errors::SPOKE_SUPPLY_CAP_REACHED,
    );

    t.set_price("USDC", usd_cents(1));
    assert!(
        t.can_be_liquidated(ALICE),
        "the crashed position must be liquidatable before cleanup"
    );

    let account_id = t.resolve_account_id(ALICE);
    t.clean_bad_debt_for(ALICE);
    t.assert_no_positions_for(ALICE, account_id);

    assert_eq!(
        spoke_usage(&t, HARNESS_SPOKE, "USDC").supplied_scaled_ray,
        0,
        "cleanup must drain the seized collateral usage"
    );
    assert_eq!(
        spoke_usage(&t, HARNESS_SPOKE, "ETH").borrowed_scaled_ray,
        0,
        "cleanup must drain the written-off debt usage"
    );
}

#[test]
fn test_spoke_cap_at_asset_domain_ceiling_accepted() {
    let t = usdc_spoke_market();
    assert_eq!(
        t.get_asset_config("USDC").asset_decimals,
        7,
        "the ceiling below is derived for a 7-decimal market"
    );

    let ceiling = common::validation::max_cap_for_decimals(7);
    assert_eq!(
        ceiling, 1_701_411_834_604_692_317,
        "i128::MAX / 10^(27 - 7) must stay the 7-decimal cap ceiling; the \
         div_floor by the supply index saturates rather than overflowing"
    );

    assert!(
        try_set_spoke_caps(&t, 2, "USDC", ceiling, ceiling).is_ok(),
        "a cap exactly at the asset domain ceiling must be accepted"
    );

    let cfg = t
        .ctrl_client()
        .get_spoke_asset(&2u32, &hub_asset(t.resolve_asset("USDC")));
    assert_eq!(cfg.supply_cap, ceiling);
    assert_eq!(cfg.borrow_cap, ceiling);
}

#[test]
fn test_spoke_cap_above_asset_domain_ceiling_rejected() {
    let t = usdc_spoke_market();
    let over = common::validation::max_cap_for_decimals(7) + 1;

    assert_contract_error(
        try_set_spoke_caps(&t, 2, "USDC", over, UNCONSTRAINED_TEST_CAP),
        errors::INVALID_BORROW_PARAMS,
    );
    assert_contract_error(
        try_set_spoke_caps(&t, 2, "USDC", UNCONSTRAINED_TEST_CAP, over),
        errors::INVALID_BORROW_PARAMS,
    );
}

#[test]
fn test_edit_asset_in_spoke_rejects_i128_max_cap() {
    let t = usdc_spoke_market();

    assert_contract_error(
        try_set_spoke_caps(&t, 2, "USDC", i128::MAX, UNCONSTRAINED_TEST_CAP),
        errors::INVALID_BORROW_PARAMS,
    );
    assert_contract_error(
        try_set_spoke_caps(&t, 2, "USDC", UNCONSTRAINED_TEST_CAP, i128::MAX),
        errors::INVALID_BORROW_PARAMS,
    );
}

#[test]
fn test_add_asset_to_spoke_rejects_i128_max_cap() {
    let t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .build();

    let args = SpokeAssetArgs {
        hub_id: HARNESS_HUB,
        asset: t.resolve_asset("USDT"),
        spoke_id: 2,
        can_collateral: true,
        can_borrow: true,
        paused: false,
        frozen: false,
        no_seize: false,
        ltv: 9_000,
        threshold: 9_300,
        bonus: 200,
        liquidation_fees: 0,
        supply_cap: i128::MAX,
        borrow_cap: UNCONSTRAINED_TEST_CAP,
    };
    let result = map_try_ok_unit(t.ctrl_client().try_add_asset_to_spoke(&args));
    assert_contract_error(result, errors::INVALID_BORROW_PARAMS);
}

#[test]
fn test_supply_of_exactly_the_cap_succeeds_then_one_unit_reverts() {
    let cap = 1_000 * UNIT;
    let mut t = usdc_spoke_market();
    set_spoke_caps(&t, 2, "USDC", cap, UNCONSTRAINED_TEST_CAP);
    t.create_spoke_account(ALICE, 2);

    t.supply_raw(ALICE, "USDC", cap);
    assert_eq!(
        spoke_usage(&t, 2, "USDC").supplied_scaled_ray,
        cap * 10i128.pow(RAY_DECIMALS - 7),
        "supplying exactly the cap must land on the ceiling, not below it"
    );

    assert_contract_error(
        try_supply_raw(&mut t, ALICE, "USDC", 1),
        errors::SPOKE_SUPPLY_CAP_REACHED,
    );
}

#[test]
fn test_single_supply_of_cap_plus_one_reverts() {
    let cap = 1_000 * UNIT;
    let mut t = usdc_spoke_market();
    set_spoke_caps(&t, 2, "USDC", cap, UNCONSTRAINED_TEST_CAP);
    t.create_spoke_account(ALICE, 2);

    assert_contract_error(
        try_supply_raw(&mut t, ALICE, "USDC", cap + 1),
        errors::SPOKE_SUPPLY_CAP_REACHED,
    );
    assert_eq!(
        spoke_usage(&t, 2, "USDC").supplied_scaled_ray,
        0,
        "the rejected supply must not have been partially applied"
    );

    t.supply_raw(ALICE, "USDC", cap);
    assert_eq!(
        spoke_usage(&t, 2, "USDC").supplied_scaled_ray,
        cap * 10i128.pow(RAY_DECIMALS - 7)
    );
}

#[test]
fn test_borrow_of_exactly_the_cap_succeeds_then_one_unit_reverts() {
    let cap = 500 * UNIT;
    let mut t = usdc_usdt_spoke_market();
    set_spoke_caps(&t, 2, "USDT", UNCONSTRAINED_TEST_CAP, cap);
    t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 10_000.0);

    t.borrow_raw(ALICE, "USDT", cap);
    assert_contract_error(
        try_borrow_raw(&mut t, ALICE, "USDT", 1),
        errors::SPOKE_BORROW_CAP_REACHED,
    );
}

/// A080 PIN — missing usage row under-counts occupancy: live supply remains, but
/// the cap check treats usage as zero and admits a second full-cap fill.
#[test]
fn test_missing_usage_row_allows_supply_fill_to_full_cap_while_positions_live() {
    let cap = 1_000 * UNIT;
    let mut t = usdc_spoke_market();
    set_spoke_caps(&t, 2, "USDC", cap, UNCONSTRAINED_TEST_CAP);
    t.create_spoke_account(ALICE, 2);
    t.create_spoke_account(BOB, 2);

    t.supply_raw(ALICE, "USDC", cap);
    assert!(spoke_usage_row_present(&t, 2, "USDC"));
    assert_contract_error(
        try_supply_raw(&mut t, BOB, "USDC", 1),
        errors::SPOKE_SUPPLY_CAP_REACHED,
    );

    clear_spoke_usage(&t, 2, "USDC");
    assert!(
        !spoke_usage_row_present(&t, 2, "USDC"),
        "planted desync: usage row gone while Alice still holds supply"
    );
    assert!(
        t.supply_balance(ALICE, "USDC") > 0.0,
        "Alice's position must still be live after the usage plant"
    );

    // Cap check starts from absent=0, so Bob can fill the configured cap again.
    t.supply_raw(BOB, "USDC", cap);
    assert_eq!(
        spoke_usage(&t, 2, "USDC").supplied_scaled_ray,
        cap * 10i128.pow(RAY_DECIMALS - 7),
        "post-plant usage only reflects Bob's fill, not Alice's still-live shares"
    );
    assert!(
        t.supply_balance(ALICE, "USDC") > 0.0 && t.supply_balance(BOB, "USDC") > 0.0,
        "both accounts keep live supply — spoke occupancy is under-counted vs positions"
    );
    assert_contract_error(
        try_supply_raw(&mut t, BOB, "USDC", 1),
        errors::SPOKE_SUPPLY_CAP_REACHED,
    );
}

/// A080 PIN — withdraw against a missing usage row is a silent no-op on usage;
/// remaining positions still leave the spoke able to refill to the full configured cap.
#[test]
fn test_missing_usage_row_withdraw_then_supply_fills_to_configured_cap() {
    let cap = 1_000 * UNIT;
    let mut t = usdc_spoke_market();
    set_spoke_caps(&t, 2, "USDC", cap, UNCONSTRAINED_TEST_CAP);
    t.create_spoke_account(ALICE, 2);
    t.create_spoke_account(BOB, 2);

    t.supply_raw(ALICE, "USDC", cap);
    clear_spoke_usage(&t, 2, "USDC");

    t.withdraw(ALICE, "USDC", 400.0);
    assert!(
        !spoke_usage_row_present(&t, 2, "USDC"),
        "A080: non-zero exit must not invent a usage row when storage was empty"
    );
    let alice_remaining = t.supply_balance(ALICE, "USDC");
    assert!(
        alice_remaining > 0.0,
        "Alice still holds residual supply after the partial withdraw"
    );

    // Occupancy under-count: recorded usage is still zero, so Bob can take the
    // entire configured cap even though Alice's residual shares remain live.
    t.supply_raw(BOB, "USDC", cap);
    assert_eq!(
        spoke_usage(&t, 2, "USDC").supplied_scaled_ray,
        cap * 10i128.pow(RAY_DECIMALS - 7)
    );
    assert!(
        t.supply_balance(ALICE, "USDC") > 0.0,
        "Alice residual + Bob full-cap fill = over-admission vs configured cap"
    );
}

/// A080 PIN — borrow-side twin: missing usage + repay no-op → refill to borrow cap.
#[test]
fn test_missing_usage_row_repay_then_borrow_fills_to_configured_borrow_cap() {
    let borrow_cap = 500 * UNIT;
    let mut t = usdc_usdt_spoke_market();
    set_spoke_caps(&t, 2, "USDT", UNCONSTRAINED_TEST_CAP, borrow_cap);
    t.create_spoke_account(ALICE, 2);
    t.create_spoke_account(BOB, 2);

    t.supply(ALICE, "USDC", 20_000.0);
    t.supply(BOB, "USDC", 20_000.0);
    // Seed borrow liquidity in the pool.
    t.supply(LIQUIDATOR, "USDT", 5_000.0);

    t.borrow_raw(ALICE, "USDT", borrow_cap);
    assert!(spoke_usage_row_present(&t, 2, "USDT"));
    assert_contract_error(
        try_borrow_raw(&mut t, BOB, "USDT", 1),
        errors::SPOKE_BORROW_CAP_REACHED,
    );

    clear_spoke_usage(&t, 2, "USDT");
    assert!(!spoke_usage_row_present(&t, 2, "USDT"));

    let alice_debt = t.borrow_balance(ALICE, "USDT");
    assert!(alice_debt > 0.0);
    t.repay(ALICE, "USDT", alice_debt * 0.25);
    assert!(
        !spoke_usage_row_present(&t, 2, "USDT"),
        "A080: repay exit against a missing row must leave usage absent"
    );
    assert!(
        t.borrow_balance(ALICE, "USDT") > 0.0,
        "Alice still carries residual debt after the partial repay"
    );

    t.borrow_raw(BOB, "USDT", borrow_cap);
    assert_eq!(
        spoke_usage(&t, 2, "USDT").borrowed_scaled_ray,
        borrow_cap * 10i128.pow(RAY_DECIMALS - 7),
        "Bob's full-cap borrow is booked as if Alice's residual debt were absent"
    );
    assert!(t.borrow_balance(ALICE, "USDT") > 0.0 && t.borrow_balance(BOB, "USDT") > 0.0);
}
