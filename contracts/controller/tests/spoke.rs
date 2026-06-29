use super::*;
use crate::storage;
use crate::Controller;
use controller_interface::types::{
    Account, MarketOracleConfigOption, PositionMode, SpokeAssetConfig,
};
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

// A spoke resolves risk from its own self-contained SpokeAsset listing.
#[test]
fn effective_asset_config_reads_listed_spoke() {
    let env = Env::default();
    let contract = new_controller(&env);
    let asset = Address::generate(&env);
    env.as_contract(&contract, || {
        storage::set_spoke_asset(&env, 1, &hub(&asset), &spoke_asset_config(9_000));
        let mut cache = Cache::new_view(&env);
        let cfg = effective_asset_config(&mut cache, 1, &hub(&asset));
        assert_eq!(cfg.loan_to_value.raw() as u32, 9_000);
        assert!(cfg.can_supply());
        assert!(cfg.can_borrow());
    });
}

// Each spoke resolves risk from its own SpokeAsset(spoke_id) listing; one spoke's
// config does not bleed into another's.
#[test]
fn effective_asset_config_reads_each_spoke_directly() {
    let env = Env::default();
    let contract = new_controller(&env);
    let asset = Address::generate(&env);
    env.as_contract(&contract, || {
        storage::set_spoke_asset(&env, 1, &hub(&asset), &spoke_asset_config(9_000));
        storage::set_spoke_asset(&env, 2, &hub(&asset), &spoke_asset_config(5_000));
        // One cache binds to one spoke per transaction, so each spoke resolves
        // through its own cache (mirroring one account = one spoke per tx).
        let mut cache_spoke_1 = Cache::new_view(&env);
        assert_eq!(
            effective_asset_config(&mut cache_spoke_1, 1, &hub(&asset)).loan_to_value.raw() as u32,
            9_000
        );
        let mut cache_spoke_2 = Cache::new_view(&env);
        assert_eq!(
            effective_asset_config(&mut cache_spoke_2, 2, &hub(&asset)).loan_to_value.raw() as u32,
            5_000
        );
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
        storage::set_spoke_asset(&env, 1, &hub(&asset), &spoke_asset_config(9_000));
        let mut cache = Cache::new_view(&env);
        effective_asset_config(&mut cache, 2, &hub(&asset));
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
        storage::set_spoke_asset(&env, 1, &hub(&asset), &spoke_asset_config(9_000));

        // First supply snapshots LTV 9000 into the position.
        let mut account = Account {
            owner: Address::generate(&env),
            spoke_id: 1,
            mode: PositionMode::Normal,
            supply_positions: Map::new(&env),
            borrow_positions: Map::new(&env),
        };
        let mut cache_before = Cache::new_view(&env);
        let cfg_9000 = effective_asset_config(&mut cache_before, 1, &hub(&asset));
        let seeded = account.get_or_create_supply_position(&hub(&asset), &cfg_9000);
        account.supply_positions.set(hub(&asset), (&seeded).into());

        // Governance lowers the spoke LTV to 5000 (a later transaction, hence a
        // fresh cache; the per-tx memo never serves a stale config).
        storage::set_spoke_asset(&env, 1, &hub(&asset), &spoke_asset_config(5_000));
        let mut cache_after = Cache::new_view(&env);
        let cfg_5000 = effective_asset_config(&mut cache_after, 1, &hub(&asset));
        assert_eq!(cfg_5000.loan_to_value.raw() as u32, 5_000);

        // The existing position keeps the snapshotted 9000.
        let existing = account.get_or_create_supply_position(&hub(&asset), &cfg_5000);
        assert_eq!(existing.loan_to_value.raw() as u32, 9_000);
    });
}
