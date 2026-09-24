use crate::config::config;
use common::types::SeizeMode;
use controller::constants::WAD;
use num_bigint::BigInt;
use num_rational::BigRational;
use proptest::prelude::*;
use proptest::test_runner::TestCaseError;
use soroban_sdk::vec;
use test_harness::reference;
use test_harness::{hub_asset, LendingTest, ALICE, LIQUIDATOR};

const ULP_BOUND_USD_WAD: i128 = 10;
const ULP_BOUND_TOKENS: i128 = 50;

fn target_hf_wad() -> BigRational {
    let wad = BigRational::from_integer(BigInt::from(WAD));
    wad * BigRational::from_integer(BigInt::from(110))
        / BigRational::from_integer(BigInt::from(100))
}

fn in_differential_scope(coll_wad: i128, debt_wad: i128) -> bool {
    if debt_wad >= coll_wad {
        return false;
    }
    coll_wad * 100 >= debt_wad * 115
}

fn price_for_debt_ratio(
    collateral_usd_wad: i128,
    debt_tokens: i128,
    debt_decimals: u32,
    debt_ratio_bps: i128,
) -> i128 {
    collateral_usd_wad
        .checked_mul(debt_ratio_bps)
        .and_then(|value| value.checked_mul(10i128.pow(debt_decimals)))
        .expect("generated liquidation price must fit i128")
        / (10_000 * debt_tokens)
}

