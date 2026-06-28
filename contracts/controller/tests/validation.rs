use super::*;
use crate::Controller;
use common::types::pool::{AccountPositionRaw, DebtPositionRaw, HubAssetKey};
use controller_interface::types::{Account, AccountPositionType, PositionLimits, PositionMode};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env, Vec};

fn new_controller(env: &Env) -> Address {
    let admin = Address::generate(env);
    env.register(Controller, (admin,))
}

/// Hub-0 key for an asset; Phase 0 keys every position on hub 0.
fn hub(asset: &Address) -> HubAssetKey {
    HubAssetKey {
        hub_id: 0,
        asset: asset.clone(),
    }
}

/// Account holding at most one existing supply and/or borrow position. Values
/// are placeholders; the guard reads only key presence.
fn account_with(env: &Env, supply: Option<&Address>, borrow: Option<&Address>) -> Account {
    let mut supply_positions = Map::new(env);
    if let Some(asset) = supply {
        supply_positions.set(
            hub(asset),
            AccountPositionRaw {
                scaled_amount_ray: 1,
                liquidation_threshold_bps: 0,
                liquidation_bonus_bps: 0,
                loan_to_value_bps: 0,
            },
        );
    }
    let mut borrow_positions = Map::new(env);
    if let Some(asset) = borrow {
        borrow_positions.set(
            hub(asset),
            DebtPositionRaw {
                scaled_amount_ray: 1,
            },
        );
    }
    Account {
        owner: Address::generate(env),
        supply_positions,
        borrow_positions,
        e_mode_category_id: 0,
        mode: PositionMode::Normal,
    }
}

/// Writes the limits and runs `f` inside the controller's storage context;
/// both the setter and the guard read instance storage.
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
    // Same asset twice is one new position (1 <= cap 2).
    let aggregated = Vec::from_array(&env, [(hub(&asset), 100i128), (hub(&asset), 200i128)]);
    with_limits(&env, &contract, 2, 2, || {
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
    // `existing` is already supplied (not new); `fresh` is the 2nd -> 2 == cap.
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
    // 1 existing + 2 new = 3 > cap 2.
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
    // Re-borrowing an existing asset adds no new position (1 == cap 1).
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
    // 2 new borrows > cap 1; exercises the Borrow branch.
    let aggregated = Vec::from_array(&env, [(hub(&a), 100i128), (hub(&b), 100i128)]);
    with_limits(&env, &contract, 0, 1, || {
        validate_bulk_position_limits(&env, &account, AccountPositionType::Borrow, &aggregated);
    });
}

#[test]
fn test_validate_bulk_position_limits_empty_aggregated_is_noop_at_cap() {
    let env = Env::default();
    let contract = new_controller(&env);
    let existing = Address::generate(&env);
    let account = account_with(&env, Some(&existing), None);
    // No new positions; current count (1) == cap (1) still passes.
    let aggregated: Vec<(HubAssetKey, i128)> = Vec::new(&env);
    with_limits(&env, &contract, 1, 1, || {
        validate_bulk_position_limits(&env, &account, AccountPositionType::Deposit, &aggregated);
    });
}
