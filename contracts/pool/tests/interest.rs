extern crate std;

use super::*;
use crate::test_support::{hub, init_ledger};
use crate::{LiquidityPool, LiquidityPoolClient};
use common::constants::RAY;
// The step primitives are no longer imported by `src/interest.rs` (it calls the
// shared `accrue_step`), but these tests rebuild the step by hand to check it.
use common::rates::{
    calculate_borrow_rate, calculate_supplier_rewards, compound_interest,
    supply_index_reward_shortfall, update_borrow_index, update_supply_index,
};
use common::types::{MarketParamsRaw, PoolKey, PoolStateRaw};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env};

struct TestSetup {
    env: Env,
    contract: Address,
    asset: Address,
}

impl TestSetup {
    fn new() -> Self {
        Self::with_decimals(7)
    }

    fn with_decimals(asset_decimals: u32) -> Self {
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
            asset_decimals,
        };
        let contract = env.register(LiquidityPool, (admin.clone(),));
        LiquidityPoolClient::new(&env, &contract).create_market(&0u32, &params);

        Self {
            env,
            contract,
            asset,
        }
    }

    fn as_contract<T>(&self, f: impl FnOnce() -> T) -> T {
        self.env.as_contract(&self.contract, f)
    }

    fn fresh_cache(&self, state: PoolStateRaw) -> Cache {
        self.env
            .storage()
            .persistent()
            .set(&PoolKey::State(hub(&self.asset)), &state);
        Cache::load(&self.env, &hub(&self.asset))
    }
}

#[test]
fn test_add_protocol_revenue_ray_zero_is_noop() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let mut cache = t.fresh_cache(PoolStateRaw {
            supplied: 100 * RAY,
            borrowed: 0,
            revenue: 0,
            borrow_index: RAY,
            supply_index: RAY,
            last_timestamp: 0,
            cash: 0,
        });
        let (rev_before, supp_before) = (cache.revenue(), cache.supplied());
        add_protocol_revenue(&mut cache, Ray::ZERO);
        assert_eq!(cache.revenue(), rev_before);
        assert_eq!(cache.supplied(), supp_before);
    });
}

#[test]
fn test_add_protocol_revenue_ray_books_fee_at_supply_index_floor() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let mut cache = t.fresh_cache(PoolStateRaw {
            supplied: 100 * RAY,
            borrowed: 0,
            revenue: 0,
            borrow_index: RAY,
            supply_index: SUPPLY_INDEX_FLOOR_RAW,
            last_timestamp: 0,
            cash: 0,
        });
        let (rev_before, supp_before) = (cache.revenue(), cache.supplied());

        let fee = Ray::from(1_000_000);
        add_protocol_revenue(&mut cache, fee);

        let minted = cache.revenue().checked_sub(&t.env, rev_before);
        assert!(minted.raw() > 0);
        assert_eq!(cache.supplied().checked_sub(&t.env, supp_before), minted);
    });
}

#[test]
fn test_apply_bad_debt_noop_when_total_supply_is_zero() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let mut cache = t.fresh_cache(PoolStateRaw {
            supplied: 0,
            borrowed: 0,
            revenue: 0,
            borrow_index: RAY,
            supply_index: RAY,
            last_timestamp: 0,
            cash: 0,
        });
        let index_before = cache.supply_index();
        apply_bad_debt_to_supply_index(&mut cache, Ray::from(5 * RAY));
        assert_eq!(cache.supply_index(), index_before);
    });
}

#[test]
fn test_apply_bad_debt_caps_at_total_supply_and_clamps_floor() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let mut cache = t.fresh_cache(PoolStateRaw {
            supplied: 10 * RAY,
            borrowed: 0,
            revenue: 0,
            borrow_index: RAY,
            supply_index: RAY,
            last_timestamp: 0,
            cash: 0,
        });

        apply_bad_debt_to_supply_index(&mut cache, Ray::from(100 * RAY));

        assert_eq!(
            cache.supply_index().raw(),
            SUPPLY_INDEX_FLOOR_RAW,
            "supply index must be clamped to floor"
        );
    });
}

#[test]
fn test_apply_bad_debt_applies_severe_reduction() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let mut cache = t.fresh_cache(PoolStateRaw {
            supplied: 1_000 * RAY,
            borrowed: 0,
            revenue: 0,
            borrow_index: RAY,

            supply_index: RAY,
            last_timestamp: 0,
            cash: 0,
        });
        let old_index = cache.supply_index().raw();

        apply_bad_debt_to_supply_index(&mut cache, Ray::from(910 * RAY));

        assert!(
            cache.supply_index().raw() < old_index / 10,
            "index should have dropped more than 10x"
        );
    });
}