fn run_liquidation_differential(
    mut t: LendingTest,
    max_ltv_frac: f64,
    supply_usdc: u64,
    borrow_eth_frac_bps: u16,
    debt_ratio_bps: u16,
    liq_repay_frac_bps: u16,
    seize_mode: SeizeMode,
) -> Result<(), TestCaseError> {
    t.supply(ALICE, "USDC", supply_usdc as f64);

    let max_eth = (supply_usdc as f64) * max_ltv_frac / 2000.0;
    let borrow_amt = max_eth * (borrow_eth_frac_bps as f64 / 10_000.0);
    let borrow_result = t.try_borrow(ALICE, "ETH", borrow_amt);
    prop_assert!(
        borrow_result.is_ok(),
        "generated in-LTV borrow failed: amount={} error={:?}",
        borrow_amt,
        borrow_result.err()
    );

    let collateral_usd_wad = t.total_collateral_raw(ALICE);
    let debt_tokens = t.borrow_balance_raw(ALICE, "ETH");
    let eth_decimals = t.resolve_market("ETH").decimals;
    let new_eth_price = price_for_debt_ratio(
        collateral_usd_wad,
        debt_tokens,
        eth_decimals,
        debt_ratio_bps as i128,
    );
    t.set_price("ETH", new_eth_price);
    prop_assert!(
        t.health_factor_raw(ALICE) < WAD,
        "generated account must be liquidatable"
    );

    let coll_wad = t.total_collateral_raw(ALICE);
    let debt_wad = t.total_debt_raw(ALICE);
    prop_assert!(
        in_differential_scope(coll_wad, debt_wad),
        "generated account escaped differential scope: collateral={} debt={}",
        coll_wad,
        debt_wad
    );

    let ref_coll = reference::snapshot_collateral(&t, ALICE);
    let ref_debt = reference::snapshot_debt(&t, ALICE);
    prop_assert!(!ref_coll.is_empty() && !ref_debt.is_empty());

    let current_debt_eth = t.borrow_balance(ALICE, "ETH");
    let repay_amt = current_debt_eth * (liq_repay_frac_bps as f64 / 10_000.0);

    let repay_tokens = reference::float_to_bigrational(repay_amt, eth_decimals);
    let ref_payments = std::vec![(0u32, repay_tokens)];
    let ref_result =
        reference::compute_liquidation(&ref_coll, &ref_debt, &ref_payments, target_hf_wad());
    let ref_total_repaid_usd_wad =
        reference::bigrational_to_i128_wad(&ref_result.total_repaid_usd_wad);
    let ref_total_seized_usd_wad =
        reference::bigrational_to_i128_wad(&ref_result.total_seized_usd_wad);

    let debt_before_usd = t.total_debt_raw(ALICE);
    let coll_before_usd = t.total_collateral_raw(ALICE);
    let usdc_supply_before_tokens = t.supply_balance_raw(ALICE, "USDC");
    let usdc_revenue_before = t.snapshot_revenue("USDC");

    let liq_res = t.try_liquidate_with_mode(LIQUIDATOR, ALICE, "ETH", repay_amt, seize_mode);
    prop_assert!(
        liq_res.is_ok(),
        "in-scope liquidation failed: repay={} reference_repaid={} mode={:?} error={:?}",
        repay_amt,
        ref_total_repaid_usd_wad,
        seize_mode,
        liq_res.err()
    );

    let debt_after_usd = if t.find_account_id(ALICE).is_some() {
        t.total_debt_raw(ALICE)
    } else {
        0
    };
    let coll_after_usd = if t.find_account_id(ALICE).is_some() {
        t.total_collateral_raw(ALICE)
    } else {
        0
    };

    let prod_debt_reduction = debt_before_usd - debt_after_usd;
    let prod_coll_reduction = coll_before_usd - coll_after_usd;

    let debt_diff = (prod_debt_reduction - ref_total_repaid_usd_wad).abs();
    let debt_ref_abs = ref_total_repaid_usd_wad.abs();
    let debt_rel_ok = debt_ref_abs == 0 || debt_diff * 1_000 <= debt_ref_abs;
    prop_assert!(debt_diff <= ULP_BOUND_USD_WAD || debt_rel_ok);

    let coll_diff = (prod_coll_reduction - ref_total_seized_usd_wad).abs();
    let coll_ref_abs = ref_total_seized_usd_wad.abs();
    let coll_rel_ok = coll_ref_abs == 0 || coll_diff * 1_000 <= coll_ref_abs;
    prop_assert!(coll_diff <= ULP_BOUND_USD_WAD || coll_rel_ok);

    let usdc_supply_after_tokens = if t.find_account_id(ALICE).is_some() {
        t.supply_balance_raw(ALICE, "USDC")
    } else {
        0
    };
    let prod_usdc_seized = usdc_supply_before_tokens - usdc_supply_after_tokens;
    let (_aid, ref_usdc_seized_tokens) = ref_result
        .seized_per_collateral
        .iter()
        .find(|(aid, _)| *aid == 0)
        .expect("reference should seize collateral");
    let ref_usdc_seized_i128 = reference::bigrational_to_i128_half_up(ref_usdc_seized_tokens);

    let usdc_diff = (prod_usdc_seized - ref_usdc_seized_i128).abs();
    let usdc_ref_abs = ref_usdc_seized_i128.abs();
    let usdc_rel_ok = usdc_ref_abs == 0 || usdc_diff * 200 <= usdc_ref_abs;
    prop_assert!(usdc_diff <= ULP_BOUND_TOKENS || usdc_rel_ok);

    // The liquidation is a single transaction at a fixed timestamp, so the whole
    // revenue delta is the withheld fee.
    let prod_usdc_fee = t.snapshot_revenue("USDC") - usdc_revenue_before;
    let (_fid, ref_usdc_fee_tokens) = ref_result
        .protocol_fee_per_collateral
        .iter()
        .find(|(aid, _)| *aid == 0)
        .expect("reference should model a fee for the seized collateral");
    let ref_usdc_fee_i128 = reference::bigrational_to_i128_half_up(ref_usdc_fee_tokens);
    let fee_diff = (prod_usdc_fee - ref_usdc_fee_i128).abs();
    let fee_ref_abs = ref_usdc_fee_i128.abs();
    let fee_rel_ok = fee_ref_abs == 0 || fee_diff * 200 <= fee_ref_abs;
    prop_assert!(
        fee_diff <= ULP_BOUND_TOKENS || fee_rel_ok,
        "protocol fee drift: production {} reference {}",
        prod_usdc_fee,
        ref_usdc_fee_i128
    );

    Ok(())
}

