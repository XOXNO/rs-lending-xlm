use common::types::{
    AccountPositionRaw, ControllerKey, DebtPositionRaw, HubAssetKey, PoolStateRaw, SeizeMode,
    SpokeUsageRaw,
};
use script_runner::{BorrowOp, LiquidateOp, Op, RepayOp, SupplyOp, WithdrawOp};
use soroban_sdk::{vec, Address, Map};
use test_harness::{hub_asset, LendingTest, HARNESS_SPOKE};

pub fn key(t: &LendingTest, asset: &str) -> HubAssetKey {
    hub_asset(t.resolve_asset(asset))
}

pub fn supply_op(t: &LendingTest, account_id: u64, asset: &str, amount: i128) -> Op {
    Op::Supply(SupplyOp {
        account_id,
        spoke_id: HARNESS_SPOKE,
        assets: vec![&t.env, (key(t, asset), amount)],
    })
}

pub fn borrow_op(
    t: &LendingTest,
    account_id: u64,
    asset: &str,
    amount: i128,
    to: Option<Address>,
) -> Op {
    Op::Borrow(BorrowOp {
        account_id,
        borrows: vec![&t.env, (key(t, asset), amount)],
        to,
    })
}

pub fn withdraw_op(
    t: &LendingTest,
    account_id: u64,
    asset: &str,
    amount: i128,
    to: Option<Address>,
) -> Op {
    Op::Withdraw(WithdrawOp {
        account_id,
        withdrawals: vec![&t.env, (key(t, asset), amount)],
        to,
    })
}

pub fn repay_op(t: &LendingTest, account_id: u64, asset: &str, amount: i128) -> Op {
    Op::Repay(RepayOp {
        account_id,
        payments: vec![&t.env, (key(t, asset), amount)],
    })
}

pub fn liquidate_op(
    t: &LendingTest,
    account_id: u64,
    asset: &str,
    amount: i128,
    seize_mode: SeizeMode,
) -> Op {
    Op::Liquidate(LiquidateOp {
        account_id,
        payments: vec![&t.env, (key(t, asset), amount)],
        seize_mode,
    })
}

/// Everything a script could touch, captured per asset and per account. Two
/// snapshots compare equal only when nothing moved.
#[derive(Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub markets: std::vec::Vec<(String, i128, i128, i128, i128, i128, i128)>,
    pub usage: std::vec::Vec<(String, i128, i128)>,
    pub wallets: std::vec::Vec<(String, i128)>,
    pub accounts: std::vec::Vec<(u64, bool, u32, u32)>,
}

impl Snapshot {
    pub fn take(t: &LendingTest, runner: &Address, assets: &[&str], accounts: &[u64]) -> Snapshot {
        let mut markets = std::vec::Vec::new();
        let mut usage = std::vec::Vec::new();
        let mut wallets = std::vec::Vec::new();
        for asset in assets {
            let k = key(t, asset);
            let s: PoolStateRaw = t.pool_client(asset).get_sync_data(&k).state;
            markets.push((
                asset.to_string(),
                s.supplied,
                s.borrowed,
                s.revenue,
                s.borrow_index,
                s.supply_index,
                s.cash,
            ));
            let u = t.env.as_contract(&t.controller, || {
                t.env
                    .storage()
                    .persistent()
                    .get::<_, SpokeUsageRaw>(&ControllerKey::SpokeUsage(HARNESS_SPOKE, k.clone()))
                    .unwrap_or_default()
            });
            usage.push((
                asset.to_string(),
                u.supplied_scaled_ray,
                u.borrowed_scaled_ray,
            ));
            wallets.push((asset.to_string(), t.runner_wallet(runner, asset)));
        }
        let mut accs = std::vec::Vec::new();
        for id in accounts {
            let exists = t.account_exists(*id);
            let (s, b): (
                Map<HubAssetKey, AccountPositionRaw>,
                Map<HubAssetKey, DebtPositionRaw>,
            ) = t.ctrl_client().get_account_positions(id);
            accs.push((*id, exists, s.len(), b.len()));
        }
        Snapshot {
            markets,
            usage,
            wallets,
            accounts: accs,
        }
    }
}
