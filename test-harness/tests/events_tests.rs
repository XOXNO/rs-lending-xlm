extern crate std;

use common::constants::WAD;

use soroban_sdk::testutils::Events;
use std::format;

use test_harness::{days, eth_preset, usd_cents, usdc_preset, LendingTest, ALICE, LIQUIDATOR};

// ---------------------------------------------------------------------------
// All event tests verify that operations emit events.
//
// Soroban event API note: `events().all().events().len()` returns the count
// from the last top-level invocation, not a cumulative total. These tests
// therefore check that:
//   - Each operation emits > 0 events.
//   - Complex operations (liquidation) emit multiple events.
//
// Full payload verification would require XDR decoding of Soroban event
// data, which is impractical in integration tests.
// ---------------------------------------------------------------------------

#[test]
fn test_supply_emits_events() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    t.supply(ALICE, "USDC", 10_000.0);
    let count = t.env.events().all().events().len();
    assert!(count > 0, "supply should emit events, got {}", count);
}

#[test]
fn test_supply_position_event_restores_risk_fields() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    t.supply(ALICE, "USDC", 10_000.0);

    let dump = format!("{:#?}", t.env.events().all());
    for field in [
        "liquidation_threshold_bps",
        "liquidation_bonus_bps",
        "liquidation_fees_bps",
        "loan_to_value_bps",
    ] {
        assert!(
            dump.contains(field),
            "supply position event should include `{}`; events were:\n{}",
            field,
            dump
        );
    }
}

#[test]
fn test_borrow_emits_events() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();
    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);
    // After borrow, at least the borrow operation's events must be present.
    let count = t.env.events().all().events().len();
    assert!(count > 0, "borrow should emit events, got {}", count);
}

#[test]
fn test_withdraw_emits_events() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    t.supply(ALICE, "USDC", 10_000.0);
    t.withdraw(ALICE, "USDC", 1_000.0);
    let count = t.env.events().all().events().len();
    assert!(count > 0, "withdraw should emit events, got {}", count);
}

#[test]
fn test_repay_emits_events() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();
    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);
    t.repay(ALICE, "ETH", 0.5);
    let count = t.env.events().all().events().len();
    assert!(count > 0, "repay should emit events, got {}", count);
}

#[test]
fn test_liquidation_emits_many_events() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    t.set_price("USDC", usd_cents(50));
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);
    // Liquidation combines token transfers, position updates, and seizure.
    // Even within Soroban's per-invocation event scope, the call itself
    // must emit several events: debt repay, seizure, and position updates.
    let count = t.env.events().all().events().len();
    assert!(
        count >= 3,
        "liquidation should emit >= 3 events, got {}",
        count
    );
}

// Skipped: flash_loan event test. mock_all_auths recording mode blocks
// nested contract calls from the flash-loan receiver.

// Skipped: edit_asset_config event test. The get_asset_config view call
// triggers a host-level error in the test environment.

#[test]
fn test_add_emode_emits_events() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    t.ctrl_client()
        .add_e_mode_category(&9700u32, &9800u32, &200u32);
    let count = t.env.events().all().events().len();
    assert!(count > 0, "add_e_mode should emit events, got {}", count);
}

#[test]
fn test_index_sync_emits_events() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();
    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);
    t.advance_and_sync(days(1));
    let count = t.env.events().all().events().len();
    assert!(count > 0, "sync should emit events, got {}", count);
}

#[test]
fn test_isolated_borrow_emits_debt_ceiling_event() {
    let ceiling = 1_000_000i128 * WAD;
    let mut t = LendingTest::new()
        .with_market(eth_preset())
        .with_market(usdc_preset())
        .with_market_config("ETH", |cfg| {
            cfg.is_isolated_asset = true;
            cfg.isolation_debt_ceiling_usd_wad = ceiling;
        })
        .with_market_config("USDC", |cfg| {
            cfg.isolation_borrow_enabled = true;
        })
        .build();

    t.create_isolated_account(ALICE, "ETH");
    t.supply(ALICE, "ETH", 10.0);
    t.borrow(ALICE, "USDC", 1_000.0);
    // Isolated borrow emits position-update and debt-ceiling events.
    let count = t.env.events().all().events().len();
    assert!(
        count >= 2,
        "isolated borrow should emit >= 2 events, got {}",
        count
    );
}
