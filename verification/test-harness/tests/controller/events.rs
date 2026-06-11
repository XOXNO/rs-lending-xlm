use common::constants::WAD;

use soroban_sdk::{
    testutils::{ContractEvents, Events},
    xdr::{ContractEventBody, ScVal},
};

use test_harness::{
    days, eth_preset, usd_cents, usdc_preset, usdt_stable_preset, wbtc_preset, xlm_preset,
    LendingTest, ALICE, LIQUIDATOR,
};

/// Returns the data payload of every event matching the 2-symbol topic.
fn data_for_topic(events: &ContractEvents, first: &str, second: &str) -> std::vec::Vec<ScVal> {
    events
        .events()
        .iter()
        .filter_map(|event| {
            let ContractEventBody::V0(body) = &event.body;
            match (body.topics.first(), body.topics.get(1)) {
                (Some(ScVal::Symbol(a)), Some(ScVal::Symbol(b)))
                    if a.0.to_string() == first && b.0.to_string() == second =>
                {
                    Some(body.data.clone())
                }
                _ => None,
            }
        })
        .collect()
}

fn as_vec(v: &ScVal) -> &soroban_sdk::xdr::VecM<ScVal> {
    match v {
        ScVal::Vec(Some(entries)) => &entries.0,
        other => panic!("expected ScVal::Vec, got {:?}", other),
    }
}

fn count_topic(events: &ContractEvents, first: &str, second: &str) -> usize {
    events
        .events()
        .iter()
        .filter(|event| {
            let ContractEventBody::V0(body) = &event.body;
            match (body.topics.first(), body.topics.get(1)) {
                (Some(ScVal::Symbol(a)), Some(ScVal::Symbol(b))) => {
                    a.0.to_string() == first && b.0.to_string() == second
                }
                _ => false,
            }
        })
        .count()
}
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

#[test]
fn test_supply_emits_events() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    t.supply(ALICE, "USDC", 10_000.0);
    let count = t.env.events().all().events().len();
    assert!(count > 0, "supply should emit events, got {}", count);
}

#[test]
fn test_bulk_supply_emits_single_position_and_market_batch() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .with_market(xlm_preset())
        .build();

    t.supply_bulk(
        ALICE,
        &[
            ("USDC", 1_000.0),
            ("USDT", 1_000.0),
            ("ETH", 1.0),
            ("WBTC", 0.1),
            ("XLM", 1_000.0),
        ],
    );

    let events = t.env.events().all();
    assert_eq!(
        count_topic(&events, "position", "batch_update"),
        1,
        "bulk supply should emit one position batch"
    );
    assert_eq!(
        count_topic(&events, "market", "batch_state_update"),
        1,
        "bulk supply should emit one market batch"
    );
    assert_eq!(
        count_topic(&events, "position", "update"),
        0,
        "legacy position:update must not be emitted"
    );
    assert_eq!(
        count_topic(&events, "market", "state_update"),
        0,
        "legacy market:state_update must not be emitted"
    );
    assert_eq!(
        events.events().len(),
        7,
        "bulk supply should emit five token transfers plus two batch events"
    );
}

#[test]
fn test_supply_position_event_restores_risk_fields() {
    // V2 wire ABI: deposit entries are vec-encoded as
    // [action, asset, scaled_amount_ray, index_ray, amount, lt_bps, lb_bps, ltv_bps].
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    t.supply(ALICE, "USDC", 10_000.0);

    let events = t.env.events().all();
    let batches = data_for_topic(&events, "position", "batch_update");
    assert_eq!(batches.len(), 1);
    let data = as_vec(&batches[0]);
    let deposits = as_vec(&data[2]);
    assert_eq!(deposits.len(), 1, "one deposit delta for the supply");
    let entry = as_vec(&deposits[0]);
    assert_eq!(entry.len(), 8, "deposit delta arity is wire ABI");
    // usdc_preset risk params: threshold 8000, bonus 500, ltv 7500.
    assert_eq!(entry[5], ScVal::U32(8000), "liquidation_threshold_bps");
    assert_eq!(entry[6], ScVal::U32(500), "liquidation_bonus_bps");
    assert_eq!(entry[7], ScVal::U32(7500), "loan_to_value_bps");
}

#[test]
fn test_position_and_market_batch_v2_wire_shape() {
    // Locks the full v2 wire layout on a liquidation, which exercises both
    // sides of the position batch (seize deposits + repay borrows) and the
    // market batch in one transaction.
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    t.set_price("USDC", usd_cents(50));
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);

    let events = t.env.events().all();

    // position:batch_update data = [account_id, attrs, deposits, borrows].
    let batches = data_for_topic(&events, "position", "batch_update");
    assert_eq!(batches.len(), 1);
    let data = as_vec(&batches[0]);
    assert_eq!(data.len(), 4, "position batch arity is wire ABI");
    assert!(matches!(data[0], ScVal::U64(_)), "account_id");
    let attrs = as_vec(&data[1]);
    assert_eq!(attrs.len(), 5, "attrs arity is wire ABI");
    assert!(matches!(attrs[0], ScVal::Address(_)), "attrs.owner");
    assert!(matches!(attrs[1], ScVal::U32(_)), "attrs.e_mode_category_id");
    assert!(matches!(attrs[2], ScVal::Bool(_)), "attrs.is_isolated_position");
    assert!(matches!(attrs[3], ScVal::U32(_)), "attrs.mode");
    let deposits = as_vec(&data[2]);
    let borrows = as_vec(&data[3]);
    assert!(!deposits.is_empty(), "liquidation seizes collateral");
    assert!(!borrows.is_empty(), "liquidation repays debt");
    for d in deposits.iter() {
        let entry = as_vec(d);
        assert_eq!(entry.len(), 8, "deposit delta arity");
        // PositionAction::LiqSeize = 5.
        assert_eq!(entry[0], ScVal::U32(5), "seize action discriminant");
        assert!(matches!(entry[1], ScVal::Address(_)));
        assert!(matches!(entry[2], ScVal::I128(_)));
    }
    for b in borrows.iter() {
        let entry = as_vec(b);
        assert_eq!(entry.len(), 5, "borrow delta arity");
        // PositionAction::LiqRepay = 4.
        assert_eq!(entry[0], ScVal::U32(4), "repay action discriminant");
    }

    // market:batch_state_update data = vec of 9-element entries.
    let market = data_for_topic(&events, "market", "batch_state_update");
    assert_eq!(market.len(), 1);
    let updates = as_vec(&market[0]);
    assert!(updates.len() >= 2, "both touched markets snapshot");
    for u in updates.iter() {
        let entry = as_vec(u);
        assert_eq!(entry.len(), 9, "market entry arity is wire ABI");
        assert!(matches!(entry[0], ScVal::Address(_)), "asset");
        assert!(matches!(entry[1], ScVal::U64(_)), "timestamp");
        assert!(matches!(entry[2], ScVal::I128(_)), "supply_index_ray");
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
