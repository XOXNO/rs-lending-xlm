#![allow(clippy::too_many_arguments)]

use common::errors::GenericError;
use common::types::{HubAssetKey, PositionMode};
use soroban_sdk::auth::{ContractContext, InvokerContractAuthEntry, SubContractInvocation};
use soroban_sdk::xdr::FromXdr;
use soroban_sdk::{
    contract, contractclient, contractimpl, contracttype, panic_with_error, symbol_short, token,
    Address, Bytes, Env, IntoVal, Vec,
};

use crate::helpers::HARNESS_HUB;

#[contractclient(name = "FlashPositionControllerClient")]
pub trait FlashPositionController {
    fn supply(
        env: Env,
        caller: Address,
        account_id: u64,
        spoke_id: u32,
        assets: Vec<(HubAssetKey, i128)>,
    ) -> u64;

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
        mode: PositionMode,
        debt: HubAssetKey,
        amount: i128,
        receiver: Address,
        data: Bytes,
        collaterals: Vec<(HubAssetKey, i128)>,
        refund_assets: Vec<Address>,
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
}

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum FlashPositionMode {
    Success = 0,
    KeepFunds = 1,
    BelowMin = 2,
    Undeclared = 3,
    ReenterSupply = 4,
    ReenterFlashLoan = 5,
    ReenterFlashPosition = 6,
    Panic = 7,
    PushDebtBack = 8,
    SupplyAndReturnDebt = 9,
    ReenterBorrow = 10,
    ReenterWithdraw = 11,
    ReenterRepay = 12,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FlashPositionRequest {
    pub mode: FlashPositionMode,
    pub collateral: Address,
    pub collateral_amount: i128,
    pub extra_asset: Address,
    pub extra_amount: i128,
    pub reenter_spoke_id: u32,
    pub reenter_account_id: u64,
}

#[contract]
pub struct FlashPositionTestReceiver;

#[contractimpl]
impl FlashPositionTestReceiver {
    pub fn execute_flash_position(
        env: Env,
        initiator: Address,
        account_id: u64,
        asset: Address,
        amount: i128,
        _fee: i128,
        amount_received: i128,
        controller: Address,
        data: Bytes,
    ) {
        let request = FlashPositionRequest::from_xdr(&env, &data).unwrap_or_else(|_| {
            panic_with_error!(&env, GenericError::InvalidPayments);
        });
        let self_addr = env.current_contract_address();

        match request.mode {
            FlashPositionMode::Success => {
                push_token(
                    &env,
                    &request.collateral,
                    request.collateral_amount,
                    &controller,
                );
            }
            FlashPositionMode::KeepFunds => {}
            FlashPositionMode::BelowMin => {
                let amount = request.collateral_amount.saturating_sub(1);
                if amount > 0 {
                    push_token(&env, &request.collateral, amount, &controller);
                }
            }
            FlashPositionMode::Undeclared => {
                if request.collateral_amount > 0 {
                    push_token(
                        &env,
                        &request.collateral,
                        request.collateral_amount,
                        &controller,
                    );
                }
                if request.extra_amount > 0 {
                    push_token(
                        &env,
                        &request.extra_asset,
                        request.extra_amount,
                        &controller,
                    );
                }
            }
            FlashPositionMode::ReenterSupply => {
                reenter_supply(&env, &controller, &request);
            }
            FlashPositionMode::ReenterFlashLoan => {
                reenter_flash_loan(&env, &controller, &initiator, &asset);
            }
            FlashPositionMode::ReenterFlashPosition => {
                reenter_flash_position(
                    &env,
                    &controller,
                    &initiator,
                    &asset,
                    amount,
                    &self_addr,
                    &request,
                    account_id,
                );
            }
            FlashPositionMode::ReenterBorrow => {
                reenter_borrow(&env, &controller, &request);
            }
            FlashPositionMode::ReenterWithdraw => {
                reenter_withdraw(&env, &controller, &request);
            }
            FlashPositionMode::ReenterRepay => {
                reenter_repay(&env, &controller, &request);
            }
            FlashPositionMode::Panic => {
                panic_with_error!(&env, GenericError::InternalError);
            }
            FlashPositionMode::PushDebtBack => {
                if amount_received > 0 {
                    token::Client::new(&env, &asset).transfer(
                        &self_addr,
                        &controller,
                        &amount_received,
                    );
                }
            }
            FlashPositionMode::SupplyAndReturnDebt => {
                push_token(
                    &env,
                    &request.collateral,
                    request.collateral_amount,
                    &controller,
                );
                if amount_received > 0 {
                    token::Client::new(&env, &asset).transfer(
                        &self_addr,
                        &controller,
                        &amount_received,
                    );
                }
            }
        }
    }
}

fn push_token(env: &Env, asset: &Address, amount: i128, to: &Address) {
    if amount <= 0 {
        return;
    }
    let self_addr = env.current_contract_address();
    token::StellarAssetClient::new(env, asset).mint(&self_addr, &amount);
    token::Client::new(env, asset).transfer(&self_addr, to, &amount);
}

fn reenter_supply(env: &Env, controller: &Address, request: &FlashPositionRequest) {
    let caller = env.current_contract_address();
    let mut assets: Vec<(HubAssetKey, i128)> = Vec::new(env);
    assets.push_back((
        HubAssetKey {
            hub_id: HARNESS_HUB,
            asset: request.collateral.clone(),
        },
        1i128,
    ));
    let supply = InvokerContractAuthEntry::Contract(SubContractInvocation {
        context: ContractContext {
            contract: controller.clone(),
            fn_name: symbol_short!("supply"),
            args: (
                caller.clone(),
                request.reenter_account_id,
                request.reenter_spoke_id,
                assets.clone(),
            )
                .into_val(env),
        },
        sub_invocations: Vec::new(env),
    });
    env.authorize_as_current_contract(soroban_sdk::vec![env, supply]);
    FlashPositionControllerClient::new(env, controller).supply(
        &caller,
        &request.reenter_account_id,
        &request.reenter_spoke_id,
        &assets,
    );
}

fn reenter_flash_loan(env: &Env, controller: &Address, caller: &Address, asset: &Address) {
    FlashPositionControllerClient::new(env, controller).flash_loan(
        caller,
        &HubAssetKey {
            hub_id: HARNESS_HUB,
            asset: asset.clone(),
        },
        &1i128,
        &env.current_contract_address(),
        &Bytes::new(env),
    );
}

#[allow(clippy::too_many_arguments)]
fn reenter_flash_position(
    env: &Env,
    controller: &Address,
    caller: &Address,
    asset: &Address,
    amount: i128,
    receiver: &Address,
    request: &FlashPositionRequest,
    account_id: u64,
) {
    let mut collaterals: Vec<(HubAssetKey, i128)> = Vec::new(env);
    collaterals.push_back((
        HubAssetKey {
            hub_id: HARNESS_HUB,
            asset: request.collateral.clone(),
        },
        1i128,
    ));
    FlashPositionControllerClient::new(env, controller).flash_position(
        caller,
        &account_id,
        &request.reenter_spoke_id,
        &PositionMode::Multiply,
        &HubAssetKey {
            hub_id: HARNESS_HUB,
            asset: asset.clone(),
        },
        &amount,
        receiver,
        &Bytes::new(env),
        &collaterals,
        &Vec::new(env),
    );
}

fn one_asset(env: &Env, asset: &Address) -> Vec<(HubAssetKey, i128)> {
    let mut assets: Vec<(HubAssetKey, i128)> = Vec::new(env);
    assets.push_back((
        HubAssetKey {
            hub_id: HARNESS_HUB,
            asset: asset.clone(),
        },
        1i128,
    ));
    assets
}

fn reenter_borrow(env: &Env, controller: &Address, request: &FlashPositionRequest) {
    let caller = env.current_contract_address();
    let assets = one_asset(env, &request.collateral);
    FlashPositionControllerClient::new(env, controller).borrow(
        &caller,
        &request.reenter_account_id,
        &assets,
        &None,
    );
}

fn reenter_withdraw(env: &Env, controller: &Address, request: &FlashPositionRequest) {
    let caller = env.current_contract_address();
    let assets = one_asset(env, &request.collateral);
    FlashPositionControllerClient::new(env, controller).withdraw(
        &caller,
        &request.reenter_account_id,
        &assets,
        &None,
    );
}

fn reenter_repay(env: &Env, controller: &Address, request: &FlashPositionRequest) {
    let caller = env.current_contract_address();
    let assets = one_asset(env, &request.collateral);
    FlashPositionControllerClient::new(env, controller).repay(
        &caller,
        &request.reenter_account_id,
        &assets,
    );
}
