use super::*;
use crate::Controller;
use common::types::SpokeAssetConfig;
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
    }
}

fn run_gate(paused: bool, frozen: bool, freeze: FreezePolicy) {
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
        enforce_spoke_asset_flags(&env, &mut cache, SPOKE_ID, &hub_asset, freeze);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #315)")]
fn paused_blocks_supply_borrow() {
    run_gate(true, false, FreezePolicy::BlockOnEntry);
}

#[test]
#[should_panic(expected = "Error(Contract, #315)")]
fn paused_blocks_withdraw_repay() {
    run_gate(true, false, FreezePolicy::AllowOnExit);
}

#[test]
#[should_panic(expected = "Error(Contract, #316)")]
fn frozen_blocks_supply_borrow() {
    run_gate(false, true, FreezePolicy::BlockOnEntry);
}

#[test]
fn frozen_allows_withdraw_repay() {
    run_gate(false, true, FreezePolicy::AllowOnExit);
}

#[test]
fn clean_asset_allows_all_verbs() {
    run_gate(false, false, FreezePolicy::BlockOnEntry);
    run_gate(false, false, FreezePolicy::AllowOnExit);
}

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

        enforce_spoke_asset_flags(
            &env,
            &mut cache,
            SPOKE_ID,
            &hub_asset,
            FreezePolicy::BlockOnEntry,
        );
    });
}
