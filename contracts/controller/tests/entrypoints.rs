//! First-pass mutant killers for `lib.rs` wrappers and the `process_*` bodies
//! they call. Package tests must exercise the client so `replace fn with ()/0/1`
//! on those wrappers cannot slip through to the slow iterate suite.
extern crate std;

use super::*;
use crate::constants::{RAY, WAD};
use crate::storage;
use common::types::{
    AccountMeta, AccountPositionRaw, DebtPositionRaw, HubAssetKey, InterestRateModel,
    MarketIndexRaw, MarketParamsRaw, PoolStateRaw, PoolSyncData, PositionLimits, PositionMode,
    PriceFeedRaw, PriceKey, SpokeAssetArgs, SpokeAssetConfig, SpokeConfig,
};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{contract, contractimpl, Address, Bytes, BytesN, Env, Map, Vec};

#[contract]
struct ViewPool;

#[contractimpl]
impl ViewPool {
    pub fn get_bulk_indexes(env: Env, hub_assets: Vec<HubAssetKey>) -> Vec<MarketIndexRaw> {
        let mut out = Vec::new(&env);
        for _ in hub_assets.iter() {
            out.push_back(MarketIndexRaw {
                borrow_index: RAY,
                supply_index: RAY,
            });
        }
        out
    }

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
                asset_decimals: 0,
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

#[contract]
struct ViewAggregator;

#[contractimpl]
impl ViewAggregator {
    pub fn prices(env: Env, keys: Vec<PriceKey>) -> Map<PriceKey, PriceFeedRaw> {
        let mut out = Map::new(&env);
        for key in keys.iter() {
            out.set(
                key,
                PriceFeedRaw {
                    price_wad: WAD,
                    asset_decimals: 0,
                    timestamp: 0,
                },
            );
        }
        out
    }
}

fn setup() -> (Env, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let id = env.register(Controller, (admin,));
    ControllerClient::new(&env, &id).unpause();
    (env, id)
}

fn client<'a>(env: &'a Env, id: &Address) -> ControllerClient<'a> {
    ControllerClient::new(env, id)
}

fn hub(env: &Env) -> HubAssetKey {
    HubAssetKey {
        hub_id: 0,
        asset: Address::generate(env),
    }
}

fn one_ray_position() -> AccountPositionRaw {
    AccountPositionRaw {
        scaled_amount: RAY,
        liquidation_threshold: 8_000,
        liquidation_bonus: 500,
        loan_to_value: 7_500,
        liquidation_fees: 100,
    }
}

/// Registers a native position-nft owned by `controller` and records it in the controller's
/// instance storage. Returns the NFT contract address. Duplicated from
/// `tests/helpers/account.rs`: pinned test modules cannot import each other.
fn setup_position_nft(env: &Env, controller: &Address) -> Address {
    let nft = env.register(
        position_nft::PositionNft,
        (
            controller.clone(),
            soroban_sdk::String::from_str(env, "uri"),
            soroban_sdk::String::from_str(env, "Position"),
            soroban_sdk::String::from_str(env, "POS"),
        ),
    );
    env.as_contract(controller, || {
        storage::set_position_nft(env, &nft);
    });
    nft
}

/// Mints the position NFT to a fresh owner (yielding account id 1, the first mint against a
/// fresh NFT), then seeds live supply/debt positions for it.
fn seed_live_account(env: &Env, contract_id: &Address) -> HubAssetKey {
    let pool = env.register(ViewPool, ());
    let aggregator = env.register(ViewAggregator, ());
    let nft = setup_position_nft(env, contract_id);
    let owner = Address::generate(env);
    let account_id = u64::from(position_nft::PositionNftClient::new(env, &nft).mint(&owner));
    let key = hub(env);
    env.as_contract(contract_id, || {
        storage::set_pool(env, &pool);
        storage::set_price_aggregator(env, &aggregator);
        storage::set_account_meta(
            env,
            account_id,
            &AccountMeta {
                spoke_id: 1,
                mode: PositionMode::Normal,
            },
        );
        let mut supply = Map::new(env);
        supply.set(key.clone(), one_ray_position());
        storage::set_supply_positions(env, account_id, &supply);
        let mut debt = Map::new(env);
        debt.set(key.clone(), DebtPositionRaw { scaled_amount: RAY });
        storage::set_debt_positions(env, account_id, &debt);
    });
    key
}

