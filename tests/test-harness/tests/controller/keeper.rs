use controller::types::ControllerKey;
use soroban_sdk::{
    testutils::{ContractEvents, Events},
    xdr::{ContractEventBody, ScVal},
};
use test_harness::{
    assert_contract_error, days, errors, eth_preset, hub_asset, usd_cents, usdc_preset,
    HubAssetKey, LendingTest, ALICE, BOB, STABLECOIN_SPOKE,
};

fn supply_threshold_bps(t: &LendingTest, account_id: u64, asset_name: &str) -> u32 {
    let asset = t.resolve_asset(asset_name);
    t.env.as_contract(&t.controller_address(), || {
        let map: soroban_sdk::Map<HubAssetKey, controller::types::AccountPositionRaw> = t
            .env
            .storage()
            .persistent()
            .get(&ControllerKey::SupplyPositions(account_id))
            .expect("supply side map should exist");
        map.get(hub_asset(asset))
            .expect("supply position should exist for asset")
            .liquidation_threshold
    })
}

fn supply_fee_bps(t: &LendingTest, account_id: u64, asset_name: &str) -> u32 {
    let asset = t.resolve_asset(asset_name);
    t.env.as_contract(&t.controller_address(), || {
        let map: soroban_sdk::Map<HubAssetKey, controller::types::AccountPositionRaw> = t
            .env
            .storage()
            .persistent()
            .get(&ControllerKey::SupplyPositions(account_id))
            .expect("supply side map should exist");
        map.get(hub_asset(asset))
            .expect("supply position should exist for asset")
            .liquidation_fees
    })
}

fn supply_risk_fields(t: &LendingTest, account_id: u64, asset_name: &str) -> (u32, u32, u32) {
    let asset = t.resolve_asset(asset_name);
    t.env.as_contract(&t.controller_address(), || {
        let map: soroban_sdk::Map<HubAssetKey, controller::types::AccountPositionRaw> = t
            .env
            .storage()
            .persistent()
            .get(&ControllerKey::SupplyPositions(account_id))
            .expect("supply side map should exist");
        let p = map
            .get(hub_asset(asset))
            .expect("supply position should exist for asset");
        (
            p.liquidation_threshold,
            p.liquidation_bonus,
            p.loan_to_value,
        )
    })
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

#[test]
fn test_supply_roundtrip_preserves_risk_fields() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    t.supply(ALICE, "USDC", 1_000.0);
    let id = t.resolve_account_id(ALICE);
    let first = supply_risk_fields(&t, id, "USDC");
    assert!(
        first.0 > 0 && first.2 > 0,
        "preset should seed non-zero threshold/ltv; got {:?}",
        first
    );

    t.supply(ALICE, "USDC", 1_000.0);
    let second = supply_risk_fields(&t, id, "USDC");

    assert_eq!(
        first, second,
        "supply round-trip must preserve (threshold, bonus, ltv); pool return \
         merge zeroed risk fields"
    );
}
#[test]
fn test_update_indexes_refreshes_rates() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_dust_disabled_all_markets()
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 10.0);

    let borrow_before = t.borrow_balance(ALICE, "ETH");

    t.advance_and_sync(days(30));

    let borrow_after = t.borrow_balance(ALICE, "ETH");
    assert!(
        borrow_after > borrow_before,
        "borrow balance should increase after index update: before={}, after={}",
        borrow_before,
        borrow_after
    );
}
#[test]
fn test_clean_bad_debt_removes_positions() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_dust_disabled_all_markets()
        .build();

    t.supply(ALICE, "USDC", 10.0);
    t.borrow(ALICE, "ETH", 0.003);

    t.set_price("USDC", usd_cents(1));

    assert!(t.can_be_liquidated(ALICE), "Alice should be liquidatable");

    t.clean_bad_debt_for(ALICE);

    t.assert_no_positions(ALICE);
}
#[test]
fn test_clean_bad_debt_rejects_healthy() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_dust_disabled_all_markets()
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);
    t.assert_healthy(ALICE);

    let account_id = t.resolve_account_id(ALICE);
    let result = t.try_clean_bad_debt_by_id(account_id);
    assert_contract_error(result, errors::CANNOT_CLEAN_BAD_DEBT);
}
#[test]
fn test_clean_bad_debt_rejects_above_threshold() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_dust_disabled_all_markets()
        .build();

    t.supply(ALICE, "USDC", 1000.0);
    t.borrow(ALICE, "ETH", 0.3);

    t.set_price("USDC", usd_cents(50));

    assert!(t.can_be_liquidated(ALICE), "Alice should be liquidatable");

    let account_id = t.resolve_account_id(ALICE);
    let result = t.try_clean_bad_debt_by_id(account_id);
    assert_contract_error(result, errors::CANNOT_CLEAN_BAD_DEBT);
}

