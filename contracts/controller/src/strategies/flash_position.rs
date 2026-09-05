use common::errors::{CollateralError, FlashLoanError, GenericError, StrategyError};
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
use crate::context::Context;
use crate::events::{FlashPositionEvent, PositionAction};
use crate::payments::{
    balance_delta_since, refund_controller_balance_delta, snapshot_balances,
    transfer_amount_measured,
};
use crate::positions::supply::process_deposit;
use crate::positions::{require_can_supply, validate_position_entry_gates};
use crate::risk::validation::require_authorized_caller;
use crate::storage;
use crate::strategies::{borrow_into_controller, prefetch_strategy_prices, strategy_finalize};

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

/// Opens or extends a leveraged position with fee-free debt and a receiver
/// callback. Deposits measured collateral receipts and requires the debt and
/// collateral to remain open through risk checks and finalization.
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
    assert_with_error!(
        env,
        matches!(
            mode,
            PositionMode::Multiply | PositionMode::Long | PositionMode::Short
        ),
        CollateralError::InvalidPositionMode
    );
    require_wasm_receiver(env, receiver);

    let controller = env.current_contract_address();
    assert_with_error!(
        env,
        *receiver != controller,
        FlashLoanError::InvalidFlashloanReceiver
    );

    let mut cache = Context::new(env);
    let pool_addr = cache.cached_pool_address();
    assert_with_error!(
        env,
        *receiver != pool_addr,
        FlashLoanError::InvalidFlashloanReceiver
    );
    // Caller-selected receivers require flash loans enabled; multiply uses
    // the configured router and does not require this flag.
    assert_with_error!(
        env,
        cache.cached_pool_sync_data(debt).params.is_flashloanable,
        FlashLoanError::FlashloanNotEnabled
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

    // Guard both forwarding and the callback: token hooks can reenter first.
    let (amount_received, collateral_before, refund_before) =
        storage::with_flash_guard(env, || {
            let amount_received =
                mint_and_forward(env, &mut account, debt, amount, receiver, &mut cache);
            // Baselines exclude funding and forwarding; count callback receipts only.
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

    // Check before and after finalization: its LTV refresh can prune zero-scaled
    // supply, and persistence removes empty accounts.
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

/// Validates collateral limits, supply eligibility and non-negative minimums
/// with at least one positive. Uniqueness is by token, since different hubs
/// share the same controller token balance.
fn validate_collaterals(
    env: &Env,
    cache: &mut Context,
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
    cache: &mut Context,
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
        // Refund transfers run after the guard; restrict tokens to listed assets.
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

/// Mints fee-free debt, verifies the controller receipt against the pool result,
/// then forwards it and returns the receiver's measured receipt.
fn mint_and_forward(
    env: &Env,
    account: &mut Account,
    debt: &HubAssetKey,
    amount: i128,
    receiver: &Address,
    cache: &mut Context,
) -> i128 {
    let controller = env.current_contract_address();
    let before = token::Client::new(env, &debt.asset).balance(&controller);

    let reported = borrow_into_controller(
        env,
        account,
        debt,
        amount,
        false,
        PositionAction::FlashPos,
        cache,
    );

    let measured = balance_delta_since(env, &debt.asset, &controller, before);
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
        let delta = balance_delta_since(env, &hub_asset.asset, controller, baseline);
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

/// Requires positive scaled debt in the borrowed market and remaining supply
/// so the flash-position flow cannot finish as an empty round trip.
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
