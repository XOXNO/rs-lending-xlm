//! Governance guard regressions: a queued operation is bound to the
//! governance state it was proposed under and refused once that state moves.
//!
//! Ported from the PR #196 verification (governance checks g1a-c, g2a-b, g3,
//! g7) and from our `gov_epochs` fix family, onto the PR's `OperationGuard`
//! design (`contracts/governance/src/timelock/guard.rs`):
//!
//! - a guardian flag write advances the token's flag epoch, which every
//!   `AddAssetToSpoke` of the token, in any spoke and hub, records
//!   (`AssetFlagsChangedAfterProposal`, 59);
//! - every proposal the owner makes records the owner epoch, which a
//!   completed handover to another address advances (`NotAuthorized`, 44);
//! - an `ORACLE` band write advances the key's band epoch, which a
//!   `ConfigureAssetOracle` records next to the band value
//!   (`OracleBandChangedAfterProposal`, 58);
//! - `Unpause` records the emergency epoch (`EmergencyEpochMismatch`, 57);
//! - a canceller reset voided by a handover can be cancelled.
//!
//! Every step uses production entry points: `propose`, the guardian's and
//! oracle role's immediate calls, permissionless `execute` / `execute_self`
//! with no executor, `cancel` and `accept_ownership`. The harness owner holds
//! every role, so one address can play owner, proposer, guardian and oracle;
//! the roles are separate on mainnet.

use common::types::{
    AssetOracle, ControllerKey, HubAssetKey, PriceKey, SpokeAssetArgs, SpokeAssetConfig,
};
use controller::types::PositionLimits;
use governance_interface::{
    AdminOperation, ConfigureAssetOracleArgs, EditToleranceArgs, OperationState,
    RelaxSpokeAssetFlagsArgs, RemoveAssetFromSpokeArgs, RoleArgs, SpokeLiquidationCurveArgs,
    TransferOwnershipArgs,
};
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{contracttype, vec, Address, Bytes, BytesN, IntoVal, Symbol, Val, Vec};
use test_harness::errors::GenericError;
use test_harness::{
    assert_contract_error, errors, eth_preset, hub_asset, tolerance_band, usd, usdc_preset,
    LendingTest, ALICE, HARNESS_HUB, HARNESS_SPOKE, STABLECOIN_SPOKE,
};

/// Second spoke of the fixtures, created empty.
const SPOKE_B: u32 = 2;
const KATE: &str = "kate";
/// `governance::constants::TIMELOCK_RECOVERY_MIN_DELAY_LEDGERS`.
const RECOVERY: u32 = 518_400;
/// `governance::constants::TIMELOCK_OPERATION_GRACE_LEDGERS`.
const GRACE: u32 = 120_960;
/// stellar-contract-utils pausable `ExpectedPause`: unpause needs a paused contract.
const EXPECTED_PAUSE: u32 = 1001;
/// stellar-governance timelock `OperationAlreadyScheduled`.
const ALREADY_SCHEDULED: u32 = 4000;
const NOT_CANCELLABLE: u32 = GenericError::OperationNotCancellable as u32;
const EXPIRED: u32 = GenericError::TimelockOperationExpired as u32;
/// A routine, non-band change for an oracle operation to carry.
const ROUTINE_TOLERANCE_BPS: u32 = 400;

fn salt(t: &LendingTest, byte: u8) -> BytesN<32> {
    BytesN::<32>::from_array(&t.env, &[byte; 32])
}

fn sym(t: &LendingTest, s: &str) -> Symbol {
    Symbol::new(&t.env, s)
}

fn advance(t: &LendingTest, n: u32) {
    t.env.ledger().with_mut(|l| l.sequence_number += n);
}

fn delay(t: &LendingTest) -> u32 {
    t.gov_iface_client().get_min_delay()
}

fn flatten<T, C>(
    r: Result<Result<T, C>, Result<soroban_sdk::Error, soroban_sdk::InvokeError>>,
) -> Result<(), soroban_sdk::Error> {
    match r {
        Ok(_) => Ok(()),
        Err(Ok(e)) => Err(e),
        Err(Err(e)) => panic!("unexpected InvokeError {e:?}"),
    }
}

fn op_state(t: &LendingTest, id: &BytesN<32>) -> OperationState {
    t.gov_iface_client().get_operation_state(id)
}

fn propose(t: &LendingTest, proposer: &Address, op: &AdminOperation, byte: u8) -> BytesN<32> {
    t.gov_iface_client().propose(proposer, op, &salt(t, byte))
}

/// Permissionless `execute` with no executor and no signature at all.
fn exec_raw(
    t: &LendingTest,
    target: &Address,
    function: &str,
    args: Vec<Val>,
    byte: u8,
) -> Result<(), soroban_sdk::Error> {
    let gov = t.gov_iface_client();
    t.env.set_auths(&[]);
    let result = flatten(gov.try_execute(
        &None,
        target,
        &sym(t, function),
        &args,
        &salt(t, 0),
        &salt(t, byte),
    ));
    t.env.mock_all_auths_allowing_non_root_auth();
    result
}

/// Permissionless `execute_self` with no executor and no signature at all.
fn exec_self(t: &LendingTest, op: &AdminOperation, byte: u8) -> Result<(), soroban_sdk::Error> {
    let gov = t.gov_iface_client();
    t.env.set_auths(&[]);
    let result = flatten(gov.try_execute_self(&None, op, &salt(t, byte)));
    t.env.mock_all_auths_allowing_non_root_auth();
    result
}

fn exec_reset(t: &LendingTest, members: &Vec<Address>, byte: u8) -> Result<(), soroban_sdk::Error> {
    flatten(
        t.gov_iface_client()
            .try_execute_canceller_reset(&None, members, &salt(t, byte)),
    )
}

fn grant(t: &LendingTest, who: &Address, role: &str) -> AdminOperation {
    AdminOperation::GrantGovRole(RoleArgs {
        account: who.clone(),
        role: sym(t, role),
    })
}

fn grant_fresh(t: &LendingTest, role: &str) -> Address {
    let who = Address::generate(&t.env);
    t.gov_client()
        .execute_immediate(&t.admin(), &grant(t, &who, role));
    who
}

fn transfer_to(t: &LendingTest, who: &Address) -> AdminOperation {
    AdminOperation::TransferGovOwnership(TransferOwnershipArgs {
        new_owner: who.clone(),
        live_until_ledger: t.env.ledger().sequence() + 10_000,
    })
}

/// Proposes `op` as `proposer`, waits the harness delay (above the 12-ledger
/// Sensitive floor) and executes it with no executor.
fn run_self_op(t: &LendingTest, proposer: &Address, op: &AdminOperation, byte: u8) {
    propose(t, proposer, op, byte);
    advance(t, delay(t));
    exec_self(t, op, byte).expect("self op executes");
}

/// Executes the queued handover and has `to` accept it (mocked auth).
fn hand_over(t: &LendingTest, handover: &AdminOperation, byte: u8, to: &Address) {
    exec_self(t, handover, byte).expect("the handover executes");
    t.gov_iface_client().accept_ownership();
    assert!(t.gov_iface_client().has_role(to, &sym(t, "PROPOSER")));
    assert!(!t
        .gov_iface_client()
        .has_role(&t.admin(), &sym(t, "PROPOSER")));
}

fn key(t: &LendingTest, asset: &str) -> HubAssetKey {
    hub_asset(t.resolve_asset(asset))
}

fn listing(t: &LendingTest, spoke: u32, k: &HubAssetKey) -> SpokeAssetConfig {
    t.ctrl_client().get_spoke_asset(&spoke, k)
}

fn try_listing(t: &LendingTest, spoke: u32, k: &HubAssetKey) -> Result<(), soroban_sdk::Error> {
    flatten(t.ctrl_client().try_get_spoke_asset(&spoke, k))
}

/// The harness-spoke listing of `asset` re-encoded as a fresh, unflagged
/// listing of the same asset in `spoke`.
fn listing_args(t: &LendingTest, asset: &str, spoke: u32) -> SpokeAssetArgs {
    let cfg = listing(t, HARNESS_SPOKE, &key(t, asset));
    SpokeAssetArgs {
        hub_id: HARNESS_HUB,
        asset: t.resolve_asset(asset),
        spoke_id: spoke,
        can_collateral: cfg.is_collateralizable,
        can_borrow: cfg.is_borrowable,
        paused: false,
        frozen: false,
        no_seize: false,
        ltv: cfg.loan_to_value,
        threshold: cfg.liquidation_threshold,
        bonus: cfg.liquidation_bonus,
        liquidation_fees: cfg.liquidation_fees,
        supply_cap: cfg.supply_cap,
        borrow_cap: cfg.borrow_cap,
    }
}

fn add_call(t: &LendingTest, args: &SpokeAssetArgs) -> Vec<Val> {
    vec![&t.env, args.clone().into_val(&t.env)]
}

fn exec_add(t: &LendingTest, args: &SpokeAssetArgs, byte: u8) -> Result<(), soroban_sdk::Error> {
    exec_raw(
        t,
        &t.controller,
        "add_asset_to_spoke",
        add_call(t, args),
        byte,
    )
}

/// `guardian` freezes `asset` in the harness spoke through governance.
fn freeze(t: &LendingTest, guardian: &Address, asset: &str) {
    t.gov_iface_client().set_spoke_asset_flags(
        guardian,
        &HARNESS_SPOKE,
        &key(t, asset),
        &false,
        &true,
        &false,
    );
    assert!(listing(t, HARNESS_SPOKE, &key(t, asset)).frozen);
}

/// Supplies `raw` units of `asset` under `k` (any hub) into a new account in
/// `spoke`.
fn try_supply_key(
    t: &mut LendingTest,
    user: &str,
    asset: &str,
    k: &HubAssetKey,
    spoke: u32,
    raw: i128,
) -> Result<(), soroban_sdk::Error> {
    let who = t.get_or_create_user(user);
    t.resolve_market(asset).token_admin.mint(&who, &raw);
    flatten(
        t.ctrl_client()
            .try_supply(&who, &0u64, &spoke, &vec![&t.env, (k.clone(), raw)]),
    )
}

/// Safe outcome of g1: the pre-queued add is refused by the flag epoch and one
/// unit of USDC cannot be supplied into the listing it would have created.
fn assert_frozen_token_listing_held(
    label: &str,
    exec: Result<(), soroban_sdk::Error>,
    landed: Option<SpokeAssetConfig>,
    supply: Result<(), soroban_sdk::Error>,
) {
    let flags = landed.as_ref().map(|c| (c.paused, c.frozen, c.no_seize));
    assert_eq!(
        exec,
        Err(soroban_sdk::Error::from_contract_error(
            errors::ASSET_FLAGS_CHANGED_AFTER_PROPOSAL
        )),
        "{label}: the listing queued before the freeze was not refused (flags={flags:?})"
    );
    assert!(
        landed.is_none(),
        "{label}: a listing was written: {flags:?}"
    );
    assert!(
        supply.is_err(),
        "{label}: 1 USDC was supplied into the listing queued before the freeze"
    );
}

// ---------------------------------------------------------------------------
// MC-1: a listing of a frozen token queued before the freeze (g1a, g1b, g1c)
// ---------------------------------------------------------------------------

