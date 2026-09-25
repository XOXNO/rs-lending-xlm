#![no_std]
#![allow(clippy::too_many_arguments)]

//! Test-only script runner. Issues a sequence of controller calls from its own
//! frame, so a test can prove what one attacker contract can do in one
//! transaction: every leg commits or none does. Not for production. It has no
//! callback role; the two receiver mocks cover callback shapes.

use common::token::authorize_transfer_as_current;
use common::types::{HubAssetKey, PaymentTuple, PositionMode, SeizeMode};
use controller_interface::ControllerClient;
use soroban_sdk::auth::{ContractContext, InvokerContractAuthEntry, SubContractInvocation};
use soroban_sdk::{
    contract, contractclient, contractimpl, contracttype, symbol_short, vec, Address, Bytes, Env,
    IntoVal, Vec,
};

/// Sentinel account id: resolves to the account the script last opened
/// (`Supply` or `Multiply` whose account id resolves to `0`) or credited
/// (`Liquidate` in `Credit` mode). Lets one script open an account and act on
/// it.
pub const LAST_CREATED: u64 = u64::MAX;

#[allow(dead_code)]
#[contractclient(name = "NftTransferClient")]
pub trait NftTransfer {
    fn transfer(env: Env, from: Address, to: Address, token_id: u32);
}

