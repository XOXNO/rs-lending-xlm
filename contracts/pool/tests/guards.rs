extern crate std;

use super::*;
use crate::cache::Cache;
use crate::test_support::{hub, init_ledger};
use crate::{LiquidityPool, LiquidityPoolClient};
use common::constants::{
    BPS, LIQUIDATION_BUFFER_BPS, MAX_MARKET_VALUE_RAY, RAY, SUPPLY_INDEX_FLOOR_RAW,
};
use common::math::fp_core::mul_div_ceil;
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

#[test]
fn test_require_utilization_below_max_early_returns_when_max_util_ge_one() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let mut params = t.params.clone();
        params.max_utilization = RAY;
        let cache = cache_with(&t.env, &params, 10 * RAY, 11 * RAY, 0);
        require_utilization_below_max(&t.env, &cache);
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
        let cache = cache_with(&t.env, &t.params, 1_000 * RAY, 0, 1_000);
        let reserved = reserved_for(&t.env, &cache);
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
        let cache = cache_with(&t.env, &t.params, 1_000 * RAY, 0, 1_000);
        let reserved = reserved_for(&t.env, &cache);
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

/// One native unit of the 7-decimal test asset, in RAY.
const NATIVE_UNIT_RAY: i128 = RAY / 10_000_000;

/// Shortfall for claims and debt in tenths of a native unit (indexes at RAY)
/// and cash in native units.
fn shortfall_for(t: &TestSetup, claims_tenths: i128, debt_tenths: i128, cash: i128) -> i128 {
    t.as_contract(|| {
        let cache = cache_with(
            &t.env,
            &t.params,
            claims_tenths * NATIVE_UNIT_RAY / 10,
            debt_tenths * NATIVE_UNIT_RAY / 10,
            cash,
        );
        backing_shortfall(&cache)
    })
}

/// Claims 100.9, debt 50.1, cash 49: the exact gap is 1.8 units. Rounding
/// claims and debt to whole units before subtracting would read 0.
#[test]
fn test_backing_shortfall_floors_the_exact_gap_once() {
    let t = TestSetup::new();
    assert_eq!(shortfall_for(&t, 1_009, 501, 49), 1);
}

/// Accrual moves claims and debt by the same interest; the shortfall follows
/// the exact gap, not the fractional parts of either side.
#[test]
fn test_backing_shortfall_depends_only_on_the_exact_gap() {
    let t = TestSetup::new();
    assert_eq!(shortfall_for(&t, 1_011, 503, 49), 1);
    assert_eq!(shortfall_for(&t, 1_019, 511, 49), 1);
}

/// A gap below one native unit reads 0, so supply stays open.
#[test]
fn test_backing_shortfall_reads_zero_below_one_unit() {
    let t = TestSetup::new();
    assert_eq!(shortfall_for(&t, 1_009, 510, 49), 0);
    assert_eq!(shortfall_for(&t, 1_009, 519, 49), 0);
}

/// Debt value at or above claims leaves no shortfall, whatever the cash.
#[test]
fn test_backing_shortfall_is_zero_when_debt_covers_claims() {
    let t = TestSetup::new();
    assert_eq!(shortfall_for(&t, 1_000, 1_000, 0), 0);
    assert_eq!(shortfall_for(&t, 1_000, 1_001, 0), 0);
}

#[test]
fn test_require_market_value_within_ceiling_admits_both_totals_at_the_ceiling() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let cache = cache_with(
            &t.env,
            &t.params,
            MAX_MARKET_VALUE_RAY,
            MAX_MARKET_VALUE_RAY,
            0,
        );
        require_market_value_within_ceiling(&t.env, &cache);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #33)")]
fn test_require_market_value_within_ceiling_rejects_supply_value_above_it() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let cache = cache_with(&t.env, &t.params, MAX_MARKET_VALUE_RAY + 1, 0, 0);
        require_market_value_within_ceiling(&t.env, &cache);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #33)")]
fn test_require_market_value_within_ceiling_rejects_debt_value_above_it() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let cache = cache_with(&t.env, &t.params, RAY, MAX_MARKET_VALUE_RAY + 1, 0);
        require_market_value_within_ceiling(&t.env, &cache);
    });
}

