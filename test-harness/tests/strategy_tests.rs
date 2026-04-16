extern crate std;

use common::types::{DexDistribution, Protocol, SwapSteps};
use soroban_sdk::vec;
use test_harness::{
    assert_contract_error, errors, eth_preset, usdc_preset, wbtc_preset, LendingTest, ALICE,
};

// ---------------------------------------------------------------------------
// Helper: build SwapSteps with a single hop that yields `min_amount_out`
// from the mock swap router.
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

// ---------------------------------------------------------------------------
// 1. test_multiply_rejects_non_borrowable_debt — asserts ASSET_NOT_BORROWABLE
// ---------------------------------------------------------------------------

#[test]
fn test_multiply_rejects_non_borrowable_debt() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market_config("ETH", |c| {
            c.is_borrowable = false;
        })
        .build();

    // ETH is not borrowable -- multiply must fail with a specific error
    // (ASSET_NOT_BORROWABLE). A bare is_err() would accept upstream failures
    // like pause/flash-loan guards and miss regressions.
    let steps = build_swap_steps(&t, "ETH", "USDC", 1_0000000);
    let result = t.try_multiply(
        ALICE,
        "USDC",
        1.0,
        "ETH",
        common::types::PositionMode::Multiply,
        &steps,
    );
    assert_contract_error(result, errors::ASSET_NOT_BORROWABLE);
}

// ---------------------------------------------------------------------------
// 2. test_multiply_rejects_non_collateralizable — asserts NOT_COLLATERAL
// ---------------------------------------------------------------------------

#[test]
fn test_multiply_rejects_non_collateralizable() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market_config("USDC", |c| {
            c.is_collateralizable = false;
        })
        .build();

    let steps = build_swap_steps(&t, "ETH", "USDC", 1000_0000000);
    let result = t.try_multiply(
        ALICE,
        "USDC",
        1.0,
        "ETH",
        common::types::PositionMode::Multiply,
        &steps,
    );
    assert_contract_error(result, errors::NOT_COLLATERAL);
}

// ---------------------------------------------------------------------------
// 3. test_multiply_rejects_during_flash_loan — asserts FLASH_LOAN_ONGOING
// ---------------------------------------------------------------------------

#[test]
fn test_multiply_rejects_during_flash_loan() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Set flash loan ongoing flag to simulate reentrancy
    t.set_flash_loan_ongoing(true);

    let steps = build_swap_steps(&t, "ETH", "USDC", 1000_0000000);
    let result = t.try_multiply(
        ALICE,
        "USDC",
        1.0,
        "ETH",
        common::types::PositionMode::Multiply,
        &steps,
    );
    assert_contract_error(result, errors::FLASH_LOAN_ONGOING);
}

// ---------------------------------------------------------------------------
// 4. test_swap_collateral_rejects_isolated — asserts SWAP_COLLATERAL_NO_ISO
// ---------------------------------------------------------------------------

#[test]
fn test_swap_collateral_rejects_isolated() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market_config("USDC", |c| {
            c.is_isolated_asset = true;
            c.isolation_debt_ceiling_usd_wad = 1_000_000_000_000_000_000_000_000;
            // $1M
        })
        .build();

    // Create isolated account and supply
    t.create_isolated_account(ALICE, "USDC");
    t.supply(ALICE, "USDC", 10_000.0);

    let steps = build_swap_steps(&t, "USDC", "ETH", 5_0000000);
    let result = t.try_swap_collateral(ALICE, "USDC", 1000.0, "ETH", &steps);
    assert_contract_error(result, errors::SWAP_COLLATERAL_NO_ISO);
}

// NOTE: `test_multiply_happy_path`, `test_swap_debt_happy_path`, and
// `test_swap_collateral_happy_path` were removed as redundant. They are
// fully covered (with STRONGER assertions) by the dedicated happy-path
// suite in `strategy_happy_tests.rs`:
//   - `test_multiply_creates_leveraged_position` (supply in [2999..=3001],
//     borrow in [0.99..=1.01] vs. the old `> 0.0` loose check).
//   - `test_swap_debt_replaces_borrow` (asserts `initial_eth > 0.9`, the old
//     variant only asserted `> 0.0`).
//   - `test_swap_collateral_replaces_supply` (asserts initial supply in
//     [99_999..] plus `eth_supply in [9.99..=10.01]`).
//
// Removing these strict duplicates cuts CI time without losing coverage:
// the three happy-path behaviors remain regression-tested through the
// stricter assertions elsewhere.
//
// `test_swap_collateral_rejects_same_token`, `test_multiply_rejects_zero_amount`,
// and `test_multiply_rejects_invalid_mode` also used to live here with generic
// `is_err()` asserts. They are fully covered by the strict `assert_contract_error`
// variants in `strategy_edge_tests.rs` (`test_swap_collateral_same_token_error_code`,
// `test_multiply_zero_debt_amount`, `test_multiply_rejects_mode_4`).

