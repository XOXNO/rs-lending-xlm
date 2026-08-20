use controller::types::PositionMode;
use proptest::prelude::*;
use soroban_sdk::xdr::ToXdr;
use soroban_sdk::{Address, Vec};
use test_harness::{
    apply_flash_fee, build_aggregator_swap, f64_to_i128, hub_asset, FlashPositionMode,
    FlashPositionRequest, LendingTest, ALICE, BOB, HARNESS_SPOKE, LIQUIDATOR,
};

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
    Liquidate {
        user: &'static str,
        asset: &'static str,
        frac_bps: u16,
    },
    SwapDebt {
        user: &'static str,
        new_debt_amt: u32,
    },
    FlashLoan {
        user: &'static str,
        asset: &'static str,
        amt: u32,
    },
    FlashPosition {
        user: &'static str,
    },
    Multiply {
        user: &'static str,
        debt_amt: u32,
    },
    SwapCollateral {
        user: &'static str,
        amt: u32,
    },
    Rdwc {
        user: &'static str,
        coll_amt: u32,
    },
    /// Merge a healthy same-asset USDC Blend mock position into the user's
    /// already-tracked default account. `debt_amt == 0` is coll-only.
    MigrateBlend {
        user: &'static str,
        debt_amt: u32,
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
        1 => (user_strat(), prop_oneof![Just("ETH"), Just("WBTC")], 100u16..5_000u16)
            .prop_map(|(u, a, f)| LendingOp::Liquidate { user: u, asset: a, frac_bps: f }),
        1 => (user_strat(), 1u32..80u32)
            .prop_map(|(u, amt)| LendingOp::SwapDebt { user: u, new_debt_amt: amt }),
        1 => (user_strat(), asset_strat(), 1u32..500u32)
            .prop_map(|(u, a, amt)| LendingOp::FlashLoan { user: u, asset: a, amt }),
        1 => user_strat().prop_map(|u| LendingOp::FlashPosition { user: u }),
        1 => (user_strat(), 1u32..5u32)
            .prop_map(|(u, amt)| LendingOp::Multiply { user: u, debt_amt: amt }),
        1 => (user_strat(), 1u32..50u32)
            .prop_map(|(u, amt)| LendingOp::SwapCollateral { user: u, amt }),
        1 => (user_strat(), 1u32..50u32)
            .prop_map(|(u, amt)| LendingOp::Rdwc { user: u, coll_amt: amt }),
        1 => (user_strat(), 0u32..400u32)
            .prop_map(|(u, amt)| LendingOp::MigrateBlend { user: u, debt_amt: amt }),
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
        LendingOp::Liquidate {
            user,
            asset,
            frac_bps,
        } => {
            if t.borrow_balance(user, asset) <= 0.0001 {
                return;
            }
            t.set_price("USDC", controller::constants::WAD / 2);
            let bal = t.borrow_balance(user, asset);
            let repay = (bal * *frac_bps as f64 / 10_000.0).max(0.01);
            let _ = t.try_liquidate(LIQUIDATOR, user, asset, repay);
        }
        LendingOp::SwapDebt { user, new_debt_amt } => {
            if t.borrow_balance(user, "USDC") < 10.0 {
                return;
            }
            t.fund_router("USDC", 50.0);
            let steps = t.mock_swap_steps("ETH", "USDC", controller::constants::WAD * 2_000);
            let _ = t.try_swap_debt(user, "USDC", *new_debt_amt as f64 * 0.1, "ETH", &steps);
        }
        LendingOp::FlashLoan { user, asset, amt } => {
            let receiver = t.deploy_flash_loan_receiver();
            let _ = t.try_flash_loan(user, asset, *amt as f64, &receiver);
        }
        LendingOp::FlashPosition { user } => {
            try_flash_position_op(t, user);
        }
        LendingOp::Multiply { user, debt_amt } => {
            let debt = (*debt_amt as f64) * 0.1;
            let usdc_out = debt * 3_000.0;
            t.fund_router("USDC", usdc_out);
            let eth_raw = f64_to_i128(debt, t.resolve_market("ETH").decimals);
            let usdc_raw = f64_to_i128(usdc_out, t.resolve_market("USDC").decimals);
            let steps = build_aggregator_swap(t, "ETH", "USDC", apply_flash_fee(eth_raw), usdc_raw);
            let _ = t.try_multiply(user, "USDC", debt, "ETH", PositionMode::Multiply, &steps);
        }
        LendingOp::SwapCollateral { user, amt } => {
            if t.supply_balance(user, "USDC") < 10.0 {
                return;
            }
            t.fund_router("ETH", 1.0);
            let steps = t.mock_swap_steps("USDC", "ETH", controller::constants::WAD);
            let _ = t.try_swap_collateral(user, "USDC", *amt as f64, "ETH", &steps);
        }
        LendingOp::Rdwc { user, coll_amt } => {
            if t.supply_balance(user, "USDC") < 10.0 || t.borrow_balance(user, "ETH") < 0.01 {
                return;
            }
            t.fund_router("ETH", 1.0);
            let steps = t.mock_swap_steps("USDC", "ETH", controller::constants::WAD);
            let _ = t.try_repay_debt_with_collateral(
                user,
                "USDC",
                *coll_amt as f64,
                "ETH",
                &steps,
                false,
            );
        }
        LendingOp::MigrateBlend { user, debt_amt } => {
            try_migrate_blend_op(t, user, *debt_amt);
        }
    }
}

