extern crate std;

use super::*;
use crate::cache::Cache;
use crate::test_support::{hub, init_ledger};
use crate::{LiquidityPool, LiquidityPoolClient};
use common::constants::{
    BPS, LIQUIDATION_BUFFER_BPS, MAX_MARKET_ENTRY_RAY, RAY, SUPPLY_INDEX_FLOOR_RAW,
};
use common::math::reference::mul_div_ceil;
use common::types::{MarketParamsRaw, PoolStateRaw};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env};

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
            params,
        }
    }

    fn as_contract<T>(&self, f: impl FnOnce() -> T) -> T {
        self.env.as_contract(&self.contract, f)
    }
}

fn cache_with(
    env: &Env,
    params: &MarketParamsRaw,
    supplied: i128,
    borrowed: i128,
    cash: i128,
) -> Cache {
    Cache::from_parts(
        env,
        hub(&params.asset_id),
        params,
        &PoolStateRaw {
            supplied,
            borrowed,
            revenue: 0,
            borrow_index: RAY,
            supply_index: RAY,
            last_timestamp: 0,
            cash,
        },
        1_000_000,
    )
}

#[test]
fn test_require_utilization_below_max_early_returns_when_supplied_zero() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let cache = cache_with(&t.env, &t.params, 0, 100 * RAY, 0);
        require_utilization_below_max(&t.env, &cache);
    });
}

/// A 100% ceiling still binds a debt mint: debt value may not exceed supply
/// value (audit finding S-03).
#[test]
#[should_panic(expected = "Error(Contract, #127)")]
fn test_require_utilization_below_max_rejects_debt_above_supply_at_a_full_cap() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let mut params = t.params.clone();
        params.max_utilization = RAY;
        let cache = cache_with(&t.env, &params, 10 * RAY, 10 * RAY + 1, 0);
        require_utilization_below_max(&t.env, &cache);
    });
}

#[test]
fn test_require_utilization_below_max_admits_debt_equal_to_supply_at_a_full_cap() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let mut params = t.params.clone();
        params.max_utilization = RAY;
        let cache = cache_with(&t.env, &params, 10 * RAY, 10 * RAY, 0);
        require_utilization_below_max(&t.env, &cache);
    });
}

/// The audit's S-03 state: sub-unit dust supply against whole tokens of debt,
/// at a full cap. The old gate skipped a full cap; the comparison refuses it.
#[test]
#[should_panic(expected = "Error(Contract, #127)")]
fn test_require_utilization_below_max_rejects_debt_against_dust_supply_at_a_full_cap() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let mut params = t.params.clone();
        params.max_utilization = RAY;
        let cache = cache_with(&t.env, &params, 7_674, 1_000_000_000 * RAY, 0);
        require_utilization_below_max(&t.env, &cache);
    });
}

/// The same state below a full cap. The old gate divided debt by supply here
/// and overflowed with `MathOverflow`; the comparison gives the typed error.
#[test]
#[should_panic(expected = "Error(Contract, #127)")]
fn test_require_utilization_below_max_rejects_debt_against_dust_supply_without_overflow() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let cache = cache_with(&t.env, &t.params, 7_674, 1_000_000_000 * RAY, 0);
        require_utilization_below_max(&t.env, &cache);
    });
}

/// The exit gate skips a full cap: an exit is bounded by cash, and the
/// comparison would only fail on rounding dust.
#[test]
fn test_require_exit_utilization_below_max_skips_a_full_cap() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let mut params = t.params.clone();
        params.max_utilization = RAY;
        let cache = cache_with(&t.env, &params, 10 * RAY, 10 * RAY + 1, 0);
        require_exit_utilization_below_max(&t.env, &cache);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #127)")]
fn test_require_exit_utilization_below_max_enforces_a_cap_below_full() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let cache = cache_with(&t.env, &t.params, 10 * RAY, 95 * RAY / 10 + 1, 0);
        require_exit_utilization_below_max(&t.env, &cache);
    });
}

#[test]
fn test_require_exit_utilization_below_max_admits_exactly_the_cap() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let cache = cache_with(&t.env, &t.params, 10 * RAY, 95 * RAY / 10, 0);
        require_exit_utilization_below_max(&t.env, &cache);
    });
}

