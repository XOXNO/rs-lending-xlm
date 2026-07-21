//! Governance controller deployment and forwarding tests.

extern crate std;

use crate::op::{AdminOperation, ConfigureOracleArgs, EditToleranceArgs, RoleArgs, SpokeAssetArgs};
use common::constants::MAX_REASONABLE_PRICE_WAD;
use common::types::{
    ControllerKey, HubAssetKey, MarketOracleConfigInput,
    OracleAssetRef, OracleReadMode, OracleSourceConfigInput,
    OracleSourceConfigInputOption, OracleStrategy, PositionLimits, ReflectorSourceConfigInput,
};
use soroban_sdk::testutils::storage::Instance as _;
use soroban_sdk::testutils::{Address as _, Ledger as _, MockAuth, MockAuthInvoke};
use soroban_sdk::{vec, Address, BytesN, Env, IntoVal, Symbol};
use stellar_access::ownable;

use crate::access::EXECUTOR_ROLE;
use crate::test_support::upload_controller_wasm;
use crate::{constants, storage, Governance, GovernanceClient};

fn register_governance(env: &Env) -> (Address, Address, GovernanceClient<'_>) {
    let admin = Address::generate(env);
    let gov_id = env.register(
        Governance,
        (admin.clone(), constants::TIMELOCK_MIN_DELAY_LEDGERS),
    );
    let gov = GovernanceClient::new(env, &gov_id);
    (admin, gov_id, gov)
}

fn register_native_controller(env: &Env, gov_id: &Address, gov: &GovernanceClient<'_>) -> Address {
    let controller_id = env.register(controller::Controller, (gov_id.clone(),));
    gov.set_controller(&controller_id);
    controller_id
}

fn sample_oracle_input(env: &Env) -> MarketOracleConfigInput {
    MarketOracleConfigInput {
        max_price_stale_seconds: 900,
        tolerance_bps: 500,
        strategy: OracleStrategy::Single,
        primary: OracleSourceConfigInput::Reflector(ReflectorSourceConfigInput {
            contract: Address::generate(env),
            asset: OracleAssetRef::Stellar(Address::generate(env)),
            read_mode: OracleReadMode::Twap(5),
        }),
        anchor: OracleSourceConfigInputOption::None,
        min_sanity_price_wad: 1,
        max_sanity_price_wad: MAX_REASONABLE_PRICE_WAD,
    }
}

fn read_controller_position_limits(env: &Env, controller_id: &Address) -> PositionLimits {
    env.as_contract(controller_id, || {
        env.storage()
            .instance()
            .get(&ControllerKey::PositionLimits)
            .expect("position limits set")
    })
}

#[test]
fn deploy_controller_stores_address_and_governance_owns_it() {
    let env = Env::default();
    env.cost_estimate().budget().reset_unlimited();
    env.cost_estimate().disable_resource_limits();
    env.mock_all_auths();
    let (admin, gov_id, gov) = register_governance(&env);

    let wasm_hash = upload_controller_wasm(&env);
    let controller_id = gov.deploy_controller(&wasm_hash);

    assert_eq!(gov.controller(), controller_id);
    env.as_contract(&controller_id, || {
        assert_eq!(ownable::get_owner(&env), Some(gov_id.clone()));
    });

    // Owner-gated forwarding reaches the deployed controller's storage.
    gov.execute_immediate(
        &admin,
        &AdminOperation::SetPositionLimits(PositionLimits {
            max_supply_positions: 5,
            max_borrow_positions: 4,
        }),
    );
    let stored = read_controller_position_limits(&env, &controller_id);
    assert_eq!(stored.max_supply_positions, 5);
    assert_eq!(stored.max_borrow_positions, 4);
}

#[test]
#[should_panic(expected = "Error(Contract, #5)")]
fn deploy_controller_twice_panics() {
    let env = Env::default();
    env.cost_estimate().budget().reset_unlimited();
    env.cost_estimate().disable_resource_limits();
    env.mock_all_auths();
    let (_, _, gov) = register_governance(&env);

    let wasm_hash = upload_controller_wasm(&env);
    gov.deploy_controller(&wasm_hash);
    gov.deploy_controller(&wasm_hash);
}

