#![no_std]
#![allow(clippy::too_many_arguments)]

use common::errors::GenericError;
use common::types::{HubAssetKey, InterestRateModel, PositionMode, SeizeMode};
use soroban_sdk::auth::{ContractContext, InvokerContractAuthEntry, SubContractInvocation};
use soroban_sdk::{
    assert_with_error, contract, contractclient, contracterror, contractimpl, contracttype,
    panic_with_error, symbol_short, token, xdr::FromXdr, Address, Bytes, Env, IntoVal, Vec,
};

const TESTNET_CONTROLLER: &str = "CAYHSB4IPBJV6WIB2VJN5IMAVCAOUXHDLJTKWKBEQ4REIBC2RAWXQPEW";

#[contractclient(name = "PoolClient")]
pub trait Pool {
    fn flash_loan(
        env: Env,
        asset: Address,
        initiator: Address,
        receiver: Address,
        amount: i128,
        fee: i128,
        data: Bytes,
    );
}

#[contractclient(name = "ControllerClient")]
pub trait Controller {
    fn supply(
        env: Env,
        caller: Address,
        account_id: u64,
        spoke_id: u32,
        assets: Vec<(HubAssetKey, i128)>,
    ) -> u64;

    fn borrow(
        env: Env,
        caller: Address,
        account_id: u64,
        borrows: Vec<(HubAssetKey, i128)>,
        to: Option<Address>,
    );

    fn withdraw(
        env: Env,
        caller: Address,
        account_id: u64,
        withdrawals: Vec<(HubAssetKey, i128)>,
        to: Option<Address>,
    ) -> Vec<(HubAssetKey, i128)>;

    fn repay(env: Env, caller: Address, account_id: u64, payments: Vec<(HubAssetKey, i128)>);

    fn liquidate(
        env: Env,
        liquidator: Address,
        account_id: u64,
        debt_payments: Vec<(HubAssetKey, i128)>,
        seize_mode: SeizeMode,
    ) -> u64;

    fn flash_loan(
        env: Env,
        caller: Address,
        asset: HubAssetKey,
        amount: i128,
        receiver: Address,
        data: Bytes,
    );

    fn upgrade_liquidity_pool_params(env: Env, hub_asset: HubAssetKey, params: InterestRateModel);

    fn flash_position(
        env: Env,
        caller: Address,
        account_id: u64,
        spoke_id: u32,
        mode: PositionMode,
        debt: HubAssetKey,
        amount: i128,
        receiver: Address,
        data: Bytes,
        collaterals: Vec<(HubAssetKey, i128)>,
        refund_assets: Vec<Address>,
    ) -> u64;

    fn multiply(
        env: Env,
        caller: Address,
        account_id: u64,
        spoke_id: u32,
        collateral: HubAssetKey,
        debt_to_flash_loan: i128,
        debt: HubAssetKey,
        mode: PositionMode,
        swap: Bytes,
        initial_payment: Option<(HubAssetKey, i128)>,
        convert_swap: Option<Bytes>,
    ) -> u64;

    fn swap_debt(
        env: Env,
        caller: Address,
        account_id: u64,
        existing_debt: HubAssetKey,
        amount: i128,
        new_debt: HubAssetKey,
        swap: Bytes,
    );

    fn swap_collateral(
        env: Env,
        caller: Address,
        account_id: u64,
        current: HubAssetKey,
        amount: i128,
        new: HubAssetKey,
        swap: Bytes,
    );

    fn repay_debt_with_collateral(
        env: Env,
        caller: Address,
        account_id: u64,
        collateral: HubAssetKey,
        collateral_amount: i128,
        debt: HubAssetKey,
        swap: Bytes,
        close_position: bool,
    );

