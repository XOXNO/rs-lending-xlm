use controller::types::PositionMode;
use soroban_sdk::{Address, Bytes};
use test_harness::mock_aggregator::{ReenterMode, ReenteringAggregator};
use test_harness::mock_blend::{MockBlendClient, KIND_COLLATERAL, KIND_LIABILITY};
use test_harness::{
    apply_flash_fee, assert_contract_error, build_aggregator_swap, errors, eth_preset,
    helpers::f64_to_i128, hub_asset, map_try_ok_value, usdc_preset, LendingTest, ALICE, BOB,
    HARNESS_HUB,
};

use crate::helpers::register_approved_blend;

fn flash_guard_cleared(t: &LendingTest) -> bool {
    t.env.as_contract(&t.controller, || {
        !controller::test_support::is_flash_loan_ongoing(&t.env)
    })
}

fn mint_weird(t: &LendingTest, asset_name: &str, to: &Address, amount: i128) {
    let asset = t.resolve_asset(asset_name);
    test_harness::weird_token::WeirdTokenClient::new(&t.env, &asset).mint(to, &amount);
}

fn install_reentering_router(t: &LendingTest, mode: ReenterMode) -> Address {
    let router = t
        .env
        .register(ReenteringAggregator, (t.controller.clone(), mode));
    t.ctrl_client().set_swap_aggregator(&router);
    router
}

fn fund_router_at(t: &LendingTest, router: &Address, asset_name: &str, amount: f64) {
    let market = t.resolve_market(asset_name);
    let raw = f64_to_i128(amount, market.decimals);
    market.token_admin.mint(router, &raw);
}

fn multiply_steps(t: &LendingTest) -> soroban_sdk::Bytes {
    build_aggregator_swap(t, "ETH", "USDC", apply_flash_fee(10_000_000), 3000_0000000)
}

fn try_multiply_any(
    t: &mut LendingTest,
    steps: &soroban_sdk::Bytes,
) -> Result<u64, std::string::String> {
    let caller = t.get_or_create_user(ALICE);
    let collateral = hub_asset(t.resolve_asset("USDC"));
    let debt = hub_asset(t.resolve_asset("ETH"));
    match t.ctrl_client().try_multiply(
        &caller,
        &0u64,
        &1u32,
        &collateral,
        &10_000_000i128,
        &debt,
        &PositionMode::Multiply,
        steps,
        &None,
        &None,
    ) {
        Ok(Ok(id)) => Ok(id),
        Ok(Err(err)) => Err(std::format!("{err:?}")),
        Err(Ok(err)) => Err(std::format!("{err:?}")),
        Err(Err(err)) => Err(std::format!("{err:?}")),
    }
}

fn assert_router_reentry_rejects_multiply(mode: ReenterMode) {
    let mut t = LendingTest::new().standard_two_asset().build();
    let router = install_reentering_router(&t, mode);
    fund_router_at(&t, &router, "USDC", 3_000.0);
    let steps = multiply_steps(&t);
    let result = try_multiply_any(&mut t, &steps);
    // The host refuses the router's call back into the controller with
    // Error(Context, InvalidAction) before the #400 flash guard is reached;
    // ReenterMode::Panic traps into the same host verdict.
    assert_eq!(
        result.unwrap_err(),
        "Error(Context, InvalidAction)",
        "multiply via reentering router {mode:?} must be refused by the host"
    );
    assert!(
        flash_guard_cleared(&t),
        "flash guard must clear after multiply router reentry {mode:?}"
    );
}

#[test]
fn test_multiply_router_reenter_supply_rejects() {
    assert_router_reentry_rejects_multiply(ReenterMode::Supply);
}

#[test]
fn test_multiply_router_reenter_borrow_rejects() {
    assert_router_reentry_rejects_multiply(ReenterMode::Borrow);
}

#[test]
fn test_multiply_router_reenter_flash_loan_rejects() {
    assert_router_reentry_rejects_multiply(ReenterMode::FlashLoan);
}

#[test]
fn test_multiply_router_panic_rolls_back() {
    assert_router_reentry_rejects_multiply(ReenterMode::Panic);
}

fn seed_swap_debt_position(t: &mut LendingTest) {
    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);
}

