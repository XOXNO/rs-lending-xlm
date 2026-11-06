use common::constants::{RAY, TTL_BUMP_SHARED, TTL_THRESHOLD_SHARED};
use common::types::{HubAssetKey, MarketParamsRaw};
use curator::{CapGrant, Curator, CuratorClient, CuratorError, ROLE_MARKET, ROLE_SPOKE};
use governance::op::{AdminOperation, RelaxSpokeAssetFlagsArgs, RoleArgs, SpokeAssetArgs};
use soroban_sdk::testutils::storage::Persistent as _;
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{contracttype, Address, BytesN, Env, Symbol, TryFromVal};

fn setup(env: &Env) -> (Address, CuratorClient<'_>) {
    env.mock_all_auths();
    let owner = Address::generate(env);
    let gov_id = env.register(governance::Governance, (owner.clone(), 10u32));
    let gov = governance::GovernanceClient::new(env, &gov_id);
    let controller_id = env.register(controller::Controller, (gov_id.clone(),));
    gov.set_controller(&controller_id);
    let curator_id = env.register(Curator, (gov_id,));
    for role in ["CURATOR", "PROPOSER", "CANCELLER"] {
        gov.execute_immediate(
            &owner,
            &AdminOperation::GrantGovRole(RoleArgs {
                account: curator_id.clone(),
                role: Symbol::new(env, role),
            }),
        );
    }
    (owner, CuratorClient::new(env, &curator_id))
}

#[test]
#[should_panic(expected = "Error(Contract, #10)")]
fn upgrade_rejects_a_zero_hash() {
    let env = Env::default();
    let (_owner, curator) = setup(&env);
    curator.upgrade(&BytesN::from_array(&env, &[0; 32]));
}

#[test]
fn spoke_role_binds_the_created_ids() {
    let env = Env::default();
    let (owner, curator) = setup(&env);
    let admin = Address::generate(&env);
    let operator = Address::generate(&env);
    let registrar = register(&env, &owner, &curator, &admin);
    curator.set_roles(&admin, &registrar, &operator, &ROLE_SPOKE);

    assert_eq!(curator.create_hub(&operator, &registrar), 1);
    assert_eq!(curator.add_spoke(&admin, &registrar), 1);
    assert_eq!(curator.hub_registrar(&1), Some(registrar));
    assert_eq!(curator.spoke_registrar(&1), Some(registrar));
}

#[test]
#[should_panic(expected = "Error(Contract, #802)")]
fn market_role_cannot_create_a_hub() {
    let env = Env::default();
    let (owner, curator) = setup(&env);
    let admin = Address::generate(&env);
    let operator = Address::generate(&env);
    let registrar = register(&env, &owner, &curator, &admin);
    curator.set_roles(&admin, &registrar, &operator, &ROLE_MARKET);
    curator.create_hub(&operator, &registrar);
}