/// g1a: another existing spoke. ETH is the control.
#[test]
fn pre_queued_listing_of_the_frozen_token_in_another_spoke_is_held() {
    let mut t = LendingTest::new()
        .standard_two_asset()
        .with_spoke(SPOKE_B, STABLECOIN_SPOKE)
        .build();
    let guardian = grant_fresh(&t, "GUARDIAN");
    let usdc_args = listing_args(&t, "USDC", SPOKE_B);
    let eth_args = listing_args(&t, "ETH", SPOKE_B);
    let usdc_id = propose(
        &t,
        &t.admin(),
        &AdminOperation::AddAssetToSpoke(usdc_args.clone()),
        1,
    );
    propose(
        &t,
        &t.admin(),
        &AdminOperation::AddAssetToSpoke(eth_args.clone()),
        2,
    );

    freeze(&t, &guardian, "USDC");
    advance(&t, delay(&t));
    assert_eq!(op_state(&t, &usdc_id), OperationState::Ready);

    let exec = exec_add(&t, &usdc_args, 1);
    let landed = try_listing(&t, SPOKE_B, &key(&t, "USDC"))
        .ok()
        .map(|_| listing(&t, SPOKE_B, &key(&t, "USDC")));
    let supply = t
        .try_supply_with_spoke(KATE, "USDC", 1.0, SPOKE_B)
        .map(|_| ());

    exec_add(&t, &eth_args, 2).expect("control: a pending listing of an unfrozen asset executes");

    assert_frozen_token_listing_held("g1a (another spoke)", exec, landed, supply);
    assert_eq!(op_state(&t, &usdc_id), OperationState::Ready);
}

/// g1b: the next sequential spoke id, created by a queued `AddSpoke`.
#[test]
fn pre_queued_listing_into_the_next_spoke_id_is_held() {
    let mut t = LendingTest::new().standard_two_asset().build();
    let guardian = grant_fresh(&t, "GUARDIAN");
    let next_spoke = HARNESS_SPOKE + 1;
    let usdc_args = listing_args(&t, "USDC", next_spoke);
    propose(&t, &t.admin(), &AdminOperation::AddSpoke, 1);
    propose(
        &t,
        &t.admin(),
        &AdminOperation::AddAssetToSpoke(usdc_args.clone()),
        2,
    );

    freeze(&t, &guardian, "USDC");
    advance(&t, delay(&t));
    exec_raw(&t, &t.controller, "add_spoke", vec![&t.env], 1)
        .expect("the queued AddSpoke executes");
    assert!(!t.ctrl_client().get_spoke(&next_spoke).is_deprecated);

    let exec = exec_add(&t, &usdc_args, 2);
    let landed = try_listing(&t, next_spoke, &key(&t, "USDC"))
        .ok()
        .map(|_| listing(&t, next_spoke, &key(&t, "USDC")));
    let supply = t
        .try_supply_with_spoke(KATE, "USDC", 1.0, next_spoke)
        .map(|_| ());

    assert_frozen_token_listing_held("g1b (next spoke id)", exec, landed, supply);
}

/// g1c: another hub. Hub 2's USDC market is a different `HubAssetKey`; the
/// freeze is on hub 1's listing in the harness spoke, and the flag epoch is
/// per token.
#[test]
fn pre_queued_cross_hub_listing_is_held() {
    let mut t = LendingTest::new()
        .standard_two_asset()
        .with_spoke(SPOKE_B, STABLECOIN_SPOKE)
        .build();
    let guardian = grant_fresh(&t, "GUARDIAN");
    let hub2 = t.create_hub();
    t.list_market_on_hub(hub2, "USDC", 1_000.0);
    let mut args = listing_args(&t, "USDC", SPOKE_B);
    args.hub_id = hub2;
    let hub2_key = HubAssetKey {
        hub_id: hub2,
        asset: t.resolve_asset("USDC"),
    };
    propose(
        &t,
        &t.admin(),
        &AdminOperation::AddAssetToSpoke(args.clone()),
        1,
    );

    freeze(&t, &guardian, "USDC");
    advance(&t, delay(&t));
    t.refresh_oracle_prices();
    let exec = exec_add(&t, &args, 1);
    let landed = try_listing(&t, SPOKE_B, &hub2_key)
        .ok()
        .map(|_| listing(&t, SPOKE_B, &hub2_key));
    let supply = try_supply_key(&mut t, KATE, "USDC", &hub2_key, SPOKE_B, 10_000_000);

    assert_frozen_token_listing_held("g1c (cross-hub)", exec, landed, supply);
}

/// Our `gov_epochs` reproduction: the dead listing is cleared by a canceller,
/// its salt is reused for a listing proposed with knowledge of the freeze,
/// and that one runs after its delay.
#[test]
fn listing_proposed_after_the_freeze_reuses_the_salt_and_executes() {
    let mut t = LendingTest::new()
        .standard_two_asset()
        .with_spoke(SPOKE_B, STABLECOIN_SPOKE)
        .build();
    let admin = t.admin();
    let usdc_args = listing_args(&t, "USDC", SPOKE_B);
    let add_usdc = AdminOperation::AddAssetToSpoke(usdc_args.clone());
    let usdc_id = propose(&t, &admin, &add_usdc, 1);

    freeze(&t, &admin, "USDC");
    advance(&t, delay(&t));
    assert_contract_error(
        exec_add(&t, &usdc_args, 1),
        errors::ASSET_FLAGS_CHANGED_AFTER_PROPOSAL,
    );

    t.gov_iface_client().cancel(&admin, &usdc_id);
    assert_eq!(op_state(&t, &usdc_id), OperationState::Unset);
    let again = propose(&t, &admin, &add_usdc, 1);
    assert_eq!(again, usdc_id, "same operation id, fresh guard");
    advance(&t, delay(&t));
    exec_add(&t, &usdc_args, 1).expect("a listing proposed after the freeze executes");
    assert_eq!(op_state(&t, &usdc_id), OperationState::Unset);
    t.try_supply_with_spoke(KATE, "USDC", 1.0, SPOKE_B)
        .expect("the post-freeze listing is live");
}

/// Abuse probe: a guardian cannot move the flag epoch with a call the
/// controller refuses. Clearing the freeze through the immediate path fails
/// with `SpokeAssetFlagRelaxation` and the epoch advance reverts with it, so
/// a listing proposed after the freeze still executes.
#[test]
fn refused_flag_write_does_not_move_the_flag_epoch() {
    let mut t = LendingTest::new()
        .standard_two_asset()
        .with_spoke(SPOKE_B, STABLECOIN_SPOKE)
        .build();
    let admin = t.admin();
    freeze(&t, &admin, "USDC");
    let args = listing_args(&t, "USDC", SPOKE_B);
    propose(
        &t,
        &admin,
        &AdminOperation::AddAssetToSpoke(args.clone()),
        1,
    );

    assert_contract_error(
        flatten(t.gov_iface_client().try_set_spoke_asset_flags(
            &admin,
            &HARNESS_SPOKE,
            &key(&t, "USDC"),
            &false,
            &false,
            &false,
        )),
        errors::SPOKE_ASSET_FLAG_RELAXATION,
    );
    assert!(listing(&t, HARNESS_SPOKE, &key(&t, "USDC")).frozen);

    advance(&t, delay(&t));
    exec_add(&t, &args, 1).expect("a refused guardian write leaves the flag epoch where it was");
    t.try_supply_with_spoke(KATE, "USDC", 1.0, SPOKE_B)
        .expect("the listing proposed after the freeze is live");
}

/// Sibling: `EditAssetInSpoke` is not bound to the flag epoch, so check it
/// cannot act as the add arm. A pending edit with clear flags is refused by
/// the controller's ratchet while the listing is frozen, and once the
/// pending removal has deleted the listing it is refused with
/// `AssetNotInSpoke`: only `add_asset_to_spoke` can create a listing.
#[test]
fn pending_edit_cannot_relist_after_a_queued_removal() {
    let mut t = LendingTest::new()
        .standard_two_asset()
        .with_spoke(SPOKE_B, STABLECOIN_SPOKE)
        .with_spoke_asset(SPOKE_B, "USDC", true, true)
        .build();
    let admin = t.admin();
    let usdc = key(&t, "USDC");
    let mut edit_args = listing_args(&t, "USDC", SPOKE_B);
    let live = listing(&t, SPOKE_B, &usdc);
    edit_args.can_collateral = live.is_collateralizable;
    edit_args.can_borrow = live.is_borrowable;
    let remove = AdminOperation::RemoveAssetFromSpoke(RemoveAssetFromSpokeArgs {
        hub_asset: usdc.clone(),
        spoke_id: SPOKE_B,
    });
    propose(&t, &admin, &remove, 1);
    let edit_id = propose(
        &t,
        &admin,
        &AdminOperation::EditAssetInSpoke(edit_args.clone()),
        2,
    );

    t.gov_iface_client()
        .set_spoke_asset_flags(&admin, &SPOKE_B, &usdc, &false, &true, &false);
    advance(&t, delay(&t));

    let edit_call: Vec<Val> = vec![&t.env, edit_args.clone().into_val(&t.env)];
    assert_contract_error(
        exec_raw(
            &t,
            &t.controller,
            "edit_asset_in_spoke",
            edit_call.clone(),
            2,
        ),
        errors::SPOKE_ASSET_FLAG_RELAXATION,
    );
    exec_raw(
        &t,
        &t.controller,
        "remove_asset_from_spoke",
        remove_call(&t, &usdc, SPOKE_B),
        1,
    )
    .expect("the empty frozen listing is removed");
    assert_contract_error(
        exec_raw(&t, &t.controller, "edit_asset_in_spoke", edit_call, 2),
        errors::ASSET_NOT_IN_SPOKE,
    );
    assert_eq!(op_state(&t, &edit_id), OperationState::Ready);
    assert_contract_error(try_listing(&t, SPOKE_B, &usdc), errors::ASSET_NOT_IN_SPOKE);
    assert_contract_error(
        t.try_supply_with_spoke(KATE, "USDC", 1.0, SPOKE_B),
        errors::ASSET_NOT_IN_SPOKE,
    );
}

/// Residual: a guardian flag write that changes nothing still advances the
/// token's flag epoch, so a GUARDIAN key can void every pending listing of a
/// token that is listed somewhere, repeatedly, without tightening anything.
/// Bounded by the owner's `revoke_role_immediate`; pinned so a change to
/// this rule is deliberate.
#[test]
fn residual_noop_guardian_flag_write_voids_a_pending_listing() {
    let t = LendingTest::new()
        .standard_two_asset()
        .with_spoke(SPOKE_B, STABLECOIN_SPOKE)
        .build();
    let admin = t.admin();
    let usdc = key(&t, "USDC");
    let args = listing_args(&t, "USDC", SPOKE_B);
    let before = listing(&t, HARNESS_SPOKE, &usdc);
    propose(
        &t,
        &admin,
        &AdminOperation::AddAssetToSpoke(args.clone()),
        1,
    );

    t.gov_iface_client().set_spoke_asset_flags(
        &admin,
        &HARNESS_SPOKE,
        &usdc,
        &before.paused,
        &before.frozen,
        &before.no_seize,
    );
    let after = listing(&t, HARNESS_SPOKE, &usdc);
    assert_eq!(
        (after.paused, after.frozen, after.no_seize),
        (before.paused, before.frozen, before.no_seize),
        "nothing changed"
    );
    advance(&t, delay(&t));
    assert_contract_error(
        exec_add(&t, &args, 1),
        errors::ASSET_FLAGS_CHANGED_AFTER_PROPOSAL,
    );
}

// ---------------------------------------------------------------------------
// MC-1: delisting a frozen, empty listing (g2a, g2b)
// ---------------------------------------------------------------------------

fn remove_call(t: &LendingTest, k: &HubAssetKey, spoke: u32) -> Vec<Val> {
    vec![&t.env, k.clone().into_val(&t.env), spoke.into_val(&t.env)]
}

fn relax_call(t: &LendingTest, args: &RelaxSpokeAssetFlagsArgs) -> Vec<Val> {
    vec![
        &t.env,
        args.spoke_id.into_val(&t.env),
        args.hub_asset.clone().into_val(&t.env),
        args.expected_epoch.into_val(&t.env),
        args.paused.into_val(&t.env),
        args.frozen.into_val(&t.env),
        args.no_seize.into_val(&t.env),
    ]
}

fn propose_removal(t: &LendingTest, k: &HubAssetKey, byte: u8) -> BytesN<32> {
    propose(
        t,
        &t.admin(),
        &AdminOperation::RemoveAssetFromSpoke(RemoveAssetFromSpokeArgs {
            hub_asset: k.clone(),
            spoke_id: HARNESS_SPOKE,
        }),
        byte,
    )
}

