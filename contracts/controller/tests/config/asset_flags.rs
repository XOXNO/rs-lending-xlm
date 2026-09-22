use super::*;
use common::types::{
    HubAssetKey, MarketParamsRaw, PoolStateRaw, PoolSyncData, SpokeAssetConfig, SpokeConfig,
};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{contract, contractimpl, Address, Env};

use crate::constants::RAY;
use crate::storage;
use crate::Controller;

#[contract]
struct SyncPool;

#[contractimpl]
impl SyncPool {
    pub fn get_sync_data(_env: Env, hub_asset: HubAssetKey) -> PoolSyncData {
        PoolSyncData {
            params: MarketParamsRaw {
                max_borrow_rate: 0,
                base_borrow_rate: 0,
                slope1: 0,
                slope2: 0,
                slope3: 0,
                mid_utilization: 0,
                optimal_utilization: 0,
                max_utilization: 0,
                reserve_factor: 0,
                is_flashloanable: false,
                flashloan_fee: 0,
                asset_id: hub_asset.asset,
                asset_decimals: 7,
            },
            state: PoolStateRaw {
                supplied: 0,
                borrowed: 0,
                revenue: 0,
                borrow_index: RAY,
                supply_index: RAY,
                last_timestamp: 0,
                cash: 0,
            },
        }
    }
}

fn hub(asset: &Address) -> HubAssetKey {
    HubAssetKey {
        hub_id: 0,
        asset: asset.clone(),
    }
}

/// Seeds the spoke and pool an add or edit reads before it writes the listing.
fn seed_spoke_and_pool(env: &Env, spoke_id: u32) {
    let pool = env.register(SyncPool, ());
    storage::set_pool(env, &pool);
    storage::set_spoke(
        env,
        spoke_id,
        &SpokeConfig {
            is_deprecated: false,
            liquidation_target_hf_wad: 0,
            hf_for_max_bonus_wad: 0,
            liquidation_bonus_factor_bps: 0,
        },
    );
}

fn listing_args(
    spoke_id: u32,
    asset: &Address,
    paused: bool,
    frozen: bool,
    no_seize: bool,
) -> SpokeAssetArgs {
    SpokeAssetArgs {
        hub_id: 0,
        asset: asset.clone(),
        spoke_id,
        can_collateral: true,
        can_borrow: true,
        paused,
        frozen,
        no_seize,
        ltv: 8_000,
        threshold: 8_500,
        bonus: 500,
        liquidation_fees: 0,
        supply_cap: 2_000_000,
        borrow_cap: 1_000_000,
    }
}

fn epoch(env: &Env, asset: &Address) -> u64 {
    storage::get_spoke_flags_epoch(env, 1, &hub(asset))
}

fn listing(paused: bool, frozen: bool, no_seize: bool) -> SpokeAssetConfig {
    SpokeAssetConfig {
        is_collateralizable: true,
        is_borrowable: true,
        paused,
        frozen,
        no_seize,
        loan_to_value: 8_000,
        liquidation_threshold: 8_500,
        liquidation_bonus: 500,
        liquidation_fees: 0,
        supply_cap: 1_000_000,
        borrow_cap: 1_000_000,
    }
}

fn seed_listing(
    env: &Env,
    spoke_id: u32,
    asset: &Address,
    paused: bool,
    frozen: bool,
    no_seize: bool,
) {
    storage::set_spoke_asset(
        env,
        spoke_id,
        &hub(asset),
        &listing(paused, frozen, no_seize),
    );
}

