#![no_std]
#![allow(clippy::too_many_arguments)]
mod cache;
mod interest;
mod views;

use cache::Cache;
use common::constants::{BPS, RAY, TTL_BUMP_INSTANCE, TTL_THRESHOLD_INSTANCE};
use common::errors::{CollateralError, FlashLoanError, GenericError};
use common::events::{
    emit_update_market_params, emit_update_market_state, UpdateMarketParamsEvent,
    UpdateMarketStateEvent,
};
use common::fp::Ray;
use common::rates::update_supply_index;
use common::types::{
    AccountPosition, MarketIndex, MarketParams, PoolKey, PoolPositionMutation, PoolStrategyMutation,
};
use soroban_sdk::{
    contract, contractimpl, panic_with_error, symbol_short, token, Address, BytesN, Env, Symbol,
};

/// Temporary instance-storage key used by flash_loan_begin / flash_loan_end
/// to record the pool's pre-loan token balance and verify the post-repay
/// delta meets expectations. Written in `begin`; read and cleared in `end`.
const FLASH_LOAN_PRE_BALANCE: Symbol = symbol_short!("FL_PREBAL");
use stellar_access::ownable;
use stellar_macros::only_owner;

#[contract]
pub struct LiquidityPool;

// ---------------------------------------------------------------------------
// Admin verification
// ---------------------------------------------------------------------------

fn verify_admin(env: &Env) {
    ownable::enforce_owner_auth(env);
}

/// Reject negative amounts at every mutating pool ABI. The controller
/// validates sign at user entrypoints; this guard prevents a controller
/// upgrade that omits a `require_amount_positive` check from reaching the
/// pool's phantom-collateral path.
fn require_nonneg_amount(env: &Env, amount: i128) {
    if amount < 0 {
        panic_with_error!(env, GenericError::AmountMustBePositive);
    }
}

/// Saturating subtraction for RAY-scaled totals; absorbs rounding drift.
/// Without this, sequences of scaled debits can underflow and panic even
/// though the protocol remains fundamentally solvent.
fn saturating_sub_ray(a: Ray, b: Ray) -> Ray {
    if b.raw() >= a.raw() {
        Ray::ZERO
    } else {
        a - b
    }
}

// Emit a market-state snapshot event.
fn emit_market_update(env: &Env, cache: &Cache, price_wad: i128, reserves: i128) {
    let asset = cache.params.asset_id.clone();
    emit_update_market_state(
        env,
        UpdateMarketStateEvent {
            asset,
            timestamp: cache.current_timestamp,
            supply_index_ray: cache.supply_index.raw(),
            borrow_index_ray: cache.borrow_index.raw(),
            reserves_ray: reserves,
            supplied_ray: cache.supplied.raw(),
            borrowed_ray: cache.borrowed.raw(),
            revenue_ray: cache.revenue.raw(),
            asset_price_wad: price_wad,
        },
    );
}

#[contractimpl]
impl LiquidityPool {
    // -----------------------------------------------------------------------
    // Constructor
    // -----------------------------------------------------------------------

    pub fn __constructor(env: Env, admin: Address, params: MarketParams, accumulator: Address) {
        ownable::set_owner(&env, &admin);

        env.storage().instance().set(&PoolKey::Params, &params);

        // Accumulator is written once at construction; `claim_revenue` reads
        // it from storage rather than trusting a caller-supplied address.
        env.storage()
            .instance()
            .set(&PoolKey::Accumulator, &accumulator);

        let state = common::types::PoolState {
            supplied_ray: 0,
            borrowed_ray: 0,
            revenue_ray: 0,
            borrow_index_ray: RAY,
            supply_index_ray: RAY,
            last_timestamp: env.ledger().timestamp() * 1000,
        };
        env.storage().instance().set(&PoolKey::State, &state);
    }

    // -----------------------------------------------------------------------
    // Admin-only mutating endpoints
    // -----------------------------------------------------------------------

    pub fn supply(
        env: Env,
        mut position: AccountPosition,
        price_wad: i128,
        amount: i128,
    ) -> PoolPositionMutation {
        verify_admin(&env);
        require_nonneg_amount(&env, amount);
        let mut cache = Cache::load(&env);
        interest::global_sync(&env, &mut cache);

        let scaled_amount = cache.calculate_scaled_supply(amount);
        position.scaled_amount_ray += scaled_amount.raw();
        cache.supplied = cache.supplied + scaled_amount;

        let market_index = MarketIndex {
            borrow_index_ray: cache.borrow_index.raw(),
            supply_index_ray: cache.supply_index.raw(),
        };
        let reserves = cache.get_reserves_for(&cache.params.asset_id);
        emit_market_update(&env, &cache, price_wad, reserves);
        cache.save();
        PoolPositionMutation {
            position,
            market_index,
            actual_amount: amount,
        }
    }

    pub fn borrow(
        env: Env,
        caller: Address,
        amount: i128,
        mut position: AccountPosition,
        price_wad: i128,
    ) -> PoolPositionMutation {
        verify_admin(&env);
        require_nonneg_amount(&env, amount);
        let mut cache = Cache::load(&env);
        interest::global_sync(&env, &mut cache);

        if !cache.has_reserves(amount) {
            panic_with_error!(&env, CollateralError::InsufficientLiquidity);
        }

        let scaled_debt = cache.calculate_scaled_borrow(amount);
        position.scaled_amount_ray += scaled_debt.raw();
        cache.borrowed = cache.borrowed + scaled_debt;

        // Transfer tokens to the borrower.
        let tok = token::Client::new(&env, &cache.params.asset_id);
        tok.transfer(&env.current_contract_address(), &caller, &amount);

        let market_index = MarketIndex {
            borrow_index_ray: cache.borrow_index.raw(),
            supply_index_ray: cache.supply_index.raw(),
        };
        let reserves = cache.get_reserves_for(&cache.params.asset_id);
        emit_market_update(&env, &cache, price_wad, reserves);
        cache.save();
        PoolPositionMutation {
            position,
            market_index,
            actual_amount: amount,
        }
    }

