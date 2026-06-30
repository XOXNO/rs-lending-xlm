use super::*;
use crate::Controller;
use common::types::MarketOracleConfigOption;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env};

fn spoke_asset_with_override(oracle_override: MarketOracleConfigOption) -> SpokeAssetConfig {
    SpokeAssetConfig {
        is_collateralizable: true,
        is_borrowable: true,
        paused: false,
        frozen: false,
        loan_to_value: 9_000,
        liquidation_threshold: 9_300,
        liquidation_bonus: 300,
        liquidation_fees: 0,
        supply_cap: 0,
        borrow_cap: 0,
        oracle_override,
    }
}

// Oracle resolution is token-rooted: it returns the `AssetOracle` entry that
// `set_market_oracle_config` writes, independent of any spoke.
#[test]
fn resolve_default_returns_asset_oracle_base() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    let asset = Address::generate(&env);

    env.as_contract(&contract_id, || {
        let base = MarketOracleConfig::pending_for(asset.clone(), 7);
        storage::set_asset_oracle(&env, &asset, &base);

        assert_eq!(storage::get_asset_oracle(&env, &asset), Some(base.clone()));

        let mut cache = Cache::new_view(&env);
        assert_eq!(cache.resolve_oracle_config(&asset), base);
    });
}

// Pricing is token-rooted: a spoke-asset listing never diverts oracle resolution
// from the token's `AssetOracle` base, regardless of the listing's contents.
#[test]
fn resolve_spoke_without_override_falls_back_to_base() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    let asset = Address::generate(&env);
    let spoke_id = 1u32;

    env.as_contract(&contract_id, || {
        let base = MarketOracleConfig::pending_for(asset.clone(), 7);
        storage::set_asset_oracle(&env, &asset, &base);

        let hub_asset = HubAssetKey {
            hub_id: 0,
            asset: asset.clone(),
        };
        storage::set_spoke_asset(
            &env,
            spoke_id,
            &hub_asset,
            &spoke_asset_with_override(MarketOracleConfigOption::None),
        );

        let mut cache = Cache::new_view(&env);
        cache.ensure_spoke_context(spoke_id);
        assert_eq!(cache.resolve_oracle_config(&asset), base);
    });
}
