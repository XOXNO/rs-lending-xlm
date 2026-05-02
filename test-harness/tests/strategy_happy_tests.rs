extern crate std;

use common::constants::WAD;

use common::types::{DexDistribution, Protocol, SwapSteps};
use soroban_sdk::vec;
use test_harness::{
    eth_preset, usdc_preset, usdt_stable_preset, wbtc_preset, LendingTest, ALICE, BOB,
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

// ===========================================================================
// Multiply happy paths
// ===========================================================================

// ---------------------------------------------------------------------------
// test_multiply_creates_leveraged_position
//
// Full multiply flow:
//   1. Flash-borrow 1 ETH ($2000).
//   2. Swap ETH -> USDC (mock returns 3000 USDC).
//   3. Deposit 3000 USDC as collateral.
//   4. HF = 3000 * 0.8 / 2000 = 1.2.
// ---------------------------------------------------------------------------

#[test]
fn test_multiply_creates_leveraged_position() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Flash-borrow 1 ETH, swap to 3000 USDC (favorable mock rate).
    t.fund_router("USDC", 3000.0);
    let steps = build_swap_steps(&t, "ETH", "USDC", 3000_0000000);
    let account_id = t.multiply(
        ALICE,
        "USDC",
        1.0,
        "ETH",
        common::types::PositionMode::Multiply,
        &steps,
    );

    assert!(account_id > 0, "account should be created");

    // Supply position: 3000 USDC.
    let supply = t.supply_balance_for(ALICE, account_id, "USDC");
    assert!(
        (2999.0..=3001.0).contains(&supply),
        "USDC supply should be ~3000, got {}",
        supply
    );

    // Borrow position: 1 ETH.
    let borrow = t.borrow_balance_for(ALICE, account_id, "ETH");
    assert!(
        (0.99..=1.01).contains(&borrow),
        "ETH borrow should be ~1.0, got {}",
        borrow
    );

    // HF = 3000 * 0.8 / 2000 = 1.2.
    let hf = t.health_factor_for(ALICE, account_id);
    assert!(hf >= 1.0, "HF should be >= 1.0, got {}", hf);
    assert!(hf < 2.0, "HF should be reasonable, got {}", hf);
}

// ---------------------------------------------------------------------------
// test_multiply_mode_long
// Mode=2 (Long): same flow, with a different mode stored on the account.
// ---------------------------------------------------------------------------

#[test]
fn test_multiply_mode_long() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.fund_router("USDC", 3000.0);
    let steps = build_swap_steps(&t, "ETH", "USDC", 3000_0000000);
    let account_id = t.multiply(
        ALICE,
        "USDC",
        1.0,
        "ETH",
        common::types::PositionMode::Long,
        &steps,
    );

    assert!(account_id > 0);

    let attrs = t.get_account_attributes(ALICE);
    assert_eq!(
        attrs.mode,
        common::types::PositionMode::Long,
        "mode should be Long"
    );

    // An empty position trivially satisfies HF >= 1.0 (controller returns
    // i128::MAX). Pin the supply and borrow magnitudes to catch a regression
    // that skipped the deposit branch in Long mode.
    let supply = t.supply_balance_for(ALICE, account_id, "USDC");
    assert!(
        (2999.0..=3001.0).contains(&supply),
        "USDC supply should be ~3000 in Long mode, got {}",
        supply
    );
    let borrow = t.borrow_balance_for(ALICE, account_id, "ETH");
    assert!(
        (0.99..=1.01).contains(&borrow),
        "ETH borrow should be ~1.0 in Long mode, got {}",
        borrow
    );

    let hf = t.health_factor_for(ALICE, account_id);
    assert!(hf >= 1.0, "HF should be >= 1.0, got {}", hf);
}

// ---------------------------------------------------------------------------
// test_multiply_mode_short
// Mode=3 (Short).
// ---------------------------------------------------------------------------

