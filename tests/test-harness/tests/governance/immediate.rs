//! Role-gated immediate (timelock-bypassing) governance operations: guardian
//! spoke-listing flags, oracle sanity-band moves, instant hub/spoke creation,
//! and the owner's emergency role revocation.
//!
//! The harness admin holds every operational role (constructor grant), so it
//! doubles as GUARDIAN/ORACLE here; strangers prove the role gates.

use controller::types::{ControllerKey, MarketOracleConfig};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Symbol};
use test_harness::{
    assert_contract_error, errors, hub_asset, usd, usdc_preset, LendingTest, ALICE, HARNESS_SPOKE,
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

fn stored_oracle(t: &LendingTest, asset: &Address) -> MarketOracleConfig {
    t.env.as_contract(&t.controller, || {
        t.env
            .storage()
            .persistent()
            .get(&ControllerKey::AssetOracle(asset.clone()))
            .expect("oracle configured")
    })
}

// GUARDIAN flips a listing's flags instantly; the flags bind and every other
// listing field survives untouched.
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

    // Clearing is equally immediate.
    t.gov_iface_client().set_spoke_asset_flags(
        &admin,
        &HARNESS_SPOKE,
        &hub_asset(usdc.clone()),
        &false,
        &false,
    );
    assert!(
        t.try_supply(ALICE, "USDC", 10.0).is_ok(),
        "unpause must re-open supply"
    );
}

// A caller without GUARDIAN is rejected with the OZ AccessControl error.
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

// ORACLE moves the sanity band instantly when the new band contains the live
// price; the stored config carries only the new bounds.
#[test]
fn oracle_role_moves_sanity_band_containing_price() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let gov = t.gov_iface_client();
    let admin = t.admin();
    let usdc = t.resolve_asset("USDC");

    let before = stored_oracle(&t, &usdc);
    // Live price is $1; the new band contains it.
    gov.set_oracle_sanity_bounds(&admin, &usdc, &(usd(1) / 2), &(usd(2)));

    let after = stored_oracle(&t, &usdc);
    assert_eq!(after.min_sanity_price_wad, usd(1) / 2);
    assert_eq!(after.max_sanity_price_wad, usd(2));
    assert_eq!(
        after.max_price_stale_seconds,
        before.max_price_stale_seconds
    );
    assert_eq!(after.tolerance, before.tolerance);
}

// A band that does not contain the live price is rejected in both directions.
#[test]
fn sanity_band_not_containing_price_rejected() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let gov = t.gov_iface_client();
    let admin = t.admin();
    let usdc = t.resolve_asset("USDC");

    // Entirely above the $1 live price.
    let result = gov.try_set_oracle_sanity_bounds(&admin, &usdc, &usd(2), &usd(3));
    assert_contract_error(flatten(result), errors::SANITY_BOUND_VIOLATED);

    // Entirely below the $1 live price.
    let result = gov.try_set_oracle_sanity_bounds(&admin, &usdc, &(usd(1) / 100), &(usd(1) / 2));
    assert_contract_error(flatten(result), errors::SANITY_BOUND_VIOLATED);
}

// Malformed bounds and missing roles are rejected before any oracle read.
#[test]
fn sanity_band_input_and_role_gates() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let gov = t.gov_iface_client();
    let admin = t.admin();
    let stranger = Address::generate(&t.env);
    let usdc = t.resolve_asset("USDC");

    let result = gov.try_set_oracle_sanity_bounds(&admin, &usdc, &usd(2), &usd(1));
    assert_contract_error(flatten(result), errors::INVALID_SANITY_BOUNDS);

    let result = gov.try_set_oracle_sanity_bounds(&stranger, &usdc, &(usd(1) / 2), &usd(2));
    assert_contract_error(flatten(result), errors::UNAUTHORIZED);
}

// GUARDIAN creates hubs and spokes instantly; both registries are inert until
// assets are listed through the timelocked path.
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
    // Fresh spoke exists and is active.
    assert!(!t.ctrl_client().get_spoke(&spoke_id).is_deprecated);

    assert_contract_error(flatten(gov.try_create_hub(&stranger)), errors::UNAUTHORIZED);
    assert_contract_error(flatten(gov.try_add_spoke(&stranger)), errors::UNAUTHORIZED);
}

// The owner strips an immediate role instantly; the stripped key loses its
// powers in the same ledger, and no-op/unknown revokes are rejected.
#[test]
fn owner_revokes_role_immediately() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let gov = t.gov_iface_client();
    let admin = t.admin();
    let usdc = t.resolve_asset("USDC");
    let guardian_role = Symbol::new(&t.env, "GUARDIAN");

    assert!(gov.has_role(&admin, &guardian_role));
    gov.revoke_role_immediate(&admin, &guardian_role);
    assert!(!gov.has_role(&admin, &guardian_role));

    let result =
        gov.try_set_spoke_asset_flags(&admin, &HARNESS_SPOKE, &hub_asset(usdc), &true, &false);
    assert_contract_error(flatten(result), errors::UNAUTHORIZED);

    // Revoking a role the account no longer holds is a no-op reject.
    let result = gov.try_revoke_role_immediate(&admin, &guardian_role);
    assert_contract_error(flatten(result), errors::INVALID_ROLE);

    // Unknown roles are rejected outright.
    let result = gov.try_revoke_role_immediate(&admin, &Symbol::new(&t.env, "NOPE"));
    assert_contract_error(flatten(result), errors::INVALID_ROLE);
}