    fn migrate_from_blend(
        env: Env,
        caller: Address,
        account_id: u64,
        spoke_id: u32,
        hub_id: u32,
        blend_pool: Address,
        collateral_assets: Vec<Address>,
        supply_assets: Vec<Address>,
        debt_caps: Vec<(Address, i128)>,
    ) -> u64;
}

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum FlashLoanMode {
    Success = 0,
    NoRepay = 1,
    UnderRepay = 2,
    ReenterPoolFlashLoan = 3,
    Panic = 4,
    ReenterControllerSupply = 5,
    OverRepay = 6,
    PushToPool = 7,
    ReenterControllerBorrow = 8,
    ReenterControllerWithdraw = 9,
    ReenterControllerRepay = 10,
    ReenterControllerFlashLoan = 11,
    ReenterControllerFlashPosition = 12,
    ReenterControllerMultiply = 13,
    ReenterControllerSwapDebt = 14,
    ReenterControllerSwapCollateral = 15,
    ReenterControllerRdwc = 16,
    ReenterControllerLiquidate = 17,
    ReenterMigrateBlend = 18,
    /// V3 audit: reach the ONLY unguarded controller->pool state mutation
    /// (`markets.rs:104` `pool_update_indexes_call`) from inside the callback.
    ReenterControllerUpgradePoolParams = 19,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct FlashLoanRequest {
    pub mode: FlashLoanMode,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct Plan {
    pub controller: Address,
    pub hub_id: u32,
    pub spoke_id: u32,
    pub account_id: u64,
}

#[contracttype]
pub enum DataKey {
    Plan,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum ReceiverError {
    InvalidData = 1,
    InvalidShortfall = 2,
    CallbackPanic = 3,
}

#[contract]
pub struct FlashLoanTestReceiver;

#[contractimpl]
impl FlashLoanTestReceiver {
    /// Points nested reentry at a live controller. Any address may call this;
    /// the mock is test-only.
    pub fn set_plan(env: Env, controller: Address, hub_id: u32, spoke_id: u32, account_id: u64) {
        env.storage().instance().set(
            &DataKey::Plan,
            &Plan {
                controller,
                hub_id,
                spoke_id,
                account_id,
            },
        );
    }

    pub fn execute_flash_loan(
        env: Env,
        initiator: Address,
        asset: Address,
        amount: i128,
        fee: i128,
        pool: Address,
        data: Bytes,
    ) {
        let request = decode_request(&env, &data);
        let total = checked_total(&env, amount, fee);

        match request.mode {
            FlashLoanMode::Success => {
                approve_repayment(&env, &asset, &pool, total);
            }
            FlashLoanMode::NoRepay => {}
            FlashLoanMode::UnderRepay => {
                approve_under_repayment(&env, &asset, &pool, amount, fee);
            }
            FlashLoanMode::ReenterPoolFlashLoan => {
                reenter_pool_flash_loan(&env, &asset, &pool);
                approve_repayment(&env, &asset, &pool, total);
            }
            FlashLoanMode::Panic => {
                panic_with_error!(&env, ReceiverError::CallbackPanic);
            }
            FlashLoanMode::ReenterControllerSupply => {
                reenter_controller_supply(&env, &asset);
                approve_repayment(&env, &asset, &pool, total);
            }
            FlashLoanMode::OverRepay => {
                approve_repayment(&env, &asset, &pool, total.saturating_add(1));
            }
            FlashLoanMode::PushToPool => {
                push_to_pool(&env, &asset, &pool, amount);
            }
            FlashLoanMode::ReenterControllerBorrow => {
                reenter_controller_borrow(&env, &asset);
                approve_repayment(&env, &asset, &pool, total);
            }
            FlashLoanMode::ReenterControllerWithdraw => {
                reenter_controller_withdraw(&env, &asset);
                approve_repayment(&env, &asset, &pool, total);
            }
            FlashLoanMode::ReenterControllerRepay => {
                reenter_controller_repay(&env, &asset);
                approve_repayment(&env, &asset, &pool, total);
            }
            FlashLoanMode::ReenterControllerFlashLoan => {
                reenter_controller_flash_loan(&env, &initiator, &asset);
                approve_repayment(&env, &asset, &pool, total);
            }
            FlashLoanMode::ReenterControllerFlashPosition => {
                reenter_controller_flash_position(&env, &initiator, &asset, amount);
                approve_repayment(&env, &asset, &pool, total);
            }
            FlashLoanMode::ReenterControllerMultiply => {
                reenter_controller_multiply(&env, &asset);
                approve_repayment(&env, &asset, &pool, total);
            }
            FlashLoanMode::ReenterControllerSwapDebt => {
                reenter_controller_swap_debt(&env, &asset);
                approve_repayment(&env, &asset, &pool, total);
            }
            FlashLoanMode::ReenterControllerSwapCollateral => {
                reenter_controller_swap_collateral(&env, &asset);
                approve_repayment(&env, &asset, &pool, total);
            }
            FlashLoanMode::ReenterControllerRdwc => {
                reenter_controller_rdwc(&env, &asset);
                approve_repayment(&env, &asset, &pool, total);
            }
            FlashLoanMode::ReenterControllerLiquidate => {
                reenter_controller_liquidate(&env, &asset);
                approve_repayment(&env, &asset, &pool, total);
            }
            FlashLoanMode::ReenterControllerUpgradePoolParams => {
                reenter_controller_upgrade_pool_params(&env, &asset);
                approve_repayment(&env, &asset, &pool, total);
            }
            FlashLoanMode::ReenterMigrateBlend => {
                reenter_migrate_blend(&env, &asset);
                approve_repayment(&env, &asset, &pool, total);
            }
        }
    }
}

fn decode_request(env: &Env, data: &Bytes) -> FlashLoanRequest {
    FlashLoanRequest::from_xdr(env, data).unwrap_or_else(|_| {
        panic_with_error!(env, ReceiverError::InvalidData);
    })
}

fn resolve_plan(env: &Env) -> Plan {
    env.storage()
        .instance()
        .get(&DataKey::Plan)
        .unwrap_or_else(|| Plan {
            controller: Address::from_str(env, TESTNET_CONTROLLER),
            hub_id: 1,
            spoke_id: 1,
            account_id: 0,
        })
}

fn hub(plan: &Plan, asset: &Address) -> HubAssetKey {
    HubAssetKey {
        hub_id: plan.hub_id,
        asset: asset.clone(),
    }
}

fn one_asset(env: &Env, plan: &Plan, asset: &Address) -> Vec<(HubAssetKey, i128)> {
    let mut assets: Vec<(HubAssetKey, i128)> = Vec::new(env);
    assets.push_back((hub(plan, asset), 1i128));
    assets
}

fn checked_total(env: &Env, amount: i128, fee: i128) -> i128 {
    amount
        .checked_add(fee)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow))
}

fn approve_repayment(env: &Env, asset: &Address, pool: &Address, amount: i128) {
    let expiration_ledger = env
        .ledger()
        .sequence()
        .checked_add(1)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));

    authorize_token_approve(env, asset, pool, amount, expiration_ledger);
    token::Client::new(env, asset).approve(
        &env.current_contract_address(),
        pool,
        &amount,
        &expiration_ledger,
    );
}

