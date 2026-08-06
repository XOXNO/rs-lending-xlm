//! Regression coverage for spoke caps after the "cap == 0 means unlimited"
//! sentinel was removed.
//!
//! A cap is always an enforced ceiling in asset units: `0` closes the market
//! on that side and `i128::MAX` is an ordinary value that the per-asset domain
//! check rejects at config time. Exits stay uncapped, so closing a market must
//! never trap an existing position.

use controller::constants::RAY_DECIMALS;
use controller::types::{ControllerKey, SpokeAssetArgs, SpokeUsageRaw};
use soroban_sdk::{vec, Vec};
use test_harness::{
    assert_contract_error, errors, eth_preset, hub_asset, usd_cents, usdc_preset,
    usdt_stable_preset, HubAssetKey, LendingTest, ALICE, HARNESS_HUB, HARNESS_SPOKE,
    STABLECOIN_SPOKE, UNCONSTRAINED_TEST_CAP,
};

/// One whole token at the 7 decimals every harness market uses.
const UNIT: i128 = 10_000_000;

/// Rebuilds the stored listing with new caps, leaving every other field alone
/// so a cap edit cannot be confused with a flag or risk-parameter change.
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
    match t.ctrl_client().try_edit_asset_in_spoke(&args) {
        Ok(res) => res.map_err(|e| e.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    }
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

/// Raw-amount `try_supply`. The harness only exposes a panicking `supply_raw`
/// and an `f64` `try_supply`; cap boundaries need an exact sub-unit amount and
/// a recoverable error.
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
    match t
        .ctrl_client()
        .try_supply(&addr, &account_id, &spoke_id, &assets)
    {
        Ok(Ok(id)) => Ok(id),
        Ok(Err(err)) => Err(err),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    }
}

/// Raw-amount `try_borrow`, for the same reason as `try_supply_raw`.
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
    match t
        .ctrl_client()
        .try_borrow(&addr, &account_id, &borrows, &None)
    {
        Ok(Ok(())) => Ok(()),
        Ok(Err(err)) => Err(err.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    }
}

fn usdc_spoke_market() -> LendingTest {
    LendingTest::new()
        .with_market(usdc_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .build()
}

fn usdc_usdt_spoke_market() -> LendingTest {
    LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .with_spoke_asset(2, "USDT", true, true)
        .build()
}

/// `supply_cap == 0` closes the supply side outright. Under the old sentinel
/// this listing was treated as unlimited and every one of these supplies
/// succeeded.
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

/// `borrow_cap == 0` closes the borrow side outright, even for an account with
/// ample collateral.
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

/// The load-bearing property of the new semantics: closing a market blocks new
/// entries but never traps an existing position. Repay and withdraw are
/// deliberately not routed through the cap check, so a fully closed market
/// still unwinds to zero.
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

/// Bad-debt cleanup seizes both sides through the exit path, so it must keep
/// working against a market that accepts nothing.
#[test]
fn test_closed_market_still_allows_bad_debt_cleanup() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_dust_disabled_all_markets()
        .build();

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

/// A cap sitting exactly on the per-asset domain ceiling is legal: it is the
/// largest value `Ray::from_asset` can rescale without overflow.
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

/// One unit above the ceiling would overflow `Ray::from_asset` at supply time,
/// so it has to be refused at config time instead.
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

/// `i128::MAX` lost its "unlimited" exemption, so `edit_asset_in_spoke` now
/// rejects it like any other out-of-domain cap.
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

/// The listing entry point has to reject `i128::MAX` too, otherwise a market
/// could be created with a cap that panics on its first supply.
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
        ltv: 9_000,
        threshold: 9_300,
        bonus: 200,
        liquidation_fees: 0,
        supply_cap: i128::MAX,
        borrow_cap: UNCONSTRAINED_TEST_CAP,
    };
    let result = match t.ctrl_client().try_add_asset_to_spoke(&args) {
        Ok(res) => res.map_err(|e| e.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    };
    assert_contract_error(result, errors::INVALID_BORROW_PARAMS);
}

/// The cap is inclusive: usage may reach it exactly, and the next sub-unit is
/// refused.
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

/// A single supply of `cap + 1` is refused whole; the market is not left
/// partially filled, and a supply of exactly `cap` still goes through after.
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

/// Borrow side of the same inclusive boundary.
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
