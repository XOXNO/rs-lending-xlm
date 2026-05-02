extern crate std;

use common::types::{ControllerKey, MarketConfig};
use common::types::{DexDistribution, Protocol, SwapSteps};
use soroban_sdk::{token, vec};
use test_harness::{
    assert_contract_error, errors, eth_preset, usd, usdc_preset, usdt_stable_preset, wbtc_preset,
    LendingTest, MarketPreset, ALICE, BOB, DEFAULT_ASSET_CONFIG, DEFAULT_MARKET_PARAMS,
    STABLECOIN_EMODE,
};

// ---------------------------------------------------------------------------
// Helper: build SwapSteps with a single hop that yields `min_amount_out` from
// the mock swap router.
// ---------------------------------------------------------------------------

fn build_swap_steps(t: &LendingTest, token_in: &str, token_out: &str, min_out: i128) -> SwapSteps {
    let env = &t.env;
    let in_addr = t.resolve_market(token_in).asset.clone();
    let out_addr = t.resolve_market(token_out).asset.clone();
    SwapSteps {
        amount_out_min: min_out,
        distribution: vec![
            env,
            DexDistribution {
                protocol_id: Protocol::Soroswap,
                path: vec![env, in_addr, out_addr],
                parts: 1,
                bytes: None,
            },
        ],
    }
}

fn dai_preset() -> MarketPreset {
    MarketPreset {
        name: "DAI",
        decimals: 7,
        price_wad: usd(1),
        initial_liquidity: 1_000_000.0,
        config: DEFAULT_ASSET_CONFIG,
        params: DEFAULT_MARKET_PARAMS,
    }
}

/// Flatten the nested result returned by the raw `ctrl_client().try_*` calls
/// into `Result<T, soroban_sdk::Error>` so it can feed `assert_contract_error`.
/// A host-level InvokeError (pre-contract host check) is escalated via
/// `.expect()` so regressions that never reach the contract surface loudly.
fn flatten<T>(
    r: Result<Result<T, soroban_sdk::Error>, Result<soroban_sdk::Error, soroban_sdk::InvokeError>>,
) -> Result<T, soroban_sdk::Error> {
    match r {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(e),
        Err(invoke) => Err(invoke.expect("expected contract error, got host-level InvokeError")),
    }
}

fn supply_position_params(t: &LendingTest, account_id: u64, asset_name: &str) -> (i128, i128) {
    let asset = t.resolve_asset(asset_name);
    t.env.as_contract(&t.controller_address(), || {
        let map: soroban_sdk::Map<soroban_sdk::Address, common::types::AccountPosition> = t
            .env
            .storage()
            .persistent()
            .get(&ControllerKey::SupplyPositions(account_id))
            .expect("supply side map should exist");
        let position = map
            .get(asset)
            .expect("supply position should exist for asset");
        (
            position.loan_to_value_bps,
            position.liquidation_threshold_bps,
        )
    })
}

// ===========================================================================
// Multiply edge cases
// ===========================================================================

// ---------------------------------------------------------------------------
// test_multiply_with_debt_token_initial_payment
// An initial payment in the debt token must enlarge the swap input without
// enlarging the stored debt leg.
// ---------------------------------------------------------------------------

#[test]
fn test_multiply_with_debt_token_initial_payment() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let alice = t.get_or_create_user(ALICE);
    let usdc = t.resolve_asset("USDC");
    let eth = t.resolve_asset("ETH");
    let eth_market = t.resolve_market("ETH");
    eth_market.token_admin.mint(&alice, &5_000000i128); // 0.5 ETH

    let alice_eth_before = t.token_balance(ALICE, "ETH");
    t.fund_router("USDC", 4_500.0);
    let steps = build_swap_steps(&t, "ETH", "USDC", 4500_0000000);

    let account_id = t.ctrl_client().multiply(
        &alice,
        &0u64,
        &0u32,
        &usdc,
        &1_0000000i128,
        &eth,
        &common::types::PositionMode::Multiply,
        &steps,
        &Some((eth.clone(), 5_000000i128)),
        &None,
    );

    let supply = t.supply_balance_for(ALICE, account_id, "USDC");
    let borrow = t.borrow_balance_for(ALICE, account_id, "ETH");

    assert!(
        (4499.0..=4501.0).contains(&supply),
        "USDC supply should include flash debt plus initial debt-token payment, got {}",
        supply
    );
    assert!(
        (0.99..=1.01).contains(&borrow),
        "borrowed ETH should remain the strategy debt amount only, got {}",
        borrow
    );
    // The 0.5 ETH initial payment must come out of Alice's wallet; the
    // controller must not mint or otherwise replace it.
    let alice_eth_after = t.token_balance(ALICE, "ETH");
    assert!(
        (alice_eth_before - alice_eth_after - 0.5).abs() < 1e-6,
        "Alice's ETH wallet should drop by exactly 0.5 ETH, before={}, after={}",
        alice_eth_before,
        alice_eth_after
    );
}

// ---------------------------------------------------------------------------
// test_multiply_rejects_when_paused
// ---------------------------------------------------------------------------

#[test]
fn test_multiply_rejects_when_paused() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.pause();

    let steps = build_swap_steps(&t, "ETH", "USDC", 1000_0000000);
    let result = t.try_multiply(
        ALICE,
        "USDC",
        1.0,
        "ETH",
        common::types::PositionMode::Multiply,
        &steps,
    );
    assert_contract_error(result, errors::CONTRACT_PAUSED);
}

// ---------------------------------------------------------------------------
// test_multiply_borrow_cap_would_exceed
// The borrow-cap check runs after pool.create_strategy(). The borrow cap is
// set extremely low ($0.001), so multiply rejects after the borrow exceeds
// the cap.
// ---------------------------------------------------------------------------

