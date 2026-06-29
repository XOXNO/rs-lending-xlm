//! Anti-ratchet regression tests for partial liquidations.
//!
//! A liquidation's bonus rate scales inversely with health factor
//! (`calculate_linear_bonus_with_target`), and a partial liquidation of
//! high-threshold collateral can lower HF. The concern: an attacker spams
//! small partials, each driving HF lower and so (hypothetically) earning a
//! larger bonus than a single liquidation would.
//!
//! These tests pin the invariant that this cannot happen. The engine applies
//! the HF-scaled bonus only on the recoverable branches of
//! `estimate_liquidation_amount` (where partials RAISE HF, so the bonus
//! auto-decreases on the next call) and pins the unrecoverable max-seizure
//! path to the BASE bonus. So in no regime can a chain of partial liquidations
//! extract more collateral per unit of debt repaid than one liquidation.
//!
//! Metric: the "effective seizure multiple" = USD value of collateral the
//! liquidator receives per USD of debt repaid (= `1 + realized_bonus`). A
//! ratchet would make the chain's multiple exceed the single's.

use test_harness::{
    eth_preset, usd_cents, usdc_preset, usdt_stable_preset, LendingTest, ALICE, LIQUIDATOR,
    STABLECOIN_SPOKE,
};

/// Runs one liquidation and returns `(collateral_usd_received, debt_usd_repaid)`.
/// `coll_price` is the seized collateral's USD price (fixed during the tx).
/// The liquidator's collateral-token balance delta is clean because
/// `liquidate` auto-mints only the debt token; collateral is never minted to it.
fn liquidate_once(
    t: &mut LendingTest,
    debt_asset: &str,
    debt_amount: f64,
    coll_asset: &str,
    coll_price: f64,
) -> (f64, f64) {
    let coll_before = t.token_balance(LIQUIDATOR, coll_asset);
    let debt_before = t.total_debt(ALICE);
    t.liquidate(LIQUIDATOR, ALICE, debt_asset, debt_amount);
    let coll_usd = (t.token_balance(LIQUIDATOR, coll_asset) - coll_before) * coll_price;
    let debt_usd = debt_before - t.total_debt(ALICE);
    assert!(
        debt_usd > 0.0 && coll_usd > 0.0,
        "liquidation must move positive value: coll_usd={coll_usd}, debt_usd={debt_usd}"
    );
    (coll_usd, debt_usd)
}

// Regime 1: recoverable (normal LT). HF ~0.92 sits in the band where partials
// RAISE HF, so each successive bite earns a SMALLER bonus and the chain
// extracts strictly less than a single liquidation that locks in the high
// initial-HF bonus on the whole repayment.
#[test]
fn test_partial_chain_does_not_out_extract_single_recoverable() {
    let build = || {
        let mut t = LendingTest::new()
            .with_market(usdc_preset())
            .with_market(eth_preset())
            .build();
        t.get_or_create_user(LIQUIDATOR);
        t.supply(ALICE, "USDC", 10_000.0);
        t.borrow(ALICE, "ETH", 3.0); // ~$6000 debt
        t.set_price("USDC", usd_cents(69)); // HF ~0.92
        t.assert_liquidatable(ALICE);
        t
    };
    let coll_price = 0.69;

    // Single liquidation of 0.5 ETH (~$1000).
    let mut single = build();
    let (s_coll, s_debt) = liquidate_once(&mut single, "ETH", 0.5, "USDC", coll_price);
    let single_multiple = s_coll / s_debt;

    // The same total, repaid as five 0.1 ETH partials from an identical position.
    let mut chain = build();
    let (mut c_coll, mut c_debt) = (0.0_f64, 0.0_f64);
    let mut prev_slice = f64::INFINITY;
    for _ in 0..5 {
        if !chain.can_be_liquidated(ALICE) {
            break;
        }
        let (coll, debt) = liquidate_once(&mut chain, "ETH", 0.1, "USDC", coll_price);
        let slice = coll / debt;
        // Recoverable: each bite raises HF, so the bonus must not increase.
        assert!(
            slice <= prev_slice + 0.005,
            "recoverable partials must not ratchet bonus up: {prev_slice:.5} -> {slice:.5}"
        );
        prev_slice = slice;
        c_coll += coll;
        c_debt += debt;
    }
    let chain_multiple = c_coll / c_debt;

    assert!(
        chain_multiple <= single_multiple * 1.01,
        "chained partials must not out-extract a single liquidation: \
         chain={chain_multiple:.5}, single={single_multiple:.5}"
    );
}

