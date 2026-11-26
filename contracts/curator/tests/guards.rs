//! Boundary and caller checks on the curator registry: who must sign, which
//! roles may cancel, how a hub owner's cap edit is judged against the live
//! listing, and where a grantee's listing meets its grant exactly.

use common::types::{ControllerKey, HubAssetKey, SpokeAssetConfig};
use curator::{
    CapGrant, Curator, CuratorClient, CuratorError, ROLE_FLAGS, ROLE_MARKET, ROLE_SPOKE,
};
use governance::op::{AdminOperation, RelaxSpokeAssetFlagsArgs, RoleArgs, SpokeAssetArgs};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::xdr::{ScErrorCode, ScErrorType};
use soroban_sdk::{Address, BytesN, Env, Error, Symbol, TryFromVal};

struct Pair<'a> {
    env: &'a Env,
    curator: CuratorClient<'a>,
    gov: governance::GovernanceClient<'a>,
    admin: Address,
    other: Address,
    first: u32,
    second: u32,
    hub: u32,
    spoke: u32,
    asset: Address,
}

fn auth_error() -> Result<Error, soroban_sdk::InvokeError> {
    Ok(Error::from_type_and_code(
        ScErrorType::Context,
        ScErrorCode::InvalidAction,
    ))
}

fn setup(env: &Env) -> (Address, governance::GovernanceClient<'_>, CuratorClient<'_>) {
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
    (owner, gov, CuratorClient::new(env, &curator_id))
}

fn register(
    gov: &governance::GovernanceClient<'_>,
    owner: &Address,
    curator: &CuratorClient<'_>,
    admin: &Address,
) -> u32 {
    let id = gov.execute_immediate(
        owner,
        &AdminOperation::RegisterCuratorRegistrar(curator.address.clone(), admin.clone()),
    );
    u32::try_from_val(&gov.env, &id).unwrap()
}

/// `first` owns the hub, `second` owns the spoke, and `first` granted
/// `second` the `grant` for `asset` in that spoke.
fn pair_with_grant<'a>(env: &'a Env, grant: &CapGrant) -> Pair<'a> {
    let (owner, gov, curator) = setup(env);
    let admin = Address::generate(env);
    let other = Address::generate(env);
    let first = register(&gov, &owner, &curator, &admin);
    let second = register(&gov, &owner, &curator, &other);
    let hub = curator.create_hub(&admin, &first);
    let spoke = curator.add_spoke(&other, &second);
    let asset = Address::generate(env);
    curator.set_cap_grant(&admin, &first, &second, &hub, &spoke, &asset, grant);
    Pair {
        env,
        curator,
        gov,
        admin,
        other,
        first,
        second,
        hub,
        spoke,
        asset,
    }
}

fn full_grant() -> CapGrant {
    CapGrant {
        supply_cap: 1_000,
        borrow_cap: 900,
        can_collateral: true,
        can_borrow: true,
        ltv: 5_000,
        threshold: 8_000,
        bonus: 500,
        liquidation_fees: 1_000,
    }
}

fn pair(env: &Env) -> Pair<'_> {
    pair_with_grant(env, &full_grant())
}

