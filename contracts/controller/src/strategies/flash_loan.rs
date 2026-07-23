//! User flash loans: pool pays `receiver`, exact principal+fee repaid in-callback.
//!
//! Caller auth; no account/HF. Reentrancy guard blocks nested controller entry
//! for the callback. See `architecture/INVARIANTS.md` §2.5.

use crate::events::FlashLoanEvent;
use common::types::HubAssetKey;
use soroban_sdk::{contractimpl, Address, Bytes, Env};
use stellar_macros::when_not_paused;

use crate::context::Cache;
use crate::external::pool::pool_flash_loan_call;
use crate::{risk::validation, storage, Controller, ControllerArgs, ControllerClient};

#[contractimpl]
impl Controller {
    /// Flash-loans `amount` of `asset` to `receiver` with opaque `data`.
    /// Caller auth. Pool enforces exact principal+fee repayment before return.
    ///
    /// # Errors
    /// * `FlashLoanOngoing` — a flash loan or strategy is mid-execution.
    /// * `AmountMustBePositive` — `amount` is not strictly positive.
    /// * `HubNotActive` — hub is inactive.
    /// * `InvalidFlashloanReceiver` — `receiver` is not a WASM contract.
    /// * Pool-side flash errors (`FlashloanNotEnabled`, `InvalidFlashloanRepay`, etc.).
    /// * The `#[when_not_paused]` guard reverts while the contract is paused.
    ///
    /// # Events
    /// * topics — `["position", "flash_loan"]`
    #[when_not_paused]
    pub fn flash_loan(
        env: Env,
        caller: Address,
        asset: HubAssetKey,
        amount: i128,
        receiver: Address,
        data: Bytes,
    ) {
        process_flash_loan(&env, &caller, &asset, amount, &receiver, &data);
    }
}

/// Pool flash loan to `receiver` with principal+fee repaid before return. No account.
pub(crate) fn process_flash_loan(
    env: &Env,
    caller: &Address,
    hub_asset: &HubAssetKey,
    amount: i128,
    receiver: &Address,
    data: &Bytes,
) {
    caller.require_auth();

    validation::require_not_flash_loaning(env);
    validation::require_positive_amount(env, amount);
    validation::require_hub_active(env, hub_asset.hub_id);

    validation::require_wasm_receiver(env, receiver);

    let mut cache = Cache::new(env);
    let pool_addr = cache.cached_pool_address();

    // Availability (`is_flashloanable`) and fee are pool-owned: the pool gates
    // the market, computes the fee from its `flashloan_fee` bps, and returns it
    // for the event. A non-market asset reverts pool-side. Flash loans never
    // price, so no oracle gate is needed. The guard blocks nested entry.
    let fee = storage::with_flash_guard(env, || {
        pool_flash_loan_call(env, &pool_addr, hub_asset, caller, receiver, amount, data)
    });

    FlashLoanEvent {
        hub_id: hub_asset.hub_id,
        asset: hub_asset.asset.clone(),
        receiver: receiver.clone(),
        caller: caller.clone(),
        amount,
        fee,
    }
    .publish(env);
}
