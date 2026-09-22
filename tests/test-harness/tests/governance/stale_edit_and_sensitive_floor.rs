//! A listing edit or flag relaxation queued before a guardian freeze cannot
//! clear it when executed; a timely relaxation can; a `Sensitive` operation is
//! ready at `max(min_delay, floor)`.

use controller::types::{HubAssetKey, PositionLimits};
use governance::op::{
    AdminOperation, RelaxSpokeAssetFlagsArgs, RoleArgs, SpokeAssetArgs, TransferOwnershipArgs,
};
use governance_interface::OperationState;
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{Address, BytesN, Env, IntoVal, Symbol, Val, Vec};
use test_harness::{
    assert_contract_error, errors, hub_asset, usdc_preset, LendingTest, ALICE, HARNESS_HUB,
    HARNESS_SPOKE,
};

/// Copy of `governance::constants::TIMELOCK_SENSITIVE_MIN_DELAY_LEDGERS` (private).
const SENSITIVE_FLOOR: u32 = 12;

fn salt(env: &Env, byte: u8) -> BytesN<32> {
    BytesN::<32>::from_array(env, &[byte; 32])
}

fn flatten<T, C>(
    result: Result<Result<T, C>, Result<soroban_sdk::Error, soroban_sdk::InvokeError>>,
) -> Result<(), soroban_sdk::Error> {
    match result {
        Ok(_) => Ok(()),
        Err(Ok(err)) => Err(err),
        Err(Err(invoke)) => panic!("expected contract error, got {invoke:?}"),
    }
}

/// Grants GUARDIAN, and nothing else, to a fresh address.
fn grant_guardian(t: &LendingTest) -> Address {
    let guardian = Address::generate(&t.env);
    t.gov_client().execute_immediate(
        &t.admin(),
        &AdminOperation::GrantGovRole(RoleArgs {
            account: guardian.clone(),
            role: Symbol::new(&t.env, "GUARDIAN"),
        }),
    );
    guardian
}

fn relax_call_args(env: &Env, args: &RelaxSpokeAssetFlagsArgs) -> Vec<Val> {
    soroban_sdk::vec![
        env,
        args.spoke_id.into_val(env),
        args.hub_asset.clone().into_val(env),
        args.expected_epoch.into_val(env),
        args.paused.into_val(env),
        args.frozen.into_val(env),
        args.no_seize.into_val(env),
    ]
}

fn clear_all(key: &HubAssetKey, expected_epoch: u64) -> RelaxSpokeAssetFlagsArgs {
    RelaxSpokeAssetFlagsArgs {
        spoke_id: HARNESS_SPOKE,
        hub_asset: key.clone(),
        expected_epoch,
        paused: false,
        frozen: false,
        no_seize: false,
    }
}

/// Executes a Ready relaxation with no signature at all, as any address can.
fn execute_relax_as_stranger(
    t: &LendingTest,
    args: &RelaxSpokeAssetFlagsArgs,
    salt_byte: u8,
) -> Result<(), soroban_sdk::Error> {
    let gov = governance_interface::GovernanceClient::new(&t.env, &t.governance);
    t.env.set_auths(&[]);
    let result = flatten(gov.try_execute(
        &None,
        &t.controller,
        &Symbol::new(&t.env, "relax_spoke_asset_flags"),
        &relax_call_args(&t.env, args),
        &salt(&t.env, 0),
        &salt(&t.env, salt_byte),
    ));
    t.env.mock_all_auths_allowing_non_root_auth();
    result
}