/// g2a: a frozen listing with zero usage is delisted directly.
#[test]
fn frozen_empty_listing_can_be_removed() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let k = key(&t, "USDC");
    let guardian = grant_fresh(&t, "GUARDIAN");
    freeze(&t, &guardian, "USDC");

    propose_removal(&t, &k, 7);
    advance(&t, delay(&t));
    exec_raw(
        &t,
        &t.controller,
        "remove_asset_from_spoke",
        remove_call(&t, &k, HARNESS_SPOKE),
        7,
    )
    .expect("a frozen listing with zero usage is removed");
    assert_contract_error(
        try_listing(&t, HARNESS_SPOKE, &k),
        errors::ASSET_NOT_IN_SPOKE,
    );
}

/// g2b, nearest safe variant: delisting a frozen listing needs no relaxation,
/// so it keeps the listing closed until it is gone and a stranger's deposit
/// in the same ledger cannot block it. (The verification's g2b drove the
/// PR's relax-then-remove flow; see the residual test below.)
#[test]
fn delisting_a_frozen_listing_cannot_be_griefed_by_a_deposit() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    let k = key(&t, "USDC");
    let guardian = grant_fresh(&t, "GUARDIAN");
    freeze(&t, &guardian, "USDC");
    propose_removal(&t, &k, 7);
    advance(&t, delay(&t));
    t.refresh_oracle_prices();

    let grief = t.try_supply(KATE, "USDC", 1.0).map(|_| ());
    assert_contract_error(grief, errors::SPOKE_ASSET_FROZEN);
    exec_raw(
        &t,
        &t.controller,
        "remove_asset_from_spoke",
        remove_call(&t, &k, HARNESS_SPOKE),
        7,
    )
    .expect("the removal lands");
    assert_contract_error(
        try_listing(&t, HARNESS_SPOKE, &k),
        errors::ASSET_NOT_IN_SPOKE,
    );
}

/// Residual (the PR's former flow, g2b as verified): a relaxation queued next
/// to the removal can be executed by anyone first, which reopens entry, and a
/// one-unit deposit then blocks the removal with `SpokeAssetInUse`. The
/// freeze runbook therefore removes a frozen listing without a relaxation.
#[test]
fn residual_a_relaxation_queued_with_the_removal_reopens_entry_for_a_grief_deposit() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    let k = key(&t, "USDC");
    let guardian = grant_fresh(&t, "GUARDIAN");
    freeze(&t, &guardian, "USDC");
    let relax = RelaxSpokeAssetFlagsArgs {
        spoke_id: HARNESS_SPOKE,
        hub_asset: k.clone(),
        expected_epoch: t
            .ctrl_client()
            .get_spoke_asset_flags_epoch(&HARNESS_SPOKE, &k),
        paused: false,
        frozen: false,
        no_seize: false,
    };
    propose(
        &t,
        &t.admin(),
        &AdminOperation::RelaxSpokeAssetFlags(relax.clone()),
        9,
    );
    propose_removal(&t, &k, 7);
    advance(&t, delay(&t));
    t.refresh_oracle_prices();

    exec_raw(
        &t,
        &t.controller,
        "relax_spoke_asset_flags",
        relax_call(&t, &relax),
        9,
    )
    .expect("the Ready relax executes for anyone");
    t.try_supply(KATE, "USDC", 1.0)
        .expect("residual: the relax reopened entry");
    assert_contract_error(
        exec_raw(
            &t,
            &t.controller,
            "remove_asset_from_spoke",
            remove_call(&t, &k, HARNESS_SPOKE),
            7,
        ),
        errors::SPOKE_ASSET_IN_USE,
    );
}

// ---------------------------------------------------------------------------
// MC-2: the former owner's proposals die with the handover (g3)
// ---------------------------------------------------------------------------

fn eth_live_args(t: &LendingTest) -> SpokeAssetArgs {
    let live = listing(t, HARNESS_SPOKE, &key(t, "ETH"));
    SpokeAssetArgs {
        hub_id: HARNESS_HUB,
        asset: t.resolve_asset("ETH"),
        spoke_id: HARNESS_SPOKE,
        can_collateral: live.is_collateralizable,
        can_borrow: live.is_borrowable,
        paused: live.paused,
        frozen: live.frozen,
        no_seize: live.no_seize,
        ltv: live.loan_to_value,
        threshold: live.liquidation_threshold,
        bonus: live.liquidation_bonus,
        liquidation_fees: live.liquidation_fees,
        supply_cap: live.supply_cap,
        borrow_cap: live.borrow_cap,
    }
}

fn manager_active(t: &LendingTest, m: &Address) -> bool {
    t.env.as_contract(&t.controller, || {
        t.env
            .storage()
            .persistent()
            .has(&ControllerKey::PositionManager(m.clone()))
    })
}

/// g3: proposer-open operations the former owner queued (a liquidation
/// threshold cut, `SetPositionManager`, a `RevokeGovRole` against the new
/// owner's guardian) all die with the handover.
#[test]
fn former_owner_proposer_open_leftovers_die_with_the_handover() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(ALICE, "ETH", 10.0);
    t.borrow(ALICE, "USDC", 6000.0);
    let o1 = t.admin();
    let o2 = Address::generate(&t.env);
    let g = Address::generate(&t.env);
    let m = Address::generate(&t.env);
    let gov = t.gov_iface_client();

    let live = eth_live_args(&t);
    let mut cut = live.clone();
    cut.threshold = live.ltv + 1;
    propose(&t, &o1, &AdminOperation::EditAssetInSpoke(cut.clone()), 1);
    let handover = transfer_to(&t, &o2);
    propose(&t, &o1, &handover, 2);
    propose(
        &t,
        &o1,
        &AdminOperation::SetPositionManager(m.clone(), true),
        3,
    );
    let revoke_g = AdminOperation::RevokeGovRole(RoleArgs {
        account: g.clone(),
        role: sym(&t, "GUARDIAN"),
    });
    propose(&t, &o1, &revoke_g, 4);

    advance(&t, delay(&t));
    t.refresh_oracle_prices();
    hand_over(&t, &handover, 2, &o2);
    // The new owner seats its own guardian after acceptance.
    t.gov_client()
        .execute_immediate(&o2, &grant(&t, &g, "GUARDIAN"));
    assert!(gov.has_role(&g, &sym(&t, "GUARDIAN")));

    assert_contract_error(
        exec_raw(
            &t,
            &t.controller,
            "edit_asset_in_spoke",
            add_call(&t, &cut),
            1,
        ),
        errors::NOT_AUTHORIZED,
    );
    assert_eq!(
        listing(&t, HARNESS_SPOKE, &key(&t, "ETH")).liquidation_threshold,
        live.threshold
    );
    assert_contract_error(
        exec_raw(
            &t,
            &t.controller,
            "set_position_manager",
            vec![&t.env, m.clone().into_val(&t.env), true.into_val(&t.env)],
            3,
        ),
        errors::NOT_AUTHORIZED,
    );
    assert!(!manager_active(&t, &m));
    assert_contract_error(exec_self(&t, &revoke_g, 4), errors::NOT_AUTHORIZED);
    assert!(gov.has_role(&g, &sym(&t, "GUARDIAN")));
}

/// MC-2, independent proposer: a PROPOSER that is not the owner keeps its
/// pending work across the handover, while the former owner's own copy of
/// the same change dies with it.
#[test]
fn independent_proposer_work_survives_the_handover() {
    let t = LendingTest::new().standard_two_asset().build();
    let o1 = t.admin();
    let o2 = Address::generate(&t.env);
    let p = grant_fresh(&t, "PROPOSER");
    let limits = PositionLimits {
        max_supply_positions: 4,
        max_borrow_positions: 3,
    };
    let set_limits = AdminOperation::SetPositionLimits(limits.clone());
    let limits_args: Vec<Val> = vec![&t.env, limits.into_val(&t.env)];
    propose(&t, &p, &set_limits, 1);
    propose(&t, &o1, &set_limits, 2);
    let handover = transfer_to(&t, &o2);
    propose(&t, &o1, &handover, 3);
    advance(&t, delay(&t));
    hand_over(&t, &handover, 3, &o2);

    assert_contract_error(
        exec_raw(
            &t,
            &t.controller,
            "set_position_limits",
            limits_args.clone(),
            2,
        ),
        errors::NOT_AUTHORIZED,
    );
    exec_raw(&t, &t.controller, "set_position_limits", limits_args, 1)
        .expect("the independent proposer's operation survives");
    assert_eq!(t.get_position_limits().max_supply_positions, 4);
}

/// Our `gov_epochs` MC-2 reproduction: every operation the previous owner
/// queued is refused after `accept_ownership`; the new owner, who holds
/// CANCELLER, clears them and its own proposals run. A self-transfer does not
/// move the owner epoch, so the owner's own pending work survives it.
#[test]
fn owner_queue_dies_with_a_handover_and_survives_a_self_transfer() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let gov = t.gov_iface_client();
    let d = delay(&t);
    let a = t.admin();
    let b = Address::generate(&t.env);
    let canceller = sym(&t, "CANCELLER");
    let limits = PositionLimits {
        max_supply_positions: 4,
        max_borrow_positions: 3,
    };
    let limits_args: Vec<Val> = vec![&t.env, limits.clone().into_val(&t.env)];

    let to_b = transfer_to(&t, &b);
    let rearm = grant(&t, &a, "CANCELLER");
    let set_limits = AdminOperation::SetPositionLimits(limits.clone());
    propose(&t, &a, &to_b, 1);
    let rearm_id = propose(&t, &a, &rearm, 2);
    let limits_id = propose(&t, &a, &set_limits, 3);
    advance(&t, d);

    exec_self(&t, &to_b, 1).expect("the handover itself executes");
    gov.accept_ownership();
    assert!(gov.has_role(&b, &canceller));
    assert!(!gov.has_role(&a, &canceller));

    assert_contract_error(exec_self(&t, &rearm, 2), errors::NOT_AUTHORIZED);
    assert_contract_error(
        exec_raw(
            &t,
            &t.controller,
            "set_position_limits",
            limits_args.clone(),
            3,
        ),
        errors::NOT_AUTHORIZED,
    );
    assert!(
        !gov.has_role(&a, &canceller),
        "the former owner stays unarmed"
    );
    assert_eq!(op_state(&t, &rearm_id), OperationState::Ready);
    assert_eq!(op_state(&t, &limits_id), OperationState::Ready);

    gov.cancel(&b, &rearm_id);
    gov.cancel(&b, &limits_id);
    assert_eq!(op_state(&t, &rearm_id), OperationState::Unset);
    let fresh_id = propose(&t, &b, &set_limits, 3);
    assert_eq!(fresh_id, limits_id);
    advance(&t, d);
    exec_raw(
        &t,
        &t.controller,
        "set_position_limits",
        limits_args.clone(),
        3,
    )
    .expect("the new owner's proposal executes");
    assert_eq!(t.get_position_limits().max_supply_positions, 4);

    // A self-transfer B -> B keeps the epoch: B's pending work survives it.
    let later_limits = PositionLimits {
        max_supply_positions: 5,
        max_borrow_positions: 3,
    };
    let later_id = propose(
        &t,
        &b,
        &AdminOperation::SetPositionLimits(later_limits.clone()),
        4,
    );
    run_self_op(&t, &b, &transfer_to(&t, &b), 5);
    gov.accept_ownership();
    assert!(gov.has_role(&b, &canceller));
    assert_eq!(op_state(&t, &later_id), OperationState::Ready);
    exec_raw(
        &t,
        &t.controller,
        "set_position_limits",
        vec![&t.env, later_limits.into_val(&t.env)],
        4,
    )
    .expect("a self-transfer does not invalidate the owner's queue");
}

