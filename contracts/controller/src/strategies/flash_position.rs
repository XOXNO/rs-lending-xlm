use common::errors::{FlashLoanError, GenericError, StrategyError};
use common::types::{Account, AccountPositionType, HubAssetKey, PositionMode};
use common::validation::{
    require_non_empty_payments, require_nonneg_amount, require_positive_amount,
    require_wasm_receiver,
};
use soroban_sdk::{
    assert_with_error, panic_with_error, token, vec, Address, Bytes, Env, IntoVal, Map, Symbol, Vec,
};

use crate::account;
use crate::config;
use crate::context::Cache;
use crate::events::{FlashPositionEvent, PositionAction};
use crate::payments::transfer_amount_measured;
use crate::positions::supply::process_deposit;
use crate::positions::{require_can_supply, validate_position_entry_gates};
use crate::risk::validation::require_authorized_caller;
use crate::storage;
use crate::strategies::{
    borrow_into_controller, legs::refund_controller_balance_delta, prefetch_strategy_prices,
    snapshot_balances, strategy_finalize,
};

pub(crate) struct FlashPositionParams<'a> {
    pub account_id: u64,
    pub spoke_id: u32,
    pub mode: PositionMode,
    pub debt: &'a HubAssetKey,
    pub amount: i128,
    pub receiver: &'a Address,
    pub data: &'a Bytes,
    pub collaterals: &'a Vec<(HubAssetKey, i128)>,
    pub refund_assets: &'a Vec<Address>,
}

/// Opens or extends a leveraged position by minting strategy debt with no
/// flash fee, forwarding the measured tokens to `receiver`, invoking
/// `execute_flash_position`, and depositing measured controller-balance
/// increases of the declared collaterals. Does not repay. Returns the
/// account id after ordinary solvency finalize.
pub(crate) fn process_flash_position(
    env: &Env,
    caller: &Address,
    params: FlashPositionParams<'_>,
) -> u64 {
    require_authorized_caller(env, caller);

    let FlashPositionParams {
        account_id,
        spoke_id,
        mode,
        debt,
        amount,
        receiver,
        data,
        collaterals,
        refund_assets,
    } = params;

    require_positive_amount(env, amount);
    config::require_hub_active(env, debt.hub_id);
    require_wasm_receiver(env, receiver);

    let controller = env.current_contract_address();
    assert_with_error!(
        env,
        *receiver != controller,
        FlashLoanError::InvalidFlashloanReceiver
    );

    let mut cache = Cache::new(env);
    let pool_addr = cache.cached_pool_address();
    assert_with_error!(
        env,
        *receiver != pool_addr,
        FlashLoanError::InvalidFlashloanReceiver
    );

    let (account_id, mut account) = account::load_or_create_account(
        env,
        caller,
        account_id,
        spoke_id,
        mode,
        account::AccountGuard::Multiply,
        &mut cache,
    );

    validate_collaterals(env, &mut cache, &account, collaterals);
    validate_refund_assets(
        env,
        &mut cache,
        account.spoke_id,
        debt.hub_id,
        collaterals,
        refund_assets,
    );

    let mut extra_assets = vec![env, debt.asset.clone()];
    for (hub_asset, _) in collaterals.iter() {
        extra_assets.push_back(hub_asset.asset.clone());
    }
    prefetch_strategy_prices(&mut cache, &account, &extra_assets);

    // Guard covers the untrusted token forward *and* the receiver callback.
    // A listed token with a transfer hook could otherwise reenter before the
    // callback, which is the only window that is not a normal borrow.
    let (amount_received, collateral_before, refund_before) =
        storage::with_flash_guard(env, || {
            let amount_received =
                mint_and_forward(env, &mut account, debt, amount, receiver, &mut cache);
            let collateral_before = snapshot_balances(
                env,
                &controller,
                collaterals.iter().map(|(hub_asset, _)| hub_asset.asset),
            );
            let refund_before = snapshot_balances(env, &controller, refund_assets.iter());
            invoke_receiver(
                env,
                receiver,
                caller,
                account_id,
                &debt.asset,
                amount,
                amount_received,
                &controller,
                data,
            );
            (amount_received, collateral_before, refund_before)
        });

    let deposits = collect_collateral_deposits(env, &controller, collaterals, &collateral_before);
    process_deposit(env, &controller, &mut account, &deposits, &mut cache);

    refund_listed_assets(env, caller, refund_assets, &refund_before);

    require_flash_position_still_open(env, &account, debt);
    strategy_finalize(env, account_id, &mut account, &mut cache);
    require_flash_position_still_open(env, &account, debt);

    FlashPositionEvent {
        account_id,
        hub_id: debt.hub_id,
        asset: debt.asset.clone(),
        receiver: receiver.clone(),
        caller: caller.clone(),
        amount,
        amount_received,
        fee: 0,
    }
    .publish(env);

    account_id
}