#[allow(dead_code)]
#[contractclient(name = "StrategyClient")]
pub trait Strategy {
    fn asset(env: Env) -> Address;
    fn deposit(env: Env, amount: i128, from: Address) -> i128;
    fn harvest(env: Env, from: Address, data: Option<Bytes>);
    fn withdraw(env: Env, amount: i128, from: Address, to: Address) -> i128;
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct StrategyDepositOp {
    pub strategy: Address,
    pub amount: i128,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct StrategyWithdrawOp {
    pub strategy: Address,
    pub amount: i128,
    pub to: Address,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct SupplyOp {
    pub account_id: u64,
    pub spoke_id: u32,
    pub assets: Vec<(HubAssetKey, i128)>,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct BorrowOp {
    pub account_id: u64,
    pub borrows: Vec<(HubAssetKey, i128)>,
    pub to: Option<Address>,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct WithdrawOp {
    pub account_id: u64,
    pub withdrawals: Vec<(HubAssetKey, i128)>,
    pub to: Option<Address>,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct RepayOp {
    pub account_id: u64,
    pub payments: Vec<(HubAssetKey, i128)>,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct LiquidateOp {
    pub account_id: u64,
    pub payments: Vec<(HubAssetKey, i128)>,
    pub seize_mode: SeizeMode,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct FlashLoanOp {
    pub asset: HubAssetKey,
    pub amount: i128,
    pub receiver: Address,
    pub data: Bytes,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct MultiplyOp {
    pub account_id: u64,
    pub spoke_id: u32,
    pub collateral: HubAssetKey,
    pub debt_amount: i128,
    pub debt: HubAssetKey,
    pub mode: PositionMode,
    pub swap: Bytes,
    /// Empty for none; one `(asset, amount)` leg for an initial payment.
    pub initial_payment: Vec<(HubAssetKey, i128)>,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct SwapDebtOp {
    pub account_id: u64,
    pub existing_debt: HubAssetKey,
    pub amount: i128,
    pub new_debt: HubAssetKey,
    pub swap: Bytes,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct SwapCollateralOp {
    pub account_id: u64,
    pub current: HubAssetKey,
    pub amount: i128,
    pub new: HubAssetKey,
    pub swap: Bytes,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct RdwcOp {
    pub account_id: u64,
    pub collateral: HubAssetKey,
    pub collateral_amount: i128,
    pub debt: HubAssetKey,
    pub swap: Bytes,
    pub close_position: bool,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct RecapOp {
    pub hub_asset: HubAssetKey,
    pub amount: i128,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct AssetsOp {
    pub assets: Vec<HubAssetKey>,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct ThresholdOp {
    pub has_risks: bool,
    pub account_ids: Vec<u64>,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct AccountOp {
    pub account_id: u64,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct DelegateOp {
    pub account_id: u64,
    pub delegate: Address,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct NftTransferOp {
    pub to: Address,
    pub token_id: u64,
}

#[contracttype]
#[derive(Clone, Debug)]
pub enum Op {
    Supply(SupplyOp),
    Borrow(BorrowOp),
    Withdraw(WithdrawOp),
    Repay(RepayOp),
    Liquidate(LiquidateOp),
    FlashLoan(FlashLoanOp),
    Multiply(MultiplyOp),
    SwapDebt(SwapDebtOp),
    SwapCollateral(SwapCollateralOp),
    RepayDebtWithCollateral(RdwcOp),
    Recapitalize(RecapOp),
    ClaimRevenue(AssetsOp),
    UpdateIndexes(AssetsOp),
    UpdateAccountThreshold(ThresholdOp),
    CleanBadDebt(AccountOp),
    AddDelegate(DelegateOp),
    RemoveDelegate(DelegateOp),
    RenewAccount(AccountOp),
    NftTransfer(NftTransferOp),
    StrategyDeposit(StrategyDepositOp),
    StrategyHarvest(Address),
    StrategyWithdraw(StrategyWithdrawOp),
}

#[contract]
pub struct ScriptRunner;

#[contractimpl]
impl ScriptRunner {
    /// Pins the external owner of the test vault before it is funded.
    pub fn configure_vault(env: Env, owner: Address) {
        owner.require_auth();
        let key = symbol_short!("v_owner");
        assert!(
            !env.storage().instance().has(&key),
            "vault already configured"
        );
        env.storage().instance().set(&key, &owner);
    }

    /// Runs `ops` in order from this contract's frame. Any failing op reverts
    /// the whole run. Returns the id `LAST_CREATED` resolves to after the last
    /// op, or `0` if no op set it.
    pub fn run(env: Env, controller: Address, nft: Address, ops: Vec<Op>) -> u64 {
        // A configured vault must not expose its funds through legacy ops either.
        let vault_configured = env.storage().instance().has(&symbol_short!("v_owner"));
        if vault_configured {
            require_vault_owner(&env);
        }
        let me = env.current_contract_address();
        let ctrl = ControllerClient::new(&env, &controller);
        let pool = ctrl.get_pool_address();
        let mut last_created = 0u64;
        for op in ops.iter() {
            match op {
                Op::Supply(o) => {
                    authorize_pulls(&env, &me, &pool, &merge_legs(&env, &o.assets));
                    let account_id = resolve(o.account_id, last_created);
                    let id = ctrl.supply(&me, &account_id, &o.spoke_id, &o.assets);
                    if account_id == 0 {
                        last_created = id;
                    }
                }
                Op::Borrow(o) => {
                    ctrl.borrow(&me, &resolve(o.account_id, last_created), &o.borrows, &o.to);
                }
                Op::Withdraw(o) => {
                    let _ = ctrl.withdraw(
                        &me,
                        &resolve(o.account_id, last_created),
                        &o.withdrawals,
                        &o.to,
                    );
                }
                Op::Repay(o) => {
                    authorize_pulls(&env, &me, &pool, &merge_legs(&env, &o.payments));
                    ctrl.repay(&me, &resolve(o.account_id, last_created), &o.payments);
                }
                Op::Liquidate(o) => {
                    let account_id = resolve(o.account_id, last_created);
                    // `Credit(0)` keeps its own meaning: open a fresh account.
                    let seize_mode = match o.seize_mode {
                        SeizeMode::Credit(id) => SeizeMode::Credit(resolve(id, last_created)),
                        mode => mode,
                    };
                    let refunds = ctrl
                        .try_get_liquidation_estimate(&account_id, &o.payments, &seize_mode)
                        .ok()
                        .and_then(Result::ok)
                        .map_or_else(|| Vec::new(&env), |estimate| estimate.refunds);
                    authorize_pulls(
                        &env,
                        &me,
                        &pool,
                        &liquidation_pulls(&env, &o.payments, &refunds),
                    );
                    let id = ctrl.liquidate(&me, &account_id, &o.payments, &seize_mode);
                    if id != 0 {
                        last_created = id;
                    }
                }
                Op::FlashLoan(o) => {
                    ctrl.flash_loan(&me, &o.asset, &o.amount, &o.receiver, &o.data);
                }
                Op::Multiply(o) => {
                    authorize_pulls(&env, &me, &controller, &o.initial_payment);
                    let account_id = resolve(o.account_id, last_created);
                    let id = ctrl.multiply(
                        &me,
                        &account_id,
                        &o.spoke_id,
                        &o.collateral,
                        &o.debt_amount,
                        &o.debt,
                        &o.mode,
                        &o.swap,
                        &o.initial_payment.first(),
                        &None,
                    );
                    if account_id == 0 {
                        last_created = id;
                    }
                }
                Op::SwapDebt(o) => {
                    ctrl.swap_debt(
                        &me,
                        &resolve(o.account_id, last_created),
                        &o.existing_debt,
                        &o.amount,
                        &o.new_debt,
                        &o.swap,
                    );
                }
                Op::SwapCollateral(o) => {
                    ctrl.swap_collateral(
                        &me,
                        &resolve(o.account_id, last_created),
                        &o.current,
                        &o.amount,
                        &o.new,
                        &o.swap,
                    );
                }
                Op::RepayDebtWithCollateral(o) => {
                    ctrl.repay_debt_with_collateral(
                        &me,
                        &resolve(o.account_id, last_created),
                        &o.collateral,
                        &o.collateral_amount,
                        &o.debt,
                        &o.swap,
                        &o.close_position,
                    );
                }
                Op::Recapitalize(o) => {
                    authorize_pulls(
                        &env,
                        &me,
                        &pool,
                        &vec![&env, (o.hub_asset.clone(), o.amount)],
                    );
                    let _ = ctrl.recapitalize(&me, &o.hub_asset, &o.amount);
                }
                Op::ClaimRevenue(o) => {
                    let _ = ctrl.claim_revenue(&me, &o.assets);
                }
                Op::UpdateIndexes(o) => {
                    ctrl.update_indexes(&me, &o.assets);
                }
                Op::UpdateAccountThreshold(o) => {
                    let mut account_ids = Vec::new(&env);
                    for id in o.account_ids.iter() {
                        account_ids.push_back(resolve(id, last_created));
                    }
                    ctrl.update_account_threshold(&me, &o.has_risks, &account_ids);
                }
                Op::CleanBadDebt(o) => {
                    ctrl.clean_bad_debt(&me, &resolve(o.account_id, last_created));
                }
                Op::AddDelegate(o) => {
                    ctrl.add_delegate(&me, &resolve(o.account_id, last_created), &o.delegate);
                }
                Op::RemoveDelegate(o) => {
                    ctrl.remove_delegate(&me, &resolve(o.account_id, last_created), &o.delegate);
                }
                Op::RenewAccount(o) => {
                    ctrl.renew_account(&me, &resolve(o.account_id, last_created));
                }
                Op::NftTransfer(o) => {
                    let token_id =
                        u32::try_from(resolve(o.token_id, last_created)).unwrap_or(u32::MAX);
                    NftTransferClient::new(&env, &nft).transfer(&me, &o.to, &token_id);
                }
                Op::StrategyDeposit(o) => {
                    assert!(vault_configured, "vault not configured");
                    let strategy = StrategyClient::new(&env, &o.strategy);
                    authorize_transfer_as_current(
                        &env,
                        &strategy.asset(),
                        &me,
                        &o.strategy,
                        o.amount,
                    );
                    strategy.deposit(&o.amount, &me);
                }
                Op::StrategyHarvest(strategy) => {
                    assert!(vault_configured, "vault not configured");
                    StrategyClient::new(&env, &strategy).harvest(&me, &None);
                }
                Op::StrategyWithdraw(o) => {
                    assert!(vault_configured, "vault not configured");
                    StrategyClient::new(&env, &o.strategy).withdraw(&o.amount, &me, &o.to);
                }
            }
        }
        last_created
    }
}

fn require_vault_owner(env: &Env) {
    let owner: Address = env
        .storage()
        .instance()
        .get(&symbol_short!("v_owner"))
        .expect("vault not configured");
    owner.require_auth();
}

fn resolve(requested: u64, last_created: u64) -> u64 {
    if requested == LAST_CREATED {
        last_created
    } else {
        requested
    }
}

/// Sums legs per hub asset in first-seen order, as the controller does before
/// it pulls.
fn merge_legs(env: &Env, legs: &Vec<(HubAssetKey, i128)>) -> Vec<(HubAssetKey, i128)> {
    let mut merged: Vec<(HubAssetKey, i128)> = Vec::new(env);
    for (key, amount) in legs.iter() {
        match (0..merged.len()).find(|&i| merged.get_unchecked(i).0 == key) {
            Some(i) => {
                let (key, total) = merged.get_unchecked(i);
                merged.set(i, (key, total.saturating_add(amount)));
            }
            None => merged.push_back((key, amount)),
        }
    }
    merged
}

/// Each merged leg at the full offer, which a full close pulls, and at the
/// offer less the estimate's refunds for that asset, which a partial pulls.
/// Assumes one hub asset per token within a liquidation.
fn liquidation_pulls(
    env: &Env,
    legs: &Vec<(HubAssetKey, i128)>,
    refunds: &Vec<PaymentTuple>,
) -> Vec<(HubAssetKey, i128)> {
    let mut pulls = Vec::new(env);
    for (key, offered) in merge_legs(env, legs).iter() {
        let refunded = refunds
            .iter()
            .filter(|refund| refund.asset == key.asset)
            .fold(0i128, |sum, refund| sum.saturating_add(refund.amount));
        if refunded != 0 {
            pulls.push_back((key.clone(), offered.saturating_sub(refunded)));
        }
        pulls.push_back((key, offered));
    }
    pulls
}

/// One exact `transfer(me, to, amount)` authorization per leg, so the
/// controller can pull the runner's tokens into `to` under enforcing auth.
fn authorize_pulls(env: &Env, me: &Address, to: &Address, legs: &Vec<(HubAssetKey, i128)>) {
    let mut entries: Vec<InvokerContractAuthEntry> = Vec::new(env);
    for (key, amount) in legs.iter() {
        entries.push_back(InvokerContractAuthEntry::Contract(SubContractInvocation {
            context: ContractContext {
                contract: key.asset.clone(),
                fn_name: symbol_short!("transfer"),
                args: (me.clone(), to.clone(), amount).into_val(env),
            },
            sub_invocations: Vec::new(env),
        }));
    }
    env.authorize_as_current_contract(entries);
}

#[cfg(test)]
mod tests;