// ---------------------------------------------------------------------------
// CR-1 residual (verify check pool-6), pinned. The floored RAY-precision
// shortfall is exact at the moment of recapitalization, but accrual can widen
// the RAY gap by up to two raw units per step, so a residue that sits within
// those units of a whole asset unit reopens a one-unit shortfall.
// ---------------------------------------------------------------------------

mod cr1_residual {
    extern crate std;

    use super::*;
    use crate::ops::{recapitalize, synced_market};
    use common::errors::CollateralError;
    use common::types::{HubAssetKey, PoolAction, PoolKey, PoolSupplyEntry, ScaledPositionRaw};
    use soroban_sdk::testutils::Ledger;
    use soroban_sdk::{vec, Error};

    /// The 18-decimal book found by seeded search: an exact recapitalization
    /// of a 7-unit shortfall, then one accrual of 14,200,455 s widens the RAY
    /// under-backing from 999,999,999 to 1,000,000,000 raw (one unit = 1e9).
    /// The shortfall reads one unit again, supply is refused with
    /// `PoolInsolvent`, and a second recapitalization applies that unit. The
    /// residual is about 2e-9 per recapitalization at 18 decimals and 2e-20 at
    /// 7; supply reopens after a one-unit top-up.
    #[test]
    fn exact_recapitalization_can_reopen_one_unit_after_accrual_at_18_decimals() {
        let t = TestSetup::new();
        let mut params = t.params.clone();
        params.asset_decimals = 18;
        LiquidityPoolClient::new(&t.env, &t.contract).create_market(&1u32, &params);
        let key = HubAssetKey {
            hub_id: 1,
            asset: params.asset_id.clone(),
        };
        let now_ms = t.env.ledger().timestamp() * 1_000;
        t.as_contract(|| {
            t.env.storage().persistent().set(
                &PoolKey::State(key.clone()),
                &PoolStateRaw {
                    supplied: 21_499_434_139_412_608_696_344_347_841,
                    borrowed: 4_595_072_133_136_371_792_993_694_569,
                    revenue: 0,
                    borrow_index: 1_854_542_708_114_033_910_434_688_508,
                    supply_index: RAY,
                    last_timestamp: now_ms,
                    cash: 12_977_676_621_646_551_166,
                },
            );
            assert_eq!(backing_shortfall(&Cache::load(&t.env, &key)), 7);
            let first = recapitalize::accounting(&t.env, key.clone(), 100);
            assert_eq!(first.mutation.actual_amount, 7, "exact recapitalization");
            assert_eq!(backing_shortfall(&Cache::load(&t.env, &key)), 0);
        });
        let gap_ray = |cache: &Cache| -> i128 {
            let claims = cache
                .supplied()
                .mul_floor(&t.env, cache.supply_index())
                .raw();
            let debt = cache.unscale_borrow_ceil_ray(cache.borrowed()).raw();
            claims - debt - cache.cash() * 1_000_000_000
        };
        let gap_before = t.as_contract(|| gap_ray(&Cache::load(&t.env, &key)));
        assert_eq!(gap_before, 999_999_999, "under one unit after the fill");

        t.env
            .ledger()
            .with_mut(|ledger| ledger.timestamp += 14_200_455);
        let reopened = t.as_contract(|| backing_shortfall(&synced_market(&t.env, &key)));
        let gap_after = t.as_contract(|| gap_ray(&synced_market(&t.env, &key)));
        assert_eq!(
            gap_after, 1_000_000_000,
            "one accrual widened the gap by 1 raw"
        );
        assert_eq!(reopened, 1, "the residual reopens one unit");

        let supply = PoolSupplyEntry {
            action: PoolAction {
                position: ScaledPositionRaw { scaled_amount: 0 },
                amount: 1_000_000_000_000_000_000,
                hub_asset: key.clone(),
            },
        };
        let client = LiquidityPoolClient::new(&t.env, &t.contract);
        let refused = client.try_supply(&vec![&t.env, supply.clone()]);
        assert_eq!(
            refused.err().and_then(|e| e.ok()),
            Some(Error::from_contract_error(
                CollateralError::PoolInsolvent as u32
            )),
            "supply is refused until the unit is topped up"
        );
        let second = t.as_contract(|| {
            recapitalize::accounting(&t.env, key.clone(), 100)
                .mutation
                .actual_amount
        });
        assert_eq!(second, 1, "a one-unit recapitalization closes it");
        assert!(
            client.try_supply(&vec![&t.env, supply]).is_ok(),
            "supply reopens after the top-up"
        );
    }
}
