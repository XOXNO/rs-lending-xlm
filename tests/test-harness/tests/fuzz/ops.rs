use controller::types::PositionMode;
use proptest::prelude::*;
use script_runner::{BorrowOp, Op, RepayOp, SupplyOp, WithdrawOp, LAST_CREATED};
use soroban_sdk::xdr::ToXdr;
use soroban_sdk::{Address, Vec};
use test_harness::{
    apply_flash_fee, build_aggregator_swap, f64_to_i128, hub_asset, FlashPositionMode,
    FlashPositionRequest, LendingTest, ALICE, BOB, CAROL, HARNESS_SPOKE, LIQUIDATOR,
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
    /// LIQUIDATOR donates `amt` units; only the measured shortfall is applied
    /// and the rest is refunded, so no position appears anywhere.
    Recapitalize {
        asset: &'static str,
        amt: u32,
    },
    /// Keeper restamp of the user's default account.
    UpdateAccountThreshold {
        user: &'static str,
        has_risks: bool,
    },
    /// CAROL, an active position manager, is granted on the user's account.
    AddDelegate {
        user: &'static str,
    },
    RemoveDelegate {
        user: &'static str,
    },
    /// The user's default account NFT moves to the other tracked user, and
    /// the harness bookkeeping follows it, so the conservation sums move too.
    NftTransfer {
        from: &'static str,
    },
    /// A script-runner contract opens an account with `open_amt` of `asset`,
    /// runs `legs`, then repays every borrowed asset and withdraws every
    /// supplied one, all in one invocation. Either every leg commits and the
    /// runner ends with no position, or nothing does.
    Script {
        asset: &'static str,
        open_amt: u32,
        legs: std::vec::Vec<ScriptLeg>,
    },
}

/// One leg of a `LendingOp::Script`, always addressed to the account the
/// script opened.
#[derive(Clone, Debug)]
pub enum ScriptLeg {
    Supply {
        asset: &'static str,
        amt: u32,
    },
    /// `amt` hundredths of a unit.
    Borrow {
        asset: &'static str,
        amt: u32,
    },
    /// `amt` hundredths of a unit; an overpayment is refunded.
    Repay {
        asset: &'static str,
        amt: u32,
    },
    /// `amt` units; zero means the whole position.
    Withdraw {
        asset: &'static str,
        amt: u32,
    },
}

fn leg_strat() -> impl Strategy<Value = ScriptLeg> {
    prop_oneof![
        (asset_strat(), 1u32..2_000u32).prop_map(|(a, amt)| ScriptLeg::Supply { asset: a, amt }),
        (asset_strat(), 1u32..50u32).prop_map(|(a, amt)| ScriptLeg::Borrow { asset: a, amt }),
        (asset_strat(), 1u32..60u32).prop_map(|(a, amt)| ScriptLeg::Repay { asset: a, amt }),
        (asset_strat(), 0u32..500u32).prop_map(|(a, amt)| ScriptLeg::Withdraw { asset: a, amt }),
    ]
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
        1 => (asset_strat(), 1u32..5_000u32)
            .prop_map(|(a, amt)| LendingOp::Recapitalize { asset: a, amt }),
        1 => (user_strat(), any::<bool>())
            .prop_map(|(u, r)| LendingOp::UpdateAccountThreshold { user: u, has_risks: r }),
        1 => user_strat().prop_map(|u| LendingOp::AddDelegate { user: u }),
        1 => user_strat().prop_map(|u| LendingOp::RemoveDelegate { user: u }),
        1 => user_strat().prop_map(|u| LendingOp::NftTransfer { from: u }),
        2 => (asset_strat(), 100u32..5_000u32, prop::collection::vec(leg_strat(), 2..6))
            .prop_map(|(a, open, legs)| LendingOp::Script { asset: a, open_amt: open, legs }),
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
        LendingOp::Recapitalize { asset, amt } => {
            let raw = f64_to_i128(*amt as f64, t.resolve_market(asset).decimals);
            let payer = t.get_or_create_user(LIQUIDATOR);
            t.resolve_market(asset).token_admin.mint(&payer, &raw);
            let key = hub_asset(t.resolve_asset(asset));
            let _ = t.ctrl_client().try_recapitalize(&payer, &key, &raw);
        }
        LendingOp::UpdateAccountThreshold { user, has_risks } => {
            if let Some(id) = t.find_account_id(user) {
                let _ = t.try_update_account_threshold(*has_risks, &[id]);
            }
        }
        LendingOp::AddDelegate { user } => {
            let Some(id) = t.find_account_id(user) else {
                return;
            };
            let owner = t.get_or_create_user(user);
            let delegate = t.get_or_create_user(CAROL);
            t.ctrl_client().set_position_manager(&delegate, &true);
            let _ = t.ctrl_client().try_add_delegate(&owner, &id, &delegate);
        }
        LendingOp::RemoveDelegate { user } => {
            let Some(id) = t.find_account_id(user) else {
                return;
            };
            let owner = t.get_or_create_user(user);
            let delegate = t.get_or_create_user(CAROL);
            let _ = t.ctrl_client().try_remove_delegate(&owner, &id, &delegate);
        }
        LendingOp::NftTransfer { from } => {
            let to = if *from == ALICE { BOB } else { ALICE };
            let Some(id) = t.find_account_id(from) else {
                return;
            };
            let attrs = t.ctrl_client().get_account_attributes(&id);
            t.nft_transfer(from, to, id);
            t.adopt_account(to, id, attrs.spoke_id, attrs.mode);
            // Every tracked user keeps a default account so the conservation
            // sums can always resolve one.
            if t.find_account_id(from).is_none() {
                t.create_account(from);
            }
        }
        LendingOp::Script {
            asset,
            open_amt,
            legs,
        } => {
            run_script_op(t, asset, *open_amt, legs);
        }
    }
}