#[test]
fn test_multiply_borrow_cap_would_exceed() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market_config("ETH", |c| {
            // Set borrow cap extremely low: 1 unit (0.0000001 ETH).
            c.borrow_cap = 1;
        })
        .build();

    // Attempt to multiply with 1 ETH debt, exceeding the borrow cap. Flow:
    // create_strategy -> check borrow cap -> reject with a specific code.
    let steps = build_swap_steps(&t, "ETH", "USDC", 5000_0000000);
    let result = t.try_multiply(
        ALICE,
        "USDC",
        1.0,
        "ETH",
        common::types::PositionMode::Multiply,
        &steps,
    );
    assert_contract_error(result, errors::BORROW_CAP_REACHED);
}

// NOTE: `test_multiply_supply_cap_would_exceed` used to live here. It did not
// fund the mock router, so the mock's own `transfer` failed before the cap
// check fired. The strict, deterministic replacement is
// `test_multiply_rejects_supply_cap_after_deposit` (below), which funds the
// router and asserts SUPPLY_CAP_REACHED.

// ---------------------------------------------------------------------------
// test_multiply_preserves_existing_collateral_balance
// Reusing an account that already holds the collateral asset must add to the
// existing position, not replace it.
// ---------------------------------------------------------------------------

#[test]
fn test_multiply_preserves_existing_collateral_balance() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let account_id = t.create_account_full(ALICE, 0, common::types::PositionMode::Multiply, false);
    t.supply_to(ALICE, account_id, "USDC", 1_000.0);

    t.fund_router("USDC", 3_000.0);
    let steps = build_swap_steps(&t, "ETH", "USDC", 30_000_000_000);

    let caller = t.get_or_create_user(ALICE);
    let ctrl = t.ctrl_client();
    let usdc = t.resolve_asset("USDC");
    let eth = t.resolve_asset("ETH");
    let result = ctrl.try_multiply(
        &caller,
        &account_id,
        &0u32,
        &usdc,
        &1_0000000i128,
        &eth,
        &common::types::PositionMode::Multiply,
        &steps,
        &None,
        &None,
    );
    assert!(matches!(result, Ok(Ok(_))), "multiply should succeed");

    let final_supply = t.supply_balance_for(ALICE, account_id, "USDC");
    assert!(
        final_supply > 3_500.0,
        "existing collateral must be preserved and increased, got {}",
        final_supply
    );

    // The multiply must also open the new ETH borrow leg; without this the
    // test would silently pass if the borrow side regressed to a no-op.
    let final_borrow = t.borrow_balance_for(ALICE, account_id, "ETH");
    assert!(
        (0.99..=1.01).contains(&final_borrow),
        "new ETH borrow leg should be ~1.0 ETH, got {}",
        final_borrow
    );
    let hf = t.health_factor_for(ALICE, account_id);
    assert!(
        hf >= 1.0,
        "post-multiply HF must remain solvent, got {}",
        hf
    );
}

// ---------------------------------------------------------------------------
// test_multiply_emode_wrong_category_debt
// E-mode account in the stablecoin category, but debt is ETH (not in
// category). Validation runs before the swap, so the error is clean.
// ---------------------------------------------------------------------------

#[test]
fn test_multiply_emode_wrong_category_debt() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_market(eth_preset())
        .with_emode(1, STABLECOIN_EMODE)
        .with_emode_asset(1, "USDC", true, true)
        .with_emode_asset(1, "USDT", true, true)
        // ETH is NOT in e-mode category 1
        .build();

    // Use the raw controller client so `e_mode_category=1` can be passed
    // explicitly.
    let caller = t.get_or_create_user(ALICE);
    let collateral_addr = t.resolve_asset("USDC");
    let debt_addr = t.resolve_asset("ETH");
    let steps = build_swap_steps(&t, "ETH", "USDC", 1000_0000000);

    let ctrl = t.ctrl_client();
    let result = ctrl.try_multiply(
        &caller,
        &0u64, // account_id = 0 (create new)
        &1u32, // e_mode_category = 1
        &collateral_addr,
        &10_0000000i128,                        // 1 ETH worth of debt
        &debt_addr,                             // ETH -- not in e-mode category 1
        &common::types::PositionMode::Multiply, // mode = 1 (multiply)
        &steps,
        &None, // initial_payment
        &None, // convert_steps
    );

    // ETH is not in e-mode category 1. `token_e_mode_config` surfaces
    // `EModeCategoryNotFound` (300) when the asset is unregistered.
    assert_contract_error(flatten(result), errors::EMODE_CATEGORY_NOT_FOUND);
}

// ---------------------------------------------------------------------------
// test_multiply_emode_wrong_category_collateral
// E-mode account in the stablecoin category, but collateral is ETH (not in
// category).
// ---------------------------------------------------------------------------

#[test]
fn test_multiply_emode_wrong_category_collateral() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_market(eth_preset())
        .with_emode(1, STABLECOIN_EMODE)
        .with_emode_asset(1, "USDC", true, true)
        .with_emode_asset(1, "USDT", true, true)
        .build();

    let caller = t.get_or_create_user(ALICE);
    let collateral_addr = t.resolve_asset("ETH"); // not in e-mode category
    let debt_addr = t.resolve_asset("USDC"); // in e-mode category
                                             // Fund the mock router so the swap itself succeeds; this lets the emode
                                             // check on the deposit leg fire (otherwise the router fails first).
    t.fund_router("ETH", 5.0);
    let steps = build_swap_steps(&t, "USDC", "ETH", 5_0000000);

    let ctrl = t.ctrl_client();
    let result = ctrl.try_multiply(
        &caller,
        &0u64,            // account_id = 0 (create new)
        &1u32,            // e_mode_category = 1
        &collateral_addr, // ETH: not in e-mode category
        &1000_0000000i128,
        &debt_addr,
        &common::types::PositionMode::Multiply,
        &steps,
        &None, // initial_payment
        &None, // convert_steps
    );

    // ETH is not in the e-mode category, so `token_e_mode_config` rejects
    // with EMODE_CATEGORY_NOT_FOUND (300).
    assert_contract_error(flatten(result), errors::EMODE_CATEGORY_NOT_FOUND);
}

// ---------------------------------------------------------------------------
// test_multiply_isolated_debt_not_enabled
// New isolated collateral via multiply must still enforce the debt asset's
// isolation_borrow_enabled flag.
// ---------------------------------------------------------------------------