// Regime 2: deep / unrecoverable (normal LT). HF ~0.33 is far below
// m = proportion*(1+bonus), so every partial LOWERS HF — the exact setting the
// ratchet hypothesis targets. The engine pins the bonus to BASE here; it must
// NOT scale toward the ~25% max for 80%-LT collateral. So the realized bonus
// stays flat across the chain and the chain cannot out-extract a single shot.
#[test]
fn test_partial_chain_bonus_pinned_to_base_when_deep() {
    let build = || {
        let mut t = LendingTest::new()
            .with_market(usdc_preset())
            .with_market(eth_preset())
            .with_dust_disabled_all_markets()
            .build();
        t.get_or_create_user(LIQUIDATOR);
        t.supply(ALICE, "USDC", 10_000.0);
        t.borrow(ALICE, "ETH", 3.0);
        t.set_price("USDC", usd_cents(25)); // HF ~0.33, deeply underwater
        t.assert_liquidatable(ALICE);
        t
    };
    let coll_price = 0.25;

    let mut single = build();
    let (s_coll, s_debt) = liquidate_once(&mut single, "ETH", 0.5, "USDC", coll_price);
    let single_multiple = s_coll / s_debt;
    // Deep liquidation must use the (small) base bonus, NOT the scaled max
    // (`max_bonus_for_threshold(0.8)` ~= 25%). A scaled multiple would be ~1.25.
    assert!(
        single_multiple < 1.20,
        "deep liquidation must use base bonus, not the scaled max: multiple={single_multiple:.5}"
    );

    let mut chain = build();
    let (mut c_coll, mut c_debt) = (0.0_f64, 0.0_f64);
    let mut first_slice: Option<f64> = None;
    for _ in 0..5 {
        if !chain.can_be_liquidated(ALICE) {
            break;
        }
        let (coll, debt) = liquidate_once(&mut chain, "ETH", 0.1, "USDC", coll_price);
        let slice = coll / debt;
        match first_slice {
            None => first_slice = Some(slice),
            // Pinned to base: later (lower-HF) slices must not earn a larger
            // bonus than the first. A ratchet would make them strictly grow.
            Some(first) => assert!(
                slice <= first * 1.01,
                "deep partials must not ratchet bonus: first={first:.5}, slice={slice:.5}"
            ),
        }
        c_coll += coll;
        c_debt += debt;
    }
    let chain_multiple = c_coll / c_debt;
    assert!(
        chain_multiple <= single_multiple * 1.01,
        "chained deep partials must not out-extract a single liquidation: \
         chain={chain_multiple:.5}, single={single_multiple:.5}"
    );
}

// Regime 2 variant: deep spoke (98% threshold). Any liquidatable spoke
// position is unrecoverable, so partials lower HF — and the high threshold is
// where the operator most fears a bonus ratchet. The spoke bonus is bounded
// by the category, so the chain cannot out-extract a single liquidation.
#[test]
fn test_partial_chain_no_ratchet_spoke() {
    let build = || {
        let mut t = LendingTest::new()
            .with_market(usdc_preset())
            .with_market(usdt_stable_preset())
            .with_spoke(2, STABLECOIN_SPOKE)
            .with_spoke_asset(2, "USDC", true, true)
            .with_spoke_asset(2, "USDT", true, true)
            .with_dust_disabled_all_markets()
            .build();
        t.get_or_create_user(LIQUIDATOR);
        t.create_spoke_account(ALICE, 2);
        t.supply(ALICE, "USDC", 10_000.0);
        t.borrow(ALICE, "USDT", 9_500.0);
        t.set_price("USDC", usd_cents(85)); // HF ~0.88
        t.assert_liquidatable(ALICE);
        t
    };
    let coll_price = 0.85;

    let mut single = build();
    let (s_coll, s_debt) = liquidate_once(&mut single, "USDT", 1_000.0, "USDC", coll_price);
    let single_multiple = s_coll / s_debt;

    let mut chain = build();
    let (mut c_coll, mut c_debt) = (0.0_f64, 0.0_f64);
    let mut first_slice: Option<f64> = None;
    for _ in 0..5 {
        if !chain.can_be_liquidated(ALICE) {
            break;
        }
        let (coll, debt) = liquidate_once(&mut chain, "USDT", 200.0, "USDC", coll_price);
        let slice = coll / debt;
        match first_slice {
            None => first_slice = Some(slice),
            Some(first) => assert!(
                slice <= first * 1.01,
                "spoke partials must not ratchet bonus: first={first:.5}, slice={slice:.5}"
            ),
        }
        c_coll += coll;
        c_debt += debt;
    }
    let chain_multiple = c_coll / c_debt;
    assert!(
        chain_multiple <= single_multiple * 1.01,
        "chained spoke partials must not out-extract a single liquidation: \
         chain={chain_multiple:.5}, single={single_multiple:.5}"
    );
}