#[test]
#[should_panic(expected = "Error(Contract, #30)")]
fn controller_view_panics_when_unset() {
    let env = Env::default();
    let (_, _, gov) = register_governance(&env);
    gov.controller();
}

// Validation runs before controller lookup.
#[test]
#[should_panic(expected = "Error(Contract, #36)")]
fn validation_runs_before_controller_lookup() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, _, gov) = register_governance(&env);

    gov.execute_immediate(
        &admin,
        &AdminOperation::SetPositionLimits(PositionLimits {
            max_supply_positions: 0,
            max_borrow_positions: 5,
        }),
    );
}

#[test]
fn set_position_limits_forwards_to_native_controller() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, gov_id, gov) = register_governance(&env);
    let controller_id = register_native_controller(&env, &gov_id, &gov);

    gov.execute_immediate(
        &admin,
        &AdminOperation::SetPositionLimits(PositionLimits {
            max_supply_positions: 3,
            max_borrow_positions: 2,
        }),
    );

    let stored = read_controller_position_limits(&env, &controller_id);
    assert_eq!(stored.max_supply_positions, 3);
    assert_eq!(stored.max_borrow_positions, 2);
}

// Mock governance-owner auth; controller owner auth must pass through the
// invoker path.
#[test]
fn forwarding_passes_controller_owner_auth_via_invoker() {
    let env = Env::default();
    let (admin, gov_id, gov) = register_governance(&env);
    let controller_id = env.register(controller::Controller, (gov_id.clone(),));
    env.as_contract(&gov_id, || {
        storage::set_controller(&env, &controller_id);
    });

    let limits = PositionLimits {
        max_supply_positions: 7,
        max_borrow_positions: 6,
    };
    let op = AdminOperation::SetPositionLimits(limits);
    env.mock_auths(&[MockAuth {
        address: &admin,
        invoke: &MockAuthInvoke {
            contract: &gov_id,
            fn_name: "execute_immediate",
            args: vec![
                &env,
                admin.clone().into_val(&env),
                op.clone().into_val(&env)
            ],
            sub_invokes: &[],
        },
    }]);
    gov.execute_immediate(&admin, &op);

    let stored = read_controller_position_limits(&env, &controller_id);
    assert_eq!(stored.max_supply_positions, 7);
    assert_eq!(stored.max_borrow_positions, 6);
}

#[test]
fn pause_and_unpause_forward_to_controller() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, gov_id, gov) = register_governance(&env);
    let controller_id = register_native_controller(&env, &gov_id, &gov);

    // Unpause is a timelocked controller op; `execute_immediate` applies it in-test.
    gov.execute_immediate(&admin, &AdminOperation::Unpause);
    assert!(!env.as_contract(&controller_id, || {
        stellar_contract_utils::pausable::paused(&env)
    }));

    // GUARDIAN halts the controller immediately.
    gov.pause(&admin);
    assert!(env.as_contract(&controller_id, || {
        stellar_contract_utils::pausable::paused(&env)
    }));

    gov.execute_immediate(&admin, &AdminOperation::Unpause);
    assert!(!env.as_contract(&controller_id, || {
        stellar_contract_utils::pausable::paused(&env)
    }));
}

#[test]
#[should_panic(expected = "Error(Contract, #2000)")]
fn configure_market_oracle_requires_oracle_role() {
    let env = Env::default();
    env.mock_all_auths();
    let (_admin, _, gov) = register_governance(&env);
    let stranger = Address::generate(&env);
    let asset = Address::generate(&env);

    gov.execute_immediate(
        &stranger,
        &AdminOperation::ConfigureMarketOracle(ConfigureOracleArgs {
            hub_asset: HubAssetKey { hub_id: 0, asset },
            cfg: sample_oracle_input(&env),
        }),
    );
}

