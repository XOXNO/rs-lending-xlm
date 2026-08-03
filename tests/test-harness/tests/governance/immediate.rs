use governance::op::{AdminOperation, RoleArgs, SpokeAssetArgs};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Symbol};
use test_harness::{
    assert_contract_error, errors, hub_asset, usd, usdc_preset, LendingTest, ALICE, HARNESS_HUB,
    HARNESS_SPOKE,
};

fn flatten<T, C>(
    result: Result<Result<T, C>, Result<soroban_sdk::Error, soroban_sdk::InvokeError>>,
) -> Result<(), soroban_sdk::Error> {
    match result {
        Ok(_) => Ok(()),
        Err(Ok(err)) => Err(err),
        Err(Err(_)) => panic!("expected contract error, got InvokeError"),
    }
}

#[test]
fn guardian_sets_spoke_asset_flags_immediately() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    let admin = t.admin();
    let usdc = t.resolve_asset("USDC");

    let before = t
        .ctrl_client()
        .get_spoke_asset(&HARNESS_SPOKE, &hub_asset(usdc.clone()));

    t.gov_iface_client().set_spoke_asset_flags(
        &admin,
        &HARNESS_SPOKE,
        &hub_asset(usdc.clone()),
        &true,
        &false,
    );

    let after = t
        .ctrl_client()
        .get_spoke_asset(&HARNESS_SPOKE, &hub_asset(usdc.clone()));
    assert!(after.paused, "paused flag must flip");
    assert!(!after.frozen);
    assert_eq!(after.loan_to_value, before.loan_to_value);
    assert_eq!(after.supply_cap, before.supply_cap);
    assert_eq!(after.liquidation_threshold, before.liquidation_threshold);

    assert_contract_error(
        t.try_supply(ALICE, "USDC", 10.0),
        errors::SPOKE_ASSET_PAUSED,
    );

    t.gov_iface_client().set_spoke_asset_flags(
        &admin,
        &HARNESS_SPOKE,
        &hub_asset(usdc.clone()),
        &true,
        &true,
    );

    let relax = t.gov_iface_client().try_set_spoke_asset_flags(
        &admin,
        &HARNESS_SPOKE,
        &hub_asset(usdc.clone()),
        &false,
        &false,
    );
    assert_contract_error(flatten(relax), errors::SPOKE_ASSET_FLAG_RELAXATION);

    let relax_frozen = t.gov_iface_client().try_set_spoke_asset_flags(
        &admin,
        &HARNESS_SPOKE,
        &hub_asset(usdc.clone()),
        &true,
        &false,
    );
    assert_contract_error(flatten(relax_frozen), errors::SPOKE_ASSET_FLAG_RELAXATION);
    assert_contract_error(
        t.try_supply(ALICE, "USDC", 10.0),
        errors::SPOKE_ASSET_PAUSED,
    );

    let cfg = t
        .ctrl_client()
        .get_spoke_asset(&HARNESS_SPOKE, &hub_asset(usdc.clone()));
    t.gov_client().execute_immediate(
        &admin,
        &AdminOperation::EditAssetInSpoke(SpokeAssetArgs {
            hub_id: HARNESS_HUB,
            asset: usdc.clone(),
            spoke_id: HARNESS_SPOKE,
            can_collateral: cfg.is_collateralizable,
            can_borrow: cfg.is_borrowable,
            paused: false,
            frozen: false,
            ltv: cfg.loan_to_value,
            threshold: cfg.liquidation_threshold,
            bonus: cfg.liquidation_bonus,
            liquidation_fees: cfg.liquidation_fees,
            supply_cap: cfg.supply_cap,
            borrow_cap: cfg.borrow_cap,
        }),
    );
    assert!(
        t.try_supply(ALICE, "USDC", 10.0).is_ok(),
        "timelocked edit must re-open supply"
    );
}

#[test]
fn non_guardian_flags_rejected() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let gov = t.gov_iface_client();
    let stranger = Address::generate(&t.env);
    let usdc = t.resolve_asset("USDC");

    let result =
        gov.try_set_spoke_asset_flags(&stranger, &HARNESS_SPOKE, &hub_asset(usdc), &true, &false);
    assert_contract_error(flatten(result), errors::UNAUTHORIZED);
}

#[test]
fn oracle_role_moves_sanity_band_containing_price() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let gov = t.gov_iface_client();
    let admin = t.admin();
    let usdc = t.resolve_asset("USDC");

    let before = t.market_oracle_config(&usdc);

    let min = usd(1) * 95 / 100;
    let max = usd(1) * 105 / 100;
    gov.set_sanity_band(
        &admin,
        &controller::types::PriceKey::Token(usdc.clone()),
        &min,
        &max,
    );

    let after = t.market_oracle_config(&usdc);
    assert_eq!(after.min_sanity_price_wad, min);
    assert_eq!(after.max_sanity_price_wad, max);
    assert_eq!(
        after.max_price_stale_seconds,
        before.max_price_stale_seconds
    );
    assert_eq!(after.tolerance, before.tolerance);
}