#[test]
fn stale_edit_executed_by_a_stranger_cannot_clear_a_guardian_freeze() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    let admin = t.admin();
    let usdc = t.resolve_asset("USDC");
    let key = hub_asset(usdc.clone());
    let (env, gov_addr) = (t.env.clone(), t.governance.clone());
    let gov = governance_interface::GovernanceClient::new(&env, &gov_addr);
    let guardian = grant_guardian(&t);

    // Routine cap change, flags copied from the live (unfrozen) listing.
    let cfg = t.ctrl_client().get_spoke_asset(&HARNESS_SPOKE, &key);
    assert!(!cfg.frozen);
    let args = SpokeAssetArgs {
        hub_id: HARNESS_HUB,
        asset: usdc.clone(),
        spoke_id: HARNESS_SPOKE,
        can_collateral: cfg.is_collateralizable,
        can_borrow: cfg.is_borrowable,
        paused: false,
        frozen: false,
        no_seize: false,
        ltv: cfg.loan_to_value,
        threshold: cfg.liquidation_threshold,
        bonus: cfg.liquidation_bonus,
        liquidation_fees: cfg.liquidation_fees,
        supply_cap: cfg.supply_cap - 1,
        borrow_cap: cfg.borrow_cap,
    };
    let id = gov.propose(
        &admin,
        &AdminOperation::EditAssetInSpoke(args.clone()),
        &salt(&t.env, 1),
    );

    // Incident: the guardian freezes the listing through the immediate path.
    gov.set_spoke_asset_flags(&guardian, &HARNESS_SPOKE, &key, &false, &true, &false);
    assert!(t.ctrl_client().get_spoke_asset(&HARNESS_SPOKE, &key).frozen);
    assert_contract_error(
        t.try_supply(ALICE, "USDC", 10.0),
        errors::SPOKE_ASSET_FROZEN,
    );
    // The guardian itself cannot relax the flag it set.
    assert_contract_error(
        flatten(gov.try_set_spoke_asset_flags(
            &guardian,
            &HARNESS_SPOKE,
            &key,
            &false,
            &false,
            &false,
        )),
        errors::SPOKE_ASSET_FLAG_RELAXATION,
    );

    let delay = gov.get_min_delay();
    t.env.ledger().with_mut(|l| l.sequence_number += delay);
    assert_eq!(gov.get_operation_state(&id), OperationState::Ready);

    // A stranger, with no signature at all, drives the stale operation.
    t.env.set_auths(&[]);
    let result = flatten(gov.try_execute(
        &None,
        &t.controller,
        &Symbol::new(&t.env, "edit_asset_in_spoke"),
        &soroban_sdk::vec![&t.env, args.into_val(&t.env)],
        &salt(&t.env, 0),
        &salt(&t.env, 1),
    ));
    t.env.mock_all_auths_allowing_non_root_auth();
    assert_contract_error(result, errors::SPOKE_ASSET_FLAG_RELAXATION);

    let after = t.ctrl_client().get_spoke_asset(&HARNESS_SPOKE, &key);
    assert!(
        after.frozen,
        "the guardian freeze must survive the stale edit"
    );
    assert_eq!(
        after.supply_cap, cfg.supply_cap,
        "the reverted edit must not land"
    );
    assert_contract_error(
        t.try_supply(ALICE, "USDC", 10.0),
        errors::SPOKE_ASSET_FROZEN,
    );
}

#[test]
fn relax_executed_after_its_delay_reopens_a_frozen_listing() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    let key = hub_asset(t.resolve_asset("USDC"));
    let gov = governance_interface::GovernanceClient::new(&t.env, &t.governance);
    let guardian = grant_guardian(&t);

    gov.set_spoke_asset_flags(&guardian, &HARNESS_SPOKE, &key, &true, &true, &false);
    let epoch = t
        .ctrl_client()
        .get_spoke_asset_flags_epoch(&HARNESS_SPOKE, &key);
    let args = clear_all(&key, epoch);
    let id = gov.propose(
        &t.admin(),
        &AdminOperation::RelaxSpokeAssetFlags(args.clone()),
        &salt(&t.env, 2),
    );
    assert_eq!(gov.get_operation_state(&id), OperationState::Waiting);
    assert_contract_error(
        execute_relax_as_stranger(&t, &args, 2),
        errors::TIMELOCK_UNEXPECTED_STATE,
    );
    assert!(t.ctrl_client().get_spoke_asset(&HARNESS_SPOKE, &key).paused);

    let delay = gov.get_min_delay();
    t.env.ledger().with_mut(|l| l.sequence_number += delay);
    assert_eq!(gov.get_operation_state(&id), OperationState::Ready);
    execute_relax_as_stranger(&t, &args, 2).expect("a timely relaxation executes");

    let after = t.ctrl_client().get_spoke_asset(&HARNESS_SPOKE, &key);
    assert!(!after.paused && !after.frozen && !after.no_seize);
    assert_eq!(
        t.ctrl_client()
            .get_spoke_asset_flags_epoch(&HARNESS_SPOKE, &key),
        epoch + 1
    );
    assert_eq!(gov.get_operation_state(&id), OperationState::Unset);
    assert!(
        t.try_supply(ALICE, "USDC", 10.0).is_ok(),
        "the relaxation must reopen supply"
    );
}