fn authorize_token_approve(
    env: &Env,
    asset: &Address,
    spender: &Address,
    amount: i128,
    expiration_ledger: u32,
) {
    let token_approve = InvokerContractAuthEntry::Contract(SubContractInvocation {
        context: ContractContext {
            contract: asset.clone(),
            fn_name: symbol_short!("approve"),
            args: (
                env.current_contract_address(),
                spender.clone(),
                amount,
                expiration_ledger,
            )
                .into_val(env),
        },
        sub_invocations: Vec::new(env),
    });
    let mut auth_entries: Vec<InvokerContractAuthEntry> = Vec::new(env);
    auth_entries.push_back(token_approve);
    env.authorize_as_current_contract(auth_entries);
}

fn approve_under_repayment(env: &Env, asset: &Address, pool: &Address, amount: i128, fee: i128) {
    let shortfall = 1;
    let total = checked_total(env, amount, fee);
    assert_with_error!(env, shortfall < total, ReceiverError::InvalidShortfall);

    let partial = total
        .checked_sub(shortfall)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    approve_repayment(env, asset, pool, partial);
}

fn push_to_pool(env: &Env, asset: &Address, pool: &Address, amount: i128) {
    let dust = if amount > 1 { 1 } else { amount };
    if dust > 0 {
        token::Client::new(env, asset).transfer(&env.current_contract_address(), pool, &dust);
    }
}

