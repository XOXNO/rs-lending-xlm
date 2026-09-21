use crate::config::config;
use crate::ops::{capture_indexes, execute_op, op_strategy, LendingOp, ASSETS, USERS};
use crate::strategy_helpers::{flash_guard_cleared, router_allowance};
use common::math::fp::Ray;
use proptest::prelude::*;
use soroban_sdk::token;
use std::sync::atomic::{AtomicI64, Ordering};
use test_harness::presets::{eth_preset, usdc_preset, wbtc_preset};
use test_harness::{
    f64_to_i128, hub_asset, seed_fuzz_conservation_book, LendingTest, ALICE, BOB, LIQUIDATOR,
};

const TOLERANCE_UNITS: i128 = 4;

fn sum_supply(t: &test_harness::LendingTest, asset: &str) -> i128 {
    USERS
        .iter()
        .map(|u| sum_user_side(t, u, asset, false))
        .sum()
}

fn sum_borrow(t: &test_harness::LendingTest, asset: &str) -> i128 {
    USERS.iter().map(|u| sum_user_side(t, u, asset, true)).sum()
}

fn sum_user_side(t: &test_harness::LendingTest, user: &str, asset: &str, borrow: bool) -> i128 {
    let Some(state) = t.users.get(user) else {
        return 0;
    };
    if state.accounts.is_empty() {
        return if borrow {
            t.borrow_balance_raw(user, asset)
        } else {
            t.supply_balance_raw(user, asset)
        };
    }
    state
        .accounts
        .iter()
        .map(|entry| {
            if borrow {
                t.borrow_balance_raw_for(entry.account_id, asset)
            } else {
                t.supply_balance_raw_for(entry.account_id, asset)
            }
        })
        .sum()
}

fn controller_balance(t: &LendingTest, asset: &str) -> i128 {
    let market = t.resolve_market(asset);
    token::Client::new(&t.env, &market.asset).balance(&t.controller)
}