#[test]
fn test_simulate_matches_global_sync_over_multi_year_delta() {
    use common::rates::simulate_update_indexes;
    use common::types::PoolSyncData;

    // Both accrual paths run the shared `accrue_step`, so they must agree on
    // indexes. They differ only in where the step's revenue shares land: the
    // pool books them on revenue *and* total supply, the simulator folds them
    // into supply alone. A starting revenue balance keeps that split honest.
    for (label, revenue) in [("no prior revenue", 0), ("prior revenue", 7 * RAY)] {
        let t = TestSetup::new();
        t.as_contract(|| {
            let state = PoolStateRaw {
                supplied: 100 * RAY,
                borrowed: 60 * RAY,
                revenue,
                borrow_index: RAY,
                supply_index: RAY,
                last_timestamp: 0,
                cash: 40_000_000,
            };
            let params: MarketParamsRaw = t
                .env
                .storage()
                .persistent()
                .get(&PoolKey::Params(hub(&t.asset)))
                .unwrap();
            let sync = PoolSyncData {
                params,
                state: state.clone(),
            };

            let mut cache = t.fresh_cache(state.clone());

            let delta_ms = 2 * MAX_COMPOUND_DELTA_MS + MAX_COMPOUND_DELTA_MS / 2;
            cache.set_current_timestamp(cache.last_timestamp() + delta_ms);
            let simulated = simulate_update_indexes(&t.env, cache.current_timestamp(), &sync);

            global_sync(&t.env, &mut cache);

            assert_eq!(
                cache.borrow_index().raw(),
                simulated.borrow_index.raw(),
                "{label}: read-path borrow index must equal mutating accrual"
            );
            assert_eq!(
                cache.supply_index().raw(),
                simulated.supply_index.raw(),
                "{label}: read-path supply index must equal mutating accrual"
            );

            // Accrual grows total supply only by minting revenue shares, so the
            // two deltas must match exactly. A plumbing slip in either caller
            // (double-crediting supply, or crediting revenue without supply)
            // shows up here even though the indexes still agree.
            assert_eq!(
                cache.supplied().raw() - state.supplied,
                cache.revenue().raw() - state.revenue,
                "{label}: supply growth from accrual must equal revenue minted"
            );
            assert!(
                cache.revenue().raw() > state.revenue,
                "{label}: a 2.5-year accrual at 60% utilization must mint revenue"
            );
        });
    }
}

#[test]
fn test_apply_bad_debt_mild_reduction_preserves_index_above_floor() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let mut cache = t.fresh_cache(PoolStateRaw {
            supplied: 1_000 * RAY,
            borrowed: 0,
            revenue: 0,
            borrow_index: RAY,
            supply_index: RAY,
            last_timestamp: 0,
            cash: 0,
        });
        let old_index = cache.supply_index().raw();

        apply_bad_debt_to_supply_index(&mut cache, Ray::from(100 * RAY));

        let new_index = cache.supply_index().raw();
        assert!(new_index > old_index / 10, "should be a mild reduction");
        assert!(new_index > SUPPLY_INDEX_FLOOR_RAW, "should be above floor");
        assert!(new_index < old_index, "should be reduced");
    });
}

#[test]
fn test_global_sync_respects_chunk_boundary() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let state = PoolStateRaw {
            supplied: 100 * RAY,
            borrowed: 60 * RAY,
            revenue: 0,
            borrow_index: RAY,
            supply_index: RAY,
            last_timestamp: 0,
            cash: 40_000_000,
        };
        let mut cache = t.fresh_cache(state);

        cache.set_current_timestamp(MAX_COMPOUND_DELTA_MS);
        global_sync(&t.env, &mut cache);
        assert!(cache.borrow_index().raw() > RAY);
    });
}

#[test]
fn test_apply_bad_debt_exactly_at_total_supplied_hits_cap_and_floor() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let mut cache = t.fresh_cache(PoolStateRaw {
            supplied: 100 * RAY,
            borrowed: 0,
            revenue: 0,
            borrow_index: RAY,
            supply_index: RAY,
            last_timestamp: 0,
            cash: 0,
        });
        apply_bad_debt_to_supply_index(&mut cache, Ray::from(100 * RAY));
        assert_eq!(cache.supply_index().raw(), SUPPLY_INDEX_FLOOR_RAW);
    });
}

#[test]
fn test_raw_cache_floor_residual_can_consume_fresh_cash_without_supply_guard() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let scaled_a_raw = 1_000_000 * RAY;
        let mut cache = t.fresh_cache(PoolStateRaw {
            supplied: scaled_a_raw,
            borrowed: 0,
            revenue: 0,
            borrow_index: RAY,
            supply_index: RAY,
            last_timestamp: 0,
            cash: 0,
        });
        let scaled_a = Ray::from(scaled_a_raw);

        apply_bad_debt_to_supply_index(&mut cache, Ray::from(2_000_000 * RAY));
        assert_eq!(
            cache.supply_index().raw(),
            SUPPLY_INDEX_FLOOR_RAW,
            "wipeout must clamp supply index UP to the floor, not reset the base"
        );

        let stranded = cache.unscale_supply_floor(scaled_a);
        assert!(stranded > 0, "floor clamp leaves userA a phantom claim");
        assert_eq!(cache.cash(), 0, "empty market: no cash to extract yet");

        let c = stranded;
        let scaled_b = cache.calculate_scaled_supply(c);
        cache.mint_supply(scaled_b);
        cache.credit_cash(c);

        let b_claim = cache.unscale_supply_floor(scaled_b);
        assert_eq!(b_claim, c, "userB's honest claim equals their deposit");

        let (burn, gross) = cache.resolve_withdrawal(i128::MAX, scaled_a);
        cache.require_reserves(gross);
        cache.burn_supply(burn);
        cache.debit_cash(gross);

        assert!(gross > 0, "stranded position pays out non-zero");
        assert_eq!(
            gross, c,
            "userA drains exactly userB's fresh deposit out of the pool"
        );

        assert!(
            cache.cash() < b_claim,
            "pool cash ({}) can no longer cover userB's claim ({}): honest supplier lost funds",
            cache.cash(),
            b_claim
        );
        assert_eq!(cache.cash(), 0, "userA drained the pool to empty");
    });
}

