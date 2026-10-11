extern crate std;

use super::*;
use crate::cache::Cache;
use crate::ops::flash;
use crate::test_support::{hub, LEDGER_PROTOCOL_VERSION};
use common::constants::{
    BPS, MILLISECONDS_PER_YEAR, MS_PER_SECOND, RAY, TTL_BUMP_INSTANCE, TTL_BUMP_SHARED,
    TTL_THRESHOLD_INSTANCE, TTL_THRESHOLD_SHARED,
};
use common::errors::{CollateralError, FlashLoanError, GenericError};
use common::math::fp::Ray;
use common::rates::simulate_update_indexes;
use common::types::{
    AccountPositionType, InterestRateModel, MarketIndexRaw, MarketParamsRaw, PoolKey,
    PoolNetSettleEntry, PoolStateRaw, ScaledPositionRaw,
};
use soroban_sdk::testutils::storage::{Instance as _, Persistent as _};
use soroban_sdk::testutils::{Address as _, ContractEvents, Events, Ledger, LedgerInfo};
use soroban_sdk::xdr::{ContractEventBody, ScVal, SorobanAuthorizationEntry};
use soroban_sdk::{
    contract, contractimpl, panic_with_error, token, vec, Address, Bytes, Env, Error, InvokeError,
    Vec,
};

const WAD_PER_RAW: i128 = 100_000_000_000_000_000_000;

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

fn count_first_topic(events: &ContractEvents, first: &str) -> usize {
    events
        .events()
        .iter()
        .filter(|event| {
            let ContractEventBody::V0(body) = &event.body;
            matches!(
                body.topics.first(),
                Some(ScVal::Symbol(topic)) if topic.0.to_utf8_string().as_deref() == Ok(first)
            )
        })
        .count()
}

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

#[contract]
pub struct PoolCallbackOverpayReceiver;

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