#[test]
#[should_panic(expected = "Error(Contract, #812)")]
fn listing_another_registrars_hub_requires_a_grant() {
    let env = Env::default();
    let (owner, curator) = setup(&env);
    let admin = Address::generate(&env);
    let other = Address::generate(&env);
    let first = register(&env, &owner, &curator, &admin);
    let second = register(&env, &owner, &curator, &other);
    let hub = curator.create_hub(&admin, &first);
    let spoke = curator.add_spoke(&other, &second);
    curator.propose(
        &other,
        &second,
        &AdminOperation::AddAssetToSpoke(SpokeAssetArgs {
            hub_id: hub,
            asset: Address::generate(&env),
            spoke_id: spoke,
            can_collateral: false,
            can_borrow: false,
            paused: false,
            frozen: false,
            no_seize: false,
            ltv: 0,
            threshold: 0,
            bonus: 0,
            liquidation_fees: 0,
            supply_cap: 0,
            borrow_cap: 0,
        }),
        &BytesN::from_array(&env, &[1; 32]),
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #813)")]
fn cross_hub_listing_rejects_ltv_above_the_grant() {
    let env = Env::default();
    let (owner, curator) = setup(&env);
    let admin = Address::generate(&env);
    let other = Address::generate(&env);
    let first = register(&env, &owner, &curator, &admin);
    let second = register(&env, &owner, &curator, &other);
    let hub = curator.create_hub(&admin, &first);
    let spoke = curator.add_spoke(&other, &second);
    let asset = Address::generate(&env);
    curator.set_cap_grant(
        &admin,
        &first,
        &second,
        &hub,
        &spoke,
        &asset,
        &CapGrant {
            supply_cap: 1_000,
            borrow_cap: 1_000,
            can_collateral: true,
            can_borrow: true,
            ltv: 5_000,
            threshold: 8_000,
            bonus: 500,
            liquidation_fees: 1_000,
        },
    );
    curator.propose(
        &other,
        &second,
        &AdminOperation::AddAssetToSpoke(SpokeAssetArgs {
            hub_id: hub,
            asset,
            spoke_id: spoke,
            can_collateral: true,
            can_borrow: false,
            paused: false,
            frozen: false,
            no_seize: false,
            ltv: 5_001,
            threshold: 7_000,
            bonus: 100,
            liquidation_fees: 0,
            supply_cap: 100,
            borrow_cap: 0,
        }),
        &BytesN::from_array(&env, &[0; 32]),
    );
}

fn grant(
    curator: &CuratorClient<'_>,
    admin: &Address,
    first: u32,
    second: u32,
    hub: u32,
    spoke: u32,
    asset: &Address,
) {
    curator.set_cap_grant(
        admin,
        &first,
        &second,
        &hub,
        &spoke,
        asset,
        &CapGrant {
            supply_cap: 1_000,
            borrow_cap: 1_000,
            can_collateral: true,
            can_borrow: true,
            ltv: 5_000,
            threshold: 8_000,
            bonus: 500,
            liquidation_fees: 1_000,
        },
    );
}

fn borrow_listing(hub: u32, spoke: u32, asset: Address) -> AdminOperation {
    AdminOperation::AddAssetToSpoke(SpokeAssetArgs {
        hub_id: hub,
        asset,
        spoke_id: spoke,
        can_collateral: false,
        can_borrow: true,
        paused: false,
        frozen: false,
        no_seize: false,
        ltv: 0,
        threshold: 0,
        bonus: 0,
        liquidation_fees: 0,
        supply_cap: 1_000,
        borrow_cap: 1_000,
    })
}

#[test]
#[should_panic(expected = "Error(Contract, #812)")]
fn cap_grant_does_not_cover_a_second_grantee_spoke() {
    let env = Env::default();
    let (owner, curator) = setup(&env);
    let admin = Address::generate(&env);
    let other = Address::generate(&env);
    let first = register(&env, &owner, &curator, &admin);
    let second = register(&env, &owner, &curator, &other);
    let hub = curator.create_hub(&admin, &first);
    let granted = curator.add_spoke(&other, &second);
    let extra = curator.add_spoke(&other, &second);
    let asset = Address::generate(&env);
    grant(&curator, &admin, first, second, hub, granted, &asset);

    assert!(curator.cap_grant(&second, &hub, &granted, &asset).is_some());
    assert!(curator.cap_grant(&second, &hub, &extra, &asset).is_none());
    curator.propose(
        &other,
        &second,
        &borrow_listing(hub, extra, asset),
        &BytesN::from_array(&env, &[2; 32]),
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #809)")]
fn cap_grant_rejects_a_spoke_the_grantee_does_not_own() {
    let env = Env::default();
    let (owner, curator) = setup(&env);
    let admin = Address::generate(&env);
    let other = Address::generate(&env);
    let first = register(&env, &owner, &curator, &admin);
    let second = register(&env, &owner, &curator, &other);
    let hub = curator.create_hub(&admin, &first);
    let foreign = curator.add_spoke(&admin, &first);
    grant(
        &curator,
        &admin,
        first,
        second,
        hub,
        foreign,
        &Address::generate(&env),
    );
}

fn register(env: &Env, owner: &Address, curator: &CuratorClient<'_>, admin: &Address) -> u32 {
    let id = governance::GovernanceClient::new(env, &curator.governance()).execute_immediate(
        owner,
        &AdminOperation::RegisterCuratorRegistrar(curator.address.clone(), admin.clone()),
    );
    u32::try_from_val(env, &id).unwrap()
}

fn suspend(env: &Env, owner: &Address, curator: &CuratorClient<'_>, registrar: u32, on: bool) {
    governance::GovernanceClient::new(env, &curator.governance()).execute_immediate(
        owner,
        &AdminOperation::SuspendCuratorRegistrar(curator.address.clone(), registrar, on),
    );
}

#[test]
fn suspended_registrar_cannot_act_until_governance_resumes_it() {
    let env = Env::default();
    let (owner, curator) = setup(&env);
    let admin = Address::generate(&env);
    let registrar = register(&env, &owner, &curator, &admin);

    suspend(&env, &owner, &curator, registrar, true);
    assert!(curator.registrar_suspended(&registrar));
    assert_eq!(curator.registrar_admin(&registrar), Some(admin.clone()));
    assert_eq!(
        curator.try_add_spoke(&admin, &registrar),
        Err(Ok(CuratorError::RegistrarSuspended.into()))
    );
    assert_eq!(
        curator.try_set_roles(&admin, &registrar, &Address::generate(&env), &ROLE_MARKET),
        Err(Ok(CuratorError::RegistrarSuspended.into()))
    );

    suspend(&env, &owner, &curator, registrar, false);
    assert!(!curator.registrar_suspended(&registrar));
    assert_eq!(curator.add_spoke(&admin, &registrar), 1);
}

#[test]
#[should_panic(expected = "Error(Contract, #815)")]
fn suspended_hub_owner_grant_stops_admitting_listings() {
    let env = Env::default();
    let (owner, curator) = setup(&env);
    let admin = Address::generate(&env);
    let other = Address::generate(&env);
    let first = register(&env, &owner, &curator, &admin);
    let second = register(&env, &owner, &curator, &other);
    let hub = curator.create_hub(&admin, &first);
    let spoke = curator.add_spoke(&other, &second);
    let asset = Address::generate(&env);
    grant(&curator, &admin, first, second, hub, spoke, &asset);

    suspend(&env, &owner, &curator, first, true);
    curator.propose(
        &other,
        &second,
        &borrow_listing(hub, spoke, asset),
        &BytesN::from_array(&env, &[3; 32]),
    );
}

fn gov<'a>(env: &'a Env, curator: &CuratorClient<'a>) -> governance::GovernanceClient<'a> {
    governance::GovernanceClient::new(env, &curator.governance())
}

fn relax(hub: u32, spoke: u32, asset: Address) -> AdminOperation {
    AdminOperation::RelaxSpokeAssetFlags(RelaxSpokeAssetFlagsArgs {
        spoke_id: spoke,
        hub_asset: HubAssetKey { hub_id: hub, asset },
        expected_epoch: 0,
        paused: false,
        frozen: false,
        no_seize: false,
    })
}

struct Pair<'a> {
    env: &'a Env,
    owner: Address,
    curator: CuratorClient<'a>,
    admin: Address,
    other: Address,
    first: u32,
    second: u32,
    hub: u32,
    spoke: u32,
    asset: Address,
}

fn granted_pair(env: &Env) -> Pair<'_> {
    let (owner, curator) = setup(env);
    let admin = Address::generate(env);
    let other = Address::generate(env);
    let first = register(env, &owner, &curator, &admin);
    let second = register(env, &owner, &curator, &other);
    let hub = curator.create_hub(&admin, &first);
    let spoke = curator.add_spoke(&other, &second);
    let asset = Address::generate(env);
    grant(&curator, &admin, first, second, hub, spoke, &asset);
    Pair {
        env,
        owner,
        curator,
        admin,
        other,
        first,
        second,
        hub,
        spoke,
        asset,
    }
}

#[test]
fn anyone_cancels_a_suspended_registrars_queued_proposal() {
    let env = Env::default();
    let p = granted_pair(&env);
    let op_id = p.curator.propose(
        &p.other,
        &p.second,
        &relax(p.hub, p.spoke, p.asset.clone()),
        &BytesN::from_array(p.env, &[4; 32]),
    );
    assert_eq!(p.curator.pending_proposal(&op_id), Some((p.second, p.hub)));
    assert_eq!(
        p.curator.try_cancel_suspended(&op_id),
        Err(Ok(CuratorError::NotCancellable.into()))
    );

    suspend(p.env, &p.owner, &p.curator, p.second, true);
    p.curator.cancel_suspended(&op_id);
    assert_eq!(p.curator.pending_proposal(&op_id), None);
    assert_eq!(
        gov(p.env, &p.curator).get_operation_state(&op_id),
        governance::OperationState::Unset
    );
}

#[test]
fn hub_owner_cancels_a_grantee_proposal_against_its_hub() {
    let env = Env::default();
    let p = granted_pair(&env);
    let op_id = p.curator.propose(
        &p.other,
        &p.second,
        &relax(p.hub, p.spoke, p.asset.clone()),
        &BytesN::from_array(p.env, &[5; 32]),
    );
    assert_eq!(
        p.curator
            .try_cancel_grantee_proposal(&p.other, &p.second, &op_id),
        Err(Ok(CuratorError::NotCancellable.into()))
    );
    p.curator
        .cancel_grantee_proposal(&p.admin, &p.first, &op_id);
    assert_eq!(
        gov(p.env, &p.curator).get_operation_state(&op_id),
        governance::OperationState::Unset
    );
}

#[test]
fn revoked_grant_blocks_a_grantee_relaxation_of_the_hub_asset() {
    let env = Env::default();
    let p = granted_pair(&env);
    p.curator
        .revoke_cap_grant(&p.admin, &p.first, &p.second, &p.hub, &p.spoke, &p.asset);
    assert!(p
        .curator
        .cap_grant(&p.second, &p.hub, &p.spoke, &p.asset)
        .is_none());
    assert_eq!(
        p.curator.try_propose(
            &p.other,
            &p.second,
            &relax(p.hub, p.spoke, p.asset.clone()),
            &BytesN::from_array(p.env, &[6; 32]),
        ),
        Err(Ok(CuratorError::GrantMissing.into()))
    );
}

#[test]
fn a_third_registrar_cannot_close_the_listing() {
    let env = Env::default();
    let p = granted_pair(&env);
    let stranger = Address::generate(p.env);
    let third = register(p.env, &p.owner, &p.curator, &stranger);
    assert_eq!(
        p.curator.try_close_listing(
            &stranger,
            &third,
            &p.spoke,
            &HubAssetKey {
                hub_id: p.hub,
                asset: p.asset.clone()
            },
        ),
        Err(Ok(CuratorError::NotListingParty.into()))
    );
}

#[test]
fn reassigned_hub_drops_grants_from_the_previous_owner() {
    let env = Env::default();
    let p = granted_pair(&env);
    let stranger = Address::generate(p.env);
    let third = register(p.env, &p.owner, &p.curator, &stranger);
    gov(p.env, &p.curator).execute_immediate(
        &p.owner,
        &AdminOperation::ReassignCuratorHub(p.curator.address.clone(), p.hub, Some(third)),
    );
    assert_eq!(p.curator.hub_registrar(&p.hub), Some(third));
    assert_eq!(
        p.curator.try_propose(
            &p.other,
            &p.second,
            &borrow_listing(p.hub, p.spoke, p.asset.clone()),
            &BytesN::from_array(p.env, &[7; 32]),
        ),
        Err(Ok(CuratorError::GrantMissing.into()))
    );

    gov(p.env, &p.curator).execute_immediate(
        &p.owner,
        &AdminOperation::ReassignCuratorSpoke(p.curator.address.clone(), p.spoke, None),
    );
    assert_eq!(p.curator.spoke_registrar(&p.spoke), None);
}

#[test]
fn governance_replaces_the_admin_of_a_suspended_registrar() {
    let env = Env::default();
    let p = granted_pair(&env);
    let operator = Address::generate(p.env);
    p.curator
        .set_roles(&p.other, &p.second, &operator, &ROLE_MARKET);
    suspend(p.env, &p.owner, &p.curator, p.second, true);

    let fresh = Address::generate(p.env);
    gov(p.env, &p.curator).execute_immediate(
        &p.owner,
        &AdminOperation::SetCuratorRegistrarAdmin(
            p.curator.address.clone(),
            p.second,
            fresh.clone(),
        ),
    );
    suspend(p.env, &p.owner, &p.curator, p.second, false);
    assert_eq!(p.curator.registrar_admin(&p.second), Some(fresh.clone()));
    p.curator.set_roles(&fresh, &p.second, &operator, &0);
    assert_eq!(p.curator.roles(&p.second, &operator), 0);
    assert_eq!(
        p.curator
            .try_set_roles(&p.other, &p.second, &operator, &ROLE_MARKET),
        Err(Ok(CuratorError::RoleMissing.into()))
    );
}

#[test]
fn guardian_suspends_a_registrar_immediately() {
    let env = Env::default();
    let p = granted_pair(&env);
    gov(p.env, &p.curator).suspend_curator_registrar(&p.owner, &p.curator.address, &p.second);
    assert!(p.curator.registrar_suspended(&p.second));
    assert_eq!(
        gov(p.env, &p.curator).try_suspend_curator_registrar(
            &p.owner,
            &Address::generate(p.env),
            &p.second
        ),
        Err(Ok(soroban_sdk::Error::from_contract_error(44)))
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #813)")]
fn cross_hub_listing_rejects_liquidation_fees_above_the_grant() {
    let env = Env::default();
    let p = granted_pair(&env);
    p.curator.propose(
        &p.other,
        &p.second,
        &AdminOperation::AddAssetToSpoke(SpokeAssetArgs {
            hub_id: p.hub,
            asset: p.asset.clone(),
            spoke_id: p.spoke,
            can_collateral: true,
            can_borrow: false,
            paused: false,
            frozen: false,
            no_seize: false,
            ltv: 5_000,
            threshold: 7_000,
            bonus: 100,
            liquidation_fees: 1_001,
            supply_cap: 100,
            borrow_cap: 0,
        }),
        &BytesN::from_array(p.env, &[8; 32]),
    );
}

#[test]
fn a_grant_may_cap_ltv_and_threshold_at_the_same_value() {
    let env = Env::default();
    let p = granted_pair(&env);
    let flat = CapGrant {
        supply_cap: 1,
        borrow_cap: 0,
        can_collateral: true,
        can_borrow: false,
        ltv: 7_000,
        threshold: 7_000,
        bonus: 0,
        liquidation_fees: 0,
    };
    p.curator.set_cap_grant(
        &p.admin, &p.first, &p.second, &p.hub, &p.spoke, &p.asset, &flat,
    );
    let stored = p
        .curator
        .cap_grant(&p.second, &p.hub, &p.spoke, &p.asset)
        .unwrap();
    assert_eq!((stored.ltv, stored.threshold), (flat.ltv, flat.threshold));
}

#[contracttype]
enum RegistrarKey {
    RegistrarAdmin(u32),
}

#[test]
fn a_read_inside_the_threshold_window_renews_the_registrar_row() {
    let env = Env::default();
    let (owner, curator) = setup(&env);
    let admin = Address::generate(&env);
    let registrar = register(&env, &owner, &curator, &admin);
    let ttl = || {
        env.as_contract(&curator.address, || {
            env.storage()
                .persistent()
                .get_ttl(&RegistrarKey::RegistrarAdmin(registrar))
        })
    };
    assert_eq!(ttl(), TTL_BUMP_SHARED);

    env.ledger()
        .with_mut(|l| l.sequence_number += TTL_BUMP_SHARED - TTL_THRESHOLD_SHARED + 1);
    assert_eq!(ttl(), TTL_THRESHOLD_SHARED - 1);
    assert_eq!(curator.registrar_admin(&registrar), Some(admin));
    assert_eq!(ttl(), TTL_BUMP_SHARED);
}

fn market_params(asset: &Address) -> MarketParamsRaw {
    MarketParamsRaw {
        max_borrow_rate: RAY,
        base_borrow_rate: 0,
        slope1: RAY / 100,
        slope2: RAY / 10,
        slope3: RAY / 2,
        mid_utilization: RAY / 2,
        optimal_utilization: 8 * RAY / 10,
        max_utilization: 95 * RAY / 100,
        reserve_factor: 1_000,
        is_flashloanable: false,
        flashloan_fee: 0,
        asset_id: asset.clone(),
        asset_decimals: 7,
    }
}

#[test]
#[should_panic(expected = "Error(Contract, #801)")]
fn set_admin_rejects_an_unknown_registrar() {
    let env = Env::default();
    let (_owner, curator) = setup(&env);
    let admin = Address::generate(&env);
    curator.set_admin(&admin, &7, &Address::generate(&env));
}

#[test]
#[should_panic(expected = "Error(Contract, #803)")]
fn set_roles_rejects_a_bit_outside_the_mask() {
    let env = Env::default();
    let (owner, curator) = setup(&env);
    let admin = Address::generate(&env);
    let registrar = register(&env, &owner, &curator, &admin);
    curator.set_roles(&admin, &registrar, &Address::generate(&env), &8);
}

#[test]
#[should_panic(expected = "Error(Contract, #804)")]
fn create_hub_rejects_an_id_governance_already_bound() {
    let env = Env::default();
    let (owner, curator) = setup(&env);
    let admin = Address::generate(&env);
    let registrar = register(&env, &owner, &curator, &admin);
    gov(&env, &curator).execute_immediate(
        &owner,
        &AdminOperation::ReassignCuratorHub(curator.address.clone(), 1, Some(registrar)),
    );
    curator.create_hub(&admin, &registrar);
}

#[test]
#[should_panic(expected = "Error(Contract, #805)")]
fn create_market_rejects_an_unowned_hub() {
    let env = Env::default();
    let (owner, curator) = setup(&env);
    let admin = Address::generate(&env);
    let registrar = register(&env, &owner, &curator, &admin);
    let asset = Address::generate(&env);
    curator.create_market(&admin, &registrar, &7, &market_params(&asset));
}

#[test]
#[should_panic(expected = "Error(Contract, #806)")]
fn add_spoke_rejects_an_id_governance_already_bound() {
    let env = Env::default();
    let (owner, curator) = setup(&env);
    let admin = Address::generate(&env);
    let registrar = register(&env, &owner, &curator, &admin);
    gov(&env, &curator).execute_immediate(
        &owner,
        &AdminOperation::ReassignCuratorSpoke(curator.address.clone(), 1, Some(registrar)),
    );
    curator.add_spoke(&admin, &registrar);
}

#[test]
#[should_panic(expected = "Error(Contract, #807)")]
fn lower_caps_rejects_an_unowned_spoke() {
    let env = Env::default();
    let (owner, curator) = setup(&env);
    let admin = Address::generate(&env);
    let registrar = register(&env, &owner, &curator, &admin);
    curator.lower_caps(
        &admin,
        &registrar,
        &9,
        &HubAssetKey {
            hub_id: 1,
            asset: Address::generate(&env),
        },
        &0,
        &0,
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #808)")]
fn create_market_rejects_another_registrars_hub() {
    let env = Env::default();
    let p = granted_pair(&env);
    p.curator
        .create_market(&p.other, &p.second, &p.hub, &market_params(&p.asset));
}

#[test]
#[should_panic(expected = "Error(Contract, #814)")]
fn propose_rejects_an_operation_outside_the_curator_set() {
    let env = Env::default();
    let (owner, curator) = setup(&env);
    let admin = Address::generate(&env);
    let registrar = register(&env, &owner, &curator, &admin);
    curator.propose(
        &admin,
        &registrar,
        &AdminOperation::Unpause,
        &BytesN::from_array(&env, &[9; 32]),
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #812)")]
fn an_unbound_hub_stops_admitting_a_grantee_listing() {
    let env = Env::default();
    let p = granted_pair(&env);
    gov(p.env, &p.curator).execute_immediate(
        &p.owner,
        &AdminOperation::ReassignCuratorHub(p.curator.address.clone(), p.hub, None),
    );
    assert_eq!(p.curator.hub_registrar(&p.hub), None);
    p.curator.propose(
        &p.other,
        &p.second,
        &borrow_listing(p.hub, p.spoke, p.asset.clone()),
        &BytesN::from_array(p.env, &[10; 32]),
    );
}

#[test]
fn the_admin_hands_the_registrar_to_a_new_admin() {
    let env = Env::default();
    let (owner, curator) = setup(&env);
    let admin = Address::generate(&env);
    let fresh = Address::generate(&env);
    let registrar = register(&env, &owner, &curator, &admin);

    curator.set_admin(&admin, &registrar, &fresh);
    assert_eq!(curator.registrar_admin(&registrar), Some(fresh.clone()));
    assert_eq!(curator.add_spoke(&fresh, &registrar), 1);
    assert_eq!(
        curator.try_add_spoke(&admin, &registrar),
        Err(Ok(CuratorError::RoleMissing.into()))
    );
}

#[test]
fn cancel_suspended_rejects_an_unknown_proposal() {
    let env = Env::default();
    let (_owner, curator) = setup(&env);
    assert_eq!(
        curator.try_cancel_suspended(&BytesN::from_array(&env, &[11; 32])),
        Err(Ok(CuratorError::NotCancellable.into()))
    );
}

#[test]
fn only_a_guardian_suspends_a_registrar_immediately() {
    let env = Env::default();
    let p = granted_pair(&env);
    assert_eq!(
        gov(p.env, &p.curator).try_suspend_curator_registrar(
            &Address::generate(p.env),
            &p.curator.address,
            &p.second
        ),
        Err(Ok(soroban_sdk::Error::from_contract_error(2000)))
    );
    assert!(!p.curator.registrar_suspended(&p.second));
}
