use crate::config::config;
use controller::constants::WAD;
use num_bigint::BigInt;
use num_rational::BigRational;
use proptest::prelude::*;
use test_harness::reference;
use test_harness::{LendingTest, ALICE, LIQUIDATOR};

const ULP_BOUND_USD_WAD: i128 = 10;
const ULP_BOUND_TOKENS: i128 = 50;

fn target_hf_wad() -> BigRational {
    let wad = BigRational::from_integer(BigInt::from(WAD));
    wad * BigRational::from_integer(BigInt::from(102))
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

proptest! {
    #![proptest_config(config(32))]

    #[test]
    fn prop_liquidation_matches_bigrational_reference(
        supply_usdc in 1_000u64..500_000u64,
        borrow_eth_frac_bps in 5_000u16..9_000u16,
        debt_ratio_bps in 8_150u16..8_600u16,
        liq_repay_frac_bps in 500u16..10_000u16,
    ) {
        let mut t = LendingTest::new().standard_two_asset().build();
        t.supply(ALICE, "USDC", supply_usdc as f64);

        let max_eth = (supply_usdc as f64) * 0.75 / 2000.0;
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
        prop_assert!(t.health_factor_raw(ALICE) < WAD, "generated account must be liquidatable");

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
        let ref_result = reference::compute_liquidation(
            &ref_coll,
            &ref_debt,
            &ref_payments,
            target_hf_wad(),
        );
        let ref_total_repaid_usd_wad =
            reference::bigrational_to_i128_wad(&ref_result.total_repaid_usd_wad);
        let ref_total_seized_usd_wad =
            reference::bigrational_to_i128_wad(&ref_result.total_seized_usd_wad);

        let debt_before_usd = t.total_debt_raw(ALICE);
        let coll_before_usd = t.total_collateral_raw(ALICE);
        let usdc_supply_before_tokens = t.supply_balance_raw(ALICE, "USDC");
        let liq_res = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", repay_amt);
        prop_assert!(
            liq_res.is_ok(),
            "in-scope liquidation failed: repay={} reference_repaid={} error={:?}",
            repay_amt,
            ref_total_repaid_usd_wad,
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
        let ref_usdc_seized_i128 =
            reference::bigrational_to_i128_half_up(ref_usdc_seized_tokens);

        let usdc_diff = (prod_usdc_seized - ref_usdc_seized_i128).abs();
        let usdc_ref_abs = ref_usdc_seized_i128.abs();
        let usdc_rel_ok = usdc_ref_abs == 0 || usdc_diff * 200 <= usdc_ref_abs;
        prop_assert!(usdc_diff <= ULP_BOUND_TOKENS || usdc_rel_ok);
    }
}
