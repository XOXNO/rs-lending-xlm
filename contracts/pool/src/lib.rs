#![no_std]
mod cache;
mod interest;
mod utils;
mod views;

#[cfg(test)]
mod test_support;

#[cfg(feature = "certora")]
#[path = "../../../verification/certora/pool/spec/mod.rs"]
pub mod spec;

use cache::Cache;
use common::constants::{MS_PER_SECOND, RAY};
use common::errors::{FlashLoanError, GenericError};
use common::math::fp::Ray;
use common::rates::update_supply_index;
use common::types::{
    AccountPositionType, InterestRateModel, MarketParamsRaw, MarketStateSnapshot, PoolAction,
    PoolAmountMutation, PoolKey, PoolPositionMutation, PoolStateRaw, PoolStrategyMutation,
    PoolSyncData, ScaledPositionRaw,
};
use pool_interface::LiquidityPoolInterface;
use soroban_sdk::{
    assert_with_error, contract, contractimpl, contractmeta, panic_with_error, token, Address,
    Bytes, BytesN, Env, IntoVal, Symbol,
};

contractmeta!(key = "name", val = "Liquidity Pool");
contractmeta!(key = "binver", val = env!("CARGO_PKG_VERSION"));
contractmeta!(
    key = "repo",
    val = "https://github.com/xoxno/rs-lending-xlm"
);

use stellar_access::ownable;
use stellar_macros::only_owner;

use utils::{
    apply_liquidation_fee, apply_rate_model, authorize_token_transfer_from, enforce_borrow_cap,
    enforce_supply_cap, renew_market_keys, renew_pool_instance, require_nonneg_amount,
    require_positive_amount, require_wasm_receiver,
};

fn load_synced_cache(env: &Env, asset: &Address) -> Cache {
    renew_pool_instance(env);
    let mut cache = Cache::load(env, asset);
    interest::global_sync(env, &mut cache);
    cache
}

#[contract]
pub struct LiquidityPool;

// Soroban constructors cannot be declared in contractclient traits.
#[contractimpl]
impl LiquidityPool {
    pub fn __constructor(env: Env, admin: Address) {
        ownable::set_owner(&env, &admin);
    }
}

// This impl is the pool ABI; signatures must match `LiquidityPoolInterface`.
#[contractimpl]
impl LiquidityPoolInterface for LiquidityPool {
    #[only_owner]
    fn create_market(env: Env, params: MarketParamsRaw) {
        renew_pool_instance(&env);
        params.verify(&env);

        let asset = params.asset_id.clone();
        assert_with_error!(
            &env,
            !env.storage()
                .persistent()
                .has(&PoolKey::Params(asset.clone())),
            GenericError::AssetAlreadySupported
        );

        env.storage()
            .persistent()
            .set(&PoolKey::Params(asset.clone()), &params);

        let state = PoolStateRaw {
            supplied_ray: 0,
            borrowed_ray: 0,
            revenue_ray: 0,
            borrow_index_ray: RAY,
            supply_index_ray: RAY,
            last_timestamp: env.ledger().timestamp() * MS_PER_SECOND,
            cash: 0,
        };
        env.storage()
            .persistent()
            .set(&PoolKey::State(asset.clone()), &state);

        renew_market_keys(&env, &asset);
    }

    #[only_owner]
    fn supply(env: Env, action: PoolAction, supply_cap: i128) -> PoolPositionMutation {
        // `caller` is carried but unused: the controller pre-transfers the tokens.
        let PoolAction {
            position,
            amount,
            asset,
            ..
        } = action;
        require_nonneg_amount(&env, amount);
        let mut cache = load_synced_cache(&env, &asset);

        let mut scaled = Ray::from(position.scaled_amount_ray);
        let scaled_amount = cache.calculate_scaled_supply(amount);

        enforce_supply_cap(&env, &cache, scaled_amount, supply_cap);

        scaled += scaled_amount;
        cache.supplied += scaled_amount;
        // Controller already transferred `amount` into the pool before this call.
        cache.cash += amount;

        cache.save();
        cache.position_mutation(scaled, amount)
    }