// ---------------------------------------------------------------------------
// 5. test_multiply_rejects_isolated_debt_ceiling_breach
// ---------------------------------------------------------------------------

#[test]
fn test_multiply_rejects_isolated_debt_ceiling_breach() {
    let shitcoin_preset = test_harness::MarketPreset {
        name: "SHITCOIN",
        decimals: 18,
        price_wad: 1_000_000_000_000_000_000,
        initial_liquidity: 10_000_000.0,
        config: test_harness::AssetConfigPreset {
            loan_to_value_bps: 8000, // 80% LTV
            liquidation_threshold_bps: 8500,
            liquidation_bonus_bps: 500,
            liquidation_fees_bps: 100,
            is_collateralizable: true,
            is_borrowable: true,
            is_flashloanable: true,
            is_isolated_asset: true, // ISOLATED ASSET!
            is_siloed_borrowing: false,
            e_mode_enabled: false,
            isolation_borrow_enabled: true,
            isolation_debt_ceiling_usd_wad: 100 * 1_000_000_000_000_000_000, // ONLY 100 USD BORROW ALLOWED!
            flashloan_fee_bps: 9,
            borrow_cap: 10_000_000_000_000_000_000_000_000, // 10M tokens (18 decimals)
            supply_cap: 10_000_000_000_000_000_000_000_000, // 10M tokens (18 decimals)
        },
        params: test_harness::MarketParamsPreset {
            mid_utilization_ray: 500_000_000_000_000_000_000_000_000, // 0.5 RAY
            optimal_utilization_ray: 800_000_000_000_000_000_000_000_000, // 0.8 RAY
            ..test_harness::DEFAULT_MARKET_PARAMS
        },
    };

    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(shitcoin_preset)
        .with_market_config("USDC", |c| {
            c.isolation_borrow_enabled = true; // USDC borrowable in isolation
        })
        .build();

    // Seed liquidity for the flash loan and the USDC borrow.
    t.supply(test_harness::KEEPER_USER, "USDC", 1_000_000.0);

    // 1. Give Alice the isolated asset
    let alice_addr = t.get_or_create_user(test_harness::ALICE);
    let shit_market = t.resolve_market("SHITCOIN");
    shit_market
        .token_admin
        .mint(&alice_addr, &(100_000 * 1_000_000_000_000_000_000_i128));

    // Provide initial payment as collateral to the multiply function.
    // Because the collateral is isolated, multiply must create an isolated
    // account and enforce the $100 ceiling on the USDC debt leg.
    // amount_out_min = 1 is a trivial positive sentinel (passes M-10 entry
    // check) — this test fails before the swap router is reached.
    let steps = build_swap_steps(&t, "USDC", "SHITCOIN", 1);

    // Call multiply directly using the raw client
    let ctrl = t.ctrl_client();
    let usdc_addr = t.resolve_asset("USDC");
    let shit_addr = t.resolve_asset("SHITCOIN");

    let result = ctrl.try_multiply(
        &alice_addr,
        &0u64,
        &0u32,
        &shit_addr,
        &50_000_000_000_i128, // 50k USDC debt to flash loan (decimals 6) -> 50,000 * 10^6
        &usdc_addr,
        &common::types::PositionMode::Multiply,
        &steps,
        &Some((shit_addr.clone(), 100_000 * 1_000_000_000_000_000_000_i128)),
        &None,
    );

    // The multiply must surface the isolated debt ceiling breach with a specific
    // error code so regressions that substitute a different error (e.g. HF guard
    // triggering first) are caught. Convert the nested Result from `try_multiply`
    // into a single Result<_, Error>.
    let flat: Result<u64, soroban_sdk::Error> = match result {
        Ok(Ok(id)) => Ok(id),
        Ok(Err(err)) => Err(err),
        Err(invoke_err) => Err(invoke_err
            .expect("expected contract error, got host-level InvokeError")),
    };
    assert_contract_error(flat, errors::DEBT_CEILING_REACHED);

    // Normal borrow is stopped correctly by the ceiling.
    t.supply(test_harness::BOB, "SHITCOIN", 10_000.0);
    let res = t.try_borrow(test_harness::BOB, "USDC", 200.0);
    assert_contract_error(res, errors::DEBT_CEILING_REACHED);
}