#[contractimpl]
impl PoolCallbackOverpayReceiver {
    pub fn execute_flash_loan(
        env: Env,
        _initiator: Address,
        asset: Address,
        amount: i128,
        fee: i128,
        pool: Address,
        _data: Bytes,
    ) {
        let receiver = env.current_contract_address();
        let token = token::Client::new(&env, &asset);
        token.transfer(&receiver, &pool, &1);
        token.approve(
            &receiver,
            &pool,
            &amount.checked_add(fee).unwrap(),
            &env.ledger().sequence().checked_add(1).unwrap(),
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

        let pool_address = env.register(LiquidityPool, (admin.clone(),));
        LiquidityPoolClient::new(&env, &pool_address)
            .create_market(&0u32, &market_params(&asset_address));

        let token_admin = token::StellarAssetClient::new(&env, &asset_address);
        token_admin.mint(&pool_address, &100_000_000_000_000i128);

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

    fn sup(&self, scaled_amount: i128, amount: i128) -> Vec<PoolSupplyEntry> {
        self.sup_for(&self.asset, scaled_amount, amount)
    }

    fn sup_for(&self, asset: &Address, scaled_amount: i128, amount: i128) -> Vec<PoolSupplyEntry> {
        vec![&self.env, self.sup_entry(asset, scaled_amount, amount)]
    }

    fn bor(&self, scaled_amount: i128, amount: i128) -> Vec<PoolBorrowEntry> {
        vec![
            &self.env,
            PoolBorrowEntry {
                action: self.action(scaled_amount, amount),
            },
        ]
    }

    fn wdr(&self, scaled_amount: i128, amount: i128, protocol_fee: i128) -> Vec<PoolWithdrawEntry> {
        vec![
            &self.env,
            PoolWithdrawEntry {
                action: self.action(scaled_amount, amount),
                protocol_fee,
            },
        ]
    }

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

    fn sez(&self, side: AccountPositionType, position: &ScaledPositionRaw) -> Vec<PoolSeizeEntry> {
        vec![&self.env, self.sez_entry(side, position)]
    }

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
            protocol_version: LEDGER_PROTOCOL_VERSION,
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

fn flatten_contract_result<T, E: core::fmt::Debug>(
    result: Result<Result<T, E>, Result<Error, InvokeError>>,
) -> Result<T, Error> {
    match result {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(err)) => panic!("contract call succeeded but output conversion failed: {err:?}"),
        Err(invoke) => Err(invoke.expect("expected contract error, got host-level InvokeError")),
    }
}

/// Asserts `rate` exceeds `MAX_BORROW_RATE_RAY` (200% APR) in per-millisecond
/// form, the largest per-ms rate any curve can emit. View getters must return
/// annual RAY, so a per-ms value fails.
fn assert_rate_is_annual_apr(rate: i128, label: &str) {
    let max_per_ms = (2 * RAY) / (MILLISECONDS_PER_YEAR as i128);
    assert!(
        rate > max_per_ms,
        "{label} view must be annual RAY APR, got {rate} (200% per-ms is {max_per_ms})"
    );
}

fn assert_annual_apr_views(
    client: &LiquidityPoolClient<'_>,
    asset: &Address,
    expected_util: i128,
    expected_borrow_apr: i128,
    expected_deposit_apr: i128,
) {
    let key = hub(asset);
    assert_eq!(client.get_utilisation(&key), expected_util, "utilization");
    let borrow = client.get_borrow_rate(&key);
    let deposit = client.get_deposit_rate(&key);
    assert_eq!(borrow, expected_borrow_apr, "borrow APR");
    assert_eq!(deposit, expected_deposit_apr, "deposit APR");
    assert_rate_is_annual_apr(borrow, "borrow");
    if expected_deposit_apr > 0 {
        assert_rate_is_annual_apr(deposit, "deposit");
    }
}

fn assert_contract_error<T: core::fmt::Debug>(result: Result<T, Error>, expected_code: u32) {
    match result {
        Ok(value) => panic!("expected contract error {expected_code}, got Ok({value:?})"),
        Err(err) => assert_eq!(
            err,
            Error::from_contract_error(expected_code),
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

    assert_eq!(
        client.get_supplied_amount(&hub(&t.asset)),
        amount,
        "supplied amount should round-trip at the initial unit index"
    );
}

#[test]
fn test_market_mutations_emit_indexer_events() {
    let t = TestSetup::new();
    let client = t.client();
    let second_asset = Address::generate(&t.env);

    client.create_market(&1, &market_params(&second_asset));
    assert_eq!(
        count_topic(&t.env.events().all(), "market", "batch_params_update"),
        1,
        "last invocation (create_market) should retain one params batch event"
    );

    client.supply(&t.sup(0, 10_000_000_000));
    assert_eq!(
        count_topic(&t.env.events().all(), "market", "batch_state_update"),
        1,
        "last invocation (supply) should retain one state batch event"
    );

    client.update_indexes(&vec![&t.env, hub(&t.asset)]);
    assert_eq!(
        count_topic(&t.env.events().all(), "market", "batch_state_update"),
        1,
        "last invocation (update_indexes) should retain one state batch event"
    );

    t.env.as_contract(&t.pool, || {
        let key = PoolKey::Params(hub(&t.asset));
        let mut params: MarketParamsRaw = t.env.storage().persistent().get(&key).unwrap();
        params.flashloan_fee = 100;
        t.env.storage().persistent().set(&key, &params);
    });
    client.create_strategy(
        &Address::generate(&t.env),
        &t.action(0, 1_000_000_000),
        &true,
    );
    let strategy_events = t.env.events().all();
    assert_eq!(
        count_topic(&strategy_events, "strategy", "fee"),
        1,
        "last invocation (create_strategy) should retain one strategy fee event"
    );
    assert_eq!(
        count_topic(&strategy_events, "market", "batch_state_update"),
        1,
        "last invocation (create_strategy) should retain one state batch event"
    );
}

#[test]
fn test_pool_mutation_renews_instance_and_market_ttls() {
    let t = TestSetup::new();
    let params_key = PoolKey::Params(hub(&t.asset));
    let state_key = PoolKey::State(hub(&t.asset));
    let initial_instance_ttl = t
        .env
        .as_contract(&t.pool, || t.env.storage().instance().get_ttl());
    let ledgers_to_age = initial_instance_ttl - TTL_THRESHOLD_INSTANCE + 1;

    t.env
        .ledger()
        .with_mut(|ledger| ledger.sequence_number += ledgers_to_age);
    t.env.as_contract(&t.pool, || {
        assert!(t.env.storage().instance().get_ttl() < TTL_THRESHOLD_INSTANCE);
        assert!(t.env.storage().persistent().get_ttl(&params_key) < TTL_THRESHOLD_SHARED);
        assert!(t.env.storage().persistent().get_ttl(&state_key) < TTL_THRESHOLD_SHARED);
    });

    t.client().supply(&t.sup(0, 10_000_000_000));

    t.env.as_contract(&t.pool, || {
        assert!(
            t.env.storage().instance().get_ttl() >= TTL_BUMP_INSTANCE - 1,
            "instance TTL should be restored to the bump horizon"
        );
        assert!(
            t.env.storage().persistent().get_ttl(&params_key) >= TTL_BUMP_SHARED - 1,
            "market params TTL should be restored to the bump horizon"
        );
        assert!(
            t.env.storage().persistent().get_ttl(&state_key) >= TTL_BUMP_SHARED - 1,
            "market state TTL should be restored to the bump horizon"
        );
    });
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
    assert_contract_error(result, GenericError::AmountMustBePositive as u32);
}

#[test]
fn test_borrow_rejects_when_reserves_are_insufficient() {
    let t = TestSetup::new();
    let client = t.client();
    let borrower = Address::generate(&t.env);

    let result =
        flatten_contract_result(client.try_borrow(&borrower, &t.bor(0, 200_000_000_000_000i128)));
    assert_contract_error(result, CollateralError::InsufficientLiquidity as u32);
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
fn test_withdraw_rounds_share_burn_up_for_positive_transfer() {
    let t = TestSetup::new();
    t.env.as_contract(&t.pool, || {
        let params_key = PoolKey::Params(hub(&t.asset));
        let mut params: MarketParamsRaw = t.env.storage().persistent().get(&params_key).unwrap();
        params.asset_decimals = 27;
        t.env.storage().persistent().set(&params_key, &params);

        let state_key = PoolKey::State(hub(&t.asset));
        let mut state: PoolStateRaw = t.env.storage().persistent().get(&state_key).unwrap();
        state.supplied = 100 * RAY;
        state.supply_index = 3 * RAY;
        t.env.storage().persistent().set(&state_key, &state);
    });

    let user = Address::generate(&t.env);
    let result = t.client().withdraw(&user, &false, &t.wdr(10 * RAY, 1, 0));
    let mutation = result.get_unchecked(0);
    assert_eq!(mutation.actual_amount, 1);
    assert_eq!(mutation.position.scaled_amount, 10 * RAY - 1);
}

#[test]
fn test_supply_rejects_positive_amount_that_rounds_to_zero_shares() {
    let t = TestSetup::new();
    t.env.as_contract(&t.pool, || {
        let params_key = PoolKey::Params(hub(&t.asset));
        let mut params: MarketParamsRaw = t.env.storage().persistent().get(&params_key).unwrap();
        params.asset_decimals = 27;
        t.env.storage().persistent().set(&params_key, &params);

        let state_key = PoolKey::State(hub(&t.asset));
        let mut state: PoolStateRaw = t.env.storage().persistent().get(&state_key).unwrap();
        state.supply_index = 3 * RAY;
        t.env.storage().persistent().set(&state_key, &state);
    });

    let result = flatten_contract_result(t.client().try_supply(&t.sup(0, 1)));
    assert_contract_error(result, GenericError::SupplyRoundsToZeroShares as u32);
}

#[test]
fn test_supply_rejects_underbacked_market_after_floor_index_lift() {
    let t = TestSetup::new();
    t.edit_state(|state| {
        state.supplied = 1_000 * RAY;
        state.borrowed = 0;
        state.revenue = 0;
        state.supply_index = common::constants::SUPPLY_INDEX_FLOOR_RAW + 1;
        state.borrow_index = RAY;
        state.cash = 0;
    });
    let before = t.state_snapshot();

    let result = flatten_contract_result(t.client().try_supply(&t.sup(0, 10_000_000)));
    assert_contract_error(result, CollateralError::PoolInsolvent as u32);
    let after = t.state_snapshot();
    assert_pool_state_eq(&after, &before);
    assert_eq!(after.cash, before.cash);
}

#[test]
fn test_recapitalize_caps_to_shortfall_and_refunds_every_excess_unit() {
    let t = TestSetup::new();
    let payer = Address::generate(&t.env);
    let token_admin = token::StellarAssetClient::new(&t.env, &t.asset);
    let token = token::Client::new(&t.env, &t.asset);

    let claim = 100 * RAY;
    let cash = 900_000_000i128;
    let shortfall = 100_000_000i128;
    t.edit_state(|state| {
        state.supplied = claim;
        state.borrowed = 0;
        state.revenue = 0;
        state.supply_index = RAY;
        state.borrow_index = RAY;
        state.cash = cash;
    });

    let offered = 250_000_000i128;
    token_admin.mint(&payer, &offered);
    token.transfer(&payer, &t.pool, &offered);
    let pool_balance_before_call = token.balance(&t.pool);

    let before = t.state_snapshot();
    let result = t.client().recapitalize(&hub(&t.asset), &payer, &offered);
    let after = t.state_snapshot();

    assert_eq!(result.actual_amount, shortfall);
    assert_eq!(after.cash, cash + shortfall);
    assert_eq!(after.supplied, before.supplied);
    assert_eq!(after.borrowed, before.borrowed);
    assert_eq!(after.revenue, before.revenue);
    assert_eq!(after.supply_index, before.supply_index);
    assert_eq!(after.borrow_index, before.borrow_index);
    assert_eq!(token.balance(&payer), offered - shortfall);
    assert_eq!(
        token.balance(&t.pool),
        pool_balance_before_call - (offered - shortfall)
    );

    let supplied = t.client().supply(&t.sup(0, 1));
    assert_eq!(supplied.get_unchecked(0).actual_amount, 1);
}

#[test]
fn test_recapitalize_refunds_everything_when_market_is_already_backed() {
    let t = TestSetup::new();
    let payer = Address::generate(&t.env);
    let token_admin = token::StellarAssetClient::new(&t.env, &t.asset);
    let token = token::Client::new(&t.env, &t.asset);
    let offered = 250_000_000i128;

    token_admin.mint(&payer, &offered);
    token.transfer(&payer, &t.pool, &offered);
    let before = t.state_snapshot();
    let pool_balance_before_call = token.balance(&t.pool);

    let result = t.client().recapitalize(&hub(&t.asset), &payer, &offered);
    let after = t.state_snapshot();

    assert_eq!(result.actual_amount, 0);
    assert_pool_state_eq(&after, &before);
    assert_eq!(token.balance(&payer), offered);
    assert_eq!(token.balance(&t.pool), pool_balance_before_call - offered);
}

#[test]
fn test_repay_rejects_positive_amount_that_rounds_to_zero_shares() {
    let t = TestSetup::new();
    t.env.as_contract(&t.pool, || {
        let params_key = PoolKey::Params(hub(&t.asset));
        let mut params: MarketParamsRaw = t.env.storage().persistent().get(&params_key).unwrap();
        params.asset_decimals = 27;
        t.env.storage().persistent().set(&params_key, &params);

        let state_key = PoolKey::State(hub(&t.asset));
        let mut state: PoolStateRaw = t.env.storage().persistent().get(&state_key).unwrap();
        state.borrowed = 100 * RAY;
        state.borrow_index = 3 * RAY;
        t.env.storage().persistent().set(&state_key, &state);
    });

    let payer = Address::generate(&t.env);
    let result = flatten_contract_result(t.client().try_repay(&payer, &t.ract(10 * RAY, 1)));
    assert_contract_error(result, GenericError::RepayRoundsToZeroShares as u32);
}

#[test]
fn test_net_settle_uses_directed_rounding_and_rejects_zero_debt_burn() {
    let t = TestSetup::new();
    t.env.as_contract(&t.pool, || {
        let params_key = PoolKey::Params(hub(&t.asset));
        let mut params: MarketParamsRaw = t.env.storage().persistent().get(&params_key).unwrap();
        params.asset_decimals = 27;
        t.env.storage().persistent().set(&params_key, &params);

        let state_key = PoolKey::State(hub(&t.asset));
        let mut state: PoolStateRaw = t.env.storage().persistent().get(&state_key).unwrap();
        state.supplied = 100 * RAY;
        state.borrowed = 100 * RAY;
        state.supply_index = 3 * RAY;
        state.borrow_index = RAY;
        t.env.storage().persistent().set(&state_key, &state);
    });

    let entry = PoolNetSettleEntry {
        hub_asset: hub(&t.asset),
        amount: 1,
        supply_position: ScaledPositionRaw {
            scaled_amount: 10 * RAY,
        },
        debt_position: ScaledPositionRaw {
            scaled_amount: 10 * RAY,
        },
    };
    let directed = t.client().net_settle(&entry);
    assert_eq!(directed.settled_amount, 1);
    assert_eq!(directed.supply_position.scaled_amount, 10 * RAY - 1);
    assert_eq!(directed.debt_position.scaled_amount, 10 * RAY - 1);

    t.env.as_contract(&t.pool, || {
        let state_key = PoolKey::State(hub(&t.asset));
        let mut state: PoolStateRaw = t.env.storage().persistent().get(&state_key).unwrap();
        state.supply_index = RAY;
        state.borrow_index = 3 * RAY;
        t.env.storage().persistent().set(&state_key, &state);
    });
    let debt_leg_zero = flatten_contract_result(t.client().try_net_settle(&entry));
    assert_contract_error(
        debt_leg_zero,
        GenericError::NetSettleRoundsToZeroShares as u32,
    );
}

#[test]
fn test_net_settle_closes_both_when_floor_supply_matches_ceil_debt() {
    let t = TestSetup::new();
    // 100 tokens + 0.6 stroops: half-up is one stroop above the floor.
    let supply_pos = 100 * RAY + 6 * 10i128.pow(19);
    let debt_pos = 100 * RAY;
    t.edit_state(|s| {
        s.supplied = supply_pos;
        s.borrowed = debt_pos;
        s.supply_index = RAY;
        s.borrow_index = RAY;
    });

    let result = t.client().net_settle(&PoolNetSettleEntry {
        hub_asset: hub(&t.asset),
        amount: i128::MAX,
        supply_position: ScaledPositionRaw {
            scaled_amount: supply_pos,
        },
        debt_position: ScaledPositionRaw {
            scaled_amount: debt_pos,
        },
    });

    assert_eq!(result.settled_amount, 1_000_000_000);
    assert_eq!(result.supply_position.scaled_amount, 0);
    assert_eq!(result.debt_position.scaled_amount, 0);
    let state = t.state_snapshot();
    assert_eq!(state.supplied, 0);
    assert_eq!(state.borrowed, 0);
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
    assert_contract_error(result, CollateralError::WithdrawLessThanFee as u32);
}

#[test]
fn test_borrow_above_max_utilization_panics() {
    let t = TestSetup::new();
    t.edit_state(|s| {
        s.supplied = 100_000_000_000_000;
        s.borrowed = 0;
    });

    t.env.as_contract(&t.pool, || {
        let key = PoolKey::Params(hub(&t.asset));
        let mut params: MarketParamsRaw = t.env.storage().persistent().get(&key).unwrap();
        params.max_utilization = RAY / 2;
        t.env.storage().persistent().set(&key, &params);
    });

    let client = t.client();
    let borrower = Address::generate(&t.env);

    let result = flatten_contract_result(client.try_borrow(&borrower, &t.bor(0, 60_000i128)));
    assert_contract_error(result, CollateralError::UtilizationAboveMax as u32);
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
    assert_contract_error(result, CollateralError::InsufficientLiquidity as u32);
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
    assert_contract_error(result, GenericError::MathOverflow as u32);
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

    let repay_amount = 100_0000000i128;
    let final_pos = client
        .repay(
            &borrower,
            &t.ract(updated_borrow.position.scaled_amount, repay_amount),
        )
        .get_unchecked(0);

    assert_eq!(final_pos.actual_amount, repay_amount);
    assert_eq!(
        final_pos.position.scaled_amount, 0,
        "exact repay with no accrual should clear scaled debt"
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

    client.update_indexes(&vec![&t.env, hub(&t.asset)]);
    let initial_indexes = client.get_sync_data(&hub(&t.asset)).state;

    t.advance_time(31_556_926);

    client.update_indexes(&vec![&t.env, hub(&t.asset)]);
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
fn same_ledger_index_update_keeps_state_writable_at_inclusion() {
    use soroban_sdk::xdr::ToXdr;
    use soroban_sdk::{xdr, TryIntoVal};

    let t = TestSetup::new();
    t.client().supply(&t.sup(0, 50_000_000_000));
    t.client()
        .borrow(&Address::generate(&t.env), &t.bor(0, 10_000_000_000));
    let initial = t.client().get_sync_data(&hub(&t.asset)).state;
    let ledger = t.env.to_ledger_snapshot();

    for omit_same_ledger_commit in [true, false] {
        // Setup writes must not grant the simulation a writable State footprint.
        let env = Env::from_ledger_snapshot(ledger.clone());
        let pool: Address = xdr::ScAddress::from(&t.pool).try_into_val(&env).unwrap();
        let asset: Address = xdr::ScAddress::from(&t.asset).try_into_val(&env).unwrap();
        let key = hub(&asset);
        // Keep recording and enforcing in one frame: separate SDK invocations
        // reset their footprint. No rollback write may contaminate recording.
        let replay = env.try_as_contract::<_, Error>(&pool, || {
            if omit_same_ledger_commit {
                // Negative control: the exact former zero-elapsed-time branch.
                common::ttl::renew_instance(&env);
                let mut cache = Cache::load(&env, &key);
                assert!(!cache.needs_accrual());
                crate::interest::global_sync(&env, &mut cache);
                crate::events::emit_market_state(&env, cache.snapshot());
            } else {
                crate::ops::market::accrue(&env, vec![&env, key.clone()]);
            }
            assert_eq!(
                crate::storage::read_state(&env, &key).to_xdr(&env),
                initial.clone().to_xdr(&env)
            );
            env.ledger().with_mut(|ledger| {
                ledger.timestamp += 5;
                ledger.sequence_number += 1;
            });
            env.host().switch_to_enforcing_storage().unwrap();
            crate::ops::market::accrue(&env, vec![&env, key.clone()]);
            let committed = crate::storage::read_state(&env, &key);
            assert_eq!(committed.last_timestamp, initial.last_timestamp + 5_000);
            assert!(committed.borrow_index > initial.borrow_index);
            assert!(committed.supply_index > initial.supply_index);
            assert_eq!(committed.borrowed, initial.borrowed);
            assert_eq!(committed.cash, initial.cash);
        });
        if omit_same_ledger_commit {
            assert_eq!(
                replay,
                Err(Ok(Error::from_type_and_code(
                    xdr::ScErrorType::Storage,
                    xdr::ScErrorCode::ExceededLimit,
                )))
            );
        } else {
            assert_eq!(replay, Ok(()));
        }
    }
}

fn enable_flashloan(t: &TestSetup) {
    t.env.as_contract(&t.pool, || {
        let key = PoolKey::Params(hub(&t.asset));
        let mut params: MarketParamsRaw = t.env.storage().persistent().get(&key).unwrap();
        params.is_flashloanable = true;
        params.flashloan_fee = 100;
        t.env.storage().persistent().set(&key, &params);
    });
}

#[test]
fn test_flash_loan() {
    let t = TestSetup::new();
    let client = t.client();
    enable_flashloan(&t);

    client.supply(&t.sup(0, 10_000_000_000i128));

    let receiver = t.env.register(PoolFlashLoanReceiver, ());
    let flash_amount = 100_0000000i128;
    let flash_fee = 1_0000000i128;

    let token_admin_client = token::StellarAssetClient::new(&t.env, &t.asset);
    token_admin_client.mint(&receiver, &flash_fee);

    let tok = token::Client::new(&t.env, &t.asset);
    let pool_balance_before = tok.balance(&t.pool);
    let revenue_before = client.get_revenue(&hub(&t.asset));
    let fee = client.flash_loan(
        &hub(&t.asset),
        &t.admin,
        &receiver,
        &flash_amount,
        &Bytes::new(&t.env),
    );
    let revenue_after = client.get_revenue(&hub(&t.asset));
    let pool_balance_after = tok.balance(&t.pool);

    assert_eq!(
        fee, flash_fee,
        "pool derives the fee from flashloan_fee bps"
    );
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
        &Bytes::new(&t.env),
    ));

    assert_contract_error(result, GenericError::AmountMustBePositive as u32);
}

#[test]
fn test_flash_loan_rejects_non_contract_receiver_at_pool() {
    let t = TestSetup::new();
    let client = t.client();
    enable_flashloan(&t);
    let receiver = Address::generate(&t.env);

    let result = flatten_contract_result(client.try_flash_loan(
        &hub(&t.asset),
        &t.admin,
        &receiver,
        &1_0000000i128,
        &Bytes::new(&t.env),
    ));

    assert_contract_error(result, FlashLoanError::InvalidFlashloanReceiver as u32);
}

#[test]
fn test_flash_loan_rejects_direct_non_owner_pool_call() {
    let t = TestSetup::new();
    let client = t.client();
    enable_flashloan(&t);
    let receiver = t.env.register(PoolFlashLoanReceiver, ());
    let attacker = Address::generate(&t.env);
    let no_auths: [SorobanAuthorizationEntry; 0] = [];

    let result = client.set_auths(&no_auths).try_flash_loan(
        &hub(&t.asset),
        &attacker,
        &receiver,
        &1_0000000i128,
        &Bytes::new(&t.env),
    );

    assert!(
        result.is_err(),
        "direct pool flash loan without owner/controller auth must fail"
    );
}

#[test]
fn test_flash_loan_rejects_market_not_flashloanable() {
    let t = TestSetup::new();
    let client = t.client();

    let receiver = t.env.register(PoolFlashLoanReceiver, ());

    let result = flatten_contract_result(client.try_flash_loan(
        &hub(&t.asset),
        &t.admin,
        &receiver,
        &1_0000000i128,
        &Bytes::new(&t.env),
    ));

    assert_contract_error(result, FlashLoanError::FlashloanNotEnabled as u32);
}

#[test]
fn test_flash_loan_rejects_under_repay_with_invalid_flashloan_repay() {
    let t = TestSetup::new();
    let client = t.client();
    enable_flashloan(&t);
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
        &Bytes::new(&t.env),
    ));

    assert_contract_error(result, FlashLoanError::InvalidFlashloanRepay as u32);
}

#[test]
fn test_flash_loan_rejects_callback_balance_change() {
    let t = TestSetup::new();
    let client = t.client();
    enable_flashloan(&t);
    let receiver = t.env.register(PoolCallbackOverpayReceiver, ());
    let flash_amount = 100_0000000i128;
    let flash_fee = 1_0000000i128;

    client.supply(&t.sup(0, 10_000_000_000i128));
    token::StellarAssetClient::new(&t.env, &t.asset).mint(&receiver, &(flash_fee + 1));

    let result = flatten_contract_result(client.try_flash_loan(
        &hub(&t.asset),
        &t.admin,
        &receiver,
        &flash_amount,
        &Bytes::new(&t.env),
    ));

    assert_contract_error(result, FlashLoanError::InvalidFlashloanRepay as u32);
}

#[test]
fn test_flash_loan_callback_failure_rolls_back_pool_state() {
    let t = TestSetup::new();
    let client = t.client();
    enable_flashloan(&t);
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
    enable_flashloan(&t);
    let receiver = Address::generate(&t.env);

    let result = flatten_contract_result(client.try_flash_loan(
        &hub(&t.asset),
        &t.admin,
        &receiver,
        &200_000_000_000_000i128,
        &Bytes::new(&t.env),
    ));
    assert_contract_error(result, CollateralError::InsufficientLiquidity as u32);
}

#[test]
fn test_create_strategy_rejects_zero_amount() {
    let t = TestSetup::new();
    let client = t.client();
    let caller = Address::generate(&t.env);

    let result =
        flatten_contract_result(client.try_create_strategy(&caller, &t.action(0, 0i128), &true));
    assert_contract_error(result, GenericError::AmountMustBePositive as u32);
}

#[test]
fn test_create_strategy_rejects_insufficient_liquidity() {
    let t = TestSetup::new();
    let client = t.client();
    let caller = Address::generate(&t.env);

    let result = flatten_contract_result(client.try_create_strategy(
        &caller,
        &t.action(0, 200_000_000_000_000i128),
        &true,
    ));
    assert_contract_error(result, CollateralError::InsufficientLiquidity as u32);
}

/// Min-unit principal with a positive fee bps charges fee=1 and pays out nothing.
#[test]
fn test_create_strategy_dust_fee_consumes_entire_payout() {
    let t = TestSetup::new();
    let client = t.client();
    enable_flashloan(&t); // flashloan_fee = 100 bps → min fee 1 on amount 1
    let caller = Address::generate(&t.env);
    let tok = token::Client::new(&t.env, &t.asset);
    client.supply(&t.sup(0, 10_000_000_000i128));

    let cash_before = client.get_reserves(&hub(&t.asset));
    let caller_before = tok.balance(&caller);
    let state_before = t.state_snapshot();

    let result = client.create_strategy(&caller, &t.action(0, 1i128), &true);

    assert_eq!(result.actual_amount, 1, "gross principal is the min unit");
    assert_eq!(
        result.amount_received, 0,
        "min fee of 1 consumes the entire dust payout"
    );
    assert_eq!(
        tok.balance(&caller),
        caller_before,
        "receiver gets no tokens when fee equals principal"
    );
    assert_eq!(
        client.get_reserves(&hub(&t.asset)),
        cash_before,
        "cash debit is amount-fee (=0); fee stays as cash"
    );
    let state_after = t.state_snapshot();
    assert!(
        state_after.borrowed > state_before.borrowed,
        "gross principal still mints debt shares"
    );
    assert!(
        state_after.revenue >= state_before.revenue,
        "fee books as protocol revenue when share mint is non-zero"
    );
    assert_eq!(
        state_after.cash, state_before.cash,
        "net cash unchanged: no outbound transfer and fee remains booked"
    );
}

#[test]
fn test_create_strategy_above_max_utilization_panics() {
    let t = TestSetup::new();
    let client = t.client();
    set_max_utilization(&t, RAY / 2);
    // 1000 tokens supplied (7 decimals).
    client.supply(&t.sup(0, 10_000_000_000i128));
    let caller = Address::generate(&t.env);
    // 600 tokens → 60% util > 50% max.
    let result = flatten_contract_result(client.try_create_strategy(
        &caller,
        &t.action(0, 6_000_000_000i128),
        &false,
    ));
    assert_contract_error(result, CollateralError::UtilizationAboveMax as u32);
}

#[test]
fn test_create_strategy_near_max_utilization_succeeds() {
    let t = TestSetup::new();
    let client = t.client();
    set_max_utilization(&t, RAY / 2);
    client.supply(&t.sup(0, 10_000_000_000i128));
    let caller = Address::generate(&t.env);
    // 400 tokens → 40% util < 50% max.
    let result = client.create_strategy(&caller, &t.action(0, 4_000_000_000i128), &false);
    assert_eq!(result.actual_amount, 4_000_000_000i128);
    assert_eq!(result.amount_received, 4_000_000_000i128);
    assert!(result.position.scaled_amount > 0);
    let util = client.get_utilisation(&hub(&t.asset));
    assert!(
        util <= RAY / 2,
        "post-strategy util must stay at or under the cap"
    );
}

/// Util gate runs on gross debt mint before fee supply shares; a high fee cannot
/// bypass max utilization.
#[test]
fn test_create_strategy_fee_cannot_bypass_max_utilization() {
    let t = TestSetup::new();
    let client = t.client();
    enable_flashloan(&t);
    set_max_utilization(&t, RAY / 2);
    t.env.as_contract(&t.pool, || {
        let key = PoolKey::Params(hub(&t.asset));
        let mut params: MarketParamsRaw = t.env.storage().persistent().get(&key).unwrap();
        params.flashloan_fee = 5_000; // 50% fee — still mints full gross debt
        t.env.storage().persistent().set(&key, &params);
    });
    client.supply(&t.sup(0, 10_000_000_000i128));
    let caller = Address::generate(&t.env);
    // Gross 60% util fails even though net cash out is only 30% of supply.
    let result = flatten_contract_result(client.try_create_strategy(
        &caller,
        &t.action(0, 6_000_000_000i128),
        &true,
    ));
    assert_contract_error(result, CollateralError::UtilizationAboveMax as u32);
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

    client.update_indexes(&vec![&t.env, hub(&t.asset)]);
    let idx_before = client.get_sync_data(&hub(&t.asset)).state;

    client.seize_positions(&t.sez(AccountPositionType::Borrow, &updated_borrow.position));

    client.update_indexes(&vec![&t.env, hub(&t.asset)]);
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
    assert_contract_error(result, GenericError::MathOverflow as u32);
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

fn market_with_revenue_at_sixty_percent(t: &TestSetup) -> i128 {
    t.edit_state(|state| {
        state.revenue = state.supplied * 6 / 10;
    });
    41_0000000i128
}

#[test]
fn test_withdraw_rejects_burn_that_strands_revenue_above_supply() {
    let t = TestSetup::new();
    let client = t.client();

    let position = client.supply(&t.sup(0, 100_0000000i128)).get_unchecked(0);
    let over_backing = market_with_revenue_at_sixty_percent(&t);
    let user = Address::generate(&t.env);

    let result = flatten_contract_result(client.try_withdraw(
        &user,
        &false,
        &t.wdr(position.position.scaled_amount, over_backing, 0),
    ));
    assert_contract_error(result, GenericError::InternalError as u32);
}

#[test]
fn test_withdraw_liquidation_leg_rejects_burn_that_strands_revenue_above_supply() {
    let t = TestSetup::new();
    let client = t.client();

    let position = client.supply(&t.sup(0, 100_0000000i128)).get_unchecked(0);
    let over_backing = market_with_revenue_at_sixty_percent(&t);
    let user = Address::generate(&t.env);

    let result = flatten_contract_result(client.try_withdraw(
        &user,
        &true,
        &t.wdr(position.position.scaled_amount, over_backing, 0),
    ));
    assert_contract_error(result, GenericError::InternalError as u32);
}

#[test]
fn test_withdraw_allows_burn_down_to_exactly_the_revenue_slice() {
    let t = TestSetup::new();
    let client = t.client();

    let position = client.supply(&t.sup(0, 100_0000000i128)).get_unchecked(0);
    market_with_revenue_at_sixty_percent(&t);
    let user = Address::generate(&t.env);

    client.withdraw(
        &user,
        &false,
        &t.wdr(position.position.scaled_amount, 40_0000000i128, 0),
    );

    let state = t.state_snapshot();
    assert_eq!(
        state.supplied, state.revenue,
        "supply burned down to the revenue slice stays exactly backed"
    );
}

#[test]
fn test_net_settle_rejects_burn_that_strands_revenue_above_supply() {
    let t = TestSetup::new();
    let client = t.client();

    let supply_position = client.supply(&t.sup(0, 100_0000000i128)).get_unchecked(0);
    let borrower = Address::generate(&t.env);
    let debt_position = client
        .borrow(&borrower, &t.bor(0, 50_0000000i128))
        .get_unchecked(0);
    let over_backing = market_with_revenue_at_sixty_percent(&t);

    let entry = PoolNetSettleEntry {
        hub_asset: hub(&t.asset),
        amount: over_backing,
        supply_position: supply_position.position.clone(),
        debt_position: debt_position.position.clone(),
    };

    let result = flatten_contract_result(client.try_net_settle(&entry));
    assert_contract_error(result, GenericError::InternalError as u32);
}

#[test]
fn test_seize_positions_rejects_negative_scaled_amount() {
    let t = TestSetup::new();
    let client = t.client();
    client.supply(&t.sup(0, 100_0000000i128));

    let negative = ScaledPositionRaw { scaled_amount: -1 };
    for side in [AccountPositionType::Deposit, AccountPositionType::Borrow] {
        let result = flatten_contract_result(client.try_seize_positions(&t.sez(side, &negative)));
        assert_contract_error(result, GenericError::AmountMustBePositive as u32);
    }
}

#[test]
fn test_net_settle_rejects_negative_scaled_positions() {
    let t = TestSetup::new();
    let client = t.client();
    client.supply(&t.sup(0, 100_0000000i128));
    let borrower = Address::generate(&t.env);
    let debt = client
        .borrow(&borrower, &t.bor(0, 10_0000000i128))
        .get_unchecked(0);

    let good_supply = ScaledPositionRaw {
        scaled_amount: 1_000,
    };
    let good_debt = debt.position.clone();
    let neg = ScaledPositionRaw { scaled_amount: -1 };

    let bad_supply = PoolNetSettleEntry {
        hub_asset: hub(&t.asset),
        amount: 1_0000000i128,
        supply_position: neg.clone(),
        debt_position: good_debt.clone(),
    };
    assert_contract_error(
        flatten_contract_result(client.try_net_settle(&bad_supply)),
        GenericError::AmountMustBePositive as u32,
    );

    let bad_debt = PoolNetSettleEntry {
        hub_asset: hub(&t.asset),
        amount: 1_0000000i128,
        supply_position: good_supply,
        debt_position: neg,
    };
    assert_contract_error(
        flatten_contract_result(client.try_net_settle(&bad_debt)),
        GenericError::AmountMustBePositive as u32,
    );
}

#[test]
fn test_seize_positions_rejects_deposit_above_market_supply_shares() {
    let t = TestSetup::new();
    let client = t.client();

    client.supply(&t.sup(0, 100_0000000i128));

    let oversized = ScaledPositionRaw {
        scaled_amount: t.state_snapshot().supplied + 1,
    };

    let result = flatten_contract_result(
        client.try_seize_positions(&t.sez(AccountPositionType::Deposit, &oversized)),
    );
    assert_contract_error(result, GenericError::InternalError as u32);
}

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

    t.advance_time(31_556_926);

    client.update_indexes(&vec![&t.env, hub(&t.asset)]);

    let revenue = client.get_revenue(&hub(&t.asset));
    assert!(
        revenue > 0,
        "year-long accrual at positive utilization must mint claimable revenue"
    );

    let tok = token::Client::new(&t.env, &t.asset);
    let admin_balance_before = tok.balance(&t.admin);
    let claimed = client.claim_revenue(&hub(&t.asset)).actual_amount;
    let admin_balance_after = tok.balance(&t.admin);

    assert!(claimed > 0, "claim_revenue must transfer a positive amount");
    assert_eq!(
        admin_balance_after - admin_balance_before,
        claimed,
        "admin balance delta must match claimed amount"
    );
}

#[test]
fn test_claim_revenue_handles_partial_claim_when_reserves_are_lower_than_revenue() {
    let t = TestSetup::new();
    let client = t.client();

    let oversized_supply = client
        .supply(&t.sup(0, 200_000_000_000_000i128))
        .get_unchecked(0);
    client.seize_positions(&t.sez(AccountPositionType::Deposit, &oversized_supply.position));

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

/// Fix (pool claim policy, F8 and the CR-2 sibling), flipped pin: a claim whose
/// burn would push utilization past `max_utilization` used to revert with
/// `UtilizationAboveMax` and fail the controller's whole claim batch. It is
/// still refused whole, but by paying nothing: a payout above the claim
/// utilization room (`min(max_utilization, 95%)`) pays zero and moves nothing,
/// and one within it pays and leaves utilization under the ceiling.
#[test]
fn test_claim_revenue_pays_nothing_when_the_burn_would_breach_max_utilization() {
    let t = TestSetup::new();
    let client = t.client();
    let key = hub(&t.asset);
    let set_max_util = |max_utilization: i128| {
        t.env.as_contract(&t.pool, || {
            let key = PoolKey::Params(hub(&t.asset));
            let mut params: MarketParamsRaw = t.env.storage().persistent().get(&key).unwrap();
            params.max_utilization = max_utilization;
            t.env.storage().persistent().set(&key, &params);
        });
    };

    // A 95% ceiling at 90% utilization: paying the 8 tokens of cash above the
    // buffer would leave 90 / 92 = 97.8%.
    set_max_util(RAY * 95 / 100);
    t.edit_state(|state| {
        state.supplied = 100 * RAY;
        state.borrowed = 90 * RAY;
        state.revenue = 10 * RAY;
        state.cash = 10_0000000i128;
    });
    let before = t.state_snapshot();
    assert_eq!(client.claim_revenue(&key).actual_amount, 0);
    assert_pool_state_eq(&t.state_snapshot(), &before);
    assert_eq!(t.state_snapshot().cash, before.cash);

    // A 92% ceiling binds below the 95% claim bound: the room is
    // floor(100 - ceil(90 / 0.92)) tokens = 2.1739130 less two units. A
    // 4-token claim exceeds it and pays nothing; a 2-token claim fits.
    set_max_util(RAY * 92 / 100);
    t.edit_state(|state| state.revenue = 4 * RAY);
    assert_eq!(client.claim_revenue(&key).actual_amount, 0);
    t.edit_state(|state| state.revenue = 2 * RAY);
    assert_eq!(client.claim_revenue(&key).actual_amount, 2_0000000);
    let s = t.state_snapshot();
    assert_eq!((s.cash, s.revenue, s.supplied), (8_0000000, 0, 98 * RAY));
    assert!(client.get_utilisation(&key) <= RAY * 92 / 100);
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
    assert_contract_error(result, GenericError::MathOverflow as u32);
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
        is_flashloanable: false,
        flashloan_fee: 0,
    };
    let result = flatten_contract_result(client.try_update_params(&hub(&t.asset), &model));
    assert_contract_error(result, CollateralError::InvalidUtilRange as u32);
}

#[test]
fn test_update_params_mutates_flash_config() {
    let t = TestSetup::new();
    let client = t.client();

    let before = client.get_sync_data(&hub(&t.asset)).params;
    assert!(!before.is_flashloanable);

    let model = InterestRateModel {
        max_borrow_rate: 2 * RAY,
        base_borrow_rate: RAY / 100,
        slope1: RAY / 10,
        slope2: RAY / 5,
        slope3: RAY,
        mid_utilization: RAY / 2,
        optimal_utilization: RAY * 8 / 10,
        max_utilization: RAY * 95 / 100,
        reserve_factor: 1000,
        is_flashloanable: true,
        flashloan_fee: 300,
    };
    client.update_params(&hub(&t.asset), &model);

    let after = client.get_sync_data(&hub(&t.asset)).params;
    assert!(after.is_flashloanable);
    assert_eq!(after.flashloan_fee, 300);
}

#[test]
fn test_update_params_rejects_flashloan_fee_above_cap() {
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
        reserve_factor: 1000,
        is_flashloanable: true,
        flashloan_fee: 501,
    };
    let result = flatten_contract_result(client.try_update_params(&hub(&t.asset), &model));
    assert_contract_error(result, CollateralError::InvalidBorrowParams as u32);
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
        is_flashloanable: false,
        flashloan_fee: 0,
    };
    let result = flatten_contract_result(client.try_update_params(&hub(&t.asset), &model));
    assert_contract_error(result, CollateralError::OptUtilTooHigh as u32);
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
        is_flashloanable: false,
        flashloan_fee: 0,
    };
    let result = flatten_contract_result(client.try_update_params(&hub(&t.asset), &model));
    assert_contract_error(result, CollateralError::InvalidReserveFactor as u32);
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
        is_flashloanable: false,
        flashloan_fee: 0,
    };
    let result = flatten_contract_result(client.try_update_params(&hub(&t.asset), &model));
    assert_contract_error(result, CollateralError::BaseRateNegative as u32);
}