impl Pair<'_> {
    fn key(&self) -> HubAssetKey {
        HubAssetKey {
            hub_id: self.hub,
            asset: self.asset.clone(),
        }
    }

    /// Writes a live listing of the hub asset in the spoke straight into the
    /// controller, standing in for an executed `AddAssetToSpoke`.
    fn list(&self, config: &SpokeAssetConfig) {
        let controller = self.gov.controller();
        self.env.as_contract(&controller, || {
            self.env
                .storage()
                .persistent()
                .set(&ControllerKey::SpokeAsset(self.spoke, self.key()), config);
        });
    }

    fn listing(&self) -> SpokeAssetConfig {
        controller::ControllerClient::new(self.env, &self.gov.controller())
            .get_spoke_asset(&self.spoke, &self.key())
    }

    /// Args that restate `listing()` field for field.
    fn restate(&self, config: &SpokeAssetConfig) -> SpokeAssetArgs {
        restate_listing(self.hub, self.spoke, self.asset.clone(), config)
    }

    /// Args that sit exactly on `grant`.
    fn at_grant(&self, grant: &CapGrant) -> SpokeAssetArgs {
        SpokeAssetArgs {
            hub_id: self.hub,
            asset: self.asset.clone(),
            spoke_id: self.spoke,
            can_collateral: grant.can_collateral,
            can_borrow: grant.can_borrow,
            paused: false,
            frozen: false,
            no_seize: false,
            ltv: grant.ltv,
            threshold: grant.threshold,
            bonus: grant.bonus,
            liquidation_fees: grant.liquidation_fees,
            supply_cap: grant.supply_cap,
            borrow_cap: grant.borrow_cap,
        }
    }

    fn propose_as(
        &self,
        caller: &Address,
        registrar: u32,
        op: AdminOperation,
        salt: u8,
    ) -> Result<
        Result<BytesN<32>, soroban_sdk::ConversionError>,
        Result<Error, soroban_sdk::InvokeError>,
    > {
        self.curator.try_propose(
            caller,
            &registrar,
            &op,
            &BytesN::from_array(self.env, &[salt; 32]),
        )
    }

    /// The grantee lists or edits in its own spoke.
    fn grantee_proposes(
        &self,
        op: AdminOperation,
        salt: u8,
    ) -> Result<
        Result<BytesN<32>, soroban_sdk::ConversionError>,
        Result<Error, soroban_sdk::InvokeError>,
    > {
        self.propose_as(&self.other, self.second, op, salt)
    }

    /// The hub owner edits a listing of its asset in the grantee's spoke.
    fn hub_owner_edits(
        &self,
        args: SpokeAssetArgs,
        salt: u8,
    ) -> Result<
        Result<BytesN<32>, soroban_sdk::ConversionError>,
        Result<Error, soroban_sdk::InvokeError>,
    > {
        self.propose_as(
            &self.admin,
            self.first,
            AdminOperation::EditAssetInSpoke(args),
            salt,
        )
    }

    fn queued(&self, op_id: &BytesN<32>) -> bool {
        self.gov.get_operation_ledger(op_id) != 0
    }

    fn pause_relaxation(&self) -> AdminOperation {
        AdminOperation::RelaxSpokeAssetFlags(RelaxSpokeAssetFlagsArgs {
            spoke_id: self.spoke,
            hub_asset: self.key(),
            expected_epoch: 0,
            paused: true,
            frozen: false,
            no_seize: false,
        })
    }
}

fn restate_listing(
    hub_id: u32,
    spoke_id: u32,
    asset: Address,
    config: &SpokeAssetConfig,
) -> SpokeAssetArgs {
    SpokeAssetArgs {
        hub_id,
        asset,
        spoke_id,
        can_collateral: config.is_collateralizable,
        can_borrow: config.is_borrowable,
        paused: config.paused,
        frozen: config.frozen,
        no_seize: config.no_seize,
        ltv: config.loan_to_value,
        threshold: config.liquidation_threshold,
        bonus: config.liquidation_bonus,
        liquidation_fees: config.liquidation_fees,
        supply_cap: config.supply_cap,
        borrow_cap: config.borrow_cap,
    }
}

fn live_listing() -> SpokeAssetConfig {
    SpokeAssetConfig {
        is_collateralizable: true,
        is_borrowable: false,
        paused: false,
        frozen: false,
        no_seize: false,
        loan_to_value: 4_000,
        liquidation_threshold: 6_000,
        liquidation_bonus: 300,
        liquidation_fees: 200,
        supply_cap: 700,
        borrow_cap: 500,
    }
}

// --- Caller signatures -------------------------------------------------------

#[test]
fn a_registrar_call_records_the_callers_signature() {
    let env = Env::default();
    let (owner, gov, curator) = setup(&env);
    let admin = Address::generate(&env);
    let registrar = register(&gov, &owner, &curator, &admin);

    curator.add_spoke(&admin, &registrar);
    assert!(
        env.auths().iter().any(|(signer, _)| signer == &admin),
        "add_spoke did not require the caller's signature"
    );
}

#[test]
fn a_registrar_call_without_the_callers_signature_fails() {
    let env = Env::default();
    let (owner, gov, curator) = setup(&env);
    let admin = Address::generate(&env);
    let registrar = register(&gov, &owner, &curator, &admin);

    env.set_auths(&[]);
    assert_eq!(curator.try_add_spoke(&admin, &registrar), Err(auth_error()));
    assert_eq!(curator.spoke_registrar(&1), None);
}

#[test]
fn a_governance_call_records_the_governance_signature() {
    let env = Env::default();
    let (_owner, gov, curator) = setup(&env);
    let admin = Address::generate(&env);

    let registrar = curator.register_registrar(&admin);
    assert!(
        env.auths().iter().any(|(signer, _)| signer == &gov.address),
        "register_registrar did not require the governance signature"
    );
    assert_eq!(curator.registrar_admin(&registrar), Some(admin));
}