#[test]
fn test_multiply_isolated_debt_not_enabled() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market_config("USDC", |c| {
            c.is_isolated_asset = true;
            c.isolation_debt_ceiling_usd_wad = usd(1_000_000);
        })
        // ETH has isolation_borrow_enabled = false (default)
        .build();

    t.fund_router("USDC", 3000.0); // Pre-fund the router with output tokens.
    let steps = build_swap_steps(&t, "ETH", "USDC", 3000_0000000);
    let result = t.try_multiply(
        ALICE,
        "USDC",
        1.0,
        "ETH",
        common::types::PositionMode::Multiply,
        &steps,
    );
    assert_contract_error(result, errors::NOT_BORROWABLE_ISOLATION);
}

// ---------------------------------------------------------------------------
// test_multiply_rejects_isolated_collateral_on_existing_non_isolated_account
// An existing non-isolated account must not add an isolated collateral leg.
// ---------------------------------------------------------------------------

#[test]
fn test_multiply_rejects_isolated_collateral_on_existing_non_isolated_account() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .with_market_config("USDC", |c| {
            c.is_isolated_asset = true;
            c.isolation_debt_ceiling_usd_wad = usd(1_000_000);
        })
        .with_market_config("ETH", |c| {
            c.isolation_borrow_enabled = true;
        })
        .build();

    let account_id = t.create_account_full(ALICE, 0, common::types::PositionMode::Multiply, false);
    t.supply_to(ALICE, account_id, "WBTC", 0.1);

    t.fund_router("USDC", 3000.0);
    let steps = build_swap_steps(&t, "ETH", "USDC", 3000_0000000);

    let caller = t.get_or_create_user(ALICE);
    let ctrl = t.ctrl_client();
    let usdc = t.resolve_asset("USDC");
    let eth = t.resolve_asset("ETH");

    let result = ctrl.try_multiply(
        &caller,
        &account_id,
        &0u32,
        &usdc,
        &1_0000000i128,
        &eth,
        &common::types::PositionMode::Multiply,
        &steps,
        &None,
        &None,
    );

    // The account's `is_isolated` flag is false but the requested collateral
    // would force isolation: reject with MIX_ISOLATED_COLLATERAL (303).
    assert_contract_error(flatten(result), errors::MIX_ISOLATED_COLLATERAL);
}

// ---------------------------------------------------------------------------
// test_multiply_siloed_debt_conflict
// The debt asset is siloed, but `multiply` creates a fresh account with no
// existing borrows. The siloed-borrow restriction therefore lives in the
// `swap_debt` tests instead.
// ---------------------------------------------------------------------------

#[test]
fn test_multiply_rejects_normal_mode() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let steps = build_swap_steps(&t, "ETH", "USDC", 1000_0000000);
    // PositionMode::Normal is reserved for non-strategy accounts; multiply
    // requires Multiply, Long, or Short.
    let result = t.try_multiply(
        ALICE,
        "USDC",
        1.0,
        "ETH",
        common::types::PositionMode::Normal,
        &steps,
    );
    assert_contract_error(result, errors::INVALID_POSITION_MODE);
}

// ---------------------------------------------------------------------------
// test_multiply_rejects_new_collateral_when_supply_limit_reached
// An existing account at the supply-position limit cannot open a new
// collateral leg through multiply.
// ---------------------------------------------------------------------------

#[test]
fn test_multiply_rejects_new_collateral_when_supply_limit_reached() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .with_position_limits(1, 4)
        .build();

    let account_id = t.create_account_full(ALICE, 0, common::types::PositionMode::Multiply, false);
    t.supply_to(ALICE, account_id, "WBTC", 0.1);

    t.fund_router("USDC", 3000.0);
    let steps = build_swap_steps(&t, "ETH", "USDC", 3000_0000000);

    let caller = t.get_or_create_user(ALICE);
    let ctrl = t.ctrl_client();
    let usdc = t.resolve_asset("USDC");
    let eth = t.resolve_asset("ETH");

    let result = ctrl.try_multiply(
        &caller,
        &account_id,
        &0u32,
        &usdc,
        &1_0000000i128,
        &eth,
        &common::types::PositionMode::Multiply,
        &steps,
        &None,
        &None,
    );

    assert_contract_error(flatten(result), errors::POSITION_LIMIT_EXCEEDED);
}

// ---------------------------------------------------------------------------
// test_multiply_existing_account_wrong_owner
// Reusing another user's account must fail before the strategy borrow path.
// ---------------------------------------------------------------------------

#[test]
fn test_multiply_existing_account_wrong_owner() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let account_id = t.create_account_full(ALICE, 0, common::types::PositionMode::Multiply, false);
    let bob = t.get_or_create_user(BOB);
    let usdc = t.resolve_asset("USDC");
    let eth = t.resolve_asset("ETH");

    t.fund_router("USDC", 3_000.0);
    let steps = build_swap_steps(&t, "ETH", "USDC", 3000_0000000);

    let result = t.ctrl_client().try_multiply(
        &bob,
        &account_id,
        &0u32,
        &usdc,
        &1_0000000i128,
        &eth,
        &common::types::PositionMode::Multiply,
        &steps,
        &None,
        &None,
    );

    // Bob calls multiply targeting Alice's existing account. The ownership
    // check must fail with AccountNotInMarket, not as a generic auth failure.
    assert_contract_error(flatten(result), errors::ACCOUNT_NOT_IN_MARKET);
}

// ---------------------------------------------------------------------------
// test_multiply_rejects_supply_cap_after_deposit
// The post-deposit supply cap check in multiply must reject oversized output.
// ---------------------------------------------------------------------------

#[test]
fn test_multiply_rejects_supply_cap_after_deposit() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market_config("USDC", |c| {
            c.supply_cap = 1; // extremely low: 1 unit (0.0000001 USDC).
        })
        .build();

    t.fund_router("USDC", 100.0);
    let steps = build_swap_steps(&t, "ETH", "USDC", 100_0000000);

    let result = t.try_multiply(
        ALICE,
        "USDC",
        0.05,
        "ETH",
        common::types::PositionMode::Multiply,
        &steps,
    );
    assert_contract_error(result, errors::SUPPLY_CAP_REACHED);
}