#[test]
fn test_raw_cache_floor_clamp_strands_claim_without_supply_guard() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let old_scaled_raw = 1_000 * RAY;
        let mut cache = t.fresh_cache(PoolStateRaw {
            supplied: old_scaled_raw,
            borrowed: 0,
            revenue: 0,
            borrow_index: RAY,
            supply_index: RAY,
            last_timestamp: 0,
            cash: 0,
        });
        let old_scaled = Ray::from(old_scaled_raw);

        apply_bad_debt_to_supply_index(&mut cache, Ray::from(5_000 * RAY));
        assert_eq!(
            cache.supply_index().raw(),
            SUPPLY_INDEX_FLOOR_RAW,
            "wipeout clamps supply_index UP to RAY/1000 instead of resetting shares to 0",
        );

        let stranded = cache.unscale_supply_floor(old_scaled);
        assert!(stranded > 0, "floor clamp leaves S_old a phantom claim");
        assert_eq!(
            cache.cash(),
            0,
            "no cash yet: invariant only masked by require_reserves"
        );

        let fresh_cash = stranded;
        let fresh_scaled = cache.calculate_scaled_supply(fresh_cash);
        cache.mint_supply(fresh_scaled);
        cache.credit_cash(fresh_cash);

        let fresh_claim = cache.unscale_supply_floor(fresh_scaled);
        assert_eq!(
            fresh_claim, fresh_cash,
            "fresh supplier's claim equals deposit"
        );

        let (burn, gross) = cache.resolve_withdrawal(i128::MAX, old_scaled);
        cache.require_reserves(gross);
        cache.burn_supply(burn);
        cache.debit_cash(gross);

        assert!(gross > 0, "stranded wiped position pays out real tokens");
        assert_eq!(gross, fresh_cash, "S_old drains exactly the fresh deposit");
        assert!(
            cache.cash() < fresh_claim,
            "pool cash ({}) can no longer cover fresh supplier claim ({}): funds lost",
            cache.cash(),
            fresh_claim,
        );
    });
}

#[test]
fn test_raw_cache_seizure_residual_would_drain_fresh_cash_without_supply_guard() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let alice_scaled_raw = 1_000 * RAY;
        let borrowed_scaled_raw = 1_000 * RAY;
        let mut cache = t.fresh_cache(PoolStateRaw {
            supplied: alice_scaled_raw,
            borrowed: borrowed_scaled_raw,
            revenue: 0,
            borrow_index: RAY,
            supply_index: RAY,
            last_timestamp: 0,
            cash: 0,
        });
        let alice_scaled = Ray::from(alice_scaled_raw);
        let borrow_scaled = Ray::from(borrowed_scaled_raw);

        let bad_debt = cache.unscale_borrow_ceil_ray(borrow_scaled);
        apply_bad_debt_to_supply_index(&mut cache, bad_debt);
        cache.burn_debt(borrow_scaled);

        assert_eq!(
            cache.supply_index().raw(),
            SUPPLY_INDEX_FLOOR_RAW,
            "seize wipeout clamps supply_index UP to RAY/1000, leaving unburned shares a residual"
        );

        let alice_stranded = cache.unscale_supply_floor(alice_scaled);
        assert!(alice_stranded > 0, "wiped survivor keeps a stranded claim");
        assert_eq!(
            cache.cash(),
            0,
            "empty market: claim masked by require_reserves"
        );

        let deposit = alice_stranded;
        let bob_scaled = cache.calculate_scaled_supply(deposit);
        cache.mint_supply(bob_scaled);
        cache.credit_cash(deposit);

        let total_owed = cache.unscale_supply_floor(cache.supplied());
        assert!(
            total_owed > cache.cash(),
            "post-deposit books insolvent: owed {} > cash {}",
            total_owed,
            cache.cash()
        );

        let (burn, gross) = cache.resolve_withdrawal(i128::MAX, alice_scaled);
        cache.require_reserves(gross);
        cache.burn_supply(burn);
        cache.debit_cash(gross);

        assert!(gross > 0, "wiped position pays out real cash");
        assert_eq!(gross, deposit, "Alice extracts exactly Bob's fresh deposit");

        let bob_claim = cache.unscale_supply_floor(bob_scaled);
        assert!(
            cache.cash() < bob_claim,
            "cash {} cannot cover Bob's honest claim {}: fresh depositor lost funds",
            cache.cash(),
            bob_claim
        );
    });
}