#[test]
fn a_governance_call_without_the_governance_signature_fails() {
    let env = Env::default();
    let (_owner, _gov, curator) = setup(&env);
    let admin = Address::generate(&env);

    env.set_auths(&[]);
    assert_eq!(curator.try_register_registrar(&admin), Err(auth_error()));
}

// --- Governance writes on an unknown registrar -------------------------------

#[test]
fn governance_cannot_suspend_or_re_admin_an_unknown_registrar() {
    let env = Env::default();
    let (_owner, _gov, curator) = setup(&env);
    assert_eq!(
        curator.try_set_registrar_suspended(&99, &true),
        Err(Ok(CuratorError::UnknownRegistrar.into()))
    );
    assert_eq!(
        curator.try_force_set_admin(&99, &Address::generate(&env)),
        Err(Ok(CuratorError::UnknownRegistrar.into()))
    );
    assert!(!curator.registrar_suspended(&99));
    assert_eq!(curator.registrar_admin(&99), None);
}

// --- tighten_flags ----------------------------------------------------------

/// A spoke owner that does not raise a pause tightens a listing of a foreign
/// hub's asset without the live-pause check.
#[test]
fn a_spoke_owner_freezes_a_foreign_hub_listing_without_a_pause() {
    let env = Env::default();
    let p = pair(&env);
    p.list(&live_listing());

    p.curator.tighten_flags(
        &p.other,
        &p.second,
        &p.spoke,
        &p.key(),
        &false,
        &true,
        &false,
    );
    let after = p.listing();
    assert_eq!(
        (after.paused, after.frozen, after.no_seize),
        (false, true, false)
    );
}

#[test]
fn a_spoke_owner_may_keep_an_existing_pause_while_freezing_a_foreign_listing() {
    let env = Env::default();
    let p = pair(&env);
    let mut live = live_listing();
    live.paused = true;
    p.list(&live);

    p.curator.tighten_flags(
        &p.other,
        &p.second,
        &p.spoke,
        &p.key(),
        &true,
        &true,
        &false,
    );

    let after = p.listing();
    assert_eq!(
        (after.paused, after.frozen, after.no_seize),
        (true, true, false)
    );
}

#[test]
fn a_spoke_owner_cannot_add_a_pause_to_a_foreign_listing() {
    let env = Env::default();
    let p = pair(&env);
    p.list(&live_listing());

    assert_eq!(
        p.curator.try_tighten_flags(
            &p.other,
            &p.second,
            &p.spoke,
            &p.key(),
            &true,
            &true,
            &false,
        ),
        Err(Ok(CuratorError::ForeignPause.into()))
    );
    assert!(!p.listing().paused);
}

#[test]
fn a_hub_owner_freezes_a_foreign_spoke_without_changing_its_other_flags() {
    let env = Env::default();
    let p = pair(&env);
    let mut live = live_listing();
    live.paused = true;
    live.no_seize = true;
    p.list(&live);

    p.curator.tighten_flags(
        &p.admin,
        &p.first,
        &p.spoke,
        &p.key(),
        &false,
        &true,
        &false,
    );

    let after = p.listing();
    assert_eq!(
        (after.paused, after.frozen, after.no_seize),
        (true, true, true)
    );
}

// --- propose: which branch a listing edit takes ------------------------------

/// A grantee's edit of a listing in its own spoke is judged against its grant,
/// not as a hub owner's cap lowering, so it may raise a cap within the grant.
#[test]
fn a_grantee_raises_a_cap_in_its_own_spoke_within_the_grant() {
    let env = Env::default();
    let p = pair(&env);
    let live = live_listing();
    p.list(&live);
    let mut args = p.restate(&live);
    args.supply_cap = live.supply_cap + 1;

    let op_id = p
        .grantee_proposes(AdminOperation::EditAssetInSpoke(args), 1)
        .unwrap()
        .unwrap();
    assert!(p.queued(&op_id));
}

#[test]
fn a_grantee_may_propose_a_relaxation_that_keeps_a_foreign_listing_paused() {
    let env = Env::default();
    let p = pair(&env);
    let mut live = live_listing();
    live.paused = true;
    p.list(&live);

    let op_id = p
        .grantee_proposes(p.pause_relaxation(), 60)
        .unwrap()
        .unwrap();
    assert!(p.queued(&op_id));
}

#[test]
fn a_grantee_cannot_add_a_pause_to_a_foreign_listing_by_relaxation() {
    let env = Env::default();
    let p = pair(&env);
    p.list(&live_listing());

    assert_eq!(
        p.grantee_proposes(p.pause_relaxation(), 62),
        Err(Ok(CuratorError::ForeignPause.into()))
    );
    assert!(!p.listing().paused);
}