// ---------------------------------------------------------------------------
// test_swap_debt_refund_only_uses_strategy_excess
// Favorable slippage refunds must not sweep unrelated controller balances.
// ---------------------------------------------------------------------------

#[test]
fn test_swap_debt_refund_only_uses_strategy_excess() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 0.5);

    let eth_market = t.resolve_market("ETH");
    let eth_client = token::Client::new(&t.env, &eth_market.asset);
    eth_market
        .token_admin
        .mint(&t.controller_address(), &50_0000000i128);

    t.fund_router("ETH", 1.0);
    let steps = build_swap_steps(&t, "WBTC", "ETH", 1_0000000);

    let alice_eth_before = t.token_balance(ALICE, "ETH");
    t.swap_debt(ALICE, "ETH", 0.005, "WBTC", &steps);
    let alice_eth_after = t.token_balance(ALICE, "ETH");
    let controller_eth_after = eth_client.balance(&t.controller_address());

    assert!(
        (alice_eth_after - alice_eth_before - 0.5).abs() < 0.01,
        "caller should only receive the 0.5 ETH overpayment, before={}, after={}",
        alice_eth_before,
        alice_eth_after
    );
    assert_eq!(
        controller_eth_after, 50_0000000i128,
        "unrelated controller ETH balance must not be swept to the caller"
    );
}

// ---------------------------------------------------------------------------
// test_swap_debt_health_factor_guard_after_swap
// Mutate stored collateral params in test-only setup so the final HF guard is
// stricter than the borrow-side LTV check.
// ---------------------------------------------------------------------------

#[test]
fn test_swap_debt_health_factor_guard_after_swap() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .build();

    let usdc = t.resolve_asset("USDC");
    t.env.as_contract(&t.controller_address(), || {
        let mut market: MarketConfig = t
            .env
            .storage()
            .persistent()
            .get(&ControllerKey::Market(usdc.clone()))
            .expect("USDC market should exist");
        market.asset_config.loan_to_value_bps = 9000;
        market.asset_config.liquidation_threshold_bps = 5000;
        t.env
            .storage()
            .persistent()
            .set(&ControllerKey::Market(usdc.clone()), &market);
    });

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 5.0);

    t.fund_router("ETH", 5.0);
    let steps = build_swap_steps(&t, "WBTC", "ETH", 5_0000000);
    let result = t.try_swap_debt(ALICE, "ETH", 1.0, "WBTC", &steps);

    assert_contract_error(result, errors::INSUFFICIENT_COLLATERAL);
}

// ===========================================================================
// Swap debt edge cases
// ===========================================================================

// ---------------------------------------------------------------------------
// test_swap_debt_rejects_when_paused
// ---------------------------------------------------------------------------

#[test]
fn test_swap_debt_rejects_when_paused() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    t.pause();

    let steps = build_swap_steps(&t, "WBTC", "ETH", 1_0000000);
    let result = t.try_swap_debt(ALICE, "ETH", 1.0, "WBTC", &steps);
    assert_contract_error(result, errors::CONTRACT_PAUSED);
}

// ---------------------------------------------------------------------------
// test_swap_debt_rejects_during_flash_loan
// ---------------------------------------------------------------------------

#[test]
fn test_swap_debt_rejects_during_flash_loan() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    t.set_flash_loan_ongoing(true);

    let steps = build_swap_steps(&t, "WBTC", "ETH", 1_0000000);
    let result = t.try_swap_debt(ALICE, "ETH", 1.0, "WBTC", &steps);
    assert_contract_error(result, errors::FLASH_LOAN_ONGOING);
}

// ---------------------------------------------------------------------------
// test_swap_collateral_applies_emode_params_to_destination_position
// The destination collateral leg must inherit the account's active eMode
// parameters, not the market's base parameters.
// ---------------------------------------------------------------------------

#[test]
fn test_swap_collateral_applies_emode_params_to_destination_position() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_emode(1, STABLECOIN_EMODE)
        .with_emode_asset(1, "USDC", true, true)
        .with_emode_asset(1, "USDT", true, true)
        .build();

    let account_id = t.create_emode_account(ALICE, 1);
    t.supply_to(ALICE, account_id, "USDC", 5_000.0);

    t.fund_router("USDT", 1_000.0);
    let steps = build_swap_steps(&t, "USDC", "USDT", 10_000_000_000);
    t.swap_collateral(ALICE, "USDC", 1_000.0, "USDT", &steps);

    let (ltv, threshold) = supply_position_params(&t, account_id, "USDT");
    assert_eq!(ltv, 9700, "destination collateral should use eMode LTV");
    assert_eq!(
        threshold, 9800,
        "destination collateral should use eMode liquidation threshold"
    );
}

// ---------------------------------------------------------------------------
// test_swap_debt_non_borrowable_new_debt
// New debt asset is_borrowable=false: must reject before the swap.
// ---------------------------------------------------------------------------

#[test]
fn test_swap_debt_non_borrowable_new_debt() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .with_market_config("WBTC", |c| {
            c.is_borrowable = false;
        })
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    let steps = build_swap_steps(&t, "WBTC", "ETH", 1_0000000);
    let result = t.try_swap_debt(ALICE, "ETH", 1.0, "WBTC", &steps);
    assert_contract_error(result, errors::ASSET_NOT_BORROWABLE);
}

// ---------------------------------------------------------------------------
// test_swap_debt_siloed_conflict
// The new debt is siloed, but existing borrows include a different token.
// ---------------------------------------------------------------------------

