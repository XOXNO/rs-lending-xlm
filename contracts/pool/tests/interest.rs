extern crate std;

use super::*;
use crate::test_support::{hub, init_ledger};
use crate::{LiquidityPool, LiquidityPoolClient};
use common::constants::RAY;
use common::types::{MarketParamsRaw, PoolKey, PoolStateRaw};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env};

const VERBOSE_YEAR_ACCRUAL: bool = false;

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

        let mut cache = t.fresh_cache(state);

        let delta_ms = 2 * MAX_COMPOUND_DELTA_MS + MAX_COMPOUND_DELTA_MS / 2;
        cache.set_current_timestamp(cache.last_timestamp() + delta_ms);
        let simulated = simulate_update_indexes(&t.env, cache.current_timestamp(), &sync);

        global_sync(&t.env, &mut cache);

        assert_eq!(
            cache.borrow_index().raw(),
            simulated.borrow_index.raw(),
            "read-path borrow index must equal mutating accrual"
        );
        assert_eq!(
            cache.supply_index().raw(),
            simulated.supply_index.raw(),
            "read-path supply index must equal mutating accrual"
        );
    });
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
    borrow_index: i128,
    supply_index: i128,
    supplied_scaled: i128,
    revenue_scaled: i128,
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
        borrow_index: cache.borrow_index().raw(),
        supply_index: cache.supply_index().raw(),
        supplied_scaled: cache.supplied().raw(),
        revenue_scaled: cache.revenue().raw(),
    }
}

struct YearDustReport {
    label: &'static str,
    util_bps: i128,
    interest: i128,
    claims_growth: i128,
    user_growth: i128,
    revenue_claim: i128,
    dust: i128,
    debt_start: i128,
    debt_end: i128,
    user_claim_start: i128,
    user_claim_end: i128,
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
        label: "",
        util_bps: 0,
        interest,
        claims_growth,
        user_growth,
        revenue_claim: end.revenue_claim,
        dust,
        debt_start: start.debt,
        debt_end: end.debt,
        user_claim_start: start.user_claim,
        user_claim_end: end.user_claim,
    };
    (start, end, report)
}

fn print_year_report(label: &str, util_bps: i128, r: &YearDustReport, end: &AccrualSnapshot) {
    if !VERBOSE_YEAR_ACCRUAL {
        return;
    }
    let dust_bps = if r.interest > 0 {
        r.dust.saturating_mul(10_000) / r.interest
    } else {
        0
    };

    let to_milli = |v: i128| v / 1_000_000_000_000_000_000_000_000;

    std::println!("=== {label} (util ~{util_bps} bps, {DAYS_PER_YEAR} daily updates) ===");
    std::println!(
        "  debt:     start={} end={}  interest={}  (milli-tokens: {} → {} , +{})",
        r.debt_start,
        r.debt_end,
        r.interest,
        to_milli(r.debt_start),
        to_milli(r.debt_end),
        to_milli(r.interest)
    );
    std::println!(
        "  user claim (fixed scaled): {} → {}  growth={}",
        r.user_claim_start,
        r.user_claim_end,
        r.user_growth
    );
    std::println!(
        "  revenue claim: {}  (scaled_rev={} supply_index={})",
        r.revenue_claim,
        end.revenue_scaled,
        end.supply_index
    );
    std::println!(
        "  total supply claim growth: {}  (supplied_scaled end={})",
        r.claims_growth,
        end.supplied_scaled
    );
    std::println!(
        "  DUST (interest - claims_growth): {}  (~{} bps of interest)  milli-tokens={}",
        r.dust,
        dust_bps,
        to_milli(r.dust)
    );
    std::println!(
        "  indexes: borrow {} → {}  supply {} → {}",
        RAY,
        end.borrow_index,
        RAY,
        end.supply_index
    );
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

        let (_s, end, mut r) = run_daily_year(&t.env, &mut cache, user_scaled, DAYS_PER_YEAR);
        r.label = "deep 1e6 @ 50%";
        r.util_bps = util_bps;
        print_year_report(r.label, util_bps, &r, &end);

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

        let (_s, end, mut r) = run_daily_year(&t.env, &mut cache, user_scaled, DAYS_PER_YEAR);
        r.label = "medium 1e4 @ 80%";
        r.util_bps = util_bps;
        print_year_report(r.label, util_bps, &r, &end);

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

        let (_s, end, mut r) = run_daily_year(&t.env, &mut cache, user_scaled, DAYS_PER_YEAR);
        r.label = "thin 100 @ 80%";
        r.util_bps = util_bps;
        print_year_report(r.label, util_bps, &r, &end);

        assert!(r.interest > 0);
        assert!(r.user_growth > 0);
        assert!(r.revenue_claim > 0);

        let dust_bps = r.dust.saturating_mul(10_000) / r.interest;
        if VERBOSE_YEAR_ACCRUAL {
            std::println!("  thin-market dust_bps={dust_bps}");
        }

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
            let (_s, end, mut r) = run_daily_year(&t.env, &mut cache, user_scaled, DAYS_PER_YEAR);
            r.label = label;
            r.util_bps = util_bps;
            print_year_report(label, util_bps, &r, &end);

            let whole = |v: i128| v / RAY;
            let micro = |v: i128| {

                let rem = v % RAY;
                rem * 1_000_000 / RAY
            };
            if VERBOSE_YEAR_ACCRUAL {
                std::println!(
                    "  [USDC whole.micro] debt {} → {}  interest={}.{:06}  user_growth={}.{:06}  revenue={}.{:06}  DUST={}.{:06} USDC",
                    whole(r.debt_start),
                    whole(r.debt_end),
                    whole(r.interest),
                    micro(r.interest),
                    whole(r.user_growth),
                    micro(r.user_growth),
                    whole(r.revenue_claim),
                    micro(r.revenue_claim),
                    whole(r.dust),
                    micro(r.dust.abs())
                );
            }

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

        if VERBOSE_YEAR_ACCRUAL {
            std::println!("=== daily vs single-shot (100k @ 60%, 365d) ===");
            std::println!(
                "  daily:  interest={} claims_growth={} dust={} debt_end={}",
                interest_d,
                claims_d,
                dust_d,
                end_d.debt
            );
            std::println!(
                "  once:   interest={} claims_growth={} dust={} debt_end={}",
                interest_o,
                claims_o,
                dust_o,
                end_o.debt
            );
            std::println!(
                "  user daily {} → {} | once {} → {}",
                start_d.user_claim,
                end_d.user_claim,
                start_o.user_claim,
                end_o.user_claim
            );
            std::println!(
                "  revenue daily={} once={}",
                end_d.revenue_claim,
                end_o.revenue_claim
            );
        }

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
