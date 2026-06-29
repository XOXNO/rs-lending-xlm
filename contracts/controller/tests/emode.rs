use super::*;
use crate::Controller;
use controller_interface::types::{Account, MarketOracleConfigOption, PositionMode};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Map};

fn spoke_asset_config(ltv_bps: u32) -> SpokeAssetConfig {
    SpokeAssetConfig {
        is_collateralizable: true,
        is_borrowable: true,
        paused: false,
        frozen: false,
        loan_to_value: ltv_bps,
        liquidation_threshold: ltv_bps + 500,
        liquidation_bonus: 300,
        liquidation_fees: 0,
        supply_cap: 0,
        borrow_cap: 0,
        oracle_override: MarketOracleConfigOption::None,
    }
}

fn hub(asset: &Address) -> HubAssetKey {
    HubAssetKey {
        hub_id: 0,
        asset: asset.clone(),
    }
}

fn new_controller(env: &Env) -> Address {
    let admin = Address::generate(env);
    env.register(Controller, (admin,))
}

// The general spoke 0 is every listed asset's base config.
#[test]
fn effective_asset_config_reads_spoke_zero_base() {
    let env = Env::default();
    let contract = new_controller(&env);
    let asset = Address::generate(&env);
    env.as_contract(&contract, || {
        storage::set_spoke_asset(&env, 0, &hub(&asset), &spoke_asset_config(9_000));
        let cfg = effective_asset_config(&env, 0, &hub(&asset));
        assert_eq!(cfg.loan_to_value.raw() as u32, 9_000);
        assert!(cfg.can_supply());
        assert!(cfg.can_borrow());
    });
}

// An account on a named spoke resolves risk from SpokeAsset(spoke_id) directly,
// with no overlay onto the spoke-0 base.
#[test]
fn effective_asset_config_reads_named_spoke_directly() {
    let env = Env::default();
    let contract = new_controller(&env);
    let asset = Address::generate(&env);
    env.as_contract(&contract, || {
        storage::set_spoke_asset(&env, 0, &hub(&asset), &spoke_asset_config(9_000));
        storage::set_spoke_asset(&env, 1, &hub(&asset), &spoke_asset_config(5_000));
        let cfg = effective_asset_config(&env, 1, &hub(&asset));
        assert_eq!(cfg.loan_to_value.raw() as u32, 5_000);
    });
}

// An asset not listed on the account's spoke is rejected.
#[test]
#[should_panic(expected = "Error(Contract, #1)")]
fn effective_asset_config_panics_when_unlisted_on_spoke() {
    let env = Env::default();
    let contract = new_controller(&env);
    let asset = Address::generate(&env);
    env.as_contract(&contract, || {
        storage::set_spoke_asset(&env, 0, &hub(&asset), &spoke_asset_config(9_000));
        effective_asset_config(&env, 2, &hub(&asset));
    });
}

// Grandfathering: lowering a spoke's LTV after a supply leaves the existing
// position's snapshotted LTV unchanged; only the freshly resolved config drops.
#[test]
fn lowering_spoke_ltv_keeps_existing_position_ltv() {
    let env = Env::default();
    let contract = new_controller(&env);
    let asset = Address::generate(&env);
    env.as_contract(&contract, || {
        storage::set_spoke_asset(&env, 0, &hub(&asset), &spoke_asset_config(9_000));

        // First supply snapshots LTV 9000 into the position.
        let mut account = Account {
            owner: Address::generate(&env),
            spoke_id: 0,
            mode: PositionMode::Normal,
            supply_positions: Map::new(&env),
            borrow_positions: Map::new(&env),
        };
        let cfg_9000 = effective_asset_config(&env, 0, &hub(&asset));
        let seeded = account.get_or_create_supply_position(&hub(&asset), &cfg_9000);
        account.supply_positions.set(hub(&asset), (&seeded).into());

        // Governance lowers the spoke LTV to 5000.
        storage::set_spoke_asset(&env, 0, &hub(&asset), &spoke_asset_config(5_000));
        let cfg_5000 = effective_asset_config(&env, 0, &hub(&asset));
        assert_eq!(cfg_5000.loan_to_value.raw() as u32, 5_000);

        // The existing position keeps the snapshotted 9000.
        let existing = account.get_or_create_supply_position(&hub(&asset), &cfg_5000);
        assert_eq!(existing.loan_to_value.raw() as u32, 9_000);
    });
}