fn try_swap_debt_any(t: &mut LendingTest, steps: &Bytes) -> Result<(), std::string::String> {
    let account_id = t.resolve_account_id(ALICE);
    let addr = t.get_or_create_user(ALICE);
    let existing = hub_asset(t.resolve_asset("ETH"));
    let new = hub_asset(t.resolve_asset("USDC"));
    match t.ctrl_client().try_swap_debt(
        &addr,
        &account_id,
        &existing,
        &1_000_000_000i128,
        &new,
        steps,
    ) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(err)) => Err(std::format!("{err:?}")),
        Err(Ok(err)) => Err(std::format!("{err:?}")),
        Err(Err(err)) => Err(std::format!("{err:?}")),
    }
}

fn try_swap_collateral_any(t: &mut LendingTest, steps: &Bytes) -> Result<(), std::string::String> {
    let account_id = t.resolve_account_id(ALICE);
    let addr = t.get_or_create_user(ALICE);
    let current = hub_asset(t.resolve_asset("USDC"));
    let new = hub_asset(t.resolve_asset("ETH"));
    match t.ctrl_client().try_swap_collateral(
        &addr,
        &account_id,
        &current,
        &1_000_000_000i128,
        &new,
        steps,
    ) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(err)) => Err(std::format!("{err:?}")),
        Err(Ok(err)) => Err(std::format!("{err:?}")),
        Err(Err(err)) => Err(std::format!("{err:?}")),
    }
}

fn try_rdwc_any(t: &mut LendingTest, steps: &Bytes) -> Result<(), std::string::String> {
    let account_id = t.resolve_account_id(ALICE);
    let addr = t.get_or_create_user(ALICE);
    let collateral = hub_asset(t.resolve_asset("USDC"));
    let debt = hub_asset(t.resolve_asset("ETH"));
    match t.ctrl_client().try_repay_debt_with_collateral(
        &addr,
        &account_id,
        &collateral,
        &1_000_000_000i128,
        &debt,
        steps,
        &false,
    ) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(err)) => Err(std::format!("{err:?}")),
        Err(Ok(err)) => Err(std::format!("{err:?}")),
        Err(Err(err)) => Err(std::format!("{err:?}")),
    }
}

#[test]
fn test_swap_debt_router_reenter_supply_rejects() {
    let mut t = LendingTest::new().standard_two_asset().build();
    seed_swap_debt_position(&mut t);
    let router = install_reentering_router(&t, ReenterMode::Supply);
    fund_router_at(&t, &router, "ETH", 1.0);
    let steps = build_aggregator_swap(&t, "USDC", "ETH", 1_000_000_000, 1_0000000);
    let result = try_swap_debt_any(&mut t, &steps);
    assert_eq!(
        result.unwrap_err(),
        "Error(Context, InvalidAction)",
        "swap_debt via reentering router must be refused by the host"
    );
    assert!(flash_guard_cleared(&t));
}

#[test]
fn test_swap_collateral_router_reenter_supply_rejects() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(ALICE, "USDC", 100_000.0);
    let router = install_reentering_router(&t, ReenterMode::Supply);
    fund_router_at(&t, &router, "ETH", 5.0);
    let steps = build_aggregator_swap(&t, "USDC", "ETH", 1_000_000_000, 1_0000000);
    let result = try_swap_collateral_any(&mut t, &steps);
    assert_eq!(
        result.unwrap_err(),
        "Error(Context, InvalidAction)",
        "swap_collateral via reentering router must be refused by the host"
    );
    assert!(flash_guard_cleared(&t));
}

#[test]
fn test_rdwc_router_reenter_supply_rejects() {
    let mut t = LendingTest::new().standard_two_asset().build();
    seed_swap_debt_position(&mut t);
    let router = install_reentering_router(&t, ReenterMode::Supply);
    fund_router_at(&t, &router, "ETH", 1.0);
    let steps = build_aggregator_swap(&t, "USDC", "ETH", 1_000_000_000, 1_0000000);
    let result = try_rdwc_any(&mut t, &steps);
    assert_eq!(
        result.unwrap_err(),
        "Error(Context, InvalidAction)",
        "rdwc via reentering router must be refused by the host"
    );
    assert!(flash_guard_cleared(&t));
}