fn reenter_pool_flash_loan(env: &Env, asset: &Address, pool: &Address) {
    PoolClient::new(env, pool).flash_loan(
        asset,
        &env.current_contract_address(),
        &env.current_contract_address(),
        &1i128,
        &0i128,
        &Bytes::new(env),
    );
}

fn authorize_controller_supply(
    env: &Env,
    controller: &Address,
    caller: &Address,
    account_id: u64,
    spoke_id: u32,
    assets: &Vec<(HubAssetKey, i128)>,
) {
    let controller_supply = InvokerContractAuthEntry::Contract(SubContractInvocation {
        context: ContractContext {
            contract: controller.clone(),
            fn_name: symbol_short!("supply"),
            args: (caller.clone(), account_id, spoke_id, assets.clone()).into_val(env),
        },
        sub_invocations: Vec::new(env),
    });
    let mut auth_entries: Vec<InvokerContractAuthEntry> = Vec::new(env);
    auth_entries.push_back(controller_supply);
    env.authorize_as_current_contract(auth_entries);
}

fn reenter_controller_supply(env: &Env, asset: &Address) {
    let plan = resolve_plan(env);
    let caller = env.current_contract_address();
    let assets = one_asset(env, &plan, asset);
    authorize_controller_supply(
        env,
        &plan.controller,
        &caller,
        plan.account_id,
        plan.spoke_id,
        &assets,
    );
    ControllerClient::new(env, &plan.controller).supply(
        &caller,
        &plan.account_id,
        &plan.spoke_id,
        &assets,
    );
}

fn reenter_controller_borrow(env: &Env, asset: &Address) {
    let plan = resolve_plan(env);
    let caller = env.current_contract_address();
    let assets = one_asset(env, &plan, asset);
    ControllerClient::new(env, &plan.controller).borrow(&caller, &plan.account_id, &assets, &None);
}

fn reenter_controller_withdraw(env: &Env, asset: &Address) {
    let plan = resolve_plan(env);
    let caller = env.current_contract_address();
    let assets = one_asset(env, &plan, asset);
    ControllerClient::new(env, &plan.controller).withdraw(
        &caller,
        &plan.account_id,
        &assets,
        &None,
    );
}

fn reenter_controller_repay(env: &Env, asset: &Address) {
    let plan = resolve_plan(env);
    let caller = env.current_contract_address();
    let assets = one_asset(env, &plan, asset);
    ControllerClient::new(env, &plan.controller).repay(&caller, &plan.account_id, &assets);
}

fn reenter_controller_flash_loan(env: &Env, initiator: &Address, asset: &Address) {
    let plan = resolve_plan(env);
    ControllerClient::new(env, &plan.controller).flash_loan(
        initiator,
        &hub(&plan, asset),
        &1i128,
        &env.current_contract_address(),
        &Bytes::new(env),
    );
}

fn reenter_controller_flash_position(
    env: &Env,
    initiator: &Address,
    asset: &Address,
    amount: i128,
) {
    let plan = resolve_plan(env);
    let mut collaterals: Vec<(HubAssetKey, i128)> = Vec::new(env);
    collaterals.push_back((hub(&plan, asset), 1i128));
    ControllerClient::new(env, &plan.controller).flash_position(
        initiator,
        &plan.account_id,
        &plan.spoke_id,
        &PositionMode::Multiply,
        &hub(&plan, asset),
        &amount,
        &env.current_contract_address(),
        &Bytes::new(env),
        &collaterals,
        &Vec::new(env),
    );
}

