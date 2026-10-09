//! W8 interest accrual, index consistency and time (hypotheses H1..H8).
//!
//! Documented expectations: INV-IDX-01..05 (docs/reference/invariants.md),
//! formulas.md "Rates and accrual", "Compounding and interest allocation",
//! "Accrual cadence", "Bad debt" and "Caps, fees, and numeric limits", and
//! ADR-0016. Every assertion is on concrete values; `println!` lines carry the
//! numbers quoted in the report (run with `--nocapture`).

use common::math::fp::Ray;
use common::rates::{
    accrue_step, calculate_annual_borrow_rate, calculate_deposit_rate, MAX_COMPOUND_DELTA_MS,
};
use common::types::{MarketIndexRaw, MarketParams, PoolState, PoolStateRaw, PoolSyncData};
use controller::constants::{MAX_BORROW_INDEX_RAY, MS_PER_SECOND, RAY};
use proptest::prelude::*;
use soroban_sdk::testutils::{Address as _, Ledger, LedgerInfo};
use soroban_sdk::{Address, Env};
use test_harness::presets::LEDGER_PROTOCOL_VERSION;
use test_harness::{
    assert_contract_error, errors, hub_asset, usd, LendingTest, MarketParamsPreset, MarketPreset,
    ALICE, BOB, CAROL, DEFAULT_ASSET_CONFIG, DEFAULT_MARKET_PARAMS,
};

const YEAR_SECS: u64 = 31_556_926;
const DAY_SECS: u64 = 86_400;
const USDC: &str = "USDC";
const ETH: &str = "ETH";
/// One whole USDC or ETH at 7 decimals.
const UNIT7: i128 = 10_000_000;

// ---------------------------------------------------------------------------
// Fixtures and readers
// ---------------------------------------------------------------------------

fn usdc_market() -> MarketPreset {
    MarketPreset {
        name: USDC,
        decimals: 7,
        price_wad: usd(1),
        initial_liquidity: 0.0,
        config: DEFAULT_ASSET_CONFIG,
        params: DEFAULT_MARKET_PARAMS,
    }
}

fn eth_market() -> MarketPreset {
    MarketPreset {
        name: ETH,
        decimals: 7,
        price_wad: usd(2_000),
        initial_liquidity: 0.0,
        config: DEFAULT_ASSET_CONFIG,
        params: DEFAULT_MARKET_PARAMS,
    }
}

/// Two-market book, no utilization or dust gates. BOB supplies `supply_usdc`
/// whole USDC; ALICE posts ETH worth twice that and borrows `borrow_usdc`.
fn book(supply_usdc: i128, borrow_usdc: i128) -> LendingTest {
    let mut t = LendingTest::new()
        .with_market(usdc_market())
        .with_market(eth_market())
        .with_max_utilization_disabled_all_markets()
        .with_min_borrow_collateral_disabled()
        .build();
    t.supply_raw(BOB, USDC, supply_usdc * UNIT7);
    // supply_usdc / 1000 ETH at $2000 = 2 * supply_usdc dollars of collateral.
    t.supply_raw(ALICE, ETH, supply_usdc * 10_000);
    if borrow_usdc > 0 {
        t.borrow_raw(ALICE, USDC, borrow_usdc * UNIT7);
    }
    t
}

fn pool_sync(t: &LendingTest, asset: &str) -> PoolSyncData {
    t.pool_client(asset)
        .get_sync_data(&hub_asset(t.resolve_asset(asset)))
}

fn pool_state(t: &LendingTest, asset: &str) -> PoolStateRaw {
    pool_sync(t, asset).state
}

fn bulk_index(t: &LendingTest, asset: &str) -> MarketIndexRaw {
    let keys = soroban_sdk::vec![&t.env, hub_asset(t.resolve_asset(asset))];
    t.pool_client(asset).get_bulk_indexes(&keys).get(0).unwrap()
}

fn ctrl_index(t: &LendingTest, asset: &str) -> MarketIndexRaw {
    t.ctrl_client()
        .get_market_index(&hub_asset(t.resolve_asset(asset)))
}

fn now_ms(t: &LendingTest) -> u64 {
    t.env.ledger().timestamp() * MS_PER_SECOND
}

fn state_key(s: &PoolStateRaw) -> (i128, i128, i128, i128, i128, u64, i128) {
    (
        s.supplied,
        s.borrowed,
        s.revenue,
        s.borrow_index,
        s.supply_index,
        s.last_timestamp,
        s.cash,
    )
}

fn set_ledger_timestamp(env: &Env, timestamp: u64) {
    env.ledger().set(LedgerInfo {
        timestamp,
        protocol_version: LEDGER_PROTOCOL_VERSION,
        sequence_number: env.ledger().sequence(),
        network_id: Default::default(),
        base_reserve: 10,
        min_temp_entry_ttl: 10,
        min_persistent_entry_ttl: 10,
        max_entry_ttl: 3_110_400,
    });
}

/// Local model of the pool's accrual loop: the same `accrue_step` the mutator
/// runs, over the same chunking, starting from `sync`'s stored state.
struct Replay {
    borrow_index: Ray,
    supply_index: Ray,
    supplied: Ray,
    minted: Ray,
}

fn replay(env: &Env, sync: &PoolSyncData, now: u64) -> Replay {
    let state = PoolState::from(&sync.state);
    let params = MarketParams::from(&sync.params);
    let mut remaining = now.saturating_sub(state.last_timestamp);
    let mut bi = state.borrow_index;
    let mut si = state.supply_index;
    let mut supplied = state.supplied;
    let mut minted = Ray::ZERO;
    while remaining > 0 {
        let chunk = remaining.min(MAX_COMPOUND_DELTA_MS);
        let step = accrue_step(env, &params, state.borrowed, supplied, bi, si, chunk);
        bi = step.borrow_index;
        si = step.supply_index;
        supplied = supplied.checked_add(env, step.revenue_shares);
        minted = minted.checked_add(env, step.revenue_shares);
        remaining -= chunk;
    }
    Replay {
        borrow_index: bi,
        supply_index: si,
        supplied,
        minted,
    }
}

