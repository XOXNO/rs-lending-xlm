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
