extern crate std;

use crate::test_support::init_ledger;
use crate::{LiquidityPool, LiquidityPoolClient};
use common::constants::RAY;
use common::types::{HubAssetKey, MarketParamsRaw, PoolAction, PoolSupplyEntry, ScaledPositionRaw};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{token, vec, Address, Env};

/// Phase 0 markets all live on hub 0.
fn hub(asset: &Address) -> HubAssetKey {
    HubAssetKey {
        hub_id: 0,
        asset: asset.clone(),
    }
}

struct TestSetup {
    env: Env,
    contract: Address,
    asset: Address,
}

impl TestSetup {
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths();
        init_ledger(&env);

        let admin = Address::generate(&env);
        let asset = env
            .register_stellar_asset_contract_v2(admin.clone())
            .address();
        let params = MarketParamsRaw {
            max_borrow_rate_ray: 2 * RAY,
            base_borrow_rate_ray: RAY / 100,
            slope1_ray: RAY / 10,
            slope2_ray: RAY / 5,
            slope3_ray: RAY / 2,
            mid_utilization_ray: RAY / 2,
            optimal_utilization_ray: RAY * 8 / 10,
            max_utilization_ray: RAY * 95 / 100,
            reserve_factor_bps: 1_000,
            supply_cap: 0,
            borrow_cap: 0,
            is_flashloanable: false,
            flashloan_fee_bps: 0,
            asset_id: asset.clone(),
            asset_decimals: 7,
        };
        let contract = env.register(LiquidityPool, (admin.clone(),));
        LiquidityPoolClient::new(&env, &contract).create_market(&params);

        // Seed liquidity for repay/overpay scenarios.
        let tok_admin = token::StellarAssetClient::new(&env, &asset);
        tok_admin.mint(&contract, &1_000_000_000);

        Self {
            env,
            contract,
            asset,
        }
    }

    fn client(&self) -> LiquidityPoolClient<'_> {
        LiquidityPoolClient::new(&self.env, &self.contract)
    }
}

fn make_action(position_scaled: i128, amount: i128, asset: &Address) -> PoolAction {
    PoolAction {
        position: ScaledPositionRaw {
            scaled_amount_ray: position_scaled,
        },
        amount,
        hub_asset: hub(asset),
    }
}

#[test]
fn test_bulk_supply_returns_input_ordered_mutations() {
    let t = TestSetup::new();
    let client = t.client();
    // Call through the client; output order follows the *_one path.
    let entry1 = PoolSupplyEntry {
        action: make_action(0, 100_000_000, &t.asset),
    };
    let entry2 = PoolSupplyEntry {
        action: make_action(0, 50_000_000, &t.asset),
    };
    let results = client.supply(&vec![&t.env, entry1, entry2]);
    assert_eq!(results.len(), 2);
    assert_eq!(results.get(0).unwrap().actual_amount, 100_000_000);
    assert_eq!(results.get(1).unwrap().actual_amount, 50_000_000);
}

#[test]
fn test_add_rewards_emits_snapshot_and_increases_supply_index() {
    let t = TestSetup::new();
    let client = t.client();
    // Supply first so there are suppliers to reward.
    let sup = PoolSupplyEntry {
        action: make_action(0, 100_000_000, &t.asset),
    };
    let _ = client.supply(&vec![&t.env, sup]);

    client.add_rewards(&t.asset, &10_000_000);
    let snap = client.get_sync_data(&t.asset).state;
    assert!(snap.supply_index_ray > RAY);
}