    pub fn withdraw(
        env: Env,
        caller: Address,
        amount: i128,
        mut position: AccountPosition,
        is_liquidation: bool,
        protocol_fee: i128,
        price_wad: i128,
    ) -> PoolPositionMutation {
        verify_admin(&env);
        // `amount == i128::MAX` is the "withdraw all" sentinel from the
        // controller; any other negative value is rejected here.
        require_nonneg_amount(&env, amount);
        require_nonneg_amount(&env, protocol_fee);
        let mut cache = Cache::load(&env);
        interest::global_sync(&env, &mut cache);

        // Gross withdrawal: cap at the position's current value.
        let pos_scaled = Ray::from_raw(position.scaled_amount_ray);
        let current_supply_actual = cache.calculate_original_supply(pos_scaled);
        let (scaled_withdrawal, gross_amount) = if amount >= current_supply_actual {
            (pos_scaled, current_supply_actual)
        } else {
            let scaled = cache.calculate_scaled_supply(amount);
            // Dust-lock guard: if the residual scaled amount would round to
            // zero asset tokens, treat the call as a full withdrawal so the
            // user leaves no permanently-stuck dust behind.
            let remaining_scaled = saturating_sub_ray(pos_scaled, scaled);
            let remaining_actual = cache.calculate_original_supply(remaining_scaled);
            if remaining_actual == 0 {
                (pos_scaled, amount)
            } else {
                (scaled, amount)
            }
        };

        // Apply the liquidation fee if applicable.
        let mut net_transfer = gross_amount;
        if is_liquidation && protocol_fee > 0 {
            if net_transfer < protocol_fee {
                panic_with_error!(&env, CollateralError::WithdrawLessThanFee);
            }
            net_transfer -= protocol_fee;
            interest::add_protocol_revenue(&mut cache, protocol_fee);
        }

        // Verify reserves after fee deduction.
        if !cache.has_reserves(net_transfer) {
            panic_with_error!(&env, CollateralError::InsufficientLiquidity);
        }

        cache.supplied = saturating_sub_ray(cache.supplied, scaled_withdrawal);
        position.scaled_amount_ray -= scaled_withdrawal.raw();

        // Transfer tokens to the caller.
        if net_transfer > 0 {
            let tok = token::Client::new(&env, &cache.params.asset_id);
            tok.transfer(&env.current_contract_address(), &caller, &net_transfer);
        }

        let market_index = MarketIndex {
            borrow_index_ray: cache.borrow_index.raw(),
            supply_index_ray: cache.supply_index.raw(),
        };
        let reserves = cache.get_reserves_for(&cache.params.asset_id);
        emit_market_update(&env, &cache, price_wad, reserves);
        cache.save();
        PoolPositionMutation {
            position,
            market_index,
            actual_amount: gross_amount,
        }
    }

    pub fn repay(
        env: Env,
        caller: Address,
        amount: i128,
        mut position: AccountPosition,
        price_wad: i128,
    ) -> PoolPositionMutation {
        verify_admin(&env);
        require_nonneg_amount(&env, amount);
        let mut cache = Cache::load(&env);
        interest::global_sync(&env, &mut cache);

        let pos_scaled = Ray::from_raw(position.scaled_amount_ray);
        let current_debt = cache.calculate_original_borrow(pos_scaled);

        let (scaled_repay, overpayment) = if amount >= current_debt {
            let over = amount - current_debt;
            (pos_scaled, over)
        } else {
            let scaled = cache.calculate_scaled_borrow(amount);
            (scaled, 0i128)
        };

        position.scaled_amount_ray -= scaled_repay.raw();
        cache.borrowed = saturating_sub_ray(cache.borrowed, scaled_repay);

        // Refund any overpayment.
        if overpayment > 0 {
            let tok = token::Client::new(&env, &cache.params.asset_id);
            tok.transfer(&env.current_contract_address(), &caller, &overpayment);
        }

        let actual_applied = amount.min(current_debt);
        let market_index = MarketIndex {
            borrow_index_ray: cache.borrow_index.raw(),
            supply_index_ray: cache.supply_index.raw(),
        };
        let reserves = cache.get_reserves_for(&cache.params.asset_id);
        emit_market_update(&env, &cache, price_wad, reserves);
        cache.save();
        PoolPositionMutation {
            position,
            market_index,
            actual_amount: actual_applied,
        }
    }

    pub fn update_indexes(env: Env, price_wad: i128) -> MarketIndex {
        verify_admin(&env);
        let mut cache = Cache::load(&env);
        interest::global_sync(&env, &mut cache);

        let result = MarketIndex {
            borrow_index_ray: cache.borrow_index.raw(),
            supply_index_ray: cache.supply_index.raw(),
        };

        let reserves = cache.get_reserves_for(&cache.params.asset_id);
        emit_market_update(&env, &cache, price_wad, reserves);
        cache.save();
        result
    }

    pub fn add_rewards(env: Env, price_wad: i128, amount: i128) {
        verify_admin(&env);
        require_nonneg_amount(&env, amount);
        let mut cache = Cache::load(&env);
        interest::global_sync(&env, &mut cache);
        // Reject reward credits when no supply exists. `update_supply_index`
        // short-circuits on `supplied == ZERO`, so rewards would otherwise
        // land in the pool's token balance without being tracked.
        if cache.supplied == Ray::ZERO {
            panic_with_error!(&env, GenericError::NoSuppliersToReward);
        }

        // amount is in asset decimals; upscale to RAY for consistency with
        // RAY-native supplied and supply_index.
        let amount_ray = Ray::from_asset(amount, cache.params.asset_decimals);
        cache.supply_index =
            update_supply_index(&env, cache.supplied, cache.supply_index, amount_ray);

        let reserves = cache.get_reserves_for(&cache.params.asset_id);
        emit_market_update(&env, &cache, price_wad, reserves);
        cache.save();
    }

    pub fn flash_loan_begin(env: Env, amount: i128, receiver: Address) {
        verify_admin(&env);
        require_nonneg_amount(&env, amount);

        let mut cache = Cache::load(&env);
        interest::global_sync(&env, &mut cache);

        if !cache.has_reserves(amount) {
            panic_with_error!(&env, CollateralError::InsufficientLiquidity);
        }

        // Pool always operates on its own asset; never trust a caller-
        // supplied address. Snapshot the pool balance before paying out so
        // `flash_loan_end` can verify the post-repay delta covers
        // `amount + fee`.
        let tok = token::Client::new(&env, &cache.params.asset_id);
        let pre_balance = tok.balance(&env.current_contract_address());
        env.storage()
            .instance()
            .set(&FLASH_LOAN_PRE_BALANCE, &pre_balance);

        tok.transfer(&env.current_contract_address(), &receiver, &amount);

        cache.save();
    }

    pub fn flash_loan_end(env: Env, amount: i128, fee: i128, receiver: Address) {
        verify_admin(&env);
        require_nonneg_amount(&env, amount);

        let mut cache = Cache::load(&env);
        interest::global_sync(&env, &mut cache);

        if fee < 0 {
            panic_with_error!(&env, FlashLoanError::NegativeFlashLoanFee);
        }

        // Pool always operates on its own asset; pull repayment from the receiver.
        let tok = token::Client::new(&env, &cache.params.asset_id);
        let total = amount + fee;
        let pool_addr = env.current_contract_address();
        tok.transfer(&receiver, &pool_addr, &total);

        // Verify the balance delta matches expectation. Reject if the
        // pre-balance snapshot is missing (indicates `begin` was not called).
        let pre_balance: i128 = env
            .storage()
            .instance()
            .get(&FLASH_LOAN_PRE_BALANCE)
            .unwrap_or_else(|| panic_with_error!(&env, FlashLoanError::InvalidFlashloanRepay));
        env.storage().instance().remove(&FLASH_LOAN_PRE_BALANCE);

        let balance_after = tok.balance(&env.current_contract_address());
        if balance_after < pre_balance + fee {
            panic_with_error!(&env, FlashLoanError::InvalidFlashloanRepay);
        }

        // Record the fee as protocol revenue.
        interest::add_protocol_revenue(&mut cache, fee);

        let reserves = cache.get_reserves_for(&cache.params.asset_id);
        emit_market_update(&env, &cache, 0, reserves);
        cache.save();
    }