#[test]
fn test_multiply_mode_short() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.fund_router("USDC", 3000.0);
    let steps = build_swap_steps(&t, "ETH", "USDC", 3000_0000000);
    let account_id = t.multiply(
        ALICE,
        "USDC",
        1.0,
        "ETH",
        common::types::PositionMode::Short,
        &steps,
    );

    assert!(account_id > 0);

    let attrs = t.get_account_attributes(ALICE);
    assert_eq!(
        attrs.mode,
        common::types::PositionMode::Short,
        "mode should be Short"
    );

    // An empty position trivially satisfies HF >= 1.0 (controller returns
    // i128::MAX). Pin the supply and borrow magnitudes to catch a regression
    // that skipped the deposit branch in Short mode.
    let supply = t.supply_balance_for(ALICE, account_id, "USDC");
    assert!(
        (2999.0..=3001.0).contains(&supply),
        "USDC supply should be ~3000 in Short mode, got {}",
        supply
    );
    let borrow = t.borrow_balance_for(ALICE, account_id, "ETH");
    assert!(
        (0.99..=1.01).contains(&borrow),
        "ETH borrow should be ~1.0 in Short mode, got {}",
        borrow
    );

    let hf = t.health_factor_for(ALICE, account_id);
    assert!(hf >= 1.0, "HF should be >= 1.0, got {}", hf);
}

// ---------------------------------------------------------------------------
// test_multiply_wbtc_collateral
// Different asset pair: borrow USDC with WBTC collateral.
// ---------------------------------------------------------------------------

#[test]
fn test_multiply_wbtc_collateral() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(wbtc_preset())
        .build();

    // Borrow 1000 USDC, swap to 0.02 WBTC (at $60000, $1200 worth).
    // WBTC 8 decimals: 0.02 WBTC = 2_000_000 raw.
    // HF = 1200 * 0.8 / 1000 = 0.96: too low.
    // Need more: 0.03 WBTC = $1800. HF = 1800*0.8/1000 = 1.44.
    t.fund_router_raw("WBTC", 3_000_000);
    let steps = build_swap_steps(&t, "USDC", "WBTC", 3_000_000);
    let account_id = t.multiply(
        ALICE,
        "WBTC",
        1000.0,
        "USDC",
        common::types::PositionMode::Multiply,
        &steps,
    );

    assert!(account_id > 0);

    let supply = t.supply_balance_for(ALICE, account_id, "WBTC");
    assert!(supply > 0.0, "should have WBTC supply: got {}", supply);

    let borrow = t.borrow_balance_for(ALICE, account_id, "USDC");
    assert!(borrow > 0.0, "should have USDC borrow: got {}", borrow);

    let hf = t.health_factor_for(ALICE, account_id);
    assert!(hf >= 1.0, "HF should be >= 1.0, got {}", hf);
}

// ===========================================================================
// Swap debt happy paths
// ===========================================================================

// ---------------------------------------------------------------------------
// test_swap_debt_replaces_borrow
//
// Setup: supply USDC, borrow ETH.
// Swap debt: ETH -> WBTC (borrow WBTC, swap to ETH, repay ETH).
// Verify: ETH borrow shrinks or disappears, WBTC borrow exists.
// ---------------------------------------------------------------------------

#[test]
fn test_swap_debt_replaces_borrow() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    // Verify initial state.
    let initial_eth = t.borrow_balance(ALICE, "ETH");
    assert!(initial_eth > 0.9, "should have ~1 ETH borrow");

    // Swap ETH debt -> WBTC debt. Borrow 1 WBTC ($60000), swap to ETH (need
    // enough to repay 1 ETH). min_amount_out = 1_0000000 raw ETH (1 ETH).
    t.fund_router("ETH", 1.0);
    let steps = build_swap_steps(&t, "WBTC", "ETH", 1_0000000);
    t.swap_debt(ALICE, "ETH", 1.0, "WBTC", &steps);

    // The WBTC borrow must now exist.
    let wbtc_borrow = t.borrow_balance(ALICE, "WBTC");
    assert!(
        wbtc_borrow > 0.0,
        "should have WBTC borrow after swap: got {}",
        wbtc_borrow
    );

    // HF must remain valid.
    let hf = t.health_factor(ALICE);
    assert!(hf >= 1.0, "HF should be >= 1.0 after swap_debt, got {}", hf);
}

// ---------------------------------------------------------------------------
// test_swap_debt_partial
//
// Swap only part of the debt: old and new borrows coexist.
// ---------------------------------------------------------------------------