// Tolerance validation runs before controller lookup.
#[test]
#[should_panic(expected = "Error(Contract, #208)")]
fn edit_oracle_tolerance_validates_before_any_cross_call() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, _, gov) = register_governance(&env);
    let asset = Address::generate(&env);

    gov.execute_immediate(
        &admin,
        &AdminOperation::EditOracleTolerance(EditToleranceArgs {
            asset,
            tolerance: 0,
        }),
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #201)")]
fn set_aggregator_rejects_non_contract_address() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, _, gov) = register_governance(&env);

    gov.execute_immediate(
        &admin,
        &AdminOperation::SetAggregator(Address::generate(&env)),
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #201)")]
fn set_aggregator_rejects_stellar_asset_contract() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, _, gov) = register_governance(&env);
    let stellar_asset = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();

    gov.execute_immediate(&admin, &AdminOperation::SetAggregator(stellar_asset));
}

// The Wasm-executable acceptance leg of `require_contract_address`: a real
// deployed contract must pass validation and reach controller storage.
#[test]
fn set_aggregator_accepts_wasm_contract_address() {
    let env = Env::default();
    env.cost_estimate().budget().reset_unlimited();
    env.cost_estimate().disable_resource_limits();
    env.mock_all_auths();
    let (admin, _, gov) = register_governance(&env);
    let controller_id = gov.deploy_controller(&upload_controller_wasm(&env));

    gov.execute_immediate(
        &admin,
        &AdminOperation::SetAggregator(controller_id.clone()),
    );

    let stored: Address = env.as_contract(&controller_id, || {
        env.storage()
            .instance()
            .get(&ControllerKey::Aggregator)
            .expect("aggregator stored")
    });
    assert_eq!(stored, controller_id);
}

#[test]
fn set_accumulator_accepts_wallet_address() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, gov_id, gov) = register_governance(&env);
    let controller_id = register_native_controller(&env, &gov_id, &gov);
    let treasury = Address::generate(&env);

    gov.execute_immediate(&admin, &AdminOperation::SetAccumulator(treasury.clone()));

    let stored: Address = env.as_contract(&controller_id, || {
        env.storage()
            .instance()
            .get(&ControllerKey::Accumulator)
            .expect("accumulator stored")
    });
    assert_eq!(stored, treasury);
}

#[test]
#[should_panic(expected = "Error(Contract, #10)")]
fn set_liquidity_pool_template_rejects_zero_hash() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, _, gov) = register_governance(&env);

    gov.execute_immediate(
        &admin,
        &AdminOperation::SetLiquidityPoolTemplate(BytesN::from_array(&env, &[0u8; 32])),
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #10)")]
fn deploy_controller_rejects_zero_hash() {
    let env = Env::default();
    env.mock_all_auths();
    let (_, _, gov) = register_governance(&env);

    gov.deploy_controller(&BytesN::from_array(&env, &[0u8; 32]));
}

#[test]
#[should_panic(expected = "Error(Contract, #10)")]
fn propose_upgrade_pool_rejects_zero_hash() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, _, gov) = register_governance(&env);
    let salt = BytesN::from_array(&env, &[0u8; 32]);

    gov.propose(
        &admin,
        &AdminOperation::UpgradePool(BytesN::from_array(&env, &[0u8; 32])),
        &salt,
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #10)")]
fn propose_upgrade_controller_rejects_zero_hash() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, _, gov) = register_governance(&env);
    let salt = BytesN::from_array(&env, &[0u8; 32]);

    gov.propose(
        &admin,
        &AdminOperation::UpgradeController(BytesN::from_array(&env, &[0u8; 32])),
        &salt,
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #113)")]
fn edit_asset_in_spoke_rejects_bad_risk_bounds_before_any_cross_call() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, _, gov) = register_governance(&env);
    let asset = Address::generate(&env);

    let args = SpokeAssetArgs {
        hub_id: 1,
        asset,
        spoke_id: 1,
        can_collateral: true,
        can_borrow: true,
        paused: false,
        frozen: false,
        ltv: 9_000,
        // Threshold below LTV is invalid.
        threshold: 8_000,
        bonus: 500,
        liquidation_fees: 100,
        supply_cap: 0,
        borrow_cap: 0,
    };
    gov.execute_immediate(&admin, &AdminOperation::EditAssetInSpoke(args));
}

