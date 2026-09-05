extern crate std;

use common::constants::TTL_THRESHOLD_INSTANCE;
use defindex_strategy::{Config, DataKey, DeFindexStrategyError, Strategy, StrategyClient};
use soroban_sdk::testutils::storage::{Instance as _, Persistent as _};
use soroban_sdk::testutils::{Address as _, Events, Ledger as _, MockAuth, MockAuthInvoke};
use soroban_sdk::xdr::{ContractEventBody, ContractEventV0, ScVal};
use soroban_sdk::{token, vec, Address, Env, Error, IntoVal, InvokeError, Val, Vec};
use test_harness::{
    eth_preset, hub_asset, usdc_preset, LendingTest, ALICE, BOB, HARNESS_HUB, HARNESS_SPOKE,
};

const UNIT: i128 = 10_000_000;
const PPS_SCALAR: i128 = 1_000_000_000_000;
const RAY: i128 = 1_000_000_000_000_000_000_000_000_000;

const LEDGERS_PER_DAY: u32 = 17_280;
const VAULT_TTL_THRESHOLD: u32 = LEDGERS_PER_DAY * 30;

fn pps_from_supply_index(supply_index: i128) -> i128 {
    supply_index / (RAY / PPS_SCALAR)
}

fn flatten_strategy_result<T>(
    result: Result<Result<T, Error>, Result<DeFindexStrategyError, InvokeError>>,
) -> Result<T, Error> {
    match result {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(err)) => Err(err),
        Err(Ok(err)) => Err(Error::from(&err)),
        Err(Err(invoke)) => {
            panic!("expected contract error, got host-level InvokeError: {invoke:?}")
        }
    }
}

fn assert_strategy_error<T: core::fmt::Debug>(result: Result<T, Error>, code: u32) {
    match result {
        Ok(value) => panic!("expected contract error {code}, got Ok({value:?})"),
        Err(err) => assert_eq!(
            err,
            Error::from_contract_error(code),
            "unexpected contract error"
        ),
    }
}

fn topic_is(body: &ContractEventV0, first: &str, second: &str) -> bool {
    match (body.topics.first(), body.topics.get(1)) {
        (Some(ScVal::Symbol(a)), Some(ScVal::Symbol(b))) => {
            a.0.to_string() == first && b.0.to_string() == second
        }
        _ => false,
    }
}

fn map_i128_field(data: &ScVal, key: &str) -> i128 {
    match data {
        ScVal::Map(Some(m)) => {
            let val = m
                .iter()
                .find(|e| matches!(&e.key, ScVal::Symbol(s) if s.0.to_string() == key))
                .map(|e| &e.val)
                .unwrap_or_else(|| panic!("missing {key} in harvest event"));
            match val {
                ScVal::I128(parts) => i128::from(parts),
                other => panic!("expected I128 for {key}, got {other:?}"),
            }
        }
        other => panic!("expected map, got {other:?}"),
    }
}

fn harvest_pps_values(env: &Env) -> std::vec::Vec<i128> {
    env.events()
        .all()
        .events()
        .iter()
        .filter_map(|event| {
            let ContractEventBody::V0(body) = &event.body;
            topic_is(body, "strategy", "harvest")
                .then(|| map_i128_field(&body.data, "price_per_share"))
        })
        .collect()
}

fn last_harvest_pps(env: &Env) -> i128 {
    harvest_pps_values(env)
        .last()
        .copied()
        .expect("expected a harvest event")
}

struct StrategyTest {
    t: LendingTest,
    client_address: Address,
    vault: Address,
    asset: Address,
}

impl StrategyTest {
    fn new() -> Self {
        let mut t = LendingTest::new()
            .with_market(usdc_preset())
            .with_market(eth_preset())
            .build();

        t.supply(ALICE, "USDC", 10_000.0);
        t.supply(BOB, "ETH", 100.0);
        t.borrow(BOB, "USDC", 400.0);

        let asset = t.resolve_asset("USDC");
        let init_args: Vec<Val> = vec![
            &t.env,
            t.controller.clone().into_val(&t.env),
            HARNESS_HUB.into_val(&t.env),
            HARNESS_SPOKE.into_val(&t.env),
        ];
        let client_address = t.env.register(Strategy, (asset.clone(), init_args));

        let vault = Address::generate(&t.env);
        t.resolve_market("USDC")
            .token_admin
            .mint(&vault, &(100_000 * UNIT));

        Self {
            t,
            client_address,
            vault,
            asset,
        }
    }

