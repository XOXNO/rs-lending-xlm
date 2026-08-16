extern crate std;

use super::*;
use crate::constants::RAY;
use common::types::{
    AccountMeta, AccountPositionRaw, DebtPositionRaw, MarketIndexRaw, MarketParamsRaw,
    PoolStateRaw, PoolSyncData, PositionMode, PriceFeedRaw, PriceKey,
};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{contract, contractimpl, Address, Map};

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

fn register_view_deps(env: &Env, contract_id: &Address) {
    let pool = env.register(ViewPool, ());
    let aggregator = env.register(ViewAggregator, ());
    env.as_contract(contract_id, || {
        storage::set_pool(env, &pool);
        storage::set_price_aggregator(env, &aggregator);
    });
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

fn one_ray_position() -> AccountPositionRaw {
    AccountPositionRaw {
        scaled_amount: RAY,
        liquidation_threshold: 8_000,
        liquidation_bonus: 500,
        loan_to_value: 7_500,
        liquidation_fees: 100,
    }
}

/// Mints a position NFT to a fresh owner and writes matching account metadata (and, if given,
/// supply/debt positions). Returns the minted account id, which callers must use in place of a
/// hardcoded id since ownership now resolves through the NFT.
fn seed_account(
    env: &Env,
    contract_id: &Address,
    nft: &Address,
    spoke_id: u32,
    supply: Option<(HubAssetKey, AccountPositionRaw)>,
    debt: Option<(HubAssetKey, DebtPositionRaw)>,
) -> u64 {
    let owner = Address::generate(env);
    let account_id = u64::from(position_nft::PositionNftClient::new(env, nft).mint(&owner));
    env.as_contract(contract_id, || {
        storage::set_account_meta(
            env,
            account_id,
            &AccountMeta {
                spoke_id,
                mode: PositionMode::Normal,
            },
        );
        if let Some((key, pos)) = supply {
            let mut map = Map::new(env);
            map.set(key, pos);
            storage::set_supply_positions(env, account_id, &map);
        }
        if let Some((key, pos)) = debt {
            let mut map = Map::new(env);
            map.set(key, pos);
            storage::set_debt_positions(env, account_id, &map);
        }
    });
    account_id
}

#[test]
#[should_panic(expected = "Error(Contract, #16)")]
fn view_input_bound_rejects_oversized_asset_vectors() {
    let env = Env::default();
    let mut assets = Vec::new(&env);
    for _ in 0..=MAX_VIEW_INPUTS {
        assets.push_back(Address::generate(&env));
    }

    require_view_inputs_bound(&env, &assets);
}

#[test]
fn aggregate_views_return_zero_for_missing_or_empty_account() {
    use crate::Controller;
    use common::types::{AccountMeta, PositionMode};
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    env.as_contract(&contract_id, || {
        assert_eq!(total_collateral_in_usd(&env, 1), 0);
        assert_eq!(total_borrow_in_usd(&env, 1), 0);
        assert_eq!(ltv_collateral_in_usd(&env, 1), 0);

        // No position NFT is registered here: `total_collateral_in_usd` only ever
        // consults account metadata, never the owner, so this stays meta-only.
        storage::set_account_meta(
            &env,
            1,
            &AccountMeta {
                spoke_id: 0,
                mode: PositionMode::Normal,
            },
        );
        assert_eq!(total_collateral_in_usd(&env, 1), 0);
    });
}

#[test]
fn health_factor_debt_free_account_skips_pricing() {
    use crate::Controller;
    use common::types::{AccountMeta, AccountPositionRaw, PositionMode};
    use soroban_sdk::Map;
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    let nft = setup_position_nft(&env, &contract_id);
    let owner = Address::generate(&env);
    let account_id = u64::from(position_nft::PositionNftClient::new(&env, &nft).mint(&owner));
    env.as_contract(&contract_id, || {
        // A minted, debt-free account: `try_get_account` resolves `Some(account)`,
        // so `health_factor` reaches `i128::MAX` via the `!account.debt_free()`
        // guard failing on a live account, not via the `None` (missing-owner) arm.
        storage::set_account_meta(
            &env,
            account_id,
            &AccountMeta {
                spoke_id: 0,
                mode: PositionMode::Normal,
            },
        );

        let key = HubAssetKey {
            hub_id: 0,
            asset: Address::generate(&env),
        };
        let mut supplies: Map<HubAssetKey, AccountPositionRaw> = Map::new(&env);
        supplies.set(
            key,
            AccountPositionRaw {
                scaled_amount: 1_000,
                liquidation_threshold: 8_000,
                liquidation_bonus: 500,
                loan_to_value: 7_500,
                liquidation_fees: 100,
            },
        );
        storage::set_supply_positions(&env, account_id, &supplies);

        assert_eq!(health_factor(&env, account_id), i128::MAX);
        assert!(!can_be_liquidated(&env, account_id));
    });
}

#[test]
fn get_spoke_usage_returns_stored_row() {
    use crate::Controller;
    use common::types::SpokeUsageRaw;
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    let client = crate::ControllerClient::new(&env, &contract_id);

    let key = HubAssetKey {
        hub_id: 0,
        asset: Address::generate(&env),
    };
    env.as_contract(&contract_id, || {
        storage::set_spoke_usage(
            &env,
            1,
            &key,
            &SpokeUsageRaw {
                supplied_scaled_ray: 5,
                borrowed_scaled_ray: 7,
            },
        );
    });

    let usage = client.get_spoke_usage(&1u32, &key);
    assert_eq!(usage.supplied_scaled_ray, 5);
    assert_eq!(usage.borrowed_scaled_ray, 7);
}

#[test]
fn collateral_and_borrow_amount_return_zero_for_missing_position() {
    use crate::Controller;
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    env.as_contract(&contract_id, || {
        let hub = HubAssetKey {
            hub_id: 0,
            asset: Address::generate(&env),
        };
        // Missing account / position short-circuits before pool index fetch.
        assert_eq!(collateral_amount_for_hub_asset(&env, 1, &hub), 0);
        assert_eq!(borrow_amount_for_hub_asset(&env, 1, &hub), 0);
    });
}

#[test]
fn account_exists_and_positions_distinguish_missing_from_seeded() {
    use crate::Controller;
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    let nft = setup_position_nft(&env, &contract_id);
    let hub = HubAssetKey {
        hub_id: 0,
        asset: Address::generate(&env),
    };
    env.as_contract(&contract_id, || {
        assert!(!account_exists(&env, 1));
        let (empty_s, empty_d) = get_account_positions(&env, 1);
        assert!(empty_s.is_empty());
        assert!(empty_d.is_empty());
    });

    let account_id = seed_account(
        &env,
        &contract_id,
        &nft,
        0,
        Some((hub.clone(), one_ray_position())),
        Some((hub.clone(), DebtPositionRaw { scaled_amount: RAY })),
    );
    env.as_contract(&contract_id, || {
        assert!(account_exists(&env, account_id));
        let (supplies, borrows) = get_account_positions(&env, account_id);
        assert_eq!(supplies.get(hub.clone()).unwrap().scaled_amount, RAY);
        assert_eq!(borrows.get(hub).unwrap().scaled_amount, RAY);
    });
}

#[test]
fn live_amounts_and_usd_views_are_nonzero() {
    use crate::Controller;
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    register_view_deps(&env, &contract_id);
    let nft = setup_position_nft(&env, &contract_id);
    let hub = HubAssetKey {
        hub_id: 0,
        asset: Address::generate(&env),
    };
    let account_id = seed_account(
        &env,
        &contract_id,
        &nft,
        0,
        Some((hub.clone(), one_ray_position())),
        Some((hub.clone(), DebtPositionRaw { scaled_amount: RAY })),
    );
    env.as_contract(&contract_id, || {
        assert_eq!(collateral_amount_for_hub_asset(&env, account_id, &hub), 1);
        assert_eq!(borrow_amount_for_hub_asset(&env, account_id, &hub), 1);
        assert_eq!(total_collateral_in_usd(&env, account_id), WAD);
        assert_eq!(total_borrow_in_usd(&env, account_id), WAD);
        let ltv = ltv_collateral_in_usd(&env, account_id);
        assert!(ltv > 1, "ltv={ltv}");
        let weighted = liquidation_collateral_available(&env, account_id);
        assert!(weighted > 1, "weighted={weighted}");
    });
}

#[test]
fn health_factor_with_debt_is_computed_and_can_be_liquidated() {
    use crate::Controller;
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    register_view_deps(&env, &contract_id);
    let nft = setup_position_nft(&env, &contract_id);
    let hub = HubAssetKey {
        hub_id: 0,
        asset: Address::generate(&env),
    };
    let account_id = seed_account(
        &env,
        &contract_id,
        &nft,
        0,
        Some((hub.clone(), one_ray_position())),
        Some((hub.clone(), DebtPositionRaw { scaled_amount: RAY })),
    );
    env.as_contract(&contract_id, || {
        let hf = health_factor(&env, account_id);
        assert_ne!(hf, i128::MAX);
        assert!(hf < WAD, "hf={hf}");
        assert!(can_be_liquidated(&env, account_id));
    });
}

#[test]
fn healthy_at_one_wad_is_not_liquidatable() {
    use crate::Controller;
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    register_view_deps(&env, &contract_id);
    let nft = setup_position_nft(&env, &contract_id);
    let hub = HubAssetKey {
        hub_id: 0,
        asset: Address::generate(&env),
    };
    let mut pos = one_ray_position();
    pos.liquidation_threshold = 10_000;
    let account_id = seed_account(
        &env,
        &contract_id,
        &nft,
        0,
        Some((hub.clone(), pos)),
        Some((hub, DebtPositionRaw { scaled_amount: RAY })),
    );
    env.as_contract(&contract_id, || {
        assert_eq!(health_factor(&env, account_id), WAD);
        assert!(!can_be_liquidated(&env, account_id));
    });
}