// Admin entrypoints renew instance TTL for ownable, role, and controller keys.
#[test]
fn entrypoint_renews_governance_instance_ttl() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, gov_id, gov) = register_governance(&env);

    let initial_ttl = env.as_contract(&gov_id, || env.storage().instance().get_ttl());

    let role = Symbol::new(&env, EXECUTOR_ROLE);
    let salt = BytesN::<32>::from_array(&env, &[0u8; 32]);
    let grantee = Address::generate(&env);
    gov.propose(
        &admin,
        &AdminOperation::GrantGovRole(RoleArgs {
            account: grantee.clone(),
            role: role.clone(),
        }),
        &salt,
    );
    env.ledger().with_mut(|l| {
        l.sequence_number += constants::TIMELOCK_MIN_DELAY_LEDGERS;
    });
    gov.execute_self(
        &Some(admin.clone()),
        &AdminOperation::GrantGovRole(RoleArgs {
            account: grantee.clone(),
            role: role.clone(),
        }),
        &salt,
    );

    let renewed_ttl = env.as_contract(&gov_id, || env.storage().instance().get_ttl());
    assert!(
        renewed_ttl > initial_ttl,
        "instance TTL must be renewed: renewed={renewed_ttl}, initial={initial_ttl}"
    );
}

#[test]
fn propose_resolves_all_controller_and_self_variants() {
    use crate::op::{RemoveAssetFromSpokeArgs, TransferOwnershipArgs};
    let env = Env::default();
    env.mock_all_auths();
    let (admin, gov_id, gov) = register_governance(&env);
    let _controller = register_native_controller(&env, &gov_id, &gov);

    let asset = Address::generate(&env);
    let mut n: u8 = 0;
    let mut salt = || {
        n += 1;
        BytesN::<32>::from_array(&env, &[n; 32])
    };

    // Self-targeted, sensitive operation.
    gov.propose(
        &admin,
        &AdminOperation::TransferGovOwnership(TransferOwnershipArgs {
            new_owner: Address::generate(&env),
            live_until_ledger: u32::MAX,
        }),
        &salt(),
    );
    gov.propose(&admin, &AdminOperation::RemoveSpoke(2), &salt());
    gov.propose(
        &admin,
        &AdminOperation::RemoveAssetFromSpoke(RemoveAssetFromSpokeArgs {
            hub_asset: HubAssetKey {
                hub_id: 0,
                asset: asset.clone(),
            },
            spoke_id: 1,
        }),
        &salt(),
    );
    gov.propose(&admin, &AdminOperation::RevokeToken(asset.clone()), &salt());
    gov.propose(
        &admin,
        &AdminOperation::RevokeBlendPool(Address::generate(&env)),
        &salt(),
    );
    gov.propose(
        &admin,
        &AdminOperation::SetPositionManager(Address::generate(&env), true),
        &salt(),
    );
    gov.propose(&admin, &AdminOperation::MigrateController(3), &salt());
}