fn upload_controller_wasm(t: &LendingTest) -> BytesN<32> {
    let mut bytes = std::fs::read("target/wasm32v1-none/release/controller.wasm");
    if bytes.is_err() {
        bytes = std::fs::read("../../target/wasm32v1-none/release/controller.wasm");
    }
    let bytes = bytes.expect("controller WASM not found. Run 'make build' first.");
    t.env
        .deployer()
        .upload_contract_wasm(Bytes::from_slice(&t.env, &bytes))
}

/// MC-2 abuse probe: an address-valued binding would revive the old queue
/// after a round trip A -> B -> A. The epoch is monotonic, so every
/// operation A queued before the first handover stays dead after A is owner
/// again, on every execute path and with a named EXECUTOR: the ownership
/// hijack via `execute_self`, an external-target `UpgradeController` via
/// `execute`, and a routine leftover. The hijack target cannot accept.
#[test]
fn round_trip_handover_keeps_the_old_queue_dead() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    let gov = t.gov_iface_client();
    let d = delay(&t);
    let a = t.admin();
    let b = Address::generate(&t.env);
    let c = Address::generate(&t.env);
    let hash = upload_controller_wasm(&t);
    let upgrade_args: Vec<Val> = vec![&t.env, hash.clone().into_val(&t.env)];
    let limits = PositionLimits {
        max_supply_positions: 4,
        max_borrow_positions: 3,
    };
    let limits_args: Vec<Val> = vec![&t.env, limits.clone().into_val(&t.env)];

    let to_b = transfer_to(&t, &b);
    let hijack = transfer_to(&t, &c);
    propose(&t, &a, &to_b, 1);
    let hijack_id = propose(&t, &a, &hijack, 2);
    let upgrade_id = propose(&t, &a, &AdminOperation::UpgradeController(hash), 3);
    let limits_id = propose(&t, &a, &AdminOperation::SetPositionLimits(limits), 4);
    advance(&t, d);

    exec_self(&t, &to_b, 1).expect("A -> B executes");
    gov.accept_ownership();
    run_self_op(&t, &b, &transfer_to(&t, &a), 5);
    gov.accept_ownership();
    assert!(gov.has_role(&a, &sym(&t, "EXECUTOR")), "A is owner again");
    assert!(!gov.has_role(&b, &sym(&t, "EXECUTOR")));

    assert_contract_error(exec_self(&t, &hijack, 2), errors::NOT_AUTHORIZED);
    assert_contract_error(
        flatten(gov.try_execute(
            &Some(a.clone()),
            &t.controller,
            &sym(&t, "upgrade"),
            &upgrade_args,
            &salt(&t, 0),
            &salt(&t, 3),
        )),
        errors::NOT_AUTHORIZED,
    );
    assert_contract_error(
        exec_raw(&t, &t.controller, "set_position_limits", limits_args, 4),
        errors::NOT_AUTHORIZED,
    );
    for id in [&hijack_id, &upgrade_id, &limits_id] {
        assert_eq!(op_state(&t, id), OperationState::Ready);
    }
    assert_contract_error(
        flatten(gov.try_accept_ownership()),
        errors::NO_PENDING_TRANSFER,
    );
    assert!(!gov.has_role(&c, &sym(&t, "PROPOSER")));
    t.try_supply(ALICE, "USDC", 1.0)
        .expect("the controller was never upgraded or paused by the leftover");
}

/// Residual (MC-2 scope): the owner epoch severs the QUEUE at
/// `accept_ownership`, not state the departing owner creates before it. A
/// grant to an accomplice queued next to the handover can be executed by
/// anyone in the window between the transfer executing and the new owner
/// accepting; the seat survives the handover. The incoming owner audits role
/// holders after accepting.
#[test]
fn residual_leftover_executed_before_acceptance_survives() {
    let t = LendingTest::new().build();
    let gov = t.gov_iface_client();
    let a = t.admin();
    let b = Address::generate(&t.env);
    let x = Address::generate(&t.env);
    let to_b = transfer_to(&t, &b);
    let arm_x = grant(&t, &x, "CANCELLER");
    propose(&t, &a, &to_b, 1);
    propose(&t, &a, &arm_x, 2);
    advance(&t, delay(&t));

    exec_self(&t, &to_b, 1).expect("the handover executes");
    exec_self(&t, &arm_x, 2).expect("residual: the leftover lands before acceptance");
    gov.accept_ownership();
    assert!(gov.has_role(&b, &sym(&t, "CANCELLER")));
    assert!(
        gov.has_role(&x, &sym(&t, "CANCELLER")),
        "the accomplice keeps its seat across the handover"
    );
}

// ---------------------------------------------------------------------------
// MC-3: a canceller reset voided by a handover
// ---------------------------------------------------------------------------

/// The previous owner's canceller reset is refused after the handover, and
/// because it can never execute any canceller may now cancel it, which frees
/// its salt. The new owner's own reset is the usual uncancellable operation.
#[test]
fn stale_recovery_reset_is_refused_and_cancellable_after_a_handover() {
    let t = LendingTest::new().build();
    let gov = t.gov_iface_client();
    let a = t.admin();
    let b = Address::generate(&t.env);
    let a2 = Address::generate(&t.env);
    let b2 = Address::generate(&t.env);
    let b3 = Address::generate(&t.env);
    let canceller = sym(&t, "CANCELLER");
    let a_council: Vec<Address> = vec![&t.env, a2.clone()];
    let b_council: Vec<Address> = vec![&t.env, b3.clone()];

    let reset_id = gov.propose_canceller_reset(&a_council, &salt(&t, 7));
    assert_contract_error(flatten(gov.try_cancel(&a, &reset_id)), NOT_CANCELLABLE);
    run_self_op(&t, &a, &transfer_to(&t, &b), 1);
    gov.accept_ownership();
    run_self_op(&t, &b, &grant(&t, &b2, "CANCELLER"), 2);
    assert!(gov.has_role(&b2, &canceller));

    advance(&t, RECOVERY);
    assert_eq!(op_state(&t, &reset_id), OperationState::Ready);
    assert_contract_error(exec_reset(&t, &a_council, 7), errors::NOT_AUTHORIZED);
    assert!(!gov.has_role(&a2, &canceller), "A's council never seats");
    assert!(gov.has_role(&b2, &canceller), "B's council keeps its seat");
    gov.cancel(&b2, &reset_id);
    assert_eq!(op_state(&t, &reset_id), OperationState::Unset);

    let again = gov.propose_canceller_reset(&b_council, &salt(&t, 7));
    assert_contract_error(flatten(gov.try_cancel(&b2, &again)), NOT_CANCELLABLE);
    advance(&t, RECOVERY);
    exec_reset(&t, &b_council, 7).expect("the new owner's reset executes");
    assert!(gov.has_role(&b3, &canceller));
    assert!(!gov.has_role(&b2, &canceller));
}

/// A departing owner queues several resets under different salts. All of
/// them die with the handover, each can be cancelled, and the new owner's own
/// reset stays uncancellable.
#[test]
fn staggered_reset_chain_dies_with_the_handover() {
    let t = LendingTest::new().build();
    let gov = t.gov_iface_client();
    let a = t.admin();
    let b = Address::generate(&t.env);
    let canceller = sym(&t, "CANCELLER");
    let councils: [Vec<Address>; 3] = [
        vec![&t.env, Address::generate(&t.env)],
        vec![&t.env, Address::generate(&t.env)],
        vec![&t.env, Address::generate(&t.env)],
    ];
    let mut ids = std::vec::Vec::new();
    for (i, council) in councils.iter().enumerate() {
        ids.push(gov.propose_canceller_reset(council, &salt(&t, 20 + i as u8)));
        advance(&t, 1_000);
    }
    run_self_op(&t, &a, &transfer_to(&t, &b), 1);
    gov.accept_ownership();
    let own = gov.propose_canceller_reset(&vec![&t.env, Address::generate(&t.env)], &salt(&t, 30));

    advance(&t, RECOVERY + 3_000);
    for (i, council) in councils.iter().enumerate() {
        assert_eq!(op_state(&t, &ids[i]), OperationState::Ready);
        assert_contract_error(
            exec_reset(&t, council, 20 + i as u8),
            errors::NOT_AUTHORIZED,
        );
        assert!(!gov.has_role(&council.get(0).unwrap(), &canceller));
        gov.cancel(&b, &ids[i]);
        assert_eq!(op_state(&t, &ids[i]), OperationState::Unset);
    }
    assert_contract_error(flatten(gov.try_cancel(&b, &own)), NOT_CANCELLABLE);
}

/// Abuse probe: a role holder cannot make a LIVE recovery reset, or its own
/// revocation, cancellable. A canceller that also holds GUARDIAN pauses and
/// writes flags, the oracle role writes a band: none of those counters binds
/// the reset or the revocation, so both stay uncancellable for it (46).
#[test]
fn other_epochs_cannot_unlock_a_live_reset_or_a_self_revocation() {
    let t = LendingTest::new().standard_two_asset().build();
    let gov = t.gov_iface_client();
    let a = t.admin();
    let c = Address::generate(&t.env);
    run_self_op(&t, &a, &grant(&t, &c, "CANCELLER"), 1);
    run_self_op(&t, &a, &grant(&t, &c, "GUARDIAN"), 2);
    let reset = gov.propose_canceller_reset(&vec![&t.env, Address::generate(&t.env)], &salt(&t, 3));
    let revoke = propose(
        &t,
        &a,
        &AdminOperation::RevokeGovRole(RoleArgs {
            account: c.clone(),
            role: sym(&t, "CANCELLER"),
        }),
        4,
    );

    gov.pause(&c);
    let usdc = key(&t, "USDC");
    let flags = listing(&t, HARNESS_SPOKE, &usdc);
    gov.set_spoke_asset_flags(
        &c,
        &HARNESS_SPOKE,
        &usdc,
        &flags.paused,
        &flags.frozen,
        &flags.no_seize,
    );
    let (min, max) = live_band(&t, "USDC");
    gov.set_sanity_band(&a, &price_key(&t, "USDC"), &min, &max);

    assert_contract_error(flatten(gov.try_cancel(&c, &revoke)), NOT_CANCELLABLE);
    assert_contract_error(flatten(gov.try_cancel(&c, &reset)), NOT_CANCELLABLE);
    assert_contract_error(flatten(gov.try_cancel(&a, &reset)), NOT_CANCELLABLE);
    assert_eq!(op_state(&t, &revoke), OperationState::Waiting);
    assert_eq!(op_state(&t, &reset), OperationState::Waiting);
}

// ---------------------------------------------------------------------------
// MC-4, Unpause arm: the emergency epoch
// ---------------------------------------------------------------------------

/// Same XDR as stellar-contract-utils' `PausableStorageKey`, which the
/// harness does not depend on.
#[contracttype]
enum PausableStorageKey {
    Paused,
}

fn paused(t: &LendingTest) -> bool {
    t.env.as_contract(&t.controller, || {
        t.env
            .storage()
            .instance()
            .get(&PausableStorageKey::Paused)
            .unwrap_or(false)
    })
}

/// An `Unpause` proposed while the controller is open is refused by the
/// controller (1001) and survives; a guardian pause moves the emergency
/// epoch, so the stale reopen is refused and the controller stays paused. A
/// reopen proposed after the pause serves its full delay and then executes.
#[test]
fn unpause_queued_while_open_dies_on_a_guardian_pause() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(ALICE, "USDC", 10_000.0);
    let d = delay(&t);
    let no_args = Vec::new(&t.env);

    let stale_id = propose(&t, &t.admin(), &AdminOperation::Unpause, 1);
    advance(&t, d);
    assert_contract_error(
        exec_raw(&t, &t.controller, "unpause", no_args.clone(), 1),
        EXPECTED_PAUSE,
    );
    assert_eq!(op_state(&t, &stale_id), OperationState::Ready);

    t.gov_iface_client().pause(&t.admin());
    assert_contract_error(t.try_supply(ALICE, "USDC", 100.0), errors::CONTRACT_PAUSED);
    assert_contract_error(
        exec_raw(&t, &t.controller, "unpause", no_args.clone(), 1),
        errors::EMERGENCY_EPOCH_MISMATCH,
    );
    assert_eq!(op_state(&t, &stale_id), OperationState::Ready);
    assert_contract_error(t.try_supply(ALICE, "USDC", 100.0), errors::CONTRACT_PAUSED);

    t.gov_iface_client().cancel(&t.admin(), &stale_id);
    propose(&t, &t.admin(), &AdminOperation::Unpause, 2);
    assert_contract_error(
        exec_raw(&t, &t.controller, "unpause", no_args.clone(), 2),
        errors::TIMELOCK_UNEXPECTED_STATE,
    );
    advance(&t, d);
    exec_raw(&t, &t.controller, "unpause", no_args, 2).expect("the post-pause reopen executes");
    t.try_supply(ALICE, "USDC", 100.0)
        .expect("controller reopened after a fresh delay");
}