#[test]
fn test_multiply_fee_on_transfer_debt_fails_closed() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_fee_on_transfer_market(eth_preset(), 100)
        .build();
    t.fund_router("USDC", 3_000.0);
    let steps = multiply_steps(&t);
    let result = try_multiply_any(&mut t, &steps);
    let err = result.expect_err("fee-on-transfer debt must fail closed on multiply");
    assert!(
        err.contains("#34"),
        "measured mint must disagree with pool-reported amount, got {err}"
    );
    assert!(flash_guard_cleared(&t));
}

#[test]
fn test_multiply_fee_on_transfer_collateral_credits_net_or_fails() {
    let mut t = LendingTest::new()
        .with_fee_on_transfer_market(usdc_preset(), 100)
        .with_market(eth_preset())
        .build();
    let router = t.aggregator.clone();
    mint_weird(&t, "USDC", &router, 30_000_000_000);
    let steps = multiply_steps(&t);
    let result = try_multiply_any(&mut t, &steps);
    match result {
        Ok(account_id) => {
            let supply = t.supply_balance_for(ALICE, account_id, "USDC");
            assert!(
                supply < 3_000.0,
                "fee-on-transfer collateral must be credited net, got {supply}"
            );
            assert!(
                t.health_factor_for(ALICE, account_id) >= 1.0,
                "net receipt must still be solvent"
            );
        }
        Err(_) => {
            assert!(
                flash_guard_cleared(&t),
                "failed FoT multiply must still clear the flash guard"
            );
        }
    }
}

#[test]
fn test_multiply_extra_credit_is_not_pool_theft() {
    let mut t = LendingTest::new()
        .with_extra_credit_market(usdc_preset(), 100)
        .with_market(eth_preset())
        .build();
    let router = t.aggregator.clone();
    mint_weird(&t, "USDC", &router, 30_000_000_000);
    let eth_cash_before = t.pool_reserves("ETH");
    let steps = multiply_steps(&t);
    let account_id = try_multiply_any(&mut t, &steps)
        .expect("extra credit on collateral must not break a solvent multiply");
    let borrowed = t.borrow_balance_for(ALICE, account_id, "ETH");
    assert!((0.99..=1.01).contains(&borrowed));
    let eth_cash_after = t.pool_reserves("ETH");
    assert!(
        eth_cash_before - eth_cash_after > 0.0,
        "ETH cash must fall by the borrowed amount, not by extra USDC air"
    );
    assert!(flash_guard_cleared(&t));
}

#[test]
fn test_multiply_transfer_hook_on_debt_cannot_reenter() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_transfer_hook_market(eth_preset())
        .build();
    t.fund_router("USDC", 3_000.0);
    let steps = multiply_steps(&t);
    let result = try_multiply_any(&mut t, &steps);
    assert_eq!(
        result.unwrap_err(),
        "Error(Context, InvalidAction)",
        "the debt token's hook must be refused by the host when it reenters"
    );
    assert!(flash_guard_cleared(&t));
}

#[test]
fn test_swap_collateral_transfer_hook_cannot_reenter() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_transfer_hook_market(eth_preset())
        .build();
    t.supply(ALICE, "USDC", 100_000.0);
    t.fund_router("ETH", 5.0);
    let steps = build_aggregator_swap(&t, "USDC", "ETH", 1_000_000_000, 1_0000000);
    let result = try_swap_collateral_any(&mut t, &steps);
    assert_eq!(
        result.unwrap_err(),
        "Error(Context, InvalidAction)",
        "the destination token's hook must be refused by the host when it reenters"
    );
    assert!(flash_guard_cleared(&t));
}

#[test]
fn test_swap_debt_empty_payload_still_rejects_under_reentering_router() {
    let mut t = LendingTest::new().standard_two_asset().build();
    seed_swap_debt_position(&mut t);
    let _ = install_reentering_router(&t, ReenterMode::Supply);
    let empty = Bytes::new(&t.env);
    let result = t.try_swap_debt(ALICE, "ETH", 1.0, "USDC", &empty);
    assert_contract_error(result, errors::INVALID_PAYMENTS);
    assert!(flash_guard_cleared(&t));
}