    fn client(&self) -> StrategyClient<'_> {
        StrategyClient::new(&self.t.env, &self.client_address)
    }

    fn market_pps(&self) -> i128 {
        let index = self
            .t
            .ctrl_client()
            .get_market_index(&hub_asset(self.asset.clone()))
            .supply_index;
        pps_from_supply_index(index)
    }

    fn mint_vault(&self, units: i128) -> Address {
        let vault = Address::generate(&self.t.env);
        self.t
            .resolve_market("USDC")
            .token_admin
            .mint(&vault, &(units * UNIT));
        vault
    }

    fn usdc_balance(&self, of: &Address) -> i128 {
        token::Client::new(&self.t.env, &self.asset).balance(of)
    }

    fn live_account_id(&self, vault: &Address) -> u64 {
        let env = &self.t.env;
        let stored: u64 = env.as_contract(&self.client_address, || {
            env.storage()
                .persistent()
                .get(&DataKey::VaultAccount(vault.clone()))
                .unwrap_or(0)
        });
        if stored != 0 && self.t.account_exists(stored) {
            stored
        } else {
            0
        }
    }

    /// Point a vault at an account id the controller does not have, the state a
    /// closure outside this strategy leaves behind.
    fn point_vault_at(&self, vault: &Address, account_id: u64) {
        let env = &self.t.env;
        env.as_contract(&self.client_address, || {
            env.storage()
                .persistent()
                .set(&DataKey::VaultAccount(vault.clone()), &account_id);
        });
    }

    fn stored_account_id(&self, vault: &Address) -> u64 {
        let env = &self.t.env;
        env.as_contract(&self.client_address, || {
            env.storage()
                .persistent()
                .get(&DataKey::VaultAccount(vault.clone()))
                .unwrap_or(0)
        })
    }

    fn vault_mapping_ttl(&self, vault: &Address) -> u32 {
        let env = &self.t.env;
        env.as_contract(&self.client_address, || {
            env.storage()
                .persistent()
                .get_ttl(&DataKey::VaultAccount(vault.clone()))
        })
    }
}

#[test]
fn test_asset_returns_configured_underlying() {
    let s = StrategyTest::new();
    assert_eq!(s.client().asset(), s.asset);
}

#[test]
fn test_deposit_reports_underlying_and_accrues_interest() {
    let s = StrategyTest::new();
    let client = s.client();

    let reported = client.deposit(&(1_000 * UNIT), &s.vault);
    assert_eq!(reported, 1_000 * UNIT);
    assert_eq!(client.balance(&s.vault), reported);
    assert!(s.live_account_id(&s.vault) > 0);

    s.t.advance_time_no_refresh(60 * 60 * 24 * 180);
    let grown = client.balance(&s.vault);
    assert!(
        grown > reported,
        "balance must grow with interest, {reported} -> {grown}"
    );
}

#[test]
fn test_deposit_at_instance_min_borrow_collateral_floor_succeeds() {
    let s = StrategyTest::new();
    let reported = s.client().deposit(&(5 * UNIT), &s.vault);
    assert_eq!(reported, 5 * UNIT);
}

#[test]
fn test_second_deposit_can_be_small_after_account_opened() {
    let s = StrategyTest::new();
    let client = s.client();

    client.deposit(&(10 * UNIT), &s.vault);
    let after_small = client.deposit(&UNIT, &s.vault);
    assert_eq!(after_small, 11 * UNIT);
}

