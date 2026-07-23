extern crate std;

use super::*;
use crate::test_support::hub;
use common::constants::{
    BPS, MS_PER_SECOND, RAY, SUPPLY_INDEX_REWARD_CEILING_RAY, TTL_BUMP_INSTANCE, TTL_BUMP_SHARED,
    TTL_THRESHOLD_INSTANCE, TTL_THRESHOLD_SHARED,
};
use common::errors::{CollateralError, FlashLoanError, GenericError};
use common::types::{MarketIndexRaw, ScaledPositionRaw};
use soroban_sdk::testutils::storage::{Instance as _, Persistent as _};
use soroban_sdk::testutils::{Address as _, ContractEvents, Events, Ledger, LedgerInfo};
use soroban_sdk::xdr::{ContractEventBody, ScVal, SorobanAuthorizationEntry};
use soroban_sdk::{contract, contractimpl, vec, Address, Bytes, Env, Error, InvokeError, Vec};

/// Ray-per-raw-unit for the 7-decimal test asset.
const WAD_PER_RAW: i128 = 100_000_000_000_000_000_000;

/// Opt-in diagnostics for claim-dust / TTL cost report tests (`true` to print).
const VERBOSE_CLAIM_DUST: bool = false;

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

fn flatten_contract_result<T, E: core::fmt::Debug>(
    result: Result<Result<T, E>, Result<Error, InvokeError>>,
) -> Result<T, Error> {
    match result {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(err)) => panic!("contract call succeeded but output conversion failed: {err:?}"),
        Err(invoke) => Err(invoke.expect("expected contract error, got host-level InvokeError")),
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

// `events().all()` retains only the last top-level invocation, not a cumulative total.
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

    client.update_indexes(&hub(&t.asset));
    assert_eq!(
        count_topic(&t.env.events().all(), "market", "batch_state_update"),
        1,
        "last invocation (update_indexes) should retain one state batch event"
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

    // Exact repay; no overpayment because no time has passed.
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
/// Enables flash loans on hub-0's market with a 1% (100 bps) fee. The pool
/// gates `is_flashloanable` and derives the fee from `flashloan_fee` bps, so
/// direct pool tests must configure the market first.
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
    let flash_fee = 1_0000000i128; // 1% of amount, from the configured 100 bps

    // The pool will send `amount`; pre-fund only the fee.
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

    // `require_positive_amount` reverts before the flashloanable/fee logic.
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
    // Default market leaves `is_flashloanable = false`; the pool gates it.
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
    assert_contract_error(result, CollateralError::UtilizationAboveMax as u32);
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
    assert!(
        util_after > 0,
        "utilization should be positive after borrow"
    );

    let deposit_rate = client.get_deposit_rate(&hub(&t.asset));
    let borrow_rate = client.get_borrow_rate(&hub(&t.asset));
    assert!(
        deposit_rate > 1,
        "active suppliers should earn a nonzero rate"
    );
    assert!(
        borrow_rate > deposit_rate,
        "borrow rate should exceed the supplier rate after reserve retention"
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
fn test_upgrade_rejects_unknown_wasm_hash() {
    let t = TestSetup::new();
    let missing_hash = BytesN::from_array(&t.env, &[0xA5; 32]);

    assert!(
        t.client().try_upgrade(&missing_hash).is_err(),
        "upgrade must invoke the host and reject an unknown Wasm hash"
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

    // Configure a 1% flash-loan fee; the pool derives the fee amount from bps.
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
    // 1% of amount, matching the configured `flashloan_fee` bps.
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

// `charge_fee = false` (migration) borrows fee-free even when the market has a
// configured flash-loan fee: the caller receives the full amount and no
// protocol revenue accrues.
#[test]
fn test_create_strategy_fee_free_when_charge_fee_false() {
    let t = TestSetup::new();
    let client = t.client();

    client.supply(&t.sup(0, 50_000_000_000i128));

    // A nonzero market fee that must be ignored when charge_fee is false.
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

// claim_revenue returns 0 when no revenue has accrued.
#[test]
fn test_claim_revenue_zero_revenue_early_returns() {
    let t = TestSetup::new();
    let client = t.client();
    let transfers_before = count_first_topic(&t.env.events().all(), "transfer");

    // No supply, no accrual; revenue is zero.
    let claimed = client.claim_revenue(&hub(&t.asset)).actual_amount;
    assert_eq!(claimed, 0, "claim_revenue should return 0 when no revenue");
    assert_eq!(
        count_first_topic(&t.env.events().all(), "transfer"),
        transfers_before,
        "a zero claim must not invoke a token transfer"
    );
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
        is_flashloanable: false,
        flashloan_fee: 0,
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

    // Updated params remain usable for supply/borrow after the round-trip.
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
        is_flashloanable: false,
        flashloan_fee: 0,
    };
    let result = flatten_contract_result(client.try_update_params(&hub(&t.asset), &model));
    assert_contract_error(result, CollateralError::SlopeNonMonotonic as u32);
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
        is_flashloanable: false,
        flashloan_fee: 0,
    };
    let result = flatten_contract_result(client.try_update_params(&hub(&t.asset), &model));
    assert_contract_error(result, CollateralError::InvalidUtilRange as u32);
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
        is_flashloanable: false,
        flashloan_fee: 0,
    };
    let result = flatten_contract_result(client.try_update_params(&hub(&t.asset), &model));
    assert_contract_error(result, CollateralError::InvalidReserveFactor as u32);
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
    assert_contract_error(result, CollateralError::BaseRateNegative as u32);
}

// Registering the same asset twice reverts with AssetAlreadySupported (#2).
#[test]
fn test_create_market_rejects_duplicate_asset() {
    let t = TestSetup::new();
    let client = t.client();

    let result = flatten_contract_result(client.try_create_market(&0u32, &market_params(&t.asset)));
    assert_contract_error(result, GenericError::AssetAlreadySupported as u32);
}

// Unknown market operations revert with PoolNotInitialized (#30).
#[test]
fn test_supply_rejects_unknown_market() {
    let t = TestSetup::new();
    let client = t.client();

    let unknown_asset = Address::generate(&t.env);
    let result =
        flatten_contract_result(client.try_supply(&t.sup_for(&unknown_asset, 0, 1_0000000i128)));
    assert_contract_error(result, GenericError::PoolNotInitialized as u32);
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
    assert_eq!(sync.state.supply_index, RAY, "supply index must start at RAY");
    assert_eq!(sync.state.borrow_index, RAY, "borrow index must start at RAY");
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

// create_market is #[only_owner]; calls without auth fail.
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

    let assets = vec![&t.env, hub(&t.asset), hub(&asset_b)];
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

// An empty request returns an empty result without panicking.
#[test]
fn test_bulk_get_indexes_empty_request() {
    let t = TestSetup::new();
    let bulk = t.client().get_bulk_indexes(&Vec::new(&t.env));
    assert_eq!(bulk.len(), 0);
}

// Unknown assets fail bulk read with PoolNotInitialized, matching get_sync_data.
#[test]
fn test_bulk_get_indexes_unknown_asset_panics() {
    let t = TestSetup::new();
    let unknown = Address::generate(&t.env);
    let assets = vec![&t.env, hub(&unknown)];
    let result = flatten_contract_result(t.client().try_get_bulk_indexes(&assets));
    assert_contract_error(result, GenericError::PoolNotInitialized as u32);
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
    assert_contract_error(result, CollateralError::UtilizationAboveMax as u32);
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

// --- POOL-CAN-001: virtual offset bounds dust-reward growth. ---

/// Dust supply + large reward leaves the market usable: index stays below the
/// cap, the dust position recovers almost none of the reward, and a later
/// accrual/withdraw still succeeds.
#[test]
fn test_dust_supply_plus_reward_no_longer_bricks_a_fresh_market() {
    let t = TestSetup::new();
    let asset = t.add_funded_market();
    let client = t.client();

    let attacker = Address::generate(&t.env);
    let opened = client.supply(&t.sup_for(&asset, 0, 1));
    let attacker_scaled = opened.get(0).unwrap().position.scaled_amount;

    let reward = 170_141_183_459i128;
    client.add_rewards(&hub(&asset), &reward);

    let grown = t.state_of(&asset).supply_index;
    assert!(grown < common::constants::MAX_SUPPLY_INDEX_RAY);
    assert!(grown < RAY * 1_000_000);

    let exit = vec![
        &t.env,
        PoolWithdrawEntry {
            action: t.action_for(&asset, attacker_scaled, reward + 1),
            protocol_fee: 0,
        },
    ];
    let recovered = client.withdraw(&attacker, &false, &exit);
    assert!(recovered.get(0).unwrap().actual_amount < reward / 1_000);

    let victim = client.supply(&t.sup_for(&asset, 0, 10_000_000_000i128));
    let victim_scaled = victim.get(0).unwrap().position.scaled_amount;
    let borrower = Address::generate(&t.env);
    client.borrow(
        &borrower,
        &vec![
            &t.env,
            PoolBorrowEntry {
                action: t.action_for(&asset, 0, 5_000_000_000i128),
            },
        ],
    );

    t.advance_time(86_400);

    client.update_indexes(&hub(&asset));
    let _ = client.get_bulk_indexes(&vec![&t.env, hub(&asset), hub(&t.asset)]);
    let rescued = client.withdraw(
        &attacker,
        &false,
        &vec![
            &t.env,
            PoolWithdrawEntry {
                action: t.action_for(&asset, victim_scaled, 10_000_000_000i128),
                protocol_fee: 0,
            },
        ],
    );
    assert!(rescued.get(0).unwrap().actual_amount > 0);
}

// A reward exceeding the ceiling is rejected; index unchanged.
#[test]
fn test_add_rewards_rejects_reward_above_supply_index_ceiling() {
    let t = TestSetup::new();
    let asset = t.add_funded_market();
    let client = t.client();

    client.supply(&t.sup_for(&asset, 0, 1));

    let huge = 1_000_000_000_000_000i128;
    let result = flatten_contract_result(client.try_add_rewards(&hub(&asset), &huge));
    assert_contract_error(result, GenericError::SupplyIndexRewardCeiling as u32);
    assert_eq!(t.state_of(&asset).supply_index, RAY);
}

// Doubling reward legs hit the ceiling guard before the index reaches MAX.
#[test]
fn test_iterated_add_rewards_cannot_pin_supply_index_at_max() {
    let t = TestSetup::new();
    let asset = t.add_funded_market();
    let client = t.client();

    client.supply(&t.sup_for(&asset, 0, 1));

    let mut legs_applied = 0u32;
    let mut reverted = false;
    for _ in 0..80 {
        // Size each leg to the reward denominator so the index roughly doubles.
        let reward = client.get_supplied_amount(&hub(&asset)) + 10_000_000i128;
        let result = flatten_contract_result(client.try_add_rewards(&hub(&asset), &reward));
        if result.is_err() {
            assert_contract_error(result, GenericError::SupplyIndexRewardCeiling as u32);
            reverted = true;
            break;
        }
        legs_applied += 1;
    }

    assert!(reverted, "ceiling guard must reject a pinning reward leg");
    assert!(legs_applied >= 1, "earlier legs still grow the index");
    let index = t.state_of(&asset).supply_index;
    assert!(
        index <= SUPPLY_INDEX_REWARD_CEILING_RAY,
        "index stays at or below the reward ceiling"
    );
    assert!(index < common::constants::MAX_SUPPLY_INDEX_RAY);
}

// --- POOL-CAN-002: revenue claims never outpay the shares they burn. ---

/// Floor conversion never quotes more than revenue shares are worth; half-up can.
#[test]
fn test_revenue_conversion_floor_never_exceeds_entitlement_but_half_up_does() {
    let t = TestSetup::new();
    let client = t.client();
    let borrower = Address::generate(&t.env);

    client.supply(&t.sup(0, 10_000_000_000i128));
    client.borrow(&borrower, &t.bor(0, 4_000_000_000i128));
    t.advance_time(1);
    client.update_indexes(&hub(&t.asset));

    let state = t.state_snapshot();
    let entitlement_ray =
        common::math::fp_core::mul_div_floor(&t.env, state.revenue, state.supply_index, RAY);

    let (half_up, floored) = t.env.as_contract(&t.pool, || {
        let cache = Cache::load(&t.env, &hub(&t.asset));
        (
            cache.unscale_supply(cache.revenue),
            cache.unscale_supply_floor(cache.revenue),
        )
    });

    assert!(half_up * WAD_PER_RAW > entitlement_ray);
    assert!(floored * WAD_PER_RAW <= entitlement_ray);
    assert!(half_up > floored);
}

/// Repeated claims near half a raw unit: paid amount never exceeds burned share value.
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
        // Accrue before claim so the claim itself adds no new interest.
        client.update_indexes(&hub(&t.asset));

        let half_up = t.env.as_contract(&t.pool, || {
            let cache = Cache::load(&t.env, &hub(&t.asset));
            cache.unscale_supply(cache.revenue)
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

        // Invariant: paid raw never exceeds burned share value.
        assert!(paid * WAD_PER_RAW <= burned_ray);

        total_paid += paid;
        total_half_up_would_pay += half_up;

        if VERBOSE_CLAIM_DUST {
            std::println!(
                "claim {second}: half_up_would_pay={half_up} paid={paid} burned_ray={burned_ray}"
            );
        }
    }

    assert!(total_half_up_would_pay > total_paid);
    if VERBOSE_CLAIM_DUST {
        std::println!("TOTAL half_up={total_half_up_would_pay} floor={total_paid}");
    }
}

/// Flooring defers dust; once entitlement clears one raw unit the claim pays out.
#[test]
fn test_revenue_claim_pays_out_once_entitlement_clears_one_raw_unit() {
    let t = TestSetup::new();
    let client = t.client();
    let borrower = Address::generate(&t.env);

    client.supply(&t.sup(0, 10_000_000_000i128));
    client.borrow(&borrower, &t.bor(0, 4_000_000_000i128));
    t.advance_time(86_400);
    client.update_indexes(&hub(&t.asset));

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

// --- POOL-CAN-004: `load_sync_data` pays a redundant TTL renewal. ---

/// `load_sync_data` pays two `renew_market_keys` calls; the second is redundant
/// (no TTL change, measurable extra CPU).
#[test]
fn test_load_sync_data_pays_for_a_redundant_ttl_renewal() {
    let t = TestSetup::new();

    t.env.cost_estimate().budget().reset_default();
    t.env.as_contract(&t.pool, || {
        crate::utils::renew_market_keys(&t.env, &hub(&t.asset));
    });
    let one_renewal = t.env.cost_estimate().budget().cpu_instruction_cost();

    t.env.cost_estimate().budget().reset_default();
    t.env.as_contract(&t.pool, || {
        crate::utils::renew_market_keys(&t.env, &hub(&t.asset));
        crate::utils::renew_market_keys(&t.env, &hub(&t.asset));
    });
    let two_renewals = t.env.cost_estimate().budget().cpu_instruction_cost();

    let redundant = two_renewals - one_renewal;
    assert!(redundant > 0);

    const PRODUCTION_CPU_LIMIT: u64 = 100_000_000;
    assert!(redundant * 20 < PRODUCTION_CPU_LIMIT / 100);

    let ttl_before = t.env.as_contract(&t.pool, || {
        t.env
            .storage()
            .persistent()
            .get_ttl(&PoolKey::State(hub(&t.asset)))
    });
    t.env.as_contract(&t.pool, || {
        crate::utils::renew_market_keys(&t.env, &hub(&t.asset));
    });
    let ttl_after = t.env.as_contract(&t.pool, || {
        t.env
            .storage()
            .persistent()
            .get_ttl(&PoolKey::State(hub(&t.asset)))
    });
    assert_eq!(ttl_before, ttl_after);

    if VERBOSE_CLAIM_DUST {
        std::println!(
            "renew_market_keys cpu={} redundant cpu per get_sync_data={}",
            one_renewal,
            redundant
        );
    }
}

// --- Bad-debt wipeout: floor leaves deposits usable. ---

/// Full wipeout floors `supply_index` at `RAY / 1000`; a large follow-up deposit
/// still mints shares.
#[test]
fn test_bad_debt_wipeout_leaves_market_usable_at_realistic_scale() {
    let t = TestSetup::new();
    let client = t.client();

    client.supply(&t.sup(0, 10_000_000_000i128));

    t.env.as_contract(&t.pool, || {
        let mut cache = Cache::load(&t.env, &hub(&t.asset));
        let total_supplied_value = cache.supplied.mul(&t.env, cache.supply_index);
        crate::interest::apply_bad_debt_to_supply_index(&mut cache, total_supplied_value);
        cache.save();
    });

    let floored = t.state_snapshot().supply_index;
    assert_eq!(floored, common::constants::SUPPLY_INDEX_FLOOR_RAW);
    assert_eq!(RAY / floored, 1_000);

    let opened = client.supply(&t.sup(0, 10_000_000_000_000i128));
    assert!(opened.get(0).unwrap().position.scaled_amount > 0);
}
