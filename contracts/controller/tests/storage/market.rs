use super::*;
use crate::constants::{TTL_BUMP_SHARED, TTL_THRESHOLD_SHARED};
use crate::Controller;
use controller_interface::types::{AssetConfigRaw, MarketOracleConfig, MarketStatus};
use soroban_sdk::testutils::storage::Persistent as _;
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{Address, Env, Vec};

fn sample_market_config(env: &Env, asset: &Address) -> MarketConfig {
    MarketConfig {
        status: MarketStatus::Active,
        asset_config: AssetConfigRaw {
            loan_to_value_bps: 7_500,
            liquidation_threshold_bps: 8_000,
            liquidation_bonus_bps: 500,
            liquidation_fees_bps: 100,
            is_collateralizable: true,
            is_borrowable: true,
            is_flashloanable: true,
            flashloan_fee_bps: 9,
            asset_decimals: 7,
            e_mode_categories: Vec::new(env),
        },
        oracle_config: MarketOracleConfig::pending_for(asset.clone(), 7),
    }
}

// A read must renew the shared-tier TTL once it falls below threshold;
// markets that are used daily but reconfigured rarely otherwise archive.
#[test]
fn test_try_get_market_config_renews_shared_ttl_on_read() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    let asset = Address::generate(&env);

    env.as_contract(&contract_id, || {
        set_market_config(&env, &asset, &sample_market_config(&env, &asset));
        let key = ControllerKey::Market(asset.clone());

        // Burn the TTL below the renewal threshold.
        let ttl_after_set = env.storage().persistent().get_ttl(&key);
        let burn = ttl_after_set - TTL_THRESHOLD_SHARED + 1;
        env.ledger().with_mut(|li| li.sequence_number += burn);
        let before = env.storage().persistent().get_ttl(&key);
        assert!(before < TTL_THRESHOLD_SHARED);

        assert!(try_get_market_config(&env, &asset).is_some());

        let after = env.storage().persistent().get_ttl(&key);
        assert_eq!(after, TTL_BUMP_SHARED, "read must re-arm the shared bump");
    });
}