#[test]
fn test_withdraw_pays_recipient_directly_and_terminal_exit_closes_account() {
    let mut s = StrategyTest::new();
    s.client().deposit(&(1_000 * UNIT), &s.vault);

    s.t.advance_time(60 * 60 * 24 * 30);
    let client = s.client();

    let sink = Address::generate(&s.t.env);
    let remaining = client.withdraw(&(300 * UNIT), &s.vault, &sink);
    assert_eq!(s.usdc_balance(&sink), 300 * UNIT);
    assert_eq!(s.usdc_balance(&s.client_address), 0);
    assert_eq!(client.balance(&s.vault), remaining);
    assert!(s.live_account_id(&s.vault) != 0);

    let account_before = s.live_account_id(&s.vault);
    let balance = client.balance(&s.vault);
    let left = client.withdraw(&balance, &s.vault, &sink);
    assert_eq!(left, 0);
    assert_eq!(client.balance(&s.vault), 0);
    assert_eq!(s.live_account_id(&s.vault), 0);

    client.deposit(&(500 * UNIT), &s.vault);
    let account_after = s.live_account_id(&s.vault);
    assert!(account_after > account_before);
    assert!(client.balance(&s.vault) > 499 * UNIT);
}

#[test]
fn test_two_vaults_have_isolated_lending_accounts() {
    let mut s = StrategyTest::new();
    let vault_b = s.mint_vault(10_000);

    s.client().deposit(&(1_000 * UNIT), &s.vault);
    s.client().deposit(&(1_000 * UNIT), &vault_b);

    let id_a = s.live_account_id(&s.vault);
    let id_b = s.live_account_id(&vault_b);
    assert!(id_a > 0);
    assert!(id_b > 0);
    assert_ne!(
        id_a, id_b,
        "each vault must own a distinct controller account"
    );

    assert_eq!(s.client().balance(&s.vault), 1_000 * UNIT);
    assert_eq!(s.client().balance(&vault_b), 1_000 * UNIT);

    s.t.advance_time(60 * 60 * 24 * 365);
    let a = s.client().balance(&s.vault);
    let b = s.client().balance(&vault_b);
    assert!(
        (a - b).abs() <= 2,
        "isolated accounts with equal principal should accrue equally, {a} vs {b}"
    );

    let sink = Address::generate(&s.t.env);
    s.client().withdraw(&a, &s.vault, &sink);
    assert_eq!(s.client().balance(&s.vault), 0);
    assert_eq!(s.live_account_id(&s.vault), 0);
    assert!(
        s.live_account_id(&vault_b) != 0,
        "closing vault A must not affect vault B's lending account"
    );
    assert!(s.client().balance(&vault_b) > 1_000 * UNIT);
}

#[test]
fn test_supply_clears_stale_vault_mapping_after_full_withdraw() {
    let mut s = StrategyTest::new();

    s.client().deposit(&(1_000 * UNIT), &s.vault);
    let account_before = s.live_account_id(&s.vault);

    s.t.advance_time(60 * 60 * 24 * 30);
    let balance = s.client().balance(&s.vault);
    let sink = Address::generate(&s.t.env);
    s.client().withdraw(&balance, &s.vault, &sink);

    assert_eq!(s.client().balance(&s.vault), 0);
    assert_eq!(s.live_account_id(&s.vault), 0);

    s.client().deposit(&(500 * UNIT), &s.vault);
    let account_after = s.live_account_id(&s.vault);
    assert!(account_after > account_before);
    assert!(s.client().balance(&s.vault) > 499 * UNIT);
    assert!(s.live_account_id(&s.vault) != 0);
}

#[test]
fn test_full_withdraw_clears_stored_vault_mapping_immediately() {
    let mut s = StrategyTest::new();
    s.client().deposit(&(1_000 * UNIT), &s.vault);
    s.t.advance_time(60 * 60 * 24 * 30);

    let balance = s.client().balance(&s.vault);
    let sink = Address::generate(&s.t.env);
    s.client().withdraw(&balance, &s.vault, &sink);

    let env = &s.t.env;
    let raw_stored: u64 = env.as_contract(&s.client_address, || {
        env.storage()
            .persistent()
            .get(&DataKey::VaultAccount(s.vault.clone()))
            .unwrap_or(0)
    });
    assert_eq!(
        raw_stored, 0,
        "full withdraw must clear the stored vault mapping, not defer it"
    );
}

