use crate::config::config_with_rejects;
use controller::constants::WAD;
use num_bigint::BigInt;
use num_rational::BigRational;
use proptest::prelude::*;
use test_harness::reference;
use test_harness::{helpers::usd, LendingTest, ALICE, LIQUIDATOR};

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

proptest! {
    #![proptest_config(config_with_rejects(128, 100_000))]

    #[test]
    fn prop_liquidation_matches_bigrational_reference(
        supply_usdc in 1_000u64..500_000u64,
        borrow_eth_frac_bps in 100u16..9_000u16,
        eth_price_rise_bps in 5_000u16..15_000u16,
        liq_repay_frac_bps in 500u16..10_000u16,
    ) {
        let mut t = LendingTest::new().standard_two_asset().build();
        t.supply(ALICE, "USDC", supply_usdc as f64);

        let max_eth = (supply_usdc as f64) * 0.75 / 2000.0;
        let borrow_amt = max_eth * (borrow_eth_frac_bps as f64 / 10_000.0);
        if borrow_amt < 0.0001 || t.try_borrow(ALICE, "ETH", borrow_amt).is_err() {
            return Ok(());
        }

        let new_eth_price = usd(2000) * (10_000 + eth_price_rise_bps as i128) / 10_000;
        t.set_price("ETH", new_eth_price);
        if t.health_factor_raw(ALICE) >= WAD {
            return Ok(());
        }

        let coll_wad = t.total_collateral_raw(ALICE);
        let debt_wad = t.total_debt_raw(ALICE);
        if !in_differential_scope(coll_wad, debt_wad) {
            return Ok(());
        }

        let ref_coll = reference::snapshot_collateral(&t, ALICE);
        let ref_debt = reference::snapshot_debt(&t, ALICE);
        prop_assert!(!ref_coll.is_empty() && !ref_debt.is_empty());

        let current_debt_eth = t.borrow_balance(ALICE, "ETH");
        let repay_amt = current_debt_eth * (liq_repay_frac_bps as f64 / 10_000.0);
        if repay_amt < 0.0001 {
            return Ok(());
        }

        let eth_decimals = t.resolve_market("ETH").decimals;
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
        if liq_res.is_err() {
            if ref_total_repaid_usd_wad == 0 {
                return Ok(());
            }
            prop_assert!(ref_total_repaid_usd_wad.abs() < ULP_BOUND_USD_WAD);
            return Ok(());
        }

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
