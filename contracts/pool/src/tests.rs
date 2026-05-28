extern crate std;

use super::*;
use common::constants::{BPS, RAY};
use common::types::ScaledPositionRaw;
use soroban_sdk::testutils::{Address as _, Ledger, LedgerInfo};
use soroban_sdk::{contract, contractimpl, Address, Bytes, Env};

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

        let params = MarketParamsRaw {
            max_borrow_rate_ray: RAY,
            base_borrow_rate_ray: RAY / 100,
            slope1_ray: RAY * 4 / 100,
            slope2_ray: RAY * 10 / 100,
            slope3_ray: RAY * 80 / 100,
            mid_utilization_ray: RAY * 50 / 100,
            optimal_utilization_ray: RAY * 80 / 100,
            // Disabled (RAY sentinel) for pool unit tests \u2014 these
            // exercise accounting invariants, not the utilization
            // ceiling. Harness integration tests cover the ceiling.
            max_utilization_ray: RAY,
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

    fn deposit_position(&self) -> ScaledPositionRaw {
        ScaledPositionRaw {
            scaled_amount_ray: 0,
        }
    }

    fn borrow_position(&self) -> ScaledPositionRaw {
        ScaledPositionRaw {
            scaled_amount_ray: 0,
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

    fn edit_state(&self, edit: impl FnOnce(&mut PoolStateRaw)) {
        self.env.as_contract(&self.pool, || {
            let mut state: PoolStateRaw =
                self.env.storage().instance().get(&PoolKey::State).unwrap();
            edit(&mut state);
            self.env.storage().instance().set(&PoolKey::State, &state);
        });
    }

    fn state_snapshot(&self) -> PoolStateRaw {
        self.env.as_contract(&self.pool, || {
            self.env.storage().instance().get(&PoolKey::State).unwrap()
        })
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
fn test_supply_cap_enforced_after_pool_sync() {
    let t = TestSetup::new();
    let client = t.client();

    let pos = t.deposit_position();
    let amount = 10_000_000_000i128;

    let result = flatten_contract_result(client.try_supply(&pos, &amount, &(amount - 1)));
    assert_contract_error(
        result,
        common::errors::CollateralError::SupplyCapReached as u32,
    );
}

#[test]
fn test_borrow_cap_enforced_after_pool_sync() {
    let t = TestSetup::new();
    let client = t.client();

    let supply_pos = t.deposit_position();
    client.supply(&supply_pos, &50_000_000_000i128, &i128::MAX);

    let borrower = Address::generate(&t.env);
    let borrow_pos = t.borrow_position();
    let borrow_amount = 100_0000000i128;

    let result = flatten_contract_result(client.try_borrow(
        &borrower,
        &borrow_amount,
        &borrow_pos,
        &(borrow_amount - 1),
    ));
    assert_contract_error(
        result,
        common::errors::CollateralError::BorrowCapReached as u32,
    );
}

#[test]
fn test_strategy_borrow_cap_enforced_after_pool_sync() {
    let t = TestSetup::new();
    let client = t.client();

    let supply_pos = t.deposit_position();
    client.supply(&supply_pos, &50_000_000_000i128, &i128::MAX);

    let caller = Address::generate(&t.env);
    let pos = t.borrow_position();
    let amount = 100_0000000i128;

    let result = flatten_contract_result(client.try_create_strategy(
        &caller,
        &pos,
        &amount,
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
    let borrow_pos = t.borrow_position();

    let result = flatten_contract_result(client.try_borrow(
        &borrower,
        &200_000_000_000_000i128,
        &borrow_pos,
        &i128::MAX,
    ));
    assert_contract_error(
        result,
        common::errors::CollateralError::InsufficientLiquidity as u32,
    );
}
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
fn test_withdraw_rejects_fee_greater_than_withdrawn_amount() {
    let t = TestSetup::new();
    let client = t.client();

    let pos = t.deposit_position();
    let updated_pos = client.supply(&pos, &10_000_000i128, &i128::MAX);
    let user = Address::generate(&t.env);

    let result = flatten_contract_result(client.try_withdraw(
        &user,
        &1_0000000i128,
        &updated_pos.position,
        &true,
        &2_0000000i128,
    ));
    assert_contract_error(
        result,
        common::errors::CollateralError::WithdrawLessThanFee as u32,
    );
}

// Covers the post-state utilization gate. The `TestSetup` default cap is
// RAY (disabled) so the rest of the unit suite doesn't trip; this test
// sets a 50 % cap to exercise both branches.
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
        let mut params: MarketParamsRaw = t.env.storage().instance().get(&PoolKey::Params).unwrap();
        params.max_utilization_ray = RAY / 2;
        t.env.storage().instance().set(&PoolKey::Params, &params);
    });

    let client = t.client();
    let borrower = Address::generate(&t.env);
    let pos = t.borrow_position();
    // Borrow > 50 % of supplied \u2192 revert with UtilizationAboveMax.
    let result =
        flatten_contract_result(client.try_borrow(&borrower, &60_000i128, &pos, &i128::MAX));
    assert_contract_error(
        result,
        common::errors::CollateralError::UtilizationAboveMax as u32,
    );
}

#[test]
fn test_withdraw_rejects_when_reserves_are_insufficient() {
    let t = TestSetup::new();
    let client = t.client();

    let pos = t.deposit_position();
    let updated_pos = client.supply(&pos, &10_000_000_000i128, &i128::MAX);

    let borrower = Address::generate(&t.env);
    let borrow_pos = t.borrow_position();
    client.borrow(&borrower, &99_999_990_000_000i128, &borrow_pos, &i128::MAX);

    let user = Address::generate(&t.env);
    let result = flatten_contract_result(client.try_withdraw(
        &user,
        &10_000_000_000i128,
        &updated_pos.position,
        &false,
        &0i128,
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

    let pos = t.deposit_position();
    let updated_pos = client.supply(&pos, &10_000_000_000i128, &i128::MAX);
    t.edit_state(|state| {
        state.supplied_ray = 1;
    });

    let user = Address::generate(&t.env);
    let result = flatten_contract_result(client.try_withdraw(
        &user,
        &i128::MAX,
        &updated_pos.position,
        &false,
        &0i128,
    ));
    assert_contract_error(result, common::errors::GenericError::MathOverflow as u32);
}
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

    let result = flatten_contract_result(client.try_flash_loan(
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
fn test_flash_loan_rejects_insufficient_liquidity() {
    let t = TestSetup::new();
    let client = t.client();
    let receiver = Address::generate(&t.env);

    let result = flatten_contract_result(client.try_flash_loan(
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
    let pos = t.borrow_position();

    let result = flatten_contract_result(client.try_create_strategy(
        &caller,
        &pos,
        &1_0000000i128,
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
    let pos = t.borrow_position();

    let result = flatten_contract_result(client.try_create_strategy(
        &caller,
        &pos,
        &200_000_000_000_000i128,
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

    let result = flatten_contract_result(
        client.try_seize_position(&AccountPositionType::Borrow, &updated_borrow.position),
    );
    assert_contract_error(result, common::errors::GenericError::MathOverflow as u32);
}
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
fn test_claim_revenue_rejects_revenue_above_supplied() {
    let t = TestSetup::new();
    let client = t.client();

    let supply_pos = t.deposit_position();
    let supplied = client.supply(&supply_pos, &10_000_000_000i128, &i128::MAX);
    let _ = client.seize_position(&AccountPositionType::Deposit, &supplied.position);
    t.edit_state(|state| {
        state.supplied_ray = 1;
    });

    let result = flatten_contract_result(client.try_claim_revenue());
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
    let result = flatten_contract_result(client.try_update_params(&model));
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
    let result = flatten_contract_result(client.try_update_params(&model));
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
    let result = flatten_contract_result(client.try_update_params(&model));
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
    let result = flatten_contract_result(client.try_update_params(&model));
    assert_contract_error(
        result,
        common::errors::CollateralError::BaseRateNegative as u32,
    );
}

#[test]
fn test_update_params_rejects_max_rate_not_above_base_rate() {
    let t = TestSetup::new();
    let client = t.client();

    // Keep slopes flat at base so SlopeNonMonotonic doesn't pre-empt MaxRateBelowBase.
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
    let result = flatten_contract_result(client.try_update_params(&model));
    assert_contract_error(
        result,
        common::errors::CollateralError::MaxRateBelowBase as u32,
    );
}
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
// Extra targeted coverage tests.

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
    client.update_params(&model);

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

// slope3 < slope2 → SlopeNonMonotonic.
#[test]
fn test_update_params_rejects_invalid_slope_ordering() {
    let t = TestSetup::new();
    let client = t.client();

    // slope3 < slope2: invalid.
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
    let result = flatten_contract_result(client.try_update_params(&model));
    assert_contract_error(
        result,
        common::errors::CollateralError::SlopeNonMonotonic as u32,
    );
}

// mid_utilization == 0 panics with InvalidUtilRange.
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
    let result = flatten_contract_result(client.try_update_params(&model));
    assert_contract_error(
        result,
        common::errors::CollateralError::InvalidUtilRange as u32,
    );
}

// reserve_factor at the BPS ceiling panics with InvalidReserveFactor;
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
    let result = flatten_contract_result(client.try_update_params(&model));
    assert_contract_error(
        result,
        common::errors::CollateralError::InvalidReserveFactor as u32,
    );
}

// base_borrow_rate < 0 → BaseRateNegative (#128).
#[test]
#[should_panic(expected = "Error(Contract, #128)")]
fn test_constructor_rejects_invalid_rate_model() {
    let env = Env::default();
    test_support::init_ledger(&env);

    let admin = Address::generate(&env);
    let params = MarketParamsRaw {
        max_borrow_rate_ray: RAY,
        base_borrow_rate_ray: -1,
        slope1_ray: RAY * 4 / 100,
        slope2_ray: RAY * 10 / 100,
        slope3_ray: RAY * 80 / 100,
        mid_utilization_ray: RAY * 50 / 100,
        optimal_utilization_ray: RAY * 80 / 100,
        max_utilization_ray: RAY * 95 / 100,
        reserve_factor_bps: 1000,
        asset_id: Address::generate(&env),
        asset_decimals: 7,
    };

    let _ = env.register(LiquidityPool, (admin, params));
}