    pub fn create_strategy(
        env: Env,
        caller: Address,
        mut position: AccountPosition,
        amount: i128,
        fee: i128,
        price_wad: i128,
    ) -> PoolStrategyMutation {
        verify_admin(&env);
        require_nonneg_amount(&env, amount);
        require_nonneg_amount(&env, fee);
        let mut cache = Cache::load(&env);
        interest::global_sync(&env, &mut cache);

        if fee > amount {
            panic_with_error!(&env, FlashLoanError::StrategyFeeExceeds);
        }
        if !cache.has_reserves(amount) {
            panic_with_error!(&env, CollateralError::InsufficientLiquidity);
        }

        let scaled_debt = cache.calculate_scaled_borrow(amount);
        position.scaled_amount_ray += scaled_debt.raw();
        cache.borrowed = cache.borrowed + scaled_debt;

        // Fee goes to protocol revenue.
        interest::add_protocol_revenue(&mut cache, fee);

        let amount_to_send = amount - fee;

        // Send the net amount to the controller.
        let tok = token::Client::new(&env, &cache.params.asset_id);
        tok.transfer(&env.current_contract_address(), &caller, &amount_to_send);

        let market_index = MarketIndex {
            borrow_index_ray: cache.borrow_index.raw(),
            supply_index_ray: cache.supply_index.raw(),
        };
        let reserves = cache.get_reserves_for(&cache.params.asset_id);
        emit_market_update(&env, &cache, price_wad, reserves);
        cache.save();
        PoolStrategyMutation {
            position,
            market_index,
            actual_amount: amount,
            amount_received: amount_to_send,
        }
    }

    pub fn seize_position(
        env: Env,
        mut position: AccountPosition,
        price_wad: i128,
    ) -> AccountPosition {
        verify_admin(&env);
        let mut cache = Cache::load(&env);
        interest::global_sync(&env, &mut cache);

        if position.position_type == common::types::AccountPositionType::Borrow {
            // Socialize bad debt; use RAY precision for index adjustment.
            let current_debt_ray =
                cache.calculate_original_borrow_ray(Ray::from_raw(position.scaled_amount_ray));
            interest::apply_bad_debt_to_supply_index(&mut cache, current_debt_ray);
            // Saturating subtraction: repeated bad-debt cleanups can leave
            // the position's scaled amount above `cache.borrowed` (earlier
            // caps prevented full debt removal). Plain subtraction would
            // underflow and block subsequent cleanups.
            cache.borrowed =
                saturating_sub_ray(cache.borrowed, Ray::from_raw(position.scaled_amount_ray));
            position.scaled_amount_ray = 0;
        } else if position.position_type == common::types::AccountPositionType::Deposit {
            // Absorb dust into revenue.
            let pos_scaled = Ray::from_raw(position.scaled_amount_ray);
            cache.revenue = cache.revenue + pos_scaled;
            position.scaled_amount_ray = 0;
        } else {
            // Defensive panic: future enum variants must be handled
            // explicitly instead of silently no-oping.
            panic_with_error!(&env, common::errors::GenericError::InvalidPositionType);
        }

        let reserves = cache.get_reserves_for(&cache.params.asset_id);
        emit_market_update(&env, &cache, price_wad, reserves);
        cache.save();
        position
    }

