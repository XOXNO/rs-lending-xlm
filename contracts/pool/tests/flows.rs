extern crate std;

use super::*;
use common::constants::{BPS, MS_PER_SECOND, RAY};
use common::types::{HubAssetKey, ScaledPositionRaw};

/// Pool tests use hub 0 as a local fixture id.
fn hub(asset: &Address) -> HubAssetKey {
    HubAssetKey {
        hub_id: 0,
        asset: asset.clone(),
    }
}
use soroban_sdk::testutils::{Address as _, ContractEvents, Events, Ledger, LedgerInfo};
use soroban_sdk::xdr::{ContractEventBody, ScVal};
use soroban_sdk::{contract, contractimpl, vec, Address, Bytes, Env};

fn count_topic(events: &ContractEvents, first: &str, second: &str) -> usize {
    events
        .events()
        .iter()
        .filter(|event| {
            let ContractEventBody::V0(body) = &event.body;
            match (body.topics.first(), body.topics.get(1)) {
                (Some(ScVal::Symbol(a)), Some(ScVal::Symbol(b))) => {
                    a.0.to_utf8_string().as_deref() == Ok(first)
                        && b.0.to_utf8_string().as_deref() == Ok(second)
                }
                _ => false,
            }
        })
        .count()
}

/// Reads `hub_id` from the data map of the first `strategy/fee` event, if any.
fn strategy_fee_hub_id(events: &ContractEvents) -> Option<u32> {
    events.events().iter().find_map(|event| {
        let ContractEventBody::V0(body) = &event.body;
        let is_strategy_fee = matches!(
            (body.topics.first(), body.topics.get(1)),
            (Some(ScVal::Symbol(a)), Some(ScVal::Symbol(b)))
                if a.0.to_utf8_string().as_deref() == Ok("strategy")
                    && b.0.to_utf8_string().as_deref() == Ok("fee")
        );
        if !is_strategy_fee {
            return None;
        }
        match &body.data {
            ScVal::Map(Some(m)) => m.iter().find_map(|entry| match (&entry.key, &entry.val) {
                (ScVal::Symbol(s), ScVal::U32(v))
                    if s.0.to_utf8_string().as_deref() == Ok("hub_id") =>
                {
                    Some(*v)
                }
                _ => None,
            }),
            _ => None,
        }
    })
}

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
        max_borrow_rate: RAY,
        base_borrow_rate: RAY / 100,
        slope1: RAY * 4 / 100,
        slope2: RAY * 10 / 100,
        slope3: RAY * 80 / 100,
        mid_utilization: RAY * 50 / 100,
        optimal_utilization: RAY * 80 / 100,
        // Disable max-utilization checks in accounting tests.
        max_utilization: RAY,
        reserve_factor: 1000,
        is_flashloanable: false,
        flashloan_fee: 0,
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

        // Owner receives claimed protocol revenue.
        let pool_address = env.register(LiquidityPool, (admin.clone(),));
        LiquidityPoolClient::new(&env, &pool_address)
            .create_market(&0u32, &market_params(&asset_address));

        // Mint tokens to the pool for reserves.
        let token_admin = token::StellarAssetClient::new(&env, &asset_address);
        token_admin.mint(&pool_address, &100_000_000_000_000i128);

        // Seed `cash` to the minted reserve balance; pool liquidity uses `cash`.
        env.as_contract(&pool_address, || {
            let key = PoolKey::State(hub(&asset_address));
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

    fn action(&self, scaled_amount: i128, amount: i128) -> PoolAction {
        self.action_for(&self.asset, scaled_amount, amount)
    }

    fn action_for(&self, asset: &Address, scaled_amount: i128, amount: i128) -> PoolAction {
        PoolAction {
            position: ScaledPositionRaw { scaled_amount },
            amount,
            hub_asset: hub(asset),
        }
    }

    fn sup_entry(&self, asset: &Address, scaled_amount: i128, amount: i128) -> PoolSupplyEntry {
        PoolSupplyEntry {
            action: self.action_for(asset, scaled_amount, amount),
        }
    }

    /// Singleton supply batch against the default market.
    fn sup(&self, scaled_amount: i128, amount: i128) -> Vec<PoolSupplyEntry> {
        self.sup_for(&self.asset, scaled_amount, amount)
    }

    fn sup_for(&self, asset: &Address, scaled_amount: i128, amount: i128) -> Vec<PoolSupplyEntry> {
        vec![&self.env, self.sup_entry(asset, scaled_amount, amount)]
    }

    /// Singleton borrow batch against the default market.
    fn bor(&self, scaled_amount: i128, amount: i128) -> Vec<PoolBorrowEntry> {
        vec![
            &self.env,
            PoolBorrowEntry {
                action: self.action(scaled_amount, amount),
            },
        ]
    }

    /// Singleton withdraw batch against the default market.
    fn wdr(&self, scaled_amount: i128, amount: i128, protocol_fee: i128) -> Vec<PoolWithdrawEntry> {
        vec![
            &self.env,
            PoolWithdrawEntry {
                action: self.action(scaled_amount, amount),
                protocol_fee,
            },
        ]
    }

    /// Singleton repay batch against the default market.
    fn ract(&self, scaled_amount: i128, amount: i128) -> Vec<PoolAction> {
        vec![&self.env, self.action(scaled_amount, amount)]
    }

    fn sez_entry(&self, side: AccountPositionType, position: &ScaledPositionRaw) -> PoolSeizeEntry {
        PoolSeizeEntry {
            hub_asset: hub(&self.asset),
            side,
            position: position.clone(),
        }
    }

    /// Singleton seize batch against the default market.
    fn sez(&self, side: AccountPositionType, position: &ScaledPositionRaw) -> Vec<PoolSeizeEntry> {
        vec![&self.env, self.sez_entry(side, position)]
    }

    /// Registers a funded market with a fresh SAC token, minted reserves, and
    /// seeded internal `cash`.
    fn add_funded_market(&self) -> Address {
        let asset = self
            .env
            .register_stellar_asset_contract_v2(self.admin.clone())
            .address()
            .clone();
        self.client().create_market(&0u32, &market_params(&asset));
        token::StellarAssetClient::new(&self.env, &asset)
            .mint(&self.pool, &100_000_000_000_000i128);
        self.env.as_contract(&self.pool, || {
            let key = PoolKey::State(hub(&asset));
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
            let key = PoolKey::State(hub(&self.asset));
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
                .get(&PoolKey::State(hub(asset)))
                .unwrap()
        })
    }

    fn state_snapshot(&self) -> PoolStateRaw {
        self.state_of(&self.asset)
    }
}

fn assert_pool_state_eq(left: &PoolStateRaw, right: &PoolStateRaw) {
    assert_eq!(left.supplied, right.supplied);
    assert_eq!(left.borrowed, right.borrowed);
    assert_eq!(left.revenue, right.revenue);
    assert_eq!(left.borrow_index, right.borrow_index);
    assert_eq!(left.supply_index, right.supply_index);
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

    let updated = client.supply(&t.sup(0, amount)).get_unchecked(0);

    assert!(
        updated.position.scaled_amount > 0,
        "position should have scaled amount"
    );

    let supplied = client.get_supplied_amount(&hub(&t.asset));
    assert!(supplied > 0, "supplied_amount should be positive");
}
#[test]
fn test_borrow() {
    let t = TestSetup::new();
    let client = t.client();

    client.supply(&t.sup(0, 50_000_000_000i128));

    let borrower = Address::generate(&t.env);
    let borrow_amount = 100_0000000i128;

    let reserves_before = client.get_reserves(&hub(&t.asset));
    let updated = client
        .borrow(&borrower, &t.bor(0, borrow_amount))
        .get_unchecked(0);

    assert!(
        updated.position.scaled_amount > 0,
        "borrow position should have debt"
    );

    let reserves_after = client.get_reserves(&hub(&t.asset));
    assert!(
        reserves_after < reserves_before,
        "reserves should decrease after borrow"
    );
}

#[test]
fn test_borrow_rejects_zero_amount() {
    let t = TestSetup::new();
    let client = t.client();

    client.supply(&t.sup(0, 50_000_000_000i128));

    let borrower = Address::generate(&t.env);
    let result = flatten_contract_result(client.try_borrow(&borrower, &t.bor(0, 0i128)));
    assert_contract_error(
        result,
        common::errors::GenericError::AmountMustBePositive as u32,
    );
}

#[test]
fn test_borrow_rejects_when_reserves_are_insufficient() {
    let t = TestSetup::new();
    let client = t.client();
    let borrower = Address::generate(&t.env);

    let result =
        flatten_contract_result(client.try_borrow(&borrower, &t.bor(0, 200_000_000_000_000i128)));
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
    let updated_pos = client.supply(&t.sup(0, supply_amount)).get_unchecked(0);

    let user = Address::generate(&t.env);
    let tok = token::Client::new(&t.env, &t.asset);
    let user_balance_before = tok.balance(&user);

    let withdraw_amount = 500_0000000i128;
    let final_pos = client
        .withdraw(
            &user,
            &false,
            &t.wdr(updated_pos.position.scaled_amount, withdraw_amount, 0i128),
        )
        .get_unchecked(0);

    let user_balance_after = tok.balance(&user);
    assert!(
        user_balance_after > user_balance_before,
        "user should receive tokens"
    );
    assert!(
        final_pos.position.scaled_amount < updated_pos.position.scaled_amount,
        "scaled amount should decrease"
    );
}

#[test]
fn test_withdraw_rejects_fee_greater_than_withdrawn_amount() {
    let t = TestSetup::new();
    let client = t.client();

    let updated_pos = client.supply(&t.sup(0, 10_000_000i128)).get_unchecked(0);
    let user = Address::generate(&t.env);

    let result = flatten_contract_result(client.try_withdraw(
        &user,
        &true,
        &t.wdr(
            updated_pos.position.scaled_amount,
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
        s.supplied = 100_000_000_000_000;
        s.borrowed = 0;
    });
    // Tighten the cap to 50 %.
    t.env.as_contract(&t.pool, || {
        let key = PoolKey::Params(hub(&t.asset));
        let mut params: MarketParamsRaw = t.env.storage().persistent().get(&key).unwrap();
        params.max_utilization = RAY / 2;
        t.env.storage().persistent().set(&key, &params);
    });

    let client = t.client();
    let borrower = Address::generate(&t.env);
    // Borrow above 50% of supplied reverts with UtilizationAboveMax.
    let result = flatten_contract_result(client.try_borrow(&borrower, &t.bor(0, 60_000i128)));
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
        .supply(&t.sup(0, 10_000_000_000i128))
        .get_unchecked(0);

    let borrower = Address::generate(&t.env);
    client.borrow(&borrower, &t.bor(0, 99_999_990_000_000i128));

    // Reserves are tracked as `cash`; drain it below the withdrawal amount so
    // the insufficient-liquidity guard fires.
    t.edit_state(|s| s.cash = 5_000_000_000i128);

    let user = Address::generate(&t.env);
    let result = flatten_contract_result(client.try_withdraw(
        &user,
        &false,
        &t.wdr(
            updated_pos.position.scaled_amount,
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
        .supply(&t.sup(0, 10_000_000_000i128))
        .get_unchecked(0);
    t.edit_state(|state| {
        state.supplied = 1;
    });

    let user = Address::generate(&t.env);
    let result = flatten_contract_result(client.try_withdraw(
        &user,
        &false,
        &t.wdr(updated_pos.position.scaled_amount, i128::MAX, 0i128),
    ));
    assert_contract_error(result, common::errors::GenericError::MathOverflow as u32);
}
#[test]
fn test_repay() {
    let t = TestSetup::new();
    let client = t.client();

    client.supply(&t.sup(0, 50_000_000_000i128));

    let borrower = Address::generate(&t.env);
    let updated_borrow = client
        .borrow(&borrower, &t.bor(0, 100_0000000i128))
        .get_unchecked(0);

    assert!(updated_borrow.position.scaled_amount > 0);

    // Exact repay; no overpayment because no time has passed.
    let repay_amount = 100_0000000i128;
    let final_pos = client
        .repay(
            &borrower,
            &t.ract(updated_borrow.position.scaled_amount, repay_amount),
        )
        .get_unchecked(0);

    assert_eq!(final_pos.actual_amount, repay_amount);
    assert!(
        final_pos.position.scaled_amount == 0 || final_pos.position.scaled_amount <= 1,
        "position should be cleared after full repay"
    );
}

#[test]
fn test_repay_overpayment_reports_actual_applied_amount() {
    let t = TestSetup::new();
    let client = t.client();

    client.supply(&t.sup(0, 50_000_000_000i128));

    let borrower = Address::generate(&t.env);
    let updated_borrow = client
        .borrow(&borrower, &t.bor(0, 100_0000000i128))
        .get_unchecked(0);

    let repay_amount = 200_0000000i128;
    let final_pos = client
        .repay(
            &borrower,
            &t.ract(updated_borrow.position.scaled_amount, repay_amount),
        )
        .get_unchecked(0);

    assert_eq!(final_pos.actual_amount, 100_0000000i128);
    assert_eq!(final_pos.position.scaled_amount, 0);
}
#[test]
fn test_interest_accrual() {
    let t = TestSetup::new();
    let client = t.client();

    client.supply(&t.sup(0, 50_000_000_000i128));

    let borrower = Address::generate(&t.env);
    client.borrow(&borrower, &t.bor(0, 10_000_000_000i128));

    client.update_indexes(&hub(&t.asset));
    let initial_indexes = client.get_sync_data(&hub(&t.asset)).state;

    // Advance time by ~1 year.
    t.advance_time(31_556_926);

    client.update_indexes(&hub(&t.asset));
    let new_indexes = client.get_sync_data(&hub(&t.asset)).state;

    assert!(
        new_indexes.borrow_index > initial_indexes.borrow_index,
        "borrow index should increase over time"
    );
    assert!(
        new_indexes.supply_index > initial_indexes.supply_index,
        "supply index should increase over time"
    );
}
#[test]
fn test_flash_loan() {
    let t = TestSetup::new();
    let client = t.client();

    client.supply(&t.sup(0, 10_000_000_000i128));

    let receiver = t.env.register(PoolFlashLoanReceiver, ());
    let flash_amount = 100_0000000i128;
    let flash_fee = 1_0000000i128;

    // The pool will send `amount`; pre-fund only the fee.
    let token_admin_client = token::StellarAssetClient::new(&t.env, &t.asset);
    token_admin_client.mint(&receiver, &flash_fee);

    let tok = token::Client::new(&t.env, &t.asset);
    let pool_balance_before = tok.balance(&t.pool);
    let revenue_before = client.get_revenue(&hub(&t.asset));
    client.flash_loan(
        &hub(&t.asset),
        &t.admin,
        &receiver,
        &flash_amount,
        &flash_fee,
        &Bytes::new(&t.env),
    );
    let revenue_after = client.get_revenue(&hub(&t.asset));
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
        &hub(&t.asset),
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
        &hub(&t.asset),
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
        &hub(&t.asset),
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

    client.supply(&t.sup(0, 10_000_000_000i128));

    token::StellarAssetClient::new(&t.env, &t.asset).mint(&receiver, &flash_fee);

    let result = flatten_contract_result(client.try_flash_loan(
        &hub(&t.asset),
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
    let revenue_before = client.get_revenue(&hub(&t.asset));
    let state_before = t.state_snapshot();

    let result = client.try_flash_loan(
        &hub(&t.asset),
        &t.admin,
        &receiver,
        &1_0000000i128,
        &1_000i128,
        &Bytes::new(&t.env),
    );

    assert!(result.is_err(), "receiver that does not repay must fail");
    assert_eq!(tok.balance(&t.pool), balance_before);
    assert_eq!(client.get_revenue(&hub(&t.asset)), revenue_before);
    assert_pool_state_eq(&t.state_snapshot(), &state_before);
}

#[test]
fn test_flash_loan_rejects_insufficient_liquidity() {
    let t = TestSetup::new();
    let client = t.client();
    let receiver = Address::generate(&t.env);

    let result = flatten_contract_result(client.try_flash_loan(
        &hub(&t.asset),
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
        &hub(&t.asset),
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
fn test_create_strategy_rejects_zero_amount() {
    let t = TestSetup::new();
    let client = t.client();
    let caller = Address::generate(&t.env);

    let result =
        flatten_contract_result(client.try_create_strategy(&caller, &t.action(0, 0i128), &0i128));
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
    ));
    assert_contract_error(
        result,
        common::errors::CollateralError::InsufficientLiquidity as u32,
    );
}
#[test]
fn test_seize_positions_bad_debt() {
    let t = TestSetup::new();
    let client = t.client();

    client.supply(&t.sup(0, 50_000_000_000i128));

    let borrower = Address::generate(&t.env);
    let updated_borrow = client
        .borrow(&borrower, &t.bor(0, 100_0000000i128))
        .get_unchecked(0);

    client.update_indexes(&hub(&t.asset));
    let idx_before = client.get_sync_data(&hub(&t.asset)).state;

    client.seize_positions(&t.sez(AccountPositionType::Borrow, &updated_borrow.position));

    client.update_indexes(&hub(&t.asset));
    let idx_after = client.get_sync_data(&hub(&t.asset)).state;
    assert_eq!(
        idx_after.borrowed, 0,
        "seized scaled debt should be removed from the market"
    );
    assert!(
        idx_after.supply_index <= idx_before.supply_index,
        "supply index should decrease or stay same after bad debt"
    );
}

#[test]
fn test_seize_positions_rejects_borrowed_accounting_underflow() {
    let t = TestSetup::new();
    let client = t.client();

    client.supply(&t.sup(0, 50_000_000_000i128));

    let borrower = Address::generate(&t.env);
    let updated_borrow = client
        .borrow(&borrower, &t.bor(0, 100_0000000i128))
        .get_unchecked(0);

    t.edit_state(|state| {
        state.borrowed = 0;
    });

    let result = flatten_contract_result(
        client.try_seize_positions(&t.sez(AccountPositionType::Borrow, &updated_borrow.position)),
    );
    assert_contract_error(result, common::errors::GenericError::MathOverflow as u32);
}
#[test]
fn test_seize_positions_deposit_dust() {
    let t = TestSetup::new();
    let client = t.client();

    let updated = client.supply(&t.sup(0, 100_0000000i128)).get_unchecked(0);

    let revenue_before = client.get_revenue(&hub(&t.asset));
    client.seize_positions(&t.sez(AccountPositionType::Deposit, &updated.position));

    let revenue_after = client.get_revenue(&hub(&t.asset));
    assert!(
        revenue_after > revenue_before,
        "protocol revenue should increase from absorbed dust"
    );
}

// Pins the per-entry reload semantics: a batch hitting the same hub-asset
// twice must equal the same seizes issued as sequential single-entry calls.
#[test]
fn test_seize_positions_duplicate_market_batch_matches_sequential_singles() {
    let run = |batched: bool| -> PoolStateRaw {
        let t = TestSetup::new();
        let client = t.client();

        let supplied = client.supply(&t.sup(0, 100_0000000i128)).get_unchecked(0);
        let borrower = Address::generate(&t.env);
        let borrowed = client
            .borrow(&borrower, &t.bor(0, 50_0000000i128))
            .get_unchecked(0);

        let deposit = t.sez_entry(AccountPositionType::Deposit, &supplied.position);
        let borrow = t.sez_entry(AccountPositionType::Borrow, &borrowed.position);
        if batched {
            client.seize_positions(&vec![&t.env, deposit, borrow]);
        } else {
            client.seize_positions(&vec![&t.env, deposit]);
            client.seize_positions(&vec![&t.env, borrow]);
        }
        client.get_sync_data(&hub(&t.asset)).state
    };

    let batch = run(true);
    let sequential = run(false);
    assert_eq!(batch.supplied, sequential.supplied);
    assert_eq!(batch.borrowed, sequential.borrowed);
    assert_eq!(batch.revenue, sequential.revenue);
    assert_eq!(batch.supply_index, sequential.supply_index);
    assert_eq!(batch.borrow_index, sequential.borrow_index);
    assert_eq!(batch.cash, sequential.cash);
}
#[test]
fn test_claim_revenue() {
    let t = TestSetup::new();
    let client = t.client();

    client.supply(&t.sup(0, 50_000_000_000i128));

    let borrower = Address::generate(&t.env);
    client.borrow(&borrower, &t.bor(0, 10_000_000_000i128));

    // Advance time to accrue interest.
    t.advance_time(31_556_926);

    // Sync indexes to accrue revenue.
    client.update_indexes(&hub(&t.asset));

    let revenue = client.get_revenue(&hub(&t.asset));
    if revenue > 0 {
        let tok = token::Client::new(&t.env, &t.asset);
        let admin_balance_before = tok.balance(&t.admin);
        let claimed = client.claim_revenue(&hub(&t.asset)).actual_amount;
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
        .supply(&t.sup(0, 200_000_000_000_000i128))
        .get_unchecked(0);
    client.seize_positions(&t.sez(AccountPositionType::Deposit, &oversized_supply.position));

    // Reserves below revenue cap the claim at available `cash` and leave
    // residual revenue.
    t.edit_state(|s| s.cash = 100_000_000_000_000i128);

    let claimed = client.claim_revenue(&hub(&t.asset)).actual_amount;
    let remaining_revenue = client.get_revenue(&hub(&t.asset));

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
fn test_claim_revenue_rejects_utilization_above_max_after_revenue_burn() {
    let t = TestSetup::new();
    let client = t.client();

    t.env.as_contract(&t.pool, || {
        let key = PoolKey::Params(hub(&t.asset));
        let mut params: MarketParamsRaw = t.env.storage().persistent().get(&key).unwrap();
        params.max_utilization = RAY * 95 / 100;
        t.env.storage().persistent().set(&key, &params);
    });
    t.edit_state(|state| {
        state.supplied = 100 * RAY;
        state.borrowed = 90 * RAY;
        state.revenue = 10 * RAY;
        state.cash = 10_0000000i128;
    });

    let result = flatten_contract_result(client.try_claim_revenue(&hub(&t.asset)));
    assert_contract_error(
        result,
        common::errors::CollateralError::UtilizationAboveMax as u32,
    );
}

#[test]
fn test_claim_revenue_rejects_revenue_above_supplied() {
    let t = TestSetup::new();
    let client = t.client();

    let supplied = client
        .supply(&t.sup(0, 10_000_000_000i128))
        .get_unchecked(0);
    client.seize_positions(&t.sez(AccountPositionType::Deposit, &supplied.position));
    t.edit_state(|state| {
        state.supplied = 1;
    });

    let result = flatten_contract_result(client.try_claim_revenue(&hub(&t.asset)));
    assert_contract_error(result, common::errors::GenericError::MathOverflow as u32);
}

#[test]
fn test_update_params_rejects_invalid_utilization_range() {
    let t = TestSetup::new();
    let client = t.client();

    let model = InterestRateModel {
        max_borrow_rate: 2 * RAY,
        base_borrow_rate: RAY / 100,
        slope1: RAY / 10,
        slope2: RAY / 5,
        slope3: RAY,
        mid_utilization: RAY * 8 / 10,
        optimal_utilization: RAY * 8 / 10,
        max_utilization: RAY * 95 / 100,
        reserve_factor: 1000,
    };
    let result = flatten_contract_result(client.try_update_params(&hub(&t.asset), &model));
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
        max_borrow_rate: 2 * RAY,
        base_borrow_rate: RAY / 100,
        slope1: RAY / 10,
        slope2: RAY / 5,
        slope3: RAY,
        mid_utilization: RAY / 2,
        optimal_utilization: RAY,
        max_utilization: RAY * 95 / 100,
        reserve_factor: 1000,
    };
    let result = flatten_contract_result(client.try_update_params(&hub(&t.asset), &model));
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
        max_borrow_rate: 2 * RAY,
        base_borrow_rate: RAY / 100,
        slope1: RAY / 10,
        slope2: RAY / 5,
        slope3: RAY,
        mid_utilization: RAY / 2,
        optimal_utilization: RAY * 8 / 10,
        max_utilization: RAY * 95 / 100,
        reserve_factor: 10_000,
    };
    let result = flatten_contract_result(client.try_update_params(&hub(&t.asset), &model));
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
        max_borrow_rate: 2 * RAY,
        base_borrow_rate: -1i128,
        slope1: RAY / 10,
        slope2: RAY / 5,
        slope3: RAY,
        mid_utilization: RAY / 2,
        optimal_utilization: RAY * 8 / 10,
        max_utilization: RAY * 95 / 100,
        reserve_factor: 1000,
    };
    let result = flatten_contract_result(client.try_update_params(&hub(&t.asset), &model));
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
        max_borrow_rate: RAY / 100,
        base_borrow_rate: RAY / 100,
        slope1: RAY / 100,
        slope2: RAY / 100,
        slope3: RAY / 100,
        mid_utilization: RAY / 2,
        optimal_utilization: RAY * 8 / 10,
        max_utilization: RAY * 95 / 100,
        reserve_factor: 1000,
    };
    let result = flatten_contract_result(client.try_update_params(&hub(&t.asset), &model));
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
        max_borrow_rate: 2 * RAY + 1,
        base_borrow_rate: RAY / 100,
        slope1: RAY / 10,
        slope2: RAY / 5,
        slope3: RAY,
        mid_utilization: RAY / 2,
        optimal_utilization: RAY * 8 / 10,
        max_utilization: RAY * 95 / 100,
        reserve_factor: 1000,
    };
    let result = flatten_contract_result(client.try_update_params(&hub(&t.asset), &model));
    assert_contract_error(
        result,
        common::errors::CollateralError::MaxBorrowRateTooHigh as u32,
    );
}

#[test]
fn test_views() {
    let t = TestSetup::new();
    let client = t.client();

    let util = client.get_utilisation(&hub(&t.asset));
    assert_eq!(util, 0, "utilization should be zero initially");

    client.supply(&t.sup(0, 10_000_000_000i128));

    let supplied = client.get_supplied_amount(&hub(&t.asset));
    assert!(
        supplied > 0,
        "supplied_amount should be positive after supply"
    );

    let reserves = client.get_reserves(&hub(&t.asset));
    assert!(reserves > 0, "reserves should be positive");

    let borrower = Address::generate(&t.env);
    client.borrow(&borrower, &t.bor(0, 100_0000000i128));

    let borrowed = client.get_borrowed_amount(&hub(&t.asset));
    assert!(borrowed > 0, "borrowed_amount should be positive");

    let util_after = client.get_utilisation(&hub(&t.asset));
    assert!(
        util_after > 0,
        "utilization should be positive after borrow"
    );

    assert!(
        client.get_deposit_rate(&hub(&t.asset)) >= 0,
        "deposit rate view should be callable"
    );
    assert!(
        client.get_borrow_rate(&hub(&t.asset)) >= 0,
        "borrow rate view should be callable"
    );
    assert!(
        client.get_revenue(&hub(&t.asset)) >= 0,
        "protocol revenue view should be callable"
    );
    t.advance_time(60);
    assert!(
        client.get_delta_time(&hub(&t.asset)) > 0,
        "delta_time should be positive"
    );
}

// Liquidation fee on withdraw accrues to protocol revenue; user receives gross minus fee.
#[test]
fn test_withdraw_liquidation_fee_accrues_to_revenue() {
    let t = TestSetup::new();
    let client = t.client();

    let supply_amount = 10_000_000_000i128;
    let updated_pos = client.supply(&t.sup(0, supply_amount)).get_unchecked(0);

    let revenue_before = client.get_revenue(&hub(&t.asset));

    let user = Address::generate(&t.env);
    let tok = token::Client::new(&t.env, &t.asset);
    let user_balance_before = tok.balance(&user);

    let gross = 10_000_000_000_i128;
    let fee = 10_000_000_i128;
    let final_pos = client
        .withdraw(
            &user,
            &true,
            &t.wdr(updated_pos.position.scaled_amount, gross, fee),
        )
        .get_unchecked(0);

    let user_balance_after = tok.balance(&user);
    assert_eq!(
        user_balance_after - user_balance_before,
        gross - fee,
        "user should receive gross minus protocol fee"
    );
    let revenue_after = client.get_revenue(&hub(&t.asset));
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
    let updated_pos = client.supply(&t.sup(0, supply_amount)).get_unchecked(0);

    let revenue_before = client.get_revenue(&hub(&t.asset));
    let user = Address::generate(&t.env);
    let tok = token::Client::new(&t.env, &t.asset);
    let user_balance_before = tok.balance(&user);

    let gross = 1_000_000_000_i128;
    let final_pos = client
        .withdraw(
            &user,
            &true,
            &t.wdr(updated_pos.position.scaled_amount, gross, 0i128),
        )
        .get_unchecked(0);

    assert_eq!(tok.balance(&user) - user_balance_before, gross);
    assert_eq!(client.get_revenue(&hub(&t.asset)), revenue_before);
    assert_eq!(final_pos.actual_amount, gross);
}

// No-op repay with amount=0 leaves position and pool state untouched.
#[test]
fn test_repay_zero_amount_is_no_op() {
    let t = TestSetup::new();
    let client = t.client();

    client.supply(&t.sup(0, 50_000_000_000i128));

    let borrower = Address::generate(&t.env);
    let updated_borrow = client
        .borrow(&borrower, &t.bor(0, 100_0000000i128))
        .get_unchecked(0);
    let scaled_before = updated_borrow.position.scaled_amount;
    let state_before = t.state_snapshot();

    let result = client
        .repay(&borrower, &t.ract(scaled_before, 0i128))
        .get_unchecked(0);

    assert_eq!(result.actual_amount, 0);
    assert_eq!(result.position.scaled_amount, scaled_before);
    assert_pool_state_eq(&t.state_snapshot(), &state_before);
}

// Zero-amount add_rewards is accepted by `require_nonneg_amount` and leaves
// the index unchanged.
#[test]
fn test_add_rewards_zero_amount_is_no_op() {
    let t = TestSetup::new();
    let client = t.client();

    client.supply(&t.sup(0, 10_000_000_000i128));

    let snapshot_before = t.state_snapshot();
    client.add_rewards(&hub(&t.asset), &0i128);
    let result = client.get_sync_data(&hub(&t.asset)).state;

    assert_eq!(result.supply_index, snapshot_before.supply_index);
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

    client.supply(&t.sup(0, 50_000_000_000i128));

    let borrower = Address::generate(&t.env);
    let updated_borrow = client
        .borrow(&borrower, &t.bor(0, 100_0000000i128))
        .get_unchecked(0);

    // Advance time to accrue interest so current_debt > initial.
    t.advance_time(60);

    let partial = 10_0000000i128;
    let final_pos = client
        .repay(
            &borrower,
            &t.ract(updated_borrow.position.scaled_amount, partial),
        )
        .get_unchecked(0);

    assert_eq!(
        final_pos.actual_amount, partial,
        "partial repay returns the amount passed in"
    );
    assert!(
        final_pos.position.scaled_amount > 0,
        "position should still have residual debt after partial repay"
    );
    assert!(
        final_pos.position.scaled_amount < updated_borrow.position.scaled_amount,
        "scaled debt should decrease after partial repay"
    );
}

#[test]
fn test_add_rewards_increases_supply_index() {
    let t = TestSetup::new();
    let client = t.client();

    client.supply(&t.sup(0, 50_000_000_000i128));

    client.update_indexes(&hub(&t.asset));
    let idx_before = client.get_sync_data(&hub(&t.asset)).state;

    client.add_rewards(&hub(&t.asset), &1_000_000_000i128);

    client.update_indexes(&hub(&t.asset));
    let idx_after = client.get_sync_data(&hub(&t.asset)).state;
    assert!(
        idx_after.supply_index > idx_before.supply_index,
        "supply index should increase after add_rewards"
    );
}

// create_strategy records debt, transfers net amount, and accrues fee as protocol revenue.
#[test]
fn test_create_strategy_emits_position_and_transfers_net() {
    let t = TestSetup::new();
    let client = t.client();

    // Supply reserves so create_strategy can transfer.
    client.supply(&t.sup(0, 50_000_000_000i128));

    let caller = Address::generate(&t.env);
    let tok = token::Client::new(&t.env, &t.asset);
    let caller_before = tok.balance(&caller);
    let revenue_before = client.get_revenue(&hub(&t.asset));

    let amount = 100_0000000i128;
    let fee = 1_0000000i128;
    let result = client.create_strategy(&caller, &t.action(0, amount), &fee);
    let events = t.env.events().all();

    assert_eq!(result.actual_amount, amount);
    assert_eq!(result.amount_received, amount - fee);
    assert_eq!(count_topic(&events, "strategy", "fee"), 1);
    assert_eq!(
        strategy_fee_hub_id(&events),
        Some(0),
        "strategy fee event attributed to hub 0"
    );
    assert!(result.position.scaled_amount > 0, "debt recorded");

    let caller_after = tok.balance(&caller);
    assert_eq!(
        caller_after - caller_before,
        amount - fee,
        "caller receives net amount"
    );
    let revenue_after = client.get_revenue(&hub(&t.asset));
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
    let claimed = client.claim_revenue(&hub(&t.asset)).actual_amount;
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
        max_borrow_rate: new_max,
        base_borrow_rate: new_base,
        slope1: new_s1,
        slope2: new_s2,
        slope3: new_s3,
        mid_utilization: new_mid,
        optimal_utilization: new_opt,
        max_utilization: RAY * 95 / 100,
        reserve_factor: new_reserve,
    };
    client.update_params(&hub(&t.asset), &model);

    // Updated fields round-trip through get_sync_data().
    let sync = client.get_sync_data(&hub(&t.asset));
    assert_eq!(sync.params.max_borrow_rate, new_max, "max_borrow_rate");
    assert_eq!(sync.params.base_borrow_rate, new_base, "base_borrow_rate");
    assert_eq!(sync.params.slope1, new_s1, "slope1");
    assert_eq!(sync.params.slope2, new_s2, "slope2");
    assert_eq!(sync.params.slope3, new_s3, "slope3");
    assert_eq!(sync.params.mid_utilization, new_mid, "mid_utilization");
    assert_eq!(
        sync.params.optimal_utilization, new_opt,
        "optimal_utilization"
    );
    assert_eq!(sync.params.reserve_factor, new_reserve, "reserve_factor");

    // With base rate still 1% and higher slopes, 50% utilization uses updated slope1.
    client.supply(&t.sup(0, 10_000_000_000i128));
    let borrower = Address::generate(&t.env);
    let _ = client.borrow(&borrower, &t.bor(0, 100_0000000i128));
}

// slope3 < slope2 maps to SlopeNonMonotonic.
#[test]
fn test_update_params_rejects_invalid_slope_ordering() {
    let t = TestSetup::new();
    let client = t.client();

    // slope3 < slope2 is invalid.
    let model = InterestRateModel {
        max_borrow_rate: 2 * RAY,
        base_borrow_rate: RAY / 100,
        slope1: RAY / 10,
        slope2: RAY / 2,
        slope3: RAY / 5,
        mid_utilization: RAY / 2,
        optimal_utilization: RAY * 8 / 10,
        max_utilization: RAY * 95 / 100,
        reserve_factor: 1000,
    };
    let result = flatten_contract_result(client.try_update_params(&hub(&t.asset), &model));
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
        max_borrow_rate: 2 * RAY,
        base_borrow_rate: RAY / 100,
        slope1: RAY / 10,
        slope2: RAY / 5,
        slope3: RAY,
        mid_utilization: 0i128,
        optimal_utilization: RAY * 8 / 10,
        max_utilization: RAY * 95 / 100,
        reserve_factor: 1000,
    };
    let result = flatten_contract_result(client.try_update_params(&hub(&t.asset), &model));
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
        max_borrow_rate: 2 * RAY,
        base_borrow_rate: RAY / 100,
        slope1: RAY / 10,
        slope2: RAY / 5,
        slope3: RAY,
        mid_utilization: RAY / 2,
        optimal_utilization: RAY * 8 / 10,
        max_utilization: RAY * 95 / 100,
        reserve_factor: BPS as u32,
    };
    let result = flatten_contract_result(client.try_update_params(&hub(&t.asset), &model));
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
    params.base_borrow_rate = -1;

    let result = flatten_contract_result(client.try_create_market(&0u32, &params));
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

    let result = flatten_contract_result(client.try_create_market(&0u32, &market_params(&t.asset)));
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
    let result =
        flatten_contract_result(client.try_supply(&t.sup_for(&unknown_asset, 0, 1_0000000i128)));
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
    client.create_market(&0u32, &market_params(&asset_b));

    let sync = client.get_sync_data(&hub(&asset_b));
    if sync.state.supply_index != RAY {
        panic!("supply index must start at RAY");
    }
    if sync.state.borrow_index != RAY {
        panic!("borrow index must start at RAY");
    }
    if sync.state.last_timestamp != t.env.ledger().timestamp() * MS_PER_SECOND {
        panic!("last_timestamp must be ledger time in milliseconds");
    }
    assert_eq!(sync.state.supplied, 0);
    assert_eq!(sync.state.borrowed, 0);
    assert_eq!(sync.state.revenue, 0);
    assert_eq!(sync.state.cash, 0);
    assert_eq!(sync.params.asset_id, asset_b);
}

// Market mutations stay isolated by asset.
#[test]
fn test_two_market_isolation() {
    let t = TestSetup::new();
    let client = t.client();

    let asset_b = Address::generate(&t.env);
    client.create_market(&0u32, &market_params(&asset_b));
    let b_initial = t.state_of(&asset_b);

    let a_before = t.state_snapshot();
    let supply_amount = 10_000_000_000i128;
    client.supply(&t.sup(0, supply_amount));

    let a_after_supply = t.state_snapshot();
    if a_after_supply.supplied <= a_before.supplied {
        panic!("market A supplied must increase after supply");
    }
    assert_eq!(a_after_supply.cash, a_before.cash + supply_amount);

    let b_after_supply = t.state_of(&asset_b);
    assert_pool_state_eq(&b_after_supply, &b_initial);
    assert_eq!(b_after_supply.cash, b_initial.cash);

    let borrower = Address::generate(&t.env);
    let borrow_amount = 100_0000000i128;
    client.borrow(&borrower, &t.bor(0, borrow_amount));

    let a_after_borrow = t.state_snapshot();
    if a_after_borrow.borrowed <= a_after_supply.borrowed {
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
        .try_create_market(&0u32, &market_params(&asset_b));

    assert!(
        result.is_err(),
        "create_market without owner auth must fail"
    );
}

// bulk_get_indexes returns per-asset simulated indexes in request order.
#[test]
fn test_bulk_get_indexes_matches_per_asset() {
    let t = TestSetup::new();
    let client = t.client();

    // Create a borrow so indexes accrue over time.
    client.supply(&t.sup(0, 10_000_000_000i128));
    let borrower = Address::generate(&t.env);
    client.borrow(&borrower, &t.bor(0, 5_000_000_000i128));

    t.advance_time(86_400);

    let assets = soroban_sdk::vec![&t.env, hub(&t.asset)];
    let bulk = client.get_bulk_indexes(&assets);
    assert_eq!(bulk.len(), 1, "one entry per requested asset");

    let now_ms = t.env.ledger().timestamp() * common::constants::MS_PER_SECOND;
    let reference = common::types::MarketIndexRaw::from(&common::rates::simulate_update_indexes(
        &t.env,
        now_ms,
        &client.get_sync_data(&hub(&t.asset)),
    ));
    assert_eq!(
        bulk.get_unchecked(0),
        reference,
        "bulk entry equals the simulated per-asset read"
    );
    assert!(
        bulk.get_unchecked(0).borrow_index > RAY,
        "borrow index must have accrued past RAY for the equality to be meaningful"
    );
}

// Multi-asset results are input-aligned; utilized and idle markets keep
// separate indexes.
#[test]
fn test_bulk_get_indexes_multi_asset_alignment() {
    let t = TestSetup::new();
    let client = t.client();

    let asset_b = Address::generate(&t.env);
    client.create_market(&0u32, &market_params(&asset_b));

    // Only market A gets utilization, so only its indexes accrue.
    client.supply(&t.sup(0, 10_000_000_000i128));
    let borrower = Address::generate(&t.env);
    client.borrow(&borrower, &t.bor(0, 5_000_000_000i128));

    t.advance_time(86_400);

    let assets = soroban_sdk::vec![&t.env, hub(&t.asset), hub(&asset_b)];
    let bulk = client.get_bulk_indexes(&assets);
    assert_eq!(bulk.len(), 2);

    let a = bulk.get_unchecked(0);
    let b = bulk.get_unchecked(1);
    // Utilized market A accrues borrow/supply indexes; idle market B accrues
    // only the base borrow index and keeps its supply index flat.
    assert!(a.borrow_index > b.borrow_index && b.borrow_index > RAY);
    assert!(a.supply_index > RAY);
    assert_eq!(b.supply_index, RAY, "no borrows, no supplier rewards");

    // Input alignment: each entry matches its own per-asset simulation.
    let now_ms = t.env.ledger().timestamp() * common::constants::MS_PER_SECOND;
    let ref_a = common::types::MarketIndexRaw::from(&common::rates::simulate_update_indexes(
        &t.env,
        now_ms,
        &client.get_sync_data(&hub(&t.asset)),
    ));
    let ref_b = common::types::MarketIndexRaw::from(&common::rates::simulate_update_indexes(
        &t.env,
        now_ms,
        &client.get_sync_data(&hub(&asset_b)),
    ));
    assert_eq!(a, ref_a, "entry 0 matches market A");
    assert_eq!(b, ref_b, "entry 1 matches market B");
}

// An empty request returns an empty result without panicking.
#[test]
fn test_bulk_get_indexes_empty_request() {
    let t = TestSetup::new();
    let bulk = t.client().get_bulk_indexes(&soroban_sdk::Vec::new(&t.env));
    assert_eq!(bulk.len(), 0);
}

// Unknown assets fail bulk read with PoolNotInitialized, matching get_sync_data.
#[test]
fn test_bulk_get_indexes_unknown_asset_panics() {
    let t = TestSetup::new();
    let unknown = Address::generate(&t.env);
    let assets = soroban_sdk::vec![&t.env, hub(&unknown)];
    let result = flatten_contract_result(t.client().try_get_bulk_indexes(&assets));
    assert_contract_error(
        result,
        common::errors::GenericError::PoolNotInitialized as u32,
    );
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
        t.sup_entry(&t.asset, 0, amount_one),
        t.sup_entry(&asset_b, 0, amount_two),
    ];
    let bulk = client.supply(&entries);
    assert_eq!(bulk.len(), 2, "one mutation per entry");

    let first = bulk.get_unchecked(0);
    let second = bulk.get_unchecked(1);
    // PoolPositionMutation carries no asset; input alignment shows in the
    // per-entry applied amounts (A=amount_one, B=amount_two).
    assert_eq!(
        first.actual_amount, amount_one,
        "entry 0 result must be market A's input"
    );
    assert_eq!(
        second.actual_amount, amount_two,
        "entry 1 result must be market B's input"
    );

    let seq_first = client
        .supply(&t.sup_for(&asset_c, 0, amount_one))
        .get_unchecked(0);
    let seq_second = client
        .supply(&t.sup_for(&asset_d, 0, amount_two))
        .get_unchecked(0);

    assert_eq!(
        first.position.scaled_amount,
        seq_first.position.scaled_amount
    );
    assert_eq!(first.actual_amount, seq_first.actual_amount);
    assert_eq!(
        second.position.scaled_amount,
        seq_second.position.scaled_amount
    );
    assert_eq!(second.actual_amount, seq_second.actual_amount);

    // Bulk and sequential singles end with matching cash/state.
    let a_state = t.state_snapshot();
    let c_state = t.state_of(&asset_c);
    assert_pool_state_eq(&a_state, &c_state);
    assert_eq!(a_state.cash, c_state.cash);

    let b_state = t.state_of(&asset_b);
    let d_state = t.state_of(&asset_d);
    assert_pool_state_eq(&b_state, &d_state);
    assert_eq!(b_state.cash, d_state.cash);
}

// Bulk repay preserves input order and refunds overpayment.
#[test]
fn test_bulk_repay_overpayment_refunds_second_entry_surplus() {
    let t = TestSetup::new();
    let client = t.client();

    client.supply(&t.sup(0, 50_000_000_000i128));

    // Same borrower, independent debts.
    let borrower = Address::generate(&t.env);
    let debt_one = 100_0000000i128;
    let debt_two = 30_0000000i128;
    let first_borrow = client
        .borrow(&borrower, &t.bor(0, debt_one))
        .get_unchecked(0);
    let second_borrow = client
        .borrow(&borrower, &t.bor(0, debt_two))
        .get_unchecked(0);

    let overpayment = 5_0000000i128;
    let tok = token::Client::new(&t.env, &t.asset);
    let payer_before = tok.balance(&borrower);

    let actions = vec![
        &t.env,
        t.action(first_borrow.position.scaled_amount, debt_one),
        t.action(second_borrow.position.scaled_amount, debt_two + overpayment),
    ];
    let results = client.repay(&borrower, &actions);
    assert_eq!(results.len(), 2, "one mutation per entry");

    let first = results.get_unchecked(0);
    let second = results.get_unchecked(1);
    assert_eq!(
        first.actual_amount, debt_one,
        "entry 0 repays exactly, no refund"
    );
    assert_eq!(first.position.scaled_amount, 0);
    assert_eq!(
        second.actual_amount, debt_two,
        "entry 1 applies only the outstanding debt"
    );
    assert_eq!(second.position.scaled_amount, 0);
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
        t.sup_entry(&t.asset, 0, amount_one),
        t.sup_entry(&t.asset, 0, amount_two),
    ];
    let results = client.supply(&entries);
    assert_eq!(results.len(), 2);

    let first = results.get_unchecked(0);
    let second = results.get_unchecked(1);
    // Per-entry amounts preserve input order.
    assert_eq!(
        first.actual_amount, amount_one,
        "entry 1 applied the first input"
    );
    assert_eq!(
        second.actual_amount, amount_two,
        "entry 2 applied the second input"
    );

    let state_after = t.state_snapshot();
    assert_eq!(
        state_after.supplied,
        state_before.supplied + first.position.scaled_amount + second.position.scaled_amount,
        "total supplied is the sum of both entries"
    );
    assert_eq!(
        state_after.cash,
        state_before.cash + amount_one + amount_two
    );
}

// Overrides max utilization for accounting tests.
fn set_max_utilization(t: &TestSetup, max_utilization: i128) {
    t.env.as_contract(&t.pool, || {
        let key = PoolKey::Params(hub(&t.asset));
        let mut params: MarketParamsRaw = t.env.storage().persistent().get(&key).unwrap();
        params.max_utilization = max_utilization;
        t.env.storage().persistent().set(&key, &params);
    });
}

// Withdraw enforces max utilization on projected post-withdraw state.
#[test]
fn test_withdraw_above_max_utilization_panics_but_within_cap_succeeds() {
    let t = TestSetup::new();
    let client = t.client();
    set_max_utilization(&t, RAY / 2);

    // Supply 20 units; borrow 5 -> 25% utilization, well below the 50% cap.
    let supply_amount = 20_000_000_000i128;
    let supplied = client.supply(&t.sup(0, supply_amount)).get_unchecked(0);

    let borrower = Address::generate(&t.env);
    client.borrow(&borrower, &t.bor(0, 5_000_000_000i128));

    let supplier = Address::generate(&t.env);
    let scaled = supplied.position.scaled_amount;

    // Withdraw 5 units: supplied 20 -> 15, utilization 5/15 = 33% <= 50% cap.
    let ok = client
        .withdraw(&supplier, &false, &t.wdr(scaled, 5_000_000_000i128, 0i128))
        .get_unchecked(0);
    assert_eq!(ok.actual_amount, 5_000_000_000i128);

    // Withdraw 6 more units: supplied 15 -> 9, utilization 5/9 = 55% > 50% cap.
    let result = flatten_contract_result(client.try_withdraw(
        &supplier,
        &false,
        &t.wdr(ok.position.scaled_amount, 6_000_000_000i128, 0i128),
    ));
    assert_contract_error(
        result,
        common::errors::CollateralError::UtilizationAboveMax as u32,
    );
}

// `cash` changes by supply minus borrow, applied repay, and withdraw.
#[test]
fn test_cash_conservation_across_supply_borrow_overpaid_repay_withdraw() {
    let t = TestSetup::new();
    let client = t.client();

    let cash_start = t.state_snapshot().cash;

    let supply_amount = 50_000_000_000i128;
    let supplied = client.supply(&t.sup(0, supply_amount)).get_unchecked(0);
    assert_eq!(t.state_snapshot().cash, cash_start + supply_amount);

    let borrower = Address::generate(&t.env);
    let borrow_amount = 10_000_000_000i128;
    let borrowed = client
        .borrow(&borrower, &t.bor(0, borrow_amount))
        .get_unchecked(0);
    assert_eq!(
        t.state_snapshot().cash,
        cash_start + supply_amount - borrow_amount
    );

    // Surplus repayment is refunded and leaves `cash` unchanged.
    let overpayment = 4_000_000_000i128;
    let repaid = client
        .repay(
            &borrower,
            &t.ract(borrowed.position.scaled_amount, borrow_amount + overpayment),
        )
        .get_unchecked(0);
    assert_eq!(repaid.actual_amount, borrow_amount);
    assert_eq!(repaid.position.scaled_amount, 0);
    assert_eq!(t.state_snapshot().cash, cash_start + supply_amount);

    // Withdraw part of the supply; `cash` drops by exactly the net transfer.
    let withdraw_amount = 30_000_000_000i128;
    let supplier = Address::generate(&t.env);
    client.withdraw(
        &supplier,
        &false,
        &t.wdr(supplied.position.scaled_amount, withdraw_amount, 0i128),
    );
    assert_eq!(
        t.state_snapshot().cash,
        cash_start + supply_amount - withdraw_amount
    );
}