    #[only_owner]
    fn borrow(env: Env, action: PoolAction, borrow_cap: i128) -> PoolPositionMutation {
        let PoolAction {
            caller,
            position,
            amount,
            asset,
        } = action;
        require_nonneg_amount(&env, amount);
        let mut cache = load_synced_cache(&env, &asset);

        cache.require_reserves(amount);

        let mut scaled = Ray::from(position.scaled_amount_ray);
        let scaled_debt = cache.calculate_scaled_borrow(amount);

        enforce_borrow_cap(&env, &cache, scaled_debt, borrow_cap);

        scaled += scaled_debt;
        cache.borrowed += scaled_debt;
        // Borrow cannot leave the pool above its max-utilization cap.
        utils::require_utilization_below_max(&env, &cache);
        cache.cash -= amount;

        // CEI: snapshot + commit before external call.
        cache.save();
        cache.transfer_out(&caller, amount);
        cache.position_mutation(scaled, amount)
    }

    #[only_owner]
    fn withdraw(
        env: Env,
        action: PoolAction,
        is_liquidation: bool,
        protocol_fee: i128,
    ) -> PoolPositionMutation {
        let PoolAction {
            caller,
            position,
            amount,
            asset,
        } = action;
        // Controller maps user amount `0` to this full-withdraw sentinel.
        require_nonneg_amount(&env, amount);
        require_nonneg_amount(&env, protocol_fee);
        let mut cache = load_synced_cache(&env, &asset);

        let mut scaled = Ray::from(position.scaled_amount_ray);
        let (scaled_withdrawal, gross_amount) = cache.resolve_withdrawal(amount, scaled);

        let net_transfer =
            apply_liquidation_fee(&env, &mut cache, gross_amount, is_liquidation, protocol_fee);

        cache.require_reserves(net_transfer);

        cache.supplied.checked_sub_assign(&env, scaled_withdrawal);
        scaled = scaled.checked_sub(&env, scaled_withdrawal);

        // User withdrawals cannot leave the pool above max utilization.
        if !is_liquidation {
            utils::require_utilization_below_max(&env, &cache);
        }
        utils::require_solvent_withdraw_state(&env, &cache);
        cache.cash -= net_transfer;

        // CEI: snapshot + commit before external call.
        cache.save();
        cache.transfer_out(&caller, net_transfer);
        cache.position_mutation(scaled, gross_amount)
    }

    #[only_owner]
    fn repay(env: Env, action: PoolAction) -> PoolPositionMutation {
        let PoolAction {
            caller,
            position,
            amount,
            asset,
        } = action;
        require_nonneg_amount(&env, amount);
        let mut cache = load_synced_cache(&env, &asset);

        let mut scaled = Ray::from(position.scaled_amount_ray);
        let (scaled_repay, overpayment) = cache.resolve_repay(amount, scaled);

        scaled = scaled.checked_sub(&env, scaled_repay);
        cache.borrowed.checked_sub_assign(&env, scaled_repay);
        // Controller moved `amount` in; the `overpayment` is refunded below.
        cache.cash += amount - overpayment;

        // CEI: snapshot + commit before external call.
        cache.save();
        cache.transfer_out(&caller, overpayment);
        cache.position_mutation(scaled, amount - overpayment)
    }

    #[only_owner]
    fn update_indexes(env: Env, asset: Address) -> MarketStateSnapshot {
        let cache = load_synced_cache(&env, &asset);
        cache.save();
        cache.market_snapshot()
    }