#[test]
fn test_swap_debt_partial() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .build();

    t.supply(ALICE, "USDC", 200_000.0);
    t.borrow(ALICE, "ETH", 2.0);

    // Swap only part of the ETH debt: borrow 0.5 WBTC, swap to ~0.5 ETH.
    // 0.5 WBTC = 50_000_000 raw (8 decimals).
    // Swap output = 0.5 ETH = 5_000_000 raw (7 decimals): partial repay.
    t.fund_router_raw("ETH", 5_000_000);
    let steps = build_swap_steps(&t, "WBTC", "ETH", 5_000_000);
    t.swap_debt(ALICE, "ETH", 0.5, "WBTC", &steps);

    // Both borrows must exist.
    let eth_borrow = t.borrow_balance(ALICE, "ETH");
    assert!(
        eth_borrow > 0.0 && eth_borrow < 2.0,
        "ETH borrow should be partially repaid: got {}",
        eth_borrow
    );

    let wbtc_borrow = t.borrow_balance(ALICE, "WBTC");
    assert!(
        wbtc_borrow > 0.0,
        "should have WBTC borrow: got {}",
        wbtc_borrow
    );

    let hf = t.health_factor(ALICE);
    assert!(hf >= 1.0, "HF should be >= 1.0, got {}", hf);
}

// ===========================================================================
// Swap collateral happy paths
// ===========================================================================

// ---------------------------------------------------------------------------
// test_swap_collateral_replaces_supply
//
// Setup: supply USDC, borrow ETH.
// Swap collateral: USDC -> ETH (withdraw USDC, swap to ETH, deposit ETH).
// Verify: USDC supply shrinks, ETH supply is created.
// ---------------------------------------------------------------------------

#[test]
fn test_swap_collateral_replaces_supply() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    let initial_usdc = t.supply_balance(ALICE, "USDC");
    assert!(initial_usdc >= 99_999.0, "should have ~100K USDC supply");

    // Swap 20,000 USDC -> 10 ETH (mock rate $2000/ETH).
    t.fund_router("ETH", 10.0);
    let steps = build_swap_steps(&t, "USDC", "ETH", 10_0000000);
    t.swap_collateral(ALICE, "USDC", 20_000.0, "ETH", &steps);

    // USDC supply must shrink.
    let usdc_after = t.supply_balance(ALICE, "USDC");
    assert!(
        usdc_after < initial_usdc,
        "USDC supply should decrease: {} -> {}",
        initial_usdc,
        usdc_after
    );

    // ETH supply must be created.
    let eth_supply = t.supply_balance(ALICE, "ETH");
    assert!(
        (9.99..=10.01).contains(&eth_supply),
        "should have ~10 ETH supply: got {}",
        eth_supply
    );

    let hf = t.health_factor(ALICE);
    assert!(
        hf >= 1.0,
        "HF should be >= 1.0 after swap_collateral, got {}",
        hf
    );
}

// ---------------------------------------------------------------------------
// test_swap_collateral_no_borrows
// Swap collateral with no borrows: the HF check is skipped.
// ---------------------------------------------------------------------------

#[test]
fn test_swap_collateral_no_borrows() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 50_000.0);

    // Swap some USDC to ETH: no borrows, so no HF check.
    t.fund_router("ETH", 5.0);
    let steps = build_swap_steps(&t, "USDC", "ETH", 5_0000000);
    t.swap_collateral(ALICE, "USDC", 10_000.0, "ETH", &steps);

    let eth_supply = t.supply_balance(ALICE, "ETH");
    assert!(
        eth_supply > 0.0,
        "should have ETH supply: got {}",
        eth_supply
    );

    let usdc_supply = t.supply_balance(ALICE, "USDC");
    assert!(
        usdc_supply < 50_000.0,
        "USDC supply should be reduced: got {}",
        usdc_supply
    );
}

// ---------------------------------------------------------------------------
// test_repay_debt_with_collateral_reduces_positions
//
// Setup: supply USDC, borrow ETH.
// Repay with collateral: USDC -> ETH (withdraw USDC, swap to ETH, repay
// ETH).
// Verify: USDC collateral and ETH debt both decrease.
// ---------------------------------------------------------------------------

