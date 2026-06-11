//! End-to-end DeFindex-strategy flows against the full protocol stack.
//!
//! The "vault" here is a plain address standing in for a DeFindex vault: the
//! trait calls and auth shapes are identical, the harness mocks signatures.

extern crate std;

use defindex_strategy::{Strategy, StrategyClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{vec, Address, IntoVal, Val, Vec};
use test_harness::{eth_preset, usdc_preset, LendingTest, ALICE, BOB};

const UNIT: i128 = 10_000_000; // 1.0 at the presets' 7 decimals

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

        // Independent supply and a borrower: utilization stays above zero
        // so interest accrues, and the strategy can exit fully without
        // draining the market.
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
    // First deposit forfeits 1000 stroops of shares to the inflation guard.
    assert!(
        (1_000 * UNIT - 1_000..1_000 * UNIT).contains(&reported),
        "post-deposit balance should be ~1000 USDC minus the forfeit, got {reported}"
    );
    assert_eq!(client.balance(&s.vault), reported);
    assert!(client.lending_account_id() > 0);

    // Interest accrues without any keeper or oracle activity: balance() is
    // served by the controller's view-simulated index.
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

    // Refreshing advance: mutating flows need a live (or merely stale)
    // price; a missing feed fails closed.
    s.t.advance_time(60 * 60 * 24 * 30);
    let client = s.client();

    // Partial withdrawal pays an arbitrary recipient without the tokens
    // ever touching the strategy.
    let sink = Address::generate(&s.t.env);
    let remaining = client.withdraw(&(300 * UNIT), &s.vault, &sink);
    assert_eq!(s.usdc_balance(&sink), 300 * UNIT);
    assert_eq!(s.usdc_balance(&s.client_address), 0);
    assert_eq!(client.balance(&s.vault), remaining);

    // Terminal exit: sole holder withdraws the full balance; the lending
    // account closes (the controller deletes it) and the recipient also
    // receives the forfeit backing.
    let account_before = client.lending_account_id();
    let balance = client.balance(&s.vault);
    let left = client.withdraw(&balance, &s.vault, &sink);
    assert_eq!(left, 0);
    assert_eq!(client.balance(&s.vault), 0);
    assert_eq!(client.total_shares(), 0);
    assert_eq!(client.lending_account_id(), 0);
    assert!(
        s.usdc_balance(&sink) >= 300 * UNIT + balance,
        "terminal payout must include the full balance plus forfeit backing"
    );

    // Re-deposit reopens a fresh lending account: the stored id from before
    // the full close would be stale, which is exactly the lifecycle
    // integrators must handle.
    client.deposit(&(500 * UNIT), &s.vault);
    let account_after = client.lending_account_id();
    assert!(account_after > account_before);
    assert!(client.balance(&s.vault) > 499 * UNIT);
}

#[test]
fn test_two_depositors_share_pro_rata() {
    let mut s = StrategyTest::new();

    let vault_b = Address::generate(&s.t.env);
    s.t.resolve_market("USDC")
        .token_admin
        .mint(&vault_b, &(10_000 * UNIT));

    s.client().deposit(&(1_000 * UNIT), &s.vault);
    s.t.advance_time(60 * 60 * 24 * 365);

    // B enters after a year of accrual: same nominal amount, fewer shares.
    let client = s.client();
    client.deposit(&(1_000 * UNIT), &vault_b);

    let a = client.balance(&s.vault);
    let b = client.balance(&vault_b);
    assert!(a > b, "earlier depositor must hold the accrued interest");
    assert!(
        b >= 1_000 * UNIT - 2,
        "late depositor enters at par minus rounding, got {b}"
    );
    assert!(client.shares(&s.vault) > client.shares(&vault_b));

    // The gap between the position and the holders' balances is the
    // forfeit backing (1000 stroops plus its accrued interest).
    let total = client.total_underlying();
    let gap = total - a - b;
    assert!(
        (1_000..=1_200).contains(&gap),
        "gap must be the forfeit backing, got {gap} ({total} vs {a} + {b})"
    );
}

#[test]
fn test_deposit_below_protocol_dust_floor_reverts() {
    let s = StrategyTest::new();
    let client = s.client();

    // $5 is below the market's $10 minimum collateral floor: the controller
    // rejects the supply, so the strategy call fails as a whole.
    assert!(client.try_deposit(&(5 * UNIT), &s.vault).is_err());
}

#[test]
fn test_max_withdrawable_tracks_controller_preview() {
    let s = StrategyTest::new();
    let client = s.client();
    client.deposit(&(1_000 * UNIT), &s.vault);

    let account_id = client.lending_account_id();
    let asset = s.t.resolve_asset("USDC");
    let expected = s.t.ctrl_client().max_withdraw(&account_id, &asset);
    assert_eq!(client.max_withdrawable(), expected);
    assert!(expected > 0);
}
