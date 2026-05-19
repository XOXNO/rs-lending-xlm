#![no_std]
#![allow(clippy::too_many_arguments)]
mod cache;
mod interest;
mod utils;
mod views;

#[cfg(test)]
mod test_support;

#[cfg(feature = "certora")]
#[path = "../../verification/certora/pool/spec/mod.rs"]
pub mod spec;

use cache::Cache;
use common::constants::RAY;
#[cfg(test)]
use common::constants::TTL_THRESHOLD_INSTANCE;
use common::errors::{FlashLoanError, GenericError};
use common::fp::Ray;
use common::rates::update_supply_index;
use common::types::{
    AccountPosition, AccountPositionType, InterestRateModel, MarketParams, MarketStateSnapshot,
    PoolAmountMutation, PoolKey, PoolPositionMutation, PoolState, PoolStrategyMutation,
    PoolSyncData,
};
use soroban_sdk::{
    contract, contractimpl, panic_with_error, token, Address, Bytes, BytesN, Env, IntoVal, Symbol,
};

use stellar_access::ownable;
use stellar_macros::only_owner;

use utils::{
    apply_liquidation_fee, apply_rate_model, authorize_token_transfer_from, enforce_borrow_cap,
    enforce_supply_cap, renew_pool_instance, require_nonneg_amount, require_positive_amount,
    require_wasm_receiver,
};

#[contract]
pub struct LiquidityPool;

#[contractimpl]
impl LiquidityPool {
    #[allow(clippy::too_many_arguments)]
    pub fn __constructor(env: Env, admin: Address, params: MarketParams) {
        params.verify_rate_model(&env);

        ownable::set_owner(&env, &admin);

        env.storage().instance().set(&PoolKey::Params, &params);

        let state = PoolState {
            supplied_ray: 0,
            borrowed_ray: 0,
            revenue_ray: 0,
            borrow_index_ray: RAY,
            supply_index_ray: RAY,
            last_timestamp: env.ledger().timestamp() * 1000,
        };
        env.storage().instance().set(&PoolKey::State, &state);
    }

    #[only_owner]
    pub fn supply(
        env: Env,
        mut position: AccountPosition,
        amount: i128,
        supply_cap: i128,
    ) -> PoolPositionMutation {
        require_nonneg_amount(&env, amount);
        let mut cache = Cache::load(&env);
        interest::global_sync(&env, &mut cache);

        let scaled_amount = cache.calculate_scaled_supply(amount);
        enforce_supply_cap(&env, &cache, scaled_amount, supply_cap);
        position.scaled_amount_ray += scaled_amount.raw();
        cache.supplied += scaled_amount;

        let mutation = cache.position_mutation(position, amount);
        cache.save();
        mutation
    }

    #[only_owner]
    pub fn borrow(
        env: Env,
        caller: Address,
        amount: i128,
        mut position: AccountPosition,
        borrow_cap: i128,
    ) -> PoolPositionMutation {
        require_nonneg_amount(&env, amount);
        let mut cache = Cache::load(&env);
        interest::global_sync(&env, &mut cache);

        cache.require_reserves(amount);

        let scaled_debt = cache.calculate_scaled_borrow(amount);
        enforce_borrow_cap(&env, &cache, scaled_debt, borrow_cap);
        position.scaled_amount_ray += scaled_debt.raw();
        cache.borrowed += scaled_debt;
        // Max-utilization ceiling.
        utils::require_utilization_below_max(&env, &cache);

        cache.transfer_out(&caller, amount);

        let mutation = cache.position_mutation(position, amount);
        cache.save();
        mutation
    }

    #[only_owner]
    pub fn withdraw(
        env: Env,
        caller: Address,
        amount: i128,
        mut position: AccountPosition,
        is_liquidation: bool,
        protocol_fee: i128,
    ) -> PoolPositionMutation {
        // Controller uses i128::MAX for "withdraw all".
        require_nonneg_amount(&env, amount);
        require_nonneg_amount(&env, protocol_fee);
        let mut cache = Cache::load(&env);
        interest::global_sync(&env, &mut cache);

        let pos_scaled = Ray::from_raw(position.scaled_amount_ray);
        let (scaled_withdrawal, gross_amount) = cache.resolve_withdrawal(amount, pos_scaled);

        let net_transfer =
            apply_liquidation_fee(&env, &mut cache, gross_amount, is_liquidation, protocol_fee);

        cache.require_reserves(net_transfer);

        cache.supplied.checked_sub_assign(&env, scaled_withdrawal);
        position.scaled_amount_ray = pos_scaled.checked_sub(&env, scaled_withdrawal).raw();
        // Max-utilization ceiling.
        if !is_liquidation {
            utils::require_utilization_below_max(&env, &cache);
        }
        utils::require_solvent_withdraw_state(&env, &cache);

        cache.transfer_out(&caller, net_transfer);

        let mutation = cache.position_mutation(position, gross_amount);
        cache.save();
        mutation
    }

