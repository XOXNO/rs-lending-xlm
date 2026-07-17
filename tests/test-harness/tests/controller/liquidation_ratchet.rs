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

// Regime 2: deep (normal LT). HF ~0.33 takes the max bonus (~25% for 80%-LT
// collateral). At the max bonus `proportion*(1+bonus) == 1`, so a partial is
// HF-neutral-to-rising: the account heals, later slices earn a non-increasing
// bonus, and the chain cannot out-extract a single shot — anti-ratchet holds via
// the per-threshold ceiling, not a base-bonus pin.
#[test]
fn test_partial_chain_deep_does_not_ratchet() {
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
    // Deep liquidation takes the max bonus, bounded by the per-threshold ceiling.
    assert!(
        single_multiple <= 1.26,
        "deep bonus bounded by the per-threshold max: multiple={single_multiple:.5}"
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
            // As the account heals, later slices earn a non-increasing bonus.
            // A ratchet would make them strictly grow.
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

// Regime 4: solvent-toxic (high LT). Collateral still covers the debt but no
// partial repayment is HF-safe (hf/p - 1 sits below the base bonus), so the
// plan rejects partials with `FullCloseRequired`; a debt-covering payment
// closes the account with zero socializable residue.
#[test]
fn test_solvent_toxic_rejects_partial_and_accepts_full_close() {
    use test_harness::{assert_contract_error, errors, usd_cents as cents};

    let mut t = LendingTest::new()
        .with_market(usdt_stable_preset())
        .with_market(eth_preset())
        .build();
    t.get_or_create_user(LIQUIDATOR);

    // USDT: LT 95%, base bonus 2%. $10k supply, 4 ETH ($8k) debt.
    t.supply(ALICE, "USDT", 10_000.0);
    t.borrow(ALICE, "ETH", 4.0);

    // Drop USDT to $0.81: C = $8100 >= D = $8000, HF = 0.9619 in
    // [p, p*(1+base)) = [0.95, 0.969) -> cap = 126 bps < base 200.
    t.set_price("USDT", cents(81));
    t.assert_liquidatable(ALICE);

    // Partial repayment is rejected outright.
    let partial = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 0.5);
    assert_contract_error(partial, errors::FULL_CLOSE_REQUIRED);

    // A debt-covering payment closes the account: all $8100 of collateral is
    // seized (seizure 8000 * 1.02 = $8160 capped at C), debt reaches zero,
    // nothing is left to socialize, and the account is cleaned up.
    let liq_usdt_before = t.token_balance(LIQUIDATOR, "USDT");
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 4.1);
    assert!(
        t.find_account_id(ALICE).is_none(),
        "full close must clean up the emptied account"
    );
    let seized_usdt = t.token_balance(LIQUIDATOR, "USDT") - liq_usdt_before;
    assert!(
        seized_usdt > 9_900.0,
        "full close seizes ~all 10k USDT collateral, got {seized_usdt}"
    );
}