#[test]
fn test_global_sync_step_zero_borrowed_produces_zero_interest() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let mut cache = t.fresh_cache(PoolStateRaw {
            supplied: 100 * RAY,
            borrowed: 0,
            revenue: 0,
            borrow_index: RAY,
            supply_index: RAY,
            last_timestamp: 0,
            cash: 0,
        });
        let before = cache.supply_index();

        cache.set_current_timestamp(1_000);
        global_sync(&t.env, &mut cache);
        assert_eq!(cache.supply_index(), before);
    });
}

#[test]
fn test_global_sync_books_supplier_shortfall_as_protocol_revenue() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let state = PoolStateRaw {
            supplied: 100 * RAY,
            borrowed: 80 * RAY,
            revenue: 0,
            borrow_index: RAY,
            supply_index: RAY,
            last_timestamp: 0,
            cash: 20_000_000,
        };
        let mut cache = t.fresh_cache(state);
        cache.set_current_timestamp(MAX_COMPOUND_DELTA_MS);

        let old_borrow_index = cache.borrow_index();
        let old_supply_index = cache.supply_index();
        let util = cache.calculate_utilization();
        let rate = calculate_borrow_rate(&t.env, util, cache.params());
        let factor = compound_interest(&t.env, rate, MAX_COMPOUND_DELTA_MS);
        let new_borrow_index = update_borrow_index(&t.env, old_borrow_index, factor);
        let (supplier_rewards, reserve_fee) = calculate_supplier_rewards(
            &t.env,
            cache.params(),
            cache.borrowed(),
            new_borrow_index,
            old_borrow_index,
        );
        let new_supply_index =
            update_supply_index(&t.env, cache.supplied(), old_supply_index, supplier_rewards);
        let shortfall = supply_index_reward_shortfall(
            &t.env,
            cache.supplied(),
            old_supply_index,
            new_supply_index,
            supplier_rewards,
        );
        assert!(
            shortfall.raw() > 0,
            "floor-rounding leaves a residual shortfall booked to revenue"
        );
        let total_protocol_reward = reserve_fee.checked_add(&t.env, shortfall);
        let expected_revenue = protocol_fee_shares(
            &t.env,
            total_protocol_reward,
            new_supply_index,
            cache.supplied(),
        );
        let fee_only_revenue =
            protocol_fee_shares(&t.env, reserve_fee, new_supply_index, cache.supplied());

        global_sync(&t.env, &mut cache);

        assert_eq!(cache.borrow_index(), new_borrow_index);
        assert_eq!(cache.supply_index(), new_supply_index);
        assert_eq!(cache.revenue(), expected_revenue);
        assert!(cache.revenue().raw() > fee_only_revenue.raw());
        assert_eq!(cache.supplied().raw(), 100 * RAY + expected_revenue.raw());
    });
}

const DAY_MS: u64 = 86_400_000;

const DAYS_PER_YEAR: u32 = 365;

struct AccrualSnapshot {
    debt: i128,

    total_supply_claim: i128,

    user_claim: i128,

    revenue_claim: i128,
}

fn claim_ray(env: &Env, scaled: Ray, index: Ray) -> i128 {
    common::rates::scaled_to_original(env, scaled, index).raw()
}

fn snapshot(env: &Env, cache: &Cache, user_scaled: Ray) -> AccrualSnapshot {
    AccrualSnapshot {
        debt: claim_ray(env, cache.borrowed(), cache.borrow_index()),
        total_supply_claim: claim_ray(env, cache.supplied(), cache.supply_index()),
        user_claim: claim_ray(env, user_scaled, cache.supply_index()),
        revenue_claim: claim_ray(env, cache.revenue(), cache.supply_index()),
    }
}

struct YearDustReport {
    interest: i128,
    claims_growth: i128,
    user_growth: i128,
    revenue_claim: i128,
    dust: i128,
    debt_start: i128,
    debt_end: i128,
}

fn run_daily_year(
    env: &Env,
    cache: &mut Cache,
    user_scaled: Ray,
    days: u32,
) -> (AccrualSnapshot, AccrualSnapshot, YearDustReport) {
    env.cost_estimate().budget().reset_unlimited();

    let start = snapshot(env, cache, user_scaled);
    for _ in 0..days {
        cache.set_current_timestamp(cache.current_timestamp().saturating_add(DAY_MS));
        global_sync(env, cache);
    }
    let end = snapshot(env, cache, user_scaled);

    let interest = end.debt - start.debt;
    let claims_growth = end.total_supply_claim - start.total_supply_claim;
    let user_growth = end.user_claim - start.user_claim;

    let dust = interest - claims_growth;

    let report = YearDustReport {
        interest,
        claims_growth,
        user_growth,
        revenue_claim: end.revenue_claim,
        dust,
        debt_start: start.debt,
        debt_end: end.debt,
    };
    (start, end, report)
}

