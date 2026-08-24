//! Is `flash_position` an economic substitute for `multiply` that avoids the
//! strategy origination fee?
//!
//! `multiply` mints strategy debt with `charge_fee = true`
//! (contracts/controller/src/strategies/multiply.rs:73), so the pool withholds
//! `flashloan_fee` bps as protocol revenue
//! (contracts/pool/src/ops/strategy.rs:94). `flash_position` mints the same
//! strategy debt with `charge_fee = false`
//! (contracts/controller/src/strategies/flash_position.rs:258).
//!
//! Both endpoints end in the same place: new debt on the account, new measured
//! collateral on the same account, one shared solvency finalize. This measures
//! whether the two routes are interchangeable for a borrower, and by how much.

extern crate std;

use controller::types::PositionMode;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::xdr::ToXdr;
use soroban_sdk::{Address, Bytes, Vec};
use test_harness::{
    apply_flash_fee, build_aggregator_swap, f64_to_i128, hub_asset, FlashPositionMode,
    FlashPositionRequest, HubAssetKey, LendingTest, ALICE, HARNESS_SPOKE,
};

fn setup() -> LendingTest {
    LendingTest::new().standard_two_asset().build()
}

fn collaterals(t: &LendingTest, name: &str, min: f64) -> Vec<(HubAssetKey, i128)> {
    let mut out = Vec::new(&t.env);
    let decimals = t.resolve_market(name).decimals;
    out.push_back((hub_asset(t.resolve_asset(name)), f64_to_i128(min, decimals)));
    out
}

/// Opens 1.0 ETH of leverage through `multiply` and returns
/// `(usdc_collateral, eth_debt, eth_revenue_booked)`.
fn open_via_multiply() -> (f64, f64, i128) {
    let mut t = setup();
    t.fund_router("USDC", 3_000.0);

    let net_in = apply_flash_fee(10_000_000);
    // The mock router pays exactly `min_out`. Scale it by the same fee the
    // pool withheld so the two routes are compared on equal terms: multiply
    // only ever has `amount - fee` of ETH to sell.
    let steps = build_aggregator_swap(&t, "ETH", "USDC", net_in, apply_flash_fee(30_000_000_000));

    let revenue_before = t.snapshot_revenue("ETH");
    let account_id = t.multiply(ALICE, "USDC", 1.0, "ETH", PositionMode::Multiply, &steps);
    let revenue_after = t.snapshot_revenue("ETH");

    (
        t.supply_balance_for(ALICE, account_id, "USDC"),
        t.borrow_balance_for(ALICE, account_id, "ETH"),
        revenue_after - revenue_before,
    )
}

/// Opens 1.0 ETH of leverage through `flash_position`, where the receiver
/// returns the full unfeed proceeds as collateral. Returns
/// `(usdc_collateral, eth_debt, eth_revenue_booked)`.
fn open_via_flash_position() -> (f64, f64, i128) {
    let mut t = setup();
    let receiver = t.deploy_flash_position_receiver();

    // The receiver swaps the full 1.0 ETH it was forwarded and pushes the
    // proceeds back. No fee was withheld, so it has the whole 3_000 USDC.
    let request = FlashPositionRequest {
        mode: FlashPositionMode::Success,
        collateral: t.resolve_asset("USDC"),
        collateral_amount: f64_to_i128(3_000.0, t.resolve_market("USDC").decimals),
        extra_asset: Address::generate(&t.env),
        extra_amount: 0,
        reenter_spoke_id: HARNESS_SPOKE,
        reenter_account_id: 0,
    };
    let payload: Bytes = request.to_xdr(&t.env);
    let mins = collaterals(&t, "USDC", 2_990.0);
    let refunds = Vec::new(&t.env);

    let revenue_before = t.snapshot_revenue("ETH");
    let account_id = t.flash_position(
        ALICE,
        0,
        PositionMode::Multiply,
        "ETH",
        1.0,
        &receiver,
        &payload,
        &mins,
        &refunds,
    );
    let revenue_after = t.snapshot_revenue("ETH");

    (
        t.supply_balance_for(ALICE, account_id, "USDC"),
        t.borrow_balance_for(ALICE, account_id, "ETH"),
        revenue_after - revenue_before,
    )
}

#[test]
fn flash_position_substitutes_multiply_without_origination_fee() {
    let (mul_collateral, mul_debt, mul_revenue) = open_via_multiply();
    let (fp_collateral, fp_debt, fp_revenue) = open_via_flash_position();

    std::println!("multiply       : collateral={mul_collateral:.4} USDC  debt={mul_debt:.6} ETH  revenue={mul_revenue}");
    std::println!("flash_position : collateral={fp_collateral:.4} USDC  debt={fp_debt:.6} ETH  revenue={fp_revenue}");

    // Same debt taken on both routes.
    assert!(
        (mul_debt - fp_debt).abs() < 0.01,
        "routes should take comparable debt: multiply={mul_debt} flash_position={fp_debt}"
    );

    // multiply pays the origination fee; flash_position pays nothing.
    //
    // Assert the EXACT fee, not merely that something was charged. `> 0` would
    // still pass if the origination fee regressed to a single stroop, which
    // would leave the qualitative bypass intact while making it economically
    // meaningless — and the magnitude is what sets this finding's severity.
    // 1.0 ETH of strategy debt at DEFAULT_FLASHLOAN_FEE_BPS.
    let strategy_debt_raw = 10_000_000i128;
    let expected_fee = strategy_debt_raw - apply_flash_fee(strategy_debt_raw);
    assert_eq!(
        mul_revenue, expected_fee,
        "multiply must book exactly the origination fee on {strategy_debt_raw} \
         raw ETH of debt: expected {expected_fee}, got {mul_revenue}"
    );
    assert_eq!(
        fp_revenue, 0,
        "flash_position books no protocol revenue, got {fp_revenue}"
    );

    // Not independent evidence — the withheld fee IS the collateral difference,
    // so this restates the assertion above. Kept because it states the borrower-
    // facing consequence in the units a borrower would notice.
    // And the fee-free route leaves the borrower strictly better collateralised
    // for the same debt.
    assert!(
        fp_collateral > mul_collateral,
        "flash_position should yield more collateral for the same debt: \
         multiply={mul_collateral} flash_position={fp_collateral}"
    );
}
