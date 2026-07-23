extern crate std;

use super::*;
use crate::test_support::init_ledger;
use crate::{LiquidityPool, LiquidityPoolClient};
use common::constants::RAY;
use common::types::HubAssetKey;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::Address;

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
    params: MarketParamsRaw,
}

impl TestSetup {
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths();
        init_ledger(&env);

        let admin = Address::generate(&env);
        let asset = Address::generate(&env);
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
            asset,
            params,
        }
    }

    fn as_contract<T>(&self, f: impl FnOnce() -> T) -> T {
        self.env.as_contract(&self.contract, f)
    }
}

#[test]
#[should_panic(expected = "Error(Contract, #30)")]
fn test_load_panics_when_state_is_missing() {
    let t = TestSetup::new();

    t.as_contract(|| {
        t.env
            .storage()
            .persistent()
            .remove(&PoolKey::State(hub(&t.asset)));
        let _ = Cache::load(&t.env, &hub(&t.asset));
    });
}

#[test]
fn test_calculate_utilization_returns_zero_when_supply_index_zeroes_total_supply() {
    let t = TestSetup::new();

    t.as_contract(|| {
        let cache = Cache {
            env: t.env.clone(),
            supplied: Ray::from(10 * RAY),
            borrowed: Ray::from(5 * RAY),
            revenue: Ray::ZERO,
            borrow_index: Ray::from(2 * RAY),
            supply_index: Ray::ZERO,
            last_timestamp: 0,
            current_timestamp: 1_000_000,
            params: (&t.params).into(),
            hub_asset: hub(&t.asset),
            cash: 0,
        };

        assert_eq!(cache.calculate_utilization(), Ray::ZERO);
    });
}

fn cache_with(
    env: &Env,
    params: &MarketParamsRaw,
    supplied: i128,
    borrowed: i128,
    revenue: i128,
    supply_index: i128,
    borrow_index: i128,
) -> Cache {
    Cache {
        env: env.clone(),
        supplied: Ray::from(supplied),
        borrowed: Ray::from(borrowed),
        revenue: Ray::from(revenue),
        borrow_index: Ray::from(borrow_index),
        supply_index: Ray::from(supply_index),
        last_timestamp: 0,
        current_timestamp: 1_000_000,
        params: params.into(),
        hub_asset: hub(&params.asset_id),
        cash: 0,
    }
}

#[test]
fn test_calculate_utilization_returns_zero_when_supplied_is_zero() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let cache = cache_with(&t.env, &t.params, 0, 5 * RAY, 0, RAY, RAY);
        assert_eq!(cache.calculate_utilization(), Ray::ZERO);
    });
}

#[test]
fn test_calculate_utilization_returns_ratio_at_normal_state() {
    let t = TestSetup::new();
    t.as_contract(|| {
        // 5 borrowed against 10 supplied at index 1 -> 50% utilization.
        let cache = cache_with(&t.env, &t.params, 10 * RAY, 5 * RAY, 0, RAY, RAY);
        assert_eq!(cache.calculate_utilization(), Ray::from(RAY / 2));
    });
}

// Across the pool's complete 0..=27 decimal domain, directed share rounding must
// keep both roundtrips conservative: supply credits floor shares and withdrawals
// pay at most their value; borrow debits ceil shares and full repayment charges at
// least the amount borrowed.
#[test]
fn test_all_decimal_roundtrips_never_favor_user() {
    let t = TestSetup::new();
    t.env.cost_estimate().budget().reset_unlimited();
    let indexes = [
        RAY,
        RAY * 3 / 2,
        RAY * 7 / 3,
        666_666_666 * RAY,
        714_285_714 * RAY,
        common::constants::MAX_SUPPLY_INDEX_RAY,
    ];
    let amounts = [1i128, 2, 3, 7, 99, 1_000, 123_457];
    t.as_contract(|| {
        for decimals in 0u32..=27 {
            let mut params = t.params.clone();
            params.asset_decimals = decimals;
            for &index in &indexes {
                for &a in &amounts {
                    let cache = cache_with(&t.env, &params, 0, 0, 0, index, index);

                    // Supplier: supply `a` (mint shares down), withdraw all.
                    let shares = cache.calculate_scaled_supply(a);
                    let (_burned, paid) = cache.resolve_withdrawal(a, shares);
                    assert!(
                        paid <= a,
                        "supply roundtrip favored the user: dec={decimals} \
                         index={index} deposited={a} withdrew={paid}"
                    );

                    // Borrower: borrow `a` (mint debt up), owe rounded up.
                    let debt = cache.calculate_scaled_borrow(a);
                    let owed = cache.unscale_borrow_ceil(debt);
                    assert!(
                        owed >= a,
                        "borrow roundtrip favored the user: dec={decimals} \
                         index={index} borrowed={a} owed={owed}"
                    );
                }
            }
        }
    });
}

#[test]
fn test_directed_partial_rounding_blocks_high_decimal_value_creation() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let mut params = t.params.clone();
        params.asset_decimals = 27;
        let cache = cache_with(&t.env, &params, 100 * RAY, 100 * RAY, 0, 3 * RAY, 3 * RAY);

        // A two-raw-unit supply previously rounded 2/3 share up to one share
        // worth three raw units. Floor credit now rejects it through the caller.
        assert_eq!(cache.calculate_scaled_supply(2), Ray::ZERO);

        // Borrowing four raw units records two shares, covering six raw units.
        assert_eq!(cache.calculate_scaled_borrow(4).raw(), 2);

        // Withdrawing four burns two shares; repaying two burns no share. The
        // public callers respectively accept the conservative debit and reject
        // the zero credit through their existing guards.
        assert_eq!(cache.calculate_scaled_supply_ceil(4).raw(), 2);
        assert_eq!(cache.calculate_scaled_borrow_floor(2), Ray::ZERO);
    });
}