/// Abuse probe: a guardian cannot void a reopen proposed DURING the pause. A
/// second `pause` while paused is refused by the controller (1000) and the
/// epoch advance reverts with it, so the legitimate `Unpause` executes.
#[test]
fn guardian_cannot_void_a_reopen_proposed_during_the_pause() {
    let mut t = LendingTest::new().standard_two_asset().build();
    let no_args = Vec::new(&t.env);
    t.gov_iface_client().pause(&t.admin());
    propose(&t, &t.admin(), &AdminOperation::Unpause, 1);

    assert_contract_error(
        flatten(t.gov_iface_client().try_pause(&t.admin())),
        errors::CONTRACT_PAUSED,
    );
    advance(&t, delay(&t));
    exec_raw(&t, &t.controller, "unpause", no_args, 1)
        .expect("a refused re-pause leaves the emergency epoch where it was");
    t.try_supply(ALICE, "USDC", 100.0)
        .expect("controller reopened");
}

/// Sibling: two `Unpause` proposals made during one incident. The first
/// reopens, the spare is refused by the controller (1001) and stays Ready;
/// the next guardian pause voids it, so it cannot reopen in that ledger.
#[test]
fn spare_unpause_from_one_incident_dies_on_the_next_pause() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(ALICE, "USDC", 10_000.0);
    let no_args = Vec::new(&t.env);
    t.gov_iface_client().pause(&t.admin());
    propose(&t, &t.admin(), &AdminOperation::Unpause, 1);
    let spare = propose(&t, &t.admin(), &AdminOperation::Unpause, 2);
    advance(&t, delay(&t));

    exec_raw(&t, &t.controller, "unpause", no_args.clone(), 1).expect("first reopen");
    assert_contract_error(
        exec_raw(&t, &t.controller, "unpause", no_args.clone(), 2),
        EXPECTED_PAUSE,
    );
    assert_eq!(op_state(&t, &spare), OperationState::Ready);

    t.gov_iface_client().pause(&t.admin());
    assert_contract_error(
        exec_raw(&t, &t.controller, "unpause", no_args, 2),
        errors::EMERGENCY_EPOCH_MISMATCH,
    );
    assert!(paused(&t));
    assert_contract_error(t.try_supply(ALICE, "USDC", 100.0), errors::CONTRACT_PAUSED);
}

/// Guard integrity: re-proposing a pending id cannot refresh its guard
/// (`OperationAlreadyScheduled`), and the guard check sits behind the grace
/// check, so an expired stale operation reports `TimelockOperationExpired`
/// (40) and a live one its guard error.
#[test]
fn reproposing_a_pending_id_cannot_refresh_its_guard() {
    let t = LendingTest::new().standard_two_asset().build();
    let no_args = Vec::new(&t.env);
    let id = propose(&t, &t.admin(), &AdminOperation::Unpause, 1);
    t.gov_iface_client().pause(&t.admin());

    assert_contract_error(
        flatten(t.gov_iface_client().try_propose(
            &t.admin(),
            &AdminOperation::Unpause,
            &salt(&t, 1),
        )),
        ALREADY_SCHEDULED,
    );
    advance(&t, delay(&t));
    assert_contract_error(
        exec_raw(&t, &t.controller, "unpause", no_args.clone(), 1),
        errors::EMERGENCY_EPOCH_MISMATCH,
    );
    advance(&t, GRACE + 1);
    assert_contract_error(exec_raw(&t, &t.controller, "unpause", no_args, 1), EXPIRED);
    assert_eq!(op_state(&t, &id), OperationState::Ready);
    assert!(paused(&t));
}

// ---------------------------------------------------------------------------
// Oracle MC-1: the band epoch (g7)
// ---------------------------------------------------------------------------

fn price_key(t: &LendingTest, asset: &str) -> PriceKey {
    PriceKey::Token(t.resolve_asset(asset))
}

fn live_band(t: &LendingTest, asset: &str) -> (i128, i128) {
    let oracle = t.market_oracle_config(&t.resolve_asset(asset));
    (oracle.min_sanity_price_wad, oracle.max_sanity_price_wad)
}

fn oracle_args(key: &PriceKey, base: &AssetOracle, band: (i128, i128)) -> ConfigureAssetOracleArgs {
    let mut oracle = base.clone();
    oracle.min_sanity_price_wad = band.0;
    oracle.max_sanity_price_wad = band.1;
    ConfigureAssetOracleArgs {
        key: key.clone(),
        oracle,
    }
}

fn exec_oracle(
    t: &LendingTest,
    args: &ConfigureAssetOracleArgs,
    byte: u8,
) -> Result<(), soroban_sdk::Error> {
    let resolved = t
        .gov_iface_client()
        .resolve_asset_oracle(&args.key, &args.oracle);
    exec_raw(
        t,
        &t.price_aggregator,
        "set_oracle",
        vec![
            &t.env,
            args.key.clone().into_val(&t.env),
            resolved.into_val(&t.env),
        ],
        byte,
    )
}

/// g7: a configuration voided by a narrowing never revives, even once a
/// later configuration restores the band it was proposed under (ABA).
#[test]
fn config_voided_by_a_narrowing_never_revives() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let gov = t.gov_iface_client();
    let admin = t.admin();
    let usdc = t.resolve_asset("USDC");
    let pk = PriceKey::Token(usdc.clone());
    let x = (usd(1) * 99 / 100, usd(1) * 101 / 100);
    let a_band = (usd(1) * 98 / 100, usd(1) * 102 / 100);
    let y = (usd(1) * 995 / 1000, usd(1) * 1005 / 1000);
    gov.set_sanity_band(&admin, &pk, &x.0, &x.1);
    assert_eq!(live_band(&t, "USDC"), x);
    let base = t.market_oracle_config(&usdc);

    let a = oracle_args(&pk, &base, a_band);
    propose(
        &t,
        &admin,
        &AdminOperation::ConfigureAssetOracle(a.clone()),
        0x61,
    );
    advance(&t, 1);
    gov.set_sanity_band(&admin, &pk, &y.0, &y.1);
    advance(&t, delay(&t));
    assert_contract_error(
        exec_oracle(&t, &a, 0x61),
        errors::ORACLE_BAND_CHANGED_AFTER_PROPOSAL,
    );

    let b = oracle_args(&pk, &base, x);
    propose(
        &t,
        &admin,
        &AdminOperation::ConfigureAssetOracle(b.clone()),
        0x62,
    );
    advance(&t, delay(&t));
    exec_oracle(&t, &b, 0x62).expect("B, proposed after the narrowing, executes");
    assert_eq!(live_band(&t, "USDC"), x);

    assert_contract_error(
        exec_oracle(&t, &a, 0x61),
        errors::ORACLE_BAND_CHANGED_AFTER_PROPOSAL,
    );
    assert_eq!(live_band(&t, "USDC"), x, "A never widened the band");
}

/// The live oracle config of `asset` with only the tolerance replaced, so a
/// proposal keeps the sources, read mode and band the market was listed with.
fn routine_config(t: &LendingTest, asset: &str) -> AssetOracle {
    let mut cfg = t.market_oracle_config(&t.resolve_asset(asset));
    cfg.tolerance = tolerance_band(&t.env, ROUTINE_TOLERANCE_BPS);
    cfg
}

/// Band wide enough that a later narrowing can exclude the live $1 price while
/// staying at least `MIN_SANITY_BAND_BPS` wide: [0.95, 1.05].
fn wide_band() -> (i128, i128) {
    (usd(1) * 95 / 100, usd(1) * 105 / 100)
}

/// Narrowing inside `wide_band` that excludes the live $1 price: [1.005, 1.05].
fn narrowed_band() -> (i128, i128) {
    (usd(1) * 1005 / 1000, usd(1) * 105 / 100)
}

fn set_oracle_args(t: &LendingTest, asset: &str, resolved: &AssetOracle) -> Vec<Val> {
    vec![
        &t.env,
        price_key(t, asset).into_val(&t.env),
        resolved.clone().into_val(&t.env),
    ]
}

/// Proposes `ConfigureAssetOracle` for `asset` as the owner and returns the
/// operation id and the resolved oracle the schedule hashed.
fn propose_oracle(
    t: &LendingTest,
    asset: &str,
    cfg: &AssetOracle,
    byte: u8,
) -> (BytesN<32>, AssetOracle) {
    let resolved = t
        .gov_iface_client()
        .resolve_asset_oracle(&price_key(t, asset), cfg);
    let id = propose(
        t,
        &t.admin(),
        &AdminOperation::ConfigureAssetOracle(ConfigureAssetOracleArgs {
            key: price_key(t, asset),
            oracle: cfg.clone(),
        }),
        byte,
    );
    (id, resolved)
}

/// Advances `n` ledgers, then re-pushes each market's current price: the mock
/// feeds live in temporary storage, which the test ledger expires across a
/// timelock delay. The push keeps the band untouched.
fn advance_with_live_feeds(t: &mut LendingTest, n: u32, markets: &[&str]) {
    advance(t, n);
    for name in markets {
        let price_wad = t.resolve_market(name).price_wad;
        t.set_price_keeping_sanity_band(name, price_wad);
    }
}

/// Widens the live USDC band to `wide_band` through the production timelock.
fn widen_usdc_band_through_timelock(t: &mut LendingTest, byte: u8) {
    let mut cfg = t.market_oracle_config(&t.resolve_asset("USDC"));
    cfg.min_sanity_price_wad = wide_band().0;
    cfg.max_sanity_price_wad = wide_band().1;
    let (_, resolved) = propose_oracle(t, "USDC", &cfg, byte);
    let d = delay(t);
    advance_with_live_feeds(t, d, &["USDC", "ETH"]);
    exec_raw(
        t,
        &t.price_aggregator,
        "set_oracle",
        set_oracle_args(t, "USDC", &resolved),
        byte,
    )
    .expect("the widening executes");
    assert_eq!(live_band(t, "USDC"), wide_band());
}

/// A Ready `ConfigureAssetOracle` for USDC is voided when the ORACLE role
/// narrows USDC's band to exclude the live price; the narrowing survives and
/// the controller keeps refusing the excluded price. The band epoch is per
/// key: a pending ETH configuration is untouched by the USDC narrowing.
#[test]
fn band_narrowing_voids_the_pending_config_for_that_key_only() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();
    widen_usdc_band_through_timelock(&mut t, 1);
    t.supply(ALICE, "USDC", 10_000.0);
    let d = delay(&t);
    let (usdc_id, usdc_resolved) = propose_oracle(&t, "USDC", &routine_config(&t, "USDC"), 2);
    let (_, eth_resolved) = propose_oracle(&t, "ETH", &routine_config(&t, "ETH"), 3);
    advance_with_live_feeds(&mut t, d, &["USDC", "ETH"]);
    assert_eq!(op_state(&t, &usdc_id), OperationState::Ready);

    let narrowed = narrowed_band();
    t.gov_iface_client().set_sanity_band(
        &t.admin(),
        &price_key(&t, "USDC"),
        &narrowed.0,
        &narrowed.1,
    );
    assert_eq!(live_band(&t, "USDC"), narrowed);
    assert_contract_error(
        t.try_borrow(ALICE, "ETH", 0.5),
        errors::SANITY_BOUND_VIOLATED,
    );

    assert_contract_error(
        exec_raw(
            &t,
            &t.price_aggregator,
            "set_oracle",
            set_oracle_args(&t, "USDC", &usdc_resolved),
            2,
        ),
        errors::ORACLE_BAND_CHANGED_AFTER_PROPOSAL,
    );
    assert_eq!(op_state(&t, &usdc_id), OperationState::Ready);
    assert_eq!(live_band(&t, "USDC"), narrowed, "the narrowing survives");

    exec_raw(
        &t,
        &t.price_aggregator,
        "set_oracle",
        set_oracle_args(&t, "ETH", &eth_resolved),
        3,
    )
    .expect("a pending config for another key executes");
    assert_eq!(
        t.market_oracle_config(&t.resolve_asset("ETH")),
        eth_resolved
    );
}