fn assert_strategy_hygiene(
    step: usize,
    op: &LendingOp,
    t: &LendingTest,
) -> Result<(), TestCaseError> {
    prop_assert!(
        flash_guard_cleared(t),
        "step {} {:?}: flash guard still set",
        step,
        op
    );
    for asset in &ASSETS {
        prop_assert_eq!(
            router_allowance(t, asset),
            0,
            "step {} {:?}: {} router allowance leaked",
            step,
            op,
            asset
        );
        let leftover = controller_balance(t, asset);
        prop_assert!(
            leftover.abs() <= TOLERANCE_UNITS,
            "step {} {:?}: controller leftover {} of {}",
            step,
            op,
            leftover,
            asset
        );
    }
    Ok(())
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
    let asset_key = hub_asset(t.resolve_asset(asset));
    let pc = t.pool_client(asset);
    PoolSnapshot {
        supplied: pc.get_supplied_amount(&asset_key),
        borrowed: pc.get_borrowed_amount(&asset_key),
        reserves: pc.get_reserves(&asset_key),
        revenue: pc.get_revenue(&asset_key),
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
    prop_assert!(
        s.reserves >= 0,
        "step {} {:?}: {} reserves < 0",
        step,
        op,
        asset
    );
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
            assert_strategy_hygiene(i, op, &t)?;

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

// Seed-adjusted conservation, in RAY asset precision:
//   surplus = (cash - seed) + borrowed * borrow_index - supplied * supply_index
// Every rounding favours the pool, so surplus stays in [-dust, 8 token units] per op.

/// Rounding dust allowed below zero: a few raw RAY per leg.
const RAY_DUST: i128 = 1_000_000_000;
const MAX_ROUNDING_LEGS_PER_OP: i128 = 8;

fn raw_pool_state(t: &LendingTest, asset: &str) -> controller::types::PoolStateRaw {
    let market = t.resolve_market(asset);
    let key = controller::types::PoolKey::State(hub_asset(market.asset.clone()));
    t.env.as_contract(&market.pool, || {
        t.env.storage().persistent().get(&key).unwrap()
    })
}

fn pool_token_balance(t: &LendingTest, asset: &str) -> i128 {
    let market = t.resolve_market(asset);
    token::Client::new(&t.env, &market.asset).balance(&market.pool)
}

/// RAY units in one raw token unit of `asset`.
fn unit_ray(t: &LendingTest, asset: &str) -> i128 {
    10i128.pow(27 - t.resolve_market(asset).decimals)
}

/// `(cash - seed) + debt value - supplier claims`, in RAY asset precision.
fn seed_adjusted_surplus_ray(t: &LendingTest, asset: &str, seed: i128) -> i128 {
    let s = raw_pool_state(t, asset);
    let debt = Ray::from(s.borrowed).mul(&t.env, Ray::from(s.borrow_index));
    let claims = Ray::from(s.supplied).mul(&t.env, Ray::from(s.supply_index));
    (s.cash - seed) * unit_ray(t, asset) + debt.raw() - claims.raw()
}

fn assert_cash_conservation_and_custody(
    step: usize,
    op: &dyn std::fmt::Debug,
    t: &LendingTest,
    seeds: &[i128; 3],
) -> Result<(), TestCaseError> {
    for (asset, seed) in ASSETS.iter().zip(seeds) {
        let cash = raw_pool_state(t, asset).cash;
        let held = pool_token_balance(t, asset);
        // Custody: the pool holds exactly its accounting cash.
        prop_assert_eq!(
            held,
            cash,
            "step {} {:?}: {} pool token balance {} != accounting cash {}",
            step,
            op,
            asset,
            held,
            cash
        );

        let surplus = seed_adjusted_surplus_ray(t, asset, *seed);
        let unit = unit_ray(t, asset);
        let ceiling = MAX_ROUNDING_LEGS_PER_OP * (step as i128 + 1) * unit;
        prop_assert!(
            surplus >= -RAY_DUST,
            "step {} {:?}: {} cash leak: seed-adjusted surplus {} raw RAY ({} units)",
            step,
            op,
            asset,
            surplus,
            surplus / unit
        );
        prop_assert!(
            surplus <= ceiling,
            "step {} {:?}: {} over-collection: surplus {} raw RAY ({} units) > {} units",
            step,
            op,
            asset,
            surplus,
            surplus / unit,
            ceiling / unit
        );
    }
    Ok(())
}

static MAX_SURPLUS_UNITS: AtomicI64 = AtomicI64::new(0);

proptest! {
    #![proptest_config(ProptestConfig { failure_persistence: None, ..config(32) })]

    #[test]
    fn prop_seed_adjusted_cash_conservation_and_token_custody(
        ops in prop::collection::vec(op_strategy(), 5..15)
    ) {
        let mut t = LendingTest::new().three_asset_usdc_eth_wbtc().build();

        // Straight after build a market holds only its seed: no shares, no debt.
        let presets = [usdc_preset(), eth_preset(), wbtc_preset()];
        let mut seeds = [0i128; 3];
        for (i, asset) in ASSETS.iter().enumerate() {
            let s = raw_pool_state(&t, asset);
            prop_assert_eq!((s.supplied, s.borrowed, s.revenue), (0, 0, 0));
            let from_preset =
                f64_to_i128(presets[i].initial_liquidity, t.resolve_market(asset).decimals);
            prop_assert_eq!(s.cash, from_preset, "{} seed differs from its preset", asset);
            seeds[i] = s.cash;
        }

        seed_fuzz_conservation_book(&mut t);

        for (i, op) in ops.iter().enumerate() {
            execute_op(&mut t, op);
            // `i + 1`: the seeded book is six pool legs of its own.
            assert_cash_conservation_and_custody(i + 1, op, &t, &seeds)?;
        }

        for (asset, seed) in ASSETS.iter().zip(&seeds) {
            let units = (seed_adjusted_surplus_ray(&t, asset, *seed) / unit_ray(&t, asset)) as i64;
            if units > MAX_SURPLUS_UNITS.fetch_max(units, Ordering::Relaxed) {
                eprintln!("new max seed-adjusted surplus: {units} raw units of {asset}");
            }
        }
    }
}

// The generic op set never lands a liquidation; this property forces one.
static LIQUIDATIONS_LANDED: AtomicI64 = AtomicI64::new(0);
static BAD_DEBT_EVENTS: AtomicI64 = AtomicI64::new(0);

fn market_seeds(t: &LendingTest) -> [i128; 3] {
    let mut seeds = [0i128; 3];
    for (i, asset) in ASSETS.iter().enumerate() {
        let s = raw_pool_state(t, asset);
        assert_eq!((s.supplied, s.borrowed, s.revenue), (0, 0, 0));
        seeds[i] = s.cash;
    }
    seeds
}

proptest! {
    #![proptest_config(ProptestConfig { failure_persistence: None, ..config(32) })]

    #[test]
    fn prop_seed_adjusted_cash_conservation_through_liquidation_and_bad_debt(
        collateral_usdc in 10u32..50_000u32,
        borrow_bps in 3_000u32..7_400u32,
        wait_secs in 0u32..(30 * 24 * 3600),
        crash_cents in 3u32..70u32,
        liquidation_bps in prop::collection::vec(100u16..=10_000u16, 1..4),
    ) {
        let mut t = LendingTest::new().three_asset_usdc_eth_wbtc().build();
        let seeds = market_seeds(&t);
        let mut step = 0usize;

        // Real ETH suppliers, so socialized bad debt has claims to land on.
        t.supply(BOB, "ETH", 100.0);
        t.supply(ALICE, "USDC", collateral_usdc as f64);
        let debt_eth = collateral_usdc as f64 * borrow_bps as f64 / 10_000.0 / 2_000.0;
        prop_assume!(t.try_borrow(ALICE, "ETH", debt_eth).is_ok());
        assert_cash_conservation_and_custody(step, &"open", &t, &seeds)?;

        t.advance_and_sync(wait_secs as u64);
        step += 1;
        assert_cash_conservation_and_custody(step, &"accrue", &t, &seeds)?;

        t.set_price("USDC", controller::constants::WAD * crash_cents as i128 / 100);
        for bps in &liquidation_bps {
            let debt = t.borrow_balance(ALICE, "ETH");
            if debt <= 0.0 {
                break;
            }
            let index_before = raw_pool_state(&t, "ETH").supply_index;
            let repay = (debt * *bps as f64 / 10_000.0).max(0.000_001);
            let landed = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", repay).is_ok();
            step += 1;
            assert_cash_conservation_and_custody(step, &("liquidate", bps, landed), &t, &seeds)?;
            if landed {
                LIQUIDATIONS_LANDED.fetch_add(1, Ordering::Relaxed);
            }
            if raw_pool_state(&t, "ETH").supply_index < index_before {
                BAD_DEBT_EVENTS.fetch_add(1, Ordering::Relaxed);
            }
        }

        for asset in ["USDC", "ETH"] {
            let _ = t.try_claim_revenue(asset);
            step += 1;
            assert_cash_conservation_and_custody(step, &("claim", asset), &t, &seeds)?;
        }
        if std::env::var_os("CONSERVATION_STATS").is_some() {
            eprintln!(
                "liquidations landed so far: {}, bad-debt socializations so far: {}",
                LIQUIDATIONS_LANDED.load(Ordering::Relaxed),
                BAD_DEBT_EVENTS.load(Ordering::Relaxed)
            );
        }
    }
}