#[test]
fn execute_self_transfer_then_accept_migrates_owner_and_roles() {
    use crate::access::{EXECUTOR_ROLE, ORACLE_ROLE};
    use crate::constants::TIMELOCK_SENSITIVE_MIN_DELAY_LEDGERS;
    use crate::op::{AdminOperation, TransferOwnershipArgs};
    use crate::{constants, Governance, GovernanceClient};
    use soroban_sdk::testutils::{Address as _, Ledger as _};
    use soroban_sdk::{Address, BytesN, Env, Symbol};
    use stellar_access::{access_control, ownable};

    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let gov_id = env.register(
        Governance,
        (admin.clone(), constants::TIMELOCK_MIN_DELAY_LEDGERS),
    );
    let gov = GovernanceClient::new(&env, &gov_id);
    let new_owner = Address::generate(&env);

    let live_until = env.ledger().sequence() + TIMELOCK_SENSITIVE_MIN_DELAY_LEDGERS + 10_000;
    let op = AdminOperation::TransferGovOwnership(TransferOwnershipArgs {
        new_owner: new_owner.clone(),
        live_until_ledger: live_until,
    });
    let salt = BytesN::<32>::from_array(&env, &[0u8; 32]);
    gov.propose(&admin, &op, &salt);
    env.ledger()
        .with_mut(|l| l.sequence_number += TIMELOCK_SENSITIVE_MIN_DELAY_LEDGERS);
    gov.execute_self(&Some(admin.clone()), &op, &salt);

    let pending_admin = env.as_contract(&gov_id, || {
        env.storage()
            .temporary()
            .get::<_, stellar_access::role_transfer::PendingTransfer>(
                &access_control::AccessControlStorageKey::PendingAdmin,
            )
            .expect("pending admin transfer")
    });
    assert_eq!(pending_admin.address, new_owner);
    assert_eq!(pending_admin.live_until_ledger, live_until);

    // New owner accepts -> sync_owner_access_control migrates admin + roles.
    gov.accept_ownership();
    env.as_contract(&gov_id, || {
        assert_eq!(ownable::get_owner(&env), Some(new_owner.clone()));
        assert_eq!(access_control::get_admin(&env), Some(new_owner.clone()));
    });
    assert!(gov.has_role(&new_owner, &Symbol::new(&env, ORACLE_ROLE)));
    assert!(gov.has_role(&new_owner, &Symbol::new(&env, EXECUTOR_ROLE)));
    assert!(!gov.has_role(&admin, &Symbol::new(&env, ORACLE_ROLE)));
}

#[test]
fn execute_immediate_self_op_applies_inline() {
    use crate::op::RoleArgs;
    let env = Env::default();
    env.mock_all_auths();
    let (admin, _gov_id, gov) = register_governance(&env);
    let grantee = Address::generate(&env);
    let role = Symbol::new(&env, EXECUTOR_ROLE);

    gov.execute_immediate(
        &admin,
        &AdminOperation::GrantGovRole(RoleArgs {
            account: grantee.clone(),
            role: role.clone(),
        }),
    );
    assert!(gov.has_role(&grantee, &role));

    gov.execute_immediate(
        &admin,
        &AdminOperation::RevokeGovRole(RoleArgs {
            account: grantee.clone(),
            role: role.clone(),
        }),
    );
    assert!(!gov.has_role(&grantee, &role));
}

// Guardian/oracle immediate forwarders: the role gate must run and the call
// must reach the controller. These live here (not only in the harness)
// because the governance mutation scope runs governance tests alone.