#[test]
fn relax_proposed_before_a_later_guardian_freeze_reverts_at_execution() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    let key = hub_asset(t.resolve_asset("USDC"));
    let gov = governance_interface::GovernanceClient::new(&t.env, &t.governance);
    let guardian = grant_guardian(&t);

    gov.set_spoke_asset_flags(&guardian, &HARNESS_SPOKE, &key, &true, &false, &false);
    let args = clear_all(
        &key,
        t.ctrl_client()
            .get_spoke_asset_flags_epoch(&HARNESS_SPOKE, &key),
    );
    let id = gov.propose(
        &t.admin(),
        &AdminOperation::RelaxSpokeAssetFlags(args.clone()),
        &salt(&t.env, 3),
    );

    // A second incident while the relaxation waits.
    gov.set_spoke_asset_flags(&guardian, &HARNESS_SPOKE, &key, &true, &true, &false);

    let delay = gov.get_min_delay();
    t.env.ledger().with_mut(|l| l.sequence_number += delay);
    assert_eq!(gov.get_operation_state(&id), OperationState::Ready);
    assert_contract_error(
        execute_relax_as_stranger(&t, &args, 3),
        errors::SPOKE_FLAGS_EPOCH_MISMATCH,
    );

    let after = t.ctrl_client().get_spoke_asset(&HARNESS_SPOKE, &key);
    assert!(
        after.paused && after.frozen,
        "the later freeze must survive"
    );
    assert_contract_error(
        t.try_supply(ALICE, "USDC", 10.0),
        errors::SPOKE_ASSET_PAUSED,
    );
}

#[test]
fn relax_with_a_stale_epoch_is_rejected_at_proposal() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let key = hub_asset(t.resolve_asset("USDC"));
    let gov = governance_interface::GovernanceClient::new(&t.env, &t.governance);
    let epoch = t
        .ctrl_client()
        .get_spoke_asset_flags_epoch(&HARNESS_SPOKE, &key);

    assert_contract_error(
        flatten(gov.try_propose(
            &t.admin(),
            &AdminOperation::RelaxSpokeAssetFlags(clear_all(&key, epoch + 1)),
            &salt(&t.env, 4),
        )),
        errors::SPOKE_FLAGS_EPOCH_MISMATCH,
    );
}

struct SensitiveCase {
    label: &'static str,
    op: AdminOperation,
    target: Address,
    function: &'static str,
    args: Vec<Val>,
}