#[test]
fn test_resolve_repay_partial_returns_zero_overpayment() {
    let t = TestSetup::new();
    t.as_contract(|| {
        // current_debt = 1 asset unit; partial repay of 0 (no debt cleared).
        let cache = cache_with(&t.env, &t.params, 0, 10i128.pow(20), 0, RAY, RAY);
        let pos_scaled = Ray::from(10i128.pow(20));
        let (scaled, overpayment) = cache.resolve_repay(0, pos_scaled);
        assert_eq!(scaled, Ray::ZERO);
        assert_eq!(overpayment, 0);
    });
}

#[test]
fn test_resolve_repay_full_returns_positive_overpayment() {
    let t = TestSetup::new();
    t.as_contract(|| {
        // current_debt = 1; pay 5 -> overpayment = 4, burn full position.
        let cache = cache_with(&t.env, &t.params, 0, 10i128.pow(20), 0, RAY, RAY);
        let pos_scaled = Ray::from(10i128.pow(20));
        let (scaled, overpayment) = cache.resolve_repay(5, pos_scaled);
        assert_eq!(scaled, pos_scaled);
        assert_eq!(overpayment, 4);
    });
}

#[test]
fn test_resolve_withdrawal_full_when_amount_exceeds_position() {
    let t = TestSetup::new();
    t.as_contract(|| {
        // Position = 1 asset unit; request 100 -> full withdraw.
        let cache = cache_with(&t.env, &t.params, 10i128.pow(20), 0, 0, RAY, RAY);
        let pos_scaled = Ray::from(10i128.pow(20));
        let (scaled, gross) = cache.resolve_withdrawal(100, pos_scaled);
        assert_eq!(scaled, pos_scaled);
        assert_eq!(gross, 1);
    });
}

#[test]
fn test_resolve_withdrawal_partial_returns_requested_amount() {
    let t = TestSetup::new();
    t.as_contract(|| {
        // Position = 5 asset units; request 2 -> partial.
        let supplied = 5 * 10i128.pow(20);
        let cache = cache_with(&t.env, &t.params, supplied, 0, 0, RAY, RAY);
        let pos_scaled = Ray::from(supplied);
        let (scaled, gross) = cache.resolve_withdrawal(2, pos_scaled);
        assert_eq!(scaled.raw(), 2 * 10i128.pow(20));
        assert_eq!(gross, 2);
    });
}

#[test]
fn test_market_index_reflects_current_indexes() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let cache = cache_with(&t.env, &t.params, 0, 0, 0, 2 * RAY, 3 * RAY);
        let idx = cache.market_index();
        assert_eq!(idx.supply_index, 2 * RAY);
        assert_eq!(idx.borrow_index, 3 * RAY);
    });
}

#[test]
fn test_burn_claimable_revenue_zero_revenue_returns_zero() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let mut cache = cache_with(&t.env, &t.params, 100 * RAY, 0, 0, RAY, RAY);
        cache.revenue = Ray::ZERO;
        let amt = cache.burn_claimable_revenue();
        assert_eq!(amt, 0);
    });
}

#[test]
fn test_burn_claimable_revenue_capped_by_reserves() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let mut cache = cache_with(&t.env, &t.params, 100 * RAY, 0, 0, RAY, RAY);
        cache.cash = 10_000_000;
        cache.revenue = Ray::from(50 * RAY);
        let amount = cache.burn_claimable_revenue();

        assert_eq!(amount, 10_000_000);
        assert_eq!(cache.revenue, Ray::from(49 * RAY));
        assert_eq!(cache.supplied, Ray::from(99 * RAY));
    });
}

#[test]
fn test_burn_claimable_revenue_full_when_revenue_smaller_than_reserves() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let mut cache = cache_with(&t.env, &t.params, 100 * RAY, 0, 0, RAY, RAY);
        cache.cash = 100_000_000;
        cache.revenue = Ray::from(5 * RAY);
        let amount = cache.burn_claimable_revenue();

        assert_eq!(amount, 50_000_000);
        assert_eq!(cache.revenue, Ray::ZERO);
        assert_eq!(cache.supplied, Ray::from(95 * RAY));
    });
}

#[test]
fn test_position_mutation_builder_includes_scaled_and_actual() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let cache = cache_with(&t.env, &t.params, 0, 0, 0, RAY, RAY);
        let m = cache.position_mutation(Ray::from(42 * RAY), 123);
        assert_eq!(m.position.scaled_amount, 42 * RAY);
        assert_eq!(m.actual_amount, 123);
    });
}

#[test]
fn test_strategy_mutation_builder() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let cache = cache_with(&t.env, &t.params, 0, 0, 0, RAY, RAY);
        let s = cache.strategy_mutation(Ray::from(99 * RAY), 100, 90);
        assert_eq!(s.actual_amount, 100);
        assert_eq!(s.amount_received, 90);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #33)")]
fn test_ray_checked_sub_assign_panics_on_underflow() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let mut a = Ray::from(RAY);
        a.checked_sub_assign(&t.env, Ray::from(2 * RAY));
    });
}

#[test]
fn test_ray_checked_sub_assign_normal_case() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let mut a = Ray::from(5 * RAY);
        a.checked_sub_assign(&t.env, Ray::from(2 * RAY));
        assert_eq!(a, Ray::from(3 * RAY));
    });
}
