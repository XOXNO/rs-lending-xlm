extern crate std;

use super::*;
use soroban_sdk::testutils::Address as _;

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
        // Missing accounts have no aggregate value.
        assert_eq!(total_collateral_in_usd(&env, 1), 0);
        assert_eq!(total_borrow_in_usd(&env, 1), 0);
        assert_eq!(ltv_collateral_in_usd(&env, 1), 0);

        // Existing accounts without positions also have no collateral value.
        let owner = Address::generate(&env);
        storage::set_account_meta(
            &env,
            1,
            &AccountMeta {
                owner,
                spoke_id: 0,
                mode: PositionMode::Normal,
            },
        );
        assert_eq!(total_collateral_in_usd(&env, 1), 0);
    });
}
// The debt-free short-circuit must fire before any pricing: a supply-only
// account with an unconfigured oracle still reads `i128::MAX` instead of
// reverting on the missing feed.
#[test]
fn health_factor_debt_free_account_skips_pricing() {
    use crate::Controller;
    use common::types::{AccountMeta, AccountPositionRaw, PositionMode};
    use soroban_sdk::Map;
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    env.as_contract(&contract_id, || {
        let owner = Address::generate(&env);
        storage::set_account_meta(
            &env,
            1,
            &AccountMeta {
                owner,
                spoke_id: 0,
                mode: PositionMode::Normal,
            },
        );
        // Supply position on an asset with no oracle configured.
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
        storage::set_supply_positions(&env, 1, &supplies);

        assert_eq!(health_factor(&env, 1), i128::MAX);
        assert!(!can_be_liquidated(&env, 1));
    });
}

#[test]
fn max_actions_return_zero_for_missing_account_or_inactive_asset() {
    use crate::views::limits::{max_borrow, max_supply, max_withdraw};
    use crate::Controller;
    use common::types::{AccountMeta, PositionMode};
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    env.as_contract(&contract_id, || {
        let key = HubAssetKey {
            hub_id: 1,
            asset: Address::generate(&env),
        };

        // A missing account cannot borrow or withdraw.
        assert_eq!(max_borrow(&env, 1, &key), 0);
        assert_eq!(max_withdraw(&env, 1, &key), 0);
        // An inactive asset cannot be supplied.
        assert_eq!(max_supply(&env, 1, &key), 0);

        // Creating the account does not make the asset active.
        let owner = Address::generate(&env);
        storage::set_account_meta(
            &env,
            1,
            &AccountMeta {
                owner,
                spoke_id: 0,
                mode: PositionMode::Normal,
            },
        );
        assert_eq!(max_borrow(&env, 1, &key), 0);
    });
}

// The spoke-usage view reads the stored row verbatim, not a default.
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