fn units(t: &LendingTest, asset: &str, amount: f64) -> i128 {
    f64_to_i128(amount, t.resolve_market(asset).decimals)
}

/// Builds and runs the script. Borrowed assets are repaid with double the
/// borrowed total, supplied assets are withdrawn in full, so a committed
/// script leaves the runner with no position and never a richer wallet.
fn run_script_op(t: &mut LendingTest, asset: &str, open_amt: u32, legs: &[ScriptLeg]) {
    let runner = t.deploy_script_runner();
    for a in ASSETS {
        t.fund_runner(&runner, a, units(t, a, 100_000.0));
    }
    let before: std::vec::Vec<i128> = ASSETS.iter().map(|a| t.runner_wallet(&runner, a)).collect();
    let key = |t: &LendingTest, a: &str| hub_asset(t.resolve_asset(a));

    let mut ops: Vec<Op> = Vec::new(&t.env);
    let mut supplied: std::vec::Vec<&'static str> = vec![];
    let mut borrowed: std::vec::Vec<(&'static str, f64)> = vec![];
    let open_asset = ASSETS
        .iter()
        .copied()
        .find(|a| *a == asset)
        .unwrap_or("USDC");
    supplied.push(open_asset);
    ops.push_back(Op::Supply(SupplyOp {
        account_id: 0,
        spoke_id: HARNESS_SPOKE,
        assets: soroban_sdk::vec![&t.env, (key(t, asset), units(t, asset, open_amt as f64))],
    }));
    for leg in legs {
        match leg {
            ScriptLeg::Supply { asset, amt } => {
                if !supplied.contains(asset) {
                    supplied.push(asset);
                }
                ops.push_back(Op::Supply(SupplyOp {
                    account_id: LAST_CREATED,
                    spoke_id: HARNESS_SPOKE,
                    assets: soroban_sdk::vec![
                        &t.env,
                        (key(t, asset), units(t, asset, *amt as f64))
                    ],
                }));
            }
            ScriptLeg::Borrow { asset, amt } => {
                let amount = *amt as f64 * 0.01;
                match borrowed.iter_mut().find(|(a, _)| a == asset) {
                    Some(entry) => entry.1 += amount,
                    None => borrowed.push((asset, amount)),
                }
                ops.push_back(Op::Borrow(BorrowOp {
                    account_id: LAST_CREATED,
                    borrows: soroban_sdk::vec![&t.env, (key(t, asset), units(t, asset, amount))],
                    to: None,
                }));
            }
            ScriptLeg::Repay { asset, amt } => {
                ops.push_back(Op::Repay(RepayOp {
                    account_id: LAST_CREATED,
                    payments: soroban_sdk::vec![
                        &t.env,
                        (key(t, asset), units(t, asset, *amt as f64 * 0.01))
                    ],
                }));
            }
            ScriptLeg::Withdraw { asset, amt } => {
                ops.push_back(Op::Withdraw(WithdrawOp {
                    account_id: LAST_CREATED,
                    withdrawals: soroban_sdk::vec![
                        &t.env,
                        (key(t, asset), units(t, asset, *amt as f64))
                    ],
                    to: None,
                }));
            }
        }
    }
    for (asset, total) in &borrowed {
        ops.push_back(Op::Repay(RepayOp {
            account_id: LAST_CREATED,
            payments: soroban_sdk::vec![&t.env, (key(t, asset), units(t, asset, total * 2.0) + 1)],
        }));
    }
    for asset in &supplied {
        ops.push_back(Op::Withdraw(WithdrawOp {
            account_id: LAST_CREATED,
            withdrawals: soroban_sdk::vec![&t.env, (key(t, asset), 0)],
            to: None,
        }));
    }
    let _ = t.run_script(&runner, &ops);
    for (i, a) in ASSETS.iter().enumerate() {
        let after = t.runner_wallet(&runner, a);
        assert!(
            after <= before[i],
            "script extracted {} raw of {a}: {:?}",
            after - before[i],
            legs
        );
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
