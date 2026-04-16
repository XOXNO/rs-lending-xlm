extern crate std;

use soroban_sdk::testutils::Events;
use std::format;

use test_harness::{days, eth_preset, usd_cents, usdc_preset, LendingTest, ALICE, LIQUIDATOR};

// ---------------------------------------------------------------------------
// All event tests verify that operations produce events.
//
// NOTE on Soroban event API: `events().all().events().len()` returns the
// count from the last top-level invocation, not a cumulative total. These
// tests therefore verify that:
//   - Each operation produces > 0 events
//   - Complex operations (liquidation) produce multiple events
//
// Full event payload verification would require XDR decoding of Soroban
// event data, which is impractical in integration tests.
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
    // After borrow, at least the borrow operation's events should be present
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
    // Liquidation is a complex operation: token transfers + position updates + seizure.
    // Even with Soroban's per-invocation event scope, the liquidation call
    // itself should produce multiple events (debt repay + seizure + position updates).
    let count = t.env.events().all().events().len();
    assert!(
        count >= 3,
        "liquidation should emit >= 3 events, got {}",
        count
    );
}

// Note: flash_loan event test skipped — mock_all_auths recording mode
// blocks nested contract calls from the flash loan receiver.

// Note: edit_asset_config event test skipped — get_asset_config view
// call triggers host-level error in test environment.

#[test]
fn test_add_emode_emits_events() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    t.ctrl_client()
        .add_e_mode_category(&9700i128, &9800i128, &200i128);
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
    let ceiling = 1_000_000i128 * 1_000_000_000_000_000_000i128;
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
    // Isolated borrow emits position update + debt ceiling tracking events
    let count = t.env.events().all().events().len();
    assert!(
        count >= 2,
        "isolated borrow should emit >= 2 events, got {}",
        count
    );
}