#[test]
fn test_bad_debt_gap_band_resolved_by_liquidation() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_dust_disabled_all_markets()
        .build();

    t.supply(ALICE, "USDC", 1000.0);
    t.borrow(ALICE, "ETH", 0.3);

    t.set_price("USDC", usd_cents(30));
    assert!(t.can_be_liquidated(ALICE), "account should be insolvent");

    let account_id = t.resolve_account_id(ALICE);
    let gated = t.try_clean_bad_debt_by_id(account_id);
    assert_contract_error(gated, errors::CANNOT_CLEAN_BAD_DEBT);

    t.liquidate(BOB, ALICE, "ETH", 0.3);

    t.assert_no_positions(ALICE);
    assert!(
        !t.can_be_liquidated_by_id(account_id),
        "account must be resolved after the gap-band liquidation"
    );
}

#[test]
fn test_clean_bad_debt_rejected_under_oracle_deviation() {
    use test_harness::TIGHT_TOLERANCE;

    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_dust_disabled_all_markets()
        .build();

    t.enable_dual_source_oracle("USDC");

    t.set_tolerance("USDC", TIGHT_TOLERANCE);

    t.supply(ALICE, "USDC", 10.0);
    t.borrow(ALICE, "ETH", 0.003);

    t.set_price("USDC", usd_cents(1));

    t.set_safe_price("USDC", usd_cents(100));

    let account_id = t.resolve_account_id(ALICE);
    let result = t.try_clean_bad_debt_by_id(account_id);
    assert_contract_error(result, errors::UNSAFE_PRICE);
}
#[test]
fn test_update_account_threshold_safe() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_dust_disabled_all_markets()
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    let hf_before = t.health_factor(ALICE);
    let account_id = t.resolve_account_id(ALICE);

    let (lt_before, bonus_before, _) = supply_risk_fields(&t, account_id, "USDC");
    let fee_before = supply_fee_bps(&t, account_id, "USDC");
    t.edit_asset_config("USDC", |c| {
        c.loan_to_value = 5_000;
        c.liquidation_bonus = bonus_before + 100;
        c.liquidation_fees = fee_before - 50;
    });

    t.update_account_threshold(false, &[account_id]);

    let (lt_after, bonus_after, ltv_after) = supply_risk_fields(&t, account_id, "USDC");
    assert_eq!(ltv_after, 5_000, "LTV propagates on the ungated path");
    assert_eq!(
        bonus_after, bonus_before,
        "a raised bonus must not propagate without the HF gate"
    );
    assert_eq!(
        supply_fee_bps(&t, account_id, "USDC"),
        fee_before,
        "a cut fee must not propagate without the HF gate"
    );
    assert_eq!(lt_after, lt_before, "threshold moves only with the tuple");

    t.assert_healthy(ALICE);

    let hf_after = t.health_factor(ALICE);
    assert!(
        hf_after >= 1.0,
        "HF should remain healthy after safe threshold update: before={}, after={}",
        hf_before,
        hf_after
    );
}
#[test]
fn test_update_account_threshold_risky() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_dust_disabled_all_markets()
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    let hf_before = t.health_factor(ALICE);
    let account_id = t.resolve_account_id(ALICE);

    t.update_account_threshold(true, &[account_id]);

    t.assert_healthy(ALICE);

    let hf_after = t.health_factor(ALICE);
    assert!(
        hf_after >= 1.0,
        "HF should remain healthy after risky threshold update: before={}, after={}",
        hf_before,
        hf_after
    );
}

#[test]
fn test_update_account_threshold_rejects_low_hf() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_dust_disabled_all_markets()
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);

    let account_id = t.resolve_account_id(ALICE);

    t.set_price("USDC", usd_cents(78));

    let result = t.try_update_account_threshold(true, &[account_id]);
    assert_contract_error(result, errors::HEALTH_FACTOR_TOO_LOW);
}

