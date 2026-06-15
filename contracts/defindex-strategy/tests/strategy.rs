//! End-to-end DeFindex-strategy flows against the full protocol stack.
//!
//! Plain addresses stand in for DeFindex vaults; trait auth shapes are identical.

extern crate std;

use defindex_strategy::{Strategy, StrategyClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{vec, Address, IntoVal, Val, Vec};
use test_harness::{eth_preset, usdc_preset, LendingTest, ALICE, BOB};

const UNIT: i128 = 10_000_000; // 1.0 at the presets' 7 decimals
const PPS_SCALAR: i128 = 1_000_000_000_000;
const RAY: i128 = 1_000_000_000_000_000_000_000_000_000;

fn pps_from_supply_index(supply_index_ray: i128) -> i128 {
    supply_index_ray / (RAY / PPS_SCALAR)
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
        let init_args: Vec<Val> = vec![&t.env, t.controller.clone().into_val(&t.env)];
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

    fn usdc_balance(&self, of: &Address) -> i128 {
        soroban_sdk::token::Client::new(&self.t.env, &self.asset).balance(of)
    }
}

#[test]
fn test_deposit_reports_underlying_and_accrues_interest() {
    let s = StrategyTest::new();
    let client = s.client();

    let reported = client.deposit(&(1_000 * UNIT), &s.vault);
    assert_eq!(reported, 1_000 * UNIT);
    assert_eq!(client.balance(&s.vault), reported);
    assert!(client.lending_account_id(&s.vault) > 0);
    assert!(client.has_lending_account(&s.vault));

    s.t.advance_time_no_refresh(60 * 60 * 24 * 180);
    let grown = client.balance(&s.vault);
    assert!(
        grown > reported,
        "balance must grow with interest, {reported} -> {grown}"
    );
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
    assert!(client.has_lending_account(&s.vault));

    let account_before = client.lending_account_id(&s.vault);
    let balance = client.balance(&s.vault);
    let left = client.withdraw(&balance, &s.vault, &sink);
    assert_eq!(left, 0);
    assert_eq!(client.balance(&s.vault), 0);
    assert_eq!(client.lending_account_id(&s.vault), 0);
    assert!(!client.has_lending_account(&s.vault));

    client.deposit(&(500 * UNIT), &s.vault);
    let account_after = client.lending_account_id(&s.vault);
    assert!(account_after > account_before);
    assert!(client.balance(&s.vault) > 499 * UNIT);
}

#[test]
fn test_two_vaults_have_isolated_lending_accounts() {
    let mut s = StrategyTest::new();

    let vault_b = Address::generate(&s.t.env);
    s.t.resolve_market("USDC")
        .token_admin
        .mint(&vault_b, &(10_000 * UNIT));

    s.client().deposit(&(1_000 * UNIT), &s.vault);
    s.client().deposit(&(1_000 * UNIT), &vault_b);

    let id_a = s.client().lending_account_id(&s.vault);
    let id_b = s.client().lending_account_id(&vault_b);
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
    assert!(!s.client().has_lending_account(&s.vault));
    assert!(
        s.client().has_lending_account(&vault_b),
        "closing vault A must not affect vault B's lending account"
    );
    assert!(s.client().balance(&vault_b) > 1_000 * UNIT);
}

#[test]
fn test_deposit_at_instance_min_borrow_collateral_floor_succeeds() {
    let s = StrategyTest::new();
    let client = s.client();

    let reported = client.deposit(&(5 * UNIT), &s.vault);
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
fn test_harvest_price_per_share_tracks_supply_index() {
    let s = StrategyTest::new();
    s.client().deposit(&(1_000 * UNIT), &s.vault);

    let asset = s.t.resolve_asset("USDC");
    let index_before = s.t.ctrl_client().get_market_index(&asset).supply_index_ray;
    let pps_before = pps_from_supply_index(index_before);

    // Blend-compatible baseline: ~10^12 at par, not scaled by vault TVL.
    assert!(
        pps_before >= PPS_SCALAR,
        "pps at par should be at least PPS_SCALAR, got {pps_before}"
    );
    assert!(
        pps_before <= PPS_SCALAR * 11 / 10,
        "pps should stay near par before long accrual, got {pps_before}"
    );

    s.client().harvest(&s.vault, &None);

    s.t.advance_time_no_refresh(60 * 60 * 24 * 180);
    let index_after = s.t.ctrl_client().get_market_index(&asset).supply_index_ray;
    assert!(
        index_after > index_before,
        "supply index should accrue, {index_before} -> {index_after}"
    );

    let pps_after = pps_from_supply_index(index_after);
    assert!(
        pps_after > pps_before,
        "harvest metric should rise with the supply index, {pps_before} -> {pps_after}"
    );

    s.client().harvest(&s.vault, &None);
}

#[test]
fn test_harvest_price_per_share_independent_of_vault_balance() {
    let mut s = StrategyTest::new();

    let vault_b = Address::generate(&s.t.env);
    s.t.resolve_market("USDC")
        .token_admin
        .mint(&vault_b, &(100_000 * UNIT));

    s.client().deposit(&(100 * UNIT), &s.vault);
    s.client().deposit(&(10_000 * UNIT), &vault_b);

    s.t.advance_time(60 * 60 * 24 * 90);

    let asset = s.t.resolve_asset("USDC");
    let market_pps =
        pps_from_supply_index(s.t.ctrl_client().get_market_index(&asset).supply_index_ray);

    // 100 USDC vs 10_000 USDC vaults would diverge under the old balance-based formula.
    assert!(
        market_pps > PPS_SCALAR,
        "accrual should lift pps above par, got {market_pps}"
    );
    assert!(
        s.client().balance(&s.vault) < s.client().balance(&vault_b) / 50,
        "sanity: vault balances must differ in magnitude"
    );

    s.client().harvest(&s.vault, &None);
    s.client().harvest(&vault_b, &None);
}

#[test]
fn test_balance_heals_stale_account_id_after_terminal_close() {
    let mut s = StrategyTest::new();
    s.client().deposit(&(1_000 * UNIT), &s.vault);

    s.t.advance_time(60 * 60 * 24 * 30);
    let balance = s.client().balance(&s.vault);
    let sink = Address::generate(&s.t.env);
    s.client().withdraw(&balance, &s.vault, &sink);

    assert_eq!(s.client().lending_account_id(&s.vault), 0);
    assert_eq!(s.client().balance(&s.vault), 0);
    assert!(!s.client().has_lending_account(&s.vault));
}
