use test_harness::presets::{
    MarketPreset, ALICE, BOB, DEFAULT_ASSET_CONFIG, DEFAULT_MARKET_PARAMS, LIQUIDATOR,
};
use test_harness::{helpers::usd, LendingTest};

fn usdc_6dec() -> MarketPreset {
    MarketPreset {
        name: "USDC6",
        decimals: 6,
        price_wad: usd(1),
        initial_liquidity: 1_000_000.0,
        config: DEFAULT_ASSET_CONFIG,
        params: DEFAULT_MARKET_PARAMS,
    }
}

fn dai_18dec() -> MarketPreset {
    MarketPreset {
        name: "DAI18",
        decimals: 18,
        price_wad: usd(1),
        initial_liquidity: 1_000_000.0,
        config: DEFAULT_ASSET_CONFIG,
        params: DEFAULT_MARKET_PARAMS,
    }
}

fn sol_9dec() -> MarketPreset {
    MarketPreset {
        name: "SOL9",
        decimals: 9,
        price_wad: usd(150),
        initial_liquidity: 100_000.0,
        config: DEFAULT_ASSET_CONFIG,
        params: DEFAULT_MARKET_PARAMS,
    }
}

fn wbtc_8dec() -> MarketPreset {
    MarketPreset {
        name: "WBTC8",
        decimals: 8,
        price_wad: usd(60_000),
        initial_liquidity: 100_000.0,
        config: DEFAULT_ASSET_CONFIG,
        params: DEFAULT_MARKET_PARAMS,
    }
}

fn xlm_7dec() -> MarketPreset {
    MarketPreset {
        name: "XLM7",
        decimals: 7,
        price_wad: usd(1) / 10,
        initial_liquidity: 10_000_000.0,
        config: DEFAULT_ASSET_CONFIG,
        params: DEFAULT_MARKET_PARAMS,
    }
}

#[test]
fn test_supply_6dec_borrow_18dec() {
    let mut t = LendingTest::new()
        .with_market(usdc_6dec())
        .with_market(dai_18dec())
        .build();

    t.supply(ALICE, "USDC6", 10_000.0);
    t.assert_supply_near(ALICE, "USDC6", 10_000.0, 0.01);

    t.borrow(ALICE, "DAI18", 5_000.0);
    t.assert_borrow_near(ALICE, "DAI18", 5_000.0, 0.01);
    t.assert_healthy(ALICE);

    let hf = t.health_factor(ALICE);
    assert!(
        hf > 1.5 && hf < 1.7,
        "HF should be ~1.6 for 50% utilization at 80% LTV, got {}",
        hf
    );
}

#[test]
fn test_supply_18dec_borrow_6dec() {
    let mut t = LendingTest::new()
        .with_market(usdc_6dec())
        .with_market(dai_18dec())
        .build();

    t.supply(ALICE, "DAI18", 10_000.0);
    t.borrow(ALICE, "USDC6", 5_000.0);
    t.assert_borrow_near(ALICE, "USDC6", 5_000.0, 0.01);
    t.assert_healthy(ALICE);
}

#[test]
fn test_supply_9dec_borrow_8dec() {
    let mut t = LendingTest::new()
        .with_market(sol_9dec())
        .with_market(wbtc_8dec())
        .build();

    t.supply(ALICE, "SOL9", 100.0);
    t.assert_supply_near(ALICE, "SOL9", 100.0, 0.001);

    t.borrow(ALICE, "WBTC8", 0.1);
    t.assert_borrow_near(ALICE, "WBTC8", 0.1, 0.0001);
    t.assert_healthy(ALICE);
}

#[test]
fn test_mixed_decimal_types_single_account() {
    let mut t = LendingTest::new()
        .with_market(usdc_6dec())
        .with_market(wbtc_8dec())
        .with_market(sol_9dec())
        .with_market(dai_18dec())
        .with_position_limits(4, 4)
        .build();

    t.supply(ALICE, "USDC6", 5_000.0);
    t.supply_to(ALICE, t.resolve_account_id(ALICE), "WBTC8", 0.083);
    t.supply_to(ALICE, t.resolve_account_id(ALICE), "SOL9", 33.3);

    t.borrow(ALICE, "DAI18", 7_500.0);
    t.assert_healthy(ALICE);

    let hf = t.health_factor(ALICE);
    assert!(hf > 1.5 && hf < 1.7, "HF should be ~1.6, got {}", hf);

    let total_collateral = t.total_collateral(ALICE);
    assert!(
        total_collateral > 14_000.0 && total_collateral < 16_000.0,
        "Total collateral should be ~$15,000, got {}",
        total_collateral
    );

    let total_debt = t.total_debt(ALICE);
    assert!(
        total_debt > 7_000.0 && total_debt < 8_000.0,
        "Total debt should be ~$7,500, got {}",
        total_debt
    );
}