fn market_state(supplied_tokens: i128, util_bps: i128, cash_tokens: i128) -> PoolStateRaw {
    let supplied = supplied_tokens * RAY;
    let borrowed = supplied_tokens * util_bps / 10_000 * RAY;
    let cash = cash_tokens * 10_000_000;
    PoolStateRaw {
        supplied,
        borrowed,
        revenue: 0,
        borrow_index: RAY,
        supply_index: RAY,
        last_timestamp: 0,
        cash,
    }
}

#[test]
fn test_year_daily_accrual_deep_market_dust_is_tiny() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let supplied_tokens = 1_000_000_i128;
        let util_bps = 5_000;
        let state = market_state(supplied_tokens, util_bps, supplied_tokens / 2);
        let user_scaled = Ray::from(state.supplied);
        let mut cache = t.fresh_cache(state);

        let (_s, _end, r) = run_daily_year(&t.env, &mut cache, user_scaled, DAYS_PER_YEAR);

        assert!(r.interest > 0, "borrowers must pay interest over a year");
        assert!(r.debt_end > r.debt_start);
        assert!(r.user_growth > 0, "suppliers earn positive claim growth");
        assert!(r.revenue_claim > 0, "RF=10% must mint protocol revenue");

        assert!(
            r.claims_growth <= r.interest + RAY / 1_000,
            "claims_growth must not exceed interest beyond tiny rounding"
        );

        let dust_bps = r.dust.saturating_mul(10_000) / r.interest;
        assert!(
            (0..1).contains(&dust_bps),
            "deep-market dust must be < 1 bps of interest, got {dust_bps} bps (dust={})",
            r.dust
        );

        let attributed = r.user_growth + r.revenue_claim;
        let recon = r.interest - attributed;

        assert!(
            recon.abs() < 2 * RAY || r.dust >= 0,
            "interest vs user+revenue recon out of band: recon={recon}"
        );
    });
}

#[test]
fn test_year_daily_accrual_medium_market_reports_dust() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let supplied_tokens = 10_000_i128;
        let util_bps = 8_000;
        let state = market_state(supplied_tokens, util_bps, supplied_tokens * 2 / 10);
        let user_scaled = Ray::from(state.supplied);
        let mut cache = t.fresh_cache(state);

        let (_s, _end, r) = run_daily_year(&t.env, &mut cache, user_scaled, DAYS_PER_YEAR);

        assert!(r.interest > 0);
        assert!(r.revenue_claim > 0);
        assert!(r.user_growth > 0);

        assert!(
            r.dust >= -RAY / 100,
            "dust should not be largely negative, got {}",
            r.dust
        );
        let dust_bps = if r.interest > 0 {
            r.dust.saturating_mul(10_000) / r.interest
        } else {
            0
        };

        assert!(
            dust_bps < 50,
            "medium-market dust unexpectedly large: {dust_bps} bps"
        );
    });
}

#[test]
fn test_year_daily_accrual_thin_market_higher_relative_dust() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let supplied_tokens = 100_i128;
        let util_bps = 8_000;
        let state = market_state(supplied_tokens, util_bps, 20);
        let user_scaled = Ray::from(state.supplied);
        let mut cache = t.fresh_cache(state);

        let (_s, _end, r) = run_daily_year(&t.env, &mut cache, user_scaled, DAYS_PER_YEAR);

        assert!(r.interest > 0);
        assert!(r.user_growth > 0);
        assert!(r.revenue_claim > 0);

        let dust_bps = r.dust.saturating_mul(10_000) / r.interest;
        assert!(
            dust_bps < 200,
            "thin-market dust should stay under 2% of interest, got {dust_bps} bps"
        );
        assert!(
            r.claims_growth + RAY >= r.user_growth,
            "total claims must cover at least user growth (fee shares extra)"
        );
    });
}

#[test]
fn test_year_daily_accrual_usdc_millions_scale() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let cases: [(&str, i128, i128); 3] = [
            ("USDC 5M supply / 50% util", 5_000_000, 5_000),
            ("USDC 20M supply / 80% util", 20_000_000, 8_000),
            ("USDC 100M supply / 70% util", 100_000_000, 7_000),
        ];

        for (label, supplied_tokens, util_bps) in cases {
            let free = supplied_tokens - supplied_tokens * util_bps / 10_000;
            let state = market_state(supplied_tokens, util_bps, free);
            let user_scaled = Ray::from(state.supplied);
            let mut cache = t.fresh_cache(state);
            let (_s, _end, r) = run_daily_year(&t.env, &mut cache, user_scaled, DAYS_PER_YEAR);

            assert!(r.interest > 0);
            assert!(r.user_growth > 0);
            assert!(r.revenue_claim > 0);

            assert!(
                r.dust.abs() < 10 * RAY,
                "{label}: absolute dust should be << 10 USDC, got raw {}",
                r.dust
            );
            let dust_bps = r.dust.saturating_mul(10_000) / r.interest;
            assert!(
                dust_bps < 1,
                "{label}: relative dust must be < 1 bps at millions TVL, got {dust_bps}"
            );
        }
    });
}