// ---------------------------------------------------------------------------
// H1. INV-IDX-04: the read-only projection equals the mutator, bit for bit.
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 32,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    #[test]
    fn rv_view_projection_equals_mutator_across_random_books(
        supply_usdc in 1_000i128..=1_000_000,
        borrow_pct in 10i128..=85,
        elapsed_secs in 1u64..=3 * YEAR_SECS,
        pre_secs in prop_oneof![Just(0u64), 1u64..=YEAR_SECS / 4],
    ) {
        let mut t = book(supply_usdc, supply_usdc * borrow_pct / 100);
        if pre_secs > 0 {
            t.advance_and_sync_markets(pre_secs, &[USDC]);
        }
        // Advance the ledger without touching the pool: the stored state lags.
        t.advance_time(elapsed_secs);
        let now = now_ms(&t);
        let env = t.env.clone();
        let pre = pool_state(&t, USDC);
        let view_bulk = bulk_index(&t, USDC);
        let view_ctrl = ctrl_index(&t, USDC);
        let expect = replay(&env, &pool_sync(&t, USDC), now);

        t.update_indexes_for(&[USDC]);
        let post = pool_state(&t, USDC);

        prop_assert_eq!(view_bulk.borrow_index, post.borrow_index, "bulk view borrow index");
        prop_assert_eq!(view_bulk.supply_index, post.supply_index, "bulk view supply index");
        prop_assert_eq!(view_ctrl.borrow_index, post.borrow_index, "controller view borrow index");
        prop_assert_eq!(view_ctrl.supply_index, post.supply_index, "controller view supply index");
        prop_assert_eq!(expect.borrow_index.raw(), post.borrow_index, "local replay borrow index");
        prop_assert_eq!(expect.supply_index.raw(), post.supply_index, "local replay supply index");
        prop_assert_eq!(expect.supplied.raw(), post.supplied, "local replay supplied");
        prop_assert_eq!(post.last_timestamp, now, "accrual stamps the current time");
        prop_assert_eq!(post.borrowed, pre.borrowed, "accrual never moves debt shares");
        prop_assert_eq!(post.cash, pre.cash, "accrual never moves cash");
        prop_assert_eq!(
            post.revenue - pre.revenue,
            expect.minted.raw(),
            "revenue shares minted equal the replay"
        );
        prop_assert_eq!(
            post.supplied - pre.supplied,
            expect.minted.raw(),
            "every supplied share minted is a revenue share"
        );
        prop_assert!(post.borrow_index >= pre.borrow_index, "borrow index never falls");
        prop_assert!(post.supply_index >= pre.supply_index, "supply index never falls");
    }
}

#[test]
fn rv_one_unit_repay_commits_the_same_indexes_as_the_view() {
    let mut t = book(100_000, 60_000);
    t.advance_and_sync_markets(90 * DAY_SECS, &[USDC]);
    t.advance_time(200 * DAY_SECS);
    let now = now_ms(&t);
    let env = t.env.clone();
    let pre = pool_sync(&t, USDC);
    let view = bulk_index(&t, USDC);
    let expect = replay(&env, &pre, now);

    // A one-unit repay syncs the market through the repay leg, not update_indexes.
    t.repay_raw(ALICE, USDC, 1);
    let post = pool_state(&t, USDC);
    assert_eq!(
        post.borrow_index, view.borrow_index,
        "repay leg borrow index vs view"
    );
    assert_eq!(
        post.supply_index, view.supply_index,
        "repay leg supply index vs view"
    );
    assert_eq!(
        post.borrow_index,
        expect.borrow_index.raw(),
        "repay leg vs replay"
    );
    assert_eq!(
        post.supply_index,
        expect.supply_index.raw(),
        "repay leg vs replay"
    );
    assert_eq!(
        post.supplied,
        expect.supplied.raw(),
        "supplied after repay leg"
    );
    assert_eq!(
        post.revenue - pre.state.revenue,
        expect.minted.raw(),
        "minted via repay leg"
    );
    assert_eq!(
        post.last_timestamp, now,
        "repay leg stamps the current time"
    );
    println!(
        "H1 repay leg: borrow {} supply {} minted {} (view equal)",
        post.borrow_index,
        post.supply_index,
        expect.minted.raw()
    );
}

// ---------------------------------------------------------------------------
// H2. Cadence: 1 vs 12 vs 365 steps over one year (formulas "Accrual cadence").
// ---------------------------------------------------------------------------

/// Floating-point replay of the documented rule: each step of `dt_secs` uses
/// the utilization and annual rate at its start, and compounds `exp(r * dt)`.
/// Whole-token amounts are enough: only ratios enter the utilization.
fn annual_rate_f64(p: &MarketParamsPreset, util: f64) -> f64 {
    let ray = RAY as f64;
    let (base, s1, s2, s3) = (
        p.base_borrow_rate as f64 / ray,
        p.slope1 as f64 / ray,
        p.slope2 as f64 / ray,
        p.slope3 as f64 / ray,
    );
    let (mid, opt) = (
        p.mid_utilization as f64 / ray,
        p.optimal_utilization as f64 / ray,
    );
    let u = util.min(1.0);
    let rate = if u < mid {
        base + s1 * u / mid
    } else if u < opt {
        base + s1 + s2 * (u - mid) / (opt - mid)
    } else {
        base + s1 + s2 + s3 * (u - opt) / (1.0 - opt)
    };
    rate.min(p.max_borrow_rate as f64 / ray)
}