/// The allowed debt value rounds down: 7 raw units of supply at a 95% cap
/// allow `floor(6.65) = 6` units of debt.
#[test]
fn test_require_utilization_below_max_admits_the_floored_allowance() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let cache = cache_with(&t.env, &t.params, 7, 6, 0);
        require_utilization_below_max(&t.env, &cache);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #127)")]
fn test_require_utilization_below_max_rejects_one_unit_above_the_floored_allowance() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let cache = cache_with(&t.env, &t.params, 7, 7, 0);
        require_utilization_below_max(&t.env, &cache);
    });
}

fn cache_at_index(
    t: &TestSetup,
    supplied: i128,
    supply_index: i128,
    borrowed: i128,
    borrow_index: i128,
) -> Cache {
    Cache::from_parts(
        &t.env,
        hub(&t.params.asset_id),
        &t.params,
        &PoolStateRaw {
            supplied,
            borrowed,
            revenue: 0,
            borrow_index,
            supply_index,
            last_timestamp: 0,
            cash: 0,
        },
        1_000_000,
    )
}

#[test]
fn test_require_within_entry_ceiling_admits_both_totals_at_the_ceiling() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let cache = cache_at_index(&t, MAX_MARKET_ENTRY_RAY, RAY, MAX_MARKET_ENTRY_RAY, RAY);
        require_within_entry_ceiling(&t.env, &cache);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #136)")]
fn test_require_within_entry_ceiling_rejects_supply_value_one_unit_above() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let cache = cache_at_index(&t, MAX_MARKET_ENTRY_RAY + 1, RAY, 0, RAY);
        require_within_entry_ceiling(&t.env, &cache);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #136)")]
fn test_require_within_entry_ceiling_rejects_debt_value_one_unit_above() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let cache = cache_at_index(&t, 0, RAY, MAX_MARKET_ENTRY_RAY + 1, RAY);
        require_within_entry_ceiling(&t.env, &cache);
    });
}

/// Shares under the ceiling whose value at a grown index is over it.
#[test]
#[should_panic(expected = "Error(Contract, #136)")]
fn test_require_within_entry_ceiling_rejects_a_value_grown_past_the_ceiling() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let cache = cache_at_index(&t, MAX_MARKET_ENTRY_RAY / 2 + 1, 2 * RAY, 0, RAY);
        require_within_entry_ceiling(&t.env, &cache);
    });
}

/// At the supply-index floor a value far below the ceiling carries a share
/// count 1,000 times larger; the share count is what the guard rejects.
#[test]
#[should_panic(expected = "Error(Contract, #136)")]
fn test_require_within_entry_ceiling_rejects_a_share_count_above_at_the_index_floor() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let cache = cache_at_index(&t, MAX_MARKET_ENTRY_RAY + 1, SUPPLY_INDEX_FLOOR_RAW, 0, RAY);
        require_within_entry_ceiling(&t.env, &cache);
    });
}

#[test]
fn test_require_within_entry_ceiling_admits_the_share_ceiling_at_the_index_floor() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let cache = cache_at_index(&t, MAX_MARKET_ENTRY_RAY, SUPPLY_INDEX_FLOOR_RAW, 0, RAY);
        require_within_entry_ceiling(&t.env, &cache);
    });
}

/// A value that does not fit `i128` fails the guard with the typed error
/// instead of overflowing.
#[test]
#[should_panic(expected = "Error(Contract, #136)")]
fn test_require_within_entry_ceiling_rejects_an_unrepresentable_value() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let cache = cache_at_index(&t, MAX_MARKET_ENTRY_RAY, 1_000_000 * RAY, 0, RAY);
        require_within_entry_ceiling(&t.env, &cache);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #127)")]
fn test_require_utilization_below_max_panics_when_above() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let cache = cache_with(&t.env, &t.params, 10 * RAY, 10 * RAY, 0);
        require_utilization_below_max(&t.env, &cache);
    });
}

#[test]
fn test_require_supply_for_debt_happy() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let cache = cache_with(&t.env, &t.params, 10 * RAY, 5 * RAY, 0);
        require_supply_for_debt(&t.env, &cache);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #123)")]
fn test_require_supply_for_debt_rejects_debt_without_supply() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let cache = cache_with(&t.env, &t.params, 0, RAY, 0);
        require_supply_for_debt(&t.env, &cache);
    });
}

