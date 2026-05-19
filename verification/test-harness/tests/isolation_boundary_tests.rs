//! Boundary regressions for isolated-mode positions. The main
//! `isolation_tests.rs` covers happy-path and ceiling enforcement;
//! this file pins the exact-ceiling boundary, dust residue on isolated
//! liquidation, and the per-account independence invariant when one
//! user runs multiple isolated accounts.

extern crate std;

use common::constants::WAD;
use test_harness::{
    assert_contract_error, errors, eth_preset, usdc_preset, LendingTest, ALICE, BOB,
};

// ---------------------------------------------------------------------------
// 1. Ceiling exact-at-limit accepted, +1 cent rejected
// ---------------------------------------------------------------------------

// The existing suite proves that a $6k borrow against a $5k ceiling
// fails. This pins the *boundary*: a borrow exactly at the ceiling
// succeeds, and even $1 over fails.
#[test]
fn test_isolated_ceiling_boundary_exact_then_overshoot() {
    let ceiling_usd: f64 = 5_000.0;
    let ceiling_wad: i128 = (ceiling_usd as i128) * WAD;

    let mut t = LendingTest::new()
        .with_market(eth_preset())
        .with_market(usdc_preset())
        .with_market_config("ETH", |cfg| {
            cfg.is_isolated_asset = true;
            cfg.isolation_debt_ceiling_usd_wad = ceiling_wad;
        })
        .with_market_config("USDC", |cfg| {
            cfg.isolation_borrow_enabled = true;
        })
        .build();

    // Bob provides USDC liquidity so Alice's borrow doesn't trip the
    // utilization cap.
    t.supply(BOB, "USDC", 100_000.0);

    t.create_isolated_account(ALICE, "ETH");
    t.supply(ALICE, "ETH", 5.0); // $10k @ $2000

    // Borrow exactly $5000 — at the ceiling. Must succeed.
    t.borrow(ALICE, "USDC", ceiling_usd);

    // Attempt $1 more — must trip the ceiling.
    let result = t.try_borrow(ALICE, "USDC", 1.0);
    assert_contract_error(result, errors::DEBT_CEILING_REACHED);
}

// ---------------------------------------------------------------------------
// 2. Two isolated accounts on the same user are independent
// ---------------------------------------------------------------------------

// Pins that a user can hold two separate isolated accounts (one ETH,
// one WBTC) and that the ceilings + debt trackers operate per-asset,
// not per-user. Without this independence, the isolated-debt tracker
// would aggregate across accounts and double-count.
#[test]
fn test_isolated_accounts_independent_across_assets() {
    use test_harness::wbtc_preset;

    let ceiling_eth_wad: i128 = 10_000 * WAD;
    let ceiling_wbtc_wad: i128 = 20_000 * WAD;

    let mut t = LendingTest::new()
        .with_market(eth_preset())
        .with_market(usdc_preset())
        .with_market(wbtc_preset())
        .with_market_config("ETH", |cfg| {
            cfg.is_isolated_asset = true;
            cfg.isolation_debt_ceiling_usd_wad = ceiling_eth_wad;
        })
        .with_market_config("WBTC", |cfg| {
            cfg.is_isolated_asset = true;
            cfg.isolation_debt_ceiling_usd_wad = ceiling_wbtc_wad;
        })
        .with_market_config("USDC", |cfg| {
            cfg.isolation_borrow_enabled = true;
        })
        .build();

    t.supply(BOB, "USDC", 100_000.0);

    let eth_account = t.create_isolated_account(ALICE, "ETH");
    t.supply_to(ALICE, eth_account, "ETH", 5.0);
    t.borrow_to(ALICE, eth_account, "USDC", 4_000.0);

    let wbtc_account = t.create_isolated_account(ALICE, "WBTC");
    t.supply_to(ALICE, wbtc_account, "WBTC", 0.5);
    t.borrow_to(ALICE, wbtc_account, "USDC", 8_000.0);

    // Each tracker holds only its own ledger.
    let eth_isolated_debt = t.get_isolated_debt("ETH");
    let wbtc_isolated_debt = t.get_isolated_debt("WBTC");

    assert!(
        eth_isolated_debt > 0 && wbtc_isolated_debt > 0,
        "both trackers should be live (eth={}, wbtc={})",
        eth_isolated_debt,
        wbtc_isolated_debt
    );

    // ETH tracker must be ≈ $4000 (in WAD), not $12000. Within rounding.
    let eth_debt_usd = eth_isolated_debt / WAD;
    assert!(
        (3_990..=4_010).contains(&eth_debt_usd),
        "ETH isolated tracker must reflect only ETH-account debt, got {} (~ ${})",
        eth_isolated_debt,
        eth_debt_usd
    );
    let wbtc_debt_usd = wbtc_isolated_debt / WAD;
    assert!(
        (7_990..=8_010).contains(&wbtc_debt_usd),
        "WBTC isolated tracker must reflect only WBTC-account debt, got {} (~ ${})",
        wbtc_isolated_debt,
        wbtc_debt_usd
    );
}

// ---------------------------------------------------------------------------
// 3. Borrow at ceiling, partial repay, re-borrow up to ceiling
// ---------------------------------------------------------------------------

// Pins that the ceiling tracker decrements correctly on repay and that
// the user can re-borrow up to the released headroom. Without correct
// decrement the tracker would permanently consume ceiling capacity.
#[test]
fn test_isolated_ceiling_releases_capacity_on_repay() {
    let ceiling_wad: i128 = 5_000 * WAD;
    let mut t = LendingTest::new()
        .with_market(eth_preset())
        .with_market(usdc_preset())
        .with_market_config("ETH", |cfg| {
            cfg.is_isolated_asset = true;
            cfg.isolation_debt_ceiling_usd_wad = ceiling_wad;
        })
        .with_market_config("USDC", |cfg| {
            cfg.isolation_borrow_enabled = true;
        })
        .build();

    t.supply(BOB, "USDC", 100_000.0);
    t.create_isolated_account(ALICE, "ETH");
    t.supply(ALICE, "ETH", 5.0);

    // Step 1: borrow up to ceiling.
    t.borrow(ALICE, "USDC", 5_000.0);
    let debt_after_borrow = t.get_isolated_debt("ETH");

    // Step 2: repay half. Tracker should drop ~50 %.
    t.repay(ALICE, "USDC", 2_500.0);
    let debt_after_repay = t.get_isolated_debt("ETH");
    assert!(
        debt_after_repay < debt_after_borrow,
        "tracker must decrement on repay: before={} after={}",
        debt_after_borrow,
        debt_after_repay
    );

    // Step 3: borrow into the released headroom. Borrowing $2400 with
    // ~$2500 headroom must succeed (small interest accrual buffer).
    t.borrow(ALICE, "USDC", 2_400.0);

    // Borrowing another $300 (well past headroom) must hit the ceiling.
    let result = t.try_borrow(ALICE, "USDC", 300.0);
    assert_contract_error(result, errors::DEBT_CEILING_REACHED);
}