    #[only_owner]
    fn add_rewards(env: Env, asset: Address, amount: i128) -> MarketStateSnapshot {
        require_nonneg_amount(&env, amount);
        let mut cache = load_synced_cache(&env, &asset);

        assert_with_error!(
            &env,
            cache.supplied != Ray::ZERO,
            GenericError::NoSuppliersToReward
        );

        let amount_ray = Ray::from_asset(amount, cache.params.asset_decimals);
        cache.supply_index =
            update_supply_index(&env, cache.supplied, cache.supply_index, amount_ray);
        // Controller transferred `amount` of reward tokens into the pool.
        cache.cash += amount;

        cache.save();
        cache.market_snapshot()
    }

    #[only_owner]
    fn flash_loan(
        env: Env,
        asset: Address,
        initiator: Address,
        receiver: Address,
        amount: i128,
        fee: i128,
        data: Bytes,
    ) -> MarketStateSnapshot {
        require_positive_amount(&env, amount);
        require_nonneg_amount(&env, fee);

        let mut cache = load_synced_cache(&env, &asset);

        cache.require_reserves(amount);
        require_wasm_receiver(&env, &receiver);

        // Balance checks prevent repayment with any asset other than the loaned
        // token; balances are per-(token, holder) so other vault assets are inert.
        let pool_addr = env.current_contract_address();
        let tok = token::Client::new(&env, &cache.params.asset_id);
        let pre_balance = tok.balance(&pool_addr);
        let expected_after_payout = pre_balance - amount;
        let total = amount + fee;
        let expected_after_repay = pre_balance + fee;

        tok.transfer(&pool_addr, &receiver, &amount);

        assert_with_error!(
            &env,
            tok.balance(&pool_addr) == expected_after_payout,
            FlashLoanError::InvalidFlashloanRepay
        );

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

        // The callback must not retain funds or change the pool balance again.
        assert_with_error!(
            &env,
            tok.balance(&pool_addr) == expected_after_payout,
            FlashLoanError::InvalidFlashloanRepay
        );

        // Receiver must approve `amount + fee` during the callback. Check allowance
        // before transfer_from so SAC failures surface as InvalidFlashloanRepay (#402)
        // instead of bubbling token error codes.
        assert_with_error!(
            &env,
            tok.allowance(&receiver, &pool_addr) >= total,
            FlashLoanError::InvalidFlashloanRepay
        );
        authorize_token_transfer_from(&env, &cache.params.asset_id, &receiver, &pool_addr, total);
        tok.transfer_from(&pool_addr, &receiver, &pool_addr, &total);

        assert_with_error!(
            &env,
            tok.balance(&pool_addr) == expected_after_repay,
            FlashLoanError::InvalidFlashloanRepay
        );

        let fee_ray = Ray::from_asset(fee, cache.params.asset_decimals);
        interest::add_protocol_revenue_ray(&mut cache, fee_ray);
        // Net token effect: out `amount`, back `amount + fee` → +fee. The loan
        // uses direct token transfers (with balance assertions), not transfer_out.
        cache.cash += fee;

        cache.save();
        cache.market_snapshot()
    }

    #[only_owner]
    fn create_strategy(
        env: Env,
        action: PoolAction,
        fee: i128,
        borrow_cap: i128,
    ) -> PoolStrategyMutation {
        let PoolAction {
            caller,
            position,
            amount,
            asset,
        } = action;
        require_nonneg_amount(&env, amount);
        require_nonneg_amount(&env, fee);

        assert_with_error!(&env, fee <= amount, FlashLoanError::StrategyFeeExceeds);

        let mut cache = load_synced_cache(&env, &asset);
        cache.require_reserves(amount);

        let mut scaled = Ray::from(position.scaled_amount_ray);
        let scaled_debt = cache.calculate_scaled_borrow(amount);

        enforce_borrow_cap(&env, &cache, scaled_debt, borrow_cap);

        scaled += scaled_debt;
        cache.borrowed += scaled_debt;
        // Strategy debt cannot leave the pool above max utilization.
        utils::require_utilization_below_max(&env, &cache);

        let fee_ray = Ray::from_asset(fee, cache.params.asset_decimals);
        interest::add_protocol_revenue_ray(&mut cache, fee_ray);

        let amount_to_send = amount - fee;
        cache.cash -= amount_to_send;

        // CEI: snapshot + commit before external call.
        cache.save();
        cache.transfer_out(&caller, amount_to_send);
        cache.strategy_mutation(scaled, amount, amount_to_send)
    }