#[test]
fn test_update_params_rejects_max_rate_not_above_base_rate() {
    let t = TestSetup::new();
    let client = t.client();

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
        is_flashloanable: false,
        flashloan_fee: 0,
    };
    let result = flatten_contract_result(client.try_update_params(&hub(&t.asset), &model));
    assert_contract_error(result, CollateralError::MaxRateBelowBase as u32);
}

#[test]
fn test_update_params_rejects_max_borrow_rate_above_cap() {
    let t = TestSetup::new();
    let client = t.client();

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
        is_flashloanable: false,
        flashloan_fee: 0,
    };
    let result = flatten_contract_result(client.try_update_params(&hub(&t.asset), &model));
    assert_contract_error(result, CollateralError::MaxBorrowRateTooHigh as u32);
}

#[test]
fn test_views() {
    let t = TestSetup::new();
    let client = t.client();

    let util = client.get_utilisation(&hub(&t.asset));
    assert_eq!(util, 0, "utilization should be zero initially");

    client.supply(&t.sup(0, 10_000_000_000i128));

    assert_eq!(
        client.get_supplied_amount(&hub(&t.asset)),
        10_000_000_000,
        "supplied amount should round-trip at the initial unit index"
    );

    let reserves = client.get_reserves(&hub(&t.asset));
    assert!(reserves > 0, "reserves should be positive");

    let borrower = Address::generate(&t.env);
    client.borrow(&borrower, &t.bor(0, 100_0000000i128));

    assert_eq!(
        client.get_borrowed_amount(&hub(&t.asset)),
        100_0000000,
        "borrowed amount should round-trip at the initial unit index"
    );

    let util_after = client.get_utilisation(&hub(&t.asset));
    assert_eq!(
        util_after,
        RAY / 10,
        "1e9 borrowed against 1e10 supplied is 10% utilization"
    );

    // Region 1: 1% + (10%/50%)*4% = 1.8% borrow APR.
    // Deposit APR = 10% * 1.8% * 90% = 0.162%.
    assert_annual_apr_views(
        &client,
        &t.asset,
        RAY / 10,
        RAY * 18 / 1_000,
        RAY * 162 / 100_000,
    );
    assert!(
        client.get_revenue(&hub(&t.asset)) >= 0,
        "protocol revenue view should be callable"
    );
    t.advance_time(60);
    assert_eq!(
        client.get_delta_time(&hub(&t.asset)),
        60_000,
        "delta time should report elapsed milliseconds"
    );
}