#[test]
fn test_tiny_amounts_18dec() {
    let mut t = LendingTest::new()
        .with_market(dai_18dec())
        .with_market(usdc_6dec())
        .with_dust_disabled_all_markets()
        .build();

    t.supply(ALICE, "DAI18", 0.000001);

    let supply = t.supply_balance(ALICE, "DAI18");
    assert!(
        supply > 0.0,
        "Supply balance should be positive even for tiny 18-dec amount, got {}",
        supply
    );
}

#[test]
fn test_large_amounts_6dec() {
    let mut t = LendingTest::new()
        .with_market(usdc_6dec())
        .with_market(dai_18dec())
        .build();

    t.supply(ALICE, "USDC6", 500_000.0);
    t.assert_supply_near(ALICE, "USDC6", 500_000.0, 1.0);

    t.borrow(ALICE, "DAI18", 200_000.0);
    t.assert_borrow_near(ALICE, "DAI18", 200_000.0, 1.0);
    t.assert_healthy(ALICE);
}

#[test]
fn test_interest_accrual_mixed_decimals() {
    let mut t = LendingTest::new()
        .with_market(usdc_6dec())
        .with_market(dai_18dec())
        .build();

    t.supply(ALICE, "USDC6", 100_000.0);
    t.borrow(ALICE, "DAI18", 20_000.0);

    let borrow_before = t.borrow_balance(ALICE, "DAI18");

    t.advance_and_sync(7 * 24 * 60 * 60);

    let borrow_after = t.borrow_balance(ALICE, "DAI18");
    assert!(
        borrow_after > borrow_before,
        "Borrow should accrue interest: before={}, after={}",
        borrow_before,
        borrow_after
    );

    let supply_after = t.supply_balance(ALICE, "USDC6");
    assert!(
        supply_after >= 100_000.0,
        "6-dec supply should hold or grow with interest: {}",
        supply_after
    );

    t.assert_healthy(ALICE);
}

#[test]
fn test_repay_cross_decimal() {
    let mut t = LendingTest::new()
        .with_market(usdc_6dec())
        .with_market(dai_18dec())
        .build();

    t.supply(ALICE, "USDC6", 10_000.0);
    t.borrow(ALICE, "DAI18", 5_000.0);

    t.repay(ALICE, "DAI18", 2_500.0);
    t.assert_borrow_near(ALICE, "DAI18", 2_500.0, 1.0);
    t.assert_healthy(ALICE);

    t.repay(ALICE, "DAI18", 3_000.0);
    let remaining = t.borrow_balance(ALICE, "DAI18");
    assert!(
        remaining < 1.0,
        "Borrow should be fully repaid (or near-zero), got {}",
        remaining
    );
}

#[test]
fn test_withdraw_cross_decimal_hf_check() {
    let mut t = LendingTest::new()
        .with_market(usdc_6dec())
        .with_market(dai_18dec())
        .build();

    t.supply(ALICE, "USDC6", 10_000.0);
    t.borrow(ALICE, "DAI18", 4_000.0);

    t.withdraw(ALICE, "USDC6", 3_000.0);
    t.assert_healthy(ALICE);
    t.assert_supply_near(ALICE, "USDC6", 7_000.0, 1.0);
}

#[test]
fn test_liquidation_6dec_collateral_18dec_debt() {
    let mut t = LendingTest::new()
        .with_market(usdc_6dec())
        .with_market(dai_18dec())
        .build();

    t.supply(ALICE, "USDC6", 10_000.0);
    t.borrow(ALICE, "DAI18", 7_500.0);

    t.set_price("USDC6", usd(1) * 90 / 100);
    t.advance_and_sync(1000);

    let hf = t.health_factor(ALICE);
    assert!(
        hf < 1.0,
        "HF should be below 1.0 after price drop, got {}",
        hf
    );

    t.liquidate(LIQUIDATOR, ALICE, "DAI18", 3_000.0);

    let debt_after = t.borrow_balance(ALICE, "DAI18");
    assert!(
        debt_after < 7_500.0,
        "Debt should be reduced after liquidation, got {}",
        debt_after
    );
}

#[test]
fn test_liquidation_18dec_collateral_6dec_debt() {
    let mut t = LendingTest::new()
        .with_market(usdc_6dec())
        .with_market(dai_18dec())
        .build();

    t.supply(ALICE, "DAI18", 10_000.0);
    t.borrow(ALICE, "USDC6", 7_500.0);

    t.set_price("DAI18", usd(1) * 90 / 100);
    t.advance_and_sync(1000);

    let hf = t.health_factor(ALICE);
    assert!(hf < 1.0, "HF should be below 1.0, got {}", hf);

    t.liquidate(LIQUIDATOR, ALICE, "USDC6", 3_000.0);

    let debt_after = t.borrow_balance(ALICE, "USDC6");
    assert!(debt_after < 7_500.0, "Debt reduced, got {}", debt_after);
}