#[test]
fn sanity_band_not_containing_price_fails_closed_at_read() {
    for (min_wad, max_wad) in [
        (usd(1) * 1005 / 1000, usd(1) * 105 / 100),
        (usd(1) * 95 / 100, usd(1) * 995 / 1000),
    ] {
        let t = LendingTest::new().with_market(usdc_preset()).build();
        let gov = t.gov_iface_client();
        let admin = t.admin();
        let usdc = t.resolve_asset("USDC");

        flatten(gov.try_set_sanity_band(
            &admin,
            &controller::types::PriceKey::Token(usdc.clone()),
            &min_wad,
            &max_wad,
        ))
        .expect("an out-of-band live price must not block the band write");

        let read = t
            .price_agg_client()
            .try_price(&controller::types::PriceKey::Token(usdc.clone()))
            .map(|inner| inner.map(|_| ()).map_err(|e| e.into()))
            .unwrap_or_else(|e| Err(e.expect("expected contract error")));
        assert_contract_error(read, errors::SANITY_BOUND_VIOLATED);
    }
}

#[test]
fn sanity_band_disjoint_from_old_band_rejected() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let gov = t.gov_iface_client();
    let admin = t.admin();
    let usdc = t.resolve_asset("USDC");

    let narrow_min = usd(1) * 97 / 100;
    let narrow_max = usd(1) * 103 / 100;
    gov.set_sanity_band(
        &admin,
        &controller::types::PriceKey::Token(usdc.clone()),
        &narrow_min,
        &narrow_max,
    );

    let result = gov.try_set_sanity_band(
        &admin,
        &controller::types::PriceKey::Token(usdc.clone()),
        &(usd(1) * 110 / 100),
        &(usd(1) * 115 / 100),
    );
    assert_contract_error(flatten(result), errors::INVALID_SANITY_BOUNDS);

    let wide_min = usd(1) * 94 / 100;
    let wide_max = usd(1) * 106 / 100;
    gov.set_sanity_band(
        &admin,
        &controller::types::PriceKey::Token(usdc.clone()),
        &wide_min,
        &wide_max,
    );
    let after = t.market_oracle_config(&usdc);
    assert_eq!(after.min_sanity_price_wad, wide_min);
    assert_eq!(after.max_sanity_price_wad, wide_max);
}

#[test]
fn sanity_band_input_and_role_gates() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let gov = t.gov_iface_client();
    let admin = t.admin();
    let stranger = Address::generate(&t.env);
    let usdc = t.resolve_asset("USDC");

    let result = gov.try_set_sanity_band(
        &admin,
        &controller::types::PriceKey::Token(usdc.clone()),
        &usd(2),
        &usd(1),
    );
    assert_contract_error(flatten(result), errors::INVALID_SANITY_BOUNDS);

    let result = gov.try_set_sanity_band(
        &stranger,
        &controller::types::PriceKey::Token(usdc.clone()),
        &(usd(1) * 95 / 100),
        &(usd(1) * 105 / 100),
    );
    assert_contract_error(flatten(result), errors::UNAUTHORIZED);
}

#[test]
fn guardian_creates_hub_and_spoke_immediately() {
    let t = LendingTest::new().build();
    let gov = t.gov_iface_client();
    let admin = t.admin();
    let stranger = Address::generate(&t.env);

    let hub_id = gov.create_hub(&admin);
    assert!(hub_id >= 1);
    let spoke_id = gov.add_spoke(&admin);
    assert!(spoke_id >= 1);

    assert!(!t.ctrl_client().get_spoke(&spoke_id).is_deprecated);

    assert_contract_error(flatten(gov.try_create_hub(&stranger)), errors::UNAUTHORIZED);
    assert_contract_error(flatten(gov.try_add_spoke(&stranger)), errors::UNAUTHORIZED);
}

#[test]
fn owner_revokes_role_immediately() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let gov = t.gov_iface_client();
    let admin = t.admin();
    let usdc = t.resolve_asset("USDC");
    let guardian_role = Symbol::new(&t.env, "GUARDIAN");
    let canceller_role = Symbol::new(&t.env, "CANCELLER");
    let holder = Address::generate(&t.env);

    t.gov_client().execute_immediate(
        &admin,
        &AdminOperation::GrantGovRole(RoleArgs {
            account: holder.clone(),
            role: guardian_role.clone(),
        }),
    );
    assert!(gov.has_role(&holder, &guardian_role));
    gov.revoke_role_immediate(&holder, &guardian_role);
    assert!(!gov.has_role(&holder, &guardian_role));

    let result =
        gov.try_set_spoke_asset_flags(&holder, &HARNESS_SPOKE, &hub_asset(usdc), &true, &false);
    assert_contract_error(flatten(result), errors::UNAUTHORIZED);

    let result = gov.try_revoke_role_immediate(&admin, &guardian_role);
    assert_contract_error(flatten(result), errors::NOT_AUTHORIZED);

    let result = gov.try_revoke_role_immediate(&holder, &guardian_role);
    assert_contract_error(flatten(result), errors::INVALID_ROLE);

    let result = gov.try_revoke_role_immediate(&holder, &Symbol::new(&t.env, "NOPE"));
    assert_contract_error(flatten(result), errors::INVALID_ROLE);

    t.gov_client().execute_immediate(
        &admin,
        &AdminOperation::GrantGovRole(RoleArgs {
            account: holder.clone(),
            role: canceller_role.clone(),
        }),
    );
    let result = gov.try_revoke_role_immediate(&holder, &canceller_role);
    assert_contract_error(flatten(result), errors::INVALID_ROLE);

    for role in ["PROPOSER", "EXECUTOR"] {
        let result = gov.try_revoke_role_immediate(&admin, &Symbol::new(&t.env, role));
        assert_contract_error(flatten(result), errors::INVALID_ROLE);
    }
}
