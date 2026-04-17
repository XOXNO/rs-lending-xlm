//! Contract-level property test: accounting conservation.
//!
//! After every op in a random sequence (supply / borrow / repay / withdraw /
//! advance_time / claim_revenue), assert pool-level accounting conservation
//! laws drawn from INVARIANTS.md §4 (Pool State Identity), §5 (Interest
//! Split), and §12 (Claim Revenue).
//!
//! Laws (per asset X):
//!
//! 1. Pool solvency identity (reserves ≥ supplied − borrowed):
//!    `pool_reserves(X) + total_borrowed(X) ≥ total_supplied(X)`. The
//!    pool's token balance must cover every supplier's withdrawable claim.
//!    Donations or seed liquidity can push reserves above, but never below,
//!    `supplied − borrowed`.
//!
//! 2. Borrow conservation (user-aggregate ≈ pool-total):
//!    `Σ user_borrow_balance(X) ≈ total_borrowed(X)` within a small
//!    per-user rounding tolerance (1 asset-decimal unit each).
//!
//! 3. Supply conservation (user-aggregate ≤ pool-total minus revenue):
//!    `Σ user_supply_balance(X) ≈ total_supplied(X) − protocol_revenue(X)`.
//!    Protocol revenue lives inside `supplied_ray` but belongs to no user.
//!    (architecture/INVARIANTS.md §4: `0 ≤ revenue_ray ≤ supplied_ray`.)
//!
//! 4. Reserves non-negative (strict):
//!    `pool_reserves(X) ≥ 0` (no −0.0001 slack).
//!
//! The op distribution deliberately tilts toward supply over borrow, to
//! counter the bias Codex noted in `fuzz_multi_asset_solvency.rs`.

use proptest::prelude::*;
use test_harness::{eth_preset, usdc_preset, wbtc_preset, LendingTest, ALICE, BOB};

const ASSETS: [&str; 3] = ["USDC", "ETH", "WBTC"];
const USERS: [&str; 2] = [ALICE, BOB];

#[derive(Clone, Debug)]
enum Op {
    Supply {
        user: &'static str,
        asset: &'static str,
        amt: u32,
    },
    Borrow {
        user: &'static str,
        asset: &'static str,
        amt: u32,
    },
    Repay {
        user: &'static str,
        asset: &'static str,
        frac_bps: u16,
    },
    Withdraw {
        user: &'static str,
        asset: &'static str,
        frac_bps: u16,
    },
    Advance {
        secs: u32,
    },
    ClaimRevenue {
        asset: &'static str,
    },
}

fn user_strat() -> impl Strategy<Value = &'static str> {
    prop_oneof![Just(ALICE), Just(BOB)]
}

fn asset_strat() -> impl Strategy<Value = &'static str> {
    prop_oneof![Just("USDC"), Just("ETH"), Just("WBTC")]
}

fn op_strategy() -> impl Strategy<Value = Op> {
    // Weights favor supply + repay + advance over borrow/withdraw to widen
    // conservation coverage. Codex noted that fuzz_multi_asset_solvency was
    // borrow-heavy and produced few successful operations.
    prop_oneof![
        // 4x supply — should dominate
        4 => (user_strat(), asset_strat(), 1u32..20_000u32)
            .prop_map(|(u, a, amt)| Op::Supply { user: u, asset: a, amt }),
        // 2x repay (proportion of existing debt)
        2 => (user_strat(), asset_strat(), 1u16..10_000u16)
            .prop_map(|(u, a, f)| Op::Repay { user: u, asset: a, frac_bps: f }),
        // 2x advance (interest accrual = a meaningful class of state change)
        2 => (60u32..(3 * 24 * 3600)).prop_map(|s| Op::Advance { secs: s }),
        // 1x withdraw
        1 => (user_strat(), asset_strat(), 1u16..10_000u16)
            .prop_map(|(u, a, f)| Op::Withdraw { user: u, asset: a, frac_bps: f }),
        // 1x borrow (small amounts so they succeed against seeded collateral)
        1 => (user_strat(), asset_strat(), 1u32..100u32)
            .prop_map(|(u, a, amt)| Op::Borrow { user: u, asset: a, amt }),
        // 1x claim_revenue (admin)
        1 => asset_strat().prop_map(|a| Op::ClaimRevenue { asset: a }),
    ]
}

/// Tolerance in asset-decimal units: rounding error reaches up to 1 unit per
/// user per side of the sum, plus 1 unit for the pool-side scaling.
///
/// With 2 users on both sides (supply balances on LHS, borrow balances on LHS
/// and RHS), the budget is ≤ 4 units.
const TOLERANCE_UNITS: i128 = 4;