    #[only_owner]
    pub fn repay(
        env: Env,
        caller: Address,
        amount: i128,
        mut position: AccountPosition,
    ) -> PoolPositionMutation {
        require_nonneg_amount(&env, amount);
        let mut cache = Cache::load(&env);
        interest::global_sync(&env, &mut cache);

        let pos_scaled = Ray::from_raw(position.scaled_amount_ray);
        let (scaled_repay, overpayment) = cache.resolve_repay(amount, pos_scaled);

        position.scaled_amount_ray = pos_scaled.checked_sub(&env, scaled_repay).raw();
        cache.borrowed.checked_sub_assign(&env, scaled_repay);

        cache.transfer_out(&caller, overpayment);

        let mutation = cache.position_mutation(position, amount - overpayment);
        cache.save();
        mutation
    }

    #[only_owner]
    pub fn update_indexes(env: Env) -> MarketStateSnapshot {
        let mut cache = Cache::load(&env);
        interest::global_sync(&env, &mut cache);

        let result = cache.market_snapshot();
        cache.save();
        result
    }

    #[only_owner]
    pub fn add_rewards(env: Env, amount: i128) -> MarketStateSnapshot {
        require_nonneg_amount(&env, amount);
        let mut cache = Cache::load(&env);

        if cache.supplied == Ray::ZERO {
            panic_with_error!(&env, GenericError::NoSuppliersToReward);
        }

        interest::global_sync(&env, &mut cache);

        let amount_ray = Ray::from_asset(amount, cache.params.asset_decimals);
        cache.supply_index =
            update_supply_index(&env, cache.supplied, cache.supply_index, amount_ray);

        let result = cache.market_snapshot();
        cache.save();
        result
    }

    #[only_owner]
    pub fn flash_loan(
        env: Env,
        initiator: Address,
        receiver: Address,
        amount: i128,
        fee: i128,
        data: Bytes,
    ) -> MarketStateSnapshot {
        require_positive_amount(&env, amount);
        require_nonneg_amount(&env, fee);

        let mut cache = Cache::load(&env);
        interest::global_sync(&env, &mut cache);

        cache.require_reserves(amount);
        require_wasm_receiver(&env, &receiver);

        // Snapshot balance locally to prevent settlement with different asset.
        let pool_addr = env.current_contract_address();
        let tok = token::Client::new(&env, &cache.params.asset_id);
        let pre_balance = tok.balance(&pool_addr);
        let expected_after_payout = pre_balance - amount;
        let total = amount + fee;
        let expected_after_repay = pre_balance + fee;

        tok.transfer(&pool_addr, &receiver, &amount);

        // Pre-callback sanity.
        if tok.balance(&pool_addr) != expected_after_payout {
            panic_with_error!(&env, FlashLoanError::InvalidFlashloanRepay);
        }

        env.invoke_contract::<()>(
            &receiver,
            &Symbol::new(&env, "execute_flash_loan"),
            (
                initiator,
                cache.params.asset_id.clone(),
                amount,
                fee,
                pool_addr.clone(),
                data,
            )
                .into_val(&env),
        );

        // Post-callback: balance must not have been mutated.
        if tok.balance(&pool_addr) != expected_after_payout {
            panic_with_error!(&env, FlashLoanError::InvalidFlashloanRepay);
        }

        authorize_token_transfer_from(&env, &cache.params.asset_id, &receiver, &pool_addr, total);
        tok.transfer_from(&pool_addr, &receiver, &pool_addr, &total);

        if tok.balance(&pool_addr) != expected_after_repay {
            panic_with_error!(&env, FlashLoanError::InvalidFlashloanRepay);
        }

        let fee_ray = Ray::from_asset(fee, cache.params.asset_decimals);
        interest::add_protocol_revenue_ray(&mut cache, fee_ray);

        let result = cache.market_snapshot();
        cache.save();
        result
    }

    #[only_owner]
    pub fn create_strategy(
        env: Env,
        caller: Address,
        mut position: AccountPosition,
        amount: i128,
        fee: i128,
        borrow_cap: i128,
    ) -> PoolStrategyMutation {
        require_nonneg_amount(&env, amount);
        require_nonneg_amount(&env, fee);

        if fee > amount {
            panic_with_error!(&env, FlashLoanError::StrategyFeeExceeds);
        }

        let mut cache = Cache::load(&env);
        cache.require_reserves(amount);

        interest::global_sync(&env, &mut cache);

        let scaled_debt = cache.calculate_scaled_borrow(amount);
        enforce_borrow_cap(&env, &cache, scaled_debt, borrow_cap);
        position.scaled_amount_ray += scaled_debt.raw();
        cache.borrowed += scaled_debt;
        // Max-utilization ceiling.
        utils::require_utilization_below_max(&env, &cache);

        let fee_ray = Ray::from_asset(fee, cache.params.asset_decimals);
        interest::add_protocol_revenue_ray(&mut cache, fee_ray);

        let amount_to_send = amount - fee;
        cache.transfer_out(&caller, amount_to_send);

        let mutation = cache.strategy_mutation(position, amount, amount_to_send);
        cache.save();
        mutation
    }