/// Panics unless `collaterals` is non-empty, within the supply-position limit,
/// free of repeated assets, made of non-negative minimums with at least one
/// positive, supplyable into `account`'s spoke, and past the deposit entry
/// gates.
///
/// Uniqueness is enforced on the underlying asset rather than the whole
/// `HubAssetKey`: two distinct keys may share an asset (they differ only by
/// `hub_id`), so the asset check is the strictly stronger of the two and a
/// repeated key cannot slip past it.
fn validate_collaterals(
    env: &Env,
    cache: &mut Cache,
    account: &Account,
    collaterals: &Vec<(HubAssetKey, i128)>,
) {
    require_non_empty_payments(env, collaterals);

    let limits = storage::get_position_limits(env);
    assert_with_error!(
        env,
        collaterals.len() <= limits.max_supply_positions,
        GenericError::InvalidPayments
    );

    let mut seen_assets: Map<Address, bool> = Map::new(env);
    let mut has_positive_min = false;

    for (hub_asset, min_amount) in collaterals.iter() {
        require_nonneg_amount(env, min_amount);
        assert_with_error!(
            env,
            !seen_assets.contains_key(hub_asset.asset.clone()),
            GenericError::InvalidPayments
        );
        if min_amount > 0 {
            has_positive_min = true;
        }
        require_can_supply(env, cache, account.spoke_id, &hub_asset);
        seen_assets.set(hub_asset.asset.clone(), true);
    }

    assert_with_error!(env, has_positive_min, StrategyError::CollateralRequired);

    validate_position_entry_gates(
        env,
        account,
        collaterals,
        cache,
        AccountPositionType::Deposit,
    );
}

fn validate_refund_assets(
    env: &Env,
    cache: &mut Cache,
    spoke_id: u32,
    hub_id: u32,
    collaterals: &Vec<(HubAssetKey, i128)>,
    refund_assets: &Vec<Address>,
) {
    let limits = storage::get_position_limits(env);
    assert_with_error!(
        env,
        refund_assets.len() <= limits.max_supply_positions,
        GenericError::InvalidPayments
    );

    let mut seen: Map<Address, bool> = Map::new(env);
    for asset in refund_assets.iter() {
        assert_with_error!(
            env,
            !seen.contains_key(asset.clone()),
            GenericError::InvalidPayments
        );
        seen.set(asset.clone(), true);
        // The refund leg hands this address to `token::Client` after the flash
        // guard has closed. Requiring it to be listed keeps that call on a
        // governance-approved contract instead of one the caller chose.
        cache.require_listed_active_config(
            spoke_id,
            &HubAssetKey {
                hub_id,
                asset: asset.clone(),
            },
        );
        for (collateral, _) in collaterals.iter() {
            assert_with_error!(
                env,
                asset != collateral.asset,
                GenericError::InvalidPayments
            );
        }
    }
}