#[test]
fn set_spoke_asset_flags_tightens_pause_and_freeze() {
    let env = Env::default();
    let contract = env.register(Controller, (Address::generate(&env),));
    let asset = Address::generate(&env);

    env.as_contract(&contract, || {
        seed_listing(&env, 1, &asset, false, false, false);

        set_spoke_asset_flags(&env, 1, hub(&asset), true, false, false);
        let after_pause = storage::get_spoke_asset(&env, 1, &hub(&asset)).unwrap();
        assert!(after_pause.paused);
        assert!(!after_pause.frozen);

        set_spoke_asset_flags(&env, 1, hub(&asset), true, true, false);
        let after_both = storage::get_spoke_asset(&env, 1, &hub(&asset)).unwrap();
        assert!(after_both.paused);
        assert!(after_both.frozen);
        // Risk params must be untouched by the flags-only path.
        assert_eq!(after_both.loan_to_value, 8_000);
        assert_eq!(after_both.supply_cap, 1_000_000);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #317)")]
fn set_spoke_asset_flags_rejects_unpause() {
    let env = Env::default();
    let contract = env.register(Controller, (Address::generate(&env),));
    let asset = Address::generate(&env);

    env.as_contract(&contract, || {
        seed_listing(&env, 1, &asset, true, false, false);
        set_spoke_asset_flags(&env, 1, hub(&asset), false, false, false);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #317)")]
fn set_spoke_asset_flags_rejects_unfreeze() {
    let env = Env::default();
    let contract = env.register(Controller, (Address::generate(&env),));
    let asset = Address::generate(&env);

    env.as_contract(&contract, || {
        seed_listing(&env, 1, &asset, true, true, false);
        // Keep pause; clear freeze only — still a relaxation.
        set_spoke_asset_flags(&env, 1, hub(&asset), true, false, false);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #307)")]
fn set_spoke_asset_flags_rejects_unknown_listing() {
    let env = Env::default();
    let contract = env.register(Controller, (Address::generate(&env),));
    let asset = Address::generate(&env);

    env.as_contract(&contract, || {
        set_spoke_asset_flags(&env, 1, hub(&asset), true, false, false);
    });
}

#[test]
fn set_spoke_asset_flags_tightens_no_seize_independently() {
    let env = Env::default();
    let contract = env.register(Controller, (Address::generate(&env),));
    let asset = Address::generate(&env);

    env.as_contract(&contract, || {
        seed_listing(&env, 1, &asset, false, false, false);

        set_spoke_asset_flags(&env, 1, hub(&asset), false, false, true);
        let after = storage::get_spoke_asset(&env, 1, &hub(&asset)).unwrap();
        assert!(after.no_seize, "guardian must be able to halt seizure");
        assert!(
            !after.paused && !after.frozen,
            "halting seizure must not pause or freeze the listing"
        );
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #317)")]
fn set_spoke_asset_flags_rejects_clearing_no_seize() {
    let env = Env::default();
    let contract = env.register(Controller, (Address::generate(&env),));
    let asset = Address::generate(&env);

    env.as_contract(&contract, || {
        seed_listing(&env, 1, &asset, false, false, true);
        // The guardian ratchet is one-way for every flag, `no_seize` included:
        // reopening seizure stays timelocked through `relax_spoke_asset_flags`.
        set_spoke_asset_flags(&env, 1, hub(&asset), false, false, false);
    });
}

#[test]
fn flag_ratchet_allows_idempotent_tighten() {
    let env = Env::default();
    let contract = env.register(Controller, (Address::generate(&env),));
    let asset = Address::generate(&env);

    env.as_contract(&contract, || {
        seed_listing(&env, 1, &asset, true, true, false);
        // Re-asserting the same flags is not a relaxation.
        set_spoke_asset_flags(&env, 1, hub(&asset), true, true, false);
        let cfg = storage::get_spoke_asset(&env, 1, &hub(&asset)).unwrap();
        assert!(cfg.paused && cfg.frozen);
    });
}

#[test]
fn edit_can_tighten_flags_and_advances_the_epoch() {
    let env = Env::default();
    let contract = env.register(Controller, (Address::generate(&env),));
    let asset = Address::generate(&env);

    env.as_contract(&contract, || {
        seed_spoke_and_pool(&env, 1);
        seed_listing(&env, 1, &asset, false, false, false);

        edit_asset_in_spoke(&env, &listing_args(1, &asset, false, true, false));

        let cfg = storage::get_spoke_asset(&env, 1, &hub(&asset)).unwrap();
        assert!(cfg.frozen && !cfg.paused && !cfg.no_seize);
        assert_eq!(cfg.supply_cap, 2_000_000, "the edit itself must land");
        assert_eq!(epoch(&env, &asset), 1);
    });
}

#[test]
fn edit_keeping_the_flags_leaves_the_epoch() {
    let env = Env::default();
    let contract = env.register(Controller, (Address::generate(&env),));
    let asset = Address::generate(&env);

    env.as_contract(&contract, || {
        seed_spoke_and_pool(&env, 1);
        seed_listing(&env, 1, &asset, true, true, true);

        edit_asset_in_spoke(&env, &listing_args(1, &asset, true, true, true));

        let cfg = storage::get_spoke_asset(&env, 1, &hub(&asset)).unwrap();
        assert_eq!(cfg.supply_cap, 2_000_000);
        assert_eq!(epoch(&env, &asset), 0);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #317)")]
fn edit_cannot_clear_a_flag() {
    let env = Env::default();
    let contract = env.register(Controller, (Address::generate(&env),));
    let asset = Address::generate(&env);

    env.as_contract(&contract, || {
        seed_spoke_and_pool(&env, 1);
        seed_listing(&env, 1, &asset, false, true, false);
        edit_asset_in_spoke(&env, &listing_args(1, &asset, false, false, false));
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #317)")]
fn edit_cannot_clear_one_flag_while_raising_another() {
    let env = Env::default();
    let contract = env.register(Controller, (Address::generate(&env),));
    let asset = Address::generate(&env);

    env.as_contract(&contract, || {
        seed_spoke_and_pool(&env, 1);
        seed_listing(&env, 1, &asset, false, false, true);
        edit_asset_in_spoke(&env, &listing_args(1, &asset, true, true, false));
    });
}

#[test]
fn guardian_flag_writes_advance_the_epoch_even_when_idempotent() {
    let env = Env::default();
    let contract = env.register(Controller, (Address::generate(&env),));
    let asset = Address::generate(&env);

    env.as_contract(&contract, || {
        seed_listing(&env, 1, &asset, false, false, false);
        assert_eq!(epoch(&env, &asset), 0);

        set_spoke_asset_flags(&env, 1, hub(&asset), false, true, false);
        assert_eq!(epoch(&env, &asset), 1);

        set_spoke_asset_flags(&env, 1, hub(&asset), false, true, false);
        assert_eq!(epoch(&env, &asset), 2);
    });
}

#[test]
fn relax_with_the_current_epoch_clears_flags() {
    let env = Env::default();
    let contract = env.register(Controller, (Address::generate(&env),));
    let asset = Address::generate(&env);

    env.as_contract(&contract, || {
        seed_listing(&env, 1, &asset, false, false, false);
        set_spoke_asset_flags(&env, 1, hub(&asset), true, true, true);

        relax_spoke_asset_flags(&env, 1, hub(&asset), 1, false, true, false);

        let cfg = storage::get_spoke_asset(&env, 1, &hub(&asset)).unwrap();
        assert!(!cfg.paused && cfg.frozen && !cfg.no_seize);
        assert_eq!(cfg.loan_to_value, 8_000);
        assert_eq!(cfg.supply_cap, 1_000_000);
        assert_eq!(epoch(&env, &asset), 2);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #319)")]
fn relax_with_a_stale_epoch_reverts() {
    let env = Env::default();
    let contract = env.register(Controller, (Address::generate(&env),));
    let asset = Address::generate(&env);

    env.as_contract(&contract, || {
        seed_listing(&env, 1, &asset, false, false, false);
        set_spoke_asset_flags(&env, 1, hub(&asset), false, true, false);
        relax_spoke_asset_flags(&env, 1, hub(&asset), 0, false, false, false);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #319)")]
fn relax_from_before_a_flag_cycle_reverts_although_the_flags_match_again() {
    let env = Env::default();
    let contract = env.register(Controller, (Address::generate(&env),));
    let asset = Address::generate(&env);

    env.as_contract(&contract, || {
        seed_listing(&env, 1, &asset, false, false, false);
        set_spoke_asset_flags(&env, 1, hub(&asset), false, true, false);
        let proposed_at = epoch(&env, &asset);

        relax_spoke_asset_flags(&env, 1, hub(&asset), proposed_at, false, false, false);
        set_spoke_asset_flags(&env, 1, hub(&asset), false, true, false);
        assert!(
            storage::get_spoke_asset(&env, 1, &hub(&asset))
                .unwrap()
                .frozen
        );

        relax_spoke_asset_flags(&env, 1, hub(&asset), proposed_at, false, false, false);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #307)")]
fn relax_rejects_unknown_listing() {
    let env = Env::default();
    let contract = env.register(Controller, (Address::generate(&env),));
    let asset = Address::generate(&env);

    env.as_contract(&contract, || {
        relax_spoke_asset_flags(&env, 1, hub(&asset), 0, false, false, false);
    });
}

#[test]
fn relisting_never_reuses_a_flags_epoch() {
    let env = Env::default();
    let contract = env.register(Controller, (Address::generate(&env),));
    let asset = Address::generate(&env);

    env.as_contract(&contract, || {
        seed_spoke_and_pool(&env, 1);
        add_asset_to_spoke(&env, &listing_args(1, &asset, false, false, false));
        assert_eq!(epoch(&env, &asset), 1);
        set_spoke_asset_flags(&env, 1, hub(&asset), false, true, false);

        remove_asset_from_spoke(&env, hub(&asset), 1);
        assert_eq!(epoch(&env, &asset), 2, "removal keeps the epoch");

        add_asset_to_spoke(&env, &listing_args(1, &asset, false, true, false));
        assert_eq!(epoch(&env, &asset), 3);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #317)")]
fn edit_cannot_clear_paused_while_raising_frozen() {
    let env = Env::default();
    let contract = env.register(Controller, (Address::generate(&env),));
    let asset = Address::generate(&env);

    env.as_contract(&contract, || {
        seed_spoke_and_pool(&env, 1);
        seed_listing(&env, 1, &asset, true, false, false);
        edit_asset_in_spoke(&env, &listing_args(1, &asset, false, true, false));
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #319)")]
fn relax_with_a_future_epoch_reverts() {
    let env = Env::default();
    let contract = env.register(Controller, (Address::generate(&env),));
    let asset = Address::generate(&env);

    env.as_contract(&contract, || {
        seed_listing(&env, 1, &asset, false, false, false);
        set_spoke_asset_flags(&env, 1, hub(&asset), false, true, false);
        let live = epoch(&env, &asset);
        relax_spoke_asset_flags(&env, 1, hub(&asset), live + 1, false, false, false);
    });
}

#[test]
fn edit_tightening_any_single_flag_advances_the_epoch() {
    for (paused, frozen, no_seize) in [
        (true, false, false),
        (false, true, false),
        (false, false, true),
    ] {
        let env = Env::default();
        let contract = env.register(Controller, (Address::generate(&env),));
        let asset = Address::generate(&env);

        env.as_contract(&contract, || {
            seed_spoke_and_pool(&env, 1);
            seed_listing(&env, 1, &asset, false, false, false);

            edit_asset_in_spoke(&env, &listing_args(1, &asset, paused, frozen, no_seize));

            let cfg = storage::get_spoke_asset(&env, 1, &hub(&asset)).unwrap();
            assert_eq!(
                (cfg.paused, cfg.frozen, cfg.no_seize),
                (paused, frozen, no_seize)
            );
            assert_eq!(epoch(&env, &asset), 1);
        });
    }
}

#[test]
fn relax_can_raise_one_flag_while_clearing_another() {
    let env = Env::default();
    let contract = env.register(Controller, (Address::generate(&env),));
    let asset = Address::generate(&env);

    env.as_contract(&contract, || {
        seed_listing(&env, 1, &asset, true, false, false);

        relax_spoke_asset_flags(&env, 1, hub(&asset), 0, false, true, false);

        let cfg = storage::get_spoke_asset(&env, 1, &hub(&asset)).unwrap();
        assert_eq!((cfg.paused, cfg.frozen, cfg.no_seize), (false, true, false));
        assert_eq!(epoch(&env, &asset), 1);
    });
}