proptest! {
    #![proptest_config(config(32))]

    #[test]
    fn prop_liquidation_matches_bigrational_reference(
        supply_usdc in 1_000u64..500_000u64,
        borrow_eth_frac_bps in 5_000u16..9_000u16,
        debt_ratio_bps in 8_150u16..8_600u16,
        // Inclusive, so a full (100%) repayment is generated. The reference
        // clamps the payment to the outstanding debt.
        liq_repay_frac_bps in 500u16..=10_000u16,
        // Both seize modes share one planner and one reference model: the mode
        // decides only how the liquidator takes delivery, never what the
        // liquidated account gives up.
        use_credit in any::<bool>(),
    ) {
        let t = LendingTest::new().standard_two_asset().build();
        let seize_mode = if use_credit {
            SeizeMode::Credit(0)
        } else {
            SeizeMode::Transfer
        };
        run_liquidation_differential(
            t,
            0.75,
            supply_usdc,
            borrow_eth_frac_bps,
            debt_ratio_bps,
            liq_repay_frac_bps,
            seize_mode,
        )?;
    }
}

fn usd_wad(raw: i128, price_wad: i128, decimals: u32) -> i128 {
    raw * price_wad / 10i128.pow(decimals)
}

fn within_one_unit(prod: i128, reference: i128, unit_usd: i128) -> bool {
    (prod - reference).abs() <= unit_usd + ULP_BOUND_USD_WAD
}