    #[only_owner]
    pub fn seize_position(
        env: Env,
        side: AccountPositionType,
        mut position: AccountPosition,
    ) -> PoolPositionMutation {
        let mut cache = Cache::load(&env);
        interest::global_sync(&env, &mut cache);

        match side {
            AccountPositionType::Borrow => {
                // Socialize bad debt.
                let current_debt_ray =
                    cache.unscale_borrow_ray(Ray::from_raw(position.scaled_amount_ray));
                interest::apply_bad_debt_to_supply_index(&mut cache, current_debt_ray);
                cache
                    .borrowed
                    .checked_sub_assign(&env, Ray::from_raw(position.scaled_amount_ray));
                position.scaled_amount_ray = 0;
            }
            AccountPositionType::Deposit => {
                // Absorb dust into revenue.
                cache.revenue += Ray::from_raw(position.scaled_amount_ray);
                position.scaled_amount_ray = 0;
            }
        }

        let mutation = cache.position_mutation(position, 0);
        cache.save();
        mutation
    }

    #[only_owner]
    pub fn claim_revenue(env: Env) -> PoolAmountMutation {
        let mut cache = Cache::load(&env);
        interest::global_sync(&env, &mut cache);

        // Defensive: revenue must be non-negative.
        if cache.revenue.raw() < 0 {
            panic_with_error!(&env, GenericError::MathOverflow);
        }

        let amount_to_transfer = cache.burn_claimable_revenue();

        // Reject insolvent post-state.
        utils::require_solvent_withdraw_state(&env, &cache);

        // CEI: commit state before external call.
        let mutation = cache.amount_mutation(amount_to_transfer);
        cache.save();

        if amount_to_transfer > 0 {
            // Revenue routes to pool owner.
            let owner = ownable::get_owner(&env)
                .unwrap_or_else(|| panic_with_error!(&env, GenericError::OwnerNotSet));
            cache.transfer_out(&owner, amount_to_transfer);
        }
        mutation
    }

    #[allow(clippy::too_many_arguments)]
    #[only_owner]
    pub fn update_params(
        env: Env,
        max_borrow_rate: i128,
        base_borrow_rate: i128,
        slope1: i128,
        slope2: i128,
        slope3: i128,
        mid_utilization: i128,
        optimal_utilization: i128,
        max_utilization: i128,
        reserve_factor: u32,
    ) {
        // Accrue at old rate model before applying new.
        let mut cache = Cache::load(&env);
        interest::global_sync(&env, &mut cache);
        cache.save();

        let model = InterestRateModel {
            max_borrow_rate_ray: max_borrow_rate,
            base_borrow_rate_ray: base_borrow_rate,
            slope1_ray: slope1,
            slope2_ray: slope2,
            slope3_ray: slope3,
            mid_utilization_ray: mid_utilization,
            optimal_utilization_ray: optimal_utilization,
            // Governance adjustable ceiling.
            max_utilization_ray: max_utilization,
            reserve_factor_bps: reserve_factor,
        };
        model.verify(&env);
        apply_rate_model(&env, &model);
    }

    #[only_owner]
    pub fn upgrade(env: Env, new_wasm_hash: BytesN<32>) {
        stellar_contract_utils::upgradeable::upgrade(&env, &new_wasm_hash);
    }

    #[only_owner]
    pub fn keepalive(env: Env) {
        renew_pool_instance(&env);
    }

    // Read-only views.

    pub fn capital_utilisation(env: Env) -> i128 {
        views::capital_utilisation(&env)
    }

    pub fn reserves(env: Env) -> i128 {
        views::reserves(&env)
    }

    pub fn deposit_rate(env: Env) -> i128 {
        views::deposit_rate(&env)
    }

    pub fn borrow_rate(env: Env) -> i128 {
        views::borrow_rate(&env)
    }

    pub fn protocol_revenue(env: Env) -> i128 {
        views::protocol_revenue(&env)
    }

    pub fn supplied_amount(env: Env) -> i128 {
        views::supplied_amount(&env)
    }

    pub fn borrowed_amount(env: Env) -> i128 {
        views::borrowed_amount(&env)
    }

    pub fn delta_time(env: Env) -> u64 {
        views::delta_time(&env)
    }

    pub fn get_sync_data(env: Env) -> PoolSyncData {
        let params: MarketParams = env
            .storage()
            .instance()
            .get(&PoolKey::Params)
            .unwrap_or_else(|| panic_with_error!(&env, GenericError::PoolNotInitialized));
        let state: PoolState = env
            .storage()
            .instance()
            .get(&PoolKey::State)
            .unwrap_or_else(|| panic_with_error!(&env, GenericError::PoolNotInitialized));

        PoolSyncData { params, state }
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests;
