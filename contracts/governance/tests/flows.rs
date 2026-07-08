//! Governance controller deployment and forwarding tests.

extern crate std;

use crate::op::{AdminOperation, ConfigureOracleArgs, EditToleranceArgs, RoleArgs, SpokeAssetArgs};
use common::types::{
    ControllerKey, HubAssetKey, MarketOracleConfigInput, OracleAssetRef, OracleReadMode,
    OracleSourceConfigInput, OracleSourceConfigInputOption, OracleStrategy, PositionLimits,
    ReflectorSourceConfigInput,
};
use soroban_sdk::testutils::storage::Instance as _;
use soroban_sdk::testutils::{Address as _, Ledger as _, MockAuth, MockAuthInvoke};
use soroban_sdk::{Address, BytesN, Env, IntoVal, Symbol};
use stellar_access::ownable;

use crate::access::EXECUTOR_ROLE;
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

fn upload_controller_wasm(env: &Env) -> BytesN<32> {
    let path = "target/wasm32v1-none/release/controller.wasm";
    let mut bytes = std::fs::read(path);
    if bytes.is_err() {
        bytes = std::fs::read(std::format!("../{path}"));
    }
    if bytes.is_err() {
        bytes = std::fs::read(std::format!("../../{path}"));
    }
    match bytes {
        Ok(b) => env
            .deployer()
            .upload_contract_wasm(soroban_sdk::Bytes::from_slice(env, &b)),
        Err(_) => panic!("Controller WASM not found. Run 'make build' first."),
    }
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
        max_sanity_price_wad: common::constants::MAX_REASONABLE_PRICE_WAD,
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
            args: soroban_sdk::vec![
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
        oracle_override: common::types::MarketOracleConfigOption::None,
    };
    gov.execute_immediate(&admin, &AdminOperation::EditAssetInSpoke(args));
}

#[test]
#[should_panic(expected = "Error(Contract, #226)")]
fn add_asset_to_spoke_rejects_wide_single_source_override_at_propose_time() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, _, gov) = register_governance(&env);
    let asset = Address::generate(&env);
    let salt = BytesN::from_array(&env, &[0u8; 32]);

    // A `Single`-strategy override whose sanity band is far wider than
    // `MAX_SINGLE_SOURCE_SANITY_BAND_BPS`: (2_000 - 1_000) / (2_000 + 1_000) is
    // ~3_333 bps. `resolve_op` must reject it before scheduling, not at execute
    // time after the timelock delay.
    let mut override_cfg = common::types::MarketOracleConfig::pending_for(asset.clone(), 7);
    override_cfg.min_sanity_price_wad = 1_000;
    override_cfg.max_sanity_price_wad = 2_000;

    let args = SpokeAssetArgs {
        hub_id: 1,
        asset,
        spoke_id: 1,
        can_collateral: true,
        can_borrow: true,
        paused: false,
        frozen: false,
        ltv: 7_500,
        threshold: 8_000,
        bonus: 500,
        liquidation_fees: 100,
        supply_cap: 0,
        borrow_cap: 0,
        oracle_override: common::types::MarketOracleConfigOption::Some(override_cfg),
    };
    gov.propose(&admin, &AdminOperation::AddAssetToSpoke(args), &salt);
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

// ===== coverage: op-variant resolution + self-op execution =====

// propose_resolves_all_controller_and_self_variants  (+56)  contracts/governance/src/op.rs:95-104,161-166,191-200,207-212,219-224,274-285,295-300
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

    // op.rs:95-104 TransferGovOwnership (self target, Sensitive)
    gov.propose(
        &admin,
        &AdminOperation::TransferGovOwnership(TransferOwnershipArgs {
            new_owner: Address::generate(&env),
            live_until_ledger: u32::MAX,
        }),
        &salt(),
    );
    // op.rs:161-166 RemoveSpoke
    gov.propose(&admin, &AdminOperation::RemoveSpoke(2), &salt());
    // op.rs:191-200 RemoveAssetFromSpoke
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
    // op.rs:207-212 RevokeToken
    gov.propose(&admin, &AdminOperation::RevokeToken(asset.clone()), &salt());
    // op.rs:219-224 RevokeBlendPool
    gov.propose(
        &admin,
        &AdminOperation::RevokeBlendPool(Address::generate(&env)),
        &salt(),
    );
    // op.rs:274-279 DisableTokenOracle
    gov.propose(
        &admin,
        &AdminOperation::DisableTokenOracle(asset.clone()),
        &salt(),
    );
    // op.rs:280-285 SetPositionManager
    gov.propose(
        &admin,
        &AdminOperation::SetPositionManager(Address::generate(&env), true),
        &salt(),
    );
    // op.rs:295-300 MigrateController
    gov.propose(&admin, &AdminOperation::MigrateController(3), &salt());
}

// execute_self_transfer_then_accept_migrates_owner_and_roles  (+58)  contracts/governance/src/op.rs:390-392; contracts/governance/src/access.rs:34-57,59-79,90-102,194-200
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

// execute_immediate_self_op_applies_inline  (+2)  contracts/governance/src/timelock.rs:402-404
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
}