#[test]
fn test_migrate_blend_submit_hook_cannot_reenter() {
    let mut t = LendingTest::new().standard_two_asset().build();
    let caller = t.get_or_create_user(ALICE);
    let blend_addr = register_approved_blend(&t);
    let blend = MockBlendClient::new(&t.env, &blend_addr);
    let usdc = t.resolve_asset("USDC");
    let eth = t.resolve_asset("ETH");
    let coll = f64_to_i128(2_000.0, t.resolve_market("USDC").decimals);
    let debt = f64_to_i128(0.5, t.resolve_market("ETH").decimals);
    blend.seed(&caller, &usdc, &KIND_COLLATERAL, &coll);
    blend.seed(&caller, &eth, &KIND_LIABILITY, &debt);
    t.resolve_market("USDC")
        .token_admin
        .mint(&blend_addr, &coll);
    blend.set_hook(&t.controller);

    let collateral_assets = soroban_sdk::Vec::from_array(&t.env, [usdc.clone()]);
    let supply_assets = soroban_sdk::Vec::<Address>::new(&t.env);
    let debt_caps = soroban_sdk::Vec::from_array(&t.env, [(eth.clone(), debt)]);
    let result = t.ctrl_client().try_migrate_from_blend(
        &caller,
        &0u64,
        &1u32,
        &HARNESS_HUB,
        &blend_addr,
        &collateral_assets,
        &supply_assets,
        &debt_caps,
    );
    match result {
        Err(Ok(err)) => assert_eq!(
            std::format!("{err:?}"),
            "Error(Context, InvalidAction)",
            "the blend submit hook must be refused by the host when it reenters"
        ),
        other => panic!("blend submit hook must be refused by the host, got {other:?}"),
    }
    assert!(flash_guard_cleared(&t));
}

#[test]
fn test_strategy_entries_still_blocked_by_flag() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 1.0);
    t.set_flash_loan_ongoing(true);
    let empty = Bytes::new(&t.env);
    assert_contract_error(
        t.try_multiply(ALICE, "USDC", 1.0, "ETH", PositionMode::Multiply, &empty),
        errors::FLASH_LOAN_ONGOING,
    );
    assert_contract_error(
        t.try_swap_debt(ALICE, "ETH", 0.01, "USDC", &empty),
        errors::FLASH_LOAN_ONGOING,
    );
    assert_contract_error(
        t.try_swap_collateral(ALICE, "USDC", 1.0, "ETH", &empty),
        errors::FLASH_LOAN_ONGOING,
    );
    assert_contract_error(
        t.try_repay_debt_with_collateral(ALICE, "USDC", 1.0, "ETH", &empty, false),
        errors::FLASH_LOAN_ONGOING,
    );
    t.set_flash_loan_ongoing(false);
}

#[test]
fn test_multiply_wrong_owner_cannot_use_reentering_router() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(ALICE, "USDC", 10_000.0);
    let alice_id = t.resolve_account_id(ALICE);
    let router = install_reentering_router(&t, ReenterMode::Supply);
    fund_router_at(&t, &router, "USDC", 3_000.0);
    let steps = multiply_steps(&t);
    let bob = t.get_or_create_user(BOB);
    let collateral = hub_asset(t.resolve_asset("USDC"));
    let debt = hub_asset(t.resolve_asset("ETH"));
    let result = t.ctrl_client().try_multiply(
        &bob,
        &alice_id,
        &1u32,
        &collateral,
        &10_000_000i128,
        &debt,
        &PositionMode::Multiply,
        &steps,
        &None,
        &None,
    );
    assert_contract_error(map_try_ok_value(result), errors::NOT_AUTHORIZED);
}

#[test]
fn test_swap_debt_same_asset_rejected_before_router() {
    let mut t = LendingTest::new().standard_two_asset().build();
    seed_swap_debt_position(&mut t);
    let steps = build_aggregator_swap(&t, "ETH", "ETH", 0, 1);
    let result = t.try_swap_debt(ALICE, "ETH", 0.1, "ETH", &steps);
    assert_contract_error(result, errors::ASSETS_ARE_THE_SAME);
}
