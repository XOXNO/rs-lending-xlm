#![no_std]

//! Test-only flash-loan receiver for protocol smoke tests.

use common::errors::GenericError;
use common::types::HubAssetKey;
use soroban_sdk::auth::{ContractContext, InvokerContractAuthEntry, SubContractInvocation};
use soroban_sdk::{
    assert_with_error, contract, contractclient, contracterror, contractimpl, contracttype,
    panic_with_error, symbol_short, token, xdr::FromXdr, Address, Bytes, Env, IntoVal, Vec,
};

/// Testnet controller address used only by the reentrancy-attack test path.
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
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct FlashLoanRequest {
    pub mode: FlashLoanMode,
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
    /// Callback invoked by the pool mid-`flash_loan`; dispatches on
    /// `FlashLoanMode` to exercise the repayment and reentrancy paths the
    /// pool must guard against. Repayment is by `approve`, not `transfer`:
    /// the pool pulls the owed amount after this callback returns.
    pub fn execute_flash_loan(
        env: Env,
        _initiator: Address,
        asset: Address,
        amount: i128,
        fee: i128,
        pool: Address,
        data: Bytes,
    ) {
        let request = decode_request(&env, &data);

        match request.mode {
            FlashLoanMode::Success => {
                approve_repayment(&env, &asset, &pool, checked_total(&env, amount, fee));
            }
            FlashLoanMode::NoRepay => {}
            FlashLoanMode::UnderRepay => {
                approve_under_repayment(&env, &asset, &pool, amount, fee);
            }
            FlashLoanMode::ReenterPoolFlashLoan => {
                reenter_pool_flash_loan(&env, &asset, &pool);
            }
            FlashLoanMode::Panic => {
                panic_with_error!(&env, ReceiverError::CallbackPanic);
            }
            FlashLoanMode::ReenterControllerSupply => {
                reenter_controller_supply(&env, &asset);
            }
        }
    }
}

fn decode_request(env: &Env, data: &Bytes) -> FlashLoanRequest {
    FlashLoanRequest::from_xdr(env, data).unwrap_or_else(|_| {
        panic_with_error!(env, ReceiverError::InvalidData);
    })
}

fn checked_total(env: &Env, amount: i128, fee: i128) -> i128 {
    amount
        .checked_add(fee)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow))
}

/// Approves the pool to pull exactly `amount` via the token's allowance,
/// mirroring the pool's pull-based flash-loan repayment.
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

/// Approves one unit less than owed so the pool's repayment check must
/// reject it.
fn approve_under_repayment(env: &Env, asset: &Address, pool: &Address, amount: i128, fee: i128) {
    let shortfall = 1;
    let total = checked_total(env, amount, fee);
    assert_with_error!(env, shortfall < total, ReceiverError::InvalidShortfall);

    let partial = total
        .checked_sub(shortfall)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    approve_repayment(env, asset, pool, partial);
}

/// Exercises the pool's flash-loan reentrancy guard by calling back into
/// `flash_loan` from within the active callback.
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

/// Exercises the controller's flash-loan reentrancy guard by calling
/// `supply` from within the pool's active callback.
fn reenter_controller_supply(env: &Env, asset: &Address) {
    let controller = Address::from_str(env, TESTNET_CONTROLLER);
    let caller = env.current_contract_address();
    let mut assets: Vec<(HubAssetKey, i128)> = Vec::new(env);
    assets.push_back((
        HubAssetKey {
            hub_id: 0,
            asset: asset.clone(),
        },
        1i128,
    ));

    authorize_controller_supply(env, &controller, &caller, &assets);
    ControllerClient::new(env, &controller).supply(&caller, &0u64, &0u32, &assets);
}

fn authorize_controller_supply(
    env: &Env,
    controller: &Address,
    caller: &Address,
    assets: &Vec<(HubAssetKey, i128)>,
) {
    let controller_supply = InvokerContractAuthEntry::Contract(SubContractInvocation {
        context: ContractContext {
            contract: controller.clone(),
            fn_name: symbol_short!("supply"),
            args: (caller.clone(), 0u64, 0u32, assets.clone()).into_val(env),
        },
        sub_invocations: Vec::new(env),
    });
    let mut auth_entries: Vec<InvokerContractAuthEntry> = Vec::new(env);
    auth_entries.push_back(controller_supply);
    env.authorize_as_current_contract(auth_entries);
}