    #[only_owner]
    fn seize_position(
        env: Env,
        asset: Address,
        side: AccountPositionType,
        position: ScaledPositionRaw,
    ) -> PoolPositionMutation {
        let mut cache = load_synced_cache(&env, &asset);

        let scaled = Ray::from(position.scaled_amount_ray);
        match side {
            AccountPositionType::Borrow => {
                let current_debt_ray = cache.unscale_borrow_ray(scaled);
                interest::apply_bad_debt_to_supply_index(&mut cache, current_debt_ray);
                cache.borrowed.checked_sub_assign(&env, scaled);
            }
            AccountPositionType::Deposit => {
                cache.revenue += scaled;
            }
        }

        // The seized position is removed from the controller-owned account map.
        cache.save();
        cache.position_mutation(Ray::ZERO, 0)
    }

    #[only_owner]
    fn claim_revenue(env: Env, asset: Address) -> PoolAmountMutation {
        let mut cache = load_synced_cache(&env, &asset);

        assert_with_error!(&env, cache.revenue >= Ray::ZERO, GenericError::MathOverflow);

        let amount_to_transfer = cache.burn_claimable_revenue();

        utils::require_solvent_withdraw_state(&env, &cache);
        cache.cash -= amount_to_transfer;

        // CEI: commit state before external call.
        cache.save();

        if amount_to_transfer > 0 {
            let owner = ownable::get_owner(&env)
                .unwrap_or_else(|| panic_with_error!(&env, GenericError::OwnerNotSet));
            cache.transfer_out(&owner, amount_to_transfer);
        }

        cache.amount_mutation(amount_to_transfer)
    }

    #[only_owner]
    fn update_params(env: Env, asset: Address, model: InterestRateModel) {
        // Accrue at the old rate model before replacing it.
        let cache = load_synced_cache(&env, &asset);
        cache.save();

        model.verify(&env);
        apply_rate_model(&env, &asset, &model);
    }

    #[only_owner]
    fn upgrade(env: Env, new_wasm_hash: BytesN<32>) {
        renew_pool_instance(&env);
        stellar_contract_utils::upgradeable::upgrade(&env, &new_wasm_hash);
    }

    fn capital_utilisation(env: Env, asset: Address) -> i128 {
        views::capital_utilisation(&env, &asset)
    }

    fn reserves(env: Env, asset: Address) -> i128 {
        views::reserves(&env, &asset)
    }

    fn deposit_rate(env: Env, asset: Address) -> i128 {
        views::deposit_rate(&env, &asset)
    }

    fn borrow_rate(env: Env, asset: Address) -> i128 {
        views::borrow_rate(&env, &asset)
    }

    fn protocol_revenue(env: Env, asset: Address) -> i128 {
        views::protocol_revenue(&env, &asset)
    }

    fn supplied_amount(env: Env, asset: Address) -> i128 {
        views::supplied_amount(&env, &asset)
    }

    fn borrowed_amount(env: Env, asset: Address) -> i128 {
        views::borrowed_amount(&env, &asset)
    }

    fn delta_time(env: Env, asset: Address) -> u64 {
        views::delta_time(&env, &asset)
    }

    fn get_sync_data(env: Env, asset: Address) -> PoolSyncData {
        PoolSyncData {
            params: views::load_params(&env, &asset),
            state: views::load_state(&env, &asset),
        }
    }
}

#[cfg(test)]
mod tests;