/// Sibling: `EditOracleTolerance` is not bound to the band epoch, and it
/// cannot undo a narrowing: a pending tolerance edit executes after it and
/// the narrowed band survives.
#[test]
fn pending_tolerance_edit_keeps_the_narrowing() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();
    widen_usdc_band_through_timelock(&mut t, 1);
    t.supply(ALICE, "USDC", 10_000.0);
    let d = delay(&t);
    let edit = AdminOperation::EditOracleTolerance(EditToleranceArgs {
        key: price_key(&t, "USDC"),
        tolerance: ROUTINE_TOLERANCE_BPS,
    });
    propose(&t, &t.admin(), &edit, 2);
    let tolerance = t
        .gov_iface_client()
        .resolve_oracle_tolerance(&ROUTINE_TOLERANCE_BPS);
    advance_with_live_feeds(&mut t, d, &["USDC", "ETH"]);

    let narrowed = narrowed_band();
    t.gov_iface_client().set_sanity_band(
        &t.admin(),
        &price_key(&t, "USDC"),
        &narrowed.0,
        &narrowed.1,
    );
    exec_raw(
        &t,
        &t.price_aggregator,
        "set_tolerance",
        vec![
            &t.env,
            price_key(&t, "USDC").into_val(&t.env),
            tolerance.clone().into_val(&t.env),
        ],
        2,
    )
    .expect("the unbound tolerance edit executes");
    assert_eq!(
        t.market_oracle_config(&t.resolve_asset("USDC")).tolerance,
        tolerance
    );
    assert_eq!(live_band(&t, "USDC"), narrowed, "the narrowing survives");
    assert_contract_error(
        t.try_borrow(ALICE, "ETH", 0.5),
        errors::SANITY_BOUND_VIOLATED,
    );
}

/// Abuse probe: the oracle role cannot move a key's band epoch with a
/// widening the aggregator refuses (`SanityBandMustTighten`); the advance
/// reverts with the call, so the pending configuration still executes.
#[test]
fn refused_widening_does_not_move_the_band_epoch() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();
    widen_usdc_band_through_timelock(&mut t, 1);
    let d = delay(&t);
    let (_, resolved) = propose_oracle(&t, "USDC", &routine_config(&t, "USDC"), 2);
    advance_with_live_feeds(&mut t, d, &["USDC", "ETH"]);

    assert_contract_error(
        flatten(t.gov_iface_client().try_set_sanity_band(
            &t.admin(),
            &price_key(&t, "USDC"),
            &(usd(1) * 90 / 100),
            &(usd(1) * 110 / 100),
        )),
        errors::SANITY_BAND_MUST_TIGHTEN,
    );
    exec_raw(
        &t,
        &t.price_aggregator,
        "set_oracle",
        set_oracle_args(&t, "USDC", &resolved),
        2,
    )
    .expect("a refused widening leaves the band epoch where it was");
}

/// Residual: an oracle-role band write that changes nothing still advances
/// the key's band epoch, so an ORACLE key can void every pending
/// `ConfigureAssetOracle` of a key, repeatedly. Bounded by
/// `revoke_role_immediate`.
#[test]
fn residual_noop_band_write_voids_the_pending_config() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();
    let d = delay(&t);
    let (_, resolved) = propose_oracle(&t, "USDC", &routine_config(&t, "USDC"), 2);
    advance_with_live_feeds(&mut t, d, &["USDC", "ETH"]);
    let band = live_band(&t, "USDC");
    t.gov_iface_client()
        .set_sanity_band(&t.admin(), &price_key(&t, "USDC"), &band.0, &band.1);
    assert_eq!(live_band(&t, "USDC"), band, "nothing changed");
    assert_contract_error(
        exec_raw(
            &t,
            &t.price_aggregator,
            "set_oracle",
            set_oracle_args(&t, "USDC", &resolved),
            2,
        ),
        errors::ORACLE_BAND_CHANGED_AFTER_PROPOSAL,
    );
}

/// A fresh price aggregator owned by governance, as a deployer would stand
/// up the target of an aggregator switch.
fn fresh_aggregator(t: &LendingTest) -> Address {
    t.env
        .register(price_aggregator::PriceAggregator, (t.governance.clone(),))
}

/// Review finding ORA-1: a `SetPriceAggregator` proposed before an `ORACLE`
/// band narrowing would move every key, the narrowed one included, onto the
/// new aggregator's bands and leave the narrowing behind on the old one. The
/// switch records the any-key band epoch, which every `set_sanity_band`
/// advances, so it is refused once a narrowing lands; a switch proposed after
/// the narrowing executes.
#[test]
fn aggregator_switch_proposed_before_a_narrowing_is_refused() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();
    widen_usdc_band_through_timelock(&mut t, 1);
    let admin = t.admin();
    let original = t.price_aggregator.clone();
    let switch = AdminOperation::SetPriceAggregator(fresh_aggregator(&t));
    let id = propose(&t, &admin, &switch, 0x71);
    advance(&t, 1);

    let narrowed = narrowed_band();
    t.gov_iface_client()
        .set_sanity_band(&admin, &price_key(&t, "USDC"), &narrowed.0, &narrowed.1);
    advance(&t, delay(&t));
    assert_eq!(op_state(&t, &id), OperationState::Ready);

    assert_contract_error(
        exec_self(&t, &switch, 0x71),
        errors::ORACLE_BAND_CHANGED_AFTER_PROPOSAL,
    );
    assert_eq!(op_state(&t, &id), OperationState::Ready);
    assert_eq!(t.gov_iface_client().price_aggregator(), original);
    assert_eq!(t.ctrl_client().price_aggregator(), original);
    assert_eq!(
        live_band(&t, "USDC"),
        narrowed,
        "the narrowing stays in force"
    );

    let reviewed = AdminOperation::SetPriceAggregator(original.clone());
    propose(&t, &admin, &reviewed, 0x72);
    advance(&t, delay(&t));
    exec_self(&t, &reviewed, 0x72).expect("a switch proposed after the narrowing executes");
    assert_eq!(t.ctrl_client().price_aggregator(), original);
}

/// The any-key band epoch advances only with a band write the aggregator
/// accepts: a refused widening reverts the advance, so a pending switch still
/// executes.
#[test]
fn refused_widening_keeps_a_pending_aggregator_switch() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();
    widen_usdc_band_through_timelock(&mut t, 1);
    let admin = t.admin();
    let original = t.price_aggregator.clone();
    let switch = AdminOperation::SetPriceAggregator(original.clone());
    propose(&t, &admin, &switch, 0x73);
    assert_contract_error(
        flatten(t.gov_iface_client().try_set_sanity_band(
            &admin,
            &price_key(&t, "USDC"),
            &(usd(1) * 90 / 100),
            &(usd(1) * 110 / 100),
        )),
        errors::SANITY_BAND_MUST_TIGHTEN,
    );
    advance(&t, delay(&t));

    exec_self(&t, &switch, 0x73).expect("a refused widening leaves the switch live");
    assert_eq!(t.ctrl_client().price_aggregator(), original);
}

// ---------------------------------------------------------------------------
// gov_b: ordering without a predecessor (G7), the controller-ownership cancel
// (G8, verify g6a/g6b, extra-3), proposer powers over liquidation terms with
// the listing-terms compare-and-swap (G10, verify g9a), and immediate
// containment of a stolen PROPOSER key (G11, verify g9b). Ported from the
// PR #196 verification and our `fix_gov_ordering_roles` suite, without its
// predecessor parameter and Done markers.
// ---------------------------------------------------------------------------

/// `stellar-access` role_transfer `InvalidLiveUntilLedger`.
const INVALID_LIVE_UNTIL: u32 = 2201;
/// `SpokeError::AssetAlreadyInSpoke`.
const ASSET_ALREADY_IN_SPOKE: u32 = 308;
const WAD: i128 = 1_000_000_000_000_000_000;

fn try_propose(
    t: &LendingTest,
    proposer: &Address,
    op: &AdminOperation,
    byte: u8,
) -> Result<(), soroban_sdk::Error> {
    flatten(
        t.gov_iface_client()
            .try_propose(proposer, op, &salt(t, byte)),
    )
}

fn revoke(t: &LendingTest, who: &Address, role: &str) -> AdminOperation {
    AdminOperation::RevokeGovRole(RoleArgs {
        account: who.clone(),
        role: sym(t, role),
    })
}

fn limits_op(supply: u32) -> AdminOperation {
    AdminOperation::SetPositionLimits(PositionLimits {
        max_supply_positions: supply,
        max_borrow_positions: 3,
    })
}

fn exec_limits(t: &LendingTest, supply: u32, byte: u8) -> Result<(), soroban_sdk::Error> {
    let limits = PositionLimits {
        max_supply_positions: supply,
        max_borrow_positions: 3,
    };
    exec_raw(
        t,
        &t.controller,
        "set_position_limits",
        vec![&t.env, limits.into_val(&t.env)],
        byte,
    )
}

/// Uploads `target/wasm32v1-none/release/<file>` with a trailing custom
/// section carrying `tag`, so the code is the same and the hash differs.
fn upload_wasm_variant(t: &LendingTest, file: &str, tag: u8) -> BytesN<32> {
    let path = std::format!("target/wasm32v1-none/release/{file}");
    let mut bytes = std::fs::read(&path)
        .or_else(|_| std::fs::read(std::format!("../../{path}")))
        .expect("WASM not found. Run 'make build' first.");
    // Custom section: id 0, size 4, name "v2" (length 2), payload [tag].
    bytes.extend_from_slice(&[0x00, 0x04, 0x02, b'v', b'2', tag]);
    t.env
        .deployer()
        .upload_contract_wasm(Bytes::from_slice(&t.env, &bytes))
}

fn hash_call(t: &LendingTest, hash: &BytesN<32>) -> Vec<Val> {
    vec![&t.env, hash.into_val(&t.env)]
}

/// G7: upgrade epochs are per contract. Two pool builds and a position-NFT
/// build are queued together; the NFT upgrade does not void the pool
/// upgrades, the newer pool build lands, and the superseded pool build is
/// refused with `UpgradeSupersededAfterProposal` (62).
#[test]
fn upgrade_epochs_are_per_contract() {
    let t = LendingTest::new().build();
    let pool_a = upload_wasm_variant(&t, "pool.wasm", 1);
    let pool_b = upload_wasm_variant(&t, "pool.wasm", 2);
    let nft = upload_wasm_variant(&t, "position_nft.wasm", 3);
    propose(
        &t,
        &t.admin(),
        &AdminOperation::UpgradePool(pool_a.clone()),
        1,
    );
    propose(
        &t,
        &t.admin(),
        &AdminOperation::UpgradePool(pool_b.clone()),
        2,
    );
    propose(
        &t,
        &t.admin(),
        &AdminOperation::UpgradePositionNft(nft.clone()),
        3,
    );
    advance(&t, delay(&t));

    exec_raw(
        &t,
        &t.controller,
        "upgrade_position_nft",
        hash_call(&t, &nft),
        3,
    )
    .expect("the NFT upgrade lands");
    exec_raw(&t, &t.controller, "upgrade_pool", hash_call(&t, &pool_b), 2)
        .expect("another contract's upgrade does not void a pool upgrade");
    assert_contract_error(
        exec_raw(&t, &t.controller, "upgrade_pool", hash_call(&t, &pool_a), 1),
        errors::UPGRADE_SUPERSEDED_AFTER_PROPOSAL,
    );
}