#[test]
fn test_update_account_threshold_propagates_adverse_tuple_to_healthy_account() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_dust_disabled_all_markets()
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    let account_id = t.resolve_account_id(ALICE);
    let (_, bonus_before, _) = supply_risk_fields(&t, account_id, "USDC");
    let fee_before = supply_fee_bps(&t, account_id, "USDC");

    t.edit_asset_config("USDC", |c| {
        c.loan_to_value = 5_000;
        c.liquidation_threshold = 6_100;
        c.liquidation_bonus = bonus_before + 500;
        c.liquidation_fees = fee_before - 50;
    });

    t.update_account_threshold(true, &[account_id]);

    let (lt_after, bonus_after, ltv_after) = supply_risk_fields(&t, account_id, "USDC");
    assert_eq!(
        (
            ltv_after,
            lt_after,
            bonus_after,
            supply_fee_bps(&t, account_id, "USDC")
        ),
        (5_000, 6_100, bonus_before + 500, fee_before - 50),
        "a healthy account takes the whole tuple, same vintage"
    );
}

#[test]
fn regression_third_party_keeper_cannot_force_adverse_tuple_below_min_hf() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_dust_disabled_all_markets()
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);

    let account_id = t.resolve_account_id(ALICE);
    let (lt_before, bonus_before, _) = supply_risk_fields(&t, account_id, "USDC");
    let fee_before = supply_fee_bps(&t, account_id, "USDC");
    assert_eq!(
        (lt_before, bonus_before, fee_before),
        (8_000, 500, 1_200),
        "preset tuple"
    );

    t.edit_asset_config("USDC", |c| {
        c.loan_to_value = 5_000;
        c.liquidation_threshold = 6_100;
        c.liquidation_bonus = 1_000;
        c.liquidation_fees = 50;
    });

    t.update_account_threshold(true, &[account_id]);

    let (lt_after, bonus_after, ltv_after) = supply_risk_fields(&t, account_id, "USDC");
    assert_eq!(ltv_after, 5_000, "LTV rides outside the gate");
    assert_eq!(
        (
            lt_after,
            bonus_after,
            supply_fee_bps(&t, account_id, "USDC")
        ),
        (lt_before, bonus_before, fee_before),
        "M1: threshold, bonus, and fees hold their vintage together under the HF floor"
    );
    assert!(
        !t.can_be_liquidated(ALICE),
        "the held tuple keeps the account out of liquidation"
    );
}
#[test]
fn test_update_account_threshold_deprecated_spoke_retains_spoke_params() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .with_dust_disabled_all_markets()
        .build();

    let account_id = t.create_spoke_account(ALICE, 2);
    t.supply_to(ALICE, account_id, "USDC", 1_000.0);

    assert_eq!(supply_threshold_bps(&t, account_id, "USDC"), 9800);

    t.remove_spoke_category(2);
    t.update_account_threshold(true, &[account_id]);

    assert_eq!(
        supply_threshold_bps(&t, account_id, "USDC"),
        9800,
        "a deprecated spoke's positions keep reading the spoke's own threshold (no spoke-0 fallback)"
    );
}

#[test]
fn test_update_account_threshold_syncs_all_supply_assets() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_dust_disabled_all_markets()
        .build();

    t.supply(ALICE, "USDC", 50_000.0);
    t.supply(ALICE, "ETH", 10.0);
    t.borrow(ALICE, "ETH", 0.5);

    let account_id = t.resolve_account_id(ALICE);
    let (usdc_threshold_before, _, _) = supply_risk_fields(&t, account_id, "USDC");
    let (eth_threshold_before, _, _) = supply_risk_fields(&t, account_id, "ETH");
    assert_ne!(usdc_threshold_before, 6100);
    assert_ne!(eth_threshold_before, 6100);

    t.edit_asset_config("USDC", |c| {
        c.loan_to_value = 5000;
        c.liquidation_threshold = 6100;
    });
    t.edit_asset_config("ETH", |c| {
        c.loan_to_value = 5000;
        c.liquidation_threshold = 6100;
    });

    t.update_account_threshold(true, &[account_id]);

    assert_eq!(supply_threshold_bps(&t, account_id, "USDC"), 6100);
    assert_eq!(supply_threshold_bps(&t, account_id, "ETH"), 6100);
    t.assert_healthy(ALICE);
}
#[test]
fn test_permissionless_keeper_endpoints() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_dust_disabled_all_markets()
        .build();

    let bob_addr = t.get_or_create_user(BOB);

    let ctrl = t.ctrl_client();
    let assets = soroban_sdk::vec![&t.env, hub_asset(t.resolve_market("USDC").asset.clone())];

    t.env.mock_all_auths();
    let result = ctrl.try_update_indexes(&bob_addr, &assets);
    assert!(result.is_ok(), "any signed caller may update_indexes");
}

