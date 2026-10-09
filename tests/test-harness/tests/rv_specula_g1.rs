//! Second opinion on Specula governance MC-1: a `RemoveAssetFromSpoke` and an
//! `AddAssetToSpoke` queued before a guardian freeze still execute after it,
//! by anyone, and the re-add writes the queued flags without the ratchet.
//! `config::asset::upsert_spoke_asset` calls `require_flag_ratchet` on the
//! Edit arm only; the Add arm has no stored config to ratchet against because
//! `remove_asset_from_spoke` deleted it and kept only the flags epoch.
//!
//! Every step uses production entry points: governance `propose` as the
//! PROPOSER, the guardian's immediate `set_spoke_asset_flags`, permissionless
//! `execute` with no executor, and the controller's `supply` for the lending
//! effect. The harness owner holds every role, so one address plays the
//! proposer and the guardian; the roles are separate on mainnet.
//!
//! The first test pins the HEAD behaviour and names the assertion that must
//! flip once INV-AUTH-04 covers re-listing. The second pins the gate that
//! bounds the finding: a listing with live usage cannot be removed, so the
//! pair cannot clear its freeze until the listing is empty.

use common::errors::SpokeError;
use common::types::{HubAssetKey, SpokeAssetArgs, SpokeAssetConfig};
use governance_interface::{
    AdminOperation, OperationState, RelaxSpokeAssetFlagsArgs, RemoveAssetFromSpokeArgs,
};
use soroban_sdk::testutils::Ledger as _;
use soroban_sdk::{vec, BytesN, IntoVal, Symbol, Val, Vec};
use test_harness::{
    assert_contract_error, errors, hub_asset, LendingTest, ALICE, HARNESS_HUB, STABLECOIN_SPOKE,
};

/// Second spoke of the fixture; the attacked listing lives here and is empty.
const SPOKE_B: u32 = 2;
const KATE: &str = "kate";
const LIAM: &str = "liam";
const ASSET_ALREADY_IN_SPOKE: u32 = SpokeError::AssetAlreadyInSpoke as u32;

fn salt(t: &LendingTest, byte: u8) -> BytesN<32> {
    BytesN::<32>::from_array(&t.env, &[byte; 32])
}

fn key(t: &LendingTest, asset: &str) -> HubAssetKey {
    hub_asset(t.resolve_asset(asset))
}

fn listing(t: &LendingTest, spoke: u32, asset: &str) -> SpokeAssetConfig {
    t.ctrl_client().get_spoke_asset(&spoke, &key(t, asset))
}

/// `get_spoke_asset` as a contract-error result; the listing is absent when
/// the controller refuses with `AssetNotInSpoke`.
fn try_listing(t: &LendingTest, spoke: u32, asset: &str) -> Result<(), soroban_sdk::Error> {
    flatten(t.ctrl_client().try_get_spoke_asset(&spoke, &key(t, asset)))
}

fn op_state(t: &LendingTest, id: &BytesN<32>) -> OperationState {
    t.gov_iface_client().get_operation_state(id)
}

fn propose(t: &LendingTest, op: &AdminOperation, byte: u8) -> BytesN<32> {
    t.gov_iface_client().propose(&t.admin(), op, &salt(t, byte))
}

fn epoch(t: &LendingTest, spoke: u32, asset: &str) -> u64 {
    t.ctrl_client()
        .get_spoke_asset_flags_epoch(&spoke, &key(t, asset))
}

