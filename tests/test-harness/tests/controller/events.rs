use crate::shared::{as_vec, count_topic, data_for_topic};
use common::types::SeizeMode;
use controller::constants::WAD;
use soroban_sdk::{testutils::Events, xdr::ScVal};

use test_harness::{
    days, eth_preset, hub_asset, usd_cents, usdc_preset, usdt_stable_preset, wbtc_preset,
    xlm_preset, LendingTest, ALICE, LIQUIDATOR,
};

#[test]
fn test_supply_emits_events() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    t.supply(ALICE, "USDC", 10_000.0);
    let events = t.env.events().all();
    assert_eq!(
        count_topic(&events, "position", "batch_update"),
        1,
        "supply must emit exactly one position batch"
    );
    assert_eq!(
        count_topic(&events, "market", "batch_state_update"),
        1,
        "supply must emit exactly one market batch"
    );
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

    // Pre-create ALICE's account as its own top-level call so the
    // position-NFT `mint` event it emits lands outside the window
    // `events().all()` inspects below (that helper returns only events
    // from the *last* contract invocation). Otherwise a fresh account_id=0
    // supply_bulk would fold account creation's mint event into the count
    // this test is actually about: how many events supply_bulk itself
    // emits.
    t.create_account(ALICE);

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
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    t.supply(ALICE, "USDC", 10_000.0);

    let events = t.env.events().all();
    let batches = data_for_topic(&events, "position", "batch_update");
    assert_eq!(batches.len(), 1);
    let data = as_vec(&batches[0]);
    let deposits = as_vec(&data[2]);
    assert_eq!(deposits.len(), 1, "one deposit delta for the supply");
    let entry = as_vec(&deposits[0]);
    assert_eq!(entry.len(), 10, "deposit delta arity is wire ABI");
    assert_eq!(entry[1], ScVal::U32(1), "hub_id");

    assert_eq!(entry[6], ScVal::U32(8000), "liquidation_threshold");
    assert_eq!(entry[7], ScVal::U32(500), "liquidation_bonus");
    assert_eq!(entry[8], ScVal::U32(7500), "loan_to_value");
    assert_eq!(
        entry[9],
        ScVal::U32(t.get_asset_config("USDC").liquidation_fees),
        "liquidation_fees"
    );
}

#[test]
fn test_position_and_market_batch_v2_wire_shape() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    t.set_price("USDC", usd_cents(50));
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);

    let events = t.env.events().all();

    let batches = data_for_topic(&events, "position", "batch_update");
    assert_eq!(batches.len(), 1);
    let data = as_vec(&batches[0]);
    assert_eq!(data.len(), 4, "position batch arity is wire ABI");
    assert!(matches!(data[0], ScVal::U64(_)), "account_id");
    let attrs = as_vec(&data[1]);
    assert_eq!(attrs.len(), 3, "attrs arity is wire ABI");
    assert!(matches!(attrs[0], ScVal::Address(_)), "attrs.owner");
    assert!(matches!(attrs[1], ScVal::U32(_)), "attrs.spoke_id");
    assert!(matches!(attrs[2], ScVal::U32(_)), "attrs.mode");
    let deposits = as_vec(&data[2]);
    let borrows = as_vec(&data[3]);
    assert!(!deposits.is_empty(), "liquidation seizes collateral");
    assert!(!borrows.is_empty(), "liquidation repays debt");
    for d in deposits.iter() {
        let entry = as_vec(d);
        assert_eq!(entry.len(), 10, "deposit delta arity");

        assert_eq!(entry[0], ScVal::U32(5), "seize action discriminant");
        assert!(matches!(entry[1], ScVal::U32(_)), "hub_id");
        assert!(matches!(entry[2], ScVal::Address(_)), "asset");
        assert!(matches!(entry[3], ScVal::I128(_)), "scaled_amount");
    }
    for b in borrows.iter() {
        let entry = as_vec(b);
        assert_eq!(entry.len(), 6, "borrow delta arity");

        assert_eq!(entry[0], ScVal::U32(4), "repay action discriminant");
        assert!(matches!(entry[1], ScVal::U32(_)), "hub_id");
        assert!(matches!(entry[2], ScVal::Address(_)), "asset");
    }

    let market = data_for_topic(&events, "market", "batch_state_update");
    assert!(!market.is_empty(), "pool emits market snapshots");
    let mut market_entries = 0;
    for m in market.iter() {
        let updates = as_vec(m);
        for u in updates.iter() {
            let entry = as_vec(u);
            assert_eq!(entry.len(), 9, "market entry arity is wire ABI");
            assert!(matches!(entry[0], ScVal::U32(_)), "hub_id");
            assert!(matches!(entry[1], ScVal::Address(_)), "asset");
            assert!(matches!(entry[2], ScVal::U64(_)), "timestamp");
            assert!(matches!(entry[3], ScVal::I128(_)), "supply_index");
            market_entries += 1;
        }
    }
    assert!(market_entries >= 2, "both touched markets are snapshotted");
}