#[test]
fn test_read_path_reextends_vault_mapping_ttl_below_threshold() {
    let mut s = StrategyTest::new();
    s.client().deposit(&(1_000 * UNIT), &s.vault);

    let initial = s.vault_mapping_ttl(&s.vault);
    assert!(
        initial > VAULT_TTL_THRESHOLD,
        "deposit must extend the fresh mapping well past the threshold, got {initial}"
    );

    s.t.advance_time(60 * 60 * 24 * 114);
    let aged = s.vault_mapping_ttl(&s.vault);
    assert!(
        aged < VAULT_TTL_THRESHOLD && aged > 50_000,
        "aged TTL must sit between the mutant thresholds and the real one, got {aged}"
    );

    assert!(s.client().balance(&s.vault) > 0);
    let renewed = s.vault_mapping_ttl(&s.vault);
    assert!(
        renewed > VAULT_TTL_THRESHOLD,
        "read path must re-extend the mapping TTL below threshold: {aged} -> {renewed}"
    );
}

#[test]
fn test_deposit_authorizes_pool_transfer_without_global_auth_mock() {
    let s = StrategyTest::new();
    let env = &s.t.env;
    let amount = 100 * UNIT;

    env.mock_auths(&[MockAuth {
        address: &s.vault,
        invoke: &MockAuthInvoke {
            contract: &s.client_address,
            fn_name: "deposit",
            args: (amount, s.vault.clone()).into_val(env),
            sub_invokes: &[MockAuthInvoke {
                contract: &s.asset,
                fn_name: "transfer",
                args: (s.vault.clone(), s.client_address.clone(), amount).into_val(env),
                sub_invokes: &[],
            }],
        },
    }]);

    let reported = s.client().deposit(&amount, &s.vault);
    assert_eq!(reported, amount);
    assert_eq!(
        s.usdc_balance(&s.client_address),
        0,
        "no funds may strand on the adapter"
    );
}

#[test]
fn test_harvest_emits_price_per_share_from_supply_index() {
    let s = StrategyTest::new();
    s.client().deposit(&(1_000 * UNIT), &s.vault);

    let expected = s.market_pps();
    assert!(
        expected >= PPS_SCALAR,
        "pps at par should be at least PPS_SCALAR, got {expected}"
    );

    s.client().harvest(&s.vault, &None);
    let emitted = last_harvest_pps(&s.t.env);
    assert_eq!(emitted, expected);

    s.t.advance_time_no_refresh(60 * 60 * 24 * 180);
    let expected_after = s.market_pps();
    assert!(
        expected_after > expected,
        "supply index should accrue, {expected} -> {expected_after}"
    );

    s.client().harvest(&s.vault, &None);
    assert_eq!(last_harvest_pps(&s.t.env), expected_after);
}

#[test]
fn test_harvest_price_per_share_independent_of_vault_balance() {
    let mut s = StrategyTest::new();
    let vault_b = s.mint_vault(100_000);

    s.client().deposit(&(100 * UNIT), &s.vault);
    s.client().deposit(&(10_000 * UNIT), &vault_b);
    s.t.advance_time(60 * 60 * 24 * 90);

    let expected = s.market_pps();
    assert!(
        expected > PPS_SCALAR,
        "accrual should lift pps above par, got {expected}"
    );
    assert!(
        s.client().balance(&s.vault) < s.client().balance(&vault_b) / 50,
        "sanity: vault balances must differ in magnitude"
    );

    s.client().harvest(&s.vault, &None);
    let pps_small = last_harvest_pps(&s.t.env);

    s.client().harvest(&vault_b, &None);
    let pps_large = last_harvest_pps(&s.t.env);

    assert_eq!(pps_small, expected);
    assert_eq!(pps_large, expected);
}

#[test]
fn test_harvest_requires_from_auth() {
    let s = StrategyTest::new();

    s.client().deposit(&(1_000 * UNIT), &s.vault);

    let attacker_chosen_from = Address::generate(&s.t.env);

    s.t.env.set_auths(&[]);

    let blocked_harvest = s.client().try_harvest(&attacker_chosen_from, &None);
    assert!(blocked_harvest.is_err(), "harvest must require `from` auth");

    let blocked_deposit = s.client().try_deposit(&UNIT, &attacker_chosen_from);
    assert!(blocked_deposit.is_err(), "deposit must require `from` auth");
}

