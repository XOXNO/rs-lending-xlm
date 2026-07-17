//! Spoke asset pause/freeze gate tests.

use super::*;
use crate::Controller;
use common::types::{MarketOracleConfigOption, SpokeAssetConfig};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env};

const SPOKE_ID: u32 = 1;

fn spoke_asset(paused: bool, frozen: bool) -> SpokeAssetConfig {
    SpokeAssetConfig {
        is_collateralizable: true,
        is_borrowable: true,
        paused,
        frozen,
        loan_to_value: 9_000,
        liquidation_threshold: 9_300,
        liquidation_bonus: 300,
        liquidation_fees: 0,
        supply_cap: 0,
        borrow_cap: 0,
        oracle_override: MarketOracleConfigOption::None,
    }
}

fn run_gate(paused: bool, frozen: bool, block_when_frozen: bool) {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    env.as_contract(&contract_id, || {
        let hub_asset = HubAssetKey {
            hub_id: 0,
            asset: Address::generate(&env),
        };
        storage::set_spoke_asset(&env, SPOKE_ID, &hub_asset, &spoke_asset(paused, frozen));
        let mut cache = Cache::new_view(&env);
        enforce_spoke_asset_flags(&env, &mut cache, SPOKE_ID, &hub_asset, block_when_frozen);
    });
}

// Paused rejects new supply/borrow (block_when_frozen = true).
#[test]
#[should_panic(expected = "Error(Contract, #315)")]
fn paused_blocks_supply_borrow() {
    run_gate(true, false, true);
}

// Paused also rejects withdraw/repay (block_when_frozen = false).
#[test]
#[should_panic(expected = "Error(Contract, #315)")]
fn paused_blocks_withdraw_repay() {
    run_gate(true, false, false);
}

// Frozen rejects new supply/borrow.
#[test]
#[should_panic(expected = "Error(Contract, #316)")]
fn frozen_blocks_supply_borrow() {
    run_gate(false, true, true);
}

// Frozen allows withdraw/repay.
#[test]
fn frozen_allows_withdraw_repay() {
    run_gate(false, true, false);
}

// An unpaused, unfrozen asset passes every verb.
#[test]
fn clean_asset_allows_all_verbs() {
    run_gate(false, false, true);
    run_gate(false, false, false);
}

// No spoke-asset entry for the asset is a no-op for any flag.
#[test]
fn missing_spoke_asset_is_noop() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    env.as_contract(&contract_id, || {
        let hub_asset = HubAssetKey {
            hub_id: 0,
            asset: Address::generate(&env),
        };
        let mut cache = Cache::new_view(&env);
        // Spoke without an entry for this asset: no-op.
        enforce_spoke_asset_flags(&env, &mut cache, SPOKE_ID, &hub_asset, true);
    });
}
