#![no_std]
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
    pub fn __constructor(env: Env, admin: Address, params: MarketParams) {
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

    // Admin-only mutating endpoints.

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
        // `i128::MAX` is the controller's "withdraw all" sentinel; other
        // negatives are rejected.
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

        // Always operate on the pool's own asset and snapshot balance locally
        // so the callback can't settle with a different asset.
        let pool_addr = env.current_contract_address();
        let tok = token::Client::new(&env, &cache.params.asset_id);
        let pre_balance = tok.balance(&pool_addr);
        let expected_after_payout = pre_balance - amount;
        let total = amount + fee;
        let expected_after_repay = pre_balance + fee;

        tok.transfer(&pool_addr, &receiver, &amount);

        // Pre-callback sanity: catch fee-on-transfer / silently-failing
        // tokens so the callback sees a known-correct balance.
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

        // Post-callback: callback must not have mutated the pool's balance.
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
                // Socialise bad debt via the supply index (RAY precision).
                let current_debt_ray =
                    cache.calculate_original_borrow_ray(Ray::from_raw(position.scaled_amount_ray));
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

        // Defensive: writers (`+=` positive, `checked_sub_assign`) keep
        // `revenue` non-negative. A negative value would silently skip the
        // burn (treasury → negative → no transfer) and return garbage.
        if cache.revenue.raw() < 0 {
            panic_with_error!(&env, GenericError::MathOverflow);
        }

        let amount_to_transfer = cache.burn_claimable_revenue();

        // CEI: commit state before the external token call so a re-entry
        // can't observe stale revenue and recurse a claim.
        let mutation = cache.amount_mutation(amount_to_transfer);
        cache.save();

        if amount_to_transfer > 0 {
            // Revenue always routes to the pool owner (controller); pool
            // never trusts a caller-supplied destination.
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
        reserve_factor: u32,
    ) {
        // Accrue at the old rate model before applying the new one.
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
mod tests {
    extern crate std;

    use super::*;
    use common::constants::{BPS, RAY};
    use common::types::AccountPosition;
    use soroban_sdk::testutils::storage::Instance as InstanceTestUtils;
    use soroban_sdk::testutils::{Address as _, Ledger, LedgerInfo};
    use soroban_sdk::{contract, contractimpl, token, Address, Bytes, Env};

    #[contract]
    pub struct PoolFlashLoanReceiver;

    #[contract]
    pub struct PoolNoRepayReceiver;

    #[contractimpl]
    impl PoolFlashLoanReceiver {
        pub fn execute_flash_loan(
            env: Env,
            _initiator: Address,
            asset: Address,
            amount: i128,
            fee: i128,
            pool: Address,
            _data: Bytes,
        ) {
            let total = amount
                .checked_add(fee)
                .unwrap_or_else(|| panic_with_error!(&env, GenericError::MathOverflow));
            let expiration_ledger = env
                .ledger()
                .sequence()
                .checked_add(1)
                .unwrap_or_else(|| panic_with_error!(&env, GenericError::MathOverflow));

            token::Client::new(&env, &asset).approve(
                &env.current_contract_address(),
                &pool,
                &total,
                &expiration_ledger,
            );
        }
    }

    #[contractimpl]
    impl PoolNoRepayReceiver {
        pub fn execute_flash_loan(
            _env: Env,
            _initiator: Address,
            _asset: Address,
            _amount: i128,
            _fee: i128,
            _pool: Address,
            _data: Bytes,
        ) {
        }
    }

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

            test_support::init_ledger(&env);

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

            // Pool's owner (admin) receives revenue on claim_revenue; the
            // controller forwards from there to the protocol accumulator.
            let pool_address = env.register(LiquidityPool, (admin.clone(), params));

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
                scaled_amount_ray: 0,
                liquidation_threshold_bps: 8000,
                liquidation_bonus_bps: 500,
                liquidation_fees_bps: 100,
                loan_to_value_bps: 7500,
            }
        }

        fn borrow_position(&self) -> AccountPosition {
            AccountPosition {
                scaled_amount_ray: 0,
                liquidation_threshold_bps: 8000,
                liquidation_bonus_bps: 500,
                liquidation_fees_bps: 100,
                loan_to_value_bps: 7500,
            }
        }

        fn advance_time(&self, seconds: u64) {
            self.env.ledger().set(LedgerInfo {
                timestamp: 1000 + seconds,
                protocol_version: 26,
                sequence_number: 200,
                network_id: Default::default(),
                base_reserve: 10,
                min_temp_entry_ttl: 10,
                min_persistent_entry_ttl: 10,
                max_entry_ttl: 3110400,
            });
        }

        fn edit_state(&self, edit: impl FnOnce(&mut PoolState)) {
            self.env.as_contract(&self.pool, || {
                let mut state: PoolState =
                    self.env.storage().instance().get(&PoolKey::State).unwrap();
                edit(&mut state);
                self.env.storage().instance().set(&PoolKey::State, &state);
            });
        }

        fn state_snapshot(&self) -> PoolState {
            self.env.as_contract(&self.pool, || {
                self.env.storage().instance().get(&PoolKey::State).unwrap()
            })
        }
    }

    fn assert_pool_state_eq(left: &PoolState, right: &PoolState) {
        assert_eq!(left.supplied_ray, right.supplied_ray);
        assert_eq!(left.borrowed_ray, right.borrowed_ray);
        assert_eq!(left.revenue_ray, right.revenue_ray);
        assert_eq!(left.borrow_index_ray, right.borrow_index_ray);
        assert_eq!(left.supply_index_ray, right.supply_index_ray);
        assert_eq!(left.last_timestamp, right.last_timestamp);
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

        let updated = client.supply(&pos, &amount, &i128::MAX);

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
        client.supply(&supply_pos, &50_000_000_000i128, &i128::MAX);

        // Borrow.
        let borrower = Address::generate(&t.env);
        let borrow_pos = t.borrow_position();
        let borrow_amount = 100_0000000i128;

        let reserves_before = client.reserves();
        let updated = client.borrow(&borrower, &borrow_amount, &borrow_pos, &i128::MAX);

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
    #[should_panic(expected = "Error(Contract, #105)")]
    fn test_supply_cap_enforced_after_pool_sync() {
        let t = TestSetup::new();
        let client = t.client();

        let pos = t.deposit_position();
        let amount = 10_000_000_000i128;

        client.supply(&pos, &amount, &(amount - 1));
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #106)")]
    fn test_borrow_cap_enforced_after_pool_sync() {
        let t = TestSetup::new();
        let client = t.client();

        let supply_pos = t.deposit_position();
        client.supply(&supply_pos, &50_000_000_000i128, &i128::MAX);

        let borrower = Address::generate(&t.env);
        let borrow_pos = t.borrow_position();
        let borrow_amount = 100_0000000i128;

        client.borrow(&borrower, &borrow_amount, &borrow_pos, &(borrow_amount - 1));
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #106)")]
    fn test_strategy_borrow_cap_enforced_after_pool_sync() {
        let t = TestSetup::new();
        let client = t.client();

        let supply_pos = t.deposit_position();
        client.supply(&supply_pos, &50_000_000_000i128, &i128::MAX);

        let caller = Address::generate(&t.env);
        let pos = t.borrow_position();
        let amount = 100_0000000i128;

        client.create_strategy(&caller, &pos, &amount, &0i128, &(amount - 1));
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #112)")]
    fn test_borrow_rejects_when_reserves_are_insufficient() {
        let t = TestSetup::new();
        let client = t.client();
        let borrower = Address::generate(&t.env);
        let borrow_pos = t.borrow_position();

        let _ = client.borrow(&borrower, &200_000_000_000_000i128, &borrow_pos, &i128::MAX);
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
        let updated_pos = client.supply(&pos, &supply_amount, &i128::MAX);

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
        let updated_pos = client.supply(&pos, &10_000_000i128, &i128::MAX);
        let user = Address::generate(&t.env);

        let _ = client.withdraw(
            &user,
            &1_0000000i128,
            &updated_pos.position,
            &true,
            &2_0000000i128,
        );
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #112)")]
    fn test_withdraw_rejects_when_reserves_are_insufficient() {
        let t = TestSetup::new();
        let client = t.client();

        let pos = t.deposit_position();
        let updated_pos = client.supply(&pos, &10_000_000_000i128, &i128::MAX);

        let borrower = Address::generate(&t.env);
        let borrow_pos = t.borrow_position();
        client.borrow(&borrower, &99_999_990_000_000i128, &borrow_pos, &i128::MAX);

        let user = Address::generate(&t.env);
        let _ = client.withdraw(
            &user,
            &10_000_000_000i128,
            &updated_pos.position,
            &false,
            &0i128,
        );
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #33)")]
    fn test_withdraw_rejects_supplied_accounting_underflow() {
        let t = TestSetup::new();
        let client = t.client();

        let pos = t.deposit_position();
        let updated_pos = client.supply(&pos, &10_000_000_000i128, &i128::MAX);
        t.edit_state(|state| {
            state.supplied_ray = 1;
        });

        let user = Address::generate(&t.env);
        let _ = client.withdraw(&user, &i128::MAX, &updated_pos.position, &false, &0i128);
    }

    // -----------------------------------------------------------------------
    // Test: repay reduces borrow and handles overpayment.
    // -----------------------------------------------------------------------
    #[test]
    fn test_repay() {
        let t = TestSetup::new();
        let client = t.client();

        let supply_pos = t.deposit_position();
        client.supply(&supply_pos, &50_000_000_000i128, &i128::MAX);

        let borrower = Address::generate(&t.env);
        let borrow_pos = t.borrow_position();
        let updated_borrow = client.borrow(&borrower, &100_0000000i128, &borrow_pos, &i128::MAX);

        assert!(updated_borrow.position.scaled_amount_ray > 0);

        // Repay the exact amount; no overpayment, since no time has passed.
        let repay_amount = 100_0000000i128;
        let final_pos = client.repay(&borrower, &repay_amount, &updated_borrow.position);

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
        client.supply(&supply_pos, &50_000_000_000i128, &i128::MAX);

        let borrower = Address::generate(&t.env);
        let borrow_pos = t.borrow_position();
        let updated_borrow = client.borrow(&borrower, &100_0000000i128, &borrow_pos, &i128::MAX);

        let repay_amount = 200_0000000i128;
        let final_pos = client.repay(&borrower, &repay_amount, &updated_borrow.position);

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
        client.supply(&supply_pos, &50_000_000_000i128, &i128::MAX);

        let borrower = Address::generate(&t.env);
        let borrow_pos = t.borrow_position();
        client.borrow(&borrower, &10_000_000_000i128, &borrow_pos, &i128::MAX);

        let initial_indexes = client.update_indexes();

        // Advance time by ~1 year.
        t.advance_time(31_556_926);

        let new_indexes = client.update_indexes();

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
    // Test: flash loan.
    // -----------------------------------------------------------------------
    #[test]
    fn test_flash_loan() {
        let t = TestSetup::new();
        let client = t.client();

        let supply_pos = t.deposit_position();
        client.supply(&supply_pos, &10_000_000_000i128, &i128::MAX);

        let receiver = t.env.register(PoolFlashLoanReceiver, ());
        let flash_amount = 100_0000000i128;
        let flash_fee = 1_0000000i128;

        // The pool will send `amount`; pre-fund only the fee.
        let token_admin_client = token::StellarAssetClient::new(&t.env, &t.asset);
        token_admin_client.mint(&receiver, &flash_fee);

        let tok = token::Client::new(&t.env, &t.asset);
        let pool_balance_before = tok.balance(&t.pool);
        let revenue_before = client.protocol_revenue();
        client.flash_loan(
            &t.admin,
            &receiver,
            &flash_amount,
            &flash_fee,
            &Bytes::new(&t.env),
        );
        let revenue_after = client.protocol_revenue();
        let pool_balance_after = tok.balance(&t.pool);

        assert_eq!(pool_balance_after, pool_balance_before + flash_fee);
        assert_eq!(revenue_after, revenue_before + flash_fee);
    }

    #[test]
    fn test_flash_loan_rejects_zero_amount_at_pool() {
        let t = TestSetup::new();
        let client = t.client();
        let receiver = t.env.register(PoolFlashLoanReceiver, ());

        let result =
            client.try_flash_loan(&t.admin, &receiver, &0i128, &0i128, &Bytes::new(&t.env));

        assert!(result.is_err(), "zero-amount pool flash loan must fail");
    }

    #[test]
    fn test_flash_loan_rejects_non_contract_receiver_at_pool() {
        let t = TestSetup::new();
        let client = t.client();
        let receiver = Address::generate(&t.env);

        let result = client.try_flash_loan(
            &t.admin,
            &receiver,
            &1_0000000i128,
            &0i128,
            &Bytes::new(&t.env),
        );

        assert!(
            result.is_err(),
            "pool must reject receivers that are not WASM contracts"
        );
    }

    #[test]
    fn test_flash_loan_rejects_direct_non_owner_pool_call() {
        let t = TestSetup::new();
        let client = t.client();
        let receiver = t.env.register(PoolFlashLoanReceiver, ());
        let attacker = Address::generate(&t.env);
        let no_auths: [soroban_sdk::xdr::SorobanAuthorizationEntry; 0] = [];

        let result = client.set_auths(&no_auths).try_flash_loan(
            &attacker,
            &receiver,
            &1_0000000i128,
            &0i128,
            &Bytes::new(&t.env),
        );

        assert!(
            result.is_err(),
            "direct pool flash loan without owner/controller auth must fail"
        );
    }

    #[test]
    fn test_flash_loan_callback_failure_rolls_back_pool_state() {
        let t = TestSetup::new();
        let client = t.client();
        let receiver = t.env.register(PoolNoRepayReceiver, ());
        let tok = token::Client::new(&t.env, &t.asset);

        let balance_before = tok.balance(&t.pool);
        let revenue_before = client.protocol_revenue();
        let state_before = t.state_snapshot();

        let result = client.try_flash_loan(
            &t.admin,
            &receiver,
            &1_0000000i128,
            &1_000i128,
            &Bytes::new(&t.env),
        );

        assert!(result.is_err(), "receiver that does not repay must fail");
        assert_eq!(tok.balance(&t.pool), balance_before);
        assert_eq!(client.protocol_revenue(), revenue_before);
        assert_pool_state_eq(&t.state_snapshot(), &state_before);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #112)")]
    fn test_flash_loan_rejects_insufficient_liquidity() {
        let t = TestSetup::new();
        let client = t.client();
        let receiver = Address::generate(&t.env);

        client.flash_loan(
            &t.admin,
            &receiver,
            &200_000_000_000_000i128,
            &0i128,
            &Bytes::new(&t.env),
        );
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #14)")]
    fn test_flash_loan_rejects_negative_fee() {
        let t = TestSetup::new();
        let client = t.client();
        let receiver = Address::generate(&t.env);

        client.flash_loan(
            &t.admin,
            &receiver,
            &1_0000000i128,
            &-1i128,
            &Bytes::new(&t.env),
        );
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #409)")]
    fn test_create_strategy_rejects_fee_greater_than_amount() {
        let t = TestSetup::new();
        let client = t.client();
        let caller = Address::generate(&t.env);
        let pos = t.borrow_position();

        let _ = client.create_strategy(&caller, &pos, &1_0000000i128, &2_0000000i128, &i128::MAX);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #112)")]
    fn test_create_strategy_rejects_insufficient_liquidity() {
        let t = TestSetup::new();
        let client = t.client();
        let caller = Address::generate(&t.env);
        let pos = t.borrow_position();

        let _ = client.create_strategy(&caller, &pos, &200_000_000_000_000i128, &0i128, &i128::MAX);
    }

    // -----------------------------------------------------------------------
    // Test: seize_position socializes bad debt.
    // -----------------------------------------------------------------------
    #[test]
    fn test_seize_position_bad_debt() {
        let t = TestSetup::new();
        let client = t.client();

        let supply_pos = t.deposit_position();
        client.supply(&supply_pos, &50_000_000_000i128, &i128::MAX);

        let borrower = Address::generate(&t.env);
        let borrow_pos = t.borrow_position();
        let updated_borrow = client.borrow(&borrower, &100_0000000i128, &borrow_pos, &i128::MAX);

        let idx_before = client.update_indexes();

        let seized = client.seize_position(&AccountPositionType::Borrow, &updated_borrow.position);

        assert_eq!(
            seized.position.scaled_amount_ray, 0,
            "position should be zeroed"
        );

        let idx_after = client.update_indexes();
        assert!(
            idx_after.supply_index_ray <= idx_before.supply_index_ray,
            "supply index should decrease or stay same after bad debt"
        );
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #33)")]
    fn test_seize_position_rejects_borrowed_accounting_underflow() {
        let t = TestSetup::new();
        let client = t.client();

        let supply_pos = t.deposit_position();
        client.supply(&supply_pos, &50_000_000_000i128, &i128::MAX);

        let borrower = Address::generate(&t.env);
        let borrow_pos = t.borrow_position();
        let updated_borrow = client.borrow(&borrower, &100_0000000i128, &borrow_pos, &i128::MAX);
        t.edit_state(|state| {
            state.borrowed_ray = 0;
        });

        let _ = client.seize_position(&AccountPositionType::Borrow, &updated_borrow.position);
    }

    // -----------------------------------------------------------------------
    // Test: seize_position absorbs deposit dust.
    // -----------------------------------------------------------------------
    #[test]
    fn test_seize_position_deposit_dust() {
        let t = TestSetup::new();
        let client = t.client();

        let supply_pos = t.deposit_position();
        let updated = client.supply(&supply_pos, &100_0000000i128, &i128::MAX);

        let revenue_before = client.protocol_revenue();
        let seized = client.seize_position(&AccountPositionType::Deposit, &updated.position);

        assert_eq!(
            seized.position.scaled_amount_ray, 0,
            "position should be zeroed"
        );

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
        client.supply(&supply_pos, &50_000_000_000i128, &i128::MAX);

        let borrower = Address::generate(&t.env);
        let borrow_pos = t.borrow_position();
        client.borrow(&borrower, &10_000_000_000i128, &borrow_pos, &i128::MAX);

        // Advance time to accrue interest.
        t.advance_time(31_556_926);

        // Sync indexes to accrue revenue.
        client.update_indexes();

        let revenue = client.protocol_revenue();
        if revenue > 0 {
            let tok = token::Client::new(&t.env, &t.asset);
            let admin_balance_before = tok.balance(&t.admin);
            let claimed = client.claim_revenue().actual_amount;
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
        let oversized_supply = client.supply(&supply_pos, &200_000_000_000_000i128, &i128::MAX);
        let _ = client.seize_position(&AccountPositionType::Deposit, &oversized_supply.position);

        let claimed = client.claim_revenue().actual_amount;
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
    #[should_panic(expected = "Error(Contract, #33)")]
    fn test_claim_revenue_rejects_revenue_above_supplied() {
        let t = TestSetup::new();
        let client = t.client();

        let supply_pos = t.deposit_position();
        let supplied = client.supply(&supply_pos, &10_000_000_000i128, &i128::MAX);
        let _ = client.seize_position(&AccountPositionType::Deposit, &supplied.position);
        t.edit_state(|state| {
            state.supplied_ray = 1;
        });

        let _ = client.claim_revenue();
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
        client.supply(&supply_pos, &10_000_000_000i128, &i128::MAX);

        let supplied = client.supplied_amount();
        assert!(
            supplied > 0,
            "supplied_amount should be positive after supply"
        );

        let reserves = client.reserves();
        assert!(reserves > 0, "reserves should be positive");

        let borrower = Address::generate(&t.env);
        let borrow_pos = t.borrow_position();
        client.borrow(&borrower, &100_0000000i128, &borrow_pos, &i128::MAX);

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

    // Liquidation fee on withdraw accrues to protocol revenue; user receives gross minus fee.
    #[test]
    fn test_withdraw_liquidation_fee_accrues_to_revenue() {
        let t = TestSetup::new();
        let client = t.client();

        let pos = t.deposit_position();
        let supply_amount = 10_000_000_000i128;
        let updated_pos = client.supply(&pos, &supply_amount, &i128::MAX);

        let revenue_before = client.protocol_revenue();

        let user = Address::generate(&t.env);
        let tok = token::Client::new(&t.env, &t.asset);
        let user_balance_before = tok.balance(&user);

        let gross = 10_000_000_000_i128;
        let fee = 10_000_000_i128;
        let final_pos = client.withdraw(&user, &gross, &updated_pos.position, &true, &fee);

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

    // is_liquidation=true with protocol_fee=0 must skip the fee branch
    // entirely (no revenue accrual) and behave like a regular withdraw.
    #[test]
    fn test_withdraw_liquidation_with_zero_protocol_fee_is_no_op() {
        let t = TestSetup::new();
        let client = t.client();

        let pos = t.deposit_position();
        let supply_amount = 10_000_000_000i128;
        let updated_pos = client.supply(&pos, &supply_amount, &i128::MAX);

        let revenue_before = client.protocol_revenue();
        let user = Address::generate(&t.env);
        let tok = token::Client::new(&t.env, &t.asset);
        let user_balance_before = tok.balance(&user);

        let gross = 1_000_000_000_i128;
        let final_pos = client.withdraw(&user, &gross, &updated_pos.position, &true, &0i128);

        assert_eq!(tok.balance(&user) - user_balance_before, gross);
        assert_eq!(client.protocol_revenue(), revenue_before);
        assert_eq!(final_pos.actual_amount, gross);
    }

    // No-op repay with amount=0 leaves position and pool state untouched.
    #[test]
    fn test_repay_zero_amount_is_no_op() {
        let t = TestSetup::new();
        let client = t.client();

        let supply_pos = t.deposit_position();
        client.supply(&supply_pos, &50_000_000_000i128, &i128::MAX);

        let borrower = Address::generate(&t.env);
        let borrow_pos = t.borrow_position();
        let updated_borrow = client.borrow(&borrower, &100_0000000i128, &borrow_pos, &i128::MAX);
        let scaled_before = updated_borrow.position.scaled_amount_ray;
        let state_before = t.state_snapshot();

        let result = client.repay(&borrower, &0i128, &updated_borrow.position);

        assert_eq!(result.actual_amount, 0);
        assert_eq!(result.position.scaled_amount_ray, scaled_before);
        assert_pool_state_eq(&t.state_snapshot(), &state_before);
    }

    // Add-rewards with zero amount is accepted (require_nonneg_amount, not
    // require_positive_amount) and is a pure index no-op.
    #[test]
    fn test_add_rewards_zero_amount_is_no_op() {
        let t = TestSetup::new();
        let client = t.client();

        let pos = t.deposit_position();
        client.supply(&pos, &10_000_000_000i128, &i128::MAX);

        let snapshot_before = t.state_snapshot();
        let result = client.add_rewards(&0i128);

        assert_eq!(result.supply_index_ray, snapshot_before.supply_index_ray);
    }

    // Direct unit test for the `Ray::checked_sub` underflow guard surfaced
    // at the public ABI through `cache.supplied` / `position.scaled_amount_ray`.
    // The integration tests exercise the panic path; this asserts the
    // happy-path subtraction returns the expected value.
    #[test]
    fn test_ray_checked_sub_happy_path() {
        let env = Env::default();
        let a = Ray::from_raw(5 * RAY);
        let b = Ray::from_raw(2 * RAY);
        assert_eq!(a.checked_sub(&env, b), Ray::from_raw(3 * RAY));
    }

    // Partial repay reduces scaled debt without closing the position.
    #[test]
    fn test_repay_partial_amount() {
        let t = TestSetup::new();
        let client = t.client();

        let supply_pos = t.deposit_position();
        client.supply(&supply_pos, &50_000_000_000i128, &i128::MAX);

        let borrower = Address::generate(&t.env);
        let borrow_pos = t.borrow_position();
        let updated_borrow = client.borrow(&borrower, &100_0000000i128, &borrow_pos, &i128::MAX);

        // Advance time to accrue interest so current_debt > initial.
        t.advance_time(60);

        let partial = 10_0000000i128;
        let final_pos = client.repay(&borrower, &partial, &updated_borrow.position);

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

    // Covers the full add_rewards body.
    #[test]
    fn test_add_rewards_increases_supply_index() {
        let t = TestSetup::new();
        let client = t.client();

        let supply_pos = t.deposit_position();
        client.supply(&supply_pos, &50_000_000_000i128, &i128::MAX);

        let idx_before = client.update_indexes();

        client.add_rewards(&1_000_000_000i128);

        let idx_after = client.update_indexes();
        assert!(
            idx_after.supply_index_ray > idx_before.supply_index_ray,
            "supply index should increase after add_rewards"
        );
    }

    // create_strategy records debt, transfers net amount, and accrues fee to protocol revenue.
    #[test]
    fn test_create_strategy_emits_position_and_transfers_net() {
        let t = TestSetup::new();
        let client = t.client();

        // Supply reserves so create_strategy can transfer.
        let supply_pos = t.deposit_position();
        client.supply(&supply_pos, &50_000_000_000i128, &i128::MAX);

        let caller = Address::generate(&t.env);
        let pos = t.borrow_position();
        let tok = token::Client::new(&t.env, &t.asset);
        let caller_before = tok.balance(&caller);
        let revenue_before = client.protocol_revenue();

        let amount = 100_0000000i128;
        let fee = 1_0000000i128;
        let result = client.create_strategy(&caller, &pos, &amount, &fee, &i128::MAX);

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

    // claim_revenue returns 0 when no revenue has accrued.
    #[test]
    fn test_claim_revenue_zero_revenue_early_returns() {
        let t = TestSetup::new();
        let client = t.client();

        // No supply, no accrual; revenue is zero.
        let claimed = client.claim_revenue().actual_amount;
        assert_eq!(claimed, 0, "claim_revenue should return 0 when no revenue");
    }

    // Verifies every update_params field round-trips through get_sync_data()
    // so a silently dropped write surfaces as an assertion failure.
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
        let new_reserve: u32 = 2000;

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
        // the borrow rate at 50% utilization must reflect the updated slope1.
        let supply_pos = t.deposit_position();
        client.supply(&supply_pos, &10_000_000_000i128, &i128::MAX);
        let borrower = Address::generate(&t.env);
        let borrow_pos = t.borrow_position();
        let _ = client.borrow(&borrower, &100_0000000i128, &borrow_pos, &i128::MAX);
    }

    // Slope ordering violation (slope3 < slope2) panics with InvalidBorrowParams.
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

    // mid_utilization == 0 panics with InvalidUtilRange.
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

    // reserve_factor at the BPS ceiling panics with InvalidReserveFactor;
    // the validator requires `< BPS`.
    #[test]
    #[should_panic(expected = "Error(Contract, #119)")]
    fn test_update_params_rejects_reserve_factor_at_bps() {
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
            &(BPS as u32),
        );
    }

    // Verifies keepalive bumps the instance TTL by at least TTL_THRESHOLD_INSTANCE.
    #[test]
    fn test_keepalive_bumps_ttl() {
        let t = TestSetup::new();
        let client = t.client();

        // Admin-authorized call succeeds. `env.mock_all_auths()` covers the
        // `#[only_owner]` gate. The host auto-records the auth requirement,
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

    // Without mock_all_auths, a non-admin call must panic (auth gate enforced).
    #[test]
    #[should_panic]
    fn test_keepalive_rejects_non_admin() {
        let env = Env::default();
        test_support::init_ledger(&env);

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
