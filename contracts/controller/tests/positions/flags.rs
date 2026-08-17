use super::*;
use crate::Controller;
use common::types::{HubConfig, SpokeAssetConfig, SpokeConfig};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env};

const SPOKE_ID: u32 = 1;
const HUB_ID: u32 = 1;

fn spoke_asset(paused: bool, frozen: bool, no_seize: bool) -> SpokeAssetConfig {
    SpokeAssetConfig {
        is_collateralizable: true,
        is_borrowable: true,
        paused,
        frozen,
        no_seize,
        loan_to_value: 9_000,
        liquidation_threshold: 9_300,
        liquidation_bonus: 300,
        liquidation_fees: 0,
        supply_cap: 0,
        borrow_cap: 0,
    }
}

fn run_gate(paused: bool, frozen: bool, freeze: FreezePolicy) {
    run_gate_with_no_seize(paused, frozen, false, freeze);
}

fn run_gate_with_no_seize(paused: bool, frozen: bool, no_seize: bool, freeze: FreezePolicy) {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    env.as_contract(&contract_id, || {
        let hub_asset = HubAssetKey {
            hub_id: 0,
            asset: Address::generate(&env),
        };
        storage::set_spoke_asset(
            &env,
            SPOKE_ID,
            &hub_asset,
            &spoke_asset(paused, frozen, no_seize),
        );
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
    run_gate(false, false, FreezePolicy::SeizureLeg);
}

// Seizure is pro-rata over an account's whole collateral set, so gating it on
// `paused` would make one paused listing block liquidation of every account
// holding it. `SeizureLeg` therefore reads only `no_seize`.

#[test]
fn paused_does_not_block_seizure() {
    run_gate_with_no_seize(true, false, false, FreezePolicy::SeizureLeg);
}

#[test]
fn frozen_does_not_block_seizure() {
    run_gate_with_no_seize(false, true, false, FreezePolicy::SeizureLeg);
}

#[test]
#[should_panic(expected = "Error(Contract, #318)")]
fn no_seize_blocks_seizure() {
    run_gate_with_no_seize(false, false, true, FreezePolicy::SeizureLeg);
}

#[test]
fn no_seize_does_not_block_entry_or_exit() {
    run_gate_with_no_seize(false, false, true, FreezePolicy::BlockOnEntry);
    run_gate_with_no_seize(false, false, true, FreezePolicy::AllowOnExit);
}

#[test]
fn missing_spoke_asset_is_noop_for_seizure() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    env.as_contract(&contract_id, || {
        let hub_asset = HubAssetKey {
            hub_id: 0,
            asset: Address::generate(&env),
        };
        let mut cache = Cache::new_view(&env);
        // A delisted asset must stay seizable, or its holders become
        // unliquidatable.
        enforce_spoke_asset_flags(
            &env,
            &mut cache,
            SPOKE_ID,
            &hub_asset,
            FreezePolicy::SeizureLeg,
        );
    });
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

fn seed_supplyable_listing(
    env: &Env,
    hub_asset: &HubAssetKey,
    collateralizable: bool,
    paused: bool,
    frozen: bool,
) {
    storage::set_hub(env, HUB_ID, &HubConfig { is_active: true });
    storage::set_spoke(
        env,
        SPOKE_ID,
        &SpokeConfig {
            is_deprecated: false,
            liquidation_target_hf_wad: 0,
            hf_for_max_bonus_wad: 0,
            liquidation_bonus_factor_bps: 0,
        },
    );
    storage::set_spoke_asset(
        env,
        SPOKE_ID,
        hub_asset,
        &SpokeAssetConfig {
            is_collateralizable: collateralizable,
            is_borrowable: true,
            paused,
            frozen,
            no_seize: false,
            loan_to_value: 9_000,
            liquidation_threshold: 9_300,
            liquidation_bonus: 300,
            liquidation_fees: 0,
            supply_cap: 0,
            borrow_cap: 0,
        },
    );
}

fn run_require_can_supply(collateralizable: bool, paused: bool, frozen: bool) {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    env.as_contract(&contract_id, || {
        let hub_asset = HubAssetKey {
            hub_id: HUB_ID,
            asset: Address::generate(&env),
        };
        seed_supplyable_listing(&env, &hub_asset, collateralizable, paused, frozen);
        let mut cache = Cache::new_view(&env);
        require_can_supply(&env, &mut cache, SPOKE_ID, &hub_asset);
    });
}

#[test]
fn require_can_supply_allows_clean_collateral() {
    run_require_can_supply(true, false, false);
}

#[test]
#[should_panic(expected = "Error(Contract, #316)")]
fn require_can_supply_blocks_frozen() {
    run_require_can_supply(true, false, true);
}

#[test]
#[should_panic(expected = "Error(Contract, #315)")]
fn require_can_supply_blocks_paused() {
    run_require_can_supply(true, true, false);
}

#[test]
#[should_panic(expected = "Error(Contract, #104)")]
fn require_can_supply_blocks_non_collateralizable() {
    run_require_can_supply(false, false, false);
}

fn seed_borrowable_listing(env: &Env, hub_asset: &HubAssetKey, borrowable: bool) {
    storage::set_hub(env, HUB_ID, &HubConfig { is_active: true });
    storage::set_spoke(
        env,
        SPOKE_ID,
        &SpokeConfig {
            is_deprecated: false,
            liquidation_target_hf_wad: 0,
            hf_for_max_bonus_wad: 0,
            liquidation_bonus_factor_bps: 0,
        },
    );
    storage::set_spoke_asset(
        env,
        SPOKE_ID,
        hub_asset,
        &SpokeAssetConfig {
            is_collateralizable: true,
            is_borrowable: borrowable,
            paused: false,
            frozen: false,
            no_seize: false,
            loan_to_value: 9_000,
            liquidation_threshold: 9_300,
            liquidation_bonus: 300,
            liquidation_fees: 0,
            supply_cap: 0,
            borrow_cap: 0,
        },
    );
}

#[test]
fn require_can_borrow_allows_borrowable_asset() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    env.as_contract(&contract_id, || {
        let hub_asset = HubAssetKey {
            hub_id: HUB_ID,
            asset: Address::generate(&env),
        };
        seed_borrowable_listing(&env, &hub_asset, true);
        let mut cache = Cache::new_view(&env);
        require_can_borrow(&env, &mut cache, SPOKE_ID, &hub_asset);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #107)")]
fn require_can_borrow_blocks_non_borrowable() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    env.as_contract(&contract_id, || {
        let hub_asset = HubAssetKey {
            hub_id: HUB_ID,
            asset: Address::generate(&env),
        };
        seed_borrowable_listing(&env, &hub_asset, false);
        let mut cache = Cache::new_view(&env);
        require_can_borrow(&env, &mut cache, SPOKE_ID, &hub_asset);
    });
}

#[test]
fn persist_account_positions_writes_both_sides() {
    use common::constants::RAY;
    use common::types::{Account, AccountMeta, AccountPositionRaw, DebtPositionRaw, PositionMode};
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    env.as_contract(&contract_id, || {
        let hub_asset = HubAssetKey {
            hub_id: HUB_ID,
            asset: Address::generate(&env),
        };
        storage::set_account_meta(
            &env,
            1,
            &AccountMeta {
                spoke_id: SPOKE_ID,
                mode: PositionMode::Normal,
            },
        );
        let mut supply = soroban_sdk::Map::new(&env);
        supply.set(
            hub_asset.clone(),
            AccountPositionRaw {
                scaled_amount: RAY,
                liquidation_threshold: 8_000,
                liquidation_bonus: 500,
                loan_to_value: 7_500,
                liquidation_fees: 100,
            },
        );
        let mut debt = soroban_sdk::Map::new(&env);
        debt.set(hub_asset.clone(), DebtPositionRaw { scaled_amount: RAY });
        let account = Account {
            owner: Address::generate(&env),
            spoke_id: SPOKE_ID,
            mode: PositionMode::Normal,
            supply_positions: supply,
            borrow_positions: debt,
        };
        persist_account_positions(&env, 1, &account, PositionSides::BOTH, false);
        assert_eq!(
            storage::get_supply_positions(&env, 1)
                .get(hub_asset.clone())
                .unwrap()
                .scaled_amount,
            RAY
        );
        assert_eq!(
            storage::get_debt_positions(&env, 1)
                .get(hub_asset)
                .unwrap()
                .scaled_amount,
            RAY
        );
    });
}

#[test]
fn persist_account_positions_removes_empty_account() {
    use common::types::{Account, AccountMeta, PositionMode};
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    // `cleanup_if_empty` burns the account's position NFT, so a real one must be
    // registered and minted first.
    let nft = env.register(
        position_nft::PositionNft,
        (
            contract_id.clone(),
            soroban_sdk::String::from_str(&env, "uri"),
            soroban_sdk::String::from_str(&env, "Position"),
            soroban_sdk::String::from_str(&env, "POS"),
        ),
    );
    let owner = Address::generate(&env);
    let account_id = u64::from(position_nft::PositionNftClient::new(&env, &nft).mint(&owner));
    env.as_contract(&contract_id, || {
        storage::set_position_nft(&env, &nft);
        storage::set_account_meta(
            &env,
            account_id,
            &AccountMeta {
                spoke_id: SPOKE_ID,
                mode: PositionMode::Normal,
            },
        );
        let account = Account {
            owner,
            spoke_id: SPOKE_ID,
            mode: PositionMode::Normal,
            supply_positions: soroban_sdk::Map::new(&env),
            borrow_positions: soroban_sdk::Map::new(&env),
        };
        persist_account_positions(&env, account_id, &account, PositionSides::BOTH, true);
        assert!(storage::try_get_account_meta(&env, account_id).is_none());
    });
}

#[test]
#[should_panic(expected = "Error(Auth, InvalidAction)")]
fn require_position_caller_without_auth_panics() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    env.as_contract(&contract_id, || {
        require_position_caller(&env, &Address::generate(&env));
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #43)")]
fn require_can_supply_blocks_inactive_hub() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    env.as_contract(&contract_id, || {
        let hub_asset = HubAssetKey {
            hub_id: HUB_ID,
            asset: Address::generate(&env),
        };
        // Spoke/listing present, hub never activated.
        storage::set_spoke(
            &env,
            SPOKE_ID,
            &SpokeConfig {
                is_deprecated: false,
                liquidation_target_hf_wad: 0,
                hf_for_max_bonus_wad: 0,
                liquidation_bonus_factor_bps: 0,
            },
        );
        storage::set_spoke_asset(
            &env,
            SPOKE_ID,
            &hub_asset,
            &spoke_asset(false, false, false),
        );
        let mut cache = Cache::new_view(&env);
        require_can_supply(&env, &mut cache, SPOKE_ID, &hub_asset);
    });
}