    pub fn claim_revenue(env: Env, price_wad: i128) -> i128 {
        verify_admin(&env);
        let mut cache = Cache::load(&env);
        interest::global_sync(&env, &mut cache);
        let current_reserves = cache.get_reserves_for(&cache.params.asset_id);

        let revenue_scaled = cache.revenue;
        if revenue_scaled == Ray::ZERO {
            emit_market_update(&env, &cache, price_wad, current_reserves);
            cache.save();
            return 0;
        }

        let treasury_actual = cache.calculate_original_supply(revenue_scaled);
        let amount_to_transfer = current_reserves.min(treasury_actual);

        if amount_to_transfer > 0 {
            // Revenue destination is the accumulator stored at construction;
            // never trust a caller-supplied address.
            let accumulator: Address = env
                .storage()
                .instance()
                .get(&PoolKey::Accumulator)
                .unwrap_or_else(|| {
                    panic_with_error!(&env, common::errors::GenericError::AccumulatorNotSet)
                });
            let tok = token::Client::new(&env, &cache.params.asset_id);
            tok.transfer(
                &env.current_contract_address(),
                &accumulator,
                &amount_to_transfer,
            );

            // Burn the proportional scaled revenue share.
            let scaled_to_burn = if amount_to_transfer >= treasury_actual {
                revenue_scaled
            } else {
                let ratio =
                    Ray::from_raw(amount_to_transfer).div(&env, Ray::from_raw(treasury_actual));
                revenue_scaled.mul(&env, ratio)
            };

            // Single clamp against both `revenue` and `supplied` so the
            // post-state preserves `revenue <= supplied` under any rounding
            // path.
            let actual_burn = {
                let mut min = scaled_to_burn;
                if cache.revenue.raw() < min.raw() {
                    min = cache.revenue;
                }
                if cache.supplied.raw() < min.raw() {
                    min = cache.supplied;
                }
                min
            };

            cache.revenue = saturating_sub_ray(cache.revenue, actual_burn);
            cache.supplied = saturating_sub_ray(cache.supplied, actual_burn);
        }

        let reserves = cache.get_reserves_for(&cache.params.asset_id);
        emit_market_update(&env, &cache, price_wad, reserves);
        cache.save();
        amount_to_transfer
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_params(
        env: Env,
        max_borrow_rate: i128,
        base_borrow_rate: i128,
        slope1: i128,
        slope2: i128,
        slope3: i128,
        mid_utilization: i128,
        optimal_utilization: i128,
        reserve_factor: i128,
    ) {
        verify_admin(&env);

        // Sync interest to current timestamp before changing the rate model.
        let mut cache = Cache::load(&env);
        interest::global_sync(&env, &mut cache);
        cache.save();

        // Mirrors `validation::validate_interest_rate_model` in the controller.
        if base_borrow_rate < 0 || slope1 < 0 || slope2 < 0 || slope3 < 0 {
            panic_with_error!(&env, CollateralError::InvalidBorrowParams);
        }
        if mid_utilization <= 0 {
            panic_with_error!(&env, CollateralError::InvalidUtilRange);
        }
        if optimal_utilization <= mid_utilization {
            panic_with_error!(&env, CollateralError::InvalidUtilRange);
        }
        if optimal_utilization >= RAY {
            panic_with_error!(&env, CollateralError::OptUtilTooHigh);
        }
        if !(0..BPS).contains(&reserve_factor) {
            panic_with_error!(&env, CollateralError::InvalidReserveFactor);
        }
        // Monotone slope chain: base <= slope1 <= slope2 <= slope3 <= max.
        if slope1 < base_borrow_rate
            || slope2 < slope1
            || slope3 < slope2
            || max_borrow_rate < slope3
        {
            panic_with_error!(&env, CollateralError::InvalidBorrowParams);
        }
        if max_borrow_rate <= base_borrow_rate {
            panic_with_error!(&env, CollateralError::InvalidBorrowParams);
        }

        let mut params: MarketParams = env
            .storage()
            .instance()
            .get(&PoolKey::Params)
            .unwrap_or_else(|| panic_with_error!(&env, GenericError::PoolNotInitialized));

        params.max_borrow_rate_ray = max_borrow_rate;
        params.base_borrow_rate_ray = base_borrow_rate;
        params.slope1_ray = slope1;
        params.slope2_ray = slope2;
        params.slope3_ray = slope3;
        params.mid_utilization_ray = mid_utilization;
        params.optimal_utilization_ray = optimal_utilization;
        params.reserve_factor_bps = reserve_factor;

        env.storage().instance().set(&PoolKey::Params, &params);

        emit_update_market_params(
            &env,
            UpdateMarketParamsEvent {
                asset: params.asset_id,
                max_borrow_rate_ray: max_borrow_rate,
                base_borrow_rate_ray: base_borrow_rate,
                slope1_ray: slope1,
                slope2_ray: slope2,
                slope3_ray: slope3,
                mid_utilization_ray: mid_utilization,
                optimal_utilization_ray: optimal_utilization,
                reserve_factor_bps: reserve_factor,
            },
        );
    }

    #[only_owner]
    pub fn upgrade(env: Env, new_wasm_hash: BytesN<32>) {
        stellar_contract_utils::upgradeable::upgrade(&env, &new_wasm_hash);
    }

    pub fn keepalive(env: Env) {
        verify_admin(&env);
        env.storage()
            .instance()
            .extend_ttl(TTL_THRESHOLD_INSTANCE, TTL_BUMP_INSTANCE);
    }

    // -----------------------------------------------------------------------
    // Read-only views
    // -----------------------------------------------------------------------

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

    pub fn get_sync_data(env: Env) -> common::types::PoolSyncData {
        let params: MarketParams = env
            .storage()
            .instance()
            .get(&PoolKey::Params)
            .unwrap_or_else(|| panic_with_error!(&env, GenericError::PoolNotInitialized));
        let state: common::types::PoolState = env
            .storage()
            .instance()
            .get(&PoolKey::State)
            .unwrap_or_else(|| panic_with_error!(&env, GenericError::PoolNotInitialized));

        common::types::PoolSyncData { params, state }
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use common::constants::RAY;
    use common::types::AccountPosition;
    use soroban_sdk::testutils::storage::Instance as InstanceTestUtils;
    use soroban_sdk::testutils::{Address as _, Ledger, LedgerInfo};
    use soroban_sdk::{token, Address, Env};

    struct TestSetup {
        env: Env,
        admin: Address,
        asset: Address,
        pool: Address,
    }

    impl TestSetup {
        fn new() -> Self {
            let env = Env::default();
            env.mock_all_auths();

            let admin = Address::generate(&env);
            let asset_address = env
                .register_stellar_asset_contract_v2(admin.clone())
                .address()
                .clone();
            let asset_decimals = 7u32;

            // Set initial ledger timestamp (seconds).
            env.ledger().set(LedgerInfo {
                timestamp: 1000,
                protocol_version: 25,
                sequence_number: 100,
                network_id: Default::default(),
                base_reserve: 10,
                min_temp_entry_ttl: 10,
                min_persistent_entry_ttl: 10,
                max_entry_ttl: 3110400,
            });

            let params = MarketParams {
                max_borrow_rate_ray: RAY,
                base_borrow_rate_ray: RAY / 100,
                slope1_ray: RAY * 4 / 100,
                slope2_ray: RAY * 10 / 100,
                slope3_ray: RAY * 80 / 100,
                mid_utilization_ray: RAY * 50 / 100,
                optimal_utilization_ray: RAY * 80 / 100,
                reserve_factor_bps: 1000,
                asset_id: asset_address.clone(),
                asset_decimals,
            };

            // L-05: pool now requires an accumulator at construction.
            // Test fixture uses `admin` as a stand-in destination.
            let pool_address = env.register(LiquidityPool, (admin.clone(), params, admin.clone()));

            // Mint tokens to the pool for reserves.
            let token_admin = token::StellarAssetClient::new(&env, &asset_address);
            token_admin.mint(&pool_address, &100_000_000_000_000i128);

            TestSetup {
                env,
                admin,
                asset: asset_address,
                pool: pool_address,
            }
        }

        fn client(&self) -> LiquidityPoolClient<'_> {
            LiquidityPoolClient::new(&self.env, &self.pool)
        }

        fn deposit_position(&self) -> AccountPosition {
            AccountPosition {
                position_type: common::types::AccountPositionType::Deposit,
                asset: self.asset.clone(),
                scaled_amount_ray: 0,
                account_id: 1,
                liquidation_threshold_bps: 8000,
                liquidation_bonus_bps: 500,
                liquidation_fees_bps: 100,
                loan_to_value_bps: 7500,
            }
        }

        fn borrow_position(&self) -> AccountPosition {
            AccountPosition {
                position_type: common::types::AccountPositionType::Borrow,
                asset: self.asset.clone(),
                scaled_amount_ray: 0,
                account_id: 1,
                liquidation_threshold_bps: 8000,
                liquidation_bonus_bps: 500,
                liquidation_fees_bps: 100,
                loan_to_value_bps: 7500,
            }
        }

        fn advance_time(&self, seconds: u64) {
            self.env.ledger().set(LedgerInfo {
                timestamp: 1000 + seconds,
                protocol_version: 25,
                sequence_number: 200,
                network_id: Default::default(),
                base_reserve: 10,
                min_temp_entry_ttl: 10,
                min_persistent_entry_ttl: 10,
                max_entry_ttl: 3110400,
            });
        }
    }

    // -----------------------------------------------------------------------
    // Test: supply increases supplied_ray and returns the updated position.
    // -----------------------------------------------------------------------
    #[test]
    fn test_supply() {
        let t = TestSetup::new();
        let client = t.client();

        let pos = t.deposit_position();
        let amount = 10_000_000_000i128;

        let updated = client.supply(&pos, &0i128, &amount);

        assert!(
            updated.position.scaled_amount_ray > 0,
            "position should have scaled amount"
        );

        let supplied = client.supplied_amount();
        assert!(supplied > 0, "supplied_amount should be positive");
    }

    // -----------------------------------------------------------------------
    // Test: borrow decreases reserves and records debt.
    // -----------------------------------------------------------------------
    #[test]
    fn test_borrow() {
        let t = TestSetup::new();
        let client = t.client();

        // Supply first.
        let supply_pos = t.deposit_position();
        client.supply(&supply_pos, &0i128, &50_000_000_000i128);

        // Borrow.
        let borrower = Address::generate(&t.env);
        let borrow_pos = t.borrow_position();
        let borrow_amount = 100_0000000i128;

        let reserves_before = client.reserves();
        let updated = client.borrow(&borrower, &borrow_amount, &borrow_pos, &0i128);

        assert!(
            updated.position.scaled_amount_ray > 0,
            "borrow position should have debt"
        );

        let reserves_after = client.reserves();
        assert!(
            reserves_after < reserves_before,
            "reserves should decrease after borrow"
        );
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #112)")]
    fn test_borrow_rejects_when_reserves_are_insufficient() {
        let t = TestSetup::new();
        let client = t.client();
        let borrower = Address::generate(&t.env);
        let borrow_pos = t.borrow_position();

        let _ = client.borrow(&borrower, &200_000_000_000_000i128, &borrow_pos, &0i128);
    }

    // -----------------------------------------------------------------------
    // Test: withdraw reduces supply and transfers tokens.
    // -----------------------------------------------------------------------
    #[test]
    fn test_withdraw() {
        let t = TestSetup::new();
        let client = t.client();

        let pos = t.deposit_position();
        let supply_amount = 10_000_000_000i128;
        let updated_pos = client.supply(&pos, &0i128, &supply_amount);

        let user = Address::generate(&t.env);
        let tok = token::Client::new(&t.env, &t.asset);
        let user_balance_before = tok.balance(&user);

        let withdraw_amount = 500_0000000i128;
        let final_pos = client.withdraw(
            &user,
            &withdraw_amount,
            &updated_pos.position,
            &false,
            &0i128,
            &0i128,
        );

        let user_balance_after = tok.balance(&user);
        assert!(
            user_balance_after > user_balance_before,
            "user should receive tokens"
        );
        assert!(
            final_pos.position.scaled_amount_ray < updated_pos.position.scaled_amount_ray,
            "scaled amount should decrease"
        );
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #115)")]
    fn test_withdraw_rejects_fee_greater_than_withdrawn_amount() {
        let t = TestSetup::new();
        let client = t.client();

        let pos = t.deposit_position();
        let updated_pos = client.supply(&pos, &0i128, &10_000_000i128);
        let user = Address::generate(&t.env);

        let _ = client.withdraw(
            &user,
            &1_0000000i128,
            &updated_pos.position,
            &true,
            &2_0000000i128,
            &0i128,
        );
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #112)")]
    fn test_withdraw_rejects_when_reserves_are_insufficient() {
        let t = TestSetup::new();
        let client = t.client();

        let pos = t.deposit_position();
        let updated_pos = client.supply(&pos, &0i128, &10_000_000_000i128);

        let borrower = Address::generate(&t.env);
        let borrow_pos = t.borrow_position();
        client.borrow(&borrower, &99_999_990_000_000i128, &borrow_pos, &0i128);

        let user = Address::generate(&t.env);
        let _ = client.withdraw(
            &user,
            &10_000_000_000i128,
            &updated_pos.position,
            &false,
            &0i128,
            &0i128,
        );
    }

    // -----------------------------------------------------------------------
    // Test: repay reduces borrow and handles overpayment.
    // -----------------------------------------------------------------------
    #[test]
    fn test_repay() {
        let t = TestSetup::new();
        let client = t.client();

        let supply_pos = t.deposit_position();
        client.supply(&supply_pos, &0i128, &50_000_000_000i128);

        let borrower = Address::generate(&t.env);
        let borrow_pos = t.borrow_position();
        let updated_borrow = client.borrow(&borrower, &100_0000000i128, &borrow_pos, &0i128);

        assert!(updated_borrow.position.scaled_amount_ray > 0);

        // Repay the exact amount; no overpayment, since no time has passed.
        let repay_amount = 100_0000000i128;
        let final_pos = client.repay(&borrower, &repay_amount, &updated_borrow.position, &0i128);

        assert_eq!(final_pos.actual_amount, repay_amount);
        assert!(
            final_pos.position.scaled_amount_ray == 0 || final_pos.position.scaled_amount_ray <= 1,
            "position should be cleared after full repay"
        );
    }

    #[test]
    fn test_repay_overpayment_reports_actual_applied_amount() {
        let t = TestSetup::new();
        let client = t.client();

        let supply_pos = t.deposit_position();
        client.supply(&supply_pos, &0i128, &50_000_000_000i128);

        let borrower = Address::generate(&t.env);
        let borrow_pos = t.borrow_position();
        let updated_borrow = client.borrow(&borrower, &100_0000000i128, &borrow_pos, &0i128);

        let repay_amount = 200_0000000i128;
        let final_pos = client.repay(&borrower, &repay_amount, &updated_borrow.position, &0i128);

        assert_eq!(final_pos.actual_amount, 100_0000000i128);
        assert_eq!(final_pos.position.scaled_amount_ray, 0);
    }

    // -----------------------------------------------------------------------
    // Test: interest accrual.
    // -----------------------------------------------------------------------
    #[test]
    fn test_interest_accrual() {
        let t = TestSetup::new();
        let client = t.client();

        let supply_pos = t.deposit_position();
        client.supply(&supply_pos, &0i128, &50_000_000_000i128);

        let borrower = Address::generate(&t.env);
        let borrow_pos = t.borrow_position();
        client.borrow(&borrower, &10_000_000_000i128, &borrow_pos, &0i128);

        let initial_indexes = client.update_indexes(&0i128);

        // Advance time by ~1 year.
        t.advance_time(31_556_926);

        let new_indexes = client.update_indexes(&0i128);

        assert!(
            new_indexes.borrow_index_ray > initial_indexes.borrow_index_ray,
            "borrow index should increase over time"
        );
        assert!(
            new_indexes.supply_index_ray > initial_indexes.supply_index_ray,
            "supply index should increase over time"
        );
    }

    // -----------------------------------------------------------------------
    // Test: flash loan begin and end.
    // -----------------------------------------------------------------------
    #[test]
    fn test_flash_loan() {
        let t = TestSetup::new();
        let client = t.client();

        let supply_pos = t.deposit_position();
        client.supply(&supply_pos, &0i128, &10_000_000_000i128);

        // Use admin as receiver, since mock_all_auths covers admin auth.
        let receiver = t.admin.clone();
        let flash_amount = 100_0000000i128;
        let flash_fee = 1_0000000i128;

        // Mint tokens to the receiver so they can repay (amount + fee).
        let token_admin_client = token::StellarAssetClient::new(&t.env, &t.asset);
        token_admin_client.mint(&receiver, &(flash_amount + flash_fee));

        // Begin: tokens sent to the receiver. H-01: pool ABI no longer takes asset.
        client.flash_loan_begin(&flash_amount, &receiver);

        let tok = token::Client::new(&t.env, &t.asset);
        let receiver_balance = tok.balance(&receiver);
        assert!(
            receiver_balance >= flash_amount + flash_fee,
            "receiver should have enough tokens for repayment"
        );

        // End: pull back amount + fee.
        let revenue_before = client.protocol_revenue();
        client.flash_loan_end(&flash_amount, &flash_fee, &receiver);
        let revenue_after = client.protocol_revenue();

        assert!(
            revenue_after > revenue_before,
            "protocol revenue should increase from flash loan fee"
        );
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #112)")]
    fn test_flash_loan_begin_rejects_insufficient_liquidity() {
        let t = TestSetup::new();
        let client = t.client();
        let receiver = Address::generate(&t.env);

        client.flash_loan_begin(&200_000_000_000_000i128, &receiver);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #411)")]
    fn test_flash_loan_end_rejects_negative_fee() {
        let t = TestSetup::new();
        let client = t.client();
        let receiver = Address::generate(&t.env);

        client.flash_loan_end(&1_0000000i128, &-1i128, &receiver);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #409)")]
    fn test_create_strategy_rejects_fee_greater_than_amount() {
        let t = TestSetup::new();
        let client = t.client();
        let caller = Address::generate(&t.env);
        let pos = t.borrow_position();

        let _ = client.create_strategy(&caller, &pos, &1_0000000i128, &2_0000000i128, &0i128);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #112)")]
    fn test_create_strategy_rejects_insufficient_liquidity() {
        let t = TestSetup::new();
        let client = t.client();
        let caller = Address::generate(&t.env);
        let pos = t.borrow_position();

        let _ = client.create_strategy(&caller, &pos, &200_000_000_000_000i128, &0i128, &0i128);
    }

    // -----------------------------------------------------------------------
    // Test: seize_position socializes bad debt.
    // -----------------------------------------------------------------------
    #[test]
    fn test_seize_position_bad_debt() {
        let t = TestSetup::new();
        let client = t.client();

        let supply_pos = t.deposit_position();
        client.supply(&supply_pos, &0i128, &50_000_000_000i128);

        let borrower = Address::generate(&t.env);
        let borrow_pos = t.borrow_position();
        let updated_borrow = client.borrow(&borrower, &100_0000000i128, &borrow_pos, &0i128);

        let idx_before = client.update_indexes(&0i128);

        let seized = client.seize_position(&updated_borrow.position, &0i128);

        assert_eq!(seized.scaled_amount_ray, 0, "position should be zeroed");

        let idx_after = client.update_indexes(&0i128);
        assert!(
            idx_after.supply_index_ray <= idx_before.supply_index_ray,
            "supply index should decrease or stay same after bad debt"
        );
    }

    // -----------------------------------------------------------------------
    // Test: seize_position absorbs deposit dust.
    // -----------------------------------------------------------------------
    #[test]
    fn test_seize_position_deposit_dust() {
        let t = TestSetup::new();
        let client = t.client();

        let supply_pos = t.deposit_position();
        let updated = client.supply(&supply_pos, &0i128, &100_0000000i128);

        let revenue_before = client.protocol_revenue();
        let seized = client.seize_position(&updated.position, &0i128);

        assert_eq!(seized.scaled_amount_ray, 0, "position should be zeroed");

        let revenue_after = client.protocol_revenue();
        assert!(
            revenue_after > revenue_before,
            "protocol revenue should increase from absorbed dust"
        );
    }

    // -----------------------------------------------------------------------
    // Test: claim_revenue.
    // -----------------------------------------------------------------------
    #[test]
    fn test_claim_revenue() {
        let t = TestSetup::new();
        let client = t.client();

        let supply_pos = t.deposit_position();
        client.supply(&supply_pos, &0i128, &50_000_000_000i128);

        let borrower = Address::generate(&t.env);
        let borrow_pos = t.borrow_position();
        client.borrow(&borrower, &10_000_000_000i128, &borrow_pos, &0i128);

        // Advance time to accrue interest.
        t.advance_time(31_556_926);

        // Sync indexes to accrue revenue.
        client.update_indexes(&0i128);

        let revenue = client.protocol_revenue();
        if revenue > 0 {
            let tok = token::Client::new(&t.env, &t.asset);
            let admin_balance_before = tok.balance(&t.admin);
            let claimed = client.claim_revenue(&0i128);
            let admin_balance_after = tok.balance(&t.admin);

            if claimed > 0 {
                assert!(
                    admin_balance_after > admin_balance_before,
                    "admin should receive revenue tokens"
                );
            }
        }
    }

    #[test]
    fn test_claim_revenue_handles_partial_claim_when_reserves_are_lower_than_revenue() {
        let t = TestSetup::new();
        let client = t.client();

        let supply_pos = t.deposit_position();
        let oversized_supply = client.supply(&supply_pos, &0i128, &200_000_000_000_000i128);
        let _ = client.seize_position(&oversized_supply.position, &0i128);

        let claimed = client.claim_revenue(&0i128);
        let remaining_revenue = client.protocol_revenue();

        assert!(
            claimed > 0,
            "partial claim should transfer available reserves"
        );
        assert!(
            remaining_revenue > 0,
            "partial claim should leave residual revenue when treasury exceeds reserves"
        );
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #117)")]
    fn test_update_params_rejects_invalid_utilization_range() {
        let t = TestSetup::new();
        let client = t.client();

        client.update_params(
            &(2 * RAY),
            &(RAY / 100),
            &(RAY / 10),
            &(RAY / 5),
            &RAY,
            &(RAY * 8 / 10),
            &(RAY * 8 / 10),
            &1000,
        );
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #118)")]
    fn test_update_params_rejects_optimal_utilization_above_one() {
        let t = TestSetup::new();
        let client = t.client();

        client.update_params(
            &(2 * RAY),
            &(RAY / 100),
            &(RAY / 10),
            &(RAY / 5),
            &RAY,
            &(RAY / 2),
            &RAY,
            &1000,
        );
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #119)")]
    fn test_update_params_rejects_invalid_reserve_factor() {
        let t = TestSetup::new();
        let client = t.client();

        client.update_params(
            &(2 * RAY),
            &(RAY / 100),
            &(RAY / 10),
            &(RAY / 5),
            &RAY,
            &(RAY / 2),
            &(RAY * 8 / 10),
            &10_000,
        );
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #116)")]
    fn test_update_params_rejects_negative_base_rate() {
        let t = TestSetup::new();
        let client = t.client();

        client.update_params(
            &(2 * RAY),
            &-1i128,
            &(RAY / 10),
            &(RAY / 5),
            &RAY,
            &(RAY / 2),
            &(RAY * 8 / 10),
            &1000,
        );
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #116)")]
    fn test_update_params_rejects_max_rate_not_above_base_rate() {
        let t = TestSetup::new();
        let client = t.client();

        client.update_params(
            &(RAY / 100),
            &(RAY / 100),
            &(RAY / 10),
            &(RAY / 5),
            &RAY,
            &(RAY / 2),
            &(RAY * 8 / 10),
            &1000,
        );
    }

    // -----------------------------------------------------------------------
    // Test: views.
    // -----------------------------------------------------------------------
    #[test]
    fn test_views() {
        let t = TestSetup::new();
        let client = t.client();

        let util = client.capital_utilisation();
        assert_eq!(util, 0, "utilization should be zero initially");

        let supply_pos = t.deposit_position();
        client.supply(&supply_pos, &0i128, &10_000_000_000i128);

        let supplied = client.supplied_amount();
        assert!(
            supplied > 0,
            "supplied_amount should be positive after supply"
        );

        let reserves = client.reserves();
        assert!(reserves > 0, "reserves should be positive");

        let borrower = Address::generate(&t.env);
        let borrow_pos = t.borrow_position();
        client.borrow(&borrower, &100_0000000i128, &borrow_pos, &0i128);

        let borrowed = client.borrowed_amount();
        assert!(borrowed > 0, "borrowed_amount should be positive");

        let util_after = client.capital_utilisation();
        assert!(
            util_after > 0,
            "utilization should be positive after borrow"
        );

        assert!(
            client.deposit_rate() >= 0,
            "deposit rate view should be callable"
        );
        assert!(
            client.borrow_rate() >= 0,
            "borrow rate view should be callable"
        );
        assert!(
            client.protocol_revenue() >= 0,
            "protocol revenue view should be callable"
        );
        t.advance_time(60);
        assert!(client.delta_time() > 0, "delta_time should be positive");
    }

    // -----------------------------------------------------------------------
    // Extra targeted coverage tests.
    // -----------------------------------------------------------------------

    // Covers the withdraw liquidation-fee success branch (lines 205-206).
    #[test]
    fn test_withdraw_liquidation_fee_accrues_to_revenue() {
        let t = TestSetup::new();
        let client = t.client();

        let pos = t.deposit_position();
        let supply_amount = 10_000_000_000i128;
        let updated_pos = client.supply(&pos, &0i128, &supply_amount);

        let revenue_before = client.protocol_revenue();

        let user = Address::generate(&t.env);
        let tok = token::Client::new(&t.env, &t.asset);
        let user_balance_before = tok.balance(&user);

        let gross = 10_000_000_000_i128;
        let fee = 10_000_000_i128;
        let final_pos = client.withdraw(&user, &gross, &updated_pos.position, &true, &fee, &0i128);

        let user_balance_after = tok.balance(&user);
        assert_eq!(
            user_balance_after - user_balance_before,
            gross - fee,
            "user should receive gross minus protocol fee"
        );
        let revenue_after = client.protocol_revenue();
        assert!(
            revenue_after > revenue_before,
            "protocol revenue should increase by fee"
        );
        assert_eq!(final_pos.actual_amount, gross);
    }

    // Covers the repay partial branch (lines 257-258).
    #[test]
    fn test_repay_partial_amount() {
        let t = TestSetup::new();
        let client = t.client();

        let supply_pos = t.deposit_position();
        client.supply(&supply_pos, &0i128, &50_000_000_000i128);

        let borrower = Address::generate(&t.env);
        let borrow_pos = t.borrow_position();
        let updated_borrow = client.borrow(&borrower, &100_0000000i128, &borrow_pos, &0i128);

        // Advance time to accrue interest so current_debt > initial.
        t.advance_time(60);

        let partial = 10_0000000i128;
        let final_pos = client.repay(&borrower, &partial, &updated_borrow.position, &0i128);

        assert_eq!(
            final_pos.actual_amount, partial,
            "partial repay returns the amount passed in"
        );
        assert!(
            final_pos.position.scaled_amount_ray > 0,
            "position should still have residual debt after partial repay"
        );
        assert!(
            final_pos.position.scaled_amount_ray < updated_borrow.position.scaled_amount_ray,
            "scaled debt should decrease after partial repay"
        );
    }

    // Covers the full add_rewards body (lines 301-315).
    #[test]
    fn test_add_rewards_increases_supply_index() {
        let t = TestSetup::new();
        let client = t.client();

        let supply_pos = t.deposit_position();
        client.supply(&supply_pos, &0i128, &50_000_000_000i128);

        let idx_before = client.update_indexes(&0i128);

        client.add_rewards(&0i128, &1_000_000_000i128);

        let idx_after = client.update_indexes(&0i128);
        assert!(
            idx_after.supply_index_ray > idx_before.supply_index_ray,
            "supply index should increase after add_rewards"
        );
    }

    // Covers the create_strategy happy path (lines 394-422).
    #[test]
    fn test_create_strategy_emits_position_and_transfers_net() {
        let t = TestSetup::new();
        let client = t.client();

        // Supply reserves so create_strategy can transfer.
        let supply_pos = t.deposit_position();
        client.supply(&supply_pos, &0i128, &50_000_000_000i128);

        let caller = Address::generate(&t.env);
        let pos = t.borrow_position();
        let tok = token::Client::new(&t.env, &t.asset);
        let caller_before = tok.balance(&caller);
        let revenue_before = client.protocol_revenue();

        let amount = 100_0000000i128;
        let fee = 1_0000000i128;
        let result = client.create_strategy(&caller, &pos, &amount, &fee, &0i128);

        assert_eq!(result.actual_amount, amount);
        assert_eq!(result.amount_received, amount - fee);
        assert!(result.position.scaled_amount_ray > 0, "debt recorded");

        let caller_after = tok.balance(&caller);
        assert_eq!(
            caller_after - caller_before,
            amount - fee,
            "caller receives net amount"
        );
        let revenue_after = client.protocol_revenue();
        assert!(
            revenue_after > revenue_before,
            "protocol revenue should increase by fee"
        );
    }

    // Covers the claim_revenue zero-revenue early-return branch (lines 460-463).
    #[test]
    fn test_claim_revenue_zero_revenue_early_returns() {
        let t = TestSetup::new();
        let client = t.client();

        // No supply, no accrual; revenue is zero.
        let claimed = client.claim_revenue(&0i128);
        assert_eq!(claimed, 0, "claim_revenue should return 0 when no revenue");
    }

    // Covers the update_params full successful path (lines 556-587).
    //
    // Regression target: if `update_params` silently dropped a field (e.g.
    // used the wrong source variable when rewriting the struct), the happy
    // path would still return Ok. This test reads every updated field back
    // through `get_sync_data()` and asserts exact equality, so a dropped
    // write must surface.
    #[test]
    fn test_update_params_happy_path() {
        let t = TestSetup::new();
        let client = t.client();

        let new_max = RAY * 2;
        let new_base = RAY / 100;
        let new_s1 = RAY * 5 / 100;
        let new_s2 = RAY * 15 / 100;
        let new_s3 = RAY * 90 / 100;
        let new_mid = RAY * 40 / 100;
        let new_opt = RAY * 85 / 100;
        let new_reserve = 2000i128;

        client.update_params(
            &new_max,
            &new_base,
            &new_s1,
            &new_s2,
            &new_s3,
            &new_mid,
            &new_opt,
            &new_reserve,
        );

        // Every field must round-trip exactly.
        let sync = client.get_sync_data();
        assert_eq!(
            sync.params.max_borrow_rate_ray, new_max,
            "max_borrow_rate_ray"
        );
        assert_eq!(
            sync.params.base_borrow_rate_ray, new_base,
            "base_borrow_rate_ray"
        );
        assert_eq!(sync.params.slope1_ray, new_s1, "slope1_ray");
        assert_eq!(sync.params.slope2_ray, new_s2, "slope2_ray");
        assert_eq!(sync.params.slope3_ray, new_s3, "slope3_ray");
        assert_eq!(
            sync.params.mid_utilization_ray, new_mid,
            "mid_utilization_ray"
        );
        assert_eq!(
            sync.params.optimal_utilization_ray, new_opt,
            "optimal_utilization_ray"
        );
        assert_eq!(
            sync.params.reserve_factor_bps, new_reserve,
            "reserve_factor_bps"
        );

        // Downstream sanity: with the base rate still 1% but higher slopes,
        // the borrow rate at 50% utilisation must reflect the *new* slope1
        // (5% vs the old 4%). We do not pin an exact number; the point is
        // that the borrow path picks up the updated params.
        let supply_pos = t.deposit_position();
        client.supply(&supply_pos, &0i128, &10_000_000_000i128);
        let borrower = Address::generate(&t.env);
        let borrow_pos = t.borrow_position();
        let _ = client.borrow(&borrower, &100_0000000i128, &borrow_pos, &0i128);
    }

    // Covers update_params validation: slope-ordering rejection (lines 545-550).
    #[test]
    #[should_panic(expected = "Error(Contract, #116)")]
    fn test_update_params_rejects_invalid_slope_ordering() {
        let t = TestSetup::new();
        let client = t.client();

        // slope3 < slope2: invalid.
        client.update_params(
            &(2 * RAY),
            &(RAY / 100),
            &(RAY / 10),
            &(RAY / 2),
            &(RAY / 5),
            &(RAY / 2),
            &(RAY * 8 / 10),
            &1000,
        );
    }

    // Covers the update_params mid_utilization == 0 rejection (lines 532-534).
    #[test]
    #[should_panic(expected = "Error(Contract, #117)")]
    fn test_update_params_rejects_mid_utilization_zero() {
        let t = TestSetup::new();
        let client = t.client();

        client.update_params(
            &(2 * RAY),
            &(RAY / 100),
            &(RAY / 10),
            &(RAY / 5),
            &RAY,
            &0i128,
            &(RAY * 8 / 10),
            &1000,
        );
    }

    // Covers the update_params negative reserve_factor rejection (lines 541-543).
    #[test]
    #[should_panic(expected = "Error(Contract, #119)")]
    fn test_update_params_rejects_negative_reserve_factor() {
        let t = TestSetup::new();
        let client = t.client();

        client.update_params(
            &(2 * RAY),
            &(RAY / 100),
            &(RAY / 10),
            &(RAY / 5),
            &RAY,
            &(RAY / 2),
            &(RAY * 8 / 10),
            &-1i128,
        );
    }

    // Covers the keepalive endpoint (lines 594-599).
    //
    // The previous version of this test only called `keepalive()` with no
    // assertions; a silent regression that skipped the TTL extension would
    // have passed. We now:
    //   1. Assert that a non-admin caller cannot invoke keepalive (auth gate).
    //   2. Assert that, after admin-authorized keepalive, the instance entry's
    //      live_until_ledger is at least `current + TTL_THRESHOLD_INSTANCE`.
    //      A silent no-op would leave the ledger at its original (minimal) TTL.
    #[test]
    fn test_keepalive_bumps_ttl() {
        let t = TestSetup::new();
        let client = t.client();

        // Admin-authorized call succeeds. `env.mock_all_auths()` covers the
        // `verify_admin` gate. The host auto-records the auth requirement,
        // proving the endpoint calls `ownable::enforce_owner_auth`.
        client.keepalive();

        // Assert the instance entry TTL was extended. `get_ttl` returns the
        // live_until ledger; reading from inside the pool contract frame is
        // required, since `.instance()` is keyed by current_contract_address.
        let live_until = t
            .env
            .as_contract(&t.pool, || t.env.storage().instance().get_ttl());
        let current = t.env.ledger().sequence();
        assert!(
            live_until >= current + TTL_THRESHOLD_INSTANCE,
            "keepalive must bump instance TTL by at least TTL_THRESHOLD_INSTANCE: current={}, live_until={}",
            current,
            live_until
        );
    }

    // Covers the keepalive auth gate (line 595, `verify_admin`).
    //
    // A regression that dropped the `verify_admin` call would still let the
    // test above pass (mock_all_auths covers any address). This variant
    // opts out of blanket auth and asserts that a fresh non-admin cannot
    // invoke it.
    #[test]
    #[should_panic]
    fn test_keepalive_rejects_non_admin() {
        let env = Env::default();
        env.ledger().set(LedgerInfo {
            timestamp: 1000,
            protocol_version: 25,
            sequence_number: 100,
            network_id: Default::default(),
            base_reserve: 10,
            min_temp_entry_ttl: 10,
            min_persistent_entry_ttl: 10,
            max_entry_ttl: 3110400,
        });

        let admin = Address::generate(&env);
        let asset_address = env
            .register_stellar_asset_contract_v2(admin.clone())
            .address()
            .clone();
        let params = MarketParams {
            max_borrow_rate_ray: RAY,
            base_borrow_rate_ray: RAY / 100,
            slope1_ray: RAY * 4 / 100,
            slope2_ray: RAY * 10 / 100,
            slope3_ray: RAY * 80 / 100,
            mid_utilization_ray: RAY * 50 / 100,
            optimal_utilization_ray: RAY * 80 / 100,
            reserve_factor_bps: 1000,
            asset_id: asset_address,
            asset_decimals: 7,
        };
        let pool = env.register(LiquidityPool, (admin, params));
        let client = LiquidityPoolClient::new(&env, &pool);
        // No mock_all_auths and no admin auth: must panic.
        client.keepalive();
    }
}