/// Returns `(borrow_index, supply_index)` after `steps` equal steps covering
/// `total_secs`, starting from supplied `s`, borrowed `b` (whole tokens).
fn f64_replay(p: &MarketParamsPreset, s: f64, b: f64, steps: u64, total_secs: f64) -> (f64, f64) {
    let dt_years = total_secs / steps as f64 / YEAR_SECS as f64;
    let (mut supplied, mut bi, mut si) = (s, 1.0f64, 1.0f64);
    for _ in 0..steps {
        let util = if supplied == 0.0 {
            0.0
        } else {
            (b * bi) / (supplied * si)
        };
        let r = annual_rate_f64(p, util);
        let new_bi = bi * (r * dt_years).exp();
        let interest = b * (new_bi - bi);
        let fee = interest * p.reserve_factor as f64 / 10_000.0;
        let rewards = interest - fee;
        let new_si = (supplied * si + rewards) / supplied;
        supplied += fee / new_si;
        bi = new_bi;
        si = new_si;
    }
    (bi, si)
}

/// Taylor shortfall of the eighth-order series against `e^x` (relative).
fn taylor_tail(x: f64) -> f64 {
    let mut term = 1.0;
    let mut sum = 1.0;
    for k in 1..=8 {
        term *= x / k as f64;
        sum += term;
    }
    1.0 - sum / x.exp()
}

struct Cadence {
    borrow_index: i128,
    supply_index: i128,
    supplied: i128,
    revenue: i128,
}

fn run_cadence(supply_usdc: i128, util_bps: i128, steps: u64) -> Cadence {
    let mut t = book(supply_usdc, supply_usdc * util_bps / 10_000);
    let step = 365 * DAY_SECS / steps;
    assert_eq!(
        step * steps,
        365 * DAY_SECS,
        "cadence must tile the year exactly"
    );
    for _ in 0..steps {
        t.advance_and_sync_markets(step, &[USDC]);
    }
    let s = pool_state(&t, USDC);
    Cadence {
        borrow_index: s.borrow_index,
        supply_index: s.supply_index,
        supplied: s.supplied,
        revenue: s.revenue,
    }
}