#[test]
fn test_borrow_emits_events() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    let events = t.env.events().all();
    assert_eq!(
        count_topic(&events, "position", "batch_update"),
        1,
        "borrow must emit exactly one position batch"
    );
}

#[test]
fn test_withdraw_emits_events() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    t.supply(ALICE, "USDC", 10_000.0);
    t.withdraw(ALICE, "USDC", 1_000.0);
    let events = t.env.events().all();
    assert_eq!(
        count_topic(&events, "position", "batch_update"),
        1,
        "withdraw must emit exactly one position batch"
    );
}

#[test]
fn test_repay_emits_events() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);
    t.repay(ALICE, "ETH", 0.5);
    let events = t.env.events().all();
    assert_eq!(
        count_topic(&events, "position", "batch_update"),
        1,
        "repay must emit exactly one position batch"
    );
}

#[test]
fn test_liquidation_emits_many_events() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    t.set_price("USDC", usd_cents(50));

    let account_id = t.resolve_account_id(ALICE);
    let payments =
        soroban_sdk::Vec::from_array(&t.env, [(hub_asset(t.resolve_asset("ETH")), 1_0000000)]);
    let bonus_bps = t
        .ctrl_client()
        .get_liquidation_estimate(&account_id, &payments, &SeizeMode::Transfer)
        .bonus_rate_bps;
    let liquidator = t.get_or_create_user(LIQUIDATOR);

    t.liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);

    let events = t.env.events().all();
    assert_eq!(
        count_topic(&events, "position", "batch_update"),
        1,
        "liquidation must emit exactly one position batch"
    );
    assert_eq!(
        count_topic(&events, "position", "liquidation"),
        1,
        "liquidation must emit exactly one liquidation event"
    );

    // Liquidator bots read this payload; a zeroed, swapped or mis-scaled field
    // must not survive a topic-only count.
    let liquidations = data_for_topic(&events, "position", "liquidation");
    let ScVal::Map(Some(map)) = &liquidations[0] else {
        panic!("liquidation event data is a map, got {:?}", liquidations[0]);
    };
    assert_eq!(map.len(), 4, "liquidation event arity is wire ABI");
    let field = |name: &str| -> &ScVal {
        &map.iter()
            .find(|e| matches!(&e.key, ScVal::Symbol(s) if s.0.to_string() == name))
            .unwrap_or_else(|| panic!("liquidation event has no field `{name}`"))
            .val
    };
    assert_eq!(*field("liquidator"), ScVal::from(&liquidator), "liquidator");
    assert_eq!(*field("account_id"), ScVal::U64(account_id), "account_id");
    let ScVal::I128(repaid) = field("repaid_usd_wad") else {
        panic!("repaid_usd_wad must be i128");
    };
    // 1.0 ETH at the $2 000 preset price, retired in full; the USD conversion
    // floors, so allow a sub-milli-dollar shortfall but nothing near a rescale.
    let repaid = i128::from(repaid);
    assert!(
        (repaid - 2_000 * WAD).abs() < WAD / 1_000,
        "repaid_usd_wad must be ~2 000 USD in WAD, got {repaid}"
    );
    let ScVal::I128(bonus) = field("bonus_bps") else {
        panic!("bonus_bps must be i128");
    };
    assert!(bonus_bps > 0, "estimate must be live");
    assert_eq!(i128::from(bonus), bonus_bps, "bonus_bps");
}

#[test]
fn test_add_spoke_emits_events() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    t.ctrl_client().add_spoke();
    let events = t.env.events().all();
    assert_eq!(
        count_topic(&events, "config", "spoke"),
        1,
        "add_spoke must emit exactly one config:spoke event"
    );
}

#[test]
fn test_index_sync_emits_events() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);
    t.advance_and_sync(days(1));
    let events = t.env.events().all();
    assert_eq!(
        count_topic(&events, "market", "batch_state_update"),
        2,
        "syncing both markets must emit one market batch per pool sync call"
    );
}