#[test]
fn test_year_daily_vs_single_sync_dust_comparison() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let supplied_tokens = 100_000_i128;
        let util_bps = 6_000;
        let state = market_state(supplied_tokens, util_bps, 40_000);
        let user_scaled = Ray::from(state.supplied);
        t.env.cost_estimate().budget().reset_unlimited();

        let mut daily = t.fresh_cache(state);
        let start_d = snapshot(&t.env, &daily, user_scaled);
        for _ in 0..DAYS_PER_YEAR {
            daily.set_current_timestamp(daily.current_timestamp().saturating_add(DAY_MS));
            global_sync(&t.env, &mut daily);
        }
        let end_d = snapshot(&t.env, &daily, user_scaled);
        let interest_d = end_d.debt - start_d.debt;
        let claims_d = end_d.total_supply_claim - start_d.total_supply_claim;
        let dust_d = interest_d - claims_d;

        let mut once = t.fresh_cache(market_state(supplied_tokens, util_bps, 40_000));
        let start_o = snapshot(&t.env, &once, user_scaled);
        once.set_current_timestamp(
            once.current_timestamp()
                .saturating_add(DAY_MS.saturating_mul(DAYS_PER_YEAR as u64)),
        );
        global_sync(&t.env, &mut once);
        let end_o = snapshot(&t.env, &once, user_scaled);
        let interest_o = end_o.debt - start_o.debt;
        let claims_o = end_o.total_supply_claim - start_o.total_supply_claim;
        let dust_o = interest_o - claims_o;

        assert!(interest_d > 0 && interest_o > 0);

        assert!(end_d.revenue_claim > 0 && end_o.revenue_claim > 0);
        assert!(end_d.user_claim > start_d.user_claim);
        assert!(end_o.user_claim > start_o.user_claim);

        assert!(dust_d > -RAY && dust_o > -RAY);
        let max_interest = interest_d.max(interest_o);
        assert!(
            dust_d.abs() < max_interest / 50 && dust_o.abs() < max_interest / 50,
            "dust should stay under 2% of interest on both cadences"
        );
    });
}

// ---------------------------------------------------------------------------
// Accrual-cadence value leakage (CS-AAVE4-004 analogue)
//
// ChainSecurity found that Aave V4 lost the whole protocol fee when `accrue()`
// ran every second: the fee was floored to zero before the reserve factor was
// applied. `update_indexes` here is permissionless, so an attacker picks the
// cadence. These tests run the SAME elapsed span three ways (one accrual at the
// end / one per ~5s ledger / one per second) and measure where the value lands.
// ---------------------------------------------------------------------------

const SECOND_MS: u64 = 1_000;

/// Stellar closes a ledger roughly every 5 seconds.
const LEDGER_MS: u64 = 5_000;

const YEAR_MS: i128 = common::constants::MILLISECONDS_PER_YEAR as i128;

/// Span measured by the cadence harness. One day keeps the per-second path at
/// 86_400 accruals, which is the finest cadence a caller can reach on-chain and
/// still fits a unit test; results are annualized for reporting.
const CADENCE_SPAN_MS: u64 = DAY_MS;

/// A market sized to mirror ChainSecurity's scenario: ~$1M of borrowed
/// principal at ~10% APR (utilization 45% on this repo's default curve:
/// base 1% + slope1 10% ramped over mid_utilization 50%).
struct CadenceMarket {
    label: &'static str,
    /// Pool supports `0..=WAD_DECIMALS` (18); 7 is the Stellar native scale.
    decimals: u32,
    supplied_units: i128,
    borrowed_units: i128,
}

/// 45% utilization => 10% APR on the `TestSetup` curve.
const CADENCE_MARKETS: [CadenceMarket; 2] = [
    CadenceMarket {
        label: "7dp stablecoin: 1_000_000 borrowed / 2_222_222 supplied (~$1M @ ~10% APR)",
        decimals: 7,
        supplied_units: 22_222_222_222_222,
        borrowed_units: 10_000_000_000_000,
    },
    CadenceMarket {
        label: "8dp WBTC-like: 10 borrowed / 22.22222222 supplied (~$1M @ ~10% APR)",
        decimals: 8,
        supplied_units: 2_222_222_222,
        borrowed_units: 1_000_000_000,
    },
];

impl CadenceMarket {
    fn state(&self) -> PoolStateRaw {
        let env = Env::default();
        PoolStateRaw {
            supplied: Ray::from_asset(&env, self.supplied_units, self.decimals).raw(),
            borrowed: Ray::from_asset(&env, self.borrowed_units, self.decimals).raw(),
            revenue: 0,
            borrow_index: RAY,
            supply_index: RAY,
            last_timestamp: 0,
            cash: self.supplied_units - self.borrowed_units,
        }
    }
}

