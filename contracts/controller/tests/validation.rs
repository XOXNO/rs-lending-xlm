use super::*;
use crate::Controller;
use common::types::{
    Account, AccountPositionRaw, AccountPositionType, DebtPositionRaw, HubAssetKey, PositionLimits,
    PositionMode,
};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env, Vec};

fn new_controller(env: &Env) -> Address {
    let admin = Address::generate(env);
    env.register(Controller, (admin,))
}

fn hub(asset: &Address) -> HubAssetKey {
    HubAssetKey {
        hub_id: 0,
        asset: asset.clone(),
    }
}

fn account_with(env: &Env, supply: Option<&Address>, borrow: Option<&Address>) -> Account {
    let mut supply_positions = Map::new(env);
    if let Some(asset) = supply {
        supply_positions.set(
            hub(asset),
            AccountPositionRaw {
                scaled_amount: 1,
                liquidation_threshold: 0,
                liquidation_bonus: 0,
                loan_to_value: 0,
                liquidation_fees: 0,
            },
        );
    }
    let mut borrow_positions = Map::new(env);
    if let Some(asset) = borrow {
        borrow_positions.set(hub(asset), DebtPositionRaw { scaled_amount: 1 });
    }
    Account {
        owner: Address::generate(env),
        supply_positions,
        borrow_positions,
        spoke_id: 0,
        mode: PositionMode::Normal,
    }
}

fn with_limits(env: &Env, contract: &Address, max_supply: u32, max_borrow: u32, f: impl FnOnce()) {
    env.as_contract(contract, || {
        storage::set_position_limits(
            env,
            &PositionLimits {
                max_supply_positions: max_supply,
                max_borrow_positions: max_borrow,
            },
        );
        f();
    });
}

#[test]
fn test_validate_bulk_position_limits_dedupes_duplicate_assets() {
    let env = Env::default();
    let contract = new_controller(&env);
    let asset = Address::generate(&env);
    let account = account_with(&env, None, None);
    let aggregated = Vec::from_array(&env, [(hub(&asset), 100i128), (hub(&asset), 200i128)]);
    with_limits(&env, &contract, 1, 1, || {
        validate_bulk_position_limits(&env, &account, AccountPositionType::Deposit, &aggregated);
    });
}

