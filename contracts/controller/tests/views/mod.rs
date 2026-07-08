extern crate std;

use super::*;
use soroban_sdk::testutils::Address as _;

#[test]
#[should_panic]
fn view_input_bound_rejects_oversized_asset_vectors() {
    let env = Env::default();
    let mut assets = Vec::new(&env);
    for _ in 0..=MAX_VIEW_INPUTS {
        assets.push_back(Address::generate(&env));
    }

    require_view_inputs_bound(&env, &assets);
}

// ===== coverage gap-closure tests =====
// aggregate_views_zero_for_missing_or_empty_account (+4) contracts/controller/src/views/aggregates.rs (uncovered 17,21,55,93)
#[test]
fn aggregate_views_return_zero_for_missing_or_empty_account() {
    use crate::Controller;
    use common::types::{AccountMeta, PositionMode};
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    env.as_contract(&contract_id, || {
        // Missing account -> all aggregate views return 0 (17,55,93).
        assert_eq!(total_collateral_in_usd(&env, 1), 0);
        assert_eq!(total_borrow_in_usd(&env, 1), 0);
        assert_eq!(ltv_collateral_in_usd(&env, 1), 0);

        // Seeded account, no positions -> supply aggregate returns 0 (21).
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
// max_actions_zero_for_missing_account_or_inactive_asset (+4) contracts/controller/src/views/limits/{borrow.rs:20,24, withdraw.rs:28, supply.rs:22}
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

        // No account -> max_borrow (20) and max_withdraw (28) return 0.
        assert_eq!(max_borrow(&env, 1, &key), 0);
        assert_eq!(max_withdraw(&env, 1, &key), 0);
        // Not paused, asset has no oracle entry -> max_supply returns 0 (22).
        assert_eq!(max_supply(&env, 1, &key), 0);

        // Account present but asset still inactive (no oracle) -> max_borrow
        // AssetOracle-none branch (24).
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