fn sum_supply(t: &LendingTest, asset: &str) -> i128 {
    USERS.iter().map(|u| t.supply_balance_raw(u, asset)).sum()
}

fn sum_borrow(t: &LendingTest, asset: &str) -> i128 {
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

fn snapshot(t: &LendingTest, asset: &str) -> PoolSnapshot {
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

proptest! {
    #![proptest_config(ProptestConfig { cases: 32, ..ProptestConfig::default() })]

    #[test]
    fn prop_accounting_conservation(ops in prop::collection::vec(op_strategy(), 5..15)) {
        let mut t = LendingTest::new()
            .with_market(usdc_preset())
            .with_market(eth_preset())
            .with_market(wbtc_preset())
            .build();

        // Seed both users so the state is non-trivial from step 1.

        t.supply(ALICE, "USDC", 50_000.0);
        t.supply(BOB, "USDC", 50_000.0);
        t.supply(ALICE, "ETH", 20.0);
        t.supply(BOB, "WBTC", 1.0);

        for (i, op) in ops.iter().enumerate() {
            match op.clone() {
                Op::Supply { user, asset, amt } => {
                    let _ = t.try_supply(user, asset, amt as f64);
                }
                Op::Borrow { user, asset, amt } => {
                    let _ = t.try_borrow(user, asset, amt as f64 * 0.01);
                }
                Op::Repay { user, asset, frac_bps } => {
                    let bal = t.borrow_balance(user, asset);
                    if bal > 0.0001 {
                        let a = bal * frac_bps as f64 / 10_000.0;
                        let _ = t.try_repay(user, asset, a);
                    }
                }
                Op::Withdraw { user, asset, frac_bps } => {
                    let bal = t.supply_balance(user, asset);
                    if bal > 0.0001 {
                        let a = bal * frac_bps as f64 / 10_000.0;
                        let _ = t.try_withdraw(user, asset, a);
                    }
                }
                Op::Advance { secs } => {
                    t.advance_and_sync(secs as u64);
                }
                Op::ClaimRevenue { asset } => {
                    let _ = t.try_claim_revenue(asset);
                }
            }

            // Check all conservation laws per asset.
            for asset in &ASSETS {
                let s = snapshot(&t, asset);

                // Law 4: reserves ≥ 0 (strict, no slack).
                prop_assert!(
                    s.reserves >= 0,
                    "step {} op {:?}: {} reserves < 0: {}",
                    i, op, asset, s.reserves
                );

                // Law 3: revenue bounded by supply (INVARIANTS §4).
                prop_assert!(
                    s.revenue <= s.supplied + TOLERANCE_UNITS,
                    "step {} op {:?}: {} revenue ({}) exceeds supplied ({})",
                    i, op, asset, s.revenue, s.supplied
                );

                // Law 2: borrow conservation.
                let borrow_diff = (s.sum_user_borrow - s.borrowed).abs();
                prop_assert!(
                    borrow_diff <= TOLERANCE_UNITS,
                    "step {} op {:?}: {} borrow mismatch \
                     Σuser({}) vs pool({}), diff {} > {}",
                    i, op, asset, s.sum_user_borrow, s.borrowed,
                    borrow_diff, TOLERANCE_UNITS
                );

                // Law 1: solvency — the pool's token balance always covers
                // every supplier's claim: reserves + borrowed ≥ supplied.
                // Donations or bootstrap liquidity make this an inequality
                // rather than equality.
                let solvency_slack = s.reserves + s.borrowed - s.supplied;
                prop_assert!(
                    solvency_slack >= -TOLERANCE_UNITS,
                    "step {} op {:?}: {} solvency violated \
                     reserves({}) + borrowed({}) < supplied({}) by {}",
                    i, op, asset,
                    s.reserves, s.borrowed, s.supplied, -solvency_slack
                );

                // Law 3: supply conservation.
                // supplied = Σuser_supply + revenue (up to rounding).
                let supply_conservation_diff =
                    (s.supplied - s.sum_user_supply - s.revenue).abs();
                prop_assert!(
                    supply_conservation_diff <= TOLERANCE_UNITS,
                    "step {} op {:?}: {} supply conservation violated \
                     pool_supplied({}) - Σuser({}) - revenue({}) = {} > {}",
                    i, op, asset,
                    s.supplied, s.sum_user_supply, s.revenue,
                    supply_conservation_diff, TOLERANCE_UNITS
                );
            }
        }
    }
}