#[test]
fn rv_cadence_one_twelve_daily_steps_follow_documented_rule_and_spread_is_reported() {
    const SUPPLY: i128 = 100_000;
    let p = DEFAULT_MARKET_PARAMS;
    for util_bps in [1_000i128, 6_000, 9_000] {
        let one = run_cadence(SUPPLY, util_bps, 1);
        let monthly = run_cadence(SUPPLY, util_bps, 12);
        let daily = run_cadence(SUPPLY, util_bps, 365);
        let ray = RAY as f64;

        // Interest is never negative: indexes start at RAY and never fall.
        for (name, c) in [("1", &one), ("12", &monthly), ("365", &daily)] {
            assert!(c.borrow_index >= RAY, "{name}-step borrow index below RAY");
            assert!(c.supply_index >= RAY, "{name}-step supply index below RAY");
        }

        // Documented mechanism. One step: exp(r(u0) * T) up to the Taylor tail.
        let u0 = util_bps as f64 / 10_000.0;
        let r0 = annual_rate_f64(&p, u0);
        // The one-step exponent is r0 * T / YEAR, with T = 365 days, not a full year.
        let x = r0 * (365 * DAY_SECS) as f64 / YEAR_SECS as f64;
        let closed_form = x.exp();
        let one_rel = (one.borrow_index as f64 / ray - closed_form).abs() / closed_form;
        assert!(
            one_rel <= taylor_tail(x) + 1e-12,
            "one-step borrow index {} vs exp({x}) rel err {one_rel:e} > tail {:e}",
            one.borrow_index,
            taylor_tail(x)
        );

        // Daily steps re-read utilization each day: the per-day replay must match.
        let (fb_bi, fb_si) = f64_replay(
            &p,
            SUPPLY as f64,
            (SUPPLY * util_bps / 10_000) as f64,
            365,
            (365 * DAY_SECS) as f64,
        );
        let daily_rel = (daily.borrow_index as f64 / ray - fb_bi).abs() / fb_bi;
        let daily_si_rel = (daily.supply_index as f64 / ray - fb_si).abs() / fb_si;
        assert!(
            daily_rel <= 1e-9,
            "daily borrow index rel err {daily_rel:e}"
        );
        assert!(
            daily_si_rel <= 1e-9,
            "daily supply index rel err {daily_si_rel:e}"
        );

        let spread_db = (daily.borrow_index - one.borrow_index) as f64 / one.borrow_index as f64;
        let spread_mb = (monthly.borrow_index - one.borrow_index) as f64 / one.borrow_index as f64;
        let spread_ds = (daily.supply_index - one.supply_index) as f64 / one.supply_index as f64;
        println!(
            "H2 util {}%: borrow idx 1={} 12={} 365={} | rel(12-1)={spread_mb:e} rel(365-1)={spread_db:e} | supply idx 1={} 365={} rel={spread_ds:e} | daily-vs-f64 rel={daily_rel:e} one-vs-closed rel={one_rel:e} tail={:e}",
            util_bps / 100,
            one.borrow_index,
            monthly.borrow_index,
            daily.borrow_index,
            one.supply_index,
            daily.supply_index,
            taylor_tail(x)
        );
        println!(
            "H2 util {}%: revenue shares 1={} 12={} 365={} supplied 1={} 365={}",
            util_bps / 100,
            one.revenue,
            monthly.revenue,
            daily.revenue,
            one.supplied,
            daily.supplied
        );

        // Hypothesis: cadence spread <= 1e-6. Refuted at 10 percent. The spread
        // is utilization drift (formulas "Accrual cadence"): the average rate
        // exceeds the starting rate by about 0.5 * (slope1 / mid) * u * (1 - u) * r
        // = 0.5 * 0.08 * 0.1 * 0.9 * 0.018 = 6.5e-5, the measured daily spread.
        if util_bps == 1_000 {
            assert!(
                spread_db > 1e-6 && spread_db < 2e-4,
                "10% cadence spread {spread_db:e} outside the (1e-6, 2e-4) drift band"
            );
            assert!(
                spread_mb > 1e-6 && spread_mb < 2e-4,
                "10% monthly spread {spread_mb:e} outside the (1e-6, 2e-4) drift band"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// H3. INV-IDX-04: zero elapsed time is a no-op and mints nothing.
// ---------------------------------------------------------------------------

#[test]
fn rv_zero_elapsed_second_update_is_noop_and_mints_no_revenue() {
    let mut t = book(100_000, 60_000);
    t.advance_time(DAY_SECS * 30);
    t.update_indexes_for(&[USDC]);
    let s1 = pool_state(&t, USDC);
    assert!(s1.revenue > 0, "fixture must have accrued revenue");
    assert!(s1.borrow_index > RAY, "fixture must have accrued interest");
    assert_eq!(s1.last_timestamp, now_ms(&t), "first update stamps now");

    let delta = t
        .pool_client(USDC)
        .get_delta_time(&hub_asset(t.resolve_asset(USDC)));
    assert_eq!(delta, 0, "delta time after an update in the same ledger");
    assert_eq!(
        bulk_index(&t, USDC).borrow_index,
        s1.borrow_index,
        "bulk view at same time"
    );
    assert_eq!(
        bulk_index(&t, USDC).supply_index,
        s1.supply_index,
        "bulk view at same time"
    );
    assert_eq!(
        ctrl_index(&t, USDC).borrow_index,
        s1.borrow_index,
        "controller view"
    );
    assert_eq!(
        ctrl_index(&t, USDC).supply_index,
        s1.supply_index,
        "controller view"
    );

    for round in 1..=3 {
        t.update_indexes_for(&[USDC]);
        let s = pool_state(&t, USDC);
        assert_eq!(
            state_key(&s),
            state_key(&s1),
            "same-ledger update #{round} changed stored state"
        );
    }
}

// ---------------------------------------------------------------------------
// H4. INV-IDX-05: allocation of one accrual step, and the APR relation.
// ---------------------------------------------------------------------------

#[test]
fn rv_single_step_allocation_never_books_more_than_accrued_interest() {
    let mut max_gap = 0i128;
    let mut max_over = i128::MIN;
    for util_bps in [1_000i128, 5_000, 8_500] {
        for elapsed in [DAY_SECS, 30 * DAY_SECS, YEAR_SECS] {
            let supply = 100_000;
            let mut t = book(supply, supply * util_bps / 10_000);
            // Give the books a non-trivial supply index and revenue first.
            t.advance_and_sync_markets(45 * DAY_SECS, &[USDC]);
            t.advance_time(elapsed);
            let env = t.env.clone();
            let pre = pool_sync(&t, USDC);
            let state = PoolState::from(&pre.state);
            let params = MarketParams::from(&pre.params);
            let elapsed_ms = elapsed * MS_PER_SECOND;
            let step = accrue_step(
                &env,
                &params,
                state.borrowed,
                state.supplied,
                state.borrow_index,
                state.supply_index,
                elapsed_ms,
            );

            t.update_indexes_for(&[USDC]);
            let post = pool_state(&t, USDC);
            assert_eq!(post.borrow_index, step.borrow_index.raw(), "borrow index");
            assert_eq!(post.supply_index, step.supply_index.raw(), "supply index");
            let minted = Ray::from(post.revenue - pre.state.revenue);
            assert_eq!(minted, step.revenue_shares, "revenue shares minted");

            let new_bi = step.borrow_index;
            let new_si = step.supply_index;
            // INV-IDX-05 quantities, each with the same rounding the code uses.
            let accrued = state
                .borrowed
                .mul(&env, new_bi)
                .checked_sub(&env, state.borrowed.mul(&env, state.borrow_index));
            let supplier_delta = state
                .supplied
                .mul(&env, new_si)
                .checked_sub(&env, state.supplied.mul(&env, state.supply_index));
            let revenue_value = minted.mul(&env, new_si);
            let booked = supplier_delta.raw() + revenue_value.raw();
            let gap = accrued.raw() - booked; // unbooked interest; negative means over-booked

            // Half-up rounding of the revenue value can overshoot the floored
            // share value by at most one unit.
            assert!(
                gap >= -1,
                "u={}bps t={}d: booked {booked} exceeds accrued {} by {}",
                util_bps,
                elapsed / DAY_SECS,
                accrued.raw(),
                -gap
            );
            // Leftover: the revenue-share floor (< new_si / RAY units) and rounding.
            let bound = new_si.raw() / RAY + 2;
            assert!(
                gap <= bound,
                "u={}bps t={}d: unbooked {gap} above floor bound {bound}",
                util_bps,
                elapsed / DAY_SECS
            );
            max_gap = max_gap.max(gap);
            max_over = max_over.max(-gap);
            println!(
                "H4 util {}% elapsed {}d: accrued {} booked {} (supplier {} + revenue {}) unbooked {} new_si {} minted {}",
                util_bps / 100,
                elapsed / DAY_SECS,
                accrued.raw(),
                booked,
                supplier_delta.raw(),
                revenue_value.raw(),
                gap,
                new_si.raw(),
                minted.raw()
            );

            // APR relation: deposit <= borrow * utilization, and the view matches the formula.
            let util = post_utilization(&t);
            let borrow_apr = t
                .pool_client(USDC)
                .get_borrow_rate(&hub_asset(t.resolve_asset(USDC)));
            let deposit_apr = t
                .pool_client(USDC)
                .get_deposit_rate(&hub_asset(t.resolve_asset(USDC)));
            let rate_x_util = Ray::from(util).mul(&env, Ray::from(borrow_apr)).raw();
            assert!(deposit_apr >= 0, "deposit APR negative");
            assert!(
                deposit_apr <= rate_x_util,
                "deposit APR {deposit_apr} above borrow*util {rate_x_util}"
            );
            let local_deposit = calculate_deposit_rate(
                &env,
                Ray::from(util),
                Ray::from(borrow_apr),
                params.reserve_factor,
            )
            .raw();
            assert_eq!(
                deposit_apr, local_deposit,
                "deposit view equals the formula"
            );
        }
    }
    println!("H4 largest unbooked (gap) {max_gap} raw RAY units; largest over-booking {max_over}");
}

fn post_utilization(t: &LendingTest) -> i128 {
    t.pool_client(USDC)
        .get_utilisation(&hub_asset(t.resolve_asset(USDC)))
}

// ---------------------------------------------------------------------------
// H5. INV-IDX-01 ceiling, value overflow and the documented numeric limit.
// ---------------------------------------------------------------------------

/// BOB supplies 100 USDC, ALICE borrows 90 USDC. The debt value reaches the
/// index ceiling before any RAY value overflows: 9e28 * 1e9 = 9e37 < i128::MAX.
fn cap_book() -> LendingTest {
    let mut t = LendingTest::new()
        .with_market(usdc_market())
        .with_market(eth_market())
        .with_market_params(USDC, |p| p.max_borrow_rate = 2 * RAY)
        .with_max_utilization_disabled_all_markets()
        .with_min_borrow_collateral_disabled()
        .build();
    t.supply_raw(BOB, USDC, 100 * UNIT7);
    t.supply_raw(ALICE, ETH, UNIT7);
    t.borrow_raw(ALICE, USDC, 90 * UNIT7);
    t
}

/// The 100k USDC market: BOB supplies 100,000, ALICE borrows 97,000 (cash
/// stays at the 2 percent liquidation buffer or above).
fn whale_book() -> LendingTest {
    let mut t = LendingTest::new()
        .with_market(usdc_market())
        .with_market(eth_market())
        .with_market_params(USDC, |p| p.max_borrow_rate = 2 * RAY)
        .with_max_utilization_disabled_all_markets()
        .with_min_borrow_collateral_disabled()
        .build();
    t.supply_raw(BOB, USDC, 100_000 * UNIT7);
    t.supply_raw(ALICE, ETH, 100 * UNIT7);
    t.borrow_raw(ALICE, USDC, 97_000 * UNIT7);
    t
}

/// One single update call, from a fresh book, after `secs`. True when it aborts.
fn update_aborts_after(secs: u64) -> bool {
    let mut t = whale_book();
    t.advance_time(secs);
    t.try_update_indexes_for(&[USDC]).is_err()
}

#[test]
fn rv_whale_100k_value_overflow_horizon_and_blocked_exits() {
    // Monotone predicate: find the first failing second by bisection.
    let mut lo = 0u64;
    let mut hi = 40 * YEAR_SECS;
    assert!(update_aborts_after(hi), "no overflow within 40 years");
    assert!(
        !update_aborts_after(lo),
        "fresh book must not abort at zero elapsed"
    );
    while hi - lo > 1 {
        let mid = lo + (hi - lo) / 2;
        if update_aborts_after(mid) {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    assert!(
        update_aborts_after(hi + DAY_SECS),
        "a day past the boundary must still abort"
    );
    assert!(
        !update_aborts_after(lo - DAY_SECS),
        "a day before the boundary must still succeed"
    );
    println!(
        "H5 100k USDC book, one update call from fresh state: last OK at {lo}s ({:.4} years, {:.2} days), first abort at {hi}s ({:.4} years)",
        lo as f64 / YEAR_SECS as f64,
        lo as f64 / DAY_SECS as f64,
        hi as f64 / YEAR_SECS as f64
    );

    // State just before the abort: index growth and the debt value in RAY.
    let mut ok = whale_book();
    ok.advance_time(lo);
    ok.update_indexes_for(&[USDC]);
    let s = pool_state(&ok, USDC);
    let env = ok.env.clone();
    let debt_value = Ray::from(s.borrowed)
        .mul(&env, Ray::from(s.borrow_index))
        .raw();
    println!(
        "H5 last OK state: borrow index x{:.6} ({} raw), debt value {} RAY raw, i128::MAX {}, ratio {:.4}",
        s.borrow_index as f64 / RAY as f64,
        s.borrow_index,
        debt_value,
        i128::MAX,
        debt_value as f64 / i128::MAX as f64
    );
    assert!(
        s.borrow_index < MAX_BORROW_INDEX_RAY,
        "the index ceiling must not be the binding limit for a 100k book"
    );

    // Abort horizon: the failure is MathOverflow, and repay and withdraw
    // (both accrue first) are blocked as the docs say.
    let mut bad = whale_book();
    bad.advance_time(hi);
    assert_contract_error(bad.try_update_indexes_for(&[USDC]), errors::MATH_OVERFLOW);
    assert_contract_error(bad.try_repay(ALICE, USDC, 1.0), errors::MATH_OVERFLOW);
    assert_contract_error(
        bad.try_withdraw_raw(BOB, USDC, UNIT7),
        errors::MATH_OVERFLOW,
    );
    // The aborted calls leave the stored state untouched.
    let frozen = pool_state(&bad, USDC);
    assert!(frozen.borrow_index < MAX_BORROW_INDEX_RAY);
}

#[test]
fn rv_cap_index_reached_exactly_and_frozen_after_one_call_of_120_years() {
    let mut t = cap_book();
    t.advance_time(120 * YEAR_SECS);
    t.update_indexes_for(&[USDC]);
    let s = pool_state(&t, USDC);
    assert_eq!(
        s.borrow_index, MAX_BORROW_INDEX_RAY,
        "borrow index must sit on the 1e36 ceiling"
    );
    assert!(
        s.supply_index <= MAX_BORROW_INDEX_RAY,
        "supply index above its ceiling"
    );

    // A further year changes nothing: no borrower interest, no supplier reward.
    t.advance_time(YEAR_SECS);
    t.update_indexes_for(&[USDC]);
    let s2 = pool_state(&t, USDC);
    assert_eq!(
        (
            s2.borrow_index,
            s2.supply_index,
            s2.supplied,
            s2.revenue,
            s2.borrowed
        ),
        (
            s.borrow_index,
            s.supply_index,
            s.supplied,
            s.revenue,
            s.borrowed
        ),
        "a year past the ceiling must change nothing"
    );

    // Debt view: 90 USDC * 1e9 = 9e10 USDC = 9e17 raw, exact at the ceiling.
    let hub = hub_asset(t.resolve_asset(USDC));
    let debt_view = t.pool_client(USDC).get_borrowed_amount(&hub);
    let debt_ctrl = t.borrow_balance_raw(ALICE, USDC);
    println!(
        "H5 cap: index {} (= 1e36), supply index {}, debt view {debt_view} raw, controller debt {debt_ctrl} raw, supplied {} revenue {}",
        s.borrow_index, s.supply_index, s.supplied, s.revenue
    );
    assert_eq!(
        debt_view,
        9 * 10i128.pow(17),
        "debt view saturates to 9e17 raw units"
    );
    assert_eq!(
        debt_ctrl,
        9 * 10i128.pow(17),
        "controller debt at the ceiling"
    );

    // Full repayment with one unit of overpayment, then the lender exits.
    let owed = t.borrow_balance_raw(ALICE, USDC);
    t.repay_raw(ALICE, USDC, owed + 1);
    assert_eq!(
        t.borrow_balance_raw(ALICE, USDC),
        0,
        "debt cleared by full repay"
    );
    let claim = t.supply_balance_raw(BOB, USDC);
    t.withdraw_all(BOB, USDC);
    let paid = t.token_balance_raw(BOB, USDC);
    println!("H5 cap exit: BOB claim {claim} raw, paid {paid} raw (USDC 7 dec)");
    assert!(paid <= claim, "exit above the book claim");
    assert!(
        paid + 2 >= claim,
        "exit short of the claim by more than rounding"
    );
}

/// Debt value at the index ceiling is `debt_raw_ray * 1e9`, which must fit
/// i128: debt above i128::MAX / 1e36 * 1e27 = 170.14 whole tokens cannot reach
/// the ceiling. A 180-token debt in a 200-token book aborts before it.
#[test]
fn rv_cap_unreachable_above_170_whole_tokens_value_overflows_first() {
    let mut t = LendingTest::new()
        .with_market(usdc_market())
        .with_market(eth_market())
        .with_market_params(USDC, |p| p.max_borrow_rate = 2 * RAY)
        .with_max_utilization_disabled_all_markets()
        .with_min_borrow_collateral_disabled()
        .build();
    t.supply_raw(BOB, USDC, 200 * UNIT7);
    t.supply_raw(ALICE, ETH, 10 * UNIT7);
    t.borrow_raw(ALICE, USDC, 180 * UNIT7);
    t.advance_time(120 * YEAR_SECS);
    assert_contract_error(t.try_update_indexes_for(&[USDC]), errors::MATH_OVERFLOW);
    let s = pool_state(&t, USDC);
    println!(
        "H5 180-token debt: stored index still {} (x{:.3}) after the aborted 120-year call",
        s.borrow_index,
        s.borrow_index as f64 / RAY as f64
    );
    assert!(
        s.borrow_index < MAX_BORROW_INDEX_RAY,
        "ceiling reached despite the overflow"
    );
}

#[test]
fn rv_cap_index_reached_exactly_across_three_forty_year_calls() {
    let mut t = cap_book();
    for call in 1..=3 {
        t.advance_time(40 * YEAR_SECS);
        t.update_indexes_for(&[USDC]);
        let s = pool_state(&t, USDC);
        assert_eq!(
            s.borrow_index, MAX_BORROW_INDEX_RAY,
            "after 40-year call #{call} the index must equal the ceiling"
        );
        println!(
            "H5 3x40y call {call}: borrow index {} supply index {} supplied {} revenue {}",
            s.borrow_index, s.supply_index, s.supplied, s.revenue
        );
    }
}

// ---------------------------------------------------------------------------
// H6. Utilization after a bad-debt write-down (INV-IDX-03).
// ---------------------------------------------------------------------------

/// Capped curve (max 50 percent APR, slope3 = 50 percent) so the cap binds at
/// 95 percent utilization. ALICE and CAROL borrow; ALICE becomes insolvent.
fn socialization_book() -> LendingTest {
    let mut t = LendingTest::new()
        .with_market(usdc_market())
        .with_market(eth_market())
        .with_market_params(USDC, |p| {
            p.slope3 = RAY / 2;
            p.max_borrow_rate = RAY / 2;
        })
        .with_max_utilization_disabled_all_markets()
        .with_min_borrow_collateral_disabled()
        .build();
    t.supply_raw(BOB, USDC, 100 * UNIT7);
    t.supply_raw(ALICE, ETH, UNIT7);
    t.supply_raw(CAROL, ETH, UNIT7);
    t.borrow_raw(ALICE, USDC, 90 * UNIT7);
    t.borrow_raw(CAROL, USDC, 5 * UNIT7);
    t
}

#[test]
fn rv_bad_debt_writedown_keeps_utilization_at_or_below_one_and_rate_capped() {
    let mut t = socialization_book();
    t.advance_and_sync_markets(YEAR_SECS, &[USDC]);
    let hub = hub_asset(t.resolve_asset(USDC));

    let before = pool_state(&t, USDC);
    let u_before = t.pool_client(USDC).get_utilisation(&hub);
    let rate_before = t.pool_client(USDC).get_borrow_rate(&hub);
    let supplied_value_before = t.pool_client(USDC).get_supplied_amount(&hub);
    let debt_value_before = t.pool_client(USDC).get_borrowed_amount(&hub);
    let cash_before = t.pool_client(USDC).get_reserves(&hub);
    println!(
        "H6 before: utilization {u_before} ({:.6}), borrow APR {rate_before} (cap {}), supplied {supplied_value_before}, debt {debt_value_before}, cash {cash_before}, supply index {}",
        u_before as f64 / RAY as f64,
        RAY / 2,
        before.supply_index
    );
    assert!(
        u_before <= RAY,
        "utilization above one before the write-down"
    );
    assert_eq!(
        rate_before,
        RAY / 2,
        "rate must sit on the 50 percent cap at ~95 percent use"
    );

    // ETH falls to $1: ALICE's $1 collateral cannot cover her debt.
    t.set_price(ETH, usd(1));
    let alice_id = t.account_id(ALICE);
    t.force_socialize_bad_debt_by_id(alice_id);

    let after = pool_state(&t, USDC);
    let u_after = t.pool_client(USDC).get_utilisation(&hub);
    let rate_after = t.pool_client(USDC).get_borrow_rate(&hub);
    let supplied_value_after = t.pool_client(USDC).get_supplied_amount(&hub);
    let debt_value_after = t.pool_client(USDC).get_borrowed_amount(&hub);
    let cash_after = t.pool_client(USDC).get_reserves(&hub);
    println!(
        "H6 after write-down: utilization {u_after} ({:.6}), borrow APR {rate_after}, supplied {supplied_value_after}, debt {debt_value_after}, cash {cash_after}, supply index {} (was {})",
        u_after as f64 / RAY as f64,
        after.supply_index,
        before.supply_index
    );
    assert!(
        after.supply_index < before.supply_index,
        "write-down must lower the supply index"
    );
    assert!(u_after <= RAY, "utilization above one after the write-down");
    assert!(
        u_after < u_before,
        "a write-down of the only large debt should lower utilization"
    );
    // Identity behind the bound: supplied value - debt value tracks cash.
    let residual = supplied_value_after - debt_value_after - cash_after;
    println!("H6 residual supplied - debt - cash after write-down: {residual} (asset units)");
    assert!(
        residual >= -1,
        "supplied value below debt plus cash after write-down: {residual}"
    );

    // Accrual still runs with the reduced book and the cap is not exceeded.
    t.advance_and_sync_markets(YEAR_SECS, &[USDC]);
    let later = pool_state(&t, USDC);
    assert!(
        later.borrow_index >= after.borrow_index,
        "borrow index fell after write-down"
    );
    assert!(
        t.pool_client(USDC).get_borrow_rate(&hub) <= RAY / 2,
        "borrow rate above its cap"
    );
}

// ---------------------------------------------------------------------------
// H7. Rate curve continuity and cap (formulas "Utilization and annual rates").
// ---------------------------------------------------------------------------

fn curve(env: &Env, p: &MarketParamsPreset) -> MarketParams {
    let addr = Address::generate(env);
    MarketParams::from(&p.to_market_params(&addr, 7))
}

#[test]
fn rv_rate_curve_is_monotone_continuous_and_capped() {
    let env = Env::default();
    env.cost_estimate().budget().reset_unlimited();
    let params = curve(&env, &DEFAULT_MARKET_PARAMS);
    let annual = |u: i128| calculate_annual_borrow_rate(&env, Ray::from(u), &params).raw();
    let mid = DEFAULT_MARKET_PARAMS.mid_utilization;
    let opt = DEFAULT_MARKET_PARAMS.optimal_utilization;

    // Dense sweep, 100 001 points: monotone, and adjacent jumps bounded by the
    // steepest slope (slope3 / (1 - opt) = 7.5 per unit) plus rounding.
    let steps = 100_000i128;
    let du = RAY / steps;
    let mut prev = annual(0);
    let mut max_jump = 0i128;
    for k in 1..=steps {
        let cur = annual(RAY * k / steps);
        assert!(cur >= prev, "rate fell at k={k}: {prev} -> {cur}");
        max_jump = max_jump.max(cur - prev);
        prev = cur;
    }
    let jump_bound = du * 8 + 8;
    assert!(
        max_jump <= jump_bound,
        "adjacent jump {max_jump} above {jump_bound}"
    );

    // Kinks: one raw unit either side moves the rate by at most one unit
    // (region 1 and 2) or the steep slope (region 3).
    let r = |u: i128| annual(u);
    let continuity_mid = (r(mid) - r(mid - 1)).abs().max((r(mid + 1) - r(mid)).abs());
    let continuity_opt_lo = (r(opt) - r(opt - 1)).abs();
    let continuity_opt_hi = (r(opt + 1) - r(opt)).abs();
    println!(
        "H7 kink steps (raw RAY): mid -1 {} / 0 {} / +1 {}; opt -1 {} / 0 {} / +1 {}; max sweep jump {max_jump}",
        r(mid - 1), r(mid), r(mid + 1), r(opt - 1), r(opt), r(opt + 1)
    );
    assert!(continuity_mid <= 1, "kink at mid jumps by {continuity_mid}");
    assert!(
        continuity_opt_lo <= 1,
        "kink at opt, left side, jumps by {continuity_opt_lo}"
    );
    // Region 3 slope is 7.5 per unit; the two half-up steps round a 1-unit move
    // to at most 10 raw units (1e-26 APR). Anything larger is a real kink.
    assert!(
        continuity_opt_hi <= 10,
        "kink at opt, right side, jumps by {continuity_opt_hi}"
    );

    // Anchor values, exact by construction: base at 0, 1.65 RAY at 100 percent.
    assert_eq!(
        annual(0),
        RAY / 100,
        "rate at zero utilization is the base rate"
    );
    assert_eq!(
        annual(RAY),
        RAY * 165 / 100,
        "rate at full utilization is base+s1+s2+s3"
    );
    assert_eq!(
        annual(RAY + 12345),
        annual(RAY),
        "utilization above one is clamped"
    );

    // Cap: a curve whose slope3 reaches past the cap stops at the cap.
    let capped = curve(
        &env,
        &MarketParamsPreset {
            slope3: RAY / 2,
            max_borrow_rate: RAY / 2,
            ..DEFAULT_MARKET_PARAMS
        },
    );
    let capped_annual = |u: i128| calculate_annual_borrow_rate(&env, Ray::from(u), &capped).raw();
    assert_eq!(
        capped_annual(RAY),
        RAY / 2,
        "full utilization on a 50 percent cap"
    );
    assert_eq!(
        capped_annual(95 * RAY / 100),
        RAY / 2,
        "95 percent is already past the cap"
    );
    assert!(
        capped_annual(9 * RAY / 10) < RAY / 2,
        "90 percent is still below the cap"
    );
    println!(
        "H7 default curve: f(0)={} f(mid)={} f(opt)={} f(RAY)={}; capped curve at 90%={} at 100%={}",
        annual(0),
        annual(mid),
        annual(opt),
        annual(RAY),
        capped_annual(9 * RAY / 10),
        capped_annual(RAY)
    );
}

#[test]
fn rv_rate_view_matches_formula_at_exact_utilizations() {
    let mut t = LendingTest::new()
        .with_market(usdc_market())
        .with_market(eth_market())
        .with_max_utilization_disabled_all_markets()
        .with_min_borrow_collateral_disabled()
        .build();
    t.supply_raw(BOB, USDC, 1_000 * UNIT7);
    t.supply_raw(ALICE, ETH, 1_000 * UNIT7);
    let env = t.env.clone();
    let params = curve(&env, &DEFAULT_MARKET_PARAMS);
    let hub = hub_asset(t.resolve_asset(USDC));

    // Cumulative borrows give exact utilizations 0, 0.5, 0.8, 0.9, 0.95.
    let mut borrowed = 0i128;
    for (target_pct, step) in [(0i128, 0i128), (50, 500), (80, 300), (90, 100), (95, 50)] {
        if step > 0 {
            t.borrow_raw(ALICE, USDC, step * UNIT7);
            borrowed += step;
        }
        let expected_util = Ray::from(borrowed * RAY / 1_000);
        let util = post_utilization(&t);
        assert_eq!(
            util,
            expected_util.raw(),
            "view utilization at {target_pct}%"
        );
        let view_rate = t.pool_client(USDC).get_borrow_rate(&hub);
        let local_rate = calculate_annual_borrow_rate(&env, expected_util, &params).raw();
        assert_eq!(view_rate, local_rate, "borrow APR view at {target_pct}%");
        let view_deposit = t.pool_client(USDC).get_deposit_rate(&hub);
        let local_deposit = calculate_deposit_rate(
            &env,
            expected_util,
            Ray::from(local_rate),
            params.reserve_factor,
        )
        .raw();
        assert_eq!(
            view_deposit, local_deposit,
            "deposit APR view at {target_pct}%"
        );
        println!("H7 view at {target_pct}%: borrow APR {view_rate}, deposit APR {view_deposit}");
    }
}

// ---------------------------------------------------------------------------
// H8. Ledger time moved backwards (saturating elapsed, stored timestamp).
// ---------------------------------------------------------------------------

#[test]
fn rv_backwards_ledger_update_is_inert_and_forward_accrual_counts_once() {
    // Control: the same two forward calls, never moving the clock back.
    let control = {
        let mut t = book(100_000, 50_000);
        t.advance_time(DAY_SECS);
        t.update_indexes_for(&[USDC]);
        t.advance_time(3_600);
        t.update_indexes_for(&[USDC]);
        pool_state(&t, USDC)
    };

    let mut t = book(100_000, 50_000);
    let t0 = t.env.ledger().timestamp();
    t.advance_time(DAY_SECS);
    t.update_indexes_for(&[USDC]);
    let s1 = pool_state(&t, USDC);

    // Clock moves back one hour before the stored timestamp.
    set_ledger_timestamp(&t.env, t0 + 3_600);
    assert!(
        now_ms(&t) < s1.last_timestamp,
        "fixture must be in the past"
    );
    let stale_view = bulk_index(&t, USDC);
    assert_eq!(
        stale_view.borrow_index, s1.borrow_index,
        "view in the past returns stored index"
    );
    assert_eq!(
        stale_view.supply_index, s1.supply_index,
        "view in the past returns stored index"
    );
    t.update_indexes_for(&[USDC]);
    let s_back = pool_state(&t, USDC);
    assert_eq!(
        state_key(&s_back),
        state_key(&s1),
        "backwards update changed state (last_timestamp {} vs {})",
        s_back.last_timestamp,
        s1.last_timestamp
    );
    let delta = t
        .pool_client(USDC)
        .get_delta_time(&hub_asset(t.resolve_asset(USDC)));
    assert_eq!(delta, 0, "delta time in the past saturates to zero");

    // Forward again: only the hour after the stored timestamp is accrued.
    t.advance_time(DAY_SECS);
    t.update_indexes_for(&[USDC]);
    let s2 = pool_state(&t, USDC);
    println!(
        "H8 control: borrow {} supply {} last {}; travelled: borrow {} supply {} last {}",
        control.borrow_index,
        control.supply_index,
        control.last_timestamp,
        s2.borrow_index,
        s2.supply_index,
        s2.last_timestamp
    );
    assert_eq!(
        state_key(&s2),
        state_key(&control),
        "backwards clock double-counted interest"
    );
}