#[test]
fn test_rate_views_match_annual_apr_from_utilization() {
    // market_params: base 1%, slope1 4% to 50% mid, slope2 10% to 80% opt,
    // slope3 80% to 100%, cap 100%, reserve factor 10%.
    let t = TestSetup::new();
    let client = t.client();
    let borrower = Address::generate(&t.env);

    assert_annual_apr_views(&client, &t.asset, 0, RAY / 100, 0);

    client.supply(&t.sup(0, 10_000_000_000i128));
    assert_annual_apr_views(&client, &t.asset, 0, RAY / 100, 0);

    // 10% util, region 1: 1% + (10/50)*4% = 1.8%. Deposit = 10%*1.8%*90% = 0.162%.
    client.borrow(&borrower, &t.bor(0, 1_000_000_000i128));
    assert_annual_apr_views(
        &client,
        &t.asset,
        RAY / 10,
        RAY * 18 / 1_000,
        RAY * 162 / 100_000,
    );

    // 25% util, region 1: 1% + (25/50)*4% = 3%. Deposit = 25%*3%*90% = 0.675%.
    client.borrow(&borrower, &t.bor(0, 1_500_000_000i128));
    assert_annual_apr_views(
        &client,
        &t.asset,
        RAY * 25 / 100,
        RAY * 3 / 100,
        RAY * 675 / 100_000,
    );

    // 50% util, kink: base+slope1 = 5%. Deposit = 50%*5%*90% = 2.25%.
    client.borrow(&borrower, &t.bor(0, 2_500_000_000i128));
    assert_annual_apr_views(
        &client,
        &t.asset,
        RAY * 50 / 100,
        RAY * 5 / 100,
        RAY * 225 / 10_000,
    );

    // 65% util, region 2: 5% + (15/30)*10% = 10%. Deposit = 65%*10%*90% = 5.85%.
    client.borrow(&borrower, &t.bor(0, 1_500_000_000i128));
    assert_annual_apr_views(
        &client,
        &t.asset,
        RAY * 65 / 100,
        RAY / 10,
        RAY * 585 / 10_000,
    );

    // 80% util, kink: base+slope1+slope2 = 15%. Deposit = 80%*15%*90% = 10.8%.
    client.borrow(&borrower, &t.bor(0, 1_500_000_000i128));
    assert_annual_apr_views(
        &client,
        &t.asset,
        RAY * 80 / 100,
        RAY * 15 / 100,
        RAY * 108 / 1_000,
    );

    // 90% util, region 3: 15% + (10/20)*80% = 55%. Deposit = 90%*55%*90% = 44.55%.
    client.borrow(&borrower, &t.bor(0, 1_000_000_000i128));
    assert_annual_apr_views(
        &client,
        &t.asset,
        RAY * 90 / 100,
        RAY * 55 / 100,
        RAY * 4_455 / 10_000,
    );
}

#[test]
fn test_upgrade_rejects_unknown_wasm_hash() {
    let t = TestSetup::new();
    let missing_hash = BytesN::from_array(&t.env, &[0xA5; 32]);

    assert!(
        t.client().try_upgrade(&missing_hash).is_err(),
        "upgrade must invoke the host and reject an unknown Wasm hash"
    );
}

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

#[test]
fn test_ray_checked_sub_happy_path() {
    let env = Env::default();
    let a = Ray::from(5 * RAY);
    let b = Ray::from(2 * RAY);
    assert_eq!(a.checked_sub(&env, b), Ray::from(3 * RAY));
}

#[test]
fn test_repay_partial_amount() {
    let t = TestSetup::new();
    let client = t.client();

    client.supply(&t.sup(0, 50_000_000_000i128));

    let borrower = Address::generate(&t.env);
    let updated_borrow = client
        .borrow(&borrower, &t.bor(0, 100_0000000i128))
        .get_unchecked(0);

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
fn test_create_strategy_emits_position_and_transfers_net() {
    let t = TestSetup::new();
    let client = t.client();

    client.supply(&t.sup(0, 50_000_000_000i128));

    t.env.as_contract(&t.pool, || {
        let key = PoolKey::Params(hub(&t.asset));
        let mut params: MarketParamsRaw = t.env.storage().persistent().get(&key).unwrap();
        params.flashloan_fee = 100;
        t.env.storage().persistent().set(&key, &params);
    });

    let caller = Address::generate(&t.env);
    let tok = token::Client::new(&t.env, &t.asset);
    let caller_before = tok.balance(&caller);
    let revenue_before = client.get_revenue(&hub(&t.asset));

    let amount = 100_0000000i128;

    let fee = 1_0000000i128;
    let result = client.create_strategy(&caller, &t.action(0, amount), &true);
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

#[test]
fn test_create_strategy_fee_free_when_charge_fee_false() {
    let t = TestSetup::new();
    let client = t.client();

    client.supply(&t.sup(0, 50_000_000_000i128));

    t.env.as_contract(&t.pool, || {
        let key = PoolKey::Params(hub(&t.asset));
        let mut params: MarketParamsRaw = t.env.storage().persistent().get(&key).unwrap();
        params.flashloan_fee = 100;
        t.env.storage().persistent().set(&key, &params);
    });

    let caller = Address::generate(&t.env);
    let tok = token::Client::new(&t.env, &t.asset);
    let caller_before = tok.balance(&caller);
    let revenue_before = client.get_revenue(&hub(&t.asset));

    let amount = 100_0000000i128;
    let result = client.create_strategy(&caller, &t.action(0, amount), &false);

    assert_eq!(result.actual_amount, amount);
    assert_eq!(result.amount_received, amount, "fee-free: full amount");
    assert_eq!(
        tok.balance(&caller) - caller_before,
        amount,
        "caller receives the full amount"
    );
    assert_eq!(
        client.get_revenue(&hub(&t.asset)),
        revenue_before,
        "no fee accrues when charge_fee is false"
    );
}

#[test]
fn test_claim_revenue_zero_revenue_early_returns() {
    let t = TestSetup::new();
    let client = t.client();
    let transfers_before = count_first_topic(&t.env.events().all(), "transfer");

    let claimed = client.claim_revenue(&hub(&t.asset)).actual_amount;
    assert_eq!(claimed, 0, "claim_revenue should return 0 when no revenue");
    assert_eq!(
        count_first_topic(&t.env.events().all(), "transfer"),
        transfers_before,
        "a zero claim must not invoke a token transfer"
    );
}

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
        is_flashloanable: false,
        flashloan_fee: 0,
    };
    client.update_params(&hub(&t.asset), &model);

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

    client.supply(&t.sup(0, 10_000_000_000i128));
    let borrower = Address::generate(&t.env);
    let borrowed = client
        .borrow(&borrower, &t.bor(0, 100_0000000i128))
        .get_unchecked(0);
    assert_eq!(borrowed.actual_amount, 100_0000000i128);
    assert!(
        borrowed.position.scaled_amount > 0,
        "borrow under updated params must mint debt shares"
    );
}

/// `update_params` accrues elapsed time under the old rate curve and commits the
/// indexes before it writes the new model.
#[test]
fn test_update_params_accrues_under_old_curve_after_time_advance() {
    let t = TestSetup::new();
    let client = t.client();

    // Positive utilization so borrow rate depends on the rate-curve params.
    client.supply(&t.sup(0, 10_000_000_000i128));
    let borrower = Address::generate(&t.env);
    client.borrow(&borrower, &t.bor(0, 5_000_000_000i128));

    let pre = client.get_sync_data(&hub(&t.asset));
    assert_eq!(
        pre.params.base_borrow_rate,
        RAY / 100,
        "fixture starts on market_params base rate"
    );
    assert!(
        pre.state.borrowed > 0 && pre.state.supplied > 0,
        "need both supply and debt so accrual moves indexes"
    );

    t.advance_time(86_400);
    let now_ms = t.env.ledger().timestamp() * MS_PER_SECOND;

    let expected_old = MarketIndexRaw::from(&simulate_update_indexes(&t.env, now_ms, &pre));
    assert!(
        expected_old.borrow_index > pre.state.borrow_index,
        "old-curve simulation must grow the borrow index over a day"
    );

    // Counterfactual: same state + elapsed time but with a much cheaper curve.
    // If replace_rate_model wrongly accrued under the new model, indexes would land here.
    let mut pre_with_new = pre.clone();
    pre_with_new.params.base_borrow_rate = 0;
    pre_with_new.params.slope1 = RAY / 1000;
    pre_with_new.params.slope2 = RAY / 100;
    pre_with_new.params.slope3 = RAY / 10;
    pre_with_new.params.max_borrow_rate = 2 * RAY;
    let expected_if_new_curve =
        MarketIndexRaw::from(&simulate_update_indexes(&t.env, now_ms, &pre_with_new));
    assert_ne!(
        expected_old.borrow_index, expected_if_new_curve.borrow_index,
        "test requires the two curves to diverge over the same window"
    );

    let new_model = InterestRateModel {
        max_borrow_rate: 2 * RAY,
        base_borrow_rate: 0,
        slope1: RAY / 1000,
        slope2: RAY / 100,
        slope3: RAY / 10,
        mid_utilization: RAY / 2,
        optimal_utilization: RAY * 8 / 10,
        max_utilization: RAY * 95 / 100,
        reserve_factor: 1000,
        is_flashloanable: false,
        flashloan_fee: 0,
    };
    client.update_params(&hub(&t.asset), &new_model);

    let after = client.get_sync_data(&hub(&t.asset));
    assert_eq!(
        after.state.borrow_index, expected_old.borrow_index,
        "committed borrow index must match accrual under the pre-update curve"
    );
    assert_eq!(
        after.state.supply_index, expected_old.supply_index,
        "committed supply index must match accrual under the pre-update curve"
    );
    assert_eq!(
        after.state.last_timestamp, now_ms,
        "update_params must stamp accrual through the current ledger"
    );
    assert_eq!(
        after.params.base_borrow_rate, 0,
        "params must switch to the new model after the old-curve commit"
    );
    assert_ne!(
        after.state.borrow_index, expected_if_new_curve.borrow_index,
        "indexes must not match new-curve simulation for the same window"
    );

    // Further time under the new curve must follow the post-update sync data.
    t.advance_time(172_800);
    let later_ms = t.env.ledger().timestamp() * MS_PER_SECOND;
    let expected_new_window =
        MarketIndexRaw::from(&simulate_update_indexes(&t.env, later_ms, &after));
    client.update_indexes(&vec![&t.env, hub(&t.asset)]);
    let final_state = client.get_sync_data(&hub(&t.asset));
    assert_eq!(
        final_state.state.borrow_index, expected_new_window.borrow_index,
        "post-swap accrual must use the new curve"
    );
    assert_eq!(
        final_state.state.supply_index, expected_new_window.supply_index,
        "post-swap supply index must use the new curve"
    );
}

#[test]
fn test_update_params_rejects_invalid_slope_ordering() {
    let t = TestSetup::new();
    let client = t.client();

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
        is_flashloanable: false,
        flashloan_fee: 0,
    };
    let result = flatten_contract_result(client.try_update_params(&hub(&t.asset), &model));
    assert_contract_error(result, CollateralError::SlopeNonMonotonic as u32);
}

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
        is_flashloanable: false,
        flashloan_fee: 0,
    };
    let result = flatten_contract_result(client.try_update_params(&hub(&t.asset), &model));
    assert_contract_error(result, CollateralError::InvalidUtilRange as u32);
}

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
        is_flashloanable: false,
        flashloan_fee: 0,
    };
    let result = flatten_contract_result(client.try_update_params(&hub(&t.asset), &model));
    assert_contract_error(result, CollateralError::InvalidReserveFactor as u32);
}

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
    assert_contract_error(result, CollateralError::BaseRateNegative as u32);
}

#[test]
fn test_create_market_rejects_duplicate_asset() {
    let t = TestSetup::new();
    let client = t.client();

    let result = flatten_contract_result(client.try_create_market(&0u32, &market_params(&t.asset)));
    assert_contract_error(result, GenericError::AssetAlreadySupported as u32);
}

#[test]
fn test_supply_rejects_unknown_market() {
    let t = TestSetup::new();
    let client = t.client();

    let unknown_asset = Address::generate(&t.env);
    let result =
        flatten_contract_result(client.try_supply(&t.sup_for(&unknown_asset, 0, 1_0000000i128)));
    assert_contract_error(result, GenericError::PoolNotInitialized as u32);
}

#[test]
fn test_create_market_initializes_state() {
    let t = TestSetup::new();
    let client = t.client();

    let asset_b = Address::generate(&t.env);
    client.create_market(&0u32, &market_params(&asset_b));

    let sync = client.get_sync_data(&hub(&asset_b));
    assert_eq!(
        sync.state.supply_index, RAY,
        "supply index must start at RAY"
    );
    assert_eq!(
        sync.state.borrow_index, RAY,
        "borrow index must start at RAY"
    );
    assert_eq!(
        sync.state.last_timestamp,
        t.env.ledger().timestamp() * MS_PER_SECOND,
        "last_timestamp must be ledger time in milliseconds"
    );
    assert_eq!(sync.state.supplied, 0);
    assert_eq!(sync.state.borrowed, 0);
    assert_eq!(sync.state.revenue, 0);
    assert_eq!(sync.state.cash, 0);
    assert_eq!(sync.params.asset_id, asset_b);
}

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
    assert!(
        a_after_supply.supplied > a_before.supplied,
        "market A supplied must increase after supply"
    );
    assert_eq!(a_after_supply.cash, a_before.cash + supply_amount);

    let b_after_supply = t.state_of(&asset_b);
    assert_pool_state_eq(&b_after_supply, &b_initial);
    assert_eq!(b_after_supply.cash, b_initial.cash);

    let borrower = Address::generate(&t.env);
    let borrow_amount = 100_0000000i128;
    client.borrow(&borrower, &t.bor(0, borrow_amount));

    let a_after_borrow = t.state_snapshot();
    assert!(
        a_after_borrow.borrowed > a_after_supply.borrowed,
        "market A borrowed must increase after borrow"
    );
    assert_eq!(a_after_borrow.cash, a_after_supply.cash - borrow_amount);

    let b_after_borrow = t.state_of(&asset_b);
    assert_pool_state_eq(&b_after_borrow, &b_initial);
    assert_eq!(b_after_borrow.cash, b_initial.cash);
}