#[test]
fn sensitive_operations_are_not_ready_at_a_min_delay_below_the_floor() {
    let t = LendingTest::new().build();
    let env = &t.env;
    let admin = t.admin();

    // The harness governance runs above the floor; this instance runs below it.
    const LOW_MIN_DELAY: u32 = 3;
    let gov_addr = env.register(governance::Governance, (admin.clone(), LOW_MIN_DELAY));
    governance::GovernanceClient::new(env, &gov_addr).set_controller(&t.controller);
    let gov = governance_interface::GovernanceClient::new(env, &gov_addr);
    assert_eq!(gov.get_min_delay(), LOW_MIN_DELAY);
    const _: () = assert!(LOW_MIN_DELAY < SENSITIVE_FLOOR);

    let wasm_hash = BytesN::<32>::from_array(env, &[7u8; 32]);
    let grantee = Address::generate(env);
    let executor_role = Symbol::new(env, "EXECUTOR");
    let cases = [
        SensitiveCase {
            label: "UpgradeController",
            op: AdminOperation::UpgradeController(wasm_hash.clone()),
            target: t.controller.clone(),
            function: "upgrade",
            args: soroban_sdk::vec![env, wasm_hash.into_val(env)],
        },
        SensitiveCase {
            label: "TransferCtrlOwnership",
            op: AdminOperation::TransferCtrlOwnership(TransferOwnershipArgs {
                new_owner: gov_addr.clone(),
                live_until_ledger: 1_000,
            }),
            target: t.controller.clone(),
            function: "transfer_ownership",
            args: soroban_sdk::vec![env, gov_addr.into_val(env), 1_000u32.into_val(env)],
        },
        SensitiveCase {
            label: "GrantGovRole",
            op: AdminOperation::GrantGovRole(RoleArgs {
                account: grantee.clone(),
                role: executor_role.clone(),
            }),
            target: gov_addr.clone(),
            function: "grant_role",
            args: soroban_sdk::vec![env, grantee.into_val(env), executor_role.into_val(env)],
        },
    ];

    let proposed_at = env.ledger().sequence();
    let ids: std::vec::Vec<BytesN<32>> = cases
        .iter()
        .enumerate()
        .map(|(i, c)| gov.propose(&admin, &c.op, &salt(env, 10 + i as u8)))
        .collect();
    // Control: a Standard operation on the same instance uses the bare min delay.
    let standard = gov.propose(
        &admin,
        &AdminOperation::SetPositionLimits(PositionLimits {
            max_supply_positions: 4,
            max_borrow_positions: 3,
        }),
        &salt(env, 9),
    );
    assert_eq!(
        gov.get_operation_ledger(&standard),
        proposed_at + LOW_MIN_DELAY
    );

    for (c, id) in cases.iter().zip(&ids) {
        assert_eq!(
            gov.get_operation_ledger(id),
            proposed_at + SENSITIVE_FLOOR,
            "{}: ready ledger must be the sensitive floor",
            c.label
        );
    }

    let try_execute = |i: usize| {
        let c = &cases[i];
        // Self-targeted operations go through `execute_self`.
        if c.target == gov_addr {
            return flatten(gov.try_execute_self(&None, &c.op, &salt(env, 10 + i as u8)));
        }
        flatten(gov.try_execute(
            &None,
            &c.target,
            &Symbol::new(env, c.function),
            &c.args,
            &salt(env, 0),
            &salt(env, 10 + i as u8),
        ))
    };

    // At min_delay, and at the last ledger before the floor: still waiting.
    for offset in [LOW_MIN_DELAY, SENSITIVE_FLOOR - 1] {
        env.ledger().set_sequence_number(proposed_at + offset);
        assert_eq!(gov.get_operation_state(&standard), OperationState::Ready);
        for (i, (c, id)) in cases.iter().zip(&ids).enumerate() {
            assert_eq!(
                gov.get_operation_state(id),
                OperationState::Waiting,
                "{} at +{offset}",
                c.label
            );
            assert_contract_error(try_execute(i), errors::TIMELOCK_UNEXPECTED_STATE);
        }
    }

    env.ledger()
        .set_sequence_number(proposed_at + SENSITIVE_FLOOR);
    for (c, id) in cases.iter().zip(&ids) {
        assert_eq!(
            gov.get_operation_state(id),
            OperationState::Ready,
            "{}",
            c.label
        );
    }
    // The self-targeted one can be driven to completion on this instance.
    try_execute(2).expect("GrantGovRole executes at the floor");
    assert_eq!(gov.get_operation_state(&ids[2]), OperationState::Unset);
}
