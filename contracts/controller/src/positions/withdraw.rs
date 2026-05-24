use common::errors::{CollateralError, GenericError};
use common::math::fp::{Ray, Wad};
use common::types::{Account, AccountPosition, AccountPositionType, Payment, PoolPositionMutation};
use soroban_sdk::{contractimpl, panic_with_error, symbol_short, Address, Env, Vec};
use stellar_macros::when_not_paused;

use super::EventContext;

use super::dust::require_no_supply_dust_for_assets;
use super::update;
use crate::cache::ControllerCache;
use crate::cross_contract::pool::pool_withdraw_call;
use crate::oracle::policy::OraclePolicy;
use crate::{storage, utils::*, validation, Controller, ControllerArgs, ControllerClient};

// Sentinel for full-position withdraw.
const WITHDRAW_ALL_SENTINEL: i128 = i128::MAX;

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
    #[when_not_paused]
    pub fn withdraw(env: Env, caller: Address, account_id: u64, withdrawals: Vec<(Address, i128)>) {
        process_withdraw(&env, &caller, account_id, &withdrawals);
    }
}

// Processes withdraw batch.
pub fn process_withdraw(env: &Env, caller: &Address, account_id: u64, withdrawals: &Vec<Payment>) {
    // Stage 1: Pipelined Context Check
    caller.require_auth();
    validation::require_not_flash_loaning(env);

    // Stage 2: State Resolution
    let mut account = storage::get_account(env, account_id);

    validation::require_account_owner_match(env, &account, caller);

    let policy = if account.borrow_positions.is_empty() {
        OraclePolicy::RiskDecreasing
    } else {
        OraclePolicy::RiskIncreasing
    };
    let mut cache = ControllerCache::new(env, policy);

    // Stage 3 & 4: Pre-flight Validation & Core Pool Execution
    // Aggregate once and reuse for the loop AND the post-flight dust scope.
    let withdrawal_plan = aggregate_payments(env, withdrawals, true);
    for (asset, amount) in withdrawal_plan.iter() {
        process_single_withdrawal(env, caller, &mut account, &asset, amount, &mut cache);
    }

    // Stage 5: Post-flight Risk Gates
    // Enforce HF and LTV gates.
    validation::require_within_ltv(env, &mut cache, &account);
    validation::require_healthy_account(env, &mut cache, &account);
    // Dust gate is scoped to the withdrawn assets — withdraw never mutates
    // borrow positions and must not be blocked by pre-existing positions
    // that drifted under the floor on assets the user did not touch.
    require_no_supply_dust_for_assets(env, &mut cache, &account, &plan_assets(env, &withdrawal_plan));

    // Stage 6: State Persistence
    if account.is_empty() {
        remove_account(env, account_id);
    } else {
        // Mutates supply positions only.
        storage::set_supply_positions(env, account_id, &account.supply_positions);
    }
    cache.emit_position_batch(account_id, &account);
    cache.emit_market_batch();
}

fn process_single_withdrawal(
    env: &Env,
    caller: &Address,
    account: &mut Account,
    asset: &Address,
    amount: i128,
    cache: &mut ControllerCache,
) {
    // `0` means withdraw all; negative withdrawals are never valid.
    if amount < 0 {
        panic_with_error!(env, GenericError::AmountMustBePositive);
    }

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

    let _ = execute_withdrawal(
        env,
        account,
        EventContext {
            caller: caller.clone(),
            event_caller: caller.clone(),
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
}

// Executes pool withdraw.
pub fn execute_withdrawal(
    env: &Env,
    account: &mut Account,
    ctx: EventContext,
    req: WithdrawalRequest<'_>,
    flags: WithdrawFlags,
    cache: &mut ControllerCache,
) -> PoolPositionMutation {
    let EventContext {
        caller,
        event_caller,
        action,
    } = ctx;
    let pool_addr = cache.cached_pool_address(req.asset);
    let result = pool_withdraw_call(
        env,
        &pool_addr,
        caller.clone(),
        req.amount,
        req.position.into(),
        flags.is_liquidation,
        flags.protocol_fee,
    );
    cache.record_market_update_with_price(&result.market_state, Some(req.price.raw()));
    // Merge ONLY the scaled share back; preserve the collateral risk params the
    // pool does not echo.
    let mut result_position = *req.position;
    result_position.scaled_amount = Ray::from_raw(result.position.scaled_amount_ray);
    update::update_or_remove_supply_position(account, req.asset, &result_position);

    let _ = event_caller;
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