#[test]
fn test_swap_debt_siloed_conflict() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .with_market_config("WBTC", |c| {
            c.is_siloed_borrowing = true;
        })
        .build();

    // Start with an ETH borrow.
    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    // `swap_debt` borrows WBTC before repaying ETH, so the temporary state
    // contains both a siloed borrow and an existing ETH borrow.
    let steps = build_swap_steps(&t, "WBTC", "ETH", 1_0000000);
    let result = t.try_swap_debt(ALICE, "ETH", 1.0, "WBTC", &steps);
    assert_contract_error(result, errors::NOT_BORROWABLE_SILOED);
}

// ---------------------------------------------------------------------------
// test_swap_debt_existing_siloed_borrow_blocks_new
// The account has an existing siloed borrow; swapping another debt blocks.
// ---------------------------------------------------------------------------

#[test]
fn test_swap_debt_existing_siloed_borrow_blocks_new() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .with_market_config("ETH", |c| {
            c.is_siloed_borrowing = true;
        })
        .build();

    // Supply, then borrow siloed ETH.
    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    // `swap_debt` holds the old and new debt positions at the same time. If
    // either side is siloed, the "single borrow only" invariant is violated
    // and the swap must fail.
    t.fund_router("ETH", 1.0);
    let steps = build_swap_steps(&t, "WBTC", "ETH", 1_0000000);
    let result = t.try_swap_debt(ALICE, "ETH", 1.0, "WBTC", &steps);
    // The existing ETH borrow is siloed, so the strategy must reject any new
    // debt.
    assert_contract_error(result, errors::NOT_BORROWABLE_SILOED);
}

// ---------------------------------------------------------------------------
// test_swap_debt_isolated_not_borrowable
// An isolated account swaps to a debt asset without
// isolation_borrow_enabled.
// ---------------------------------------------------------------------------

#[test]
fn test_swap_debt_isolated_not_borrowable() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .with_market_config("USDC", |c| {
            c.is_isolated_asset = true;
            c.isolation_debt_ceiling_usd_wad = usd(1_000_000);
        })
        .with_market_config("ETH", |c| {
            c.isolation_borrow_enabled = true;
        })
        // WBTC does NOT have isolation_borrow_enabled (default false)
        .build();

    // Create an isolated account, supply isolated collateral, and borrow ETH
    // (which is enabled).
    t.create_isolated_account(ALICE, "USDC");
    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    // Try to swap ETH debt to WBTC debt: WBTC is not borrowable in
    // isolation.
    let steps = build_swap_steps(&t, "WBTC", "ETH", 1_00000000);
    let result = t.try_swap_debt(ALICE, "ETH", 1.0, "WBTC", &steps);
    assert_contract_error(result, errors::NOT_BORROWABLE_ISOLATION);
}

// ---------------------------------------------------------------------------
// test_swap_debt_borrow_cap_new_debt
// The new debt asset has a borrow cap that would be exceeded. The cap check
// runs after pool.borrow(), which runs before the swap.
// ---------------------------------------------------------------------------

#[test]
fn test_swap_debt_borrow_cap_new_debt() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .with_market_config("WBTC", |c| {
            // Set a very low borrow cap: 1 unit (0.0000001 WBTC).
            c.borrow_cap = 1;
        })
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    // Swap ETH debt to WBTC: the WBTC borrow cap is tiny.
    let steps = build_swap_steps(&t, "WBTC", "ETH", 1_00000000);
    let result = t.try_swap_debt(ALICE, "ETH", 1.0, "WBTC", &steps);
    assert_contract_error(result, errors::BORROW_CAP_REACHED);
}

// ---------------------------------------------------------------------------
// test_swap_debt_emode_wrong_category
// E-mode account; the new debt asset is not in the e-mode category.
// ---------------------------------------------------------------------------

#[test]
fn test_swap_debt_emode_wrong_category() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_market(eth_preset())
        .with_emode(1, STABLECOIN_EMODE)
        .with_emode_asset(1, "USDC", true, true)
        .with_emode_asset(1, "USDT", true, true)
        // ETH not in e-mode
        .build();

    // Create an e-mode account, supply USDC, borrow USDT (both in e-mode).
    t.create_emode_account(ALICE, 1);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "USDT", 5_000.0);

    // Try to swap USDT debt to ETH: ETH is not in the e-mode category.
    let steps = build_swap_steps(&t, "ETH", "USDT", 5000_0000000);
    let result = t.try_swap_debt(ALICE, "USDT", 5_000.0, "ETH", &steps);
    assert_contract_error(result, errors::EMODE_CATEGORY_NOT_FOUND);
}

// ===========================================================================
// Swap collateral edge cases
// ===========================================================================

// ---------------------------------------------------------------------------
// test_swap_collateral_rejects_when_paused
// ---------------------------------------------------------------------------

#[test]
fn test_swap_collateral_rejects_when_paused() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    t.pause();

    let steps = build_swap_steps(&t, "USDC", "ETH", 5_0000000);
    let result = t.try_swap_collateral(ALICE, "USDC", 1000.0, "ETH", &steps);
    assert_contract_error(result, errors::CONTRACT_PAUSED);
}

// ---------------------------------------------------------------------------
// test_swap_collateral_rejects_during_flash_loan
// ---------------------------------------------------------------------------

#[test]
fn test_swap_collateral_rejects_during_flash_loan() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    t.set_flash_loan_ongoing(true);

    let steps = build_swap_steps(&t, "USDC", "ETH", 5_0000000);
    let result = t.try_swap_collateral(ALICE, "USDC", 1000.0, "ETH", &steps);
    assert_contract_error(result, errors::FLASH_LOAN_ONGOING);
}

// ---------------------------------------------------------------------------
// test_swap_collateral_non_collateralizable
// New collateral is_collateralizable=false.
// ---------------------------------------------------------------------------

#[test]
fn test_swap_collateral_non_collateralizable() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .with_market_config("WBTC", |c| {
            c.is_collateralizable = false;
        })
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    // Try to swap USDC collateral to non-collateralizable WBTC
    let steps = build_swap_steps(&t, "USDC", "WBTC", 1_00000000);
    let result = t.try_swap_collateral(ALICE, "USDC", 1000.0, "WBTC", &steps);
    assert_contract_error(result, errors::NOT_COLLATERAL);
}

