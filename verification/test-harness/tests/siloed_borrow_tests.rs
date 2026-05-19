//! Siloed-borrow regressions. The existing `borrow_tests.rs` covers
//! the basic "first borrow makes other borrows impossible" rule. This
//! file pins the lifecycle:
//!
//! * Additional borrows of the *same* siloed asset are permitted.
//! * Repaying the siloed debt fully clears the restriction, opening
//!   the account to a different asset.
//! * Liquidating a siloed position preserves the invariant (the
//!   liquidator's debt-side payment must remain the same asset).

extern crate std;

use test_harness::{
    assert_contract_error, errors, eth_preset, usd, usdc_preset, wbtc_preset, LendingTest, ALICE,
    BOB, LIQUIDATOR,
};

// ---------------------------------------------------------------------------
// 1. Multiple borrows of the same siloed asset are allowed
// ---------------------------------------------------------------------------

// Pins that the "siloed = one debt" rule is per-asset, not per-borrow:
// the user can stack multiple borrow operations on the same siloed
// asset as long as they don't introduce a different debt asset.
#[test]
fn test_siloed_allows_repeated_same_asset_borrows() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market_config("ETH", |cfg| {
            cfg.is_siloed_borrowing = true;
        })
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 0.1);
    // Second ETH borrow: same siloed asset, must succeed.
    t.borrow(ALICE, "ETH", 0.2);
    // Third: also same asset.
    t.borrow(ALICE, "ETH", 0.05);

    t.assert_borrow_count(ALICE, 1); // still one debt entry (ETH)
}

// ---------------------------------------------------------------------------
// 2. After full repay, account opens to a different asset
// ---------------------------------------------------------------------------

// Pins that the siloed restriction lifts once the siloed debt is fully
// repaid. Without correct cleanup the account would be permanently
// locked to the original asset.
#[test]
fn test_siloed_full_repay_lifts_restriction() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .with_market_config("ETH", |cfg| {
            cfg.is_siloed_borrowing = true;
        })
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 0.1);
    // Fully repay the ETH debt.
    t.repay(ALICE, "ETH", 1.0); // over-pay rounded down to actual owed
    t.assert_borrow_count(ALICE, 0);

    // Now WBTC (a different asset) must be borrowable since the siloed
    // debt entry was removed.
    t.borrow(ALICE, "WBTC", 0.001);
    t.assert_borrow_count(ALICE, 1);
}

// ---------------------------------------------------------------------------
// 3. Siloed restriction holds across liquidation
// ---------------------------------------------------------------------------

// A partial liquidation of a siloed position must keep the siloed
// constraint live: the liquidator's debt-side payment must be the
// siloed asset, and the position's remaining debt remains the same
// single asset. Pins that liquidation doesn't accidentally introduce
// a second debt asset.
#[test]
fn test_siloed_invariant_preserved_under_liquidation() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market_config("ETH", |cfg| {
            cfg.is_siloed_borrowing = true;
        })
        .with_dust_disabled_all_markets()
        .build();

    // Alice supplies USDC, borrows ETH (siloed).
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0); // $6000 debt

    // Drop USDC → underwater.
    t.set_price("USDC", usd(1) * 70 / 100); // $0.70 → HF ≈ 0.93
    t.assert_liquidatable(ALICE);

    // Liquidator repays the siloed debt. Must succeed and keep the
    // invariant.
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 0.5);

    // Borrow count remains 1 (still ETH only).
    t.assert_borrow_count(ALICE, 1);

    // Bob (not in trouble) cannot now mix siloed ETH with WBTC just
    // because Alice was liquidated.
    t.supply(BOB, "USDC", 1_000.0);
    t.borrow(BOB, "ETH", 0.01);
    let bob_mixed = t.try_borrow(BOB, "USDC", 50.0);
    assert_contract_error(bob_mixed, errors::NOT_BORROWABLE_SILOED);
}