#[test]
fn test_donation_via_controller_supply_inflates_nav() {
    let s = StrategyTest::new();
    let client = s.client();

    client.deposit(&(1_000 * UNIT), &s.vault);
    let account_id = s.live_account_id(&s.vault);
    assert!(account_id > 0);
    let before = client.balance(&s.vault);

    let attacker = Address::generate(&s.t.env);
    s.t.resolve_market("USDC")
        .token_admin
        .mint(&attacker, &(500 * UNIT));
    s.t.ctrl_client().supply(
        &attacker,
        &account_id,
        &HARNESS_SPOKE,
        &vec![&s.t.env, (hub_asset(s.asset.clone()), 500 * UNIT)],
    );

    let after = client.balance(&s.vault);
    assert!(
        after >= before + 499 * UNIT,
        "external donation inflated strategy balance/NAV: {before} -> {after}"
    );
}

#[test]
fn test_deposit_zero_amount_returns_amount_not_positive() {
    let s = StrategyTest::new();
    let result = flatten_strategy_result(s.client().try_deposit(&0, &s.vault));
    assert_strategy_error(result, DeFindexStrategyError::AmountNotPositive as u32);
}

#[test]
fn test_withdraw_zero_amount_returns_amount_not_positive() {
    let s = StrategyTest::new();
    s.client().deposit(&(1_000 * UNIT), &s.vault);

    let sink = Address::generate(&s.t.env);
    let result = flatten_strategy_result(s.client().try_withdraw(&0, &s.vault, &sink));
    assert_strategy_error(result, DeFindexStrategyError::AmountNotPositive as u32);
}

#[test]
fn test_withdraw_without_position_returns_insufficient_balance() {
    let s = StrategyTest::new();
    let sink = Address::generate(&s.t.env);
    let result = flatten_strategy_result(s.client().try_withdraw(&UNIT, &s.vault, &sink));
    assert_strategy_error(result, DeFindexStrategyError::InsufficientBalance as u32);
}

#[test]
fn test_withdraw_over_balance_returns_insufficient_balance() {
    let s = StrategyTest::new();
    s.client().deposit(&(1_000 * UNIT), &s.vault);

    let sink = Address::generate(&s.t.env);
    let result = flatten_strategy_result(s.client().try_withdraw(&(1_001 * UNIT), &s.vault, &sink));
    assert_strategy_error(result, DeFindexStrategyError::InsufficientBalance as u32);
}

/// A mapping pointing at an account the controller has no record of is an
/// explicit "gone": it clears and the next supply opens a fresh account, rather
/// than surfacing as a lookup failure.
#[test]
fn test_supply_reopens_when_the_mapping_points_at_a_missing_account() {
    let s = StrategyTest::new();

    s.client().deposit(&(1_000 * UNIT), &s.vault);
    let original = s.stored_account_id(&s.vault);
    assert!(original != 0);

    const NEVER_CREATED: u64 = 9_999_999;
    assert!(!s.t.account_exists(NEVER_CREATED));
    s.point_vault_at(&s.vault, NEVER_CREATED);

    s.client().deposit(&(500 * UNIT), &s.vault);

    let reopened = s.stored_account_id(&s.vault);
    assert!(reopened != NEVER_CREATED, "stale pointer must not survive");
    assert!(reopened != 0, "a fresh account must be opened");
    assert!(s.client().balance(&s.vault) > 499 * UNIT);
}

// ---------------------------------------------------------------------------
// Construction and failure paths.
//
// `__constructor` decodes `init_args` positionally and every arity or type
// error collapses to `NotInitialized`, so a caller that passes the arguments in
// the wrong order gets the same error as one that passes none. Each position is
// probed from both sides -- absent, and present with the wrong type -- because
// a `get(n)` that drifts to `get(n+1)` still panics on one input and silently
// reads the neighbouring argument on the other.
//
// None of these reach the controller: the panic fires before
// `get_market_index`, so they need no lending harness.
// ---------------------------------------------------------------------------

const NOT_INITIALIZED: u32 = 401;
const ACCOUNT_LOOKUP_FAILED: u32 = 463;