// ---------------------------------------------------------------------------
// test_swap_collateral_to_isolated_asset
// The new collateral is an isolated asset: swap_collateral blocks this.
// ---------------------------------------------------------------------------

#[test]
fn test_swap_collateral_to_isolated_asset() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .with_market_config("WBTC", |c| {
            c.is_isolated_asset = true;
            c.isolation_debt_ceiling_usd_wad = usd(1_000_000);
        })
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    // Try swapping to an isolated asset: must be blocked.
    let steps = build_swap_steps(&t, "USDC", "WBTC", 1_00000000);
    let result = t.try_swap_collateral(ALICE, "USDC", 1000.0, "WBTC", &steps);
    assert_contract_error(result, errors::MIX_ISOLATED_COLLATERAL);
}

// NOTE: `test_swap_collateral_supply_cap` used to live here. It did not fund
// the mock router, so the output-side `transfer` could fail before the cap
// check. Strict replacement:
// `test_swap_collateral_rejects_supply_cap_after_deposit`.

// ---------------------------------------------------------------------------
// test_swap_collateral_rejects_supply_cap_after_deposit
// Fund the router so the flow reaches the post-deposit cap check.
// ---------------------------------------------------------------------------

#[test]
fn test_swap_collateral_rejects_supply_cap_after_deposit() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .with_market_config("WBTC", |c| {
            c.supply_cap = 1; // extremely low: 1 unit (0.0000001 WBTC).
        })
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    t.fund_router_raw("WBTC", 1_00000000i128);
    let steps = build_swap_steps(&t, "USDC", "WBTC", 1_00000000);
    let result = t.try_swap_collateral(ALICE, "USDC", 1_000.0, "WBTC", &steps);

    assert_contract_error(result, errors::SUPPLY_CAP_REACHED);
}

// ---------------------------------------------------------------------------
// test_repay_debt_with_collateral_same_token
// The same-asset flow is intentionally supported (self-collateralized
// unwinds): withdrawn collateral repays same-asset debt directly and skips
// the router. This exercises the direct-payment short-circuit.
// ---------------------------------------------------------------------------

#[test]
fn test_repay_debt_with_collateral_same_token_nets_positions() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Alice: USDC collateral + USDC debt (self-collateralized position).
    // Needs a second asset to open the position because LTV < 100%.
    t.supply(ALICE, "USDC", 100_000.0);
    t.supply(ALICE, "ETH", 20.0); // extra collateral so USDC debt is borrowable
    t.borrow(ALICE, "USDC", 30_000.0);

    let debt_before = t.borrow_balance(ALICE, "USDC");
    let supply_before = t.supply_balance(ALICE, "USDC");
    assert!(debt_before > 29_000.0 && debt_before < 31_000.0);

    // Net 10k USDC collateral against 10k USDC debt in one call. `steps` is
    // unused in the same-asset path, but the API still requires a value.
    let steps = t.mock_swap_steps("USDC", "USDC", 0);
    t.repay_debt_with_collateral(ALICE, "USDC", 10_000.0, "USDC", &steps, false);

    let debt_after = t.borrow_balance(ALICE, "USDC");
    let supply_after = t.supply_balance(ALICE, "USDC");

    // Debt reduces by ~10k, collateral reduces by ~10k. Allow 1% tolerance
    // for accrued interest and rounding across the withdraw and repay chain.
    let debt_delta = debt_before - debt_after;
    let supply_delta = supply_before - supply_after;
    assert!(
        (debt_delta - 10_000.0).abs() < 100.0,
        "USDC debt should drop ~10k, actually dropped {debt_delta}"
    );
    assert!(
        (supply_delta - 10_000.0).abs() < 100.0,
        "USDC supply should drop ~10k, actually dropped {supply_delta}"
    );
}

// ---------------------------------------------------------------------------
// test_repay_debt_with_collateral_refund_only_uses_repay_excess
// Favorable repay slippage must refund only the per-call excess.
// ---------------------------------------------------------------------------

#[test]
fn test_repay_debt_with_collateral_refund_only_uses_repay_excess() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 0.5);

    let eth_market = t.resolve_market("ETH");
    let eth_client = token::Client::new(&t.env, &eth_market.asset);
    eth_market
        .token_admin
        .mint(&t.controller_address(), &50_0000000i128);

    t.fund_router("ETH", 1.0);
    let steps = build_swap_steps(&t, "USDC", "ETH", 1_0000000);

    let alice_eth_before = t.token_balance(ALICE, "ETH");
    t.repay_debt_with_collateral(ALICE, "USDC", 1_000.0, "ETH", &steps, false);
    let alice_eth_after = t.token_balance(ALICE, "ETH");
    let controller_eth_after = eth_client.balance(&t.controller_address());

    assert!(
        (alice_eth_after - alice_eth_before - 0.5).abs() < 0.01,
        "caller should only receive the 0.5 ETH repayment excess, before={}, after={}",
        alice_eth_before,
        alice_eth_after
    );
    assert_eq!(
        controller_eth_after, 50_0000000i128,
        "unrelated controller ETH balance must not be swept during repay refund"
    );
}

// ---------------------------------------------------------------------------
// test_repay_debt_with_collateral_health_factor_guard
// Withdrawing too much collateral for too little debt repayment must fail the
// final HF check.
// ---------------------------------------------------------------------------

#[test]
fn test_repay_debt_with_collateral_health_factor_guard() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 30.0);

    t.fund_router("ETH", 1.0);
    let steps = build_swap_steps(&t, "USDC", "ETH", 1_0000000);
    let result = t.try_repay_debt_with_collateral(ALICE, "USDC", 50_000.0, "ETH", &steps, false);

    assert_contract_error(result, errors::INSUFFICIENT_COLLATERAL);
}

// ---------------------------------------------------------------------------
// test_repay_debt_with_collateral_close_position_removes_account
// A full close must repay the debt, drain remaining collateral, and remove
// the account.
// ---------------------------------------------------------------------------

