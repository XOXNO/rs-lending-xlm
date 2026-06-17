//! Integration tests for governance-owned controller deployment and forwarding.
//!
//! Tests use native controller registration for forwarding and the release WASM
//! fixture for one-time deployment.

extern crate std;

use controller_interface::types::{
    ControllerKey, MarketOracleConfigInput, OracleAssetRef, OracleReadMode,
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
        first_tolerance_bps: 200,
        last_tolerance_bps: 500,
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
    let (_, gov_id, gov) = register_governance(&env);

    let wasm_hash = upload_controller_wasm(&env);
    let controller_id = gov.deploy_controller(&wasm_hash);

    assert_eq!(gov.controller(), controller_id);
    env.as_contract(&controller_id, || {
        assert_eq!(ownable::get_owner(&env), Some(gov_id.clone()));
    });

    // Owner-gated forwarding reaches the deployed controller's storage.
    gov.set_position_limits(&PositionLimits {
        max_supply_positions: 5,
        max_borrow_positions: 4,
    });
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

// With no controller set, InvalidPositionLimits confirms validation runs before
// controller lookup.
#[test]
#[should_panic(expected = "Error(Contract, #36)")]
fn set_position_limits_rejects_zero_before_any_cross_call() {
    let env = Env::default();
    env.mock_all_auths();
    let (_, _, gov) = register_governance(&env);

    gov.set_position_limits(&PositionLimits {
        max_supply_positions: 0,
        max_borrow_positions: 5,
    });
}

#[test]
fn set_position_limits_forwards_to_native_controller() {
    let env = Env::default();
    env.mock_all_auths();
    let (_, gov_id, gov) = register_governance(&env);
    let controller_id = register_native_controller(&env, &gov_id, &gov);

    gov.set_position_limits(&PositionLimits {
        max_supply_positions: 3,
        max_borrow_positions: 2,
    });

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
    env.mock_auths(&[MockAuth {
        address: &admin,
        invoke: &MockAuthInvoke {
            contract: &gov_id,
            fn_name: "set_position_limits",
            args: (limits.clone(),).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    gov.set_position_limits(&limits);

    let stored = read_controller_position_limits(&env, &controller_id);
    assert_eq!(stored.max_supply_positions, 7);
    assert_eq!(stored.max_borrow_positions, 6);
}

#[test]
#[should_panic(expected = "Error(Contract, #2000)")]
fn configure_market_oracle_requires_governance_oracle_role() {
    let env = Env::default();
    env.mock_all_auths();
    let (_, _, gov) = register_governance(&env);
    let stranger = Address::generate(&env);
    let asset = Address::generate(&env);

    gov.configure_market_oracle(&stranger, &asset, &sample_oracle_input(&env));
}

// With no controller set, BadFirstTolerance confirms tolerance validation runs
// before controller lookup.
#[test]
#[should_panic(expected = "Error(Contract, #207)")]
fn edit_oracle_tolerance_validates_before_any_cross_call() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, _, gov) = register_governance(&env);
    let asset = Address::generate(&env);

    gov.edit_oracle_tolerance(&admin, &asset, &0, &200);
}

#[test]
#[should_panic(expected = "Error(Contract, #201)")]
fn set_aggregator_rejects_non_contract_address() {
    let env = Env::default();
    env.mock_all_auths();
    let (_, _, gov) = register_governance(&env);

    gov.set_aggregator(&Address::generate(&env));
}

#[test]
fn set_accumulator_accepts_wallet_address() {
    let env = Env::default();
    env.mock_all_auths();
    let (_, gov_id, gov) = register_governance(&env);
    let controller_id = register_native_controller(&env, &gov_id, &gov);
    let treasury = Address::generate(&env);

    gov.set_accumulator(&treasury);

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
    let (_, _, gov) = register_governance(&env);

    gov.set_liquidity_pool_template(&BytesN::from_array(&env, &[0u8; 32]));
}

#[test]
#[should_panic(expected = "Error(Contract, #113)")]
fn edit_asset_config_rejects_bad_risk_bounds_before_any_cross_call() {
    let env = Env::default();
    env.mock_all_auths();
    let (_, _, gov) = register_governance(&env);
    let asset = Address::generate(&env);

    let cfg = controller_interface::types::AssetConfigRaw {
        loan_to_value_bps: 9_000,
        // Threshold below LTV is invalid.
        liquidation_threshold_bps: 8_000,
        liquidation_bonus_bps: 500,
        liquidation_fees_bps: 100,
        is_collateralizable: true,
        is_borrowable: true,

        is_flashloanable: true,
        flashloan_fee_bps: 9,
        borrow_cap: 0,
        supply_cap: 0,

        e_mode_categories: soroban_sdk::Vec::new(&env),
    };
    gov.edit_asset_config(&asset, &cfg);
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
    gov.propose_grant_governance_role(&admin, &grantee, &role, &salt);
    env.ledger().with_mut(|l| {
        l.sequence_number += constants::TIMELOCK_MIN_DELAY_LEDGERS;
    });
    gov.execute_grant_governance_role(&Some(admin.clone()), &grantee, &role, &salt);

    let renewed_ttl = env.as_contract(&gov_id, || env.storage().instance().get_ttl());
    assert!(
        renewed_ttl > initial_ttl,
        "instance TTL must be renewed: renewed={renewed_ttl}, initial={initial_ttl}"
    );
}