#[test]
#[should_panic(expected = "Error(Contract, #16)")]
fn supply_without_spoke_panics() {
    let (env, id) = setup();
    let caller = Address::generate(&env);
    let _ = client(&env, &id).supply(&caller, &0u64, &1u32, &Vec::new(&env));
}

#[test]
#[should_panic(expected = "Error(Contract, #24)")]
fn borrow_missing_account_panics() {
    let (env, id) = setup();
    let caller = Address::generate(&env);
    client(&env, &id).borrow(&caller, &1u64, &Vec::new(&env), &None);
}

#[test]
#[should_panic(expected = "Error(Contract, #24)")]
fn withdraw_missing_account_panics() {
    let (env, id) = setup();
    let caller = Address::generate(&env);
    let _ = client(&env, &id).withdraw(&caller, &1u64, &Vec::new(&env), &None);
}

#[test]
#[should_panic(expected = "Error(Contract, #16)")]
fn repay_missing_account_panics() {
    let (env, id) = setup();
    let caller = Address::generate(&env);
    client(&env, &id).repay(&caller, &1u64, &Vec::new(&env));
}

#[test]
#[should_panic(expected = "Error(Contract, #24)")]
fn liquidate_missing_account_panics() {
    let (env, id) = setup();
    let liquidator = Address::generate(&env);
    client(&env, &id).liquidate(
        &liquidator,
        &1u64,
        &Vec::new(&env),
        &common::types::SeizeMode::Transfer,
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #24)")]
fn clean_bad_debt_missing_account_panics() {
    let (env, id) = setup();
    let caller = Address::generate(&env);
    client(&env, &id).clean_bad_debt(&caller, &1u64);
}

#[test]
#[should_panic(expected = "Error(Contract, #43)")]
fn flash_loan_without_hub_panics() {
    let (env, id) = setup();
    let caller = Address::generate(&env);
    client(&env, &id).flash_loan(
        &caller,
        &hub(&env),
        &1i128,
        &Address::generate(&env),
        &Bytes::new(&env),
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #43)")]
fn flash_position_without_hub_panics() {
    let (env, id) = setup();
    let caller = Address::generate(&env);
    let _ = client(&env, &id).flash_position(
        &caller,
        &0u64,
        &1u32,
        &PositionMode::Multiply,
        &hub(&env),
        &1i128,
        &Address::generate(&env),
        &Bytes::new(&env),
        &Vec::new(&env),
        &Vec::new(&env),
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #7)")]
fn multiply_same_assets_panics() {
    let (env, id) = setup();
    let caller = Address::generate(&env);
    let asset = hub(&env);
    let _ = client(&env, &id).multiply(
        &caller,
        &0u64,
        &1u32,
        &asset,
        &1i128,
        &asset,
        &PositionMode::Multiply,
        &Bytes::new(&env),
        &None,
        &None,
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #43)")]
fn swap_debt_missing_account_panics() {
    let (env, id) = setup();
    let caller = Address::generate(&env);
    let a = hub(&env);
    let b = hub(&env);
    client(&env, &id).swap_debt(&caller, &1u64, &a, &1i128, &b, &Bytes::new(&env));
}

#[test]
#[should_panic(expected = "Error(Contract, #43)")]
fn swap_collateral_missing_account_panics() {
    let (env, id) = setup();
    let caller = Address::generate(&env);
    let a = hub(&env);
    let b = hub(&env);
    client(&env, &id).swap_collateral(&caller, &1u64, &a, &1i128, &b, &Bytes::new(&env));
}

#[test]
#[should_panic(expected = "Error(Contract, #43)")]
fn repay_debt_with_collateral_missing_account_panics() {
    let (env, id) = setup();
    let caller = Address::generate(&env);
    let a = hub(&env);
    let b = hub(&env);
    client(&env, &id).repay_debt_with_collateral(
        &caller,
        &1u64,
        &a,
        &1i128,
        &b,
        &Bytes::new(&env),
        &false,
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #43)")]
fn migrate_from_blend_without_spoke_panics() {
    let (env, id) = setup();
    let caller = Address::generate(&env);
    let _ = client(&env, &id).migrate_from_blend(
        &caller,
        &0u64,
        &1u32,
        &0u32,
        &Address::generate(&env),
        &Vec::new(&env),
        &Vec::new(&env),
        &Vec::new(&env),
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #30)")]
fn update_indexes_without_pool_panics() {
    let (env, id) = setup();
    let caller = Address::generate(&env);
    let mut assets = Vec::new(&env);
    assets.push_back(hub(&env));
    client(&env, &id).update_indexes(&caller, &assets);
}

#[test]
#[should_panic(expected = "Error(Contract, #211)")]
fn claim_revenue_without_accumulator_panics() {
    let (env, id) = setup();
    let caller = Address::generate(&env);
    let mut assets = Vec::new(&env);
    assets.push_back(hub(&env));
    let _ = client(&env, &id).claim_revenue(&caller, &assets);
}

#[test]
#[should_panic(expected = "Error(Contract, #27)")]
fn update_account_threshold_without_pool_panics() {
    let (env, id) = setup();
    let key = hub(&env);
    let nft = setup_position_nft(&env, &id);
    let owner = Address::generate(&env);
    let account_id = u64::from(position_nft::PositionNftClient::new(&env, &nft).mint(&owner));
    assert_eq!(
        account_id, 1,
        "sanity: first mint against a fresh NFT is id 1"
    );
    env.as_contract(&id, || {
        storage::set_account_meta(
            &env,
            1,
            &AccountMeta {
                spoke_id: 1,
                mode: PositionMode::Normal,
            },
        );
        let mut supply = Map::new(&env);
        supply.set(key.clone(), one_ray_position());
        storage::set_supply_positions(&env, 1, &supply);
        storage::set_spoke(
            &env,
            1,
            &SpokeConfig {
                is_deprecated: false,
                liquidation_target_hf_wad: 0,
                hf_for_max_bonus_wad: 0,
                liquidation_bonus_factor_bps: 0,
            },
        );
        storage::set_spoke_asset(
            &env,
            1,
            &key,
            &SpokeAssetConfig {
                is_collateralizable: true,
                is_borrowable: true,
                paused: false,
                frozen: false,
                no_seize: false,
                loan_to_value: 7_500,
                liquidation_threshold: 8_000,
                liquidation_bonus: 500,
                liquidation_fees: 100,
                supply_cap: 0,
                borrow_cap: 0,
            },
        );
    });
    let caller = Address::generate(&env);
    let mut ids = Vec::new(&env);
    ids.push_back(1u64);
    client(&env, &id).update_account_threshold(&caller, &true, &ids);
}

#[test]
#[should_panic(expected = "Error(Contract, #30)")]
fn recapitalize_without_pool_panics() {
    let (env, id) = setup();
    let payer = Address::generate(&env);
    let _ = client(&env, &id).recapitalize(&payer, &hub(&env), &1i128);
}

#[test]
#[should_panic(expected = "Error(Contract, #13)")]
fn renew_account_missing_account_panics() {
    let (env, id) = setup();
    let caller = Address::generate(&env);
    client(&env, &id).renew_account(&caller, &1u64);
}

#[test]
#[should_panic(expected = "Error(Contract, #13)")]
fn add_delegate_missing_account_panics() {
    let (env, id) = setup();
    let caller = Address::generate(&env);
    client(&env, &id).add_delegate(&caller, &1u64, &Address::generate(&env));
}

#[test]
#[should_panic(expected = "Error(Contract, #13)")]
fn remove_delegate_missing_account_panics() {
    let (env, id) = setup();
    let caller = Address::generate(&env);
    client(&env, &id).remove_delegate(&caller, &1u64, &Address::generate(&env));
}

#[test]
fn admin_setters_persist_and_are_readable() {
    let (env, id) = setup();
    let c = client(&env, &id);
    let swap = Address::generate(&env);
    let prices = Address::generate(&env);
    let acc = Address::generate(&env);
    let manager = Address::generate(&env);
    let blend = Address::generate(&env);

    c.set_swap_aggregator(&swap);
    c.set_price_aggregator(&prices);
    c.set_accumulator(&acc);
    c.set_position_manager(&manager, &true);
    c.approve_blend_pool(&blend);
    c.set_position_limits(&PositionLimits {
        max_supply_positions: 4,
        max_borrow_positions: 5,
    });
    c.set_min_borrow_collateral_usd(&(2 * WAD));

    assert_eq!(c.price_aggregator(), prices);
    assert_eq!(c.get_min_borrow_collateral_usd(), 2 * WAD);
    assert!(c.is_blend_pool_approved(&blend));
    env.as_contract(&id, || {
        assert_eq!(storage::get_swap_aggregator(&env), swap);
        assert_eq!(storage::try_get_accumulator(&env), Some(acc));
        assert!(
            storage::get_position_manager(&env, &manager)
                .expect("manager")
                .is_active
        );
        let limits = storage::get_position_limits(&env);
        assert_eq!(limits.max_supply_positions, 4);
        assert_eq!(limits.max_borrow_positions, 5);
    });

    c.revoke_blend_pool(&blend);
    assert!(!c.is_blend_pool_approved(&blend));
}

#[test]
fn create_hub_and_spoke_assign_increasing_ids() {
    let (env, id) = setup();
    let c = client(&env, &id);
    assert_eq!(c.create_hub(), 1);
    assert_eq!(c.create_hub(), 2);
    assert_eq!(c.add_spoke(), 1);
    assert_eq!(c.add_spoke(), 2);
    let spoke = c.get_spoke(&2u32);
    assert!(!spoke.is_deprecated);
    c.remove_spoke(&2u32);
    assert!(c.get_spoke(&2u32).is_deprecated);
}

#[test]
fn set_spoke_liquidation_curve_persists() {
    let (env, id) = setup();
    let c = client(&env, &id);
    let spoke_id = c.add_spoke();
    c.set_spoke_liquidation_curve(
        &spoke_id,
        &1_010_000_000_000_000_000i128,
        &995_000_000_000_000_000i128,
        &8_000u32,
    );
    let spoke: SpokeConfig = c.get_spoke(&spoke_id);
    assert_eq!(spoke.liquidation_target_hf_wad, 1_010_000_000_000_000_000);
    assert_eq!(spoke.hf_for_max_bonus_wad, 995_000_000_000_000_000);
    assert_eq!(spoke.liquidation_bonus_factor_bps, 8_000);
}

#[test]
#[should_panic(expected = "Error(Contract, #30)")]
fn add_asset_to_spoke_without_hub_panics() {
    let (env, id) = setup();
    let c = client(&env, &id);
    let spoke_id = c.add_spoke();
    c.add_asset_to_spoke(&SpokeAssetArgs {
        hub_id: 1,
        asset: Address::generate(&env),
        spoke_id,
        can_collateral: true,
        can_borrow: true,
        paused: false,
        frozen: false,
        no_seize: false,
        ltv: 7_500,
        threshold: 8_000,
        bonus: 500,
        liquidation_fees: 100,
        supply_cap: 0,
        borrow_cap: 0,
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #30)")]
fn upgrade_liquidity_pool_params_without_pool_panics() {
    let (env, id) = setup();
    client(&env, &id).upgrade_liquidity_pool_params(
        &hub(&env),
        &InterestRateModel {
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
        },
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #24)")]
fn force_socialize_missing_account_panics() {
    let (env, id) = setup();
    client(&env, &id).force_socialize_bad_debt(&1u64);
}

#[test]
#[should_panic(expected = "Error(Storage, MissingValue)")]
fn deploy_pool_without_wasm_panics() {
    let (env, id) = setup();
    let _ = client(&env, &id).deploy_pool(&BytesN::from_array(&env, &[0u8; 32]));
}

#[test]
fn pause_and_unpause_toggle_pausable_flag() {
    let (env, id) = setup();
    let c = client(&env, &id);
    c.pause();
    env.as_contract(&id, || {
        assert!(stellar_contract_utils::pausable::paused(&env));
    });
    c.unpause();
    env.as_contract(&id, || {
        assert!(!stellar_contract_utils::pausable::paused(&env));
    });
}

#[test]
fn migrate_bumps_app_version() {
    let (env, id) = setup();
    let c = client(&env, &id);
    assert_eq!(c.get_app_version(), 1);
    c.migrate(&2u32);
    assert_eq!(c.get_app_version(), 2);
}

#[test]
fn view_wrappers_report_live_and_missing_accounts() {
    let (env, id) = setup();
    let c = client(&env, &id);
    let missing_hub = hub(&env);
    assert!(!c.account_exists(&1u64));
    assert_eq!(c.get_health_factor(&1u64), i128::MAX);
    assert!(!c.is_liquidatable(&1u64));
    assert_eq!(c.get_total_collateral_usd(&1u64), 0);
    assert_eq!(c.get_total_borrow_usd(&1u64), 0);
    assert_eq!(c.get_ltv_collateral_usd(&1u64), 0);
    assert_eq!(c.get_liquidation_collateral(&1u64), 0);
    assert_eq!(c.get_collateral_amount(&1u64, &missing_hub), 0);
    assert_eq!(c.get_borrow_amount(&1u64, &missing_hub), 0);

    let key = seed_live_account(&env, &id);
    assert!(c.account_exists(&1u64));
    assert_eq!(c.get_collateral_amount(&1u64, &key), 1);
    assert_eq!(c.get_borrow_amount(&1u64, &key), 1);
    assert_eq!(c.get_total_collateral_usd(&1u64), WAD);
    assert_eq!(c.get_total_borrow_usd(&1u64), WAD);
    let ltv = c.get_ltv_collateral_usd(&1u64);
    assert!(ltv > 1, "ltv={ltv}");
    let weighted = c.get_liquidation_collateral(&1u64);
    assert!(weighted > 1, "weighted={weighted}");
    let hf = c.get_health_factor(&1u64);
    assert_ne!(hf, i128::MAX);
    assert!(hf < WAD, "hf={hf}");
    assert!(c.is_liquidatable(&1u64));
    let attrs = c.get_account_attributes(&1u64);
    assert_eq!(attrs.spoke_id, 1);
    assert_eq!(attrs.mode, PositionMode::Normal);
    let (supplies, borrows) = c.get_account_positions(&1u64);
    assert_eq!(supplies.get(key.clone()).unwrap().scaled_amount, RAY);
    assert_eq!(borrows.get(key).unwrap().scaled_amount, RAY);
}

#[test]
fn get_pool_address_returns_configured_pool() {
    let (env, id) = setup();
    let pool = Address::generate(&env);
    env.as_contract(&id, || storage::set_pool(&env, &pool));
    assert_eq!(client(&env, &id).get_pool_address(), pool);
}