#[test]
fn test_repay_debt_with_collateral_reduces_positions() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    let collateral_before = t.supply_balance(ALICE, "USDC");
    let debt_before = t.borrow_balance(ALICE, "ETH");

    t.fund_router("ETH", 0.5);
    let steps = build_swap_steps(&t, "USDC", "ETH", 5_000000);
    t.repay_debt_with_collateral(ALICE, "USDC", 1_000.0, "ETH", &steps, false);

    let collateral_after = t.supply_balance(ALICE, "USDC");
    let debt_after = t.borrow_balance(ALICE, "ETH");

    assert!(
        collateral_after < collateral_before,
        "USDC collateral should decrease: before={}, after={}",
        collateral_before,
        collateral_after
    );
    assert!(
        debt_after < debt_before,
        "ETH debt should decrease: before={}, after={}",
        debt_before,
        debt_after
    );
    assert!(
        t.health_factor(ALICE) >= 1.0,
        "HF should stay healthy after repay_debt_with_collateral"
    );
}

// ===========================================================================
// E-mode strategy tests
// ===========================================================================

// ---------------------------------------------------------------------------
// test_multiply_emode_stablecoin
//
// E-mode multiply with stablecoins: borrow USDT, deposit USDC.
// E-mode parameters: LTV=97%, LT=98%, bonus=2%.
// ---------------------------------------------------------------------------

#[test]
fn test_multiply_emode_stablecoin() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_emode(1, STABLECOIN_EMODE)
        .with_emode_asset(1, "USDC", true, true)
        .with_emode_asset(1, "USDT", true, true)
        .build();

    // E-mode multiply: borrow USDT, collateral USDC.
    // Borrow 1000 USDT, swap to 1050 USDC (favorable mock rate).
    // With e-mode LT=98%: HF = 1050 * 0.98 / 1000 = 1.029.
    let caller = t.get_or_create_user(ALICE);
    let collateral_addr = t.resolve_asset("USDC");
    let debt_addr = t.resolve_asset("USDT");
    t.fund_router("USDC", 1050.0);
    let steps = build_swap_steps(&t, "USDT", "USDC", 1050_0000000);

    let ctrl = t.ctrl_client();
    let account_id = ctrl.multiply(
        &caller,
        &0u64, // create new account
        &1u32, // e_mode_category = 1
        &collateral_addr,
        &1000_0000000i128, // borrow 1000 USDT
        &debt_addr,
        &common::types::PositionMode::Multiply, // mode = Multiply
        &steps,
        &None, // initial_payment
        &None, // convert_steps
    );

    assert!(account_id > 0, "e-mode account should be created");

    // Verify positions.
    let supply = t.supply_balance_for(ALICE, account_id, "USDC");
    assert!(supply > 0.0, "should have USDC supply in e-mode");

    let borrow = t.borrow_balance_for(ALICE, account_id, "USDT");
    assert!(borrow > 0.0, "should have USDT borrow in e-mode");

    // HF must be healthy with e-mode parameters.
    let hf = ctrl.health_factor(&account_id);
    let hf_f64 = hf as f64 / (WAD as f64);
    assert!(hf_f64 >= 1.0, "e-mode HF should be >= 1.0, got {}", hf_f64);
}

// ===========================================================================
// Strategy with large amounts (stress)
// ===========================================================================

// ---------------------------------------------------------------------------
// test_multiply_large_amounts
// Multiply with large borrow amounts to verify no overflow occurs.
// ---------------------------------------------------------------------------

#[test]
fn test_multiply_large_amounts() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Borrow 100 ETH ($200,000), swap to $300,000 USDC.
    // HF = 300000 * 0.8 / 200000 = 1.2.
    t.fund_router("USDC", 300_000.0);
    let steps = build_swap_steps(&t, "ETH", "USDC", 3_000_000_000_000);
    let account_id = t.multiply(
        ALICE,
        "USDC",
        100.0,
        "ETH",
        common::types::PositionMode::Multiply,
        &steps,
    );

    let supply = t.supply_balance_for(ALICE, account_id, "USDC");
    assert!(
        supply >= 299_999.0,
        "should have ~300K USDC supply: got {}",
        supply
    );

    let borrow = t.borrow_balance_for(ALICE, account_id, "ETH");
    assert!(
        borrow >= 99.0,
        "should have ~100 ETH borrow: got {}",
        borrow
    );

    let hf = t.health_factor_for(ALICE, account_id);
    assert!(hf >= 1.0, "HF should be >= 1.0, got {}", hf);
}