#[test]
fn test_validate_bulk_position_limits_deposit_at_cap_with_existing_passes() {
    let env = Env::default();
    let contract = new_controller(&env);
    let existing = Address::generate(&env);
    let fresh = Address::generate(&env);
    let account = account_with(&env, Some(&existing), None);

    let aggregated = Vec::from_array(&env, [(hub(&existing), 100i128), (hub(&fresh), 100i128)]);
    with_limits(&env, &contract, 2, 0, || {
        validate_bulk_position_limits(&env, &account, AccountPositionType::Deposit, &aggregated);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #109)")]
fn test_validate_bulk_position_limits_deposit_over_cap_panics() {
    let env = Env::default();
    let contract = new_controller(&env);
    let existing = Address::generate(&env);
    let a = Address::generate(&env);
    let b = Address::generate(&env);
    let account = account_with(&env, Some(&existing), None);

    let aggregated = Vec::from_array(&env, [(hub(&a), 100i128), (hub(&b), 100i128)]);
    with_limits(&env, &contract, 2, 0, || {
        validate_bulk_position_limits(&env, &account, AccountPositionType::Deposit, &aggregated);
    });
}

#[test]
fn test_validate_bulk_position_limits_borrow_at_cap_with_existing_passes() {
    let env = Env::default();
    let contract = new_controller(&env);
    let existing = Address::generate(&env);
    let account = account_with(&env, None, Some(&existing));

    let aggregated = Vec::from_array(&env, [(hub(&existing), 100i128)]);
    with_limits(&env, &contract, 0, 1, || {
        validate_bulk_position_limits(&env, &account, AccountPositionType::Borrow, &aggregated);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #109)")]
fn test_validate_bulk_position_limits_borrow_over_cap_panics() {
    let env = Env::default();
    let contract = new_controller(&env);
    let a = Address::generate(&env);
    let b = Address::generate(&env);
    let account = account_with(&env, None, None);

    let aggregated = Vec::from_array(&env, [(hub(&a), 100i128), (hub(&b), 100i128)]);
    with_limits(&env, &contract, 0, 1, || {
        validate_bulk_position_limits(&env, &account, AccountPositionType::Borrow, &aggregated);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #400)")]
fn require_not_flash_loaning_rejects_ongoing() {
    let env = Env::default();
    let contract = new_controller(&env);
    env.as_contract(&contract, || {
        storage::set_flash_loan_ongoing(&env, true);
        require_not_flash_loaning(&env);
    });
}

#[test]
fn require_not_flash_loaning_passes_when_idle() {
    let env = Env::default();
    let contract = new_controller(&env);
    env.as_contract(&contract, || {
        require_not_flash_loaning(&env);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #27)")]
fn require_post_pool_risk_gates_with_debt_needs_prices() {
    let env = Env::default();
    let contract = new_controller(&env);
    let borrow = Address::generate(&env);
    let account = account_with(&env, None, Some(&borrow));
    env.as_contract(&contract, || {
        let mut cache = crate::context::Context::new_view(&env);
        require_post_pool_risk_gates(&env, &mut cache, &account);
    });
}

#[test]
fn test_validate_bulk_position_limits_empty_aggregated_is_noop_at_cap() {
    let env = Env::default();
    let contract = new_controller(&env);
    let existing = Address::generate(&env);
    let account = account_with(&env, Some(&existing), None);

    let aggregated: Vec<(HubAssetKey, i128)> = Vec::new(&env);
    with_limits(&env, &contract, 1, 1, || {
        validate_bulk_position_limits(&env, &account, AccountPositionType::Deposit, &aggregated);
    });
}

/// GH-16. Governance lowered the limit below the account's count; topping up
/// a held asset opens no slot and passes.
#[test]
fn test_validate_bulk_position_limits_topup_over_cap_passes() {
    let env = Env::default();
    let contract = new_controller(&env);
    let held_a = Address::generate(&env);
    let held_b = Address::generate(&env);
    let mut account = account_with(&env, Some(&held_a), None);
    account.supply_positions.set(
        hub(&held_b),
        AccountPositionRaw {
            scaled_amount: 1,
            liquidation_threshold: 0,
            liquidation_bonus: 0,
            loan_to_value: 0,
            liquidation_fees: 0,
        },
    );

    let aggregated = Vec::from_array(&env, [(hub(&held_a), 100i128)]);
    with_limits(&env, &contract, 1, 0, || {
        validate_bulk_position_limits(&env, &account, AccountPositionType::Deposit, &aggregated);
    });
}

/// GH-16. The same over-limit account still cannot open a new slot.
#[test]
#[should_panic(expected = "Error(Contract, #109)")]
fn test_validate_bulk_position_limits_new_slot_over_cap_still_panics() {
    let env = Env::default();
    let contract = new_controller(&env);
    let held_a = Address::generate(&env);
    let held_b = Address::generate(&env);
    let fresh = Address::generate(&env);
    let mut account = account_with(&env, Some(&held_a), None);
    account.supply_positions.set(
        hub(&held_b),
        AccountPositionRaw {
            scaled_amount: 1,
            liquidation_threshold: 0,
            liquidation_bonus: 0,
            loan_to_value: 0,
            liquidation_fees: 0,
        },
    );

    let aggregated = Vec::from_array(&env, [(hub(&fresh), 100i128)]);
    with_limits(&env, &contract, 1, 0, || {
        validate_bulk_position_limits(&env, &account, AccountPositionType::Deposit, &aggregated);
    });
}