/// Mints strategy debt into the controller with no fee, requires the
/// controller's measured receipt to match the pool mutation, and forwards
/// that measured amount to `receiver`.
fn mint_and_forward(
    env: &Env,
    account: &mut Account,
    debt: &HubAssetKey,
    amount: i128,
    receiver: &Address,
    cache: &mut Cache,
) -> i128 {
    let controller = env.current_contract_address();
    let tok = token::Client::new(env, &debt.asset);
    let before = tok.balance(&controller);

    let reported = borrow_into_controller(
        env,
        account,
        debt,
        amount,
        false,
        PositionAction::FlashPos,
        cache,
    );

    let measured = tok
        .balance(&controller)
        .checked_sub(before)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::InternalError));
    assert_with_error!(env, measured == reported, GenericError::InternalError);
    assert_with_error!(env, measured > 0, GenericError::AmountMustBePositive);

    let forwarded = transfer_amount_measured(
        env,
        &debt.asset,
        &controller,
        receiver,
        measured,
        GenericError::AmountMustBePositive,
    );
    assert_with_error!(env, forwarded > 0, GenericError::AmountMustBePositive);
    forwarded
}

fn invoke_receiver(
    env: &Env,
    receiver: &Address,
    initiator: &Address,
    account_id: u64,
    asset: &Address,
    amount: i128,
    amount_received: i128,
    controller: &Address,
    data: &Bytes,
) {
    env.invoke_contract::<()>(
        receiver,
        &Symbol::new(env, "execute_flash_position"),
        (
            initiator.clone(),
            account_id,
            asset.clone(),
            amount,
            0i128,
            amount_received,
            controller.clone(),
            data.clone(),
        )
            .into_val(env),
    );
}

fn collect_collateral_deposits(
    env: &Env,
    controller: &Address,
    collaterals: &Vec<(HubAssetKey, i128)>,
    before: &Map<Address, i128>,
) -> Vec<(HubAssetKey, i128)> {
    let mut deposits: Vec<(HubAssetKey, i128)> = Vec::new(env);
    for (hub_asset, min_amount) in collaterals.iter() {
        let baseline = before
            .get(hub_asset.asset.clone())
            .unwrap_or_else(|| panic_with_error!(env, GenericError::InternalError));
        let current = token::Client::new(env, &hub_asset.asset).balance(controller);
        let delta = current
            .checked_sub(baseline)
            .unwrap_or_else(|| panic_with_error!(env, GenericError::InternalError));
        assert_with_error!(
            env,
            delta >= min_amount,
            StrategyError::CollateralMinimumNotMet
        );
        if delta > 0 {
            deposits.push_back((hub_asset, delta));
        }
    }
    assert_with_error!(
        env,
        !deposits.is_empty(),
        StrategyError::CollateralMinimumNotMet
    );
    deposits
}

/// Panics with `FlashPositionClosed` unless `account` still holds a live
/// scaled debt position in `debt` and at least one supply position. This is
/// the last-line defense against a callback-plus-later-repay round trip
/// leaving an empty account.
pub(crate) fn require_flash_position_still_open(env: &Env, account: &Account, debt: &HubAssetKey) {
    assert_with_error!(
        env,
        !account.is_empty() && !account.debt_free(),
        StrategyError::FlashPositionClosed
    );
    let Some(pos) = account.borrow_positions.get(debt.clone()) else {
        panic_with_error!(env, StrategyError::FlashPositionClosed);
    };
    assert_with_error!(
        env,
        pos.scaled_amount > 0 && !account.supply_positions.is_empty(),
        StrategyError::FlashPositionClosed
    );
}

fn refund_listed_assets(
    env: &Env,
    caller: &Address,
    refund_assets: &Vec<Address>,
    before: &Map<Address, i128>,
) {
    for asset in refund_assets.iter() {
        let baseline = before
            .get(asset.clone())
            .unwrap_or_else(|| panic_with_error!(env, GenericError::InternalError));
        refund_controller_balance_delta(env, &asset, baseline, caller);
    }
}

#[cfg(test)]
#[path = "../../tests/strategies/flash_position.rs"]
mod flash_position_tests;
