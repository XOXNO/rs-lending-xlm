extern crate std;

use super::withhold_liquidation_fee;
use crate::cache::Cache;
use crate::test_support::{hub, init_ledger};
use crate::{LiquidityPool, LiquidityPoolClient};
use common::constants::RAY;
use common::math::fp::Ray;
use common::types::{
    MarketParamsRaw, PoolAction, PoolBorrowEntry, PoolStateRaw, PoolSupplyEntry, PoolWithdrawEntry,
    ScaledPositionRaw,
};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{token, vec, Address, Env};

struct TestSetup {
    env: Env,
    contract: Address,
    params: MarketParamsRaw,
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
            max_borrow_rate: 2 * RAY,
            base_borrow_rate: RAY / 100,
            slope1: RAY / 10,
            slope2: RAY / 5,
            slope3: RAY / 2,
            mid_utilization: RAY / 2,
            optimal_utilization: RAY * 8 / 10,
            max_utilization: RAY * 95 / 100,
            reserve_factor: 1_000,
            is_flashloanable: false,
            flashloan_fee: 0,
            asset_id: asset.clone(),
            asset_decimals: 7,
        };
        let contract = env.register(LiquidityPool, (admin.clone(),));
        LiquidityPoolClient::new(&env, &contract).create_market(&0u32, &params);

        Self {
            env,
            contract,
            params,
        }
    }

    fn as_contract<T>(&self, f: impl FnOnce() -> T) -> T {
        self.env.as_contract(&self.contract, f)
    }

    fn cache(&self, supplied: i128, cash: i128) -> Cache {
        Cache::from_parts(
            &self.env,
            hub(&self.params.asset_id),
            &self.params,
            &PoolStateRaw {
                supplied,
                borrowed: 0,
                revenue: 0,
                borrow_index: RAY,
                supply_index: RAY,
                last_timestamp: 0,
                cash,
            },
            1_000_000,
        )
    }
}

#[test]
fn test_withhold_liquidation_fee_noop_when_not_liquidation_or_zero_fee() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let mut cache = t.cache(100 * RAY, 50_000_000);
        let out = withhold_liquidation_fee(&t.env, &mut cache, 10_000_000, false, 1_000_000);
        assert_eq!(out, 10_000_000);
        let out2 = withhold_liquidation_fee(&t.env, &mut cache, 10_000_000, true, 0);
        assert_eq!(out2, 10_000_000);
    });
}

#[test]
fn test_withhold_liquidation_fee_accrues_to_revenue_and_reduces_net() {
    let env = Env::default();
    let t = TestSetup::new();
    t.as_contract(|| {
        let mut cache = t.cache(100 * RAY, 50_000_000);
        let fee_raw = 2_000_000i128;

        let expected_revenue = Ray::from_asset(&env, fee_raw, t.params.asset_decimals);
        let net = withhold_liquidation_fee(&t.env, &mut cache, 10_000_000, true, fee_raw);
        assert_eq!(net, 8_000_000);
        assert_eq!(cache.revenue(), expected_revenue);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #115)")]
fn test_withhold_liquidation_fee_rejects_fee_greater_than_gross() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let mut cache = t.cache(100 * RAY, 50_000_000);
        let _ = withhold_liquidation_fee(&t.env, &mut cache, 1_000_000, true, 2_000_000);
    });
}

#[test]
fn test_liquidation_withdraw_uses_post_burn_fee_headroom_and_final_debt_guard() {
    const UNIT: i128 = 10_000_000;
    let largest_deposit = i128::MAX / (RAY / UNIT);
    for (deposit, borrowed, full_close) in [
        (largest_deposit, 0, false),
        (largest_deposit, 0, true),
        (100 * UNIT, 10 * UNIT, true),
    ] {
        let t = TestSetup::new();
        let client = LiquidityPoolClient::new(&t.env, &t.contract);
        let payer = Address::generate(&t.env);
        let receiver = Address::generate(&t.env);
        let asset = &t.params.asset_id;
        let key = hub(asset);
        let tok = token::Client::new(&t.env, asset);
        token::StellarAssetClient::new(&t.env, asset).mint(&payer, &deposit);
        tok.transfer(&payer, &t.contract, &deposit);
        let supplied = client
            .supply(&vec![
                &t.env,
                PoolSupplyEntry {
                    action: PoolAction {
                        hub_asset: key.clone(),
                        position: ScaledPositionRaw { scaled_amount: 0 },
                        amount: deposit,
                    },
                },
            ])
            .get(0)
            .unwrap();
        if borrowed > 0 {
            client.borrow(
                &payer,
                &vec![
                    &t.env,
                    PoolBorrowEntry {
                        action: PoolAction {
                            hub_asset: key.clone(),
                            position: ScaledPositionRaw { scaled_amount: 0 },
                            amount: borrowed,
                        },
                    },
                ],
            );
        }

        let gross = if full_close { deposit } else { 100 * UNIT };
        let fee = 10 * UNIT;
        let result = client
            .withdraw(
                &receiver,
                &true,
                &vec![
                    &t.env,
                    PoolWithdrawEntry {
                        action: PoolAction {
                            hub_asset: key.clone(),
                            position: supplied.position,
                            amount: if full_close { i128::MAX } else { gross },
                        },
                        protocol_fee: fee,
                    },
                ],
            )
            .get(0)
            .unwrap();

        let state = client.get_sync_data(&key).state;
        let remaining_shares = (deposit - gross) * (RAY / UNIT);
        assert_eq!(result.actual_amount, gross);
        assert_eq!(result.position.scaled_amount, remaining_shares);
        assert_eq!(state.supplied, remaining_shares + 10 * RAY);
        assert_eq!(state.revenue, 10 * RAY);
        assert_eq!(client.get_revenue(&key), fee);
        assert_eq!(state.borrowed, borrowed * (RAY / UNIT));
        assert_eq!(state.cash, deposit - borrowed - (gross - fee));
        assert_eq!(tok.balance(&t.contract), state.cash);
        assert_eq!(tok.balance(&receiver), gross - fee);
    }
}