#[test]
fn test_update_account_threshold_mixed_spokes_batch() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .build();

    t.supply(ALICE, "USDC", 1_000.0);
    t.create_spoke_account(BOB, 2);
    t.supply(BOB, "USDC", 1_000.0);

    let alice_id = t.resolve_account_id(ALICE);
    let bob_id = t.resolve_account_id(BOB);
    let (_, alice_bonus_before, alice_ltv_before) = supply_risk_fields(&t, alice_id, "USDC");
    let (_, bob_bonus_before, _) = supply_risk_fields(&t, bob_id, "USDC");

    t.edit_asset_in_spoke("USDC", 2, true, true, 9600, 9700, 300);

    t.update_account_threshold(false, &[alice_id, bob_id]);

    let (_, bob_bonus, bob_ltv) = supply_risk_fields(&t, bob_id, "USDC");
    assert_eq!(bob_ltv, 9600, "BOB must sync spoke-2 LTV");
    assert_eq!(
        bob_bonus, bob_bonus_before,
        "the ungated path must leave BOB's bonus alone"
    );

    let (_, alice_bonus, alice_ltv) = supply_risk_fields(&t, alice_id, "USDC");
    assert_eq!(
        (alice_bonus, alice_ltv),
        (alice_bonus_before, alice_ltv_before),
        "ALICE must keep base-spoke params"
    );
}

#[test]
fn test_update_account_threshold_rejects_bonus_raise_below_min_hf() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_dust_disabled_all_markets()
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    let account_id = t.resolve_account_id(ALICE);
    let (_, bonus_before, _) = supply_risk_fields(&t, account_id, "USDC");

    t.set_price("USDC", usd_cents(78));
    t.edit_asset_config("USDC", |c| c.liquidation_bonus = bonus_before + 500);

    let result = t.try_update_account_threshold(true, &[account_id]);
    assert_contract_error(result, errors::HEALTH_FACTOR_TOO_LOW);

    let (_, bonus_after, _) = supply_risk_fields(&t, account_id, "USDC");
    assert_eq!(
        bonus_after, bonus_before,
        "the rejected batch must leave the stamp untouched"
    );
}

#[test]
fn test_update_account_threshold_skips_param_upd_when_stamps_unchanged() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_dust_disabled_all_markets()
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);
    let account_id = t.resolve_account_id(ALICE);
    let stamps_before = supply_risk_fields(&t, account_id, "USDC");

    let batches_before = count_topic(&t.env.events().all(), "position", "batch_update");
    t.update_account_threshold(true, &[account_id]);
    let batches_after = count_topic(&t.env.events().all(), "position", "batch_update");

    assert_eq!(
        batches_after, batches_before,
        "noop restamp must not emit position:batch_update / ParamUpd"
    );
    assert_eq!(
        supply_risk_fields(&t, account_id, "USDC"),
        stamps_before,
        "stamps stay put when listing already matches"
    );
}

#[test]
fn test_update_account_threshold_emits_param_upd_only_for_changed_assets() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_dust_disabled_all_markets()
        .build();

    t.supply(ALICE, "USDC", 50_000.0);
    t.supply(ALICE, "ETH", 10.0);
    t.borrow(ALICE, "ETH", 0.5);
    let account_id = t.resolve_account_id(ALICE);
    let eth_before = supply_risk_fields(&t, account_id, "ETH");

    // Only USDC listing moves; ETH stamps already match listing.
    t.edit_asset_config("USDC", |c| {
        c.loan_to_value = 5_000;
        c.liquidation_threshold = 6_100;
    });

    let batches_before = count_topic(&t.env.events().all(), "position", "batch_update");
    t.update_account_threshold(true, &[account_id]);
    let events = t.env.events().all();
    let batches_after = count_topic(&events, "position", "batch_update");
    assert_eq!(
        batches_after,
        batches_before + 1,
        "changed stamps emit one batch"
    );

    let batches = data_for_topic(&events, "position", "batch_update");
    let data = as_vec(batches.last().expect("param batch"));
    let deposits = as_vec(&data[2]);
    assert_eq!(
        deposits.len(),
        1,
        "only the changed supply asset should carry ParamUpd"
    );
    let entry = as_vec(&deposits[0]);
    // PositionAction::ParamUpd = 7
    assert_eq!(entry[0], ScVal::U32(7), "action discriminant is ParamUpd");
    assert_eq!(entry[8], ScVal::U32(5_000), "updated LTV in event payload");
    assert_eq!(
        entry[6],
        ScVal::U32(6_100),
        "updated threshold in event payload"
    );

    assert_eq!(supply_threshold_bps(&t, account_id, "USDC"), 6_100);
    assert_eq!(
        supply_risk_fields(&t, account_id, "ETH"),
        eth_before,
        "unchanged asset must keep its stamps and skip ParamUpd"
    );
}