#[test]
fn a_grantee_may_edit_a_foreign_listing_without_lifting_its_existing_pause() {
    let env = Env::default();
    let p = pair(&env);
    let mut live = live_listing();
    live.paused = true;
    p.list(&live);

    let op_id = p
        .grantee_proposes(AdminOperation::EditAssetInSpoke(p.restate(&live)), 61)
        .unwrap()
        .unwrap();
    assert!(p.queued(&op_id));
}

#[test]
fn a_grantee_cannot_add_a_pause_to_a_foreign_listing_by_editing() {
    let env = Env::default();
    let p = pair(&env);
    let live = live_listing();
    p.list(&live);
    let mut args = p.restate(&live);
    args.paused = true;

    assert_eq!(
        p.grantee_proposes(AdminOperation::EditAssetInSpoke(args), 63),
        Err(Ok(CuratorError::ForeignPause.into()))
    );
    assert!(!p.listing().paused);
}

/// The hub owner lowers a cap of its asset in a spoke it does not own.
#[test]
fn the_hub_owner_lowers_a_cap_in_a_foreign_spoke() {
    let env = Env::default();
    let p = pair(&env);
    let live = live_listing();
    p.list(&live);

    let unchanged = p.hub_owner_edits(p.restate(&live), 2).unwrap().unwrap();
    assert!(p.queued(&unchanged));

    let mut args = p.restate(&live);
    args.supply_cap = live.supply_cap - 1;
    args.borrow_cap = live.borrow_cap - 1;
    let lowered = p.hub_owner_edits(args, 3).unwrap().unwrap();
    assert!(p.queued(&lowered));
}

#[test]
fn the_hub_owner_cannot_raise_a_cap_in_a_foreign_spoke() {
    let env = Env::default();
    let p = pair(&env);
    let live = live_listing();
    p.list(&live);

    let mut supply = p.restate(&live);
    supply.supply_cap = live.supply_cap + 1;
    assert_eq!(
        p.hub_owner_edits(supply, 4),
        Err(Ok(CuratorError::NotCapLowering.into()))
    );
    let mut borrow = p.restate(&live);
    borrow.borrow_cap = live.borrow_cap + 1;
    assert_eq!(
        p.hub_owner_edits(borrow, 5),
        Err(Ok(CuratorError::NotCapLowering.into()))
    );
}

/// Each listing field other than the two caps must restate the live value.
/// One field changes at a time, with both caps lowered, so only that field
/// can fail the check.
#[test]
fn the_hub_owner_cannot_change_any_other_listing_field_in_a_foreign_spoke() {
    let env = Env::default();
    let p = pair(&env);
    let live = live_listing();
    p.list(&live);

    let edits: [fn(&mut SpokeAssetArgs); 9] = [
        |a| a.can_collateral = !a.can_collateral,
        |a| a.can_borrow = !a.can_borrow,
        |a| a.paused = !a.paused,
        |a| a.frozen = !a.frozen,
        |a| a.no_seize = !a.no_seize,
        |a| a.ltv -= 1,
        |a| a.threshold -= 1,
        |a| a.bonus -= 1,
        |a| a.liquidation_fees -= 1,
    ];
    for (i, edit) in edits.iter().enumerate() {
        let mut args = p.restate(&live);
        args.supply_cap -= 1;
        args.borrow_cap -= 1;
        edit(&mut args);
        assert_eq!(
            p.hub_owner_edits(args, 10 + i as u8),
            Err(Ok(CuratorError::NotCapLowering.into())),
            "edit {i} passed"
        );
    }
}

// --- cancel_proposal --------------------------------------------------------

/// A grantee edit of its listing that sits on every limit of the grant.
/// Governance queues an edit without the oracle check a new listing needs.
fn listing_op(p: &Pair<'_>) -> AdminOperation {
    AdminOperation::EditAssetInSpoke(p.at_grant(&full_grant()))
}

/// The spoke owner cancels a listing it queued for a hub it does not own.
#[test]
fn the_spoke_owner_cancels_a_listing_queued_in_its_spoke() {
    let env = Env::default();
    let p = pair(&env);
    let op_id = p.grantee_proposes(listing_op(&p), 20).unwrap().unwrap();

    p.curator.cancel_proposal(&p.other, &p.second, &op_id);
    assert!(!p.queued(&op_id));
}

