use common::types::{AssetConfig, SwapSteps};
use soroban_sdk::Vec;
use test_harness::{assert_contract_error, eth_preset, tokens, usd, usdc_preset, LendingTest, BOB};

#[test]
fn test_strategy_swap_collateral_supply_cap_reached() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Bob supplies 1M USDC to fill the pool
    t.supply(BOB, "USDC", 1_000_000.0);

    // Set supply cap on USDC to 1,010,000 tokens (7 decimals)
    // Current total = 1,000,000
    t.ctrl_client().edit_asset_config(
        &t.resolve_asset("USDC"),
        &AssetConfig {
            supply_cap: 1_010_000_0000000,
            ..usdc_preset().config.to_asset_config()
        },
    );

    // Alice supplies some ETH
    t.supply("alice", "ETH", 10.0);

    // Alice tries to swap 5 ETH collateral for USDC.
    // 5 ETH = $10,000.
    // We mock the swap to return 20,000 USDC ($20,000 at $1/USDC).
    // Total USDC = 1,000,000 + 20,000 = 1,020,000.
    // 1,020,000 > 1,010,000 triggers #105.

    // Fund the router with USDC for the swap
    t.fund_router("USDC", 100_000.0);

    let steps = SwapSteps {
        amount_out_min: tokens(20_000, 7), // Return 20k USDC
        distribution: Vec::new(&t.env),
    };

    // We expect #105 (SupplyCapReached)
    let res = t.try_swap_collateral("alice", "ETH", 5.0, "USDC", &steps);
    assert_contract_error(res, 105);
}

#[test]
fn test_strategy_multiply_supply_cap_reached() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Bob supplies 1M USDC
    t.supply(BOB, "USDC", 1_000_000.0);

    // Set supply cap on USDC to 1,010,000 tokens (7 decimals)
    t.ctrl_client().edit_asset_config(
        &t.resolve_asset("USDC"),
        &AssetConfig {
            supply_cap: 1_010_000_0000000,
            ..usdc_preset().config.to_asset_config()
        },
    );

    // Alice has some USDC
    t.supply("alice", "USDC", 5.0); // Minimal initial position

    // Alice tries to Multiply her USDC position.
    // Borrow 10 ETH ($20k). Swap to USDC.
    // Mock swap returns 30,000 USDC.
    // Total USDC = 1,000,000 (Bob) + 5 (Alice) + 30,000 (Swap) = 1,030,005.
    // 1,030,005 > 1,010,000 triggers #105.

    t.fund_router("USDC", 100_000.0);

    let steps = SwapSteps {
        amount_out_min: tokens(30_000, 7), // Return 30k USDC
        distribution: Vec::new(&t.env),
    };

    // We expect #105 (SupplyCapReached)
    let res = t.try_multiply(
        "alice",
        "USDC",
        10.0,
        "ETH",
        common::types::PositionMode::Multiply, // Multiply mode
        &steps,
    );
    assert_contract_error(res, 105);
}

#[test]
fn test_strategy_multiply_unsupported_category() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply("alice", "USDC", 10.0);
    let steps = t.mock_swap_steps("ETH", "USDC", usd(2000));

    // Try Multiply with invalid category 999 using the harness helper.
    let res = t.try_multiply_with_category(
        "alice",
        999, // category
        "USDC",
        5.0,
        "ETH",
        common::types::PositionMode::Multiply, // mode
        &steps,
    );

    // We expect EMODE_CATEGORY_NOT_FOUND (300)
    assert_contract_error(res, 300);
}