/// Terminal state of one accrual cadence over a fixed span.
struct CadenceResult {
    label: &'static str,
    steps: u64,
    revenue_scaled: i128,
    /// Withdrawable supplier claim, floor-rounded to asset units.
    supplier_claim_units: i128,
    /// Claimable protocol revenue, floor-rounded to asset units.
    revenue_claim_units: i128,
    /// Same three quantities at full RAY precision, to expose sub-unit drift.
    supplier_claim_ray: i128,
    revenue_claim_ray: i128,
    debt_ray: i128,
    /// Interest the borrowers owe over the span, in ray.
    interest_ray: i128,
    /// Interest that landed somewhere claimable (suppliers + treasury), in ray.
    attributed_ray: i128,
}

impl CadenceResult {
    /// Interest that borrowers owe but nobody can claim — the rounding residual
    /// left behind by this cadence. Positive means the books are conservative
    /// (claims under-count debt); negative would mean claims were inflated
    /// beyond the interest actually charged.
    fn unattributed_ray(&self) -> i128 {
        self.interest_ray - self.attributed_ray
    }
}

impl CadenceResult {
    /// Everything the pool owes out: suppliers plus unclaimed protocol revenue.
    fn booked_units(&self) -> i128 {
        self.supplier_claim_units + self.revenue_claim_units
    }

    fn booked_ray(&self) -> i128 {
        self.supplier_claim_ray + self.revenue_claim_ray
    }
}

/// Accrues `total_ms` in `step_ms` slices through the real mutating path
/// (`global_sync`) and reports the terminal split.
///
/// Only the initial `Cache::load` touches storage, so it is the only part that
/// runs inside a contract invocation frame; the accrual loop is pure math on
/// the cache. Keeping the loop outside the frame avoids tripping the SDK's
/// per-invocation mainnet resource limits, which a real caller would never hit
/// because each on-chain `update_indexes` is its own transaction.
fn run_cadence(
    t: &TestSetup,
    market: &CadenceMarket,
    label: &'static str,
    total_ms: u64,
    step_ms: u64,
) -> CadenceResult {
    t.env.cost_estimate().budget().reset_unlimited();

    let start = market.state();
    // Both indexes start at RAY and revenue at zero, so the scaled figures are
    // already the opening claim and debt in ray.
    assert_eq!(start.supply_index, RAY);
    assert_eq!(start.borrow_index, RAY);
    assert_eq!(start.revenue, 0);
    let (start_supplier_ray, start_debt_ray) = (start.supplied, start.borrowed);

    let mut cache = t.as_contract(|| t.fresh_cache(start));
    cache.set_current_timestamp(0);

    let mut steps = 0u64;
    let mut elapsed = 0u64;
    while elapsed < total_ms {
        elapsed = elapsed.saturating_add(step_ms).min(total_ms);
        cache.set_current_timestamp(elapsed);
        global_sync(&t.env, &mut cache);
        steps += 1;
    }

    let supplier_scaled = cache.supplied().checked_sub(&t.env, cache.revenue());
    let supplier_claim_ray = supplier_scaled
        .mul_floor(&t.env, cache.supply_index())
        .raw();
    let revenue_claim_ray = cache
        .revenue()
        .mul_floor(&t.env, cache.supply_index())
        .raw();
    let debt_ray = cache
        .borrowed()
        .mul_ceil(&t.env, cache.borrow_index())
        .raw();

    CadenceResult {
        label,
        steps,
        revenue_scaled: cache.revenue().raw(),
        supplier_claim_units: cache.unscale_supply_floor(supplier_scaled),
        revenue_claim_units: cache.unscale_supply_floor(cache.revenue()),
        supplier_claim_ray,
        revenue_claim_ray,
        debt_ray,
        interest_ray: debt_ray - start_debt_ray,
        attributed_ray: (supplier_claim_ray - start_supplier_ray) + revenue_claim_ray,
    }
}