/// An operator holding only `ROLE_FLAGS` or only `ROLE_MARKET` cancels. An
/// operator holding only `ROLE_SPOKE` does not.
#[test]
fn a_flags_or_market_operator_cancels_but_a_spoke_operator_does_not() {
    let env = Env::default();
    let p = pair(&env);
    let spoke_op = Address::generate(&env);
    let flags_op = Address::generate(&env);
    let market_op = Address::generate(&env);
    p.curator
        .set_roles(&p.other, &p.second, &spoke_op, &ROLE_SPOKE);
    p.curator
        .set_roles(&p.other, &p.second, &flags_op, &ROLE_FLAGS);
    p.curator
        .set_roles(&p.other, &p.second, &market_op, &ROLE_MARKET);
    let first = p.grantee_proposes(listing_op(&p), 21).unwrap().unwrap();
    let second = p.grantee_proposes(listing_op(&p), 22).unwrap().unwrap();

    assert_eq!(
        p.curator.try_cancel_proposal(&spoke_op, &p.second, &first),
        Err(Ok(CuratorError::RoleMissing.into()))
    );
    p.curator.cancel_proposal(&flags_op, &p.second, &first);
    assert!(!p.queued(&first));
    p.curator.cancel_proposal(&market_op, &p.second, &second);
    assert!(!p.queued(&second));
}

// --- require_grant boundaries -----------------------------------------------

/// A listing edit that sits on every limit of the grant is within it.
#[test]
fn a_grantee_lists_exactly_at_the_grant() {
    let env = Env::default();
    let p = pair(&env);
    let op_id = p.grantee_proposes(listing_op(&p), 30).unwrap().unwrap();
    assert!(p.queued(&op_id));
}

#[test]
fn an_owner_edits_a_listing_in_its_own_hub_and_spoke_without_a_grant() {
    let env = Env::default();
    let (owner, gov, curator) = setup(&env);
    let admin = Address::generate(&env);
    let registrar = register(&gov, &owner, &curator, &admin);
    let hub = curator.create_hub(&admin, &registrar);
    let spoke = curator.add_spoke(&admin, &registrar);
    let asset = Address::generate(&env);
    let listing = live_listing();
    let key = HubAssetKey {
        hub_id: hub,
        asset: asset.clone(),
    };
    env.as_contract(&gov.controller(), || {
        env.storage()
            .persistent()
            .set(&ControllerKey::SpokeAsset(spoke, key), &listing);
    });

    let op_id = curator.propose(
        &admin,
        &registrar,
        &AdminOperation::EditAssetInSpoke(restate_listing(hub, spoke, asset, &listing)),
        &BytesN::from_array(&env, &[64; 32]),
    );

    assert_ne!(gov.get_operation_ledger(&op_id), 0);
}

/// One unit past any numeric limit of the grant is outside it.
#[test]
fn a_grantee_cannot_list_one_unit_past_any_grant_limit() {
    let env = Env::default();
    let p = pair(&env);
    let grant = full_grant();
    let edits: [fn(&mut SpokeAssetArgs); 6] = [
        |a| a.supply_cap += 1,
        |a| a.borrow_cap += 1,
        |a| a.ltv += 1,
        |a| a.threshold += 1,
        |a| a.bonus += 1,
        |a| a.liquidation_fees += 1,
    ];
    for (i, edit) in edits.iter().enumerate() {
        let mut args = p.at_grant(&grant);
        edit(&mut args);
        assert_eq!(
            p.grantee_proposes(AdminOperation::AddAssetToSpoke(args), 40 + i as u8),
            Err(Ok(CuratorError::GrantExceeded.into())),
            "edit {i} passed"
        );
    }
}

/// A grant that withholds collateral and borrowing admits a listing that asks
/// for neither, and rejects one that asks for either.
#[test]
fn a_grant_without_collateral_or_borrow_admits_only_a_listing_without_them() {
    let env = Env::default();
    let grant = CapGrant {
        can_collateral: false,
        can_borrow: false,
        ..full_grant()
    };
    let p = pair_with_grant(&env, &grant);

    let op_id = p
        .grantee_proposes(AdminOperation::EditAssetInSpoke(p.at_grant(&grant)), 50)
        .unwrap()
        .unwrap();
    assert!(p.queued(&op_id));

    let mut collateral = p.at_grant(&grant);
    collateral.can_collateral = true;
    assert_eq!(
        p.grantee_proposes(AdminOperation::AddAssetToSpoke(collateral), 51),
        Err(Ok(CuratorError::GrantExceeded.into()))
    );
    let mut borrow = p.at_grant(&grant);
    borrow.can_borrow = true;
    assert_eq!(
        p.grantee_proposes(AdminOperation::AddAssetToSpoke(borrow), 52),
        Err(Ok(CuratorError::GrantExceeded.into()))
    );
}