fn grant_incident_role(
    env: &Env,
    admin: &Address,
    gov: &GovernanceClient<'_>,
    who: &Address,
    role: &str,
) {
    gov.execute_immediate(
        admin,
        &AdminOperation::GrantGovRole(RoleArgs {
            account: who.clone(),
            role: Symbol::new(env, role),
        }),
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #2000)")]
fn set_spoke_asset_flags_requires_guardian_role() {
    let env = Env::default();
    env.mock_all_auths();
    let (_, gov_id, gov) = register_governance(&env);
    register_native_controller(&env, &gov_id, &gov);
    let stranger = Address::generate(&env);

    gov.set_spoke_asset_flags(
        &stranger,
        &1u32,
        &HubAssetKey {
            hub_id: 0,
            asset: Address::generate(&env),
        },
        &true,
        &true,
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #307)")]
fn guardian_set_spoke_asset_flags_reaches_controller_listing_check() {
    use crate::access::GUARDIAN_ROLE;
    let env = Env::default();
    env.mock_all_auths();
    let (admin, gov_id, gov) = register_governance(&env);
    register_native_controller(&env, &gov_id, &gov);
    let guardian = Address::generate(&env);
    grant_incident_role(&env, &admin, &gov, &guardian, GUARDIAN_ROLE);

    // Spoke exists but the asset is not listed on it: the controller's
    // AssetNotInSpoke proves the forwarding happened.
    let spoke_id = gov.add_spoke(&guardian);
    gov.set_spoke_asset_flags(
        &guardian,
        &spoke_id,
        &HubAssetKey {
            hub_id: 0,
            asset: Address::generate(&env),
        },
        &true,
        &true,
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #2000)")]
fn set_oracle_sanity_bounds_requires_oracle_role() {
    let env = Env::default();
    env.mock_all_auths();
    let (_, gov_id, gov) = register_governance(&env);
    register_native_controller(&env, &gov_id, &gov);
    let stranger = Address::generate(&env);

    gov.set_oracle_sanity_bounds(&stranger, &Address::generate(&env), &1i128, &2i128);
}

#[test]
#[should_panic(expected = "Error(Contract, #12)")]
fn oracle_set_sanity_bounds_reaches_controller_pair_check() {
    use crate::access::ORACLE_ROLE;
    let env = Env::default();
    env.mock_all_auths();
    let (admin, gov_id, gov) = register_governance(&env);
    register_native_controller(&env, &gov_id, &gov);
    let bot = Address::generate(&env);
    grant_incident_role(&env, &admin, &gov, &bot, ORACLE_ROLE);

    // No oracle configured for the asset: the controller's PairNotActive
    // proves the forwarding happened.
    gov.set_oracle_sanity_bounds(&bot, &Address::generate(&env), &1i128, &2i128);
}

#[test]
fn guardian_create_hub_and_add_spoke_forward_and_return_controller_ids() {
    use crate::access::GUARDIAN_ROLE;
    let env = Env::default();
    env.mock_all_auths();
    let (admin, gov_id, gov) = register_governance(&env);
    register_native_controller(&env, &gov_id, &gov);
    let guardian = Address::generate(&env);
    grant_incident_role(&env, &admin, &gov, &guardian, GUARDIAN_ROLE);

    assert_eq!(gov.create_hub(&guardian), 1);
    assert_eq!(gov.create_hub(&guardian), 2);

    let first_spoke = gov.add_spoke(&guardian);
    let second_spoke = gov.add_spoke(&guardian);
    assert!(first_spoke >= 1);
    assert_eq!(second_spoke, first_spoke + 1);
}

#[test]
fn revoke_role_immediate_strips_only_the_named_incident_role() {
    use crate::access::{GUARDIAN_ROLE, ORACLE_ROLE};
    let env = Env::default();
    env.mock_all_auths();
    let (admin, _, gov) = register_governance(&env);
    let key = Address::generate(&env);
    grant_incident_role(&env, &admin, &gov, &key, GUARDIAN_ROLE);
    grant_incident_role(&env, &admin, &gov, &key, ORACLE_ROLE);

    gov.revoke_role_immediate(&key, &Symbol::new(&env, GUARDIAN_ROLE));
    assert!(!gov.has_role(&key, &Symbol::new(&env, GUARDIAN_ROLE)));
    assert!(gov.has_role(&key, &Symbol::new(&env, ORACLE_ROLE)));

    gov.revoke_role_immediate(&key, &Symbol::new(&env, ORACLE_ROLE));
    assert!(!gov.has_role(&key, &Symbol::new(&env, ORACLE_ROLE)));
}

// `Controller::upgrade` must pause a running controller before swapping the
// Wasm, and must actually perform the swap-side pause even when invoked
// while already paused (the guard skips only the double-pause panic).
#[test]
fn controller_upgrade_pauses_running_contract_before_wasm_swap() {
    let env = Env::default();
    env.cost_estimate().budget().reset_unlimited();
    env.cost_estimate().disable_resource_limits();
    env.mock_all_auths();
    let (admin, gov_id, gov) = register_governance(&env);
    let controller_id = register_native_controller(&env, &gov_id, &gov);

    // The controller deploys paused; bring it into the running state.
    gov.execute_immediate(&admin, &AdminOperation::Unpause);
    env.as_contract(&controller_id, || {
        assert!(!stellar_contract_utils::pausable::paused(&env));
    });

    let ctrl = controller::ControllerClient::new(&env, &controller_id);
    ctrl.upgrade(&upload_controller_wasm(&env));

    env.as_contract(&controller_id, || {
        assert!(
            stellar_contract_utils::pausable::paused(&env),
            "upgrade must leave the controller paused"
        );
    });
}