fn reenter_controller_multiply(env: &Env, asset: &Address) {
    let plan = resolve_plan(env);
    let caller = env.current_contract_address();
    ControllerClient::new(env, &plan.controller).multiply(
        &caller,
        &plan.account_id,
        &plan.spoke_id,
        &hub(&plan, asset),
        &1i128,
        &hub(&plan, asset),
        &PositionMode::Multiply,
        &Bytes::new(env),
        &None,
        &None,
    );
}

fn reenter_controller_swap_debt(env: &Env, asset: &Address) {
    let plan = resolve_plan(env);
    let caller = env.current_contract_address();
    ControllerClient::new(env, &plan.controller).swap_debt(
        &caller,
        &plan.account_id,
        &hub(&plan, asset),
        &1i128,
        &hub(&plan, asset),
        &Bytes::new(env),
    );
}

fn reenter_controller_swap_collateral(env: &Env, asset: &Address) {
    let plan = resolve_plan(env);
    let caller = env.current_contract_address();
    ControllerClient::new(env, &plan.controller).swap_collateral(
        &caller,
        &plan.account_id,
        &hub(&plan, asset),
        &1i128,
        &hub(&plan, asset),
        &Bytes::new(env),
    );
}

fn reenter_controller_rdwc(env: &Env, asset: &Address) {
    let plan = resolve_plan(env);
    let caller = env.current_contract_address();
    ControllerClient::new(env, &plan.controller).repay_debt_with_collateral(
        &caller,
        &plan.account_id,
        &hub(&plan, asset),
        &1i128,
        &hub(&plan, asset),
        &Bytes::new(env),
        &false,
    );
}

fn reenter_controller_liquidate(env: &Env, asset: &Address) {
    let plan = resolve_plan(env);
    let caller = env.current_contract_address();
    ControllerClient::new(env, &plan.controller).liquidate(
        &caller,
        &plan.account_id,
        &one_asset(env, &plan, asset),
        &SeizeMode::Transfer,
    );
}

fn reenter_migrate_blend(env: &Env, asset: &Address) {
    let plan = resolve_plan(env);
    let caller = env.current_contract_address();
    let mut collateral_assets: Vec<Address> = Vec::new(env);
    collateral_assets.push_back(asset.clone());
    ControllerClient::new(env, &plan.controller).migrate_from_blend(
        &caller,
        &plan.account_id,
        &plan.spoke_id,
        &plan.hub_id,
        &plan.controller,
        &collateral_assets,
        &Vec::new(env),
        &Vec::new(env),
    );
}

/// Calls the controller's `#[only_owner]` `upgrade_liquidity_pool_params` from
/// inside the flash callback. That path runs `pool_update_indexes_call` with no
/// flash guard, so it commits accrual to `PoolKey::State` while `flash::apply`
/// still holds an uncommitted `Cache`.
fn reenter_controller_upgrade_pool_params(env: &Env, asset: &Address) {
    let plan = resolve_plan(env);
    let model = InterestRateModel {
        max_borrow_rate: 2_000_000_000_000_000_000_000_000_000,
        base_borrow_rate: 30_000_000_000_000_000_000_000_000,
        slope1: 40_000_000_000_000_000_000_000_000,
        slope2: 100_000_000_000_000_000_000_000_000,
        slope3: 1_500_000_000_000_000_000_000_000_000,
        mid_utilization: 500_000_000_000_000_000_000_000_000,
        optimal_utilization: 800_000_000_000_000_000_000_000_000,
        max_utilization: 950_000_000_000_000_000_000_000_000,
        reserve_factor: 1000,
        is_flashloanable: true,
        flashloan_fee: 9,
    };
    ControllerClient::new(env, &plan.controller).upgrade_liquidity_pool_params(
        &HubAssetKey {
            hub_id: plan.hub_id,
            asset: asset.clone(),
        },
        &model,
    );
}