/// Cash the liquidation buffer holds back for `cache`, as a token amount.
fn reserved_for(env: &Env, cache: &Cache) -> i128 {
    let supplied = cache.unscale_supply_floor(cache.supplied());
    mul_div_ceil(env, supplied, LIQUIDATION_BUFFER_BPS, BPS)
}

/// An ordinary draw may take the market down to the buffer, but not through it.
#[test]
fn test_require_liquidation_buffer_admits_a_draw_down_to_the_reserve() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let supplied = Ray::from_asset(&t.env, 1_000, t.params.asset_decimals).raw();
        let cache = cache_with(&t.env, &t.params, supplied, 0, 1_000);
        let reserved = reserved_for(&t.env, &cache);
        assert_eq!(reserved, 20);
        assert!(
            reserved > 0,
            "fixture must reserve something to be meaningful"
        );
        require_liquidation_buffer(&t.env, &cache, 1_000 - reserved);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #112)")]
fn test_require_liquidation_buffer_rejects_a_draw_one_unit_past_the_reserve() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let supplied = Ray::from_asset(&t.env, 1_000, t.params.asset_decimals).raw();
        let cache = cache_with(&t.env, &t.params, supplied, 0, 1_000);
        let reserved = reserved_for(&t.env, &cache);
        assert_eq!(reserved, 20);
        require_liquidation_buffer(&t.env, &cache, 1_000 - reserved + 1);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #127)")]
fn test_require_utilization_below_max_rejects_one_raw_ray_above_the_cap() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let cache = cache_with(&t.env, &t.params, 10 * RAY, 95 * RAY / 10 + 1, 0);
        require_utilization_below_max(&t.env, &cache);
    });
}

/// One scaled unit at the supply-index floor is worth less than one raw RAY unit.
#[test]
#[should_panic(expected = "Error(Contract, #127)")]
fn test_require_utilization_below_max_rejects_debt_against_a_zero_floored_supply_value() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let cache = Cache::from_parts(
            &t.env,
            hub(&t.params.asset_id),
            &t.params,
            &PoolStateRaw {
                supplied: 1,
                borrowed: 1,
                revenue: 0,
                borrow_index: RAY,
                supply_index: SUPPLY_INDEX_FLOOR_RAW,
                last_timestamp: 0,
                cash: 0,
            },
            1_000_000,
        );
        require_utilization_below_max(&t.env, &cache);
    });
}

#[test]
fn test_require_utilization_below_max_admits_no_debt_against_a_zero_floored_supply_value() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let cache = Cache::from_parts(
            &t.env,
            hub(&t.params.asset_id),
            &t.params,
            &PoolStateRaw {
                supplied: 1,
                borrowed: 0,
                revenue: 0,
                borrow_index: RAY,
                supply_index: SUPPLY_INDEX_FLOOR_RAW,
                last_timestamp: 0,
                cash: 0,
            },
            1_000_000,
        );
        require_utilization_below_max(&t.env, &cache);
    });
}

#[test]
fn test_require_utilization_below_max_admits_exactly_the_cap() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let cache = cache_with(&t.env, &t.params, 10 * RAY, 95 * RAY / 10, 0);
        require_utilization_below_max(&t.env, &cache);
    });
}

/// 200 BPS of 1,020 units is 20.4 units, so the reserve is 21 units.
#[test]
#[should_panic(expected = "Error(Contract, #112)")]
fn test_require_liquidation_buffer_rounds_a_fractional_reserve_up() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let cache = cache_with(&t.env, &t.params, 1_020 * RAY / 10_000_000, 0, 1_020);
        require_liquidation_buffer(&t.env, &cache, 1_000);
    });
}

#[test]
fn test_require_liquidation_buffer_admits_cash_at_the_rounded_up_reserve() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let cache = cache_with(&t.env, &t.params, 1_020 * RAY / 10_000_000, 0, 1_020);
        require_liquidation_buffer(&t.env, &cache, 999);
    });
}

/// Claims 100.9 units, debt 50.1, cash 49: the exact gap is 1.8 units, so the
/// shortfall is 1. Rounding each side to whole units first would read 0.
#[test]
fn test_backing_shortfall_floors_the_exact_gap_once() {
    let t = TestSetup::new();
    let tenth = RAY / 100_000_000;
    t.as_contract(|| {
        let cache = cache_with(&t.env, &t.params, 1_009 * tenth, 501 * tenth, 49);
        assert_eq!(backing_shortfall(&cache), 1);
    });
}
