use super::*;
use crate::constants::{TTL_BUMP_SHARED, TTL_THRESHOLD_SHARED};
use crate::Controller;
use controller_interface::types::MarketOracleConfigOption;
use soroban_sdk::testutils::storage::Persistent as _;
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{Address, Env};

fn sample_spoke() -> SpokeConfig {
    SpokeConfig {
        is_deprecated: false,
        liquidation_target_hf_wad: 0,
        hf_for_max_bonus_wad: 0,
        liquidation_bonus_factor_bps: 0,
    }
}

fn sample_spoke_asset() -> SpokeAssetConfig {
    SpokeAssetConfig {
        is_collateralizable: true,
        is_borrowable: true,
        paused: false,
        frozen: false,
        loan_to_value_bps: 9_000,
        liquidation_threshold_bps: 9_300,
        liquidation_bonus_bps: 300,
        liquidation_fees_bps: 0,
        supply_cap: 0,
        borrow_cap: 0,
        oracle_override: MarketOracleConfigOption::None,
    }
}

// Spoke reads renew shared-tier TTL once it falls below threshold.
#[test]
fn test_try_get_spoke_renews_shared_ttl_on_read() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));

    env.as_contract(&contract_id, || {
        set_spoke(&env, 1, &sample_spoke());
        let key = ControllerKey::Spoke(1);

        let ttl_after_set = env.storage().persistent().get_ttl(&key);
        let burn = ttl_after_set - TTL_THRESHOLD_SHARED + 1;
        env.ledger().with_mut(|li| li.sequence_number += burn);
        assert!(env.storage().persistent().get_ttl(&key) < TTL_THRESHOLD_SHARED);

        assert!(try_get_spoke(&env, 1).is_some());

        assert_eq!(
            env.storage().persistent().get_ttl(&key),
            TTL_BUMP_SHARED,
            "read must re-arm the shared bump"
        );
    });
}

// Discrete spoke-asset keys round-trip and remove independently of the spoke.
#[test]
fn test_spoke_asset_discrete_key_roundtrip() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));

    env.as_contract(&contract_id, || {
        let hub_asset = HubAssetKey {
            hub_id: 0,
            asset: Address::generate(&env),
        };
        assert!(get_spoke_asset(&env, 1, &hub_asset).is_none());

        set_spoke_asset(&env, 1, &hub_asset, &sample_spoke_asset());
        let stored = get_spoke_asset(&env, 1, &hub_asset).expect("config present after write");
        assert_eq!(stored.loan_to_value_bps, 9_000);
        assert!(stored.oracle_override.as_ref().is_none());

        remove_spoke_asset(&env, 1, &hub_asset);
        assert!(get_spoke_asset(&env, 1, &hub_asset).is_none());
    });
}

// Usage writes round-trip and a fully-zero write prunes the key.
#[test]
fn test_spoke_usage_prunes_zero_entry() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));

    env.as_contract(&contract_id, || {
        let hub_asset = HubAssetKey {
            hub_id: 0,
            asset: Address::generate(&env),
        };

        set_spoke_usage(
            &env,
            1,
            &hub_asset,
            &SpokeUsageRaw {
                supplied_scaled_ray: 5,
                borrowed_scaled_ray: 0,
            },
        );
        let stored = get_spoke_usage(&env, 1, &hub_asset).expect("usage present after write");
        assert_eq!(stored.supplied_scaled_ray, 5);

        set_spoke_usage(&env, 1, &hub_asset, &SpokeUsageRaw::default());
        assert!(get_spoke_usage(&env, 1, &hub_asset).is_none());
    });
}