#[test]
fn test_multi_user_mixed_decimals() {
    let mut t = LendingTest::new()
        .with_market(usdc_6dec())
        .with_market(dai_18dec())
        .with_market(sol_9dec())
        .build();

    t.supply(ALICE, "USDC6", 10_000.0);
    t.borrow(ALICE, "DAI18", 5_000.0);

    t.supply(BOB, "SOL9", 100.0);
    t.borrow(BOB, "USDC6", 5_000.0);

    t.assert_healthy(ALICE);
    t.assert_healthy(BOB);

    t.assert_supply_near(ALICE, "USDC6", 10_000.0, 1.0);
    t.assert_supply_near(BOB, "SOL9", 100.0, 0.1);
}

#[test]
fn test_low_value_high_quantity_7dec() {
    let mut t = LendingTest::new()
        .with_market(xlm_7dec())
        .with_market(wbtc_8dec())
        .build();

    t.supply(ALICE, "XLM7", 1_000_000.0);
    t.assert_supply_near(ALICE, "XLM7", 1_000_000.0, 10.0);

    t.borrow(ALICE, "WBTC8", 0.5);
    t.assert_borrow_near(ALICE, "WBTC8", 0.5, 0.001);
    t.assert_healthy(ALICE);

    let hf = t.health_factor(ALICE);
    assert!(
        hf > 2.0,
        "HF should be >2 for $100k collateral / $30k debt, got {}",
        hf
    );
}

#[test]
fn test_borrow_1_raw_unit_is_properly_recorded_on_7dec() {
    let mut t = LendingTest::new()
        .with_market(xlm_7dec())
        .with_min_borrow_collateral_disabled()
        .build();

    t.supply(ALICE, "XLM7", 100.0);

    let initial_borrow = t.borrow_balance_raw(ALICE, "XLM7");
    let initial_token = t.token_balance_raw(ALICE, "XLM7");

    t.borrow_raw(ALICE, "XLM7", 1);

    let after_borrow = t.borrow_balance_raw(ALICE, "XLM7");
    let after_token = t.token_balance_raw(ALICE, "XLM7");

    assert_eq!(
        after_borrow,
        initial_borrow + 1,
        "1 raw borrow must record exactly +1 in borrow balance"
    );

    assert_eq!(after_token, initial_token + 1);

    let account_id = t.resolve_account_id(ALICE);
    let (_supplies, borrows) = t.ctrl_client().get_account_positions(&account_id);
    let asset_addr = t.resolve_asset("XLM7");
    assert!(
        borrows
            .iter()
            .any(|(k, p)| k.asset == asset_addr && p.scaled_amount > 0),
        "Positive scaled debt position must exist after borrowing 1 raw unit"
    );

    t.assert_healthy(ALICE);
}

#[test]
fn test_scaled_borrow_never_zero_for_raw_one_within_protocol_bounds() {
    let env = soroban_sdk::Env::default();
    let one_raw = common::math::fp::Ray::from_asset(1, 18);
    let samples = [
        common::constants::RAY,
        common::constants::RAY * 1_000,
        common::constants::RAY * 1_000_000,
        common::constants::MAX_BORROW_INDEX_RAY / 2,
        common::constants::MAX_BORROW_INDEX_RAY,
    ];
    for decimals in 6u32..=18 {
        let from = common::math::fp::Ray::from_asset(1, decimals);
        for &index in &samples {
            let scaled = from.div(&env, common::math::fp::Ray::from(index));
            assert!(
                scaled.raw() > 0,
                "1-raw-unit borrow on {}dec scaled to zero at borrow_index={} \
                 (within protocol bounds — this must never happen)",
                decimals,
                index
            );
        }
    }

    let worst_case = one_raw.div(
        &env,
        common::math::fp::Ray::from(common::constants::MAX_BORROW_INDEX_RAY),
    );
    assert_eq!(
        worst_case.raw(),
        1,
        "worst-case corner (18dec, raw=1, index=MAX_BORROW_INDEX_RAY) must \
         scale to exactly 1, matching the on-chain boundary test above"
    );

    let beyond_cap = one_raw.div(
        &env,
        common::math::fp::Ray::from(common::constants::MAX_BORROW_INDEX_RAY * 3),
    );
    assert_eq!(
        beyond_cap.raw(),
        0,
        "beyond the protocol's index ceiling, a 1-raw-unit 18dec borrow does \
         round to zero — this is the free-borrow shape the pool's \
         BorrowRoundsToZeroShares guard rejects if it's ever reached"
    );
}

#[test]
fn test_borrow_1_raw_unit_18dec_saturates_hf() {
    let mut t = LendingTest::new()
        .with_market(dai_18dec())
        .with_market(usdc_6dec())
        .with_min_borrow_collateral_disabled()
        .build();

    t.supply(ALICE, "USDC6", 10_000.0);
    t.borrow_raw(ALICE, "DAI18", 1);

    assert_eq!(t.borrow_balance_raw(ALICE, "DAI18"), 1);

    let hf = t.health_factor_raw(ALICE);
    assert!(
        hf > 0 && (hf == i128::MAX || hf > 1_000_000 * common::constants::WAD),
        "1-raw dust debt against $10k collateral must yield a huge healthy HF, got {hf}"
    );
    t.assert_healthy(ALICE);
}