/// The live listing re-encoded as `AddAssetToSpoke` arguments with every halt
/// flag clear: what an operator would queue to re-list the asset as it was.
fn relist_args(t: &LendingTest, spoke: u32, asset: &str) -> SpokeAssetArgs {
    let cfg = listing(t, spoke, asset);
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

fn advance(t: &LendingTest, n: u32) {
    t.env.ledger().with_mut(|l| l.sequence_number += n);
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

/// Permissionless execution of a ready controller operation: no executor, so
/// no signature and no role. The tuple must match what `propose` hashed.
fn exec_raw(
    t: &LendingTest,
    function: &str,
    args: Vec<Val>,
    s: BytesN<32>,
) -> Result<(), soroban_sdk::Error> {
    flatten(t.gov_iface_client().try_execute(
        &None,
        &t.controller,
        &Symbol::new(&t.env, function),
        &args,
        &salt(t, 0),
        &s,
    ))
}

/// `op::resolve_op` encoding of `RemoveAssetFromSpoke`: hub asset, then spoke id.
fn remove_call(t: &LendingTest, asset: &str, spoke: u32) -> Vec<Val> {
    vec![
        &t.env,
        key(t, asset).into_val(&t.env),
        spoke.into_val(&t.env),
    ]
}

/// `op::resolve_op` encoding of `AddAssetToSpoke`: the whole args struct.
fn add_call(t: &LendingTest, args: &SpokeAssetArgs) -> Vec<Val> {
    vec![&t.env, args.clone().into_val(&t.env)]
}

/// `op::resolve_op` encoding of `RelaxSpokeAssetFlags`.
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

fn remove_op(t: &LendingTest, asset: &str, spoke: u32) -> AdminOperation {
    AdminOperation::RemoveAssetFromSpoke(RemoveAssetFromSpokeArgs {
        hub_asset: key(t, asset),
        spoke_id: spoke,
    })
}

/// USDC listed in two spokes. The harness spoke carries live supply, so the
/// asset is in use protocol-wide; only the spoke-B listing is empty, which is
/// the removal gate `remove_asset_from_spoke` reads.
fn fixture() -> LendingTest {
    let mut t = LendingTest::new()
        .standard_two_asset()
        .with_spoke(SPOKE_B, STABLECOIN_SPOKE)
        .with_spoke_asset(SPOKE_B, "USDC", true, true)
        .build();
    t.supply(ALICE, "USDC", 10_000.0);
    t
}

/// Guardian freeze on spoke B's USDC through governance's immediate path.
fn guardian_freeze(t: &LendingTest) {
    t.gov_iface_client().set_spoke_asset_flags(
        &t.admin(),
        &SPOKE_B,
        &key(t, "USDC"),
        &false,
        &true,
        &false,
    );
}

/// MC-1 at HEAD. A remove and a clean re-add queued before a guardian freeze
/// both execute after it with no executor. The removal keeps the flags epoch,
/// the re-add bumps it and writes `frozen = false`, and supply into the
/// listing is accepted again. The control shows what the pair bypasses: a
/// relaxation queued at the same time, bound to the pre-freeze epoch, reverts.
#[test]
fn rv_specula_g1_queued_remove_and_readd_clear_a_later_guardian_freeze() {
    let mut t = fixture();
    let delay = t.gov_iface_client().get_min_delay();
    let e0 = epoch(&t, SPOKE_B, "USDC");
    let args = relist_args(&t, SPOKE_B, "USDC");
    let relax = RelaxSpokeAssetFlagsArgs {
        spoke_id: SPOKE_B,
        hub_asset: key(&t, "USDC"),
        expected_epoch: e0,
        paused: false,
        frozen: false,
        no_seize: false,
    };

    // The PROPOSER queues the pair, and a relaxation as the control. None of
    // these variants needs the owner as proposer (`lifecycle::propose`).
    let remove_id = propose(&t, &remove_op(&t, "USDC", SPOKE_B), 1);
    let add_id = propose(&t, &AdminOperation::AddAssetToSpoke(args.clone()), 2);
    let relax_id = propose(&t, &AdminOperation::RelaxSpokeAssetFlags(relax.clone()), 3);
    for id in [&remove_id, &add_id, &relax_id] {
        assert_eq!(op_state(&t, id), OperationState::Waiting);
    }

    // The guardian freezes the empty listing. Entry is refused.
    guardian_freeze(&t);
    assert!(listing(&t, SPOKE_B, "USDC").frozen);
    assert_eq!(
        epoch(&t, SPOKE_B, "USDC"),
        e0 + 1,
        "the freeze advances the epoch"
    );
    assert_contract_error(
        t.try_supply_with_spoke(KATE, "USDC", 1.0, SPOKE_B),
        errors::SPOKE_ASSET_FROZEN,
    );

    advance(&t, delay);

    // Control: the epoch-bound relaxation is dead, as ADR-0007 promises.
    assert_contract_error(
        exec_raw(
            &t,
            "relax_spoke_asset_flags",
            relax_call(&t, &relax),
            salt(&t, 3),
        ),
        errors::SPOKE_FLAGS_EPOCH_MISMATCH,
    );
    assert!(listing(&t, SPOKE_B, "USDC").frozen);

    // Order does not matter to the executor: a re-add before the removal is
    // refused as a duplicate listing and stays Ready for a retry.
    assert_contract_error(
        exec_raw(&t, "add_asset_to_spoke", add_call(&t, &args), salt(&t, 2)),
        ASSET_ALREADY_IN_SPOKE,
    );
    assert_eq!(op_state(&t, &add_id), OperationState::Ready);

    // Anyone drives the removal: the frozen listing is gone, the epoch kept.
    exec_raw(
        &t,
        "remove_asset_from_spoke",
        remove_call(&t, "USDC", SPOKE_B),
        salt(&t, 1),
    )
    .expect("the queued removal executes after the freeze");
    assert_contract_error(try_listing(&t, SPOKE_B, "USDC"), errors::ASSET_NOT_IN_SPOKE);
    assert_eq!(
        epoch(&t, SPOKE_B, "USDC"),
        e0 + 1,
        "removal keeps the flags epoch (endpoints.md)"
    );
    assert_eq!(op_state(&t, &remove_id), OperationState::Unset);

    // Anyone drives the re-add: the Add arm has nothing to ratchet against.
    exec_raw(&t, "add_asset_to_spoke", add_call(&t, &args), salt(&t, 2))
        .expect("the queued re-add executes after the freeze");
    assert_eq!(op_state(&t, &add_id), OperationState::Unset);
    let cfg = listing(&t, SPOKE_B, "USDC");
    // HEAD behaviour. Once the fix lands this becomes `assert!(cfg.frozen)`
    // (kept flags ratcheted on re-add) or the re-add above must revert.
    assert!(
        !cfg.frozen && !cfg.paused && !cfg.no_seize,
        "MC-1: the queued re-add cleared the guardian freeze"
    );
    assert_eq!(
        epoch(&t, SPOKE_B, "USDC"),
        e0 + 2,
        "a listing is an epoch write"
    );

    // Deposits into the listing are accepted again, and stay accepted.
    t.try_supply_with_spoke(KATE, "USDC", 1.0, SPOKE_B)
        .expect("supply reopened by the re-add");
    advance(&t, 100);
    assert!(!listing(&t, SPOKE_B, "USDC").frozen);
    t.try_supply_with_spoke(KATE, "USDC", 1.0, SPOKE_B)
        .expect("still open 100 ledgers later");
}

/// The gate that bounds MC-1. While the listing has any scaled usage the
/// removal reverts with `SpokeAssetInUse`, the re-add reverts as a duplicate,
/// both stay Ready, and the freeze holds. The gate is read at execution
/// time, so a full exit by the only supplier reopens it: `frozen` does not
/// stop withdrawals, and the pair then goes through.
#[test]
fn rv_specula_g1_live_usage_blocks_the_pair_until_the_listing_empties() {
    let mut t = fixture();
    let kate = t.create_spoke_account(KATE, SPOKE_B);
    t.supply_to(KATE, kate, "USDC", 100.0);
    let delay = t.gov_iface_client().get_min_delay();
    let args = relist_args(&t, SPOKE_B, "USDC");

    let remove_id = propose(&t, &remove_op(&t, "USDC", SPOKE_B), 4);
    let add_id = propose(&t, &AdminOperation::AddAssetToSpoke(args.clone()), 5);
    guardian_freeze(&t);
    advance(&t, delay);

    assert_contract_error(
        exec_raw(
            &t,
            "remove_asset_from_spoke",
            remove_call(&t, "USDC", SPOKE_B),
            salt(&t, 4),
        ),
        errors::SPOKE_ASSET_IN_USE,
    );
    assert_contract_error(
        exec_raw(&t, "add_asset_to_spoke", add_call(&t, &args), salt(&t, 5)),
        ASSET_ALREADY_IN_SPOKE,
    );
    assert_eq!(op_state(&t, &remove_id), OperationState::Ready);
    assert_eq!(op_state(&t, &add_id), OperationState::Ready);
    assert!(listing(&t, SPOKE_B, "USDC").frozen, "the freeze holds");
    assert_contract_error(
        t.try_supply_with_spoke(LIAM, "USDC", 1.0, SPOKE_B),
        errors::SPOKE_ASSET_FROZEN,
    );

    // A frozen listing still lets its supplier leave; the gate then opens.
    t.withdraw_all(KATE, "USDC");
    assert_eq!(
        t.ctrl_client()
            .get_spoke_usage(&SPOKE_B, &key(&t, "USDC"))
            .supplied_scaled_ray,
        0
    );
    exec_raw(
        &t,
        "remove_asset_from_spoke",
        remove_call(&t, "USDC", SPOKE_B),
        salt(&t, 4),
    )
    .expect("removal once the listing is empty");
    exec_raw(&t, "add_asset_to_spoke", add_call(&t, &args), salt(&t, 5))
        .expect("re-add once the listing is gone");
    assert!(
        !listing(&t, SPOKE_B, "USDC").frozen,
        "MC-1 again: the pair cleared the freeze as soon as usage hit zero"
    );
    t.try_supply_with_spoke(LIAM, "USDC", 1.0, SPOKE_B)
        .expect("supply reopened");
}
