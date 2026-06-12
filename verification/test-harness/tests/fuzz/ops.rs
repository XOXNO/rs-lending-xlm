//! Shared operation alphabet for accounting conservation properties.

use proptest::prelude::*;
use soroban_sdk::Vec;
use test_harness::{LendingTest, ALICE, BOB};

pub const ASSETS: [&str; 3] = ["USDC", "ETH", "WBTC"];
pub const USERS: [&str; 2] = [ALICE, BOB];

#[derive(Clone, Debug)]
pub enum LendingOp {
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

pub fn op_strategy() -> impl Strategy<Value = LendingOp> {
    prop_oneof![
        4 => (user_strat(), asset_strat(), 1u32..20_000u32)
            .prop_map(|(u, a, amt)| LendingOp::Supply { user: u, asset: a, amt }),
        2 => (user_strat(), asset_strat(), 1u16..10_000u16)
            .prop_map(|(u, a, f)| LendingOp::Repay { user: u, asset: a, frac_bps: f }),
        2 => (60u32..(3 * 24 * 3600)).prop_map(|s| LendingOp::Advance { secs: s }),
        1 => (user_strat(), asset_strat(), 1u16..10_000u16)
            .prop_map(|(u, a, f)| LendingOp::Withdraw { user: u, asset: a, frac_bps: f }),
        1 => (user_strat(), asset_strat(), 1u32..100u32)
            .prop_map(|(u, a, amt)| LendingOp::Borrow { user: u, asset: a, amt }),
        1 => asset_strat().prop_map(|a| LendingOp::ClaimRevenue { asset: a }),
    ]
}

pub fn execute_op(t: &mut LendingTest, op: &LendingOp) {
    match op {
        LendingOp::Supply { user, asset, amt } => {
            let _ = t.try_supply(user, asset, *amt as f64);
        }
        LendingOp::Borrow { user, asset, amt } => {
            let _ = t.try_borrow(user, asset, *amt as f64 * 0.01);
        }
        LendingOp::Repay {
            user,
            asset,
            frac_bps,
        } => {
            let bal = t.borrow_balance(user, asset);
            if bal > 0.0001 {
                let a = bal * *frac_bps as f64 / 10_000.0;
                let _ = t.try_repay(user, asset, a);
            }
        }
        LendingOp::Withdraw {
            user,
            asset,
            frac_bps,
        } => {
            let bal = t.supply_balance(user, asset);
            if bal > 0.0001 {
                let a = bal * *frac_bps as f64 / 10_000.0;
                let _ = t.try_withdraw(user, asset, a);
            }
        }
        LendingOp::Advance { secs } => {
            t.advance_and_sync(*secs as u64);
        }
        LendingOp::ClaimRevenue { asset } => {
            let _ = t.try_claim_revenue(asset);
        }
    }
}

pub fn capture_indexes(t: &LendingTest) -> [(i128, i128); 3] {
    let mut out = [(0i128, 0i128); 3];
    for (i, asset) in ASSETS.iter().enumerate() {
        let mut assets = Vec::new(&t.env);
        assets.push_back(t.resolve_asset(asset));
        let v = t
            .ctrl_client()
            .get_all_market_indexes_detailed(&assets)
            .get(0)
            .unwrap();
        out[i] = (v.supply_index_ray, v.borrow_index_ray);
    }
    out
}
