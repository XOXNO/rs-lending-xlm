extern crate std;

use super::*;
use common::constants::{BPS, RAY};
use common::types::ScaledPositionRaw;
use soroban_sdk::testutils::{Address as _, Ledger, LedgerInfo};
use soroban_sdk::{contract, contractimpl, vec, Address, Bytes, Env};

#[contract]
pub struct PoolFlashLoanReceiver;

#[contract]
pub struct PoolNoRepayReceiver;

#[contract]
pub struct PoolUnderRepayReceiver;

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

#[contractimpl]
impl PoolUnderRepayReceiver {
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
        let shortfall = 1i128;
        let partial = total
            .checked_sub(shortfall)
            .unwrap_or_else(|| panic_with_error!(&env, GenericError::MathOverflow));
        let expiration_ledger = env
            .ledger()
            .sequence()
            .checked_add(1)
            .unwrap_or_else(|| panic_with_error!(&env, GenericError::MathOverflow));

        token::Client::new(&env, &asset).approve(
            &env.current_contract_address(),
            &pool,
            &partial,
            &expiration_ledger,
        );
    }
}

fn market_params(asset: &Address) -> MarketParamsRaw {
    MarketParamsRaw {
        max_borrow_rate_ray: RAY,
        base_borrow_rate_ray: RAY / 100,
        slope1_ray: RAY * 4 / 100,
        slope2_ray: RAY * 10 / 100,
        slope3_ray: RAY * 80 / 100,
        mid_utilization_ray: RAY * 50 / 100,
        optimal_utilization_ray: RAY * 80 / 100,
        // RAY sentinel disables max-utilization checks for accounting tests;
        // integration harness covers the ceiling.
        max_utilization_ray: RAY,
        reserve_factor_bps: 1000,
        asset_id: asset.clone(),
        asset_decimals: 7,
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

        test_support::init_ledger(&env);

        // Pool owner receives claimed revenue; controller forwards it to the
        // protocol accumulator.
        let pool_address = env.register(LiquidityPool, (admin.clone(),));
        LiquidityPoolClient::new(&env, &pool_address).create_market(&market_params(&asset_address));

        // Mint tokens to the pool for reserves.
        let token_admin = token::StellarAssetClient::new(&env, &asset_address);
        token_admin.mint(&pool_address, &100_000_000_000_000i128);

        // Seed `cash` to the minted reserve balance; pool liquidity uses `cash`.
        env.as_contract(&pool_address, || {
            let key = PoolKey::State(asset_address.clone());
            let mut state: PoolStateRaw = env.storage().persistent().get(&key).unwrap();
            state.cash = 100_000_000_000_000i128;
            env.storage().persistent().set(&key, &state);
        });

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

    fn action(&self, scaled_amount_ray: i128, amount: i128) -> PoolAction {
        self.action_for(&self.asset, scaled_amount_ray, amount)
    }

    fn action_for(&self, asset: &Address, scaled_amount_ray: i128, amount: i128) -> PoolAction {
        PoolAction {
            position: ScaledPositionRaw { scaled_amount_ray },
            amount,
            asset: asset.clone(),
        }
    }

    fn sup_entry(
        &self,
        asset: &Address,
        scaled_amount_ray: i128,
        amount: i128,
        supply_cap: i128,
    ) -> PoolSupplyEntry {
        PoolSupplyEntry {
            action: self.action_for(asset, scaled_amount_ray, amount),
            supply_cap,
        }
    }

    /// Singleton supply batch against the default market.
    fn sup(&self, scaled_amount_ray: i128, amount: i128, supply_cap: i128) -> Vec<PoolSupplyEntry> {
        self.sup_for(&self.asset, scaled_amount_ray, amount, supply_cap)
    }

    fn sup_for(
        &self,
        asset: &Address,
        scaled_amount_ray: i128,
        amount: i128,
        supply_cap: i128,
    ) -> Vec<PoolSupplyEntry> {
        vec![
            &self.env,
            self.sup_entry(asset, scaled_amount_ray, amount, supply_cap),
        ]
    }

    /// Singleton borrow batch against the default market.
    fn bor(&self, scaled_amount_ray: i128, amount: i128, borrow_cap: i128) -> Vec<PoolBorrowEntry> {
        vec![
            &self.env,
            PoolBorrowEntry {
                action: self.action(scaled_amount_ray, amount),
                borrow_cap,
            },
        ]
    }

    /// Singleton withdraw batch against the default market.
    fn wdr(
        &self,
        scaled_amount_ray: i128,
        amount: i128,
        protocol_fee: i128,
    ) -> Vec<PoolWithdrawEntry> {
        vec![
            &self.env,
            PoolWithdrawEntry {
                action: self.action(scaled_amount_ray, amount),
                protocol_fee,
            },
        ]
    }

    /// Singleton repay batch against the default market.
    fn ract(&self, scaled_amount_ray: i128, amount: i128) -> Vec<PoolAction> {
        vec![&self.env, self.action(scaled_amount_ray, amount)]
    }

    /// Registers a funded market with a fresh SAC token, minted reserves, and
    /// seeded internal `cash`.
    fn add_funded_market(&self) -> Address {
        let asset = self
            .env
            .register_stellar_asset_contract_v2(self.admin.clone())
            .address()
            .clone();
        self.client().create_market(&market_params(&asset));
        token::StellarAssetClient::new(&self.env, &asset)
            .mint(&self.pool, &100_000_000_000_000i128);
        self.env.as_contract(&self.pool, || {
            let key = PoolKey::State(asset.clone());
            let mut state: PoolStateRaw = self.env.storage().persistent().get(&key).unwrap();
            state.cash = 100_000_000_000_000i128;
            self.env.storage().persistent().set(&key, &state);
        });
        asset
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

    fn edit_state(&self, edit: impl FnOnce(&mut PoolStateRaw)) {
        self.env.as_contract(&self.pool, || {
            let key = PoolKey::State(self.asset.clone());
            let mut state: PoolStateRaw = self.env.storage().persistent().get(&key).unwrap();
            edit(&mut state);
            self.env.storage().persistent().set(&key, &state);
        });
    }

    fn state_of(&self, asset: &Address) -> PoolStateRaw {
        self.env.as_contract(&self.pool, || {
            self.env
                .storage()
                .persistent()
                .get(&PoolKey::State(asset.clone()))
                .unwrap()
        })
    }

    fn state_snapshot(&self) -> PoolStateRaw {
        self.state_of(&self.asset)
    }
}

fn assert_pool_state_eq(left: &PoolStateRaw, right: &PoolStateRaw) {
    assert_eq!(left.supplied_ray, right.supplied_ray);
    assert_eq!(left.borrowed_ray, right.borrowed_ray);
    assert_eq!(left.revenue_ray, right.revenue_ray);
    assert_eq!(left.borrow_index_ray, right.borrow_index_ray);
    assert_eq!(left.supply_index_ray, right.supply_index_ray);
    assert_eq!(left.last_timestamp, right.last_timestamp);
}

fn flatten_contract_result<T>(
    result: Result<
        Result<T, soroban_sdk::ConversionError>,
        Result<soroban_sdk::Error, soroban_sdk::InvokeError>,
    >,
) -> Result<T, soroban_sdk::Error> {
    match result {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(err)) => panic!("contract call succeeded but output conversion failed: {err:?}"),
        Err(invoke) => Err(invoke.expect("expected contract error, got host-level InvokeError")),
    }
}

fn assert_contract_error<T: core::fmt::Debug>(
    result: Result<T, soroban_sdk::Error>,
    expected_code: u32,
) {
    match result {
        Ok(value) => panic!("expected contract error {expected_code}, got Ok({value:?})"),
        Err(err) => assert_eq!(
            err,
            soroban_sdk::Error::from_contract_error(expected_code),
            "unexpected contract error"
        ),
    }
}
#[test]
fn test_supply() {
    let t = TestSetup::new();
    let client = t.client();

    let amount = 10_000_000_000i128;

    let updated = client.supply(&t.sup(0, amount, i128::MAX)).get_unchecked(0);

    assert!(
        updated.position.scaled_amount_ray > 0,
        "position should have scaled amount"
    );

    let supplied = client.supplied_amount(&t.asset);
    assert!(supplied > 0, "supplied_amount should be positive");
}
#[test]
fn test_borrow() {
    let t = TestSetup::new();
    let client = t.client();

    client.supply(&t.sup(0, 50_000_000_000i128, i128::MAX));

    let borrower = Address::generate(&t.env);
    let borrow_amount = 100_0000000i128;

    let reserves_before = client.reserves(&t.asset);
    let updated = client
        .borrow(&borrower, &t.bor(0, borrow_amount, i128::MAX))
        .get_unchecked(0);

    assert!(
        updated.position.scaled_amount_ray > 0,
        "borrow position should have debt"
    );

    let reserves_after = client.reserves(&t.asset);
    assert!(
        reserves_after < reserves_before,
        "reserves should decrease after borrow"
    );
}

#[test]
fn test_supply_cap_enforced_after_pool_sync() {
    let t = TestSetup::new();
    let client = t.client();

    let amount = 10_000_000_000i128;

    let result = flatten_contract_result(client.try_supply(&t.sup(0, amount, amount - 1)));
    assert_contract_error(
        result,
        common::errors::CollateralError::SupplyCapReached as u32,
    );
}

#[test]
fn test_borrow_cap_enforced_after_pool_sync() {
    let t = TestSetup::new();
    let client = t.client();

    client.supply(&t.sup(0, 50_000_000_000i128, i128::MAX));

    let borrower = Address::generate(&t.env);
    let borrow_amount = 100_0000000i128;

    let result = flatten_contract_result(
        client.try_borrow(&borrower, &t.bor(0, borrow_amount, borrow_amount - 1)),
    );
    assert_contract_error(
        result,
        common::errors::CollateralError::BorrowCapReached as u32,
    );
}

#[test]
fn test_strategy_borrow_cap_enforced_after_pool_sync() {
    let t = TestSetup::new();
    let client = t.client();

    client.supply(&t.sup(0, 50_000_000_000i128, i128::MAX));

    let caller = Address::generate(&t.env);
    let amount = 100_0000000i128;

    let result = flatten_contract_result(client.try_create_strategy(
        &caller,
        &t.action(0, amount),
        &0i128,
        &(amount - 1),
    ));
    assert_contract_error(
        result,
        common::errors::CollateralError::BorrowCapReached as u32,
    );
}

#[test]
fn test_borrow_rejects_when_reserves_are_insufficient() {
    let t = TestSetup::new();
    let client = t.client();
    let borrower = Address::generate(&t.env);

    let result = flatten_contract_result(
        client.try_borrow(&borrower, &t.bor(0, 200_000_000_000_000i128, i128::MAX)),
    );
    assert_contract_error(
        result,
        common::errors::CollateralError::InsufficientLiquidity as u32,
    );
}
#[test]
fn test_withdraw() {
    let t = TestSetup::new();
    let client = t.client();

    let supply_amount = 10_000_000_000i128;
    let updated_pos = client
        .supply(&t.sup(0, supply_amount, i128::MAX))
        .get_unchecked(0);

    let user = Address::generate(&t.env);
    let tok = token::Client::new(&t.env, &t.asset);
    let user_balance_before = tok.balance(&user);

    let withdraw_amount = 500_0000000i128;
    let final_pos = client
        .withdraw(
            &user,
            &false,
            &t.wdr(
                updated_pos.position.scaled_amount_ray,
                withdraw_amount,
                0i128,
            ),
        )
        .get_unchecked(0);

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
fn test_withdraw_rejects_fee_greater_than_withdrawn_amount() {
    let t = TestSetup::new();
    let client = t.client();

    let updated_pos = client
        .supply(&t.sup(0, 10_000_000i128, i128::MAX))
        .get_unchecked(0);
    let user = Address::generate(&t.env);

    let result = flatten_contract_result(client.try_withdraw(
        &user,
        &true,
        &t.wdr(
            updated_pos.position.scaled_amount_ray,
            1_0000000i128,
            2_0000000i128,
        ),
    ));
    assert_contract_error(
        result,
        common::errors::CollateralError::WithdrawLessThanFee as u32,
    );
}

// Post-state utilization gate with a 50% cap.
#[test]
fn test_borrow_above_max_utilization_panics() {
    let t = TestSetup::new();
    t.edit_state(|s| {
        // Pre-seed supplied so utilization is defined.
        s.supplied_ray = 100_000_000_000_000;
        s.borrowed_ray = 0;
    });
    // Tighten the cap to 50 %.
    t.env.as_contract(&t.pool, || {
        let key = PoolKey::Params(t.asset.clone());
        let mut params: MarketParamsRaw = t.env.storage().persistent().get(&key).unwrap();
        params.max_utilization_ray = RAY / 2;
        t.env.storage().persistent().set(&key, &params);
    });

    let client = t.client();
    let borrower = Address::generate(&t.env);
    // Borrow above 50% of supplied reverts with UtilizationAboveMax.
    let result =
        flatten_contract_result(client.try_borrow(&borrower, &t.bor(0, 60_000i128, i128::MAX)));
    assert_contract_error(
        result,
        common::errors::CollateralError::UtilizationAboveMax as u32,
    );
}

#[test]
fn test_withdraw_rejects_when_reserves_are_insufficient() {
    let t = TestSetup::new();
    let client = t.client();

    let updated_pos = client
        .supply(&t.sup(0, 10_000_000_000i128, i128::MAX))
        .get_unchecked(0);

    let borrower = Address::generate(&t.env);
    client.borrow(&borrower, &t.bor(0, 99_999_990_000_000i128, i128::MAX));

    // Reserves are tracked as `cash`; drain it below the withdrawal amount so
    // the insufficient-liquidity guard fires.
    t.edit_state(|s| s.cash = 5_000_000_000i128);

    let user = Address::generate(&t.env);
    let result = flatten_contract_result(client.try_withdraw(
        &user,
        &false,
        &t.wdr(
            updated_pos.position.scaled_amount_ray,
            10_000_000_000i128,
            0i128,
        ),
    ));
    assert_contract_error(
        result,
        common::errors::CollateralError::InsufficientLiquidity as u32,
    );
}

#[test]
fn test_withdraw_rejects_supplied_accounting_underflow() {
    let t = TestSetup::new();
    let client = t.client();

    let updated_pos = client
        .supply(&t.sup(0, 10_000_000_000i128, i128::MAX))
        .get_unchecked(0);
    t.edit_state(|state| {
        state.supplied_ray = 1;
    });

    let user = Address::generate(&t.env);
    let result = flatten_contract_result(client.try_withdraw(
        &user,
        &false,
        &t.wdr(updated_pos.position.scaled_amount_ray, i128::MAX, 0i128),
    ));
    assert_contract_error(result, common::errors::GenericError::MathOverflow as u32);
}
#[test]
fn test_repay() {
    let t = TestSetup::new();
    let client = t.client();

    client.supply(&t.sup(0, 50_000_000_000i128, i128::MAX));

    let borrower = Address::generate(&t.env);
    let updated_borrow = client
        .borrow(&borrower, &t.bor(0, 100_0000000i128, i128::MAX))
        .get_unchecked(0);

    assert!(updated_borrow.position.scaled_amount_ray > 0);

    // Exact repay; no overpayment because no time has passed.
    let repay_amount = 100_0000000i128;
    let final_pos = client
        .repay(
            &borrower,
            &t.ract(updated_borrow.position.scaled_amount_ray, repay_amount),
        )
        .get_unchecked(0);

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

    client.supply(&t.sup(0, 50_000_000_000i128, i128::MAX));

    let borrower = Address::generate(&t.env);
    let updated_borrow = client
        .borrow(&borrower, &t.bor(0, 100_0000000i128, i128::MAX))
        .get_unchecked(0);

    let repay_amount = 200_0000000i128;
    let final_pos = client
        .repay(
            &borrower,
            &t.ract(updated_borrow.position.scaled_amount_ray, repay_amount),
        )
        .get_unchecked(0);

    assert_eq!(final_pos.actual_amount, 100_0000000i128);
    assert_eq!(final_pos.position.scaled_amount_ray, 0);
}
#[test]
fn test_interest_accrual() {
    let t = TestSetup::new();
    let client = t.client();

    client.supply(&t.sup(0, 50_000_000_000i128, i128::MAX));

    let borrower = Address::generate(&t.env);
    client.borrow(&borrower, &t.bor(0, 10_000_000_000i128, i128::MAX));

    let initial_indexes = client.update_indexes(&t.asset);

    // Advance time by ~1 year.
    t.advance_time(31_556_926);

    let new_indexes = client.update_indexes(&t.asset);

    assert!(
        new_indexes.borrow_index_ray > initial_indexes.borrow_index_ray,
        "borrow index should increase over time"
    );
    assert!(
        new_indexes.supply_index_ray > initial_indexes.supply_index_ray,
        "supply index should increase over time"
    );
}
#[test]
fn test_flash_loan() {
    let t = TestSetup::new();
    let client = t.client();

    client.supply(&t.sup(0, 10_000_000_000i128, i128::MAX));

    let receiver = t.env.register(PoolFlashLoanReceiver, ());
    let flash_amount = 100_0000000i128;
    let flash_fee = 1_0000000i128;

    // The pool will send `amount`; pre-fund only the fee.
    let token_admin_client = token::StellarAssetClient::new(&t.env, &t.asset);
    token_admin_client.mint(&receiver, &flash_fee);

    let tok = token::Client::new(&t.env, &t.asset);
    let pool_balance_before = tok.balance(&t.pool);
    let revenue_before = client.protocol_revenue(&t.asset);
    client.flash_loan(
        &t.asset,
        &t.admin,
        &receiver,
        &flash_amount,
        &flash_fee,
        &Bytes::new(&t.env),
    );
    let revenue_after = client.protocol_revenue(&t.asset);
    let pool_balance_after = tok.balance(&t.pool);

    assert_eq!(pool_balance_after, pool_balance_before + flash_fee);
    assert_eq!(revenue_after, revenue_before + flash_fee);
}

#[test]
fn test_flash_loan_rejects_zero_amount_at_pool() {
    let t = TestSetup::new();
    let client = t.client();
    let receiver = t.env.register(PoolFlashLoanReceiver, ());

    let result = flatten_contract_result(client.try_flash_loan(
        &t.asset,
        &t.admin,
        &receiver,
        &0i128,
        &0i128,
        &Bytes::new(&t.env),
    ));

    assert_contract_error(
        result,
        common::errors::GenericError::AmountMustBePositive as u32,
    );
}

#[test]
fn test_flash_loan_rejects_non_contract_receiver_at_pool() {
    let t = TestSetup::new();
    let client = t.client();
    let receiver = Address::generate(&t.env);

    let result = flatten_contract_result(client.try_flash_loan(
        &t.asset,
        &t.admin,
        &receiver,
        &1_0000000i128,
        &0i128,
        &Bytes::new(&t.env),
    ));

    assert_contract_error(
        result,
        common::errors::FlashLoanError::InvalidFlashloanReceiver as u32,
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
        &t.asset,
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
fn test_flash_loan_rejects_under_repay_with_invalid_flashloan_repay() {
    let t = TestSetup::new();
    let client = t.client();
    let receiver = t.env.register(PoolUnderRepayReceiver, ());
    let flash_amount = 100_0000000i128;
    let flash_fee = 1_0000000i128;

    client.supply(&t.sup(0, 10_000_000_000i128, i128::MAX));

    token::StellarAssetClient::new(&t.env, &t.asset).mint(&receiver, &flash_fee);

    let result = flatten_contract_result(client.try_flash_loan(
        &t.asset,
        &t.admin,
        &receiver,
        &flash_amount,
        &flash_fee,
        &Bytes::new(&t.env),
    ));

    assert_contract_error(
        result,
        common::errors::FlashLoanError::InvalidFlashloanRepay as u32,
    );
}

#[test]
fn test_flash_loan_callback_failure_rolls_back_pool_state() {
    let t = TestSetup::new();
    let client = t.client();
    let receiver = t.env.register(PoolNoRepayReceiver, ());
    let tok = token::Client::new(&t.env, &t.asset);

    let balance_before = tok.balance(&t.pool);
    let revenue_before = client.protocol_revenue(&t.asset);
    let state_before = t.state_snapshot();

    let result = client.try_flash_loan(
        &t.asset,
        &t.admin,
        &receiver,
        &1_0000000i128,
        &1_000i128,
        &Bytes::new(&t.env),
    );

    assert!(result.is_err(), "receiver that does not repay must fail");
    assert_eq!(tok.balance(&t.pool), balance_before);
    assert_eq!(client.protocol_revenue(&t.asset), revenue_before);
    assert_pool_state_eq(&t.state_snapshot(), &state_before);
}

#[test]
fn test_flash_loan_rejects_insufficient_liquidity() {
    let t = TestSetup::new();
    let client = t.client();
    let receiver = Address::generate(&t.env);

    let result = flatten_contract_result(client.try_flash_loan(
        &t.asset,
        &t.admin,
        &receiver,
        &200_000_000_000_000i128,
        &0i128,
        &Bytes::new(&t.env),
    ));
    assert_contract_error(
        result,
        common::errors::CollateralError::InsufficientLiquidity as u32,
    );
}

#[test]
fn test_flash_loan_rejects_negative_fee() {
    let t = TestSetup::new();
    let client = t.client();
    let receiver = Address::generate(&t.env);

    let result = flatten_contract_result(client.try_flash_loan(
        &t.asset,
        &t.admin,
        &receiver,
        &1_0000000i128,
        &-1i128,
        &Bytes::new(&t.env),
    ));
    assert_contract_error(
        result,
        common::errors::GenericError::AmountMustBePositive as u32,
    );
}

#[test]
fn test_create_strategy_rejects_fee_greater_than_amount() {
    let t = TestSetup::new();
    let client = t.client();
    let caller = Address::generate(&t.env);

    let result = flatten_contract_result(client.try_create_strategy(
        &caller,
        &t.action(0, 1_0000000i128),
        &2_0000000i128,
        &i128::MAX,
    ));
    assert_contract_error(
        result,
        common::errors::FlashLoanError::StrategyFeeExceeds as u32,
    );
}

#[test]
fn test_create_strategy_rejects_insufficient_liquidity() {
    let t = TestSetup::new();
    let client = t.client();
    let caller = Address::generate(&t.env);

    let result = flatten_contract_result(client.try_create_strategy(
        &caller,
        &t.action(0, 200_000_000_000_000i128),
        &0i128,
        &i128::MAX,
    ));
    assert_contract_error(
        result,
        common::errors::CollateralError::InsufficientLiquidity as u32,
    );
}
#[test]
fn test_seize_position_bad_debt() {
    let t = TestSetup::new();
    let client = t.client();

    client.supply(&t.sup(0, 50_000_000_000i128, i128::MAX));

    let borrower = Address::generate(&t.env);
    let updated_borrow = client
        .borrow(&borrower, &t.bor(0, 100_0000000i128, i128::MAX))
        .get_unchecked(0);

    let idx_before = client.update_indexes(&t.asset);

    let seized = client.seize_position(
        &t.asset,
        &AccountPositionType::Borrow,
        &updated_borrow.position,
    );

    assert_eq!(
        seized.position.scaled_amount_ray, 0,
        "position should be zeroed"
    );

    let idx_after = client.update_indexes(&t.asset);
    assert!(
        idx_after.supply_index_ray <= idx_before.supply_index_ray,
        "supply index should decrease or stay same after bad debt"
    );
}

#[test]
fn test_seize_position_rejects_borrowed_accounting_underflow() {
    let t = TestSetup::new();
    let client = t.client();

    client.supply(&t.sup(0, 50_000_000_000i128, i128::MAX));

    let borrower = Address::generate(&t.env);
    let updated_borrow = client
        .borrow(&borrower, &t.bor(0, 100_0000000i128, i128::MAX))
        .get_unchecked(0);

    t.edit_state(|state| {
        state.borrowed_ray = 0;
    });

    let result = flatten_contract_result(client.try_seize_position(
        &t.asset,
        &AccountPositionType::Borrow,
        &updated_borrow.position,
    ));
    assert_contract_error(result, common::errors::GenericError::MathOverflow as u32);
}
#[test]
fn test_seize_position_deposit_dust() {
    let t = TestSetup::new();
    let client = t.client();

    let updated = client
        .supply(&t.sup(0, 100_0000000i128, i128::MAX))
        .get_unchecked(0);

    let revenue_before = client.protocol_revenue(&t.asset);
    let seized = client.seize_position(&t.asset, &AccountPositionType::Deposit, &updated.position);

    assert_eq!(
        seized.position.scaled_amount_ray, 0,
        "position should be zeroed"
    );

    let revenue_after = client.protocol_revenue(&t.asset);
    assert!(
        revenue_after > revenue_before,
        "protocol revenue should increase from absorbed dust"
    );
}
#[test]
fn test_claim_revenue() {
    let t = TestSetup::new();
    let client = t.client();

    client.supply(&t.sup(0, 50_000_000_000i128, i128::MAX));

    let borrower = Address::generate(&t.env);
    client.borrow(&borrower, &t.bor(0, 10_000_000_000i128, i128::MAX));

    // Advance time to accrue interest.
    t.advance_time(31_556_926);

    // Sync indexes to accrue revenue.
    client.update_indexes(&t.asset);

    let revenue = client.protocol_revenue(&t.asset);
    if revenue > 0 {
        let tok = token::Client::new(&t.env, &t.asset);
        let admin_balance_before = tok.balance(&t.admin);
        let claimed = client.claim_revenue(&t.asset).actual_amount;
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

    let oversized_supply = client
        .supply(&t.sup(0, 200_000_000_000_000i128, i128::MAX))
        .get_unchecked(0);
    let _ = client.seize_position(
        &t.asset,
        &AccountPositionType::Deposit,
        &oversized_supply.position,
    );

    // Reserves below revenue cap the claim at available `cash` and leave
    // residual revenue.
    t.edit_state(|s| s.cash = 100_000_000_000_000i128);

    let claimed = client.claim_revenue(&t.asset).actual_amount;
    let remaining_revenue = client.protocol_revenue(&t.asset);

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
fn test_claim_revenue_rejects_revenue_above_supplied() {
    let t = TestSetup::new();
    let client = t.client();

    let supplied = client
        .supply(&t.sup(0, 10_000_000_000i128, i128::MAX))
        .get_unchecked(0);
    let _ = client.seize_position(&t.asset, &AccountPositionType::Deposit, &supplied.position);
    t.edit_state(|state| {
        state.supplied_ray = 1;
    });

    let result = flatten_contract_result(client.try_claim_revenue(&t.asset));
    assert_contract_error(result, common::errors::GenericError::MathOverflow as u32);
}

#[test]
fn test_update_params_rejects_invalid_utilization_range() {
    let t = TestSetup::new();
    let client = t.client();

    let model = InterestRateModel {
        max_borrow_rate_ray: 2 * RAY,
        base_borrow_rate_ray: RAY / 100,
        slope1_ray: RAY / 10,
        slope2_ray: RAY / 5,
        slope3_ray: RAY,
        mid_utilization_ray: RAY * 8 / 10,
        optimal_utilization_ray: RAY * 8 / 10,
        max_utilization_ray: RAY * 95 / 100,
        reserve_factor_bps: 1000,
    };
    let result = flatten_contract_result(client.try_update_params(&t.asset, &model));
    assert_contract_error(
        result,
        common::errors::CollateralError::InvalidUtilRange as u32,
    );
}

#[test]
fn test_update_params_rejects_optimal_utilization_above_one() {
    let t = TestSetup::new();
    let client = t.client();

    let model = InterestRateModel {
        max_borrow_rate_ray: 2 * RAY,
        base_borrow_rate_ray: RAY / 100,
        slope1_ray: RAY / 10,
        slope2_ray: RAY / 5,
        slope3_ray: RAY,
        mid_utilization_ray: RAY / 2,
        optimal_utilization_ray: RAY,
        max_utilization_ray: RAY * 95 / 100,
        reserve_factor_bps: 1000,
    };
    let result = flatten_contract_result(client.try_update_params(&t.asset, &model));
    assert_contract_error(
        result,
        common::errors::CollateralError::OptUtilTooHigh as u32,
    );
}

#[test]
fn test_update_params_rejects_invalid_reserve_factor() {
    let t = TestSetup::new();
    let client = t.client();

    let model = InterestRateModel {
        max_borrow_rate_ray: 2 * RAY,
        base_borrow_rate_ray: RAY / 100,
        slope1_ray: RAY / 10,
        slope2_ray: RAY / 5,
        slope3_ray: RAY,
        mid_utilization_ray: RAY / 2,
        optimal_utilization_ray: RAY * 8 / 10,
        max_utilization_ray: RAY * 95 / 100,
        reserve_factor_bps: 10_000,
    };
    let result = flatten_contract_result(client.try_update_params(&t.asset, &model));
    assert_contract_error(
        result,
        common::errors::CollateralError::InvalidReserveFactor as u32,
    );
}

#[test]
fn test_update_params_rejects_negative_base_rate() {
    let t = TestSetup::new();
    let client = t.client();

    let model = InterestRateModel {
        max_borrow_rate_ray: 2 * RAY,
        base_borrow_rate_ray: -1i128,
        slope1_ray: RAY / 10,
        slope2_ray: RAY / 5,
        slope3_ray: RAY,
        mid_utilization_ray: RAY / 2,
        optimal_utilization_ray: RAY * 8 / 10,
        max_utilization_ray: RAY * 95 / 100,
        reserve_factor_bps: 1000,
    };
    let result = flatten_contract_result(client.try_update_params(&t.asset, &model));
    assert_contract_error(
        result,
        common::errors::CollateralError::BaseRateNegative as u32,
    );
}

#[test]
fn test_update_params_rejects_max_rate_not_above_base_rate() {
    let t = TestSetup::new();
    let client = t.client();

    // Flat slopes keep SlopeNonMonotonic from pre-empting MaxRateBelowBase.
    let model = InterestRateModel {
        max_borrow_rate_ray: RAY / 100,
        base_borrow_rate_ray: RAY / 100,
        slope1_ray: RAY / 100,
        slope2_ray: RAY / 100,
        slope3_ray: RAY / 100,
        mid_utilization_ray: RAY / 2,
        optimal_utilization_ray: RAY * 8 / 10,
        max_utilization_ray: RAY * 95 / 100,
        reserve_factor_bps: 1000,
    };
    let result = flatten_contract_result(client.try_update_params(&t.asset, &model));
    assert_contract_error(
        result,
        common::errors::CollateralError::MaxRateBelowBase as u32,
    );
}

#[test]
fn test_update_params_rejects_max_borrow_rate_above_cap() {
    let t = TestSetup::new();
    let client = t.client();

    // `2 * RAY + 1` exceeds MAX_BORROW_RATE_RAY; slopes are below the cap.
    let model = InterestRateModel {
        max_borrow_rate_ray: 2 * RAY + 1,
        base_borrow_rate_ray: RAY / 100,
        slope1_ray: RAY / 10,
        slope2_ray: RAY / 5,
        slope3_ray: RAY,
        mid_utilization_ray: RAY / 2,
        optimal_utilization_ray: RAY * 8 / 10,
        max_utilization_ray: RAY * 95 / 100,
        reserve_factor_bps: 1000,
    };
    let result = flatten_contract_result(client.try_update_params(&t.asset, &model));
    assert_contract_error(
        result,
        common::errors::CollateralError::MaxBorrowRateTooHigh as u32,
    );
}

#[test]
fn test_views() {
    let t = TestSetup::new();
    let client = t.client();

    let util = client.capital_utilisation(&t.asset);
    assert_eq!(util, 0, "utilization should be zero initially");

    client.supply(&t.sup(0, 10_000_000_000i128, i128::MAX));

    let supplied = client.supplied_amount(&t.asset);
    assert!(
        supplied > 0,
        "supplied_amount should be positive after supply"
    );

    let reserves = client.reserves(&t.asset);
    assert!(reserves > 0, "reserves should be positive");

    let borrower = Address::generate(&t.env);
    client.borrow(&borrower, &t.bor(0, 100_0000000i128, i128::MAX));

    let borrowed = client.borrowed_amount(&t.asset);
    assert!(borrowed > 0, "borrowed_amount should be positive");

    let util_after = client.capital_utilisation(&t.asset);
    assert!(
        util_after > 0,
        "utilization should be positive after borrow"
    );

    assert!(
        client.deposit_rate(&t.asset) >= 0,
        "deposit rate view should be callable"
    );
    assert!(
        client.borrow_rate(&t.asset) >= 0,
        "borrow rate view should be callable"
    );
    assert!(
        client.protocol_revenue(&t.asset) >= 0,
        "protocol revenue view should be callable"
    );
    t.advance_time(60);
    assert!(
        client.delta_time(&t.asset) > 0,
        "delta_time should be positive"
    );
}

// Liquidation fee on withdraw accrues to protocol revenue; user receives gross minus fee.
#[test]
fn test_withdraw_liquidation_fee_accrues_to_revenue() {
    let t = TestSetup::new();
    let client = t.client();

    let supply_amount = 10_000_000_000i128;
    let updated_pos = client
        .supply(&t.sup(0, supply_amount, i128::MAX))
        .get_unchecked(0);

    let revenue_before = client.protocol_revenue(&t.asset);

    let user = Address::generate(&t.env);
    let tok = token::Client::new(&t.env, &t.asset);
    let user_balance_before = tok.balance(&user);

    let gross = 10_000_000_000_i128;
    let fee = 10_000_000_i128;
    let final_pos = client
        .withdraw(
            &user,
            &true,
            &t.wdr(updated_pos.position.scaled_amount_ray, gross, fee),
        )
        .get_unchecked(0);

    let user_balance_after = tok.balance(&user);
    assert_eq!(
        user_balance_after - user_balance_before,
        gross - fee,
        "user should receive gross minus protocol fee"
    );
    let revenue_after = client.protocol_revenue(&t.asset);
    assert!(
        revenue_after > revenue_before,
        "protocol revenue should increase by fee"
    );
    assert_eq!(final_pos.actual_amount, gross);
}

// `is_liquidation=true` with `protocol_fee=0` skips fee accrual and follows
// regular withdraw.
#[test]
fn test_withdraw_liquidation_with_zero_protocol_fee_is_no_op() {
    let t = TestSetup::new();
    let client = t.client();

    let supply_amount = 10_000_000_000i128;
    let updated_pos = client
        .supply(&t.sup(0, supply_amount, i128::MAX))
        .get_unchecked(0);

    let revenue_before = client.protocol_revenue(&t.asset);
    let user = Address::generate(&t.env);
    let tok = token::Client::new(&t.env, &t.asset);
    let user_balance_before = tok.balance(&user);

    let gross = 1_000_000_000_i128;
    let final_pos = client
        .withdraw(
            &user,
            &true,
            &t.wdr(updated_pos.position.scaled_amount_ray, gross, 0i128),
        )
        .get_unchecked(0);

    assert_eq!(tok.balance(&user) - user_balance_before, gross);
    assert_eq!(client.protocol_revenue(&t.asset), revenue_before);
    assert_eq!(final_pos.actual_amount, gross);
}

// No-op repay with amount=0 leaves position and pool state untouched.
#[test]
fn test_repay_zero_amount_is_no_op() {
    let t = TestSetup::new();
    let client = t.client();

    client.supply(&t.sup(0, 50_000_000_000i128, i128::MAX));

    let borrower = Address::generate(&t.env);
    let updated_borrow = client
        .borrow(&borrower, &t.bor(0, 100_0000000i128, i128::MAX))
        .get_unchecked(0);
    let scaled_before = updated_borrow.position.scaled_amount_ray;
    let state_before = t.state_snapshot();

    let result = client
        .repay(&borrower, &t.ract(scaled_before, 0i128))
        .get_unchecked(0);

    assert_eq!(result.actual_amount, 0);
    assert_eq!(result.position.scaled_amount_ray, scaled_before);
    assert_pool_state_eq(&t.state_snapshot(), &state_before);
}

// Zero-amount add_rewards is accepted by `require_nonneg_amount` and leaves
// the index unchanged.
#[test]
fn test_add_rewards_zero_amount_is_no_op() {
    let t = TestSetup::new();
    let client = t.client();

    client.supply(&t.sup(0, 10_000_000_000i128, i128::MAX));

    let snapshot_before = t.state_snapshot();
    let result = client.add_rewards(&t.asset, &0i128);

    assert_eq!(result.supply_index_ray, snapshot_before.supply_index_ray);
}

// Public ABI panic tests cover `Ray::checked_sub` underflow through
// supplied/position accounting; this case checks normal subtraction.
#[test]
fn test_ray_checked_sub_happy_path() {
    let env = Env::default();
    let a = Ray::from(5 * RAY);
    let b = Ray::from(2 * RAY);
    assert_eq!(a.checked_sub(&env, b), Ray::from(3 * RAY));
}

// Partial repay reduces scaled debt without closing the position.
#[test]
fn test_repay_partial_amount() {
    let t = TestSetup::new();
    let client = t.client();

    client.supply(&t.sup(0, 50_000_000_000i128, i128::MAX));

    let borrower = Address::generate(&t.env);
    let updated_borrow = client
        .borrow(&borrower, &t.bor(0, 100_0000000i128, i128::MAX))
        .get_unchecked(0);

    // Advance time to accrue interest so current_debt > initial.
    t.advance_time(60);

    let partial = 10_0000000i128;
    let final_pos = client
        .repay(
            &borrower,
            &t.ract(updated_borrow.position.scaled_amount_ray, partial),
        )
        .get_unchecked(0);

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

#[test]
fn test_add_rewards_increases_supply_index() {
    let t = TestSetup::new();
    let client = t.client();

    client.supply(&t.sup(0, 50_000_000_000i128, i128::MAX));

    let idx_before = client.update_indexes(&t.asset);

    client.add_rewards(&t.asset, &1_000_000_000i128);

    let idx_after = client.update_indexes(&t.asset);
    assert!(
        idx_after.supply_index_ray > idx_before.supply_index_ray,
        "supply index should increase after add_rewards"
    );
}

// create_strategy records debt, transfers net amount, and accrues fee as protocol revenue.
#[test]
fn test_create_strategy_emits_position_and_transfers_net() {
    let t = TestSetup::new();
    let client = t.client();

    // Supply reserves so create_strategy can transfer.
    client.supply(&t.sup(0, 50_000_000_000i128, i128::MAX));

    let caller = Address::generate(&t.env);
    let tok = token::Client::new(&t.env, &t.asset);
    let caller_before = tok.balance(&caller);
    let revenue_before = client.protocol_revenue(&t.asset);

    let amount = 100_0000000i128;
    let fee = 1_0000000i128;
    let result = client.create_strategy(&caller, &t.action(0, amount), &fee, &i128::MAX);

    assert_eq!(result.actual_amount, amount);
    assert_eq!(result.amount_received, amount - fee);
    assert!(result.position.scaled_amount_ray > 0, "debt recorded");

    let caller_after = tok.balance(&caller);
    assert_eq!(
        caller_after - caller_before,
        amount - fee,
        "caller receives net amount"
    );
    let revenue_after = client.protocol_revenue(&t.asset);
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
    let claimed = client.claim_revenue(&t.asset).actual_amount;
    assert_eq!(claimed, 0, "claim_revenue should return 0 when no revenue");
}

// update_params fields round-trip through get_sync_data().
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

    let model = InterestRateModel {
        max_borrow_rate_ray: new_max,
        base_borrow_rate_ray: new_base,
        slope1_ray: new_s1,
        slope2_ray: new_s2,
        slope3_ray: new_s3,
        mid_utilization_ray: new_mid,
        optimal_utilization_ray: new_opt,
        max_utilization_ray: RAY * 95 / 100,
        reserve_factor_bps: new_reserve,
    };
    client.update_params(&t.asset, &model);

    // Updated fields round-trip through get_sync_data().
    let sync = client.get_sync_data(&t.asset);
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

    // With base rate still 1% and higher slopes, 50% utilization uses updated slope1.
    client.supply(&t.sup(0, 10_000_000_000i128, i128::MAX));
    let borrower = Address::generate(&t.env);
    let _ = client.borrow(&borrower, &t.bor(0, 100_0000000i128, i128::MAX));
}

// slope3 < slope2 maps to SlopeNonMonotonic.
#[test]
fn test_update_params_rejects_invalid_slope_ordering() {
    let t = TestSetup::new();
    let client = t.client();

    // slope3 < slope2 is invalid.
    let model = InterestRateModel {
        max_borrow_rate_ray: 2 * RAY,
        base_borrow_rate_ray: RAY / 100,
        slope1_ray: RAY / 10,
        slope2_ray: RAY / 2,
        slope3_ray: RAY / 5,
        mid_utilization_ray: RAY / 2,
        optimal_utilization_ray: RAY * 8 / 10,
        max_utilization_ray: RAY * 95 / 100,
        reserve_factor_bps: 1000,
    };
    let result = flatten_contract_result(client.try_update_params(&t.asset, &model));
    assert_contract_error(
        result,
        common::errors::CollateralError::SlopeNonMonotonic as u32,
    );
}

// mid_utilization == 0 maps to InvalidUtilRange.
#[test]
fn test_update_params_rejects_mid_utilization_zero() {
    let t = TestSetup::new();
    let client = t.client();

    let model = InterestRateModel {
        max_borrow_rate_ray: 2 * RAY,
        base_borrow_rate_ray: RAY / 100,
        slope1_ray: RAY / 10,
        slope2_ray: RAY / 5,
        slope3_ray: RAY,
        mid_utilization_ray: 0i128,
        optimal_utilization_ray: RAY * 8 / 10,
        max_utilization_ray: RAY * 95 / 100,
        reserve_factor_bps: 1000,
    };
    let result = flatten_contract_result(client.try_update_params(&t.asset, &model));
    assert_contract_error(
        result,
        common::errors::CollateralError::InvalidUtilRange as u32,
    );
}

// reserve_factor == BPS maps to InvalidReserveFactor;
// the validator requires `< BPS`.
#[test]
fn test_update_params_rejects_reserve_factor_at_bps() {
    let t = TestSetup::new();
    let client = t.client();

    let model = InterestRateModel {
        max_borrow_rate_ray: 2 * RAY,
        base_borrow_rate_ray: RAY / 100,
        slope1_ray: RAY / 10,
        slope2_ray: RAY / 5,
        slope3_ray: RAY,
        mid_utilization_ray: RAY / 2,
        optimal_utilization_ray: RAY * 8 / 10,
        max_utilization_ray: RAY * 95 / 100,
        reserve_factor_bps: BPS as u32,
    };
    let result = flatten_contract_result(client.try_update_params(&t.asset, &model));
    assert_contract_error(
        result,
        common::errors::CollateralError::InvalidReserveFactor as u32,
    );
}

// base_borrow_rate < 0 maps to BaseRateNegative (#128) at create_market.
#[test]
fn test_create_market_rejects_invalid_rate_model() {
    let env = Env::default();
    env.mock_all_auths();
    test_support::init_ledger(&env);

    let admin = Address::generate(&env);
    let pool = env.register(LiquidityPool, (admin.clone(),));
    let client = LiquidityPoolClient::new(&env, &pool);

    let mut params = market_params(&Address::generate(&env));
    params.base_borrow_rate_ray = -1;

    let result = flatten_contract_result(client.try_create_market(&params));
    assert_contract_error(
        result,
        common::errors::CollateralError::BaseRateNegative as u32,
    );
}

// Registering the same asset twice reverts with AssetAlreadySupported (#2).
#[test]
fn test_create_market_rejects_duplicate_asset() {
    let t = TestSetup::new();
    let client = t.client();

    let result = flatten_contract_result(client.try_create_market(&market_params(&t.asset)));
    assert_contract_error(
        result,
        common::errors::GenericError::AssetAlreadySupported as u32,
    );
}

// Unknown market operations revert with PoolNotInitialized (#30).
#[test]
fn test_supply_rejects_unknown_market() {
    let t = TestSetup::new();
    let client = t.client();

    let unknown_asset = Address::generate(&t.env);
    let result = flatten_contract_result(client.try_supply(&t.sup_for(
        &unknown_asset,
        0,
        1_0000000i128,
        i128::MAX,
    )));
    assert_contract_error(
        result,
        common::errors::GenericError::PoolNotInitialized as u32,
    );
}

// create_market seeds RAY indexes, zero totals/cash, and last_timestamp from
// ledger milliseconds.
#[test]
fn test_create_market_initializes_state() {
    let t = TestSetup::new();
    let client = t.client();

    let asset_b = Address::generate(&t.env);
    client.create_market(&market_params(&asset_b));

    let sync = client.get_sync_data(&asset_b);
    if sync.state.supply_index_ray != RAY {
        panic!("supply index must start at RAY");
    }
    if sync.state.borrow_index_ray != RAY {
        panic!("borrow index must start at RAY");
    }
    if sync.state.last_timestamp != t.env.ledger().timestamp() * MS_PER_SECOND {
        panic!("last_timestamp must be ledger time in milliseconds");
    }
    assert_eq!(sync.state.supplied_ray, 0);
    assert_eq!(sync.state.borrowed_ray, 0);
    assert_eq!(sync.state.revenue_ray, 0);
    assert_eq!(sync.state.cash, 0);
    assert_eq!(sync.params.asset_id, asset_b);
}

// Two markets in one pool instance stay isolated; market A mutations leave
// market B state unchanged.
#[test]
fn test_two_market_isolation() {
    let t = TestSetup::new();
    let client = t.client();

    let asset_b = Address::generate(&t.env);
    client.create_market(&market_params(&asset_b));
    let b_initial = t.state_of(&asset_b);

    let a_before = t.state_snapshot();
    let supply_amount = 10_000_000_000i128;
    client.supply(&t.sup(0, supply_amount, i128::MAX));

    let a_after_supply = t.state_snapshot();
    if a_after_supply.supplied_ray <= a_before.supplied_ray {
        panic!("market A supplied must increase after supply");
    }
    assert_eq!(a_after_supply.cash, a_before.cash + supply_amount);

    let b_after_supply = t.state_of(&asset_b);
    assert_pool_state_eq(&b_after_supply, &b_initial);
    assert_eq!(b_after_supply.cash, b_initial.cash);

    let borrower = Address::generate(&t.env);
    let borrow_amount = 100_0000000i128;
    client.borrow(&borrower, &t.bor(0, borrow_amount, i128::MAX));

    let a_after_borrow = t.state_snapshot();
    if a_after_borrow.borrowed_ray <= a_after_supply.borrowed_ray {
        panic!("market A borrowed must increase after borrow");
    }
    assert_eq!(a_after_borrow.cash, a_after_supply.cash - borrow_amount);

    let b_after_borrow = t.state_of(&asset_b);
    assert_pool_state_eq(&b_after_borrow, &b_initial);
    assert_eq!(b_after_borrow.cash, b_initial.cash);
}

// create_market is #[only_owner]; calls without auth fail.
#[test]
fn test_create_market_rejects_non_owner() {
    let t = TestSetup::new();
    let client = t.client();
    let asset_b = Address::generate(&t.env);
    let no_auths: [soroban_sdk::xdr::SorobanAuthorizationEntry; 0] = [];

    let result = client
        .set_auths(&no_auths)
        .try_create_market(&market_params(&asset_b));

    assert!(
        result.is_err(),
        "create_market without owner auth must fail"
    );
}

// bulk_get_sync_data returns per-asset simulated indexes in request order after
// time-based accrual.
#[test]
fn test_bulk_get_sync_data_matches_per_asset_simulation() {
    let t = TestSetup::new();
    let client = t.client();

    // Create a borrow so indexes accrue over time.
    client.supply(&t.sup(0, 10_000_000_000i128, i128::MAX));
    let borrower = Address::generate(&t.env);
    client.borrow(&borrower, &t.bor(0, 5_000_000_000i128, i128::MAX));

    t.advance_time(86_400);

    let now_ms = t.env.ledger().timestamp() * common::constants::MS_PER_SECOND;
    let sync = client.get_sync_data(&t.asset);
    let expected =
        common::types::MarketIndexRaw::from(&simulate_update_indexes(&t.env, now_ms, &sync));

    let assets = soroban_sdk::vec![&t.env, t.asset.clone()];
    let bulk = client.bulk_get_sync_data(&assets);

    assert_eq!(bulk.len(), 1, "one entry per requested asset");
    assert_eq!(bulk.get_unchecked(0), expected);
    assert!(
        expected.borrow_index_ray > RAY,
        "borrow index must have accrued past RAY for the equality to be meaningful"
    );
}

// Multi-asset results are input-aligned; utilized and idle markets keep
// separate indexes.
#[test]
fn test_bulk_get_sync_data_multi_asset_alignment() {
    let t = TestSetup::new();
    let client = t.client();

    let asset_b = Address::generate(&t.env);
    client.create_market(&market_params(&asset_b));

    // Only market A gets utilization, so only its indexes accrue.
    client.supply(&t.sup(0, 10_000_000_000i128, i128::MAX));
    let borrower = Address::generate(&t.env);
    client.borrow(&borrower, &t.bor(0, 5_000_000_000i128, i128::MAX));

    t.advance_time(86_400);

    let now_ms = t.env.ledger().timestamp() * common::constants::MS_PER_SECOND;
    let expected_a = common::types::MarketIndexRaw::from(&simulate_update_indexes(
        &t.env,
        now_ms,
        &client.get_sync_data(&t.asset),
    ));
    let expected_b = common::types::MarketIndexRaw::from(&simulate_update_indexes(
        &t.env,
        now_ms,
        &client.get_sync_data(&asset_b),
    ));

    let assets = soroban_sdk::vec![&t.env, t.asset.clone(), asset_b.clone()];
    let bulk = client.bulk_get_sync_data(&assets);
    assert_eq!(bulk.len(), 2);

    let a = bulk.get_unchecked(0);
    let b = bulk.get_unchecked(1);
    assert_eq!(a, expected_a, "entry 0 must match market A's simulation");
    assert_eq!(b, expected_b, "entry 1 must match market B's simulation");
    // Utilized market accrues borrow/supply indexes; idle market accrues base
    // borrow index only.
    assert!(a.borrow_index_ray > b.borrow_index_ray && b.borrow_index_ray > RAY);
    assert!(a.supply_index_ray > RAY);
    assert_eq!(b.supply_index_ray, RAY, "no borrows, no supplier rewards");
}

// An empty request returns an empty result without panicking.
#[test]
fn test_bulk_get_sync_data_empty_request() {
    let t = TestSetup::new();
    let bulk = t
        .client()
        .bulk_get_sync_data(&soroban_sdk::Vec::new(&t.env));
    assert_eq!(bulk.len(), 0);
}

// Unknown assets fail bulk read with PoolNotInitialized, matching get_sync_data.
#[test]
fn test_bulk_get_sync_data_unknown_asset_panics() {
    let t = TestSetup::new();
    let unknown = Address::generate(&t.env);
    let assets = soroban_sdk::vec![&t.env, unknown];
    let result = t.client().try_bulk_get_sync_data(&assets);
    assert!(result.is_err(), "unknown asset must fail the bulk read");
}

// Bulk supply across two markets returns input-ordered mutations and matches
// sequential single-entry calls.
#[test]
fn test_bulk_supply_two_markets_matches_sequential_singles() {
    let t = TestSetup::new();
    let client = t.client();

    // Bulk targets: default market A and funded market B. Sequential
    // references: funded markets C and D.
    let asset_b = t.add_funded_market();
    let asset_c = t.add_funded_market();
    let asset_d = t.add_funded_market();

    let amount_one = 10_000_000_000i128;
    let amount_two = 25_000_000_000i128;

    let entries = vec![
        &t.env,
        t.sup_entry(&t.asset, 0, amount_one, i128::MAX),
        t.sup_entry(&asset_b, 0, amount_two, i128::MAX),
    ];
    let bulk = client.supply(&entries);
    assert_eq!(bulk.len(), 2, "one mutation per entry");

    let first = bulk.get_unchecked(0);
    let second = bulk.get_unchecked(1);
    assert_eq!(
        first.market_state.asset, t.asset,
        "entry 0 result must be market A"
    );
    assert_eq!(
        second.market_state.asset, asset_b,
        "entry 1 result must be market B"
    );

    let seq_first = client
        .supply(&t.sup_for(&asset_c, 0, amount_one, i128::MAX))
        .get_unchecked(0);
    let seq_second = client
        .supply(&t.sup_for(&asset_d, 0, amount_two, i128::MAX))
        .get_unchecked(0);

    assert_eq!(
        first.position.scaled_amount_ray,
        seq_first.position.scaled_amount_ray
    );
    assert_eq!(first.actual_amount, seq_first.actual_amount);
    assert_eq!(
        second.position.scaled_amount_ray,
        seq_second.position.scaled_amount_ray
    );
    assert_eq!(second.actual_amount, seq_second.actual_amount);

    // A/C and B/D start from matching state; bulk and sequential singles end
    // with matching cash/state.
    let a_state = t.state_snapshot();
    let c_state = t.state_of(&asset_c);
    assert_pool_state_eq(&a_state, &c_state);
    assert_eq!(a_state.cash, c_state.cash);

    let b_state = t.state_of(&asset_b);
    let d_state = t.state_of(&asset_d);
    assert_pool_state_eq(&b_state, &d_state);
    assert_eq!(b_state.cash, d_state.cash);
}

// Bulk repay keeps input order and refunds second-entry overpayment to payer.
#[test]
fn test_bulk_repay_overpayment_refunds_second_entry_surplus() {
    let t = TestSetup::new();
    let client = t.client();

    client.supply(&t.sup(0, 50_000_000_000i128, i128::MAX));

    // Two independent debt positions for the same borrower.
    let borrower = Address::generate(&t.env);
    let debt_one = 100_0000000i128;
    let debt_two = 30_0000000i128;
    let first_borrow = client
        .borrow(&borrower, &t.bor(0, debt_one, i128::MAX))
        .get_unchecked(0);
    let second_borrow = client
        .borrow(&borrower, &t.bor(0, debt_two, i128::MAX))
        .get_unchecked(0);

    let overpayment = 5_0000000i128;
    let tok = token::Client::new(&t.env, &t.asset);
    let payer_before = tok.balance(&borrower);

    let actions = vec![
        &t.env,
        t.action(first_borrow.position.scaled_amount_ray, debt_one),
        t.action(
            second_borrow.position.scaled_amount_ray,
            debt_two + overpayment,
        ),
    ];
    let results = client.repay(&borrower, &actions);
    assert_eq!(results.len(), 2, "one mutation per entry");

    let first = results.get_unchecked(0);
    let second = results.get_unchecked(1);
    assert_eq!(
        first.actual_amount, debt_one,
        "entry 0 repays exactly, no refund"
    );
    assert_eq!(first.position.scaled_amount_ray, 0);
    assert_eq!(
        second.actual_amount, debt_two,
        "entry 1 applies only the outstanding debt"
    );
    assert_eq!(second.position.scaled_amount_ray, 0);
    assert_eq!(
        tok.balance(&borrower) - payer_before,
        overpayment,
        "payer receives the second entry's surplus back"
    );
}

// Duplicate-asset bulk supply applies entries in order; entry 2 prices against
// post-entry-1 state.
#[test]
fn test_bulk_supply_duplicate_asset_applies_sequentially() {
    let t = TestSetup::new();
    let client = t.client();

    let amount_one = 10_000_000_000i128;
    let amount_two = 25_000_000_000i128;
    let state_before = t.state_snapshot();

    let entries = vec![
        &t.env,
        t.sup_entry(&t.asset, 0, amount_one, i128::MAX),
        t.sup_entry(&t.asset, 0, amount_two, i128::MAX),
    ];
    let results = client.supply(&entries);
    assert_eq!(results.len(), 2);

    let first = results.get_unchecked(0);
    let second = results.get_unchecked(1);
    assert_eq!(
        first.market_state.supplied_ray,
        state_before.supplied_ray + first.position.scaled_amount_ray,
        "entry 1 snapshot reflects only its own supply"
    );
    assert_eq!(
        second.market_state.supplied_ray,
        first.market_state.supplied_ray + second.position.scaled_amount_ray,
        "entry 2 snapshot must build on entry 1's already-applied state"
    );

    let state_after = t.state_snapshot();
    assert_eq!(
        state_after.supplied_ray,
        state_before.supplied_ray
            + first.position.scaled_amount_ray
            + second.position.scaled_amount_ray,
        "total supplied is the sum of both entries"
    );
    assert_eq!(
        state_after.cash,
        state_before.cash + amount_one + amount_two
    );
}

// Batch supply is atomic: entry 2 supply-cap failure rolls back entry 1 state.
#[test]
fn test_bulk_supply_cap_violation_reverts_whole_batch() {
    let t = TestSetup::new();
    let client = t.client();

    let asset_b = t.add_funded_market();
    let a_before = t.state_snapshot();
    let b_before = t.state_of(&asset_b);

    let amount = 10_000_000_000i128;
    let entries = vec![
        &t.env,
        t.sup_entry(&t.asset, 0, amount, i128::MAX),
        t.sup_entry(&asset_b, 0, amount, amount - 1),
    ];
    let result = flatten_contract_result(client.try_supply(&entries));
    assert_contract_error(
        result,
        common::errors::CollateralError::SupplyCapReached as u32,
    );

    // Entry 1 passed its cap check; transaction rollback restores both markets.
    let a_after = t.state_snapshot();
    assert_pool_state_eq(&a_after, &a_before);
    assert_eq!(a_after.cash, a_before.cash);
    let b_after = t.state_of(&asset_b);
    assert_pool_state_eq(&b_after, &b_before);
    assert_eq!(b_after.cash, b_before.cash);
}