#[test]
fn test_create_market_rejects_non_owner() {
    let t = TestSetup::new();
    let client = t.client();
    let asset_b = Address::generate(&t.env);
    let no_auths: [SorobanAuthorizationEntry; 0] = [];

    let result = client
        .set_auths(&no_auths)
        .try_create_market(&0u32, &market_params(&asset_b));

    assert!(
        result.is_err(),
        "create_market without owner auth must fail"
    );
}

#[test]
fn test_bulk_get_indexes_matches_per_asset() {
    let t = TestSetup::new();
    let client = t.client();

    client.supply(&t.sup(0, 10_000_000_000i128));
    let borrower = Address::generate(&t.env);
    client.borrow(&borrower, &t.bor(0, 5_000_000_000i128));

    t.advance_time(86_400);

    let assets = vec![&t.env, hub(&t.asset)];
    let bulk = client.get_bulk_indexes(&assets);
    assert_eq!(bulk.len(), 1, "one entry per requested asset");

    let now_ms = t.env.ledger().timestamp() * MS_PER_SECOND;
    let reference = MarketIndexRaw::from(&simulate_update_indexes(
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

#[test]
fn test_bulk_get_indexes_multi_asset_alignment() {
    let t = TestSetup::new();
    let client = t.client();

    let asset_b = Address::generate(&t.env);
    client.create_market(&0u32, &market_params(&asset_b));

    client.supply(&t.sup(0, 10_000_000_000i128));
    let borrower = Address::generate(&t.env);
    client.borrow(&borrower, &t.bor(0, 5_000_000_000i128));

    t.advance_time(86_400);

    let assets = vec![&t.env, hub(&t.asset), hub(&asset_b)];
    let bulk = client.get_bulk_indexes(&assets);
    assert_eq!(bulk.len(), 2);

    let a = bulk.get_unchecked(0);
    let b = bulk.get_unchecked(1);

    assert!(a.borrow_index > b.borrow_index && b.borrow_index > RAY);
    assert!(a.supply_index > RAY);
    assert_eq!(b.supply_index, RAY, "no borrows, no supplier rewards");

    let now_ms = t.env.ledger().timestamp() * MS_PER_SECOND;
    let ref_a = MarketIndexRaw::from(&simulate_update_indexes(
        &t.env,
        now_ms,
        &client.get_sync_data(&hub(&t.asset)),
    ));
    let ref_b = MarketIndexRaw::from(&simulate_update_indexes(
        &t.env,
        now_ms,
        &client.get_sync_data(&hub(&asset_b)),
    ));
    assert_eq!(a, ref_a, "entry 0 matches market A");
    assert_eq!(b, ref_b, "entry 1 matches market B");
}

#[test]
fn test_bulk_get_indexes_empty_request() {
    let t = TestSetup::new();
    let bulk = t.client().get_bulk_indexes(&Vec::new(&t.env));
    assert_eq!(bulk.len(), 0);
}

#[test]
fn test_bulk_get_indexes_unknown_asset_panics() {
    let t = TestSetup::new();
    let unknown = Address::generate(&t.env);
    let assets = vec![&t.env, hub(&unknown)];
    let result = flatten_contract_result(t.client().try_get_bulk_indexes(&assets));
    assert_contract_error(result, GenericError::PoolNotInitialized as u32);
}

#[test]
fn test_bulk_supply_two_markets_matches_sequential_singles() {
    let t = TestSetup::new();
    let client = t.client();

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

    let a_state = t.state_snapshot();
    let c_state = t.state_of(&asset_c);
    assert_pool_state_eq(&a_state, &c_state);
    assert_eq!(a_state.cash, c_state.cash);

    let b_state = t.state_of(&asset_b);
    let d_state = t.state_of(&asset_d);
    assert_pool_state_eq(&b_state, &d_state);
    assert_eq!(b_state.cash, d_state.cash);
}

#[test]
fn test_bulk_repay_overpayment_refunds_second_entry_surplus() {
    let t = TestSetup::new();
    let client = t.client();

    client.supply(&t.sup(0, 50_000_000_000i128));

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

fn set_max_utilization(t: &TestSetup, max_utilization: i128) {
    t.env.as_contract(&t.pool, || {
        let key = PoolKey::Params(hub(&t.asset));
        let mut params: MarketParamsRaw = t.env.storage().persistent().get(&key).unwrap();
        params.max_utilization = max_utilization;
        t.env.storage().persistent().set(&key, &params);
    });
}

#[test]
fn test_withdraw_above_max_utilization_panics_but_within_cap_succeeds() {
    let t = TestSetup::new();
    let client = t.client();
    set_max_utilization(&t, RAY / 2);

    let supply_amount = 20_000_000_000i128;
    let supplied = client.supply(&t.sup(0, supply_amount)).get_unchecked(0);

    let borrower = Address::generate(&t.env);
    client.borrow(&borrower, &t.bor(0, 5_000_000_000i128));

    let supplier = Address::generate(&t.env);
    let scaled = supplied.position.scaled_amount;

    let ok = client
        .withdraw(&supplier, &false, &t.wdr(scaled, 5_000_000_000i128, 0i128))
        .get_unchecked(0);
    assert_eq!(ok.actual_amount, 5_000_000_000i128);

    let result = flatten_contract_result(client.try_withdraw(
        &supplier,
        &false,
        &t.wdr(ok.position.scaled_amount, 6_000_000_000i128, 0i128),
    ));
    assert_contract_error(result, CollateralError::UtilizationAboveMax as u32);
}

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

#[test]
fn test_revenue_conversion_floor_never_exceeds_entitlement_but_half_up_does() {
    let t = TestSetup::new();
    let client = t.client();
    let borrower = Address::generate(&t.env);

    client.supply(&t.sup(0, 10_000_000_000i128));
    client.borrow(&borrower, &t.bor(0, 4_000_000_000i128));
    t.advance_time(1);
    client.update_indexes(&vec![&t.env, hub(&t.asset)]);

    let state = t.state_snapshot();
    let entitlement_ray =
        common::math::fp_core::mul_div_floor(&t.env, state.revenue, state.supply_index, RAY);

    let (half_up, floored) = t.env.as_contract(&t.pool, || {
        let cache = Cache::load(&t.env, &hub(&t.asset));
        (
            cache.unscale_supply(cache.revenue()),
            cache.unscale_supply_floor(cache.revenue()),
        )
    });

    assert!(half_up * WAD_PER_RAW > entitlement_ray);
    assert!(floored * WAD_PER_RAW <= entitlement_ray);
    assert!(half_up > floored);
}

#[test]
fn test_claims_never_outpay_burned_shares_where_half_up_would() {
    let t = TestSetup::new();
    let client = t.client();
    let borrower = Address::generate(&t.env);

    client.supply(&t.sup(0, 10_000_000_000i128));
    client.borrow(&borrower, &t.bor(0, 4_000_000_000i128));

    let mut total_paid = 0i128;
    let mut total_half_up_would_pay = 0i128;

    for second in 1..=10u64 {
        t.advance_time(second);

        client.update_indexes(&vec![&t.env, hub(&t.asset)]);

        let half_up = t.env.as_contract(&t.pool, || {
            let cache = Cache::load(&t.env, &hub(&t.asset));
            cache.unscale_supply(cache.revenue())
        });

        let before = t.state_snapshot();
        let paid = client.claim_revenue(&hub(&t.asset)).actual_amount;
        let after = t.state_snapshot();

        let burned_ray = common::math::fp_core::mul_div_floor(
            &t.env,
            before.revenue - after.revenue,
            before.supply_index,
            RAY,
        );

        assert!(paid * WAD_PER_RAW <= burned_ray);

        total_paid += paid;
        total_half_up_would_pay += half_up;
    }

    assert!(total_half_up_would_pay > total_paid);
}

#[test]
fn test_revenue_claim_pays_out_once_entitlement_clears_one_raw_unit() {
    let t = TestSetup::new();
    let client = t.client();
    let borrower = Address::generate(&t.env);

    client.supply(&t.sup(0, 10_000_000_000i128));
    client.borrow(&borrower, &t.bor(0, 4_000_000_000i128));
    t.advance_time(86_400);
    client.update_indexes(&vec![&t.env, hub(&t.asset)]);

    let before = t.state_snapshot();
    let owed_ray =
        common::math::fp_core::mul_div_floor(&t.env, before.revenue, before.supply_index, RAY);
    assert!(owed_ray > WAD_PER_RAW);

    let paid = client.claim_revenue(&hub(&t.asset)).actual_amount;
    let after = t.state_snapshot();

    assert!(paid > 0);
    assert_eq!(after.revenue, 0);
    assert!(paid * WAD_PER_RAW <= owed_ray);
}

#[test]
fn test_load_sync_data_renews_market_keys_once() {
    let t = TestSetup::new();

    t.env.cost_estimate().budget().reset_default();
    t.env.as_contract(&t.pool, || {
        crate::storage::renew_market(&t.env, &hub(&t.asset));
    });
    let one_renewal = t.env.cost_estimate().budget().cpu_instruction_cost();

    t.env.cost_estimate().budget().reset_default();
    t.env.as_contract(&t.pool, || {
        crate::storage::renew_market(&t.env, &hub(&t.asset));
        crate::storage::renew_market(&t.env, &hub(&t.asset));
    });
    let two_renewals = t.env.cost_estimate().budget().cpu_instruction_cost();

    let redundant = two_renewals - one_renewal;
    assert!(
        redundant > 0,
        "second renew still costs CPU when TTL already fresh"
    );

    t.env.cost_estimate().budget().reset_default();
    t.env.as_contract(&t.pool, || {
        let _ = crate::storage::load_sync_data(&t.env, &hub(&t.asset));
    });
    let sync_cpu = t.env.cost_estimate().budget().cpu_instruction_cost();

    t.env.cost_estimate().budget().reset_default();
    t.env.as_contract(&t.pool, || {
        let _ = crate::storage::read_params(&t.env, &hub(&t.asset));
        let _ = crate::storage::read_state(&t.env, &hub(&t.asset));
        crate::storage::renew_market(&t.env, &hub(&t.asset));
        crate::storage::renew_market(&t.env, &hub(&t.asset));
    });
    let double_renew_cpu = t.env.cost_estimate().budget().cpu_instruction_cost();

    assert!(
        sync_cpu + redundant / 2 < double_renew_cpu,
        "load_sync_data should renew once, not twice; sync_cpu={sync_cpu} double_renew_cpu={double_renew_cpu} redundant={redundant}"
    );
}

#[test]
fn test_bad_debt_wipeout_leaves_market_usable_at_realistic_scale() {
    let t = TestSetup::new();
    let client = t.client();

    client.supply(&t.sup(0, 10_000_000_000i128));

    t.env.as_contract(&t.pool, || {
        let mut cache = Cache::load(&t.env, &hub(&t.asset));
        let total_supplied_value = cache.supplied().mul(&t.env, cache.supply_index());
        crate::interest::apply_bad_debt_to_supply_index(&mut cache, total_supplied_value);
        cache.commit();
    });

    let floored = t.state_snapshot().supply_index;
    assert_eq!(floored, common::constants::SUPPLY_INDEX_FLOOR_RAW);
    assert_eq!(RAY / floored, 1_000);

    let opened = client.supply(&t.sup(0, 10_000_000_000_000i128));
    assert!(opened.get(0).unwrap().position.scaled_amount > 0);
}

fn backing_snapshot(t: &TestSetup) -> (i128, i128, i128, i128, i128, i128) {
    t.env.as_contract(&t.pool, || {
        let cache = Cache::load(&t.env, &hub(&t.asset));
        (
            cache.supply_index().raw(),
            cache.supplied().raw(),
            cache.borrowed().raw(),
            cache.cash(),
            cache.unscale_supply_floor(cache.supplied()),
            crate::guards::backing_shortfall(&cache),
        )
    })
}

/// Live seize wipeout → index floor → blocked supply → recap → new supply is safe.
#[test]
fn test_floor_wipeout_blocks_supply_until_recap_then_new_deposit_is_safe() {
    let t = TestSetup::new();
    let client = t.client();
    let token_admin = token::StellarAssetClient::new(&t.env, &t.asset);
    let token = token::Client::new(&t.env, &t.asset);

    let alice_shares = 1_000 * RAY;
    t.edit_state(|state| {
        state.supplied = alice_shares;
        state.borrowed = alice_shares;
        state.revenue = 0;
        state.supply_index = RAY;
        state.borrow_index = RAY;
        state.cash = 0;
    });

    client.seize_positions(&t.sez(
        AccountPositionType::Borrow,
        &ScaledPositionRaw {
            scaled_amount: alice_shares,
        },
    ));

    let (index, supplied, borrowed, cash, claim, shortfall) = backing_snapshot(&t);

    assert_eq!(index, common::constants::SUPPLY_INDEX_FLOOR_RAW);
    assert_eq!(supplied, alice_shares);
    assert_eq!(borrowed, 0);
    assert_eq!(cash, 0);
    assert_eq!(claim, 10_000_000, "1000 tokens at index 0.001 → 1 token");
    assert_eq!(shortfall, claim);

    let blocked = flatten_contract_result(client.try_supply(&t.sup(0, 50_000_000)));
    assert_contract_error(blocked, CollateralError::PoolInsolvent as u32);

    let payer = Address::generate(&t.env);
    let offered = 25_000_000i128;
    token_admin.mint(&payer, &offered);
    token.transfer(&payer, &t.pool, &offered);
    let recap = client.recapitalize(&hub(&t.asset), &payer, &offered);
    let (index2, _, _, cash2, _claim2, shortfall2) = backing_snapshot(&t);

    assert_eq!(recap.actual_amount, shortfall);
    assert_eq!(token.balance(&payer), offered - shortfall);
    assert_eq!(cash2, shortfall);
    assert_eq!(shortfall2, 0);
    assert_eq!(index2, common::constants::SUPPLY_INDEX_FLOOR_RAW);

    let bob_deposit = 50_000_000i128;
    let bob = client.supply(&t.sup(0, bob_deposit)).get_unchecked(0);
    let (_, _, _, cash3, _claim3, shortfall3) = backing_snapshot(&t);
    assert_eq!(bob.actual_amount, bob_deposit);
    assert_eq!(cash3, shortfall + bob_deposit);
    assert_eq!(shortfall3, 0);

    let alice_recv = Address::generate(&t.env);
    let alice_out = client
        .withdraw(&alice_recv, &false, &t.wdr(alice_shares, i128::MAX, 0))
        .get_unchecked(0);
    let (_, _, _, cash4, claim4, _) = backing_snapshot(&t);

    assert_eq!(alice_out.actual_amount, shortfall);
    assert_eq!(token.balance(&alice_recv), shortfall);
    assert_eq!(
        cash4, bob_deposit,
        "alice can take the recap, not bob's fresh deposit"
    );
    assert_eq!(claim4, bob_deposit);

    let bob_recv = Address::generate(&t.env);
    let bob_out = client
        .withdraw(
            &bob_recv,
            &false,
            &t.wdr(bob.position.scaled_amount, i128::MAX, 0),
        )
        .get_unchecked(0);
    assert_eq!(bob_out.actual_amount, bob_deposit);
    assert_eq!(token.balance(&bob_recv), bob_deposit);
}

/// `prepare_with_balance` exists only under `cfg(any(test, feature = "certora"))`.
/// The Certora flash-loan specs prove properties over its result, so these tests
/// pin that it builds the same terms `apply` builds from the live SAC balance.
fn flash_terms_setup() -> (TestSetup, i128, i128) {
    let t = TestSetup::new();
    enable_flashloan(&t);
    t.client().supply(&t.sup(0, 10_000_000_000i128));
    // enable_flashloan sets flashloan_fee to 100 bps.
    (t, 100_0000000i128, 500_0000000i128)
}

#[test]
fn prepare_with_balance_derives_terms_from_the_supplied_balance() {
    let (t, amount, pre_balance) = flash_terms_setup();

    t.env.as_contract(&t.pool, || {
        let (cache, got) = flash::prepare_with_balance(&t.env, hub(&t.asset), amount, pre_balance);

        assert_eq!(
            cache.params().flashloan_fee,
            100,
            "fee_bps read from market"
        );
        // Computed independently of the implementation rather than by calling
        // `terms` again, which would only prove the function calls itself.
        assert_eq!(got.fee, amount / 100, "100 bps of principal");
        assert_eq!(got.total_repayment, amount + amount / 100);
        assert_eq!(
            got.balance_after_payout,
            pre_balance - amount,
            "the payout leaves the pool short by exactly the principal"
        );
        assert_eq!(
            got.balance_after_repayment,
            pre_balance + amount / 100,
            "the round trip leaves the pool up by exactly the fee"
        );
    });
}

#[test]
fn prepare_with_balance_agrees_with_the_terms_apply_would_build() {
    let (t, amount, pre_balance) = flash_terms_setup();

    t.env.as_contract(&t.pool, || {
        let (cache, composed) =
            flash::prepare_with_balance(&t.env, hub(&t.asset), amount, pre_balance);
        // The same two calls `apply` makes, in the same order, against the same
        // market state. Any divergence means the Certora specs and the
        // production path have drifted apart.
        let direct = flash::terms(&t.env, amount, cache.params().flashloan_fee, pre_balance);

        assert_eq!(composed.fee, direct.fee);
        assert_eq!(composed.total_repayment, direct.total_repayment);
        assert_eq!(composed.balance_after_payout, direct.balance_after_payout);
        assert_eq!(
            composed.balance_after_repayment,
            direct.balance_after_repayment
        );
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #401)")]
fn prepare_with_balance_still_runs_the_flashloan_gate() {
    let t = TestSetup::new();
    // Deliberately not enabling flash loans: the symbolic entry point must not
    // be a way around the gate `apply` enforces.
    t.client().supply(&t.sup(0, 10_000_000_000i128));

    t.env.as_contract(&t.pool, || {
        flash::prepare_with_balance(&t.env, hub(&t.asset), 100_0000000i128, 500_0000000i128);
    });
}

// ---------------------------------------------------------------------------
// The pool bounds the recapitalize refund by neither the cash book nor a
// balance check of its own. It trusts its owner's measured amount by design
// (INV-AUTH-01).
// ---------------------------------------------------------------------------

/// `recapitalize` pays `amount - applied` out through `Cache::transfer_out`,
/// which checks neither the cash book nor the live SAC balance and does not
/// debit `cash`. On a market with no backing shortfall `applied` is zero, so
/// the entire declared `amount` is refunded out of tokens the pool never
/// received.
///
/// Every pool mutator is `#[only_owner]`, so reaching this needs the controller
/// to pass an `amount` it did not measure; the controller's
/// `markets::recapitalize` measures the receipt. The gap is defence in depth,
/// not a reachable loss. This test pins that the pool adds no bound of its
/// own: only the owner's declared amount limits the payout.
#[test]
fn test_recapitalize_refund_pays_out_tokens_the_pool_never_received() {
    let t = TestSetup::new();
    let token = token::Client::new(&t.env, &t.asset);
    let payer = Address::generate(&t.env);

    // `TestSetup::new` mints the pool's custody and sets `cash` to match, so
    // the book and the real balance agree before the call.
    let custody_before = token.balance(&t.pool);
    let before = t.state_snapshot();
    assert_eq!(
        before.cash, custody_before,
        "fixture guard: cash book and SAC balance must start in sync"
    );
    let shortfall = t.env.as_contract(&t.pool, || {
        crate::guards::backing_shortfall(&Cache::load(&t.env, &hub(&t.asset)))
    });
    assert_eq!(shortfall, 0, "fixture must be fully backed");

    // Nothing is transferred in; the owner declares a payment on the payer's behalf.
    let applied = t
        .client()
        .recapitalize(&hub(&t.asset), &payer, &custody_before)
        .actual_amount;
    assert_eq!(
        applied, 0,
        "a backed market has no shortfall, so nothing is applied"
    );
    assert_unfunded_refund_drained_custody(&t, &payer, custody_before, &before);
}

/// Shared tail of the unfunded-refund tests: the declared amount came back to
/// the payer, custody is gone, and the cash book did not move.
fn assert_unfunded_refund_drained_custody(
    t: &TestSetup,
    payer: &Address,
    declared: i128,
    before: &PoolStateRaw,
) {
    let token = token::Client::new(&t.env, &t.asset);
    let after = t.state_snapshot();
    assert_eq!(
        token.balance(payer),
        declared,
        "the payer is refunded in full for a payment that never happened"
    );
    assert_eq!(
        token.balance(&t.pool),
        0,
        "the pool's entire custody has left the contract"
    );
    assert_eq!(
        after.cash, before.cash,
        "the cash book is untouched, so the pool still reports the paid-out \
         funds as present"
    );
    assert_pool_state_eq(&after, before);
}

/// After an unfunded refund, `cash` overstates custody. `Cache::require_reserves`
/// reads the book, not the balance, so it admits exits that then fail inside
/// the SAC transfer. The market reports itself solvent and cannot pay.
#[test]
fn test_unfunded_recapitalize_leaves_the_cash_book_overstating_custody() {
    let t = TestSetup::new();
    let token = token::Client::new(&t.env, &t.asset);
    let token_admin = token::StellarAssetClient::new(&t.env, &t.asset);
    let payer = Address::generate(&t.env);
    let receiver = Address::generate(&t.env);

    // A legitimate supplier, with custody moved in as the controller would.
    let deposit = 10_000_000_000i128;
    token_admin.mint(&t.pool, &deposit);
    let supplied = t
        .client()
        .supply(&t.sup(0, deposit))
        .get_unchecked(0)
        .position
        .scaled_amount;
    assert_eq!(
        t.state_snapshot().cash,
        token.balance(&t.pool),
        "fixture guard: book and custody must still agree after the deposit"
    );

    // Drain every token via an unfunded refund.
    let custody = token.balance(&t.pool);
    t.client().recapitalize(&hub(&t.asset), &payer, &custody);
    assert_eq!(token.balance(&t.pool), 0, "custody is gone");

    let book = t.state_snapshot().cash;
    assert!(
        book >= deposit,
        "the book still claims more than the supplier's deposit: {book}"
    );

    // The supplier's exit clears the pool's own liquidity guard -- the book
    // says the cash is there -- and then fails in the token transfer.
    let outcome = t
        .client()
        .try_withdraw(&receiver, &false, &t.wdr(supplied, i128::MAX, 0));
    assert!(
        outcome.is_err(),
        "the withdraw cannot be paid, so it must fail"
    );
    // The failure must come from custody, not from a pool guard. The SAC
    // reports insufficient balance as its own contract error `BalanceError = 10`.
    // The asserts check that exact code and that neither pool liquidity guard
    // fired: `require_reserves` read the book and let the exit through.
    const SAC_BALANCE_ERROR: u32 = 10;
    match outcome {
        Err(Ok(err)) => {
            assert_ne!(
                err,
                Error::from_contract_error(CollateralError::InsufficientLiquidity as u32),
                "the pool's own liquidity guard must NOT be what stopped this -- \
                 it reads the cash book, which still shows the funds"
            );
            assert_ne!(
                err,
                Error::from_contract_error(CollateralError::PoolInsolvent as u32),
                "the pool's solvency guard must NOT be what stopped this either"
            );
            assert_eq!(
                err,
                Error::from_contract_error(SAC_BALANCE_ERROR),
                "the exit must fail inside the SAC transfer for want of custody"
            );
        }
        Err(Err(host_abort)) => panic!("expected a SAC contract error, got {host_abort:?}"),
        Ok(_) => unreachable!("asserted is_err above"),
    }
    assert_eq!(token.balance(&receiver), 0, "the supplier received nothing");
    assert_eq!(
        t.state_snapshot().cash,
        book,
        "the failed exit rolled back, so the book still overstates custody"
    );
}

/// Live custody bounds the payout, not the declared amount: claiming more than
/// the pool holds reverts inside the SAC. This caps an unmeasured owner call at
/// the pool's real holdings of that asset.
#[test]
fn test_unfunded_recapitalize_is_bounded_by_custody_not_by_the_declared_amount() {
    let t = TestSetup::new();
    let token = token::Client::new(&t.env, &t.asset);
    let payer = Address::generate(&t.env);

    let custody = token.balance(&t.pool);
    let before = t.state_snapshot();

    let outcome = t
        .client()
        .try_recapitalize(&hub(&t.asset), &payer, &(custody + 1));

    assert!(
        outcome.is_err(),
        "over-claiming past custody must not succeed"
    );
    assert_eq!(
        token.balance(&t.pool),
        custody,
        "a reverted over-claim leaves custody intact"
    );
    assert_eq!(token.balance(&payer), 0);
    assert_eq!(t.state_snapshot().cash, before.cash);
}

/// The asset-substitution payload: pay in a worthless token, be refunded in a
/// real one. The controller's `markets::recapitalize` moves `hub_asset.asset`
/// inbound while `Cache::transfer_out` refunds in `params.asset_id`, so if those
/// two could name different tokens the refund would be a swap.
///
/// They cannot. `ops::market::create` derives the storage key from
/// `params.asset_id`, so `key.asset == params.asset_id` holds for every market,
/// and a `hub_asset` naming any other token resolves to no market. This test
/// pins both halves: the guarantee is structural, and a refactor that passes
/// the key and the params separately would lose it.
#[test]
fn test_recapitalize_rejects_a_hub_asset_that_names_no_market() {
    let t = TestSetup::new();
    let payer = Address::generate(&t.env);

    // A second, unrelated token the payer controls outright.
    let worthless = t
        .env
        .register_stellar_asset_contract_v2(payer.clone())
        .address()
        .clone();
    let worthless_admin = token::StellarAssetClient::new(&t.env, &worthless);
    let real = token::Client::new(&t.env, &t.asset);

    let stake = 1_000_000_000i128;
    worthless_admin.mint(&payer, &stake);
    let pool_custody_before = real.balance(&t.pool);

    // Declaring the foreign token as the market key resolves to no market.
    let outcome = flatten_contract_result(t.client().try_recapitalize(
        &hub(&worthless),
        &payer,
        &stake,
    ));
    assert_contract_error(outcome, GenericError::PoolNotInitialized as u32);

    assert_eq!(
        real.balance(&t.pool),
        pool_custody_before,
        "no real custody may move for a payload naming another token"
    );
    assert_eq!(
        real.balance(&payer),
        0,
        "the payer must not be refunded in the market's asset"
    );

    // The structural half: the market's key and its configured asset are the
    // same address, which is what makes the substitution unreachable.
    t.env.as_contract(&t.pool, || {
        let key = hub(&t.asset);
        let params: MarketParamsRaw = t
            .env
            .storage()
            .persistent()
            .get(&PoolKey::Params(key.clone()))
            .expect("market must exist");
        assert_eq!(
            params.asset_id, key.asset,
            "the market key's asset and params.asset_id must be the same token"
        );
    });
}

/// The refund gap is not specific to `recapitalize`. Two pool legs refund an
/// excess derived from a declared inbound amount, not from the cash book:
/// `ops::recapitalize::apply` (the excess over the shortfall) and
/// `ops::repay::apply` (the excess over the debt). Neither refund debits `cash`
/// or passes `require_reserves`.
///
/// A repay against a market with no debt makes the entire declared amount an
/// overpayment: `current_debt_ceil` is zero, so `resolve_repay` takes the
/// full-close branch and `net_repay` is zero. The `RepayRoundsToZeroShares`
/// assert in `ops::repay::accounting` passes on its `net_repay == 0` disjunct,
/// and the whole amount is refunded out of real custody with the book untouched.
#[test]
fn test_unfunded_repay_overpayment_refund_also_pays_out_of_custody() {
    let t = TestSetup::new();
    let token = token::Client::new(&t.env, &t.asset);
    let payer = Address::generate(&t.env);

    let custody_before = token.balance(&t.pool);
    let before = t.state_snapshot();
    assert_eq!(
        before.cash, custody_before,
        "fixture guard: book and custody must start in sync"
    );
    assert_eq!(before.borrowed, 0, "fixture must carry no debt");

    // Nothing transferred in, no debt to retire: the whole amount is "excess".
    let credited = t
        .client()
        .repay(&payer, &t.ract(0, custody_before))
        .get_unchecked(0)
        .actual_amount;
    assert_eq!(credited, 0, "no debt was retired, so nothing is credited");
    assert_unfunded_refund_drained_custody(&t, &payer, custody_before, &before);
}

// ---------------------------------------------------------------------------
// Revenue claim policy (F8 and the CR-2 siblings) and negative scaled
// positions (F5).
// ---------------------------------------------------------------------------

/// F5: a negative scaled position is rejected with the same typed error on
/// every leg, entries and exits alike. A position of -5 with a one-unit amount
/// used to book `1e27 - 5` shares on supply, borrow and create_strategy, and
/// exits failed with `MathOverflow` or `RepayRoundsToZeroShares`.
#[test]
fn test_negative_scaled_position_is_rejected_on_every_leg() {
    let t = TestSetup::new();
    let client = t.client();
    client.supply(&t.sup(0, 100_0000000i128));
    let who = Address::generate(&t.env);
    let code = GenericError::AmountMustBePositive as u32;

    assert_contract_error(
        flatten_contract_result(client.try_supply(&t.sup(-5, 1))),
        code,
    );
    assert_contract_error(
        flatten_contract_result(client.try_borrow(&who, &t.bor(-5, 1))),
        code,
    );
    assert_contract_error(
        flatten_contract_result(client.try_create_strategy(&who, &t.action(-5, 1), &false)),
        code,
    );
    assert_contract_error(
        flatten_contract_result(client.try_withdraw(&who, &false, &t.wdr(-1, 1, 0))),
        code,
    );
    assert_contract_error(
        flatten_contract_result(client.try_repay(&who, &t.ract(-1, 1))),
        code,
    );
    assert_contract_error(
        flatten_contract_result(client.try_repay(&who, &t.ract(-RAY, 1))),
        code,
    );
}

/// F5: the rejection holds inside a batch (a valid first leg does not commit
/// when a later leg carries a negative position), on the zero-amount legs that
/// mint or burn nothing (which used to echo the negative position back), on
/// the full-close withdraw sentinel, and on a fee-charging strategy open.
#[test]
fn test_negative_position_cannot_slip_through_a_batch_or_a_zero_leg() {
    const UNIT: i128 = 10_000_000;
    let t = TestSetup::new();
    let client = t.client();
    let key = hub(&t.asset);
    let who = Address::generate(&t.env);
    let code = GenericError::AmountMustBePositive as u32;
    t.env.as_contract(&t.pool, || {
        let pkey = PoolKey::Params(key.clone());
        let mut params: MarketParamsRaw = t.env.storage().persistent().get(&pkey).unwrap();
        params.flashloan_fee = 100;
        t.env.storage().persistent().set(&pkey, &params);
    });
    client.supply(&t.sup(0, 1_000 * UNIT));
    let before = client.get_sync_data(&key).state;

    let valid = t.sup_entry(&t.asset, 0, UNIT);
    let negative = t.sup_entry(&t.asset, i128::MIN, UNIT);
    assert_contract_error(
        flatten_contract_result(client.try_supply(&vec![&t.env, valid, negative])),
        code,
    );
    assert_contract_error(
        flatten_contract_result(client.try_supply(&t.sup(-1, 0))),
        code,
    );
    let mut borrows = t.bor(0, UNIT);
    borrows.push_back(PoolBorrowEntry {
        action: t.action(-1, UNIT),
    });
    assert_contract_error(
        flatten_contract_result(client.try_borrow(&who, &borrows)),
        code,
    );
    assert_contract_error(
        flatten_contract_result(client.try_create_strategy(&who, &t.action(-1, UNIT), &true)),
        code,
    );
    assert_contract_error(
        flatten_contract_result(client.try_withdraw(&who, &true, &t.wdr(-1, i128::MAX, 0))),
        code,
    );
    assert_contract_error(
        flatten_contract_result(client.try_withdraw(&who, &false, &t.wdr(i128::MIN, 0, 0))),
        code,
    );
    assert_contract_error(
        flatten_contract_result(client.try_repay(&who, &t.ract(i128::MIN, 0))),
        code,
    );
    let after = client.get_sync_data(&key).state;
    let books = |s: &PoolStateRaw| {
        (
            s.supplied,
            s.borrowed,
            s.revenue,
            s.cash,
            s.supply_index,
            s.borrow_index,
        )
    };
    assert_eq!(
        books(&after),
        books(&before),
        "every refused call leaves the books unchanged"
    );
}

/// F8: while debt is outstanding a revenue claim draws only the cash above the
/// liquidation buffer, and pays nothing if that payout would raise utilization
/// past the 95% claim bound (`MAX_CLAIM_UTILIZATION_RAY`), here with the
/// market's own ceiling disabled; it never reverts on either. Once the debt is
/// gone neither bound applies.
#[test]
fn test_claim_revenue_leaves_the_liquidation_buffer_while_debt_is_outstanding() {
    let t = TestSetup::new();
    let client = t.client();
    let key = hub(&t.asset);
    // 90% utilization: 100 tokens supplied (10 of them revenue), 90 borrowed,
    // 10 tokens of cash. The buffer admits 8 tokens (2% of 100 held back),
    // but paying 8 would leave 90 / 92 = 97.8%: the claim utilization room is
    // floor(100 - ceil(90 / 0.95)) tokens = 5.2631578 less two units, so the
    // claim is refused whole and pays nothing.
    t.edit_state(|s| {
        s.supplied = 100 * RAY;
        s.borrowed = 90 * RAY;
        s.revenue = 10 * RAY;
        s.cash = 10_0000000;
    });
    let before = t.state_snapshot();
    assert_eq!(client.claim_revenue(&key).actual_amount, 0);
    assert_pool_state_eq(&t.state_snapshot(), &before);

    // 5 tokens of revenue fit the room: the claim pays all of it and burns
    // every revenue share, leaving 90 / 95 = 94.7%.
    t.edit_state(|s| s.revenue = 5 * RAY);
    assert_eq!(client.claim_revenue(&key).actual_amount, 5_0000000);
    let s = t.state_snapshot();
    assert_eq!((s.cash, s.revenue, s.supplied), (5_0000000, 0, 95 * RAY));
    assert!(client.get_utilisation(&key) <= RAY * 95 / 100);

    // 30% utilization, every share revenue: the buffer (2 tokens held back)
    // admits 68 tokens, within the 68.42-token room, so the claim pays them
    // and burns the pro-rata ceiling of revenue shares.
    t.edit_state(|s| {
        s.supplied = 100 * RAY;
        s.borrowed = 30 * RAY;
        s.revenue = 100 * RAY;
        s.cash = 70_0000000;
    });
    assert_eq!(client.claim_revenue(&key).actual_amount, 68_0000000);
    let s = t.state_snapshot();
    assert_eq!(
        (s.cash, s.revenue, s.supplied),
        (2_0000000, 32 * RAY, 32 * RAY)
    );

    // Debt gone: neither bound applies and the reserve pays out.
    t.edit_state(|s| s.borrowed = 0);
    assert_eq!(client.claim_revenue(&key).actual_amount, 2_0000000);
    assert_eq!(t.state_snapshot().cash, 0);
}

/// CR-2 sibling: a claim with nothing to pay on a market whose shares all left
/// while dust was borrowed returns zero instead of failing the controller's
/// batch with `PoolInsolvent`, and a claim never pays out the last supply
/// share under debt.
#[test]
fn test_claim_revenue_with_nothing_to_pay_skips_the_supply_for_debt_check() {
    let t = TestSetup::new();
    let client = t.client();
    let key = hub(&t.asset);
    t.edit_state(|s| {
        s.supplied = 0;
        s.revenue = 0;
        s.borrowed = 2 * (RAY / 10_000_000);
        s.cash = 0;
    });
    assert_eq!(client.claim_revenue(&key).actual_amount, 0);

    // Every remaining share is revenue and debt is open. Paying them out used
    // to be refused with `PoolInsolvent` because it would orphan the debt,
    // failing the batch. The whole floored revenue claim now always exceeds
    // the claim utilization room, which stops at least two units short of the
    // floored supply value, so the claim pays nothing and the shares stay
    // under the debt: with a whole unit of debt, and with one raw unit of debt
    // and a sub-unit supply remainder. The supply-for-debt check, still run on
    // every positive payout, is defence in depth for claims.
    t.edit_state(|s| {
        s.supplied = 5 * RAY;
        s.revenue = 5 * RAY;
        s.borrowed = RAY;
        s.cash = 100_0000000;
    });
    assert_eq!(client.claim_revenue(&key).actual_amount, 0);
    assert_eq!(t.state_snapshot().supplied, 5 * RAY);

    t.edit_state(|s| {
        s.supplied = 5 * RAY + RAY / 10_000_000 - 1;
        s.revenue = 5 * RAY + RAY / 10_000_000 - 1;
        s.borrowed = 1;
        s.cash = 100_0000000;
    });
    assert_eq!(client.claim_revenue(&key).actual_amount, 0);
    let s = t.state_snapshot();
    assert_eq!(
        s.supplied,
        5 * RAY + RAY / 10_000_000 - 1,
        "the shares stay"
    );
}

/// CR-2 sibling: a claim with nothing to pay on a market already past its
/// utilization ceiling only accrues and returns zero; it used to revert with
/// `UtilizationAboveMax` and take the controller's whole claim batch with it.
#[test]
fn test_claim_revenue_with_nothing_to_pay_skips_the_utilization_check() {
    let t = TestSetup::new();
    let client = t.client();
    let key = hub(&t.asset);
    t.env.as_contract(&t.pool, || {
        let key = PoolKey::Params(hub(&t.asset));
        let mut params: MarketParamsRaw = t.env.storage().persistent().get(&key).unwrap();
        params.max_utilization = RAY * 90 / 100;
        t.env.storage().persistent().set(&key, &params);
    });
    // 97% utilization, above the 90% ceiling, and revenue to claim, but the
    // claim bounds leave nothing payable.
    t.edit_state(|s| {
        s.supplied = 100 * RAY;
        s.borrowed = 97 * RAY;
        s.revenue = 10 * RAY;
        s.cash = 3_0000000;
    });
    let before = t.state_snapshot();
    assert_eq!(client.claim_revenue(&key).actual_amount, 0);
    let after = t.state_snapshot();
    assert_eq!(
        (after.cash, after.revenue, after.supplied, after.borrowed),
        (
            before.cash,
            before.revenue,
            before.supplied,
            before.borrowed
        )
    );
}

/// F8: under a 95% market ceiling, equal to the claim utilization bound, a
/// claim never trips the ceiling guard and never burns the last supply share
/// under debt, whatever the indexes, the utilization or the revenue share (400
/// pseudo-random books): a payout within the claim utilization room lands at
/// or under the ceiling. Without the two-unit burn margin the pro-rata ceiling
/// burn can land a raw unit past the 95% line and the guard refuses the claim,
/// and with it the controller's whole claim batch.
#[test]
fn test_bounded_claim_never_trips_a_95_percent_ceiling() {
    const SCALE: i128 = RAY / 10_000_000;
    let t = TestSetup::new();
    let client = t.client();
    let key = hub(&t.asset);
    let env = &t.env;
    t.env.as_contract(&t.pool, || {
        let key = PoolKey::Params(hub(&t.asset));
        let mut params: MarketParamsRaw = t.env.storage().persistent().get(&key).unwrap();
        params.max_utilization = RAY * 95 / 100;
        t.env.storage().persistent().set(&key, &params);
    });
    let max_util = Ray::from(RAY * 95 / 100);
    let mut seed: u64 = 0x5eed_f8f8;
    let mut next = |bound: i128| -> i128 {
        let mut draw = || {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            seed
        };
        let wide = (u128::from(draw()) << 64) | u128::from(draw());
        (wide % bound as u128) as i128
    };
    let mut paid_any = 0;
    for _ in 0..400 {
        let supply_index = RAY + next(3 * RAY);
        let borrow_index = RAY + next(5 * RAY);
        let supplied = 10 * RAY + next(990 * RAY);
        let revenue = supplied / 10 * 9;
        let util_bps = 5_000 + next(4_500);
        let supply_value = Ray::from(supplied).mul_floor(env, Ray::from(supply_index));
        let borrowed = supply_value.raw() / 10_000 * util_bps / borrow_index * RAY;
        let debt_value = Ray::from(borrowed).mul_ceil(env, Ray::from(borrow_index));
        // Cash that puts the buffer-limited payout right at the edge of the
        // room the 95% line leaves (no margin), then one to three units under.
        let reserve = (supply_value.raw() / SCALE * 200 + 9_999) / 10_000;
        let edge = (supply_value.raw() - debt_value.div_ceil(env, max_util).raw()) / SCALE;
        for under in 0..4 {
            t.edit_state(|s| {
                s.supplied = supplied;
                s.revenue = revenue;
                s.borrowed = borrowed;
                s.supply_index = supply_index;
                s.borrow_index = borrow_index;
                s.cash = reserve + edge - under;
            });
            let paid = client
                .try_claim_revenue(&key)
                .unwrap_or_else(|e| {
                    panic!("si={supply_index} bi={borrow_index} s={supplied} under={under}: {e:?}")
                })
                .unwrap()
                .actual_amount;
            paid_any += paid;
            let after = t.state_snapshot();
            assert!(after.supplied > 0, "the last shares stay under debt");
            let after_supply = Ray::from(after.supplied).mul_floor(env, Ray::from(supply_index));
            assert!(
                debt_value.div_ceil(env, after_supply) <= max_util,
                "si={supply_index} bi={borrow_index} s={supplied} under={under} paid={paid}"
            );
        }
    }
    assert!(paid_any > 0, "the sweep exercises positive payouts");
}

/// F8: the claim utilization room saturates. A debt valued near the market
/// value ceiling over a 50% market ceiling puts `debt_value / bound` past
/// `i128`; the room reads as empty and the claim pays nothing instead of
/// trapping with `MathOverflow` (which would fail the controller's whole claim
/// batch).
#[test]
fn test_claim_revenue_room_saturates_instead_of_trapping_on_a_low_ceiling() {
    use common::constants::MAX_MARKET_VALUE_RAY;

    let t = TestSetup::new();
    let client = t.client();
    let key = hub(&t.asset);
    t.env.as_contract(&t.pool, || {
        let key = PoolKey::Params(hub(&t.asset));
        let mut params: MarketParamsRaw = t.env.storage().persistent().get(&key).unwrap();
        params.max_utilization = RAY / 2;
        t.env.storage().persistent().set(&key, &params);
    });
    // Supply at the value ceiling, debt at 90% of it: 0.9 * MAX / 0.5 is past
    // `i128::MAX`.
    t.edit_state(|s| {
        s.supplied = MAX_MARKET_VALUE_RAY;
        s.borrowed = MAX_MARKET_VALUE_RAY / 10 * 9;
        s.revenue = 10 * RAY;
        s.cash = MAX_MARKET_VALUE_RAY / (RAY / 10_000_000) / 10;
    });
    let before = t.state_snapshot();
    assert_eq!(client.claim_revenue(&key).actual_amount, 0);
    let after = t.state_snapshot();
    assert_eq!(
        (after.cash, after.revenue, after.supplied),
        (before.cash, before.revenue, before.supplied)
    );
}

// ---------------------------------------------------------------------------
// Market value ceiling (CR-21) end to end: the fee guards in `flash::book_fee`
// and after the strategy fee in `strategy::accounting`, and the liveness of a
// capped market. Deleting either fee guard makes the matching test fail.
// ---------------------------------------------------------------------------

mod market_value_ceiling {
    use super::*;
    use common::constants::MAX_MARKET_VALUE_RAY;
    use common::math::fp_core::{mul_div_ceil, mul_div_floor_saturating};

    const UNIT7: i128 = 10_000_000;
    const YEAR_SECS: u64 = 31_556_926;

    fn set_params(t: &TestSetup, key: &HubAssetKey, edit: impl FnOnce(&mut MarketParamsRaw)) {
        t.env.as_contract(&t.pool, || {
            let pkey = PoolKey::Params(key.clone());
            let mut params: MarketParamsRaw = t.env.storage().persistent().get(&pkey).unwrap();
            edit(&mut params);
            t.env.storage().persistent().set(&pkey, &params);
        });
    }

    fn state_of(t: &TestSetup, key: &HubAssetKey) -> PoolStateRaw {
        t.client().get_sync_data(key).state
    }

    /// `(floor(supplied * si), floor(borrowed * bi))` in raw RAY, saturating.
    fn totals(t: &TestSetup, key: &HubAssetKey) -> (i128, i128) {
        let s = state_of(t, key);
        (
            mul_div_floor_saturating(&t.env, s.supplied, s.supply_index, RAY),
            mul_div_floor_saturating(&t.env, s.borrowed, s.borrow_index, RAY),
        )
    }

    fn act(key: &HubAssetKey, scaled_amount: i128, amount: i128) -> PoolAction {
        PoolAction {
            position: ScaledPositionRaw { scaled_amount },
            amount,
            hub_asset: key.clone(),
        }
    }

    fn ceiling_events(t: &TestSetup) -> usize {
        count_topic(&t.env.events().all(), "market", "value_ceiling")
    }

    /// Verify check pool-1. Repeated whole-cash flash loans at the 500 bps
    /// maximum fee on a market supplied at 90% of the ceiling:
    /// `flash::book_fee` refuses the loan whose fee would cross the ceiling
    /// (#33, nothing moves), the peak supplied value stays at or under it, and
    /// every later total-materialising op stays live. Without the guard every
    /// loan is admitted and the supplied value saturates at `i128::MAX`.
    #[test]
    fn flash_fee_bookings_cannot_push_supply_past_the_value_ceiling() {
        let t = TestSetup::new();
        let client = t.client();
        let key = hub(&t.asset);
        set_params(&t, &key, |p| {
            p.is_flashloanable = true;
            p.flashloan_fee = 500;
        });
        let extra_cash = MAX_MARKET_VALUE_RAY / 10 * 9 / (RAY / UNIT7);
        let token_admin = token::StellarAssetClient::new(&t.env, &t.asset);
        token_admin.mint(&t.pool, &extra_cash);
        t.edit_state(|s| {
            s.supply_index = RAY * 11 / 10;
            s.supplied = MAX_MARKET_VALUE_RAY / 11 * 9;
            s.cash += extra_cash;
        });
        // A real one-unit supply position to exit later.
        token_admin.mint(&t.pool, &UNIT7);
        let position = client
            .supply(&t.sup(0, UNIT7))
            .get_unchecked(0)
            .position
            .scaled_amount;
        assert!(totals(&t, &key).0 <= MAX_MARKET_VALUE_RAY);

        let tok = token::Client::new(&t.env, &t.asset);
        let receiver = t.env.register(PoolFlashLoanReceiver, ());
        let mut refusal = None;
        let mut peak = totals(&t, &key).0;
        for loan in 0..20 {
            let amount = state_of(&t, &key).cash;
            token_admin.mint(&receiver, &(amount / 20 + 1));
            let state_before = state_of(&t, &key);
            let pool_balance_before = tok.balance(&t.pool);
            let receiver_balance_before = tok.balance(&receiver);
            match flatten_contract_result(client.try_flash_loan(
                &key,
                &t.admin,
                &receiver,
                &amount,
                &Bytes::new(&t.env),
            )) {
                Ok(_) => peak = peak.max(totals(&t, &key).0),
                Err(err) => {
                    let state_after = state_of(&t, &key);
                    assert_eq!(
                        (state_after.supplied, state_after.revenue, state_after.cash),
                        (
                            state_before.supplied,
                            state_before.revenue,
                            state_before.cash
                        ),
                        "the refused loan books nothing"
                    );
                    assert_eq!(tok.balance(&t.pool), pool_balance_before);
                    assert_eq!(tok.balance(&receiver), receiver_balance_before);
                    refusal = Some((loan, err));
                    break;
                }
            }
        }
        assert!(
            peak <= MAX_MARKET_VALUE_RAY,
            "a flash fee pushed the supplied value past the ceiling: {peak}"
        );
        let (loan, err) = refusal.expect("the loan whose fee crosses the ceiling must be refused");
        assert!(loan == 1 || loan == 2, "refused at loan {loan}");
        assert_eq!(
            err,
            Error::from_contract_error(GenericError::MathOverflow as u32)
        );

        t.advance_time(86_400);
        let keys = vec![&t.env, key.clone()];
        assert!(flatten_contract_result(client.try_update_indexes(&keys)).is_ok());
        assert!(flatten_contract_result(client.try_get_utilisation(&key)).is_ok());
        assert!(flatten_contract_result(client.try_get_bulk_indexes(&keys)).is_ok());
        assert!(flatten_contract_result(client.try_get_supplied_amount(&key)).is_ok());
        let who = Address::generate(&t.env);
        let exit =
            flatten_contract_result(client.try_withdraw(&who, &false, &t.wdr(position, 1, 0)));
        assert!(exit.is_ok(), "one-unit withdraw: {exit:?}");
        let claim = flatten_contract_result(client.try_claim_revenue(&key));
        assert!(claim.is_ok(), "claim_revenue: {claim:?}");
        assert!(totals(&t, &key).0 <= MAX_MARKET_VALUE_RAY);
    }

    /// Verify check pool-2. A strategy fee that would push supply value past
    /// the ceiling is refused whole (#33, books unchanged); a smaller fee that
    /// fits is admitted and lands at or under the ceiling. Without the guard
    /// after the strategy fee the large open is admitted and the supplied
    /// value saturates at `i128::MAX`.
    #[test]
    fn strategy_fee_cannot_push_supply_past_the_value_ceiling() {
        let t = TestSetup::new();
        let client = t.client();
        let key = hub(&t.asset);
        set_params(&t, &key, |p| p.flashloan_fee = 500);
        let fill = MAX_MARKET_VALUE_RAY / (RAY / UNIT7);
        token::StellarAssetClient::new(&t.env, &t.asset).mint(&t.pool, &fill);
        t.edit_state(|s| {
            s.supplied = MAX_MARKET_VALUE_RAY - 10 * RAY;
            s.cash = fill;
        });
        let caller = Address::generate(&t.env);
        let before = state_of(&t, &key);

        // (a) 1,000 tokens at 5%: a 50-token fee against 10 tokens of room.
        let refused = flatten_contract_result(client.try_create_strategy(
            &caller,
            &t.action(0, 1_000 * UNIT7),
            &true,
        ));
        let after = state_of(&t, &key);
        assert_contract_error(refused, GenericError::MathOverflow as u32);
        assert_eq!(
            (after.supplied, after.borrowed, after.revenue, after.cash),
            (
                before.supplied,
                before.borrowed,
                before.revenue,
                before.cash
            ),
            "the refused strategy books nothing"
        );

        // (b) 100 tokens at 5%: a 5-token fee fits.
        let opened = client.create_strategy(&caller, &t.action(0, 100 * UNIT7), &true);
        assert_eq!(opened.actual_amount, 100 * UNIT7);
        let (supply_value, _) = totals(&t, &key);
        assert!(supply_value <= MAX_MARKET_VALUE_RAY);
    }

    /// Verify check pool-3a. Supply exactly at the ceiling, borrow down to the
    /// liquidation buffer, then a year of accrual at a 200% max rate. Debt
    /// value never passes the ceiling, the ceiling event fires, and every view
    /// and every risk-reducing op on the capped market stays live.
    fn capped_market_stays_live(decimals: u32) {
        let t = TestSetup::new();
        let client = t.client();
        let mut params = market_params(&t.asset);
        params.asset_decimals = decimals;
        params.max_borrow_rate = 2 * RAY;
        params.slope3 = 2 * RAY;
        params.max_utilization = RAY;
        params.is_flashloanable = true;
        params.flashloan_fee = 500;
        client.create_market(&1u32, &params);
        let key = HubAssetKey {
            hub_id: 1,
            asset: t.asset.clone(),
        };
        let unit = 10i128.pow(decimals);
        let scale = RAY / unit;
        let fill = MAX_MARKET_VALUE_RAY / scale;
        let token_admin = token::StellarAssetClient::new(&t.env, &t.asset);
        token_admin.mint(&t.pool, &fill);
        let supplier = client
            .supply(&vec![
                &t.env,
                PoolSupplyEntry {
                    action: act(&key, 0, fill),
                },
            ])
            .get_unchecked(0)
            .position
            .scaled_amount;
        assert_eq!(totals(&t, &key).0, fill * scale);

        let reserved = mul_div_ceil(&t.env, fill, 200, 10_000);
        let draw = fill - reserved;
        let borrower = Address::generate(&t.env);
        let debt = client
            .borrow(
                &borrower,
                &vec![
                    &t.env,
                    PoolBorrowEntry {
                        action: act(&key, 0, draw),
                    },
                ],
            )
            .get_unchecked(0)
            .position
            .scaled_amount;
        let one_more = flatten_contract_result(client.try_borrow(
            &borrower,
            &vec![
                &t.env,
                PoolBorrowEntry {
                    action: act(&key, debt, 1),
                },
            ],
        ));
        assert_contract_error(one_more, CollateralError::InsufficientLiquidity as u32);
        let (_, debt_at_mint) = totals(&t, &key);
        assert!(debt_at_mint <= MAX_MARKET_VALUE_RAY);

        t.advance_time(YEAR_SECS);
        let keys = vec![&t.env, key.clone()];
        client.update_indexes(&keys);
        let events = ceiling_events(&t);
        let (supply_value, debt_value) = totals(&t, &key);
        assert!(
            events >= 1,
            "MarketValueCeilingEvent fires on the capped sync"
        );
        assert!(supply_value <= MAX_MARKET_VALUE_RAY && debt_value <= MAX_MARKET_VALUE_RAY);

        assert!(flatten_contract_result(client.try_get_bulk_indexes(&keys)).is_ok());
        assert!(flatten_contract_result(client.try_get_sync_data(&key)).is_ok());
        assert!(flatten_contract_result(client.try_get_utilisation(&key)).is_ok());
        assert!(flatten_contract_result(client.try_get_supplied_amount(&key)).is_ok());
        assert!(flatten_contract_result(client.try_get_borrowed_amount(&key)).is_ok());

        let repaid =
            flatten_contract_result(client.try_repay(&borrower, &vec![&t.env, act(&key, debt, 1)]));
        assert!(repaid.is_ok(), "repay 1 unit: {repaid:?}");

        let liquidator = Address::generate(&t.env);
        let seized = flatten_contract_result(client.try_withdraw(
            &liquidator,
            &true,
            &vec![
                &t.env,
                PoolWithdrawEntry {
                    action: act(&key, supplier, 1_000 * unit),
                    protocol_fee: 10 * unit,
                },
            ],
        ));
        assert!(seized.is_ok(), "liquidation withdraw: {seized:?}");

        // 98% utilization sits above the 95% claim bound: the claim pays
        // nothing and does not revert.
        let claim = flatten_contract_result(client.try_claim_revenue(&key));
        assert!(claim.is_ok(), "claim_revenue: {claim:?}");

        let receiver = t.env.register(PoolFlashLoanReceiver, ());
        token_admin.mint(&receiver, &(50 * unit + 1));
        let flash = flatten_contract_result(client.try_flash_loan(
            &key,
            &t.admin,
            &receiver,
            &(1_000 * unit),
            &Bytes::new(&t.env),
        ));
        if let Err(err) = flash {
            assert_eq!(
                err,
                Error::from_contract_error(GenericError::MathOverflow as u32),
                "a flash loan on the capped market is either admitted or refused by the ceiling"
            );
        }
        let (supply_value, debt_value) = totals(&t, &key);
        assert!(supply_value <= MAX_MARKET_VALUE_RAY && debt_value <= MAX_MARKET_VALUE_RAY);
    }

    #[test]
    fn capped_market_stays_live_at_7_decimals() {
        capped_market_stays_live(7);
    }

    #[test]
    fn capped_market_stays_live_at_18_decimals() {
        capped_market_stays_live(18);
    }

    /// Verify check pool-4b. A stored total past `i128` (out of the market
    /// value domain) syncs and projects without trapping, leaves the indexes
    /// unchanged, and emits the ceiling event.
    #[test]
    fn a_stored_total_past_i128_syncs_and_projects_without_trapping() {
        let t = TestSetup::new();
        let client = t.client();
        client.create_market(&1u32, &market_params(&t.asset));
        let key = HubAssetKey {
            hub_id: 1,
            asset: t.asset.clone(),
        };
        let now_ms = t.env.ledger().timestamp() * 1_000;
        t.env.as_contract(&t.pool, || {
            t.env.storage().persistent().set(
                &PoolKey::State(key.clone()),
                &PoolStateRaw {
                    supplied: i128::MAX / 2,
                    borrowed: i128::MAX / 4,
                    revenue: 0,
                    borrow_index: 3 * RAY,
                    supply_index: 3 * RAY,
                    last_timestamp: now_ms,
                    cash: 0,
                },
            );
        });
        t.advance_time(86_400);
        let keys = vec![&t.env, key.clone()];
        let synced = flatten_contract_result(client.try_update_indexes(&keys));
        let events = ceiling_events(&t);
        assert!(synced.is_ok(), "update_indexes: {synced:?}");
        assert!(events >= 1, "the ceiling event fires");
        let projected = flatten_contract_result(client.try_get_bulk_indexes(&keys));
        assert!(projected.is_ok(), "get_bulk_indexes: {projected:?}");
        let index = projected.unwrap().get_unchecked(0);
        assert_eq!((index.borrow_index, index.supply_index), (3 * RAY, 3 * RAY));
        let s = state_of(&t, &key);
        assert_eq!((s.borrow_index, s.supply_index), (3 * RAY, 3 * RAY));
    }

    /// Verify check pool-5. Shared residual, pinned: supply parked at the
    /// ceiling switches borrower interest off, and `MarketValueCeilingEvent`
    /// makes it visible. The ceiling (`i128::MAX - RAY`) is not a multiple of
    /// one raw 7-decimal unit (1e20 raw RAY), so a supply parked at
    /// floor(MAX / 1e20) units leaves under one raw unit of headroom: a year's
    /// interest on 1M tokens accrues less than one raw unit. Interest resumes
    /// once the supplier withdraws.
    #[test]
    fn supply_parked_at_the_ceiling_switches_off_borrower_interest() {
        let t = TestSetup::new();
        let client = t.client();
        let key = hub(&t.asset);
        let fill = MAX_MARKET_VALUE_RAY / (RAY / UNIT7);
        token::StellarAssetClient::new(&t.env, &t.asset).mint(&t.pool, &fill);
        client.supply(&t.sup(0, fill));
        assert_eq!(totals(&t, &key).0, fill * (RAY / UNIT7));

        let borrower = Address::generate(&t.env);
        client.borrow(&borrower, &t.bor(0, 1_000_000 * UNIT7));
        t.advance_time(YEAR_SECS);
        client.update_indexes(&vec![&t.env, key.clone()]);
        let events = ceiling_events(&t);
        let parked = state_of(&t, &key);
        let debt_growth_ray =
            mul_div_floor_saturating(&t.env, parked.borrowed, parked.borrow_index, RAY)
                - mul_div_floor_saturating(&t.env, parked.borrowed, RAY, RAY);
        assert!(
            debt_growth_ray < RAY / UNIT7,
            "a year at >= 1% on 1M tokens of debt accrued less than one raw token unit"
        );
        assert!(
            events >= 1,
            "the ceiling event makes the parked market visible"
        );

        client.withdraw(
            &borrower,
            &false,
            &t.wdr(fill * (RAY / UNIT7), 1_000 * UNIT7, 0),
        );
        t.advance_time(2 * YEAR_SECS);
        client.update_indexes(&vec![&t.env, key.clone()]);
        assert!(state_of(&t, &key).borrow_index > RAY);
    }
}