fn ctrl_transfer_call(t: &LendingTest, x: &Address, live_until: u32) -> Vec<Val> {
    vec![
        &t.env,
        x.clone().into_val(&t.env),
        live_until.into_val(&t.env),
    ]
}

fn ctrl_transfer(x: &Address, live_until_ledger: u32) -> AdminOperation {
    AdminOperation::TransferCtrlOwnership(TransferOwnershipArgs {
        new_owner: x.clone(),
        live_until_ledger,
    })
}

/// g6a/g6b body. `run_first`: the reverted shape (the cancel is executed once
/// with nothing pending), otherwise the idle shape (it sits Ready). Either
/// way the stale `TransferCtrlOwnership{X, 0}` cannot void a later
/// nomination of X.
fn controller_cancel_cannot_void_a_later_nomination(run_first: bool) {
    let t = LendingTest::new().build();
    let d = delay(&t).max(12);
    // `TransferCtrlOwnership` needs a deployed contract as nominee.
    let x = t.position_nft.clone();
    let stale_id = propose(&t, &t.admin(), &ctrl_transfer(&x, 0), 1);
    advance(&t, d);
    if run_first {
        // CR-1 controller leg (gov_b G8): was a 2200 revert that left the
        // cancel Ready; the idempotent zero-deadline arm consumes it.
        exec_raw(
            &t,
            &t.controller,
            "transfer_ownership",
            ctrl_transfer_call(&t, &x, 0),
            1,
        )
        .expect("a cancel with nothing pending is a consumed no-op");
        assert_eq!(op_state(&t, &stale_id), OperationState::Unset);
    }

    let live_until = t.env.ledger().sequence() + 10_000;
    propose(&t, &t.admin(), &ctrl_transfer(&x, live_until), 2);
    advance(&t, d);
    exec_raw(
        &t,
        &t.controller,
        "transfer_ownership",
        ctrl_transfer_call(&t, &x, live_until),
        2,
    )
    .expect("the controller nomination of X executes");

    let stale = exec_raw(
        &t,
        &t.controller,
        "transfer_ownership",
        ctrl_transfer_call(&t, &x, 0),
        1,
    );
    if run_first {
        assert_contract_error(stale, errors::TIMELOCK_UNEXPECTED_STATE);
    } else {
        stale.expect("the idle stale cancel completes as a no-op");
        assert_eq!(op_state(&t, &stale_id), OperationState::Unset);
    }
    flatten(t.ctrl_client().try_accept_ownership()).expect("X's nomination stands");
}

#[test]
fn reverted_controller_cancel_cannot_void_a_later_nomination() {
    controller_cancel_cannot_void_a_later_nomination(true);
}

#[test]
fn idle_controller_cancel_cannot_void_a_later_nomination() {
    controller_cancel_cannot_void_a_later_nomination(false);
}

/// A controller cancel proposed after the nomination it targets executed
/// still cancels it: the nominee has nothing to accept.
#[test]
fn controller_cancel_proposed_after_the_nomination_cancels_it() {
    let t = LendingTest::new().build();
    let d = delay(&t).max(12);
    let x = t.position_nft.clone();
    let live_until = t.env.ledger().sequence() + 10_000;
    propose(&t, &t.admin(), &ctrl_transfer(&x, live_until), 1);
    advance(&t, d);
    exec_raw(
        &t,
        &t.controller,
        "transfer_ownership",
        ctrl_transfer_call(&t, &x, live_until),
        1,
    )
    .expect("nomination");
    propose(&t, &t.admin(), &ctrl_transfer(&x, 0), 2);
    advance(&t, d);
    exec_raw(
        &t,
        &t.controller,
        "transfer_ownership",
        ctrl_transfer_call(&t, &x, 0),
        2,
    )
    .expect("the cancel executes");

    assert_contract_error(
        flatten(t.ctrl_client().try_accept_ownership()),
        errors::NO_PENDING_TRANSFER,
    );
}

/// extra-3, documented residual on both legs: a nomination cancel proposed
/// before the nomination it means to stop has executed is bound to the
/// earlier nomination epoch, so it completes as a no-op and the nominee can
/// still accept. A queued nomination is stopped by cancelling its operation
/// id; a cancel to revoke an executed nomination is proposed after it.
#[test]
fn residual_cancel_proposed_before_its_nomination_executes_is_a_no_op() {
    let t = LendingTest::new().build();
    let gov = t.gov_iface_client();
    let d = delay(&t).max(12);
    let x = Address::generate(&t.env);
    let nominate = transfer_to(&t, &x);
    propose(&t, &t.admin(), &nominate, 1);
    let cancel = AdminOperation::TransferGovOwnership(TransferOwnershipArgs {
        new_owner: x.clone(),
        live_until_ledger: 0,
    });
    let cancel_id = propose(&t, &t.admin(), &cancel, 2);
    let nft = t.position_nft.clone();
    let live_until = t.env.ledger().sequence() + 10_000;
    propose(&t, &t.admin(), &ctrl_transfer(&nft, live_until), 3);
    propose(&t, &t.admin(), &ctrl_transfer(&nft, 0), 4);
    advance(&t, d);

    exec_self(&t, &nominate, 1).expect("X is nominated");
    exec_self(&t, &cancel, 2).expect("the early cancel completes");
    assert_eq!(op_state(&t, &cancel_id), OperationState::Unset);
    exec_raw(
        &t,
        &t.controller,
        "transfer_ownership",
        ctrl_transfer_call(&t, &nft, live_until),
        3,
    )
    .expect("controller nomination");
    exec_raw(
        &t,
        &t.controller,
        "transfer_ownership",
        ctrl_transfer_call(&t, &nft, 0),
        4,
    )
    .expect("the early controller cancel completes");

    gov.accept_ownership();
    assert!(gov.has_role(&x, &sym(&t, "PROPOSER")), "X is the owner");
    flatten(t.ctrl_client().try_accept_ownership()).expect("the controller nominee accepts too");
}

/// CR-1 residual, the wider class: a nomination whose deadline is beyond the
/// network's maximum TTL at its ready ledger reverts (2201) and stays Ready,
/// then lands later in its grace window, executed by anyone. Low impact (the
/// owner chose that nominee); only the zero-deadline arm is idempotent.
#[test]
fn residual_reverted_nomination_stays_ready_and_lands_late() {
    let t = LendingTest::new().build();
    let gov = t.gov_iface_client();
    let d = delay(&t).max(12);
    let x = Address::generate(&t.env);
    let max_ttl = t.env.ledger().get().max_entry_ttl;
    let ready = t.env.ledger().sequence() + d;
    let nominate = AdminOperation::TransferGovOwnership(TransferOwnershipArgs {
        new_owner: x.clone(),
        live_until_ledger: ready + max_ttl + 1_000,
    });
    let id = propose(&t, &t.admin(), &nominate, 1);
    advance(&t, d);
    assert_contract_error(exec_self(&t, &nominate, 1), INVALID_LIVE_UNTIL);
    assert_eq!(op_state(&t, &id), OperationState::Ready);
    advance(&t, 2_000);
    exec_self(&t, &nominate, 1).expect("the reverted nomination lands late");
    gov.accept_ownership();
    assert!(gov.has_role(&x, &sym(&t, "PROPOSER")), "X is the owner");
}

/// g9a (G10): a non-owner proposer cannot schedule liquidation terms: a
/// one-point threshold cut, bonus raise or fee cut of a live listing, the
/// spoke liquidation curve, forced socialization and position-manager
/// activation are refused at proposal (44). The owner schedules each of them,
/// and routine listing power stays with the proposer: an edit that keeps the
/// terms, a cap tightening, a fee raise, a manager deactivation and a spoke
/// removal schedule.
#[test]
fn non_owner_proposer_cannot_schedule_liquidation_terms() {
    let t = LendingTest::new().standard_two_asset().build();
    let p = grant_fresh(&t, "PROPOSER");
    let manager = Address::generate(&t.env);
    let live = eth_live_args(&t);
    let mut cut = live.clone();
    cut.threshold -= 1;
    let mut sweeter = live.clone();
    sweeter.bonus += 1;
    let mut cheaper = live.clone();
    cheaper.liquidation_fees -= 1;
    let owner_only = [
        AdminOperation::EditAssetInSpoke(cut),
        AdminOperation::EditAssetInSpoke(sweeter),
        AdminOperation::EditAssetInSpoke(cheaper),
        AdminOperation::SetSpokeLiquidationCurve(SpokeLiquidationCurveArgs {
            spoke_id: HARNESS_SPOKE,
            target_hf_wad: 10 * WAD,
            hf_for_max_bonus_wad: 10 * WAD - 1,
            liquidation_bonus_factor_bps: 10_000,
        }),
        AdminOperation::ForceSocializeBadDebt(1),
        AdminOperation::SetPositionManager(manager.clone(), true),
    ];
    for (i, op) in owner_only.iter().enumerate() {
        let byte = 10 + i as u8;
        assert_contract_error(try_propose(&t, &p, op, byte), errors::NOT_AUTHORIZED);
        assert_eq!(
            op_state(&t, &propose(&t, &t.admin(), op, byte)),
            OperationState::Waiting,
            "the owner schedules it"
        );
    }

    let mut tighter = live.clone();
    tighter.supply_cap = live.supply_cap / 2;
    let mut dearer = live.clone();
    dearer.liquidation_fees += 1;
    let routine = [
        AdminOperation::EditAssetInSpoke(live),
        AdminOperation::EditAssetInSpoke(tighter),
        AdminOperation::EditAssetInSpoke(dearer),
        AdminOperation::SetPositionManager(manager, false),
        AdminOperation::RemoveSpoke(HARNESS_SPOKE),
    ];
    for (i, op) in routine.iter().enumerate() {
        assert_eq!(
            op_state(&t, &propose(&t, &p, op, 20 + i as u8)),
            OperationState::Waiting,
            "routine listing power stays with the proposer"
        );
    }
}

fn edit_call(t: &LendingTest, args: &SpokeAssetArgs) -> Vec<Val> {
    vec![&t.env, args.clone().into_val(&t.env)]
}

/// G10 compare-and-swap: a non-owner proposer queues an edit that keeps the
/// live terms and one that raises the threshold. Executed raise-first, the
/// kept edit would walk the live threshold back and is refused with
/// `ListingTermsChangedAfterProposal` (63); it stays Ready, the raise stands
/// and a canceller clears it. A direct cut is still refused at proposal.
#[test]
fn proposer_self_ratchet_is_refused_at_execution() {
    let t = LendingTest::new().standard_two_asset().build();
    let p = grant_fresh(&t, "PROPOSER");
    let start = eth_live_args(&t);
    let eth = key(&t, "ETH");
    let mut raise = start.clone();
    raise.threshold += 200;
    let kept_id = propose(&t, &p, &AdminOperation::EditAssetInSpoke(start.clone()), 1);
    propose(&t, &p, &AdminOperation::EditAssetInSpoke(raise.clone()), 2);
    advance(&t, delay(&t));
    exec_raw(
        &t,
        &t.controller,
        "edit_asset_in_spoke",
        edit_call(&t, &raise),
        2,
    )
    .expect("raise");
    assert_eq!(
        listing(&t, HARNESS_SPOKE, &eth).liquidation_threshold,
        start.threshold + 200
    );

    let mut cut_now = raise.clone();
    cut_now.threshold = start.threshold + 100;
    assert_contract_error(
        try_propose(&t, &p, &AdminOperation::EditAssetInSpoke(cut_now), 3),
        errors::NOT_AUTHORIZED,
    );
    assert_contract_error(
        exec_raw(
            &t,
            &t.controller,
            "edit_asset_in_spoke",
            edit_call(&t, &start),
            1,
        ),
        errors::LISTING_TERMS_CHANGED_AFTER_PROPOSAL,
    );
    assert_eq!(
        listing(&t, HARNESS_SPOKE, &eth).liquidation_threshold,
        start.threshold + 200,
        "the stale kept edit cannot walk the live threshold back"
    );
    assert_eq!(op_state(&t, &kept_id), OperationState::Ready);
    t.gov_iface_client().cancel(&t.admin(), &kept_id);
    assert_eq!(op_state(&t, &kept_id), OperationState::Unset);
}

