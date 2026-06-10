use common::errors::{CollateralError, GenericError};
use common::math::fp::{Ray, Wad};
use common::types::{
    Account, AccountPosition, AccountPositionType, Payment, PoolAction, PoolPositionMutation,
};
use soroban_sdk::{
    assert_with_error, contractimpl, panic_with_error, symbol_short, Address, Env, Vec,
};
use stellar_macros::when_not_paused;

use crate::utils::EventContext;

use crate::cache::Cache;
use crate::cross_contract::pool::pool_withdraw_call;
use crate::helpers::{
    refresh_supply_risk_params_for_asset, require_no_supply_dust_for_assets,
    update_or_remove_supply_position,
};
use crate::oracle::policy::OraclePolicy;
use crate::{storage, utils::*, validation, Controller, ControllerArgs, ControllerClient};

/// Pool ABI sentinel for full-position withdraw (`withdraw` maps user `0` here).
pub(crate) const WITHDRAW_ALL_SENTINEL: i128 = i128::MAX;

/// Per-call withdrawal inputs that travel together through the pipeline.
pub(crate) struct WithdrawalRequest<'a> {
    pub asset: &'a Address,
    pub amount: i128,
    pub position: &'a AccountPosition,
    pub price: Wad,
}

/// Liquidation-only modifiers; default is a plain withdraw.
#[derive(Clone, Copy)]
pub(crate) struct WithdrawFlags {
    pub is_liquidation: bool,
    pub protocol_fee: i128,
}

impl WithdrawFlags {
    pub fn plain() -> Self {
        Self {
            is_liquidation: false,
            protocol_fee: 0,
        }
    }
}

#[contractimpl]
impl Controller {
    /// Tokens go to `to` (else `caller`); returns actual paid per asset.
    #[when_not_paused]
    pub fn withdraw(
        env: Env,
        caller: Address,
        account_id: u64,
        withdrawals: Vec<(Address, i128)>,
        to: Option<Address>,
    ) -> Vec<(Address, i128)> {
        process_withdraw(&env, &caller, account_id, &withdrawals, to)
    }
}

/// Withdraws collateral and re-checks LTV, health factor, and touched-asset dust.
///
/// User amount `0` maps to the pool's `i128::MAX` full-withdraw sentinel.
pub fn process_withdraw(
    env: &Env,
    caller: &Address,
    account_id: u64,
    withdrawals: &Vec<Payment>,
    to: Option<Address>,
) -> Vec<Payment> {
    caller.require_auth();
    validation::require_not_flash_loaning(env);

    let mut account = storage::get_account(env, account_id);

    validation::require_account_owner_match(env, &account, caller);

    let recipient = to.unwrap_or_else(|| caller.clone());

    let policy = if account.borrow_positions.is_empty() {
        OraclePolicy::RiskDecreasing
    } else {
        OraclePolicy::RiskIncreasing
    };
    let mut cache = Cache::new(env, policy);

    // Aggregate once and reuse for the loop AND the post-flight dust scope.
    let withdrawal_plan = aggregate_payments(env, withdrawals, true);
    let mut paid: Vec<Payment> = Vec::new(env);
    for (asset, amount) in withdrawal_plan.iter() {
        let actual =
            process_single_withdrawal(env, &recipient, &mut account, &asset, amount, &mut cache);
        paid.push_back((asset, actual));
    }

    validation::require_within_ltv(env, &mut cache, &account);
    validation::require_healthy_account(env, &mut cache, &account);
    // Dust gate scoped to withdrawn assets only: withdraw never touches borrow
    // positions, so untouched positions that drifted under the floor must not block it.
    require_no_supply_dust_for_assets(
        env,
        &mut cache,
        &account,
        &plan_assets(env, &withdrawal_plan),
    );

    if account.is_empty() {
        remove_account(env, account_id);
    } else {
        // Mutates supply positions only.
        storage::set_supply_positions(env, account_id, &account.supply_positions);
    }
    cache.emit_position_batch(account_id, &account);
    cache.emit_market_batch();

    paid
}

/// Returns the actual amount the pool paid out for this asset.
fn process_single_withdrawal(
    env: &Env,
    recipient: &Address,
    account: &mut Account,
    asset: &Address,
    amount: i128,
    cache: &mut Cache,
) -> i128 {
    // `0` means withdraw all; negative withdrawals are never valid.
    assert_with_error!(env, amount >= 0, GenericError::AmountMustBePositive);

    let feed = cache.cached_price(asset);

    let position: AccountPosition = match account.supply_positions.get(asset.clone()) {
        Some(pos) => (&pos).into(),
        None => panic_with_error!(env, CollateralError::PositionNotFound),
    };

    let withdraw_amount = if amount == 0 {
        WITHDRAW_ALL_SENTINEL
    } else {
        amount
    };

    let result = execute_withdrawal(
        env,
        account,
        EventContext {
            caller: recipient.clone(),
            action: symbol_short!("withdraw"),
        },
        WithdrawalRequest {
            asset,
            amount: withdraw_amount,
            position: &position,
            price: feed.price,
        },
        WithdrawFlags::plain(),
        cache,
    );
    result.actual_amount
}

/// Calls the pool and merges the returned scaled supply share into the account.
pub fn execute_withdrawal(
    env: &Env,
    account: &mut Account,
    ctx: EventContext,
    req: WithdrawalRequest<'_>,
    flags: WithdrawFlags,
    cache: &mut Cache,
) -> PoolPositionMutation {
    let EventContext { caller, action } = ctx;
    let pool_addr = cache.cached_pool_address();
    let pool_action = PoolAction {
        caller: caller.clone(),
        position: req.position.into(),
        amount: req.amount,
        asset: req.asset.clone(),
    };
    let result = pool_withdraw_call(
        env,
        &pool_addr,
        pool_action,
        flags.is_liquidation,
        flags.protocol_fee,
    );
    cache.record_market_update_with_price(&result.market_state, Some(req.price.raw()));
    let mut result_position = *req.position;
    result_position.scaled_amount = Ray::from(result.position.scaled_amount_ray);
    if !flags.is_liquidation {
        refresh_supply_risk_params_for_asset(env, cache, account, req.asset, &mut result_position);
    }
    update_or_remove_supply_position(account, req.asset, &result_position);

    cache.record_position_update(
        action,
        AccountPositionType::Deposit,
        req.asset,
        result.market_index.supply_index_ray,
        result.actual_amount,
        &result_position,
        Some(req.price.raw()),
    );

    result
}