fn ctor_env() -> (Env, Address, Address) {
    let env = Env::default();
    let asset = Address::generate(&env);
    let controller = Address::generate(&env);
    (env, asset, controller)
}

#[test]
#[should_panic(expected = "Error(Contract, #401)")]
fn constructor_rejects_missing_controller_argument() {
    let (env, asset, _controller) = ctor_env();
    let args: Vec<Val> = vec![&env];
    env.register(Strategy, (asset, args));
}

#[test]
#[should_panic(expected = "Error(Contract, #401)")]
fn constructor_rejects_controller_argument_of_the_wrong_type() {
    let (env, asset, _controller) = ctor_env();
    let args: Vec<Val> = vec![&env, 7u32.into_val(&env)];
    env.register(Strategy, (asset, args));
}

#[test]
#[should_panic(expected = "Error(Contract, #401)")]
fn constructor_rejects_missing_hub_id_argument() {
    let (env, asset, controller) = ctor_env();
    let args: Vec<Val> = vec![&env, controller.into_val(&env)];
    env.register(Strategy, (asset, args));
}

#[test]
#[should_panic(expected = "Error(Contract, #401)")]
fn constructor_rejects_hub_id_argument_of_the_wrong_type() {
    let (env, asset, controller) = ctor_env();
    // An Address where a u32 is expected -- the shape a caller gets by passing
    // (controller, controller, spoke) instead of (controller, hub, spoke).
    let args: Vec<Val> = vec![
        &env,
        controller.clone().into_val(&env),
        controller.into_val(&env),
    ];
    env.register(Strategy, (asset, args));
}

#[test]
#[should_panic(expected = "Error(Contract, #401)")]
fn constructor_rejects_missing_spoke_id_argument() {
    let (env, asset, controller) = ctor_env();
    let args: Vec<Val> = vec![&env, controller.into_val(&env), HARNESS_HUB.into_val(&env)];
    env.register(Strategy, (asset, args));
}

#[test]
#[should_panic(expected = "Error(Contract, #401)")]
fn constructor_rejects_spoke_id_argument_of_the_wrong_type() {
    let (env, asset, controller) = ctor_env();
    let args: Vec<Val> = vec![
        &env,
        controller.clone().into_val(&env),
        HARNESS_HUB.into_val(&env),
        controller.into_val(&env),
    ];
    env.register(Strategy, (asset, args));
}

// ---------------------------------------------------------------------------
// Every entry point fails closed once the instance `Config` is gone.
//
// The constructor always writes `Config`, so the `NotInitialized` arm of
// `config()` is unreachable through the front door. It is reachable through
// instance-storage archival: if the instance entry expires and is not restored,
// a live strategy holding vault collateral wakes up with no configuration. What
// must not happen then is an entry point reading a default and continuing, so
// each one is checked to return `NotInitialized` rather than a value.
// ---------------------------------------------------------------------------

/// `try_*` methods whose success type converts infallibly surface the inner
/// error as `ConversionError` rather than `Error`, so the shared
/// `flatten_strategy_result` does not fit them. Only the outer contract error
/// matters here, which is the same for every entry point.
fn assert_not_initialized<T: core::fmt::Debug, E: core::fmt::Debug>(
    result: Result<Result<T, E>, Result<DeFindexStrategyError, InvokeError>>,
) {
    match result {
        Err(Ok(err)) => assert_eq!(
            Error::from(&err),
            Error::from_contract_error(NOT_INITIALIZED),
            "expected NotInitialized"
        ),
        other => panic!("expected NotInitialized, got {other:?}"),
    }
}

fn strategy_with_config_erased() -> StrategyTest {
    let s = StrategyTest::new();
    s.t.env.mock_all_auths();
    s.t.env.as_contract(&s.client_address, || {
        s.t.env.storage().instance().remove(&DataKey::Config);
    });
    s
}

#[test]
fn asset_reports_not_initialized_when_the_config_is_gone() {
    let s = strategy_with_config_erased();
    assert_not_initialized(s.client().try_asset());
}