#[test]
fn test_repay_debt_with_collateral_close_position_removes_account() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);
    let account_id = t.resolve_account_id(ALICE);

    let alice_usdc_before = t.token_balance(ALICE, "USDC");
    t.fund_router("ETH", 1.0);
    let steps = build_swap_steps(&t, "USDC", "ETH", 1_0000000);
    t.repay_debt_with_collateral(ALICE, "USDC", 1_000.0, "ETH", &steps, true);

    assert!(
        !t.account_exists(account_id),
        "close_position should remove the fully closed account"
    );
    // Close-position semantics: residual collateral must be returned to the
    // caller's wallet, not swept inside the controller. A regression that
    // dropped the refund would leave usdc_after < usdc_before.
    let alice_usdc_after = t.token_balance(ALICE, "USDC");
    assert!(
        alice_usdc_after >= alice_usdc_before,
        "close_position must refund residual USDC collateral to Alice, before={}, after={}",
        alice_usdc_before,
        alice_usdc_after
    );
}

// ---------------------------------------------------------------------------
// test_repay_debt_with_collateral_removes_empty_account_without_close
// Even without close_position=true, the account must be removed when the
// flow zeroes every remaining position.
// ---------------------------------------------------------------------------

#[test]
fn test_repay_debt_with_collateral_removes_empty_account_without_close() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 2_000.0);
    t.borrow(ALICE, "ETH", 0.5);
    let account_id = t.resolve_account_id(ALICE);

    t.fund_router("ETH", 0.5);
    let steps = build_swap_steps(&t, "USDC", "ETH", 5_000000);
    t.repay_debt_with_collateral(ALICE, "USDC", 2_000.0, "ETH", &steps, false);

    assert!(
        !t.account_exists(account_id),
        "repay-with-collateral should remove the account when both sides reach zero"
    );
}

// ---------------------------------------------------------------------------
// test_swap_collateral_rejects_new_asset_when_supply_limit_reached
// Partial swap into a new asset should respect the supply-position limit.
// ---------------------------------------------------------------------------

#[test]
fn test_swap_collateral_rejects_new_asset_when_supply_limit_reached() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .with_market(usdt_stable_preset())
        .with_market(dai_preset())
        .with_position_limits(4, 10)
        .build();

    let account_id = t.create_account(ALICE);
    t.supply_to(ALICE, account_id, "USDC", 10_000.0);
    t.supply_to(ALICE, account_id, "ETH", 1.0);
    t.supply_to(ALICE, account_id, "WBTC", 0.1);
    t.supply_to(ALICE, account_id, "USDT", 5_000.0);

    t.fund_router("DAI", 1.0);
    let steps = build_swap_steps(&t, "USDC", "DAI", 1_0000000);
    let result = t.try_swap_collateral(ALICE, "USDC", 100.0, "DAI", &steps);
    assert_contract_error(result, errors::POSITION_LIMIT_EXCEEDED);
}

// ---------------------------------------------------------------------------
// test_swap_collateral_emode_wrong_category
// E-mode account; new collateral is not in the e-mode category.
// ---------------------------------------------------------------------------

#[test]
fn test_swap_collateral_emode_wrong_category() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_market(eth_preset())
        .with_emode(1, STABLECOIN_EMODE)
        .with_emode_asset(1, "USDC", true, true)
        .with_emode_asset(1, "USDT", true, true)
        .build();

    // Create an e-mode account, supply USDC, borrow USDT.
    t.create_emode_account(ALICE, 1);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "USDT", 5_000.0);

    // Try to swap USDC collateral to ETH: ETH is not in e-mode. The error
    // may be ASSET_NOT_IN_EMODE or EMODE_CATEGORY_NOT_FOUND, depending on
    // how the validation resolves the e-mode asset lookup.
    let steps = build_swap_steps(&t, "USDC", "ETH", 5_0000000);
    let result = t.try_swap_collateral(ALICE, "USDC", 1000.0, "ETH", &steps);
    assert_contract_error(result, errors::EMODE_CATEGORY_NOT_FOUND);
}

// ---------------------------------------------------------------------------
// test_swap_collateral_no_borrows_skip_hf
// Swap collateral with no borrows: the HF check is skipped. With the
// working mock router, this succeeds.
// ---------------------------------------------------------------------------

#[test]
fn test_swap_collateral_no_borrows_skip_hf() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Supply only, no borrows.
    t.supply(ALICE, "USDC", 100_000.0);

    // Swap collateral: the HF check is skipped (no borrows). With the
    // working mock router, this succeeds.
    t.fund_router("ETH", 5.0); // Pre-fund the router with output tokens.
    let steps = build_swap_steps(&t, "USDC", "ETH", 5_0000000);
    let result = t.try_swap_collateral(ALICE, "USDC", 1000.0, "ETH", &steps);
    assert!(
        result.is_ok(),
        "swap_collateral with no borrows should succeed"
    );

    // Verify the ETH supply position was created.
    let eth_supply = t.supply_balance(ALICE, "ETH");
    assert!(
        eth_supply > 0.0,
        "should have ETH supply: got {}",
        eth_supply
    );
    // The 1000 USDC of source collateral must be removed from the supply
    // side; otherwise the swap leg silently regressed to "deposit only".
    let usdc_supply_after = t.supply_balance(ALICE, "USDC");
    assert!(
        (98_999.0..=99_001.0).contains(&usdc_supply_after),
        "USDC supply should drop by ~1000 after swap_collateral, got {}",
        usdc_supply_after
    );
}

// ===========================================================================
// Attack vectors
// ===========================================================================

// ---------------------------------------------------------------------------
// test_strategy_empty_swap_steps
// An empty hops vec underflows: swap_tokens reads steps.hops.len() - 1.
// This must panic and crash gracefully.
// ---------------------------------------------------------------------------

