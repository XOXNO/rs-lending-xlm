//! Role-gated immediate (timelock-bypassing) governance operations: guardian
//! spoke-listing flags, oracle sanity-band moves, instant hub/spoke creation,
//! and the owner's emergency role revocation.
//!
//! The harness admin holds every operational role (constructor grant), so it
//! doubles as GUARDIAN/ORACLE here; strangers prove the role gates.

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

    // Re-asserting the same tightened state stays allowed (idempotent brake),
    // and tightening the remaining flag on top works.
    t.gov_iface_client().set_spoke_asset_flags(
        &admin,
        &HARNESS_SPOKE,
        &hub_asset(usdc.clone()),
        &true,
        &true,
    );

    // Clearing a set flag is risk-loosening and must ride the timelock.
    let relax = t.gov_iface_client().try_set_spoke_asset_flags(
        &admin,
        &HARNESS_SPOKE,
        &hub_asset(usdc.clone()),
        &false,
        &false,
    );
    assert_contract_error(flatten(relax), errors::SPOKE_ASSET_FLAG_RELAXATION);
    // Partial relaxation (keep paused, clear frozen) is rejected too.
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

    // The timelocked `EditAssetInSpoke` path clears the flags and reopens
    // supply (the harness forwarder stands in for a matured proposal).
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
            oracle_override: cfg.oracle_override,
        }),
    );
    assert!(
        t.try_supply(ALICE, "USDC", 10.0).is_ok(),
        "timelocked edit must re-open supply"
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

    let before = t.market_oracle_config(&usdc);
    // Live price is $1; the new band contains it.
    gov.set_oracle_sanity_bounds(&admin, &usdc, &(usd(1) / 2), &(usd(2)));

    let after = t.market_oracle_config(&usdc);
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

// The new band must overlap the old one: bands are walked, never teleported
// to a disjoint range on one call.
#[test]
fn sanity_band_disjoint_from_old_band_rejected() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let gov = t.gov_iface_client();
    let admin = t.admin();
    let usdc = t.resolve_asset("USDC");

    // Narrow the band around the live $1 price first.
    gov.set_oracle_sanity_bounds(&admin, &usdc, &(usd(1) / 2), &usd(2));

    // A new band disjoint from [0.5, 2.0] is rejected even before pricing
    // (containment would also fail here; the overlap rule fires first).
    let result = gov.try_set_oracle_sanity_bounds(&admin, &usdc, &usd(3), &usd(4));
    assert_contract_error(flatten(result), errors::INVALID_SANITY_BOUNDS);

    // An overlapping widening that still contains the live price passes.
    gov.set_oracle_sanity_bounds(&admin, &usdc, &(usd(1) / 4), &usd(3));
    let after = t.market_oracle_config(&usdc);
    assert_eq!(after.min_sanity_price_wad, usd(1) / 4);
    assert_eq!(after.max_sanity_price_wad, usd(3));
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

// The owner strips an immediate role from a non-owner key instantly; the
// stripped key loses its powers in the same ledger. The owner's own roles are
// never revocable, and no-op/unknown revokes are rejected.
#[test]
fn owner_revokes_role_immediately() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let gov = t.gov_iface_client();
    let admin = t.admin();
    let usdc = t.resolve_asset("USDC");
    let guardian_role = Symbol::new(&t.env, "GUARDIAN");
    let canceller_role = Symbol::new(&t.env, "CANCELLER");
    let holder = Address::generate(&t.env);

    // Grant GUARDIAN to a fresh non-owner key, then the owner strips it instantly.
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

    // The owner's own roles are never revocable.
    let result = gov.try_revoke_role_immediate(&admin, &guardian_role);
    assert_contract_error(flatten(result), errors::NOT_AUTHORIZED);

    // Revoking a role the account no longer holds is a no-op reject.
    let result = gov.try_revoke_role_immediate(&holder, &guardian_role);
    assert_contract_error(flatten(result), errors::INVALID_ROLE);

    // Unknown roles are rejected outright.
    let result = gov.try_revoke_role_immediate(&holder, &Symbol::new(&t.env, "NOPE"));
    assert_contract_error(flatten(result), errors::INVALID_ROLE);

    // CANCELLER is immediately revocable so the owner can break a colluding
    // pair that would otherwise cross-veto each other's timelocked removal.
    t.gov_client().execute_immediate(
        &admin,
        &AdminOperation::GrantGovRole(RoleArgs {
            account: holder.clone(),
            role: canceller_role.clone(),
        }),
    );
    assert!(gov.has_role(&holder, &canceller_role));
    gov.revoke_role_immediate(&holder, &canceller_role);
    assert!(!gov.has_role(&holder, &canceller_role));

    // PROPOSER/EXECUTOR stay timelock-only for revocation (rejected before any
    // holder/owner check by the immediate-role allow-list).
    for role in ["PROPOSER", "EXECUTOR"] {
        let result = gov.try_revoke_role_immediate(&admin, &Symbol::new(&t.env, role));
        assert_contract_error(flatten(result), errors::INVALID_ROLE);
    }
}
