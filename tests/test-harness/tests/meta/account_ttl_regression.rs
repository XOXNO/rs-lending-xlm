use controller::types::ControllerKey;
use soroban_sdk::testutils::storage::Persistent as _;
use test_harness::{eth_preset, usdc_preset, LendingTest, ALICE};

// One-sided activity must renew both side TTLs. Open both sides, then
// advance the ledger sequence to age the keys, then mutate only the
// supply side. The borrow side must keep its TTL fresh.
#[test]
fn test_one_sided_activity_renews_both_side_ttls() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 1_000.0);
    t.borrow(ALICE, "ETH", 0.1);

    let account_id = t.resolve_account_id(ALICE);
    let supply_key = ControllerKey::SupplyPositions(account_id);
    let borrow_key = ControllerKey::BorrowPositions(account_id);

    // Read initial TTLs of both side keys.
    let (initial_supply_ttl, initial_borrow_ttl) = t.env.as_contract(&t.controller, || {
        let p = t.env.storage().persistent();
        (
            p.get_ttl::<ControllerKey>(&supply_key),
            p.get_ttl::<ControllerKey>(&borrow_key),
        )
    });
    assert!(
        initial_supply_ttl > 0 && initial_borrow_ttl > 0,
        "both side keys must be live: supply_ttl={}, borrow_ttl={}",
        initial_supply_ttl,
        initial_borrow_ttl
    );

    // Advance the ledger sequence past `TTL_THRESHOLD_USER` (~30 days)
    // so the next renew call actually bumps the TTL. (`extend_ttl` is a
    // no-op when the current TTL is already above the threshold.)
    // 95 days at 5 s/ledger drops both keys' TTL below the threshold.
    t.advance_time(60 * 60 * 24 * 95);

    let (aged_supply_ttl, aged_borrow_ttl) = t.env.as_contract(&t.controller, || {
        let p = t.env.storage().persistent();
        (
            p.get_ttl::<ControllerKey>(&supply_key),
            p.get_ttl::<ControllerKey>(&borrow_key),
        )
    });
    assert!(
        aged_supply_ttl < initial_supply_ttl,
        "supply TTL must have decreased after time advance: aged={}, initial={}",
        aged_supply_ttl,
        initial_supply_ttl
    );
    assert!(
        aged_borrow_ttl < initial_borrow_ttl,
        "borrow TTL must have decreased after time advance: aged={}, initial={}",
        aged_borrow_ttl,
        initial_borrow_ttl
    );

    // Supply-only mutation must renew both side TTLs.
    t.supply(ALICE, "USDC", 100.0);

    let (renewed_supply_ttl, renewed_borrow_ttl) = t.env.as_contract(&t.controller, || {
        let p = t.env.storage().persistent();
        (
            p.get_ttl::<ControllerKey>(&supply_key),
            p.get_ttl::<ControllerKey>(&borrow_key),
        )
    });

    assert!(
        renewed_supply_ttl > aged_supply_ttl,
        "supply TTL must have been renewed by the supply call: renewed={}, aged={}",
        renewed_supply_ttl,
        aged_supply_ttl
    );

    // Counterpart borrow side renews on supply-only activity.
    assert!(
        renewed_borrow_ttl > aged_borrow_ttl,
        "borrow TTL must have been renewed by the supply call (counterpart-side renewal): renewed={}, aged={}",
        renewed_borrow_ttl,
        aged_borrow_ttl
    );

    // Symmetric: repay (borrow-side) renews supply-side TTL.
    t.advance_time(60 * 60 * 24 * 95);
    let (aged_supply_ttl_2, aged_borrow_ttl_2) = t.env.as_contract(&t.controller, || {
        let p = t.env.storage().persistent();
        (
            p.get_ttl::<ControllerKey>(&supply_key),
            p.get_ttl::<ControllerKey>(&borrow_key),
        )
    });
    t.repay(ALICE, "ETH", 0.01);
    let (final_supply_ttl, final_borrow_ttl) = t.env.as_contract(&t.controller, || {
        let p = t.env.storage().persistent();
        (
            p.get_ttl::<ControllerKey>(&supply_key),
            p.get_ttl::<ControllerKey>(&borrow_key),
        )
    });
    assert!(
        final_supply_ttl > aged_supply_ttl_2,
        "supply TTL must have been renewed by the repay call (counterpart-side renewal): renewed={}, aged={}",
        final_supply_ttl,
        aged_supply_ttl_2
    );
    assert!(
        final_borrow_ttl > aged_borrow_ttl_2,
        "borrow TTL must have been renewed by the repay call: renewed={}, aged={}",
        final_borrow_ttl,
        aged_borrow_ttl_2
    );
}
