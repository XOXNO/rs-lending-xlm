use crate::config::config;
use crate::ops::{capture_indexes, execute_op, op_strategy, ASSETS, LendingOp, USERS};
use proptest::prelude::*;
use test_harness::{seed_fuzz_conservation_book, LendingTest};

const TOLERANCE_UNITS: i128 = 4;

fn sum_supply(t: &test_harness::LendingTest, asset: &str) -> i128 {
    USERS.iter().map(|u| t.supply_balance_raw(u, asset)).sum()
}

fn sum_borrow(t: &test_harness::LendingTest, asset: &str) -> i128 {
    USERS.iter().map(|u| t.borrow_balance_raw(u, asset)).sum()
}

struct PoolSnapshot {
    supplied: i128,
    borrowed: i128,
    reserves: i128,
    revenue: i128,
    sum_user_supply: i128,
    sum_user_borrow: i128,
}

fn pool_snapshot(t: &test_harness::LendingTest, asset: &str) -> PoolSnapshot {
    let pc = t.pool_client(asset);
    PoolSnapshot {
        supplied: pc.supplied_amount(),
        borrowed: pc.borrowed_amount(),
        reserves: pc.reserves(),
        revenue: pc.protocol_revenue(),
        sum_user_supply: sum_supply(t, asset),
        sum_user_borrow: sum_borrow(t, asset),
    }
}

fn assert_accounting_laws(
    step: usize,
    op: &LendingOp,
    asset: &str,
    s: &PoolSnapshot,
) -> Result<(), TestCaseError> {
    prop_assert!(s.reserves >= 0, "step {} {:?}: {} reserves < 0", step, op, asset);
    prop_assert!(
        s.revenue <= s.supplied + TOLERANCE_UNITS,
        "step {} {:?}: {} revenue ({}) > supplied ({})",
        step,
        op,
        asset,
        s.revenue,
        s.supplied
    );
    let borrow_diff = (s.sum_user_borrow - s.borrowed).abs();
    prop_assert!(
        borrow_diff <= TOLERANCE_UNITS,
        "step {} {:?}: {} borrow mismatch user_sum={} pool={}",
        step,
        op,
        asset,
        s.sum_user_borrow,
        s.borrowed
    );
    let solvency_slack = s.reserves + s.borrowed - s.supplied;
    prop_assert!(
        solvency_slack >= -TOLERANCE_UNITS,
        "step {} {:?}: {} solvency violated",
        step,
        op,
        asset
    );
    let supply_diff = (s.supplied - s.sum_user_supply - s.revenue).abs();
    prop_assert!(
        supply_diff <= TOLERANCE_UNITS,
        "step {} {:?}: {} supply conservation violated",
        step,
        op,
        asset
    );
    Ok(())
}

proptest! {
    #![proptest_config(config(32))]

    #[test]
    fn prop_accounting_conservation(ops in prop::collection::vec(op_strategy(), 5..15)) {
        let mut t = LendingTest::new().three_asset_usdc_eth_wbtc().build();
        seed_fuzz_conservation_book(&mut t);
        let mut last_idx = capture_indexes(&t);

        for (i, op) in ops.iter().enumerate() {
            execute_op(&mut t, op);

            for asset in &ASSETS {
                assert_accounting_laws(i, op, asset, &pool_snapshot(&t, asset))?;
            }

            let next_idx = capture_indexes(&t);
            for (j, (before, after)) in last_idx.iter().zip(next_idx.iter()).enumerate() {
                prop_assert!(
                    after.0 >= before.0,
                    "step {} {:?}: asset[{}] supply_index regressed {} -> {}",
                    i, op, j, before.0, after.0
                );
                prop_assert!(
                    after.1 >= before.1,
                    "step {} {:?}: asset[{}] borrow_index regressed {} -> {}",
                    i, op, j, before.1, after.1
                );
            }
            last_idx = next_idx;
        }
    }
}