#[test]
fn test_strategy_empty_swap_steps_multiply() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Build empty swap steps.
    let empty_steps = SwapSteps {
        amount_out_min: 0,
        distribution: soroban_sdk::Vec::new(&t.env),
    };

    // Must fail: empty hops cause underflow in swap_tokens
    // (steps.hops.len() - 1 on an empty vec). The error happens after
    // create_strategy, inside swap_tokens.
    let result = t.try_multiply(
        ALICE,
        "USDC",
        1.0,
        "ETH",
        common::types::PositionMode::Multiply,
        &empty_steps,
    );
    // M-10: the controller rejects `amount_out_min <= 0` at the multiply
    // entry point with AmountMustBePositive. Before the M-10 fix, this test
    // relied on a deeper chain: empty distribution -> zero swap output ->
    // AmountMustBePositive at the deposit path. The outcome is unchanged;
    // the check now fails fast.
    assert_contract_error(result, errors::AMOUNT_MUST_BE_POSITIVE);
}

// ---------------------------------------------------------------------------
// test_multiply_zero_debt_amount
// ---------------------------------------------------------------------------

#[test]
fn test_multiply_zero_debt_amount() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let steps = build_swap_steps(&t, "ETH", "USDC", 1000_0000000);
    let result = t.try_multiply(
        ALICE,
        "USDC",
        0.0,
        "ETH",
        common::types::PositionMode::Multiply,
        &steps,
    );
    assert_contract_error(result, errors::AMOUNT_MUST_BE_POSITIVE);
}

// ---------------------------------------------------------------------------
// test_swap_debt_zero_amount
// ---------------------------------------------------------------------------

#[test]
fn test_swap_debt_zero_amount() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    let steps = build_swap_steps(&t, "WBTC", "ETH", 1_00000000);
    // Note: try_swap_debt passes new_amount through f64_to_i128, so 0.0 -> 0.
    // The validation require_amount_positive must catch this.
    let result = t.try_swap_debt(ALICE, "ETH", 0.0, "WBTC", &steps);
    assert_contract_error(result, errors::AMOUNT_MUST_BE_POSITIVE);
}

// ---------------------------------------------------------------------------
// test_swap_collateral_zero_amount
// ---------------------------------------------------------------------------

#[test]
fn test_swap_collateral_zero_amount() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    let steps = build_swap_steps(&t, "USDC", "ETH", 5_0000000);
    let result = t.try_swap_collateral(ALICE, "USDC", 0.0, "ETH", &steps);
    assert_contract_error(result, errors::AMOUNT_MUST_BE_POSITIVE);
}

// ---------------------------------------------------------------------------
// test_swap_debt_wrong_account_owner
// Bob tries to swap Alice's debt: must be rejected.
// ---------------------------------------------------------------------------

#[test]
fn test_swap_debt_wrong_account_owner() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    // Get Alice's account ID, then try to swap using Bob's address.
    let alice_account_id = t.resolve_account_id(ALICE);
    let bob_addr = t.get_or_create_user(BOB);
    let existing_addr = t.resolve_asset("ETH");
    let new_addr = t.resolve_asset("WBTC");
    let steps = build_swap_steps(&t, "WBTC", "ETH", 1_00000000);

    let ctrl = t.ctrl_client();
    let result = ctrl.try_swap_debt(
        &bob_addr,
        &alice_account_id,
        &existing_addr,
        &10_0000000i128,
        &new_addr,
        &steps,
    );
    // Flatten Result<Result<(), Error>, InvokeError> so the code can assert.
    let flat: Result<(), soroban_sdk::Error> = match result {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(e.into()),
        Err(invoke) => Err(invoke.expect("expected contract error, got host-level InvokeError")),
    };
    assert_contract_error(flat, errors::ACCOUNT_NOT_IN_MARKET);
}

// ---------------------------------------------------------------------------
// test_swap_collateral_wrong_account_owner
// Bob tries to swap Alice's collateral: must be rejected.
// ---------------------------------------------------------------------------

#[test]
fn test_swap_collateral_wrong_account_owner() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    let alice_account_id = t.resolve_account_id(ALICE);
    let bob_addr = t.get_or_create_user(BOB);
    let current_addr = t.resolve_asset("USDC");
    let new_addr = t.resolve_asset("ETH");
    let steps = build_swap_steps(&t, "USDC", "ETH", 5_0000000);

    let ctrl = t.ctrl_client();
    let result = ctrl.try_swap_collateral(
        &bob_addr,
        &alice_account_id,
        &current_addr,
        &1000_0000000i128,
        &new_addr,
        &steps,
    );
    let flat: Result<(), soroban_sdk::Error> = match result {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(e.into()),
        Err(invoke) => Err(invoke.expect("expected contract error, got host-level InvokeError")),
    };
    assert_contract_error(flat, errors::ACCOUNT_NOT_IN_MARKET);
}

// ---------------------------------------------------------------------------
// test_multiply_same_asset_different_direction
// Verify that collateral == debt is caught even when the amounts differ.
// ---------------------------------------------------------------------------

#[test]
fn test_multiply_same_asset_is_caught() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let steps = build_swap_steps(&t, "ETH", "ETH", 1000_0000000);
    let result = t.try_multiply(
        ALICE,
        "ETH",
        1.0,
        "ETH",
        common::types::PositionMode::Multiply,
        &steps,
    );
    assert_contract_error(result, errors::ASSETS_ARE_THE_SAME);
}

// ---------------------------------------------------------------------------
// test_swap_debt_same_token
// Already tested in strategy_tests.rs; verify the error code here too.
// ---------------------------------------------------------------------------

#[test]
fn test_swap_debt_same_token_error_code() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    let steps = t.mock_swap_steps("ETH", "ETH", 0);
    let result = t.try_swap_debt(ALICE, "ETH", 1.0, "ETH", &steps);
    assert_contract_error(result, errors::ASSETS_ARE_THE_SAME);
}

// ---------------------------------------------------------------------------
// test_swap_collateral_same_token_error_code
// ---------------------------------------------------------------------------

#[test]
fn test_swap_collateral_same_token_error_code() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    let steps = t.mock_swap_steps("USDC", "USDC", 0);
    let result = t.try_swap_collateral(ALICE, "USDC", 1000.0, "USDC", &steps);
    assert_contract_error(result, errors::ASSETS_ARE_THE_SAME);
}