/// Runs every cadence in `cadences` over the same `span_ms` and asserts the
/// security property against `cadences[0]` (the single terminal accrual).
fn assert_cadence_never_leaks(
    market: &CadenceMarket,
    span_ms: u64,
    cadences: &[(&'static str, u64)],
) {
    let t = TestSetup::with_decimals(market.decimals);

    let mut results = std::vec::Vec::with_capacity(cadences.len());
    for (label, step_ms) in cadences {
        results.push(run_cadence(&t, market, label, span_ms, *step_ms));
    }
    let base = &results[0];
    assert_eq!(
        base.steps, 1,
        "{}: baseline must be one accrual",
        market.label
    );

    for r in results.iter().skip(1) {
        assert!(
            r.supplier_claim_units >= base.supplier_claim_units,
            "{}: cadence '{}' ({} accruals over {} ms) left suppliers SHORT by {} units \
             vs a single accrual ({} < {}) — accrual frequency drains suppliers",
            market.label,
            r.label,
            r.steps,
            span_ms,
            base.supplier_claim_units - r.supplier_claim_units,
            r.supplier_claim_units,
            base.supplier_claim_units,
        );
        assert!(
            r.booked_units() >= base.booked_units(),
            "{}: cadence '{}' ({} accruals over {} ms) booked {} fewer units in total \
             (suppliers + treasury) than a single accrual ({} < {}) — value vanished",
            market.label,
            r.label,
            r.steps,
            span_ms,
            base.booked_units() - r.booked_units(),
            r.booked_units(),
            base.booked_units(),
        );

        // Same direction at full RAY precision, so a sub-unit drain cannot hide
        // under the asset-decimal floor.
        assert!(
            r.supplier_claim_ray >= base.supplier_claim_ray,
            "{}: cadence '{}' left suppliers short by {} ray",
            market.label,
            r.label,
            base.supplier_claim_ray - r.supplier_claim_ray,
        );
        assert!(
            r.booked_ray() >= base.booked_ray(),
            "{}: cadence '{}' booked {} fewer ray in total",
            market.label,
            r.label,
            base.booked_ray() - r.booked_ray(),
        );
    }

    // Residual: interest charged to borrowers that nobody can claim. It must
    // never go negative (that would mean claims outran the interest actually
    // charged, i.e. the pool minted value) and must stay negligible.
    for r in results.iter() {
        let residual = r.unattributed_ray();
        assert!(
            residual >= 0,
            "{}: cadence '{}' attributed {} ray more than borrowers were charged — \
             claims inflated beyond interest",
            market.label,
            r.label,
            -residual,
        );
        // The residual is bounded by the number of accruals, not by the size of
        // the book: each accrual strands at most ~0.5 ray (1e-27 of a token),
        // regardless of a $1M or $1B position. Measured: 43_208 ray over 86_400
        // per-second accruals on a $1M book.
        assert!(
            residual <= (r.steps as i128) + 8,
            "{}: cadence '{}' stranded {} ray over {} accruals — more than ~1 ray per accrual, \
             so the residual is scaling with something other than accrual count",
            market.label,
            r.label,
            residual,
            r.steps,
        );
    }
}

/// The security property: no accrual cadence may leave suppliers — or the
/// suppliers-plus-treasury total — worse off than a single terminal accrual.
///
/// This is the inverse of CS-AAVE4-004: there, sub-second accrual floored the
/// protocol fee to zero. Here the per-step fee stays non-zero because the split
/// happens in RAY (27dp) space, and every rounding residual that cannot lift the
/// supply index is re-booked as protocol revenue by
/// `supply_index_reward_shortfall`.
///
/// One-day horizon, down to the finest cadence a caller can reach (1 s).
#[test]
fn test_accrual_cadence_never_leaks_supplier_or_total_value() {
    let cadences = [
        ("single (terminal)", DAY_MS),
        ("per ledger (~5s)", LEDGER_MS),
        ("per second", SECOND_MS),
    ];
    for market in &CADENCE_MARKETS {
        assert_cadence_never_leaks(market, CADENCE_SPAN_MS, &cadences);
    }
}

/// Same property over a full year, where utilization drift and repeated
/// compounding have time to accumulate. `MAX_COMPOUND_DELTA_MS` is one year, so
/// the baseline here is still a single compounding chunk. Per-second cadence is
/// not runnable at this horizon (31.5M accruals), so this covers hourly and
/// daily; the one-day test covers the sub-minute end.
#[test]
fn test_year_horizon_accrual_cadence_never_leaks_supplier_or_total_value() {
    let year_ms = YEAR_MS as u64;
    let cadences = [
        ("single (terminal)", year_ms),
        ("daily", DAY_MS),
        ("hourly", 3_600_000),
    ];
    for market in &CADENCE_MARKETS {
        assert_cadence_never_leaks(market, year_ms, &cadences);
    }
}

/// CS-AAVE4-004's actual failure mode: the protocol fee rounding to zero under
/// a fast cadence. Measures the treasury's capture rate at each cadence and
/// requires it to stay near the 10% reserve factor.
#[test]
fn test_frequent_accrual_does_not_round_protocol_fee_to_zero() {
    for market in &CADENCE_MARKETS {
        let t = TestSetup::with_decimals(market.decimals);

        // Indexes start at RAY, so the scaled debt is the debt in ray units.
        let start_debt = market.state().borrowed;

        let cadences: [(&'static str, u64); 3] = [
            ("single (terminal)", DAY_MS),
            ("per ledger (~5s)", LEDGER_MS),
            ("per second", SECOND_MS),
        ];

        for (label, step_ms) in cadences {
            let r = run_cadence(&t, market, label, CADENCE_SPAN_MS, step_ms);
            let interest = r.debt_ray - start_debt;
            assert!(
                interest > 0,
                "{}: {label} accrued no interest",
                market.label
            );

            let capture_bps = r.revenue_claim_ray.saturating_mul(10_000) / interest;
            assert!(
                r.revenue_scaled > 0,
                "{}: {label} minted zero protocol revenue shares — CS-AAVE4-004 repeat",
                market.label
            );
            assert!(
                (990..=1_010).contains(&capture_bps),
                "{}: {label} treasury captured {capture_bps} bps of interest, \
                 expected ~1000 bps (reserve factor)",
                market.label
            );
        }
    }
}
