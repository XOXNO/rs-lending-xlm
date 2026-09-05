//! First-pass killers for strategy auth/validation. Full swap/flash-loan legs
//! still need the iterate suite; these catch `replace fn with ()` on the gates.
extern crate std;

use crate::risk::validation::require_authorized_caller;
use crate::strategies::multiply::{process_multiply, MultiplyParams};
use crate::Controller;
use common::types::{HubAssetKey, PositionMode};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Bytes, Env};

fn hub(env: &Env) -> HubAssetKey {
    HubAssetKey {
        hub_id: 1,
        asset: Address::generate(env),
    }
}

#[test]
#[should_panic(expected = "Auth")]
fn strategy_caller_without_auth_panics() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let id = env.register(Controller, (admin,));
    env.as_contract(&id, || {
        require_authorized_caller(&env, &Address::generate(&env));
    });
}

#[test]
fn strategy_caller_with_auth_passes() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let id = env.register(Controller, (admin,));
    env.as_contract(&id, || {
        require_authorized_caller(&env, &Address::generate(&env));
    });
}

#[test]
#[should_panic(expected = "Auth")]
fn process_multiply_requires_caller_auth() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let id = env.register(Controller, (admin,));
    let caller = Address::generate(&env);
    let collateral = hub(&env);
    let debt = hub(&env);
    env.as_contract(&id, || {
        crate::governance::unpause(&env);
        let _ = process_multiply(
            &env,
            &caller,
            MultiplyParams {
                account_id: 0,
                spoke_id: 1,
                collateral: &collateral,
                debt_to_flash_loan: 1,
                debt: &debt,
                mode: PositionMode::Multiply,
                swap: &Bytes::new(&env),
                initial_payment: None,
                convert_swap: None,
            },
        );
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #7)")]
fn process_multiply_rejects_identical_assets() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let id = env.register(Controller, (admin,));
    let caller = Address::generate(&env);
    let asset = hub(&env);
    env.as_contract(&id, || {
        crate::governance::unpause(&env);
        let _ = process_multiply(
            &env,
            &caller,
            MultiplyParams {
                account_id: 0,
                spoke_id: 1,
                collateral: &asset,
                debt_to_flash_loan: 1,
                debt: &asset,
                mode: PositionMode::Multiply,
                swap: &Bytes::new(&env),
                initial_payment: None,
                convert_swap: None,
            },
        );
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #16)")]
fn process_migrate_blend_rejects_empty_request() {
    use crate::config;
    use crate::strategies::migrate_blend::{process_migrate_blend, MigrateBlendParams};
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let id = env.register(Controller, (admin,));
    let caller = Address::generate(&env);
    env.as_contract(&id, || {
        crate::governance::unpause(&env);
        let hub_id = config::spoke::create_hub(&env);
        let _ = process_migrate_blend(
            &env,
            &caller,
            MigrateBlendParams {
                account_id: 0,
                spoke_id: 1,
                hub_id,
                blend_pool: Address::generate(&env),
                collateral_assets: soroban_sdk::Vec::new(&env),
                supply_assets: soroban_sdk::Vec::new(&env),
                debt_caps: soroban_sdk::Vec::new(&env),
            },
        );
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #42)")]
fn process_migrate_blend_rejects_unapproved_pool() {
    use crate::config;
    use crate::strategies::migrate_blend::{process_migrate_blend, MigrateBlendParams};
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let id = env.register(Controller, (admin,));
    let caller = Address::generate(&env);
    env.as_contract(&id, || {
        crate::governance::unpause(&env);
        let hub_id = config::spoke::create_hub(&env);
        let mut collateral = soroban_sdk::Vec::new(&env);
        collateral.push_back(Address::generate(&env));
        let _ = process_migrate_blend(
            &env,
            &caller,
            MigrateBlendParams {
                account_id: 0,
                spoke_id: 1,
                hub_id,
                blend_pool: Address::generate(&env),
                collateral_assets: collateral,
                supply_assets: soroban_sdk::Vec::new(&env),
                debt_caps: soroban_sdk::Vec::new(&env),
            },
        );
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #30)")]
fn repay_debt_from_controller_without_pool_panics() {
    use crate::events::PositionAction;
    use crate::strategies::legs::{repay_debt_from_controller, StrategyRepay};
    use common::math::fp::Ray;
    use common::types::{Account, DebtPosition, PositionMode};
    let env = Env::default();
    let admin = Address::generate(&env);
    let id = env.register(Controller, (admin,));
    env.as_contract(&id, || {
        let mut cache = crate::context::Context::new_view(&env);
        let mut account = Account {
            owner: Address::generate(&env),
            spoke_id: 1,
            mode: PositionMode::Normal,
            supply_positions: soroban_sdk::Map::new(&env),
            borrow_positions: soroban_sdk::Map::new(&env),
        };
        let debt = hub(&env);
        let pos = DebtPosition {
            scaled_amount: Ray::from(1),
        };
        repay_debt_from_controller(
            &env,
            &mut account,
            &mut cache,
            &Address::generate(&env),
            StrategyRepay {
                debt: &debt,
                debt_available: 1,
                debt_pos: &pos,
                action: PositionAction::Repay,
            },
        );
    });
}

#[test]
#[should_panic(expected = "Error(Storage, MissingValue)")]
fn withdraw_collateral_to_controller_without_position_panics() {
    use crate::events::PositionAction;
    use crate::strategies::legs::{withdraw_collateral_to_controller, StrategyWithdraw};
    use common::math::fp::{Bps, Ray};
    use common::types::{Account, AccountPosition, PositionMode};
    let env = Env::default();
    let admin = Address::generate(&env);
    let id = env.register(Controller, (admin,));
    env.as_contract(&id, || {
        let mut cache = crate::context::Context::new_view(&env);
        let mut account = Account {
            owner: Address::generate(&env),
            spoke_id: 1,
            mode: PositionMode::Normal,
            supply_positions: soroban_sdk::Map::new(&env),
            borrow_positions: soroban_sdk::Map::new(&env),
        };
        let asset = hub(&env);
        let pos = AccountPosition {
            scaled_amount: Ray::from(1),
            liquidation_threshold: Bps::from(8_000),
            liquidation_bonus: Bps::from(500),
            loan_to_value: Bps::from(7_500),
            liquidation_fees: Bps::from(100),
        };
        let _ = withdraw_collateral_to_controller(
            &env,
            &mut account,
            &mut cache,
            StrategyWithdraw {
                hub_asset: &asset,
                amount: 1,
                position: &pos,
                action: PositionAction::Withdraw,
            },
        );
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #111)")]
fn process_multiply_rejects_normal_mode() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let id = env.register(Controller, (admin,));
    let caller = Address::generate(&env);
    let collateral = hub(&env);
    let debt = hub(&env);
    env.as_contract(&id, || {
        crate::governance::unpause(&env);
        let _ = process_multiply(
            &env,
            &caller,
            MultiplyParams {
                account_id: 0,
                spoke_id: 1,
                collateral: &collateral,
                debt_to_flash_loan: 1,
                debt: &debt,
                mode: PositionMode::Normal,
                swap: &Bytes::new(&env),
                initial_payment: None,
                convert_swap: None,
            },
        );
    });
}