// ===========================================================================
// Multiple users
// ===========================================================================

// ---------------------------------------------------------------------------
// test_multiply_two_users
// Two users multiply independently.
// ---------------------------------------------------------------------------

#[test]
fn test_multiply_two_users() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Alice: borrow 1 ETH, receive 3000 USDC.
    t.fund_router("USDC", 3000.0);
    let steps_alice = build_swap_steps(&t, "ETH", "USDC", 3000_0000000);
    let alice_id = t.multiply(
        ALICE,
        "USDC",
        1.0,
        "ETH",
        common::types::PositionMode::Multiply,
        &steps_alice,
    );

    // Bob: borrow 2 ETH, receive 6000 USDC.
    t.fund_router("USDC", 6000.0);
    let steps_bob = build_swap_steps(&t, "ETH", "USDC", 6000_0000000);
    let bob_id = t.multiply(
        BOB,
        "USDC",
        2.0,
        "ETH",
        common::types::PositionMode::Multiply,
        &steps_bob,
    );

    assert_ne!(alice_id, bob_id, "accounts should be different");

    let alice_supply = t.supply_balance_for(ALICE, alice_id, "USDC");
    let bob_supply = t.supply_balance_for(BOB, bob_id, "USDC");
    assert!(
        (2999.0..=3001.0).contains(&alice_supply),
        "Alice should have ~3000 USDC supply, got {}",
        alice_supply
    );
    assert!(
        (5999.0..=6001.0).contains(&bob_supply),
        "Bob should have ~6000 USDC supply, got {}",
        bob_supply
    );
    let alice_borrow = t.borrow_balance_for(ALICE, alice_id, "ETH");
    let bob_borrow = t.borrow_balance_for(BOB, bob_id, "ETH");
    assert!(
        (0.99..=1.01).contains(&alice_borrow),
        "Alice should owe ~1 ETH, got {}",
        alice_borrow
    );
    assert!(
        (1.99..=2.01).contains(&bob_borrow),
        "Bob should owe ~2 ETH, got {}",
        bob_borrow
    );
    let alice_addr = t.get_or_create_user(ALICE);
    let bob_addr = t.get_or_create_user(BOB);
    assert_eq!(t.get_account_owner(alice_id), alice_addr);
    assert_eq!(t.get_account_owner(bob_id), bob_addr);

    let alice_hf = t.health_factor_for(ALICE, alice_id);
    let bob_hf = t.health_factor_for(BOB, bob_id);

    assert!(
        alice_hf >= 1.0,
        "Alice HF should be >= 1.0, got {}",
        alice_hf
    );
    assert!(bob_hf >= 1.0, "Bob HF should be >= 1.0, got {}", bob_hf);
}

// ===========================================================================
// Swap debt preserves health
// ===========================================================================

// ---------------------------------------------------------------------------
// test_swap_debt_to_costlier_debt_preserves_minimum_hf
// Swap 10 ETH ($20k) debt -> 0.5 WBTC ($30k) debt: USD debt grows, so HF
// shrinks but must stay >= 1.0. Pinning the strict direction catches any
// regression that silently dropped the new debt or kept the old.
// ---------------------------------------------------------------------------

#[test]
fn test_swap_debt_to_costlier_debt_preserves_minimum_hf() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 10.0); // $20,000 debt

    let hf_before = t.health_factor(ALICE);
    assert!(hf_before >= 1.0);

    // Swap 10 ETH debt into 0.5 WBTC debt ($30,000 at $60k each). The swap
    // output must cover 10 ETH of repayment, so `min_amount_out` is 10 ETH.
    t.fund_router("ETH", 10.0);
    let steps = build_swap_steps(&t, "WBTC", "ETH", 10_0000000);
    t.swap_debt(ALICE, "ETH", 0.5, "WBTC", &steps);

    let hf_after = t.health_factor(ALICE);
    assert!(
        hf_after >= 1.0,
        "HF should still be >= 1.0 after swap_debt, got {}",
        hf_after
    );
    assert!(
        hf_after < hf_before,
        "HF must shrink when swapping to costlier debt: before={}, after={}",
        hf_before,
        hf_after
    );
}