/// G10 compare-and-swap, the owner's changes: a proposer's routine edit
/// (a tighter cap) queued before an owner threshold raise would put the old
/// threshold back and is refused (63); a fresh edit proposed against the
/// raised terms executes; an owner fee change voids a pending kept-fee edit
/// the same way. The owner's own edits are not bound: an owner edit proposed
/// before a proposer's raise still executes after it.
#[test]
fn stale_kept_edit_cannot_undo_an_owner_change() {
    let t = LendingTest::new().standard_two_asset().build();
    let p = grant_fresh(&t, "PROPOSER");
    let start = eth_live_args(&t);
    let eth = key(&t, "ETH");

    let mut tighter = start.clone();
    tighter.supply_cap = start.supply_cap / 2;
    let stale_id = propose(
        &t,
        &p,
        &AdminOperation::EditAssetInSpoke(tighter.clone()),
        1,
    );
    let mut owner_raise = start.clone();
    owner_raise.threshold += 100;
    propose(
        &t,
        &t.admin(),
        &AdminOperation::EditAssetInSpoke(owner_raise.clone()),
        2,
    );
    advance(&t, delay(&t));
    exec_raw(
        &t,
        &t.controller,
        "edit_asset_in_spoke",
        edit_call(&t, &owner_raise),
        2,
    )
    .expect("the owner's raise");
    assert_contract_error(
        exec_raw(
            &t,
            &t.controller,
            "edit_asset_in_spoke",
            edit_call(&t, &tighter),
            1,
        ),
        errors::LISTING_TERMS_CHANGED_AFTER_PROPOSAL,
    );
    let live = listing(&t, HARNESS_SPOKE, &eth);
    assert_eq!(live.liquidation_threshold, start.threshold + 100);
    assert_eq!(live.supply_cap, start.supply_cap);
    assert_eq!(op_state(&t, &stale_id), OperationState::Ready);

    let mut fresh = owner_raise.clone();
    fresh.supply_cap = start.supply_cap / 2;
    propose(&t, &p, &AdminOperation::EditAssetInSpoke(fresh.clone()), 3);
    advance(&t, delay(&t));
    exec_raw(
        &t,
        &t.controller,
        "edit_asset_in_spoke",
        edit_call(&t, &fresh),
        3,
    )
    .expect("an edit proposed against the live terms executes");
    let live = listing(&t, HARNESS_SPOKE, &eth);
    assert_eq!(live.liquidation_threshold, start.threshold + 100);
    assert_eq!(live.supply_cap, start.supply_cap / 2);

    // Fees bind the same way.
    let kept_fee_id = propose(&t, &p, &AdminOperation::EditAssetInSpoke(fresh.clone()), 4);
    let mut owner_fee = fresh.clone();
    owner_fee.liquidation_fees += 100;
    propose(
        &t,
        &t.admin(),
        &AdminOperation::EditAssetInSpoke(owner_fee.clone()),
        5,
    );
    advance(&t, delay(&t));
    exec_raw(
        &t,
        &t.controller,
        "edit_asset_in_spoke",
        edit_call(&t, &owner_fee),
        5,
    )
    .expect("the owner's fee change");
    assert_contract_error(
        exec_raw(
            &t,
            &t.controller,
            "edit_asset_in_spoke",
            edit_call(&t, &fresh),
            4,
        ),
        errors::LISTING_TERMS_CHANGED_AFTER_PROPOSAL,
    );
    assert_eq!(op_state(&t, &kept_fee_id), OperationState::Ready);

    // The owner's edits are not bound to the terms they were proposed against.
    propose(
        &t,
        &t.admin(),
        &AdminOperation::EditAssetInSpoke(owner_fee.clone()),
        6,
    );
    let mut proposer_raise = owner_fee.clone();
    proposer_raise.threshold += 50;
    propose(
        &t,
        &p,
        &AdminOperation::EditAssetInSpoke(proposer_raise.clone()),
        7,
    );
    advance(&t, delay(&t));
    exec_raw(
        &t,
        &t.controller,
        "edit_asset_in_spoke",
        edit_call(&t, &proposer_raise),
        7,
    )
    .expect("the proposer's raise keeps the terms it was proposed against");
    exec_raw(
        &t,
        &t.controller,
        "edit_asset_in_spoke",
        edit_call(&t, &owner_fee),
        6,
    )
    .expect("the owner's edit is not bound");
    assert_eq!(
        listing(&t, HARNESS_SPOKE, &eth).liquidation_threshold,
        start.threshold + 100
    );
}

/// G10 through the listing arms: `AddAssetToSpoke` with worse terms cannot
/// overwrite a live listing (308 at execution), and the remove-then-relist
/// route is closed while the listing has users.
#[test]
fn add_and_relist_arms_cannot_overwrite_a_live_listing() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(ALICE, "ETH", 1.0);
    let p = grant_fresh(&t, "PROPOSER");
    let start = eth_live_args(&t);
    let mut worse = start.clone();
    worse.ltv = 3100;
    worse.threshold = 3200;
    worse.bonus = 21_250;
    worse.liquidation_fees = 0;
    propose(&t, &p, &AdminOperation::AddAssetToSpoke(worse.clone()), 1);
    let eth = key(&t, "ETH");
    propose(
        &t,
        &p,
        &AdminOperation::RemoveAssetFromSpoke(RemoveAssetFromSpokeArgs {
            hub_asset: eth.clone(),
            spoke_id: HARNESS_SPOKE,
        }),
        2,
    );
    advance(&t, delay(&t));
    t.refresh_oracle_prices();

    assert_contract_error(exec_add(&t, &worse, 1), ASSET_ALREADY_IN_SPOKE);
    assert_contract_error(
        exec_raw(
            &t,
            &t.controller,
            "remove_asset_from_spoke",
            vec![
                &t.env,
                eth.clone().into_val(&t.env),
                HARNESS_SPOKE.into_val(&t.env),
            ],
            2,
        ),
        errors::SPOKE_ASSET_IN_USE,
    );
    let live = listing(&t, HARNESS_SPOKE, &eth);
    assert_eq!(live.liquidation_threshold, start.threshold);
    assert_eq!(live.liquidation_bonus, start.bonus);
}

/// g9b (G11): the owner contains a stolen PROPOSER key and its queue with one
/// immediate call; the key's queued operation is refused (44) once the delay
/// passes.
#[test]
fn owner_contains_a_stolen_proposer_key_and_its_queue_immediately() {
    let t = LendingTest::new().standard_two_asset().build();
    let gov = t.gov_iface_client();
    let p = grant_fresh(&t, "PROPOSER");
    let queued = propose(&t, &p, &limits_op(4), 11);

    flatten(gov.try_revoke_role_immediate(&p, &sym(&t, "PROPOSER")))
        .expect("PROPOSER is revocable immediately");
    assert!(!gov.has_role(&p, &sym(&t, "PROPOSER")));
    assert_contract_error(try_propose(&t, &p, &limits_op(2), 12), errors::UNAUTHORIZED);
    advance(&t, delay(&t));
    assert_contract_error(exec_limits(&t, 4, 11), errors::NOT_AUTHORIZED);
    assert_eq!(op_state(&t, &queued), OperationState::Ready);
    gov.cancel(&t.admin(), &queued);
    assert_eq!(op_state(&t, &queued), OperationState::Unset);
}

/// G11: the revoked key's queue stays dead on the raw and the self path, for
/// a named EXECUTOR, and after the key is granted PROPOSER again; only
/// proposals made after the re-grant execute. The owner's own queue is
/// untouched, an EXECUTOR key is revocable immediately, CANCELLER is not.
#[test]
fn revoked_proposer_queue_stays_dead_on_every_path_and_after_regrant() {
    let t = LendingTest::new().build();
    let gov = t.gov_iface_client();
    let mallory = grant_fresh(&t, "PROPOSER");
    let executor = grant_fresh(&t, "EXECUTOR");
    let guardian = grant_fresh(&t, "GUARDIAN");
    let strip_guardian = revoke(&t, &guardian, "GUARDIAN");

    propose(&t, &t.admin(), &limits_op(4), 1);
    propose(&t, &mallory, &limits_op(1), 2);
    propose(&t, &mallory, &strip_guardian, 3);

    gov.revoke_role_immediate(&mallory, &sym(&t, "PROPOSER"));
    let regrant = grant(&t, &mallory, "PROPOSER");
    run_self_op(&t, &t.admin(), &regrant, 5);
    assert!(gov.has_role(&mallory, &sym(&t, "PROPOSER")));

    exec_limits(&t, 4, 1).expect("the owner's queue is untouched");
    let limits = PositionLimits {
        max_supply_positions: 1,
        max_borrow_positions: 3,
    };
    assert_contract_error(
        flatten(gov.try_execute(
            &Some(executor.clone()),
            &t.controller,
            &sym(&t, "set_position_limits"),
            &vec![&t.env, limits.into_val(&t.env)],
            &salt(&t, 0),
            &salt(&t, 2),
        )),
        errors::NOT_AUTHORIZED,
    );
    assert_contract_error(
        flatten(gov.try_execute_self(&Some(executor.clone()), &strip_guardian, &salt(&t, 3))),
        errors::NOT_AUTHORIZED,
    );
    assert!(gov.has_role(&guardian, &sym(&t, "GUARDIAN")));

    propose(&t, &mallory, &limits_op(3), 6);
    advance(&t, delay(&t));
    exec_limits(&t, 3, 6).expect("a proposal made after the re-grant executes");

    gov.revoke_role_immediate(&executor, &sym(&t, "EXECUTOR"));
    assert!(!gov.has_role(&executor, &sym(&t, "EXECUTOR")));
    let canceller = grant_fresh(&t, "CANCELLER");
    assert_contract_error(
        flatten(gov.try_revoke_role_immediate(&canceller, &sym(&t, "CANCELLER"))),
        errors::INVALID_ROLE,
    );
}

/// G11 economics: a peer revocation voids the peer's queue. A stolen
/// proposer key schedules `RevokeGovRole(peer, PROPOSER)`; the peer cannot
/// cancel it (46, it is the target), and once it executes the honest
/// operation the peer had queued is refused (44). Bounded by the other
/// cancellers' veto during the Sensitive delay and by the owner's immediate
/// revocation of the hostile key.
#[test]
fn peer_revocation_voids_the_peer_queue() {
    let t = LendingTest::new().build();
    let gov = t.gov_iface_client();
    let mallory = grant_fresh(&t, "PROPOSER");
    let peer = grant_fresh(&t, "PROPOSER");
    t.gov_client()
        .execute_immediate(&t.admin(), &grant(&t, &peer, "CANCELLER"));

    let honest = propose(&t, &peer, &limits_op(4), 1);
    let hostile = revoke(&t, &peer, "PROPOSER");
    let hostile_id = propose(&t, &mallory, &hostile, 2);
    assert_contract_error(flatten(gov.try_cancel(&peer, &hostile_id)), NOT_CANCELLABLE);
    advance(&t, delay(&t).max(12));
    exec_self(&t, &hostile, 2).expect("nobody else vetoed the revocation");
    assert!(!gov.has_role(&peer, &sym(&t, "PROPOSER")));
    assert_eq!(op_state(&t, &honest), OperationState::Ready);
    assert_contract_error(exec_limits(&t, 4, 1), errors::NOT_AUTHORIZED);
}