fn try_migrate_blend_op(t: &mut LendingTest, user: &str, debt_amt: u32) {
    let Some(account_id) = t.find_account_id(user) else {
        return;
    };
    t.seed_blend(
        user,
        "USDC",
        test_harness::mock_blend::KIND_COLLATERAL,
        2_000.0,
    );
    let debt_caps = if debt_amt > 0 {
        let debt = (debt_amt as f64).min(400.0);
        t.seed_blend(user, "USDC", test_harness::mock_blend::KIND_LIABILITY, debt);
        vec![("USDC", debt * 1.2)]
    } else {
        vec![]
    };
    let _ = t.try_migrate_from_blend(user, account_id, &["USDC"], &[], &debt_caps);
}

fn try_flash_position_op(t: &mut LendingTest, user: &str) {
    let receiver = t.deploy_flash_position_receiver();
    let coll_raw = f64_to_i128(4_000.0, t.resolve_market("USDC").decimals);
    let payload = FlashPositionRequest {
        mode: FlashPositionMode::Success,
        collateral: t.resolve_asset("USDC"),
        collateral_amount: coll_raw,
        extra_asset: t.resolve_asset("ETH"),
        extra_amount: 0,
        reenter_spoke_id: HARNESS_SPOKE,
        reenter_account_id: 0,
    }
    .to_xdr(&t.env);
    let mut mins: Vec<(controller::types::HubAssetKey, i128)> = Vec::new(&t.env);
    mins.push_back((hub_asset(t.resolve_asset("USDC")), coll_raw));
    let refunds: Vec<Address> = Vec::new(&t.env);
    let _ = t.try_flash_position(
        user,
        0,
        PositionMode::Multiply,
        "ETH",
        1.0,
        &receiver,
        &payload,
        &mins,
        &refunds,
    );
}

pub fn capture_indexes(t: &LendingTest) -> [(i128, i128); 3] {
    let mut out = [(0i128, 0i128); 3];
    for (i, asset) in ASSETS.iter().enumerate() {
        let mut assets = Vec::new(&t.env);
        assets.push_back(hub_asset(t.resolve_asset(asset)));
        let v = t
            .ctrl_client()
            .get_market_indexes_detailed(&assets)
            .get(0)
            .unwrap();
        out[i] = (v.supply_index, v.borrow_index);
    }
    out
}