/// Accounts whose HF-preserving cap is below the base bonus: the band
/// `D <= C < D * (1 + base)` and insolvent books. Offers run up to 1.5x the
/// debt, so band closes exercise the offered pull and its pool refund, and
/// insolvent over-offers exercise the collateral-backed cap.
fn run_below_base_differential(
    mut t: LendingTest,
    supply_usdc: u64,
    debt_ratio_bps: u16,
    offer_frac_bps: u16,
) -> Result<(), TestCaseError> {
    t.supply(ALICE, "USDC", supply_usdc as f64);
    let borrow_result = t.try_borrow(ALICE, "ETH", supply_usdc as f64 * 0.75 / 2000.0 * 0.9);
    prop_assert!(
        borrow_result.is_ok(),
        "generated in-LTV borrow failed: {:?}",
        borrow_result.err()
    );

    let (eth_asset, eth_decimals) = {
        let eth = t.resolve_market("ETH");
        (eth.asset.clone(), eth.decimals)
    };
    let debt_tokens = t.borrow_balance_raw(ALICE, "ETH");
    let eth_price = price_for_debt_ratio(
        t.total_collateral_raw(ALICE),
        debt_tokens,
        eth_decimals,
        debt_ratio_bps as i128,
    );
    let eth_unit_usd = usd_wad(1, eth_price, eth_decimals);
    t.set_price("ETH", eth_price);
    prop_assert!(
        t.health_factor_raw(ALICE) < WAD,
        "generated account must be liquidatable"
    );

    let coll_before = t.total_collateral_raw(ALICE);
    let debt_before = t.total_debt_raw(ALICE);
    let offer = debt_tokens * offer_frac_bps as i128 / 10_000;
    prop_assume!(offer > 0);

    let ref_result = reference::compute_liquidation(
        &reference::snapshot_collateral(&t, ALICE),
        &reference::snapshot_debt(&t, ALICE),
        &[(0u32, BigRational::from_integer(BigInt::from(offer)))],
        target_hf_wad(),
    );
    let ref_repaid_usd = reference::bigrational_to_i128_wad(&ref_result.total_repaid_usd_wad);
    let ref_bonus = ref_result.final_bonus_bps.floor().to_integer();

    let account_id = t.resolve_account_id(ALICE);
    let liquidator = t.get_or_create_user(LIQUIDATOR);
    let payments = vec![&t.env, (hub_asset(eth_asset), offer)];
    let estimate =
        t.ctrl_client()
            .get_liquidation_estimate(&account_id, &payments, &SeizeMode::Transfer);
    let bonus_gap = BigInt::from(estimate.bonus_rate_bps) - &ref_bonus;
    prop_assert!(
        bonus_gap >= BigInt::from(-1) && bonus_gap <= BigInt::from(1),
        "bonus: production {} reference {}",
        estimate.bonus_rate_bps,
        ref_bonus
    );
    prop_assert!(
        within_one_unit(estimate.max_payment_wad, ref_repaid_usd, eth_unit_usd),
        "repayment: production {} reference {}",
        estimate.max_payment_wad,
        ref_repaid_usd
    );

    let eth_before = t.token_balance_raw(LIQUIDATOR, "ETH");
    let usdc_before = t.token_balance_raw(LIQUIDATOR, "USDC");
    t.resolve_market("ETH")
        .token_admin
        .mint(&liquidator, &offer);
    let executed =
        t.ctrl_client()
            .try_liquidate(&liquidator, &account_id, &payments, &SeizeMode::Transfer);
    prop_assert!(
        matches!(executed, Ok(Ok(_))),
        "below-base liquidation failed: offer={} debt={} error={:?}",
        offer,
        debt_tokens,
        executed.err()
    );

    let spent = eth_before + offer - t.token_balance_raw(LIQUIDATOR, "ETH");
    let spent_usd = usd_wad(spent, eth_price, eth_decimals);
    if coll_before < debt_before {
        let base_bonus_bps = i128::from(t.get_asset_config("USDC").liquidation_bonus);
        let backed = coll_before * 10_000 / (10_000 + base_bonus_bps);
        prop_assert!(
            spent_usd <= backed,
            "insolvent pull {} above the backed quote {}",
            spent_usd,
            backed
        );
    }
    prop_assert!(
        within_one_unit(spent_usd, ref_repaid_usd, eth_unit_usd),
        "pulled and refunded: net spend {} USD vs reference {}",
        spent_usd,
        ref_repaid_usd
    );
    let usdc_unit_usd = usd_wad(
        1,
        t.resolve_market("USDC").price_wad,
        t.resolve_market("USDC").decimals,
    );
    let received_usd = usd_wad(
        t.token_balance_raw(LIQUIDATOR, "USDC") - usdc_before,
        t.resolve_market("USDC").price_wad,
        t.resolve_market("USDC").decimals,
    );
    prop_assert!(
        received_usd + 2 * usdc_unit_usd + ULP_BOUND_USD_WAD >= spent_usd,
        "liquidator lost money: received {} for {}",
        received_usd,
        spent_usd
    );

    if coll_before >= debt_before && t.find_account_id(ALICE).is_some() {
        let (coll_after, debt_after) = (t.total_collateral_raw(ALICE), t.total_debt_raw(ALICE));
        if debt_after > 0 {
            let (before, after) = (
                BigInt::from(coll_before) * debt_after,
                BigInt::from(coll_after) * debt_before,
            );
            prop_assert!(
                after >= before,
                "band partial lowered coverage: {} -> {}",
                before,
                after
            );
        }
    }
    Ok(())
}

proptest! {
    #![proptest_config(config(32))]

    #[test]
    fn prop_below_base_liquidation_matches_reference_and_never_loses_the_liquidator_money(
        supply_usdc in 1_000u64..500_000u64,
        debt_ratio_bps in 9_530u16..10_800u16,
        offer_frac_bps in 500u16..=15_000u16,
    ) {
        let t = LendingTest::new().standard_two_asset().build();
        run_below_base_differential(t, supply_usdc, debt_ratio_bps, offer_frac_bps)?;
    }
}

#[test]
fn below_base_differential_holds_at_exact_cover() {
    for offer_frac_bps in [500u16, 10_000, 15_000] {
        let t = LendingTest::new().standard_two_asset().build();
        run_below_base_differential(t, 1_000, 10_000, offer_frac_bps)
            .unwrap_or_else(|e| panic!("C == D, offer {offer_frac_bps} bps: {e}"));
    }
}
