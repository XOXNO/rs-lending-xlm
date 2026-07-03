//! DeFindex strategy tests with synthetic vault addresses.

extern crate std;

use defindex_strategy::{DataKey, DeFindexStrategyError, Strategy, StrategyClient};
use soroban_sdk::testutils::{Address as _, Events};
use soroban_sdk::xdr::{ContractEventBody, ScVal};
use soroban_sdk::{vec, Address, Env, IntoVal, Val, Vec};
use test_harness::{
    eth_preset, hub_asset, usdc_preset, LendingTest, ALICE, BOB, HARNESS_HUB, HARNESS_SPOKE,
};

const UNIT: i128 = 10_000_000; // 1.0 at the presets' 7 decimals
const PPS_SCALAR: i128 = 1_000_000_000_000;
const RAY: i128 = 1_000_000_000_000_000_000_000_000_000;

fn pps_from_supply_index(supply_index: i128) -> i128 {
    supply_index / (RAY / PPS_SCALAR)
}

fn flatten_strategy_result<T>(
    result: Result<
        Result<T, soroban_sdk::Error>,
        Result<DeFindexStrategyError, soroban_sdk::InvokeError>,
    >,
) -> Result<T, soroban_sdk::Error> {
    match result {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(err)) => Err(err),
        Err(Ok(err)) => Err(soroban_sdk::Error::from(&err)),
        Err(Err(invoke)) => {
            panic!("expected contract error, got host-level InvokeError: {invoke:?}")
        }
    }
}

fn assert_strategy_error<T: core::fmt::Debug>(result: Result<T, soroban_sdk::Error>, code: u32) {
    match result {
        Ok(value) => panic!("expected contract error {code}, got Ok({value:?})"),
        Err(err) => assert_eq!(
            err,
            soroban_sdk::Error::from_contract_error(code),
            "unexpected contract error"
        ),
    }
}

fn topic_is(body: &soroban_sdk::xdr::ContractEventV0, first: &str, second: &str) -> bool {
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
            // ETH market lets Bob post borrow collateral for the USDC borrow below.
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
        soroban_sdk::token::Client::new(&self.t.env, &self.asset).balance(of)
    }

    /// Live controller account id for `vault`, mirroring the strategy's read
    /// path: the stored mapping only counts while the controller account exists.
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

    // Read paths return 0 after controller account closure.
    assert_eq!(s.client().balance(&s.vault), 0);
    assert_eq!(s.live_account_id(&s.vault), 0);

    // Supply clears the stale mapping and opens a new controller account.
    s.client().deposit(&(500 * UNIT), &s.vault);
    let account_after = s.live_account_id(&s.vault);
    assert!(account_after > account_before);
    assert!(s.client().balance(&s.vault) > 499 * UNIT);
    assert!(s.live_account_id(&s.vault) != 0);
}

// A full withdraw clears the stored vault->account mapping immediately, not
// lazily on the next deposit. This is what prevents the dust-pinning grief: if
// the controller account were kept alive by dust of another asset, a deferred
// cleanup would leave the mapping pointing at that account and the next deposit
// would reuse it (and could hit PositionLimitExceeded). `live_account_id` masks
// this because it also checks account_exists — assert the raw stored value.
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

// Harvest requires `from` auth.
#[test]
fn harvest_requires_from_auth() {
    let s = StrategyTest::new();
    // Seed valid state before auth check.
    s.client().deposit(&(1_000 * UNIT), &s.vault);

    // Attacker-selected `from`.
    let attacker_chosen_from = Address::generate(&s.t.env);

    // Disable mocked auth.
    s.t.env.set_auths(&[]);

    // Harvest fails without `from` auth.
    let blocked_harvest = s.client().try_harvest(&attacker_chosen_from, &None);
    assert!(
        blocked_harvest.is_err(),
        "harvest must require `from` auth (VECTOR #1.2 fix)"
    );

    // Deposit also fails without auth.
    let blocked_deposit = s.client().try_deposit(&UNIT, &attacker_chosen_from);
    assert!(blocked_deposit.is_err(), "deposit must require `from` auth");
}

// Direct controller supply increases vault NAV without `Strategy::deposit`.
#[test]
fn poc_third_party_inflates_strategy_balance_via_controller_supply() {
    let s = StrategyTest::new();
    let client = s.client();

    client.deposit(&(1_000 * UNIT), &s.vault);
    let account_id = s.live_account_id(&s.vault);
    assert!(account_id > 0);
    let before = client.balance(&s.vault);

    // Bypass `Strategy::deposit` through controller supply.
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

    // Donation appears in vault NAV.
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
