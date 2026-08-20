use crate::config::config;
use crate::strategy_helpers::flash_guard_cleared;
use controller::constants::WAD;
use proptest::prelude::*;
use test_harness::mock_blend::{MockBlend, KIND_COLLATERAL, KIND_LIABILITY, KIND_SUPPLY};
use test_harness::{
    assert_contract_error, errors, f64_to_i128, usdc_preset, LendingTest, ALICE, BOB, HARNESS_HUB,
    HARNESS_SPOKE,
};

const USDC: &str = "USDC";
const SLACK: i128 = 4;

fn setup() -> LendingTest {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    // Donated preset cash does not count as `supplied`. A second user must
    // actually supply or a modest hub borrow sits on max utilization (#127).
    t.supply(BOB, USDC, 100_000.0);
    t
}

fn raw(t: &LendingTest, amount: f64) -> i128 {
    f64_to_i128(amount, t.resolve_market(USDC).decimals)
}

// Migration is a pure position transfer: no accrual, no fee, no swap. It must
// reconcile exactly, so the only allowance is fixed-point dust.
fn close(actual: i128, expected: i128) -> bool {
    (actual - expected).abs() <= SLACK
}

fn assert_hygiene(t: &LendingTest) -> Result<(), TestCaseError> {
    prop_assert!(
        flash_guard_cleared(t),
        "flash guard still set after migrate"
    );
    let leftover = t.controller_token_balance_raw(USDC);
    prop_assert!(
        leftover.abs() <= SLACK,
        "controller leftover {leftover} USDC"
    );
    Ok(())
}

proptest! {
    #![proptest_config(config(32))]

    #[test]
    fn prop_migrate_blend_reconciles_same_asset(
        coll_units in 1_000u32..8_000u32,
        supply_units in 0u32..2_000u32,
        debt_units in 0u32..1_500u32,
        cap_extra_bps in 0u32..5_000u32,
        into_existing in proptest::bool::ANY,
    ) {
        let mut t = setup();
        let coll = coll_units as f64;
        let supply = supply_units as f64;
        let mut debt = debt_units as f64;
        if debt > 0.0 {
            let max_debt = (coll * 0.4).max(1.0);
            if debt > max_debt {
                debt = max_debt;
            }
        }

        let prior_supply = if into_existing {
            t.supply(ALICE, USDC, 100.0);
            t.supply_balance_raw(ALICE, USDC)
        } else {
            0
        };
        let prior_debt = t.borrow_balance_raw(ALICE, USDC);
        let account_id = if into_existing {
            t.resolve_account_id(ALICE)
        } else {
            0
        };

        t.seed_blend(ALICE, USDC, KIND_COLLATERAL, coll);
        if supply > 0.0 {
            t.seed_blend(ALICE, USDC, KIND_SUPPLY, supply);
        }
        if debt > 0.0 {
            t.seed_blend(ALICE, USDC, KIND_LIABILITY, debt);
        }

        let supply_assets: &[&str] = if supply > 0.0 { &[USDC] } else { &[] };
        let debt_caps: Vec<(&str, f64)> = if debt > 0.0 {
            let cap = debt * (10_000.0 + cap_extra_bps as f64) / 10_000.0;
            vec![(USDC, cap)]
        } else {
            vec![]
        };

        let id = t
            .try_migrate_from_blend(ALICE, account_id, &[USDC], supply_assets, &debt_caps)
            .map_err(|e| TestCaseError::fail(format!("migrate failed: {e:?}")))?;

        if into_existing {
            prop_assert_eq!(id, account_id, "existing account must be reused");
        } else {
            prop_assert!(id > 0);
            prop_assert!(t.account_exists(id));
        }

        prop_assert_eq!(t.blend_position(ALICE, USDC, KIND_COLLATERAL), 0);
        prop_assert_eq!(t.blend_position(ALICE, USDC, KIND_SUPPLY), 0);
        prop_assert_eq!(t.blend_position(ALICE, USDC, KIND_LIABILITY), 0);

        let hub_supply = t.supply_balance_raw_for(id, USDC);
        let expected_supply = prior_supply + raw(&t, coll) + raw(&t, supply);
        prop_assert!(
            close(hub_supply, expected_supply),
            "hub supply {hub_supply} want ~{expected_supply}"
        );

        let hub_debt = t.borrow_balance_raw_for(id, USDC);
        let expected_debt = prior_debt + raw(&t, debt);
        prop_assert!(
            close(hub_debt, expected_debt),
            "hub debt {hub_debt} want ~{expected_debt} (not the cap)"
        );

        if expected_debt > 0 {
            prop_assert!(
                t.health_factor_for_raw(ALICE, id) >= WAD,
                "HF below 1 after migrate"
            );
        }
        assert_hygiene(&t)?;
    }

    #[test]
    fn prop_migrate_blend_cap_too_low_reverts(
        coll_units in 1_000u32..5_000u32,
        debt_units in 200u32..800u32,
    ) {
        prop_assume!(coll_units as f64 >= debt_units as f64 * 2.0);
        let mut t = setup();
        let coll = coll_units as f64;
        let debt = debt_units as f64;
        t.seed_blend(ALICE, USDC, KIND_COLLATERAL, coll);
        t.seed_blend(ALICE, USDC, KIND_LIABILITY, debt);
        let result = t.try_migrate_from_blend(
            ALICE,
            0,
            &[USDC],
            &[],
            &[(USDC, debt * 0.5)],
        );
        assert_contract_error(result, 1);
        prop_assert!(flash_guard_cleared(&t));
        prop_assert_eq!(t.controller_token_balance_raw(USDC), 0);
        prop_assert!(t.blend_position(ALICE, USDC, KIND_COLLATERAL) > 0);
        prop_assert!(t.blend_position(ALICE, USDC, KIND_LIABILITY) > 0);
    }
}

