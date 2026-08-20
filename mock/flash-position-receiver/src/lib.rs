#![no_std]
// The contract ABI fixes these arities: the entrypoint and its generated
// client both exceed the lint. Same treatment as mock/flash-loan-receiver.
#![allow(clippy::too_many_arguments)]

//! Test-only `execute_flash_position` receiver for live testnet coverage.
//! Not for production. Any address can `set_plan`; a real receiver must gate
//! the caller to the trusted controller.

use soroban_sdk::auth::{ContractContext, InvokerContractAuthEntry, SubContractInvocation};
use soroban_sdk::{
    contract, contractclient, contracterror, contractimpl, contracttype, panic_with_error,
    symbol_short, token, vec, Address, Bytes, Env, IntoVal, Vec,
};

const MODE_SUCCESS: u32 = 0;
const MODE_KEEP_FUNDS: u32 = 1;
const MODE_BELOW_MIN: u32 = 2;
const MODE_PANIC: u32 = 3;
const MODE_REENTER_SUPPLY: u32 = 4;
const MODE_REENTER_BORROW: u32 = 5;
const MODE_REENTER_WITHDRAW: u32 = 6;
const MODE_REENTER_REPAY: u32 = 7;
const MODE_REENTER_FLASH_LOAN: u32 = 8;
const MODE_REENTER_FLASH_POSITION: u32 = 9;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum ReceiverError {
    MissingPlan = 1,
    CallbackPanic = 2,
    InvalidMode = 3,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Plan {
    pub mode: u32,
    pub collateral: Address,
    pub amount: i128,
    pub extra: Address,
    pub extra_amount: i128,
    pub spoke_id: u32,
}

#[contracttype]
pub enum DataKey {
    Plan,
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

    fn flash_loan(
        env: Env,
        caller: Address,
        asset: HubAssetKey,
        amount: i128,
        receiver: Address,
        data: Bytes,
    );

    fn flash_position(
        env: Env,
        caller: Address,
        account_id: u64,
        spoke_id: u32,
        mode: u32,
        debt: HubAssetKey,
        amount: i128,
        receiver: Address,
        data: Bytes,
        collaterals: Vec<(HubAssetKey, i128)>,
        refund_assets: Vec<Address>,
    ) -> u64;
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HubAssetKey {
    pub hub_id: u32,
    pub asset: Address,
}

#[contract]
pub struct FlashPositionTestReceiver;

#[contractimpl]
impl FlashPositionTestReceiver {
    pub fn set_plan(
        env: Env,
        mode: u32,
        collateral: Address,
        amount: i128,
        extra: Address,
        extra_amount: i128,
        spoke_id: u32,
    ) {
        env.storage().instance().set(
            &DataKey::Plan,
            &Plan {
                mode,
                collateral,
                amount,
                extra,
                extra_amount,
                spoke_id,
            },
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn execute_flash_position(
        env: Env,
        initiator: Address,
        account_id: u64,
        asset: Address,
        amount: i128,
        _fee: i128,
        amount_received: i128,
        controller: Address,
        _data: Bytes,
    ) {
        let plan = env
            .storage()
            .instance()
            .get::<DataKey, Plan>(&DataKey::Plan)
            .unwrap_or_else(|| panic_with_error!(&env, ReceiverError::MissingPlan));

        match plan.mode {
            MODE_SUCCESS => {
                push_token(&env, &plan.collateral, plan.amount, &controller);
                push_token(&env, &plan.extra, plan.extra_amount, &controller);
            }
            MODE_KEEP_FUNDS => {}
            MODE_BELOW_MIN => {
                let amount = plan.amount.saturating_sub(1);
                push_token(&env, &plan.collateral, amount, &controller);
            }
            MODE_PANIC => panic_with_error!(&env, ReceiverError::CallbackPanic),
            MODE_REENTER_SUPPLY => reenter_supply(&env, &controller, &plan, account_id),
            MODE_REENTER_BORROW => reenter_borrow(&env, &controller, &plan, account_id),
            MODE_REENTER_WITHDRAW => reenter_withdraw(&env, &controller, &plan, account_id),
            MODE_REENTER_REPAY => reenter_repay(&env, &controller, &plan, account_id),
            MODE_REENTER_FLASH_LOAN => {
                reenter_flash_loan(&env, &controller, &initiator, &asset, plan.spoke_id)
            }
            MODE_REENTER_FLASH_POSITION => reenter_flash_position(
                &env,
                &controller,
                &initiator,
                &asset,
                amount,
                account_id,
                &plan,
            ),
            _ => panic_with_error!(&env, ReceiverError::InvalidMode),
        }
        let _ = amount_received;
    }
}

fn push_token(env: &Env, asset: &Address, amount: i128, to: &Address) {
    if amount <= 0 {
        return;
    }
    let from = env.current_contract_address();
    let token_xfer = InvokerContractAuthEntry::Contract(SubContractInvocation {
        context: ContractContext {
            contract: asset.clone(),
            fn_name: symbol_short!("transfer"),
            args: (from.clone(), to.clone(), amount).into_val(env),
        },
        sub_invocations: Vec::new(env),
    });
    env.authorize_as_current_contract(vec![env, token_xfer]);
    token::Client::new(env, asset).transfer(&from, to, &amount);
}

fn one_asset(env: &Env, hub_id: u32, asset: &Address) -> Vec<(HubAssetKey, i128)> {
    let mut assets: Vec<(HubAssetKey, i128)> = Vec::new(env);
    assets.push_back((
        HubAssetKey {
            hub_id,
            asset: asset.clone(),
        },
        1i128,
    ));
    assets
}

fn reenter_supply(env: &Env, controller: &Address, plan: &Plan, account_id: u64) {
    let caller = env.current_contract_address();
    let assets = one_asset(env, 1, &plan.collateral);
    ControllerClient::new(env, controller).supply(&caller, &account_id, &plan.spoke_id, &assets);
}

fn reenter_borrow(env: &Env, controller: &Address, plan: &Plan, account_id: u64) {
    let caller = env.current_contract_address();
    let assets = one_asset(env, 1, &plan.collateral);
    ControllerClient::new(env, controller).borrow(&caller, &account_id, &assets, &None);
}

fn reenter_withdraw(env: &Env, controller: &Address, plan: &Plan, account_id: u64) {
    let caller = env.current_contract_address();
    let assets = one_asset(env, 1, &plan.collateral);
    ControllerClient::new(env, controller).withdraw(&caller, &account_id, &assets, &None);
}

fn reenter_repay(env: &Env, controller: &Address, plan: &Plan, account_id: u64) {
    let caller = env.current_contract_address();
    let assets = one_asset(env, 1, &plan.collateral);
    ControllerClient::new(env, controller).repay(&caller, &account_id, &assets);
}

fn reenter_flash_loan(
    env: &Env,
    controller: &Address,
    caller: &Address,
    asset: &Address,
    _spoke_id: u32,
) {
    ControllerClient::new(env, controller).flash_loan(
        caller,
        &HubAssetKey {
            hub_id: 1,
            asset: asset.clone(),
        },
        &1i128,
        &env.current_contract_address(),
        &Bytes::new(env),
    );
}

fn reenter_flash_position(
    env: &Env,
    controller: &Address,
    caller: &Address,
    asset: &Address,
    amount: i128,
    account_id: u64,
    plan: &Plan,
) {
    let receiver = env.current_contract_address();
    let collaterals = one_asset(env, 1, &plan.collateral);
    ControllerClient::new(env, controller).flash_position(
        caller,
        &account_id,
        &plan.spoke_id,
        &1u32,
        &HubAssetKey {
            hub_id: 1,
            asset: asset.clone(),
        },
        &amount,
        &receiver,
        &Bytes::new(env),
        &collaterals,
        &Vec::new(env),
    );
}