#[test]
fn deposit_reports_not_initialized_when_the_config_is_gone() {
    let s = strategy_with_config_erased();
    let vault = s.vault.clone();
    // Positive amount on purpose: a non-positive one short-circuits on
    // AmountNotPositive and never reaches the config load.
    assert_not_initialized(s.client().try_deposit(&UNIT, &vault));
}

#[test]
fn withdraw_reports_not_initialized_when_the_config_is_gone() {
    let s = strategy_with_config_erased();
    let vault = s.vault.clone();
    assert_not_initialized(s.client().try_withdraw(&UNIT, &vault, &vault));
}

#[test]
fn harvest_reports_not_initialized_when_the_config_is_gone() {
    let s = strategy_with_config_erased();
    let vault = s.vault.clone();
    assert_not_initialized(s.client().try_harvest(&vault, &None));
}

#[test]
fn balance_reports_not_initialized_when_the_config_is_gone() {
    let s = strategy_with_config_erased();
    let vault = s.vault.clone();
    assert_not_initialized(s.client().try_balance(&vault));
}

// ---------------------------------------------------------------------------
// A controller that cannot answer `account_exists` must not be read as "gone".
//
// resolve_vault_account clears the vault -> account mapping only on an explicit
// `Ok(false)`. The mapping is the sole route back to the collateral it points
// at, so a lookup that merely failed has to abort rather than clear. This
// points the stored config at a contract with no `account_exists` export, which
// is the shape of a controller that was upgraded out from under the strategy.
// ---------------------------------------------------------------------------

#[test]
fn a_failed_controller_lookup_aborts_instead_of_clearing_the_vault_mapping() {
    let s = StrategyTest::new();
    s.t.env.mock_all_auths();
    let vault = s.vault.clone();

    // Give the vault a stored account id so the lookup is actually attempted;
    // a stored 0 returns early and never calls the controller.
    s.t.env.as_contract(&s.client_address, || {
        s.t.env
            .storage()
            .persistent()
            .set(&DataKey::VaultAccount(vault.clone()), &7u64);
        let cfg: Config = s.t.env.storage().instance().get(&DataKey::Config).unwrap();
        s.t.env.storage().instance().set(
            &DataKey::Config,
            &Config {
                // The strategy itself: a real contract that has no
                // `account_exists` entry point, so the call errors rather than
                // returning a boolean.
                controller: s.client_address.clone(),
                ..cfg
            },
        );
    });

    assert_strategy_error(
        flatten_strategy_result(s.client().try_balance(&vault)),
        ACCOUNT_LOOKUP_FAILED,
    );

    // The mapping survives: an unanswerable lookup is not evidence of absence.
    let still_there = s.t.env.as_contract(&s.client_address, || {
        s.t.env
            .storage()
            .persistent()
            .get::<_, u64>(&DataKey::VaultAccount(vault.clone()))
    });
    assert_eq!(still_there, Some(7u64));
}

/// The strategy is the only protocol contract with no other write path into
/// its instance: nothing in the controller or keeper renews it, so without a
/// renewal on its own entrypoints it archives 120 days after deploy however
/// busy it is. `asset` is used because it touches nothing but the instance,
/// so aging the ledger past the threshold cannot expire an unrelated entry;
/// the other entrypoints share the same first-line `renew_instance` call.
#[test]
fn asset_renews_the_strategy_instance_ttl() {
    let t = StrategyTest::new();
    let instance_ttl = |t: &StrategyTest| {
        t.t.env
            .as_contract(&t.client_address, || t.t.env.storage().instance().get_ttl())
    };

    let initial = instance_ttl(&t);
    let ledgers_to_age = initial - TTL_THRESHOLD_INSTANCE + 1;
    t.t.env
        .ledger()
        .with_mut(|ledger| ledger.sequence_number += ledgers_to_age);
    let aged = instance_ttl(&t);
    assert!(
        aged < TTL_THRESHOLD_INSTANCE,
        "aging must cross the renewal threshold or the check is vacuous: aged={aged}"
    );

    assert_eq!(t.client().asset(), t.asset);

    let renewed = instance_ttl(&t);
    assert!(
        renewed > TTL_THRESHOLD_INSTANCE,
        "asset() must renew the strategy instance: aged={aged}, after={renewed}"
    );
}