#[test]
fn migrate_blend_rejects_empty_duplicate_unapproved_zero_cap() {
    let mut t = setup();
    t.ensure_approved_blend();
    let caller = t.get_or_create_user(ALICE);

    assert_contract_error(
        t.try_migrate_from_blend(ALICE, 0, &[], &[], &[]),
        errors::INVALID_PAYMENTS,
    );

    assert_contract_error(
        t.try_migrate_from_blend(ALICE, 0, &[], &[], &[(USDC, 1.0), (USDC, 1.0)]),
        errors::ASSETS_ARE_THE_SAME,
    );

    assert_contract_error(
        t.try_migrate_from_blend(ALICE, 0, &[], &[], &[(USDC, 0.0)]),
        errors::AMOUNT_MUST_BE_POSITIVE,
    );

    let unapproved = t.env.register(MockBlend, ());
    let empty: soroban_sdk::Vec<soroban_sdk::Address> = soroban_sdk::Vec::new(&t.env);
    let mut coll = soroban_sdk::Vec::new(&t.env);
    coll.push_back(t.resolve_asset(USDC));
    let empty_debt: soroban_sdk::Vec<(soroban_sdk::Address, i128)> = soroban_sdk::Vec::new(&t.env);
    let result = t.ctrl_client().try_migrate_from_blend(
        &caller,
        &0u64,
        &HARNESS_SPOKE,
        &HARNESS_HUB,
        &unapproved,
        &coll,
        &empty,
        &empty_debt,
    );
    let err = match result {
        Ok(Ok(_)) => panic!("unapproved pool migrate succeeded"),
        Ok(Err(e)) => e,
        Err(e) => e.expect("expected contract error"),
    };
    assert_eq!(
        err,
        soroban_sdk::Error::from_contract_error(errors::BLEND_POOL_NOT_APPROVED)
    );
    assert!(flash_guard_cleared(&t));
}
