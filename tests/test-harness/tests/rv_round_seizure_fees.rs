//! RV rounding audit, lens R-1 / R-2: multi-leg seizure and per-leg transfer fees.
//!
//! Every assertion encodes the documented bound from `docs/reference/formulas.md`
//! "Seizure and fees": per leg the seizure floors to token units (partial) or pays the
//! floored claim (full close); the transfer fee is the BPS half-up of the bonus, floored
//! to units, bumped to one unit when positive and sub-unit, then capped at the whole
//! units the pool pays above the uncapped principal. Account level: the liquidator and
//! the protocol together never receive more than `repaid * (1 + bonus)` plus WAD-level
//! rounding, and the liquidator never receives less than that minus one token unit per
//! collateral leg (floor) minus one unit per leg (fee bump) minus the fee itself.
//!
//! Books: five collateral legs of mixed decimals (7, 6, 18, 7, 6) including a dust leg
//! whose planned seizure floors to zero, a two-leg 18/7 book with a zero-fee leg, and an
//! insolvent three-leg book where `seize_all` takes every unit for up to one debt-token
//! unit less than the quote. Each sweep walks 120 offers from 2% to 240% of the quote
//! through `get_liquidation_estimate`, then executes a few offers in Transfer and Credit
//! mode and reconciles balances, shares and revenue against the plan.
//!
//! Round 1 additions (the sweeps above never reach the one-unit bump): a coarse-unit
//! book with a 3-decimal $6,000 leg where the bump fires and is executed; under-delivery
//! scaling on a 10% fee-on-transfer debt token, mirrored leg by leg; the per-leg half-up
//! of the USD-weighted base bonus; and an exhaustive integer model of the per-leg fee
//! chain (`calculate_seized_collateral` lines 423-490 and `split_seized_shares`) over
//! 250k legs of mixed decimals, indexes, bonuses, fees and cap positions.

use common::math::fp::{Bps, Ray, Wad};
use common::math::fp_core::{mul_div_ceil, mul_div_floor};
use common::rates::resolve_withdrawal;
use common::types::{
    AccountPositionRaw, ControllerKey, HubAssetKey, LiquidationEstimate, SeizeMode,
};
use controller::constants::{BPS, RAY, WAD};
use num_bigint::BigInt;
use num_rational::BigRational;
use num_traits::{One, ToPrimitive, Zero};
use position_nft::PositionNftClient;
use soroban_sdk::{Env, Map, Vec as SVec};
use test_harness::{
    hub_asset, AssetConfigPreset, LendingTest, MarketPreset, ALICE, DEFAULT_ASSET_CONFIG,
    DEFAULT_MARKET_PARAMS, LIQUIDATOR,
};

const DEBT: &str = "USDT";
const DEBT_DEC: u32 = 7;

const fn usd(whole: i128) -> i128 {
    whole * WAD
}

fn big(v: i128) -> BigInt {
    BigInt::from(v)
}

fn rat(v: i128) -> BigRational {
    BigRational::from_integer(big(v))
}

fn pow10(dec: u32) -> BigInt {
    big(10).pow(dec)
}

/// Exact WAD USD value of `amount` base units at `price_wad`.
fn val(amount: i128, price_wad: i128, dec: u32) -> BigRational {
    BigRational::new(big(amount) * big(price_wad), pow10(dec))
}

fn floor(r: &BigRational) -> BigInt {
    r.floor().to_integer()
}

#[derive(Clone, Copy)]
struct LegSpec {
    name: &'static str,
    dec: u32,
    price: i128,
    supply_raw: i128,
    ltv: u32,
    lt: u32,
    bonus: u32,
    fees: u32,
}

fn build(legs: &[LegSpec], debt_f64: f64, target_hf: f64) -> LendingTest {
    let mut b = LendingTest::new();
    for l in legs {
        b = b.with_market(MarketPreset {
            name: l.name,
            decimals: l.dec,
            price_wad: l.price,
            initial_liquidity: 1_000.0,
            config: AssetConfigPreset {
                loan_to_value: l.ltv,
                liquidation_threshold: l.lt,
                liquidation_bonus: l.bonus,
                liquidation_fees: l.fees,
                ..DEFAULT_ASSET_CONFIG
            },
            params: DEFAULT_MARKET_PARAMS,
        });
    }
    b = b.with_market(MarketPreset {
        name: DEBT,
        decimals: DEBT_DEC,
        price_wad: usd(1),
        initial_liquidity: 10_000_000.0,
        config: AssetConfigPreset {
            loan_to_value: 9000,
            liquidation_threshold: 9500,
            liquidation_bonus: 200,
            ..DEFAULT_ASSET_CONFIG
        },
        params: DEFAULT_MARKET_PARAMS,
    });
    let mut t = b.build();
    for l in legs {
        t.supply_raw(ALICE, l.name, l.supply_raw);
    }
    t.borrow(ALICE, DEBT, debt_f64);
    let acc = t.resolve_account_id(ALICE);
    let hf0 = t.ctrl_client().get_health_factor(&acc) as f64 / WAD as f64;
    let factor = target_hf / hf0;
    for l in legs {
        t.set_price(l.name, (l.price as f64 * factor).round() as i128);
    }
    t.get_or_create_user(LIQUIDATOR);
    t.assert_liquidatable(ALICE);
    t
}

fn key(t: &LendingTest, name: &str) -> HubAssetKey {
    hub_asset(t.resolve_asset(name))
}

fn price(t: &LendingTest, name: &str) -> i128 {
    t.resolve_market(name).price_wad
}

fn supply_index(t: &LendingTest, name: &str) -> i128 {
    t.pool_client(name)
        .get_sync_data(&key(t, name))
        .state
        .supply_index
}

/// Scaled protocol revenue shares of the market.
fn revenue_shares(t: &LendingTest, name: &str) -> i128 {
    t.pool_client(name)
        .get_sync_data(&key(t, name))
        .state
        .revenue
}

/// Scaled supply shares of `key` held by account `id`.
fn shares_of(t: &LendingTest, id: u64, k: &HubAssetKey) -> i128 {
    t.env.as_contract(&t.controller, || {
        t.env
            .storage()
            .persistent()
            .get::<_, Map<HubAssetKey, AccountPositionRaw>>(&ControllerKey::SupplyPositions(id))
            .and_then(|book| book.get(k.clone()))
            .map_or(0, |p| p.scaled_amount)
    })
}

/// `floor(shares * index)` in base units (the pool's conservative claim).
fn tokens_floor(shares: i128, index: i128, dec: u32) -> BigInt {
    floor(&BigRational::new(
        big(shares) * big(index),
        big(RAY) * pow10(27 - dec),
    ))
}

/// `half_up(shares * index)` in base units (the pool's full-withdrawal trigger).
fn tokens_half_up(shares: i128, index: i128, dec: u32) -> BigInt {
    let scale = big(RAY) * pow10(27 - dec);
    let num = big(shares) * big(index);
    (&num * big(2) + &scale) / (scale * big(2))
}

fn est(t: &LendingTest, acc: u64, offer_raw: i128, mode: SeizeMode) -> LiquidationEstimate {
    let mut pays: SVec<(HubAssetKey, i128)> = SVec::new(&t.env);
    pays.push_back((key(t, DEBT), offer_raw));
    t.ctrl_client().get_liquidation_estimate(&acc, &pays, &mode)
}

fn est_leg(e: &LiquidationEstimate, t: &LendingTest, name: &str) -> (i128, i128) {
    let asset = t.resolve_asset(name);
    let amount = e
        .seized_collaterals
        .iter()
        .find(|p| p.asset == asset)
        .map_or(0, |p| p.amount);
    let fee = e
        .protocol_fees
        .iter()
        .find(|p| p.asset == asset)
        .map_or(0, |p| p.amount);
    (amount, fee)
}

struct PlanCheck {
    repay_usd: i128,
    bonus_bps: i128,
    paid_usd: BigRational,
    fee_usd: BigRational,
    omitted: usize,
    bumped: usize,
}

/// Checks one planned seizure against the per-leg and account-level bounds.
/// `extra_slack_usd` is added to the upper bound (the `seize_all` debt-unit slack).
fn check_plan(
    t: &LendingTest,
    acc: u64,
    legs: &[LegSpec],
    e: &LiquidationEstimate,
    extra_slack_usd: &BigRational,
    label: &str,
) -> PlanCheck {
    let b = e.bonus_rate_bps;
    let one_plus_b = BigRational::new(big(BPS + b), big(BPS));
    let repay = rat(e.max_payment_wad);
    let planned = &repay * &one_plus_b;
    let total_c = t.ctrl_client().get_total_collateral_usd(&acc);

    let mut paid_usd = BigRational::zero();
    let mut fee_usd = BigRational::zero();
    let mut unit_sum = BigRational::zero();
    let mut slop = rat(2);
    let mut omitted = 0usize;
    let mut bumped = 0usize;

    for l in legs {
        let (amount, fee) = est_leg(e, t, l.name);
        let p = price(t, l.name);
        let unit = val(1, p, l.dec);
        unit_sum += &unit;
        // Half-up chain per leg: share (0.5e-18 of total), seizure USD (0.5 raw),
        // division by price (0.5 of a WAD token unit = price/2e18 raw).
        slop += &planned / rat(WAD) + rat(1) + BigRational::new(big(p), big(2 * WAD));

        if amount == 0 {
            assert_eq!(fee, 0, "[{label}] {}: fee on an omitted leg", l.name);
            omitted += 1;
            continue;
        }
        assert!(
            amount > 0 && fee >= 0 && fee <= amount,
            "[{label}] {}: sign",
            l.name
        );

        // Full close pays floor(held); partial pays `amount`.
        let k = key(t, l.name);
        let held = shares_of(t, acc, &k);
        let idx = supply_index(t, l.name);
        let held_half_up = tokens_half_up(held, idx, l.dec);
        let paid_units = if big(amount) >= held_half_up {
            tokens_floor(held, idx, l.dec)
        } else {
            big(amount)
        };
        assert!(
            paid_units >= big(amount) - big(1),
            "[{label}] {}: full-close payout {paid_units} below planned {amount} by more than one unit",
            l.name
        );
        let paid = BigRational::new(&paid_units * big(p), pow10(l.dec));
        let fee_v = val(fee, p, l.dec);
        paid_usd += &paid;
        fee_usd += &fee_v;

        // Uncapped principal of this leg in units: `repay * value_i / C / price`, the
        // same in every regime (the uncapped seizure is `repay * (1 + b) * value_i / C`).
        // A millionth of a unit covers the WAD-level half-up steps on the way.
        let value_i = BigRational::new(big(held) * big(idx) * big(p), big(RAY) * big(RAY));
        let principal_units = &repay * &value_i / rat(total_c) * pow10(l.dec) / rat(p);
        let slack = BigRational::new(big(1), big(1_000_000));
        // R-2: fee never exceeds the whole units paid above the uncapped principal.
        let excess_units =
            BigRational::from_integer(paid_units.clone()) - &principal_units + &slack;
        // A floored payout can sit below principal (documented); the fee is then zero.
        let excess_bound = floor(&excess_units).max(BigInt::zero());
        assert!(
            big(fee) <= excess_bound,
            "[{label}] {}: fee {fee} above realised excess bound {excess_bound} (amount {amount}, b {b})",
            l.name
        );
        // R-2: fee never exceeds fee_bps of the planned RAY bonus, which sits below
        // `paid + 1 unit - principal` (the payout floors the planned seizure), except
        // for the one-unit bump.
        let bonus_units_upper = &excess_units + BigRational::one();
        let fee_cap = floor(&(&bonus_units_upper * rat(i128::from(l.fees)) / rat(BPS)));
        let allowed = if fee_cap.is_zero() {
            BigInt::one()
        } else {
            fee_cap.clone()
        };
        assert!(
            big(fee) <= allowed,
            "[{label}] {}: fee {fee} above fee_bps bound {allowed} (amount {amount}, fees {} bps, b {b})",
            l.name,
            l.fees
        );
        if fee == 1 && fee_cap.is_zero() {
            bumped += 1;
        }
    }

    // Account level, upper: liquidator + protocol never above repaid * (1 + b) + rounding.
    let upper = &planned + &slop + extra_slack_usd;
    assert!(
        paid_usd <= upper,
        "[{label}] account-level over-seizure: paid {} > planned {} + slack {} (repay {}, b {b})",
        paid_usd.to_f64().unwrap(),
        planned.to_f64().unwrap(),
        (&slop + extra_slack_usd).to_f64().unwrap(),
        e.max_payment_wad
    );
    // Account level, lower: the liquidator loses at most one unit per leg to floors.
    let lower = &planned - &unit_sum - &slop;
    assert!(
        paid_usd >= lower,
        "[{label}] liquidator under-seizure beyond one unit per leg: paid {} < planned {} - units {}",
        paid_usd.to_f64().unwrap(),
        planned.to_f64().unwrap(),
        unit_sum.to_f64().unwrap()
    );
    PlanCheck {
        repay_usd: e.max_payment_wad,
        bonus_bps: b,
        paid_usd,
        fee_usd,
        omitted,
        bumped,
    }
}

fn quote_raw(t: &LendingTest, acc: u64) -> i128 {
    let debt_raw = t.ctrl_client().get_borrow_amount(&acc, &key(t, DEBT));
    let q = est(t, acc, 2 * debt_raw, SeizeMode::Transfer).max_payment_wad;
    // WAD USD at $1 to 7-decimal units.
    q / 10i128.pow(18 - DEBT_DEC)
}

/// Sweeps 120 offers through the estimate view. Returns the maximum fee share of the
/// bonus seen and the count of omitted and bumped legs.
fn sweep(
    t: &LendingTest,
    legs: &[LegSpec],
    extra_slack_per_debt_unit: bool,
    label: &str,
) -> (f64, usize, usize) {
    let acc = t.resolve_account_id(ALICE);
    let q = quote_raw(t, acc);
    let debt_unit = val(1, price(t, DEBT), DEBT_DEC);
    let mut max_fee_share = 0f64;
    let mut omitted = 0usize;
    let mut bumped = 0usize;
    for k in 1..=120i128 {
        let offer = (q * k / 50).max(1);
        let e = est(t, acc, offer, SeizeMode::Transfer);
        if e.max_payment_wad == 0 {
            continue;
        }
        let b = e.bonus_rate_bps;
        let extra = if extra_slack_per_debt_unit {
            &debt_unit * BigRational::new(big(BPS + b), big(BPS))
        } else {
            BigRational::zero()
        };
        let c = check_plan(t, acc, legs, &e, &extra, &format!("{label} k={k}"));
        let bonus_usd = rat(c.repay_usd) * BigRational::new(big(c.bonus_bps), big(BPS));
        if bonus_usd > BigRational::zero() {
            let share = (&c.fee_usd / &bonus_usd).to_f64().unwrap();
            max_fee_share = max_fee_share.max(share);
        }
        omitted += c.omitted;
        bumped += c.bumped;
        if k % 20 == 0 || k == 1 {
            std::println!(
                "RV-R1R2 {label} k={k} offer={offer} repay_usd={} b={} paid_usd={:.6} fee_usd={:.6} omitted={} bumped={}",
                c.repay_usd,
                c.bonus_bps,
                c.paid_usd.to_f64().unwrap() / WAD as f64,
                c.fee_usd.to_f64().unwrap() / WAD as f64,
                c.omitted,
                c.bumped
            );
        }
    }
    (max_fee_share, omitted, bumped)
}

/// Executes one Transfer-mode liquidation and reconciles balances, burned shares and
/// revenue against the plan.
fn execute_transfer(t: &mut LendingTest, legs: &[LegSpec], offer_raw: i128, label: &str) {
    let acc = t.resolve_account_id(ALICE);
    let e = est(t, acc, offer_raw, SeizeMode::Transfer);
    let liq = t.get_or_create_user(LIQUIDATOR);
    t.resolve_market(DEBT).token_admin.mint(&liq, &offer_raw);

    let before: std::vec::Vec<(i128, i128, i128, i128)> = legs
        .iter()
        .map(|l| {
            (
                t.token_balance_raw(LIQUIDATOR, l.name),
                shares_of(t, acc, &key(t, l.name)),
                t.snapshot_revenue(l.name),
                supply_index(t, l.name),
            )
        })
        .collect();
    let debt_before = t.token_balance_raw(LIQUIDATOR, DEBT);

    let mut pays: SVec<(HubAssetKey, i128)> = SVec::new(&t.env);
    pays.push_back((key(t, DEBT), offer_raw));
    t.ctrl_client()
        .liquidate(&liq, &acc, &pays, &SeizeMode::Transfer);
    t.assert_spoke_usage_matches_positions();

    let repaid_raw = debt_before - t.token_balance_raw(LIQUIDATOR, DEBT);
    // A liquidation that leaves an insolvent residue at or below the $5 dust cap is
    // followed by bad-debt cleanup, which sweeps the remaining collateral to revenue and
    // deletes the account (INV-LIQ-04). The borrower-side checks then do not apply.
    let alive = t.find_account_id(ALICE).is_some();
    std::println!(
        "RV-R1R2 {label} exec offer={offer_raw} pulled={repaid_raw} plan_repay_usd={} b={} account_alive={alive}",
        e.max_payment_wad,
        e.bonus_rate_bps
    );

    for (l, (bal0, sh0, rev0, idx)) in legs.iter().zip(before) {
        let (amount, fee) = est_leg(&e, t, l.name);
        let received = t.token_balance_raw(LIQUIDATOR, l.name) - bal0;
        let sh1 = shares_of(t, acc, &key(t, l.name));
        let burned = sh0 - sh1;
        let rev_delta = t.snapshot_revenue(l.name) - rev0;
        let held_half_up = tokens_half_up(sh0, idx, l.dec);
        let expected_paid = if amount > 0 && big(amount) >= held_half_up {
            tokens_floor(sh0, idx, l.dec)
        } else {
            big(amount)
        };
        // Liquidator receipt is exactly plan minus fee (full close pays the floor).
        assert_eq!(
            big(received),
            &expected_paid - big(fee),
            "[{label}] {}: received {received} != paid {expected_paid} - fee {fee}",
            l.name
        );
        // Borrower loses at most the planned amount plus one unit (pool full-close promotion).
        let lost_units = tokens_floor(burned, idx, l.dec);
        if !alive {
            std::println!(
                "RV-R1R2 {label} exec {}: cleanup swept the residue; amount={amount} fee={fee} received={received} burned_units={lost_units}",
                l.name
            );
            continue;
        }
        assert!(
            lost_units <= big(amount) + BigInt::one(),
            "[{label}] {}: borrower lost {lost_units} units for a planned {amount}",
            l.name
        );
        if amount > 0 && big(amount) < held_half_up {
            // Partial: pool burns ceil(amount / index) shares exactly.
            let want = BigRational::new(big(amount) * pow10(27 - l.dec) * big(RAY), big(idx))
                .ceil()
                .to_integer();
            assert_eq!(big(burned), want, "[{label}] {}: partial burn", l.name);
        }
        // Fee tokens stay in the pool as revenue shares (floor(fee / index)); the view
        // unscales them with its own floor, so the token delta is within one unit.
        let rev_tokens = big(rev_delta);
        assert!(
            rev_tokens <= big(fee) && rev_tokens >= big(fee) - BigInt::one(),
            "[{label}] {}: revenue {rev_tokens} units vs fee {fee}",
            l.name
        );
        std::println!(
            "RV-R1R2 {label} exec {}: amount={amount} fee={fee} received={received} burned_units={lost_units} rev_units={rev_tokens}",
            l.name
        );
    }
}

/// Executes one Credit(0) liquidation: shares are conserved exactly and the fee shares
/// never exceed the ceiling of fee_bps on the bonus shares (bounded from the plan).
fn execute_credit(t: &mut LendingTest, legs: &[LegSpec], offer_raw: i128, label: &str) {
    let acc = t.resolve_account_id(ALICE);
    let e = est(t, acc, offer_raw, SeizeMode::Credit(0));
    let liq = t.get_or_create_user(LIQUIDATOR);
    t.resolve_market(DEBT).token_admin.mint(&liq, &offer_raw);
    let before: std::vec::Vec<(i128, i128, i128)> = legs
        .iter()
        .map(|l| {
            (
                shares_of(t, acc, &key(t, l.name)),
                revenue_shares(t, l.name),
                supply_index(t, l.name),
            )
        })
        .collect();
    let mut pays: SVec<(HubAssetKey, i128)> = SVec::new(&t.env);
    pays.push_back((key(t, DEBT), offer_raw));
    let receiver = t
        .ctrl_client()
        .liquidate(&liq, &acc, &pays, &SeizeMode::Credit(0));
    assert!(receiver != 0, "[{label}] credit receiver");
    let nft = PositionNftClient::new(&t.env, &t.position_nft);
    assert!(nft.total_supply() >= 2);

    let b = e.bonus_rate_bps;
    for (l, (sh0, rev0, idx)) in legs.iter().zip(before) {
        let (amount, _) = est_leg(&e, t, l.name);
        let k = key(t, l.name);
        let seized = sh0 - shares_of(t, acc, &k);
        let credited = shares_of(t, receiver, &k);
        let fee_shares = revenue_shares(t, l.name) - rev0;
        assert_eq!(
            seized,
            credited + fee_shares,
            "[{label}] {}: share conservation",
            l.name
        );
        if amount == 0 {
            assert_eq!(seized, 0, "[{label}] {}: omitted leg moved shares", l.name);
            continue;
        }
        // The Credit estimate reports seized shares and fee shares; execution moves
        // exactly those (no token-unit floor and no bump in Credit mode).
        let (_, fee_planned) = est_leg(&e, t, l.name);
        assert_eq!(
            seized, amount,
            "[{label}] {}: seized shares != estimate",
            l.name
        );
        assert_eq!(
            fee_shares, fee_planned,
            "[{label}] {}: fee shares != estimate",
            l.name
        );
        // bonus_shares <= floor(((amount + 1) * b / (1 + b)) / index); fee = ceil(fee_bps * bonus).
        let bonus_units_upper = BigRational::new(big(amount + 1) * big(b), big(BPS + b));
        let bonus_shares_upper =
            &bonus_units_upper * BigRational::new(pow10(27 - l.dec) * big(RAY), big(idx));
        let fee_upper = (&bonus_shares_upper * rat(i128::from(l.fees)) / rat(BPS))
            .ceil()
            .to_integer();
        assert!(
            big(fee_shares) <= fee_upper,
            "[{label}] {}: credit fee shares {fee_shares} above ceil bound {fee_upper}",
            l.name
        );
        // Partial credit seizes floor(amount_ray / index) shares at most.
        let seized_upper =
            BigRational::new(big(amount + 1) * pow10(27 - l.dec) * big(RAY), big(idx))
                .ceil()
                .to_integer();
        assert!(
            big(seized) <= seized_upper,
            "[{label}] {}: seized shares {seized} above {seized_upper}",
            l.name
        );
        std::println!(
            "RV-R1R2 {label} credit {}: amount={amount} seized={seized} credited={credited} fee_shares={fee_shares}",
            l.name
        );
    }
}

// ---------------------------------------------------------------------------
// Books
// ---------------------------------------------------------------------------

/// Five legs: 7, 6, 18, 7 and 6 decimals. DUST6 holds two base units ($0.20 at $100k,
/// one unit = $0.10) so every partial seizure of it floors to zero and the leg is omitted.
const FIVE_LEGS: [LegSpec; 5] = [
    LegSpec {
        name: "USDC7",
        dec: 7,
        price: usd(1),
        supply_raw: 10_000 * 10_000_000,
        ltv: 7500,
        lt: 8500,
        bonus: 400,
        fees: 1000,
    },
    LegSpec {
        name: "ETH6",
        dec: 6,
        price: usd(2_000),
        supply_raw: 2 * 1_000_000,
        ltv: 7000,
        lt: 7800,
        bonus: 800,
        fees: 1500,
    },
    LegSpec {
        name: "WBTC18",
        dec: 18,
        price: usd(60_000),
        supply_raw: 100_000_000_000_000_000, // 0.1
        ltv: 6500,
        lt: 7500,
        bonus: 1000,
        fees: 1200,
    },
    LegSpec {
        name: "XLM7",
        dec: 7,
        price: WAD / 10,
        supply_raw: 20_000 * 10_000_000,
        ltv: 5000,
        lt: 6500,
        bonus: 1200,
        fees: 2000,
    },
    LegSpec {
        name: "DUST6",
        dec: 6,
        price: usd(100_000),
        supply_raw: 2,
        ltv: 7000,
        lt: 8000,
        bonus: 500,
        fees: 1000,
    },
];

/// 18-decimal leg with a zero fee next to a 7-decimal leg with the maximum fee.
const TWO_LEGS: [LegSpec; 2] = [
    LegSpec {
        name: "WETH18",
        dec: 18,
        price: usd(2_000),
        supply_raw: 3_000_000_000_000_000_000, // 3
        ltv: 7000,
        lt: 8000,
        bonus: 500,
        fees: 0,
    },
    LegSpec {
        name: "USDC7",
        dec: 7,
        price: usd(1),
        supply_raw: 4_000 * 10_000_000,
        ltv: 7500,
        lt: 8500,
        bonus: 400,
        fees: 5000,
    },
];

#[test]
fn rv_round_r1r2_five_leg_mixed_decimals_partial_sweep_holds_documented_bounds() {
    // ~$20k of collateral, $13k of debt, HF pushed to 0.97 (curve regime, partial quote).
    let t = build(&FIVE_LEGS, 13_000.0, 0.97);
    let (max_fee_share, omitted, bumped) = sweep(&t, &FIVE_LEGS, false, "five-0.97");
    std::println!("RV-R1R2 five-0.97: max fee/bonus={max_fee_share:.4} omitted_legs={omitted} bumped_fees={bumped}");
    assert!(omitted > 0, "the dust leg must be omitted at least once");
}

#[test]
fn rv_round_r1r2_five_leg_band_regime_full_close_sweep_holds_documented_bounds() {
    // HF 0.85 with these thresholds puts the account in the band: quote = D at the cap.
    let t = build(&FIVE_LEGS, 13_000.0, 0.85);
    let (max_fee_share, omitted, bumped) = sweep(&t, &FIVE_LEGS, false, "five-0.85");
    std::println!("RV-R1R2 five-0.85: max fee/bonus={max_fee_share:.4} omitted_legs={omitted} bumped_fees={bumped}");
}

#[test]
fn rv_round_r1r2_five_leg_execution_matches_plan_transfer_and_credit() {
    for (frac, label) in [
        (0.07f64, "small"),
        (0.5, "half"),
        (1.0, "quote"),
        (1.6, "over"),
    ] {
        let mut t = build(&FIVE_LEGS, 13_000.0, 0.97);
        let acc = t.resolve_account_id(ALICE);
        let q = quote_raw(&t, acc);
        let offer = ((q as f64) * frac) as i128;
        execute_transfer(&mut t, &FIVE_LEGS, offer.max(1), &format!("five-T-{label}"));
    }
    for (frac, label) in [(0.3f64, "small"), (1.0, "quote")] {
        let mut t = build(&FIVE_LEGS, 13_000.0, 0.97);
        let acc = t.resolve_account_id(ALICE);
        let q = quote_raw(&t, acc);
        let offer = ((q as f64) * frac) as i128;
        execute_credit(&mut t, &FIVE_LEGS, offer.max(1), &format!("five-C-{label}"));
    }
}

#[test]
fn rv_round_r1r2_two_leg_zero_and_max_fee_sweep_holds_documented_bounds() {
    let t = build(&TWO_LEGS, 7_000.0, 0.96);
    let (max_fee_share, omitted, bumped) = sweep(&t, &TWO_LEGS, false, "two-0.96");
    std::println!("RV-R1R2 two-0.96: max fee/bonus={max_fee_share:.4} omitted_legs={omitted} bumped_fees={bumped}");
    let mut t = build(&TWO_LEGS, 7_000.0, 0.96);
    let acc = t.resolve_account_id(ALICE);
    let q = quote_raw(&t, acc);
    execute_transfer(&mut t, &TWO_LEGS, q / 3, "two-T-third");
}

/// Insolvent three-leg book: the quote is `floor(C / (1 + base))`; an offer within one
/// debt-token unit of it seizes every unit (`seize_all`). The upper bound therefore
/// carries one debt unit times `(1 + b)` of slack, and nothing more.
#[test]
fn rv_round_r1r2_insolvent_three_leg_seize_all_within_one_debt_unit() {
    let legs = [FIVE_LEGS[0], FIVE_LEGS[1], FIVE_LEGS[2]];
    let t = build(&legs, 13_000.0, 0.55);
    let acc = t.resolve_account_id(ALICE);
    let c = t.ctrl_client().get_total_collateral_usd(&acc);
    let d = t.ctrl_client().get_total_borrow_usd(&acc);
    assert!(c < d, "fixture must be insolvent: C={c} D={d}");
    let (max_fee_share, omitted, bumped) = sweep(&t, &legs, true, "insolvent");
    std::println!("RV-R1R2 insolvent: C={c} D={d} max fee/bonus={max_fee_share:.4} omitted={omitted} bumped={bumped}");
    let mut t = build(&legs, 13_000.0, 0.55);
    let acc = t.resolve_account_id(ALICE);
    let q = quote_raw(&t, acc);
    execute_transfer(&mut t, &legs, q, "insolvent-T-quote");
    let mut t = build(&legs, 13_000.0, 0.55);
    execute_transfer(&mut t, &legs, q - 1, "insolvent-T-quote-minus-one");
}

// ---------------------------------------------------------------------------
// Round 1 additions: coarse-unit leg and the one-unit bump
// ---------------------------------------------------------------------------

/// Same five-leg shape with a 3-decimal $6,000 leg (one base unit is $6) in place of
/// XLM, so the 10% fee on a bonus below $60 floors to zero and is bumped to one unit.
const COARSE_LEGS: [LegSpec; 5] = [
    LegSpec {
        name: "GOLD3",
        dec: 3,
        price: usd(6_000),
        supply_raw: 1_000, // 1.000 token = $6,000
        ltv: 6000,
        lt: 7000,
        bonus: 500,
        fees: 1000,
    },
    FIVE_LEGS[0],
    FIVE_LEGS[1],
    FIVE_LEGS[2],
    FIVE_LEGS[4],
];

/// Uncapped principal of `name` in base units for the plan `e`:
/// `repay * value_i / C / price * 10^dec` (the uncapped seizure is `repay * (1 + b) *
/// value_i / C`, and principal divides that by `1 + b`).
fn principal_units(
    t: &LendingTest,
    acc: u64,
    name: &str,
    dec: u32,
    e: &LiquidationEstimate,
) -> BigRational {
    let k = key(t, name);
    let held = shares_of(t, acc, &k);
    let idx = supply_index(t, name);
    let p = price(t, name);
    let total_c = t.ctrl_client().get_total_collateral_usd(&acc);
    let value_i = BigRational::new(big(held) * big(idx) * big(p), big(RAY) * big(RAY));
    rat(e.max_payment_wad) * &value_i / rat(total_c) * pow10(dec) / rat(p)
}

#[test]
fn rv_round_r1r2_coarse_unit_leg_fee_bump_takes_at_most_one_unit_per_leg() {
    let t = build(&COARSE_LEGS, 13_000.0, 0.97);
    let (max_fee_share, omitted, bumped) = sweep(&t, &COARSE_LEGS, false, "coarse-0.97");
    std::println!(
        "RV-R1R2 coarse-0.97: max fee/bonus={max_fee_share:.4} omitted_legs={omitted} bumped_fees={bumped}"
    );
    assert!(bumped > 0, "the GOLD3 one-unit bump must fire in the sweep");

    // Smallest offer whose GOLD3 fee is the bump: exact fee below one unit, charged one.
    let acc = t.resolve_account_id(ALICE);
    let q = quote_raw(&t, acc);
    let mut chosen: Option<(i128, LiquidationEstimate)> = None;
    for k in 1..=120i128 {
        let offer = (q * k / 50).max(1);
        let e = est(&t, acc, offer, SeizeMode::Transfer);
        let (amount, fee) = est_leg(&e, &t, "GOLD3");
        if amount == 0 || fee != 1 {
            continue;
        }
        let principal = principal_units(&t, acc, "GOLD3", 3, &e);
        let bonus_units = rat(amount) - &principal;
        let exact_fee = &bonus_units * rat(1000) / rat(BPS);
        if exact_fee < BigRational::one() {
            chosen = Some((offer, e));
            break;
        }
    }
    let (offer, e) = chosen.expect("an offer with a bumped GOLD3 fee");
    let principal = principal_units(&t, acc, "GOLD3", 3, &e);
    let (amount, fee) = est_leg(&e, &t, "GOLD3");
    let bonus_units = rat(amount) - &principal;
    let exact_fee = &bonus_units * rat(1000) / rat(BPS);
    std::println!(
        "RV-R1R2 coarse bump: offer={offer} repay_usd={} b={} GOLD3 amount={amount} principal={:.4} bonus_units={:.4} exact_fee_units={:.4} charged={fee} overcharge_usd={:.4}",
        e.max_payment_wad,
        e.bonus_rate_bps,
        principal.to_f64().unwrap(),
        bonus_units.to_f64().unwrap(),
        exact_fee.to_f64().unwrap(),
        (rat(fee) - &exact_fee).to_f64().unwrap() * 6.0
    );
    // Documented: the bump is one unit and never takes the leg below principal.
    assert_eq!(fee, 1);
    assert!(
        rat(amount - fee) >= principal.floor(),
        "GOLD3 net {} below principal {}",
        amount - fee,
        principal.to_f64().unwrap()
    );

    let mut t = build(&COARSE_LEGS, 13_000.0, 0.97);
    execute_transfer(&mut t, &COARSE_LEGS, offer, "coarse-T-bump");
    let mut t = build(&COARSE_LEGS, 13_000.0, 0.97);
    execute_credit(&mut t, &COARSE_LEGS, offer, "coarse-C-bump");
}

// ---------------------------------------------------------------------------
// Under-delivery scaling: a 10% fee-on-transfer debt token
// ---------------------------------------------------------------------------

fn build_fot(legs: &[LegSpec], debt_f64: f64, target_hf: f64, shortfall_bps: i128) -> LendingTest {
    let mut b = LendingTest::new();
    for l in legs {
        b = b.with_market(MarketPreset {
            name: l.name,
            decimals: l.dec,
            price_wad: l.price,
            initial_liquidity: 1_000.0,
            config: AssetConfigPreset {
                loan_to_value: l.ltv,
                liquidation_threshold: l.lt,
                liquidation_bonus: l.bonus,
                liquidation_fees: l.fees,
                ..DEFAULT_ASSET_CONFIG
            },
            params: DEFAULT_MARKET_PARAMS,
        });
    }
    b = b.with_fee_on_transfer_market(
        MarketPreset {
            name: DEBT,
            decimals: DEBT_DEC,
            price_wad: usd(1),
            initial_liquidity: 10_000_000.0,
            config: AssetConfigPreset {
                loan_to_value: 9000,
                liquidation_threshold: 9500,
                liquidation_bonus: 200,
                ..DEFAULT_ASSET_CONFIG
            },
            params: DEFAULT_MARKET_PARAMS,
        },
        shortfall_bps,
    );
    let mut t = b.build();
    for l in legs {
        t.supply_raw(ALICE, l.name, l.supply_raw);
    }
    t.borrow(ALICE, DEBT, debt_f64);
    let acc = t.resolve_account_id(ALICE);
    let hf0 = t.ctrl_client().get_health_factor(&acc) as f64 / WAD as f64;
    let factor = target_hf / hf0;
    for l in legs {
        t.set_price(l.name, (l.price as f64 * factor).round() as i128);
    }
    t.get_or_create_user(LIQUIDATOR);
    t.assert_liquidatable(ALICE);
    t
}

/// Transfer mode on a partial plan: the pool receives 90% of the offer, the controller
/// floor-scales every leg by `received_usd / planned_usd` (`scale_seizures_to_received`),
/// and each leg is mirrored exactly. Documented bound: each leg's net receipt is within
/// one unit below the scaled principal, and the fee never rises under scaling.
#[test]
fn rv_round_r1r2_under_delivery_floor_scales_each_leg_within_one_unit_of_scaled_principal() {
    let mut t = build_fot(&FIVE_LEGS, 13_000.0, 0.97, 1_000);
    let acc = t.resolve_account_id(ALICE);
    let q = quote_raw(&t, acc);
    let offer = q / 2;
    let e = est(&t, acc, offer, SeizeMode::Transfer);
    assert!(e.refunds.is_empty(), "a half-quote offer is kept whole");
    let b = e.bonus_rate_bps;
    let den = e.max_payment_wad;
    // WeirdToken: delivered = amount - amount * shortfall / 10_000.
    let delivered = offer - offer * 1_000 / 10_000;
    // apply_liquidation_repayments: leg_usd = floor(usd_wad * received / amount).
    let num = mul_div_floor(&t.env, den, delivered, offer);
    assert!(num < den);

    let liq = t.get_or_create_user(LIQUIDATOR);
    t.resolve_market(DEBT).token_admin.mint(&liq, &offer);
    let before: std::vec::Vec<(i128, i128, i128, i128)> = FIVE_LEGS
        .iter()
        .map(|l| {
            (
                t.token_balance_raw(LIQUIDATOR, l.name),
                shares_of(&t, acc, &key(&t, l.name)),
                t.snapshot_revenue(l.name),
                supply_index(&t, l.name),
            )
        })
        .collect();
    let debt_before = t.token_balance_raw(LIQUIDATOR, DEBT);
    let mut pays: SVec<(HubAssetKey, i128)> = SVec::new(&t.env);
    pays.push_back((key(&t, DEBT), offer));
    t.ctrl_client()
        .liquidate(&liq, &acc, &pays, &SeizeMode::Transfer);
    t.assert_spoke_usage_matches_positions();
    let pulled = debt_before - t.token_balance_raw(LIQUIDATOR, DEBT);
    assert_eq!(pulled, offer, "the liquidator pays the whole offer");
    assert!(t.find_account_id(ALICE).is_some());

    let one_plus_b = BigRational::new(big(BPS + b), big(BPS));
    let bound_usd = rat(num) * &one_plus_b;
    let mut paid_usd = BigRational::zero();
    let mut slop = rat(2);
    for (l, (bal0, sh0, rev0, idx)) in FIVE_LEGS.iter().zip(before) {
        let (amount, fee) = est_leg(&e, &t, l.name);
        let p = price(&t, l.name);
        slop += &bound_usd / rat(WAD) + rat(1) + BigRational::new(big(p), big(2 * WAD));
        let received = t.token_balance_raw(LIQUIDATOR, l.name) - bal0;
        let rev_delta = t.snapshot_revenue(l.name) - rev0;
        if amount == 0 {
            assert_eq!(received, 0, "{}: omitted leg paid", l.name);
            continue;
        }
        // scale_seizures_to_received: floor by num/den on amount and fee.
        let amount_s = mul_div_floor(&t.env, amount, num, den);
        let fee_s = mul_div_floor(&t.env, fee, num, den);
        assert!(fee_s <= fee, "{}: scaling raised the fee", l.name);
        let held_half_up = tokens_half_up(sh0, idx, l.dec);
        let expected_paid = if big(amount_s) >= held_half_up {
            tokens_floor(sh0, idx, l.dec)
        } else {
            big(amount_s)
        };
        assert_eq!(
            big(received),
            &expected_paid - big(fee_s),
            "{}: received {received} != scaled plan {amount_s} - scaled fee {fee_s}",
            l.name
        );
        let principal = principal_units(&t, acc, l.name, l.dec, &e);
        let scaled_principal = &principal * rat(num) / rat(den);
        assert!(
            rat(received) >= scaled_principal.floor() - BigRational::one(),
            "{}: net {received} more than one unit below the scaled principal {}",
            l.name,
            scaled_principal.to_f64().unwrap()
        );
        assert!(
            big(rev_delta) <= big(fee_s) && big(rev_delta) + BigInt::one() >= big(fee_s),
            "{}: revenue {rev_delta} vs scaled fee {fee_s}",
            l.name
        );
        paid_usd += val(received + fee_s, p, l.dec);
        std::println!(
            "RV-R1R2 fot {}: amount={amount}->{amount_s} fee={fee}->{fee_s} received={received} scaled_principal={:.4} shares_burned={}",
            l.name,
            scaled_principal.to_f64().unwrap(),
            sh0 - shares_of(&t, acc, &key(&t, l.name))
        );
    }
    std::println!(
        "RV-R1R2 fot: offer={offer} delivered={delivered} planned_usd={den} received_usd={num} b={b} paid_usd={:.6} bound_usd={:.6}",
        paid_usd.to_f64().unwrap() / WAD as f64,
        bound_usd.to_f64().unwrap() / WAD as f64
    );
    assert!(
        paid_usd <= &bound_usd + &slop,
        "account-level: paid {} above received repayment * (1 + b) {}",
        paid_usd.to_f64().unwrap(),
        bound_usd.to_f64().unwrap()
    );
}

/// Credit mode under the same under-delivery: shares are conserved per leg and the
/// seized shares are within one unit of the floor-scaled planned amount.
#[test]
fn rv_round_r1r2_under_delivery_credit_conserves_shares_per_leg() {
    let mut t = build_fot(&FIVE_LEGS, 13_000.0, 0.97, 1_000);
    let acc = t.resolve_account_id(ALICE);
    let q = quote_raw(&t, acc);
    let offer = q / 2;
    let e = est(&t, acc, offer, SeizeMode::Credit(0));
    let den = e.max_payment_wad;
    let delivered = offer - offer * 1_000 / 10_000;
    let num = mul_div_floor(&t.env, den, delivered, offer);
    let liq = t.get_or_create_user(LIQUIDATOR);
    t.resolve_market(DEBT).token_admin.mint(&liq, &offer);
    let before: std::vec::Vec<(i128, i128, i128)> = FIVE_LEGS
        .iter()
        .map(|l| {
            (
                shares_of(&t, acc, &key(&t, l.name)),
                revenue_shares(&t, l.name),
                supply_index(&t, l.name),
            )
        })
        .collect();
    let mut pays: SVec<(HubAssetKey, i128)> = SVec::new(&t.env);
    pays.push_back((key(&t, DEBT), offer));
    let receiver = t
        .ctrl_client()
        .liquidate(&liq, &acc, &pays, &SeizeMode::Credit(0));
    assert!(receiver != 0);
    for (l, (sh0, rev0, idx)) in FIVE_LEGS.iter().zip(before) {
        let (amount, _) = est_leg(&e, &t, l.name);
        let k = key(&t, l.name);
        let seized = sh0 - shares_of(&t, acc, &k);
        let credited = shares_of(&t, receiver, &k);
        let fee_shares = revenue_shares(&t, l.name) - rev0;
        assert_eq!(
            seized,
            credited + fee_shares,
            "{}: share conservation",
            l.name
        );
        if amount == 0 {
            assert_eq!(seized, 0);
            continue;
        }
        // The Credit estimate reports seized shares and fee shares, not token units.
        let (_, fee_planned) = est_leg(&e, &t, l.name);
        let seized_planned = mul_div_floor(&t.env, amount, num, den);
        assert_eq!(
            seized, seized_planned,
            "{}: seized shares must be floor(planned shares * received / planned)",
            l.name
        );
        // fee' = ceil(floor(bonus * r) * f / BPS) <= floor(ceil(bonus * f / BPS) * r) + 1.
        assert!(
            fee_shares <= mul_div_floor(&t.env, fee_planned, num, den) + 1,
            "{}: scaled credit fee {fee_shares} above floor(planned fee {fee_planned} * r) + 1",
            l.name
        );
        let seized_units = tokens_floor(seized, idx, l.dec);
        std::println!(
            "RV-R1R2 fot credit {}: shares={amount}->{seized_planned} seized_units={seized_units} credited={credited} fee_shares={fee_shares} planned_fee_shares={fee_planned}",
            l.name
        );
    }
}

// ---------------------------------------------------------------------------
// Base bonus: the USD-weighted average rounds half-up once per leg
// ---------------------------------------------------------------------------

/// Four equal-value legs at 250 BPS: each term is `half_up(0.25 * 250) = 63`, so the
/// base bonus is 252 where a single rounding would give 250. Documented as half-up
/// division and multiplication per leg (`formulas.md`, "Bonus and target repayment"),
/// so the bound is one half BPS per leg; the insolvent quote uses the base directly.
#[test]
fn rv_round_r1r2_base_bonus_half_up_once_per_leg_is_within_half_bps_per_leg() {
    const LEGS: [LegSpec; 4] = [
        LegSpec {
            name: "A7",
            dec: 7,
            price: usd(1),
            supply_raw: 2_500 * 10_000_000,
            ltv: 7500,
            lt: 8500,
            bonus: 250,
            fees: 1000,
        },
        LegSpec {
            name: "B7",
            dec: 7,
            price: usd(1),
            supply_raw: 2_500 * 10_000_000,
            ltv: 7500,
            lt: 8500,
            bonus: 250,
            fees: 1000,
        },
        LegSpec {
            name: "C7",
            dec: 7,
            price: usd(1),
            supply_raw: 2_500 * 10_000_000,
            ltv: 7500,
            lt: 8500,
            bonus: 250,
            fees: 1000,
        },
        LegSpec {
            name: "D7",
            dec: 7,
            price: usd(1),
            supply_raw: 2_500 * 10_000_000,
            ltv: 7500,
            lt: 8500,
            bonus: 250,
            fees: 1000,
        },
    ];
    let mut b = LendingTest::new();
    for l in LEGS.iter() {
        b = b.with_market(MarketPreset {
            name: l.name,
            decimals: l.dec,
            price_wad: l.price,
            initial_liquidity: 1_000.0,
            config: AssetConfigPreset {
                loan_to_value: l.ltv,
                liquidation_threshold: l.lt,
                liquidation_bonus: l.bonus,
                liquidation_fees: l.fees,
                ..DEFAULT_ASSET_CONFIG
            },
            params: DEFAULT_MARKET_PARAMS,
        });
    }
    b = b.with_market(MarketPreset {
        name: DEBT,
        decimals: DEBT_DEC,
        price_wad: usd(1),
        initial_liquidity: 10_000_000.0,
        config: AssetConfigPreset {
            loan_to_value: 9000,
            liquidation_threshold: 9500,
            liquidation_bonus: 200,
            ..DEFAULT_ASSET_CONFIG
        },
        params: DEFAULT_MARKET_PARAMS,
    });
    let mut t = b.build();
    for l in LEGS.iter() {
        t.supply_raw(ALICE, l.name, l.supply_raw);
    }
    t.borrow(ALICE, DEBT, 7_000.0);
    for l in LEGS.iter() {
        t.set_price(l.name, usd(69) / 100);
    }
    t.get_or_create_user(LIQUIDATOR);
    let acc = t.resolve_account_id(ALICE);
    let c = t.ctrl_client().get_total_collateral_usd(&acc);
    let d = t.ctrl_client().get_total_borrow_usd(&acc);
    assert!(c < d, "insolvent fixture: C={c} D={d}");
    let debt_raw = t.ctrl_client().get_borrow_amount(&acc, &key(&t, DEBT));
    let e = est(&t, acc, 2 * debt_raw, SeizeMode::Transfer);
    std::println!(
        "RV-R1R2 base-bonus: C={c} D={d} legs=4 listing_bonus=250 exact_weighted=250 base_bps={} quote={}",
        e.bonus_rate_bps,
        e.max_payment_wad
    );
    assert!(
        (e.bonus_rate_bps - 250).abs() <= 2,
        "base bonus {} further than half a BPS per leg from 250",
        e.bonus_rate_bps
    );
    assert_eq!(e.bonus_rate_bps, 252, "four half-up terms of 62.5");
}

// ---------------------------------------------------------------------------
// Integer model of the per-leg fee chain
// ---------------------------------------------------------------------------

struct LegInput {
    dec: u32,
    index: Ray,
    scaled: Ray,
    seizure_ray: Ray,
    bonus_bps: i128,
    fees_bps: i128,
}

struct LegModel {
    omitted: bool,
    is_full: bool,
    capped_ray: Ray,
    base_ray: Ray,
    bonus_ray: Ray,
    fee_ray: Ray,
    fee_asset: i128,
    bumped: bool,
    pool_gross: i128,
    realised_excess: i128,
    protocol_fee: i128,
    seized_scaled: Ray,
    bonus_scaled: Ray,
    credit_fee: Ray,
}

/// One partial-or-full seizure leg of `calculate_seized_collateral`
/// (`contracts/controller/src/positions/liquidation/math.rs` lines 423-490) and the
/// credit split of `split_seized_shares` (line 547), with the same `fp` calls.
fn leg_model(env: &Env, i: &LegInput) -> LegModel {
    let one_plus_bonus = Wad::ONE.checked_add(env, Bps::from(i.bonus_bps).to_wad(env));
    let actual_ray = i.scaled.mul(env, i.index);
    let capped_ray = i.seizure_ray.min(actual_ray);
    let is_full = capped_ray == actual_ray;
    let base_ray = i.seizure_ray.div_floor(env, one_plus_bonus.to_ray(env));
    let bonus_ray = if capped_ray > base_ray {
        capped_ray.checked_sub(env, base_ray)
    } else {
        Ray::ZERO
    };
    let fee_ray = Bps::from(i.fees_bps).apply_to_ray(env, bonus_ray);
    let seized_scaled = if is_full {
        i.scaled
    } else {
        capped_ray.div_floor(env, i.index)
    };
    let bonus_scaled = bonus_ray.div_floor(env, i.index).min(seized_scaled);
    let capped_amount = if is_full {
        capped_ray.to_asset(env, i.dec)
    } else {
        capped_ray.to_asset_floor(env, i.dec)
    };
    let omitted = i.seizure_ray <= Ray::ZERO
        || capped_ray <= Ray::ZERO
        || seized_scaled <= Ray::ZERO
        || capped_amount <= 0;
    let mut m = LegModel {
        omitted,
        is_full,
        capped_ray,
        base_ray,
        bonus_ray,
        fee_ray,
        fee_asset: 0,
        bumped: false,
        pool_gross: 0,
        realised_excess: 0,
        protocol_fee: 0,
        seized_scaled,
        bonus_scaled,
        credit_fee: Ray::ZERO,
    };
    if omitted {
        return m;
    }
    let (_, pool_gross) = resolve_withdrawal(env, capped_amount, i.scaled, i.index, i.dec);
    let fee_asset = fee_ray.to_asset_floor(env, i.dec);
    let bumped = fee_ray > Ray::ZERO && fee_asset == 0;
    let bumped_fee = if bumped { 1 } else { fee_asset };
    let paid_ray = Ray::from_asset(env, pool_gross, i.dec);
    let realised_excess = if paid_ray > base_ray {
        paid_ray
            .checked_sub(env, base_ray)
            .to_asset_floor(env, i.dec)
    } else {
        0
    };
    m.fee_asset = fee_asset;
    m.bumped = bumped;
    m.pool_gross = pool_gross;
    m.realised_excess = realised_excess;
    m.protocol_fee = bumped_fee.min(realised_excess);
    m.credit_fee = Ray::from(mul_div_ceil(env, bonus_scaled.raw(), i.fees_bps, BPS));
    m
}

#[derive(Default)]
struct Tally {
    legs: u64,
    omitted: u64,
    full: u64,
    bumped: u64,
    paid_below_principal: u64,
    /// Protocol fee above the exact `fee_bps * bonus`, in token units (the bump).
    max_fee_over_exact_units: f64,
    /// Liquidator net receipt below `min(principal, capped)`, in token units.
    max_shortfall_units: f64,
    /// Planned seizure of an omitted leg, in token units.
    max_omitted_units: f64,
    /// Credit fee shares above the exact `fee_bps * bonus_shares`, in raw shares.
    max_credit_fee_over_exact_raw: i128,
}

fn units(r: Ray, unit_ray: Ray) -> f64 {
    r.raw() as f64 / unit_ray.raw() as f64
}

/// Asserts the documented per-leg bounds ("Seizure and fees") on one modelled leg.
fn check_leg(env: &Env, i: &LegInput, m: &LegModel, tally: &mut Tally) {
    let unit_ray = Ray::from_asset(env, 1, i.dec);
    tally.legs += 1;
    let label = format!(
        "dec={} index={} scaled={} seizure={} b={} f={}",
        i.dec,
        i.index.raw(),
        i.scaled.raw(),
        i.seizure_ray.raw(),
        i.bonus_bps,
        i.fees_bps
    );
    if m.omitted {
        tally.omitted += 1;
        // An omitted leg is below one unit (a half unit on a full close) or below one
        // raw share's value; the liquidator paid for it and gets nothing from it.
        assert!(
            m.capped_ray < unit_ray,
            "[{label}] omitted leg holds {} units",
            units(m.capped_ray, unit_ray)
        );
        tally.max_omitted_units = tally.max_omitted_units.max(units(m.capped_ray, unit_ray));
        return;
    }
    if m.is_full {
        tally.full += 1;
    }
    let paid_ray = Ray::from_asset(env, m.pool_gross, i.dec);
    let net_ray = Ray::from_asset(env, m.pool_gross - m.protocol_fee, i.dec);

    // Fee within the realised excess and the payout.
    assert!(m.protocol_fee >= 0, "[{label}] negative fee");
    assert!(
        m.protocol_fee <= m.realised_excess && m.protocol_fee <= m.pool_gross,
        "[{label}] fee {} above excess {} or payout {}",
        m.protocol_fee,
        m.realised_excess,
        m.pool_gross
    );
    // Fee never above the RAY fee rounded up to one unit; the floor without a bump.
    assert!(
        m.protocol_fee <= m.fee_ray.to_asset_ceil(env, i.dec),
        "[{label}] fee {} above ceil(fee_ray)",
        m.protocol_fee
    );
    if m.bumped {
        tally.bumped += 1;
        assert!(m.protocol_fee <= 1, "[{label}] bump above one unit");
    } else {
        assert!(
            m.protocol_fee <= m.fee_asset,
            "[{label}] fee above its floor"
        );
    }
    // The fee never takes the leg below principal.
    assert!(
        net_ray >= m.base_ray.min(paid_ray),
        "[{label}] net {} below min(principal {}, paid {})",
        net_ray.raw(),
        m.base_ray.raw(),
        paid_ray.raw()
    );
    // Payout never above the planned capped seizure and within one unit below it.
    assert!(
        paid_ray <= m.capped_ray,
        "[{label}] paid {} above capped {}",
        paid_ray.raw(),
        m.capped_ray.raw()
    );
    // A full close pays the double-floored claim, so a true value a hair below a
    // whole unit pays one unit under the half-up RAY plan (`>=`, not `>`).
    assert!(
        paid_ray.checked_add(env, unit_ray) >= m.capped_ray,
        "[{label}] paid {} more than one unit below capped {}",
        paid_ray.raw(),
        m.capped_ray.raw()
    );
    // Net receipt within one unit below min(principal, capped).
    let floor_ref = m.base_ray.min(m.capped_ray);
    assert!(
        net_ray.checked_add(env, unit_ray) >= floor_ref,
        "[{label}] net {} more than one unit below {}",
        net_ray.raw(),
        floor_ref.raw()
    );
    if paid_ray < m.base_ray {
        tally.paid_below_principal += 1;
    }
    if net_ray < floor_ref {
        tally.max_shortfall_units = tally
            .max_shortfall_units
            .max(units(floor_ref.checked_sub(env, net_ray), unit_ray));
    }
    let exact_fee_units = units(m.bonus_ray, unit_ray) * i.fees_bps as f64 / BPS as f64;
    tally.max_fee_over_exact_units = tally
        .max_fee_over_exact_units
        .max(m.protocol_fee as f64 - exact_fee_units);

    // Credit mode: bonus shares within seized shares, fee shares = ceil(f * bonus_shares)
    // adds under one raw share, seized shares never above the capped seizure.
    assert!(
        m.bonus_scaled <= m.seized_scaled,
        "[{label}] bonus shares above seized"
    );
    assert!(
        m.credit_fee <= m.bonus_scaled,
        "[{label}] credit fee above bonus shares"
    );
    let over = big(m.credit_fee.raw()) * big(BPS) - big(m.bonus_scaled.raw()) * big(i.fees_bps);
    assert!(
        over < big(BPS) && over >= BigInt::zero(),
        "[{label}] credit fee ceiling off by {over} / BPS shares"
    );
    if over > BigInt::zero() {
        tally.max_credit_fee_over_exact_raw = tally.max_credit_fee_over_exact_raw.max(1);
    }
    assert!(
        m.seized_scaled.mul_floor(env, i.index) <= m.capped_ray,
        "[{label}] seized shares worth more than the capped seizure"
    );
}

#[test]
fn rv_round_r1r2_fee_chain_integer_model_holds_per_leg_bounds() {
    let env = Env::default();
    // I256 multiply-divides are metered host calls; the grid needs no budget.
    env.cost_estimate().budget().reset_unlimited();
    let decimals = [3u32, 6, 7, 18];
    let indexes = [
        RAY,
        1_234_567_890_123_456_789_012_345_678i128,
        3 * RAY,
        RAY / 1_000,
    ];
    let bonuses = [1i128, 50, 250, 500, 1000, 2557, 4611, 6666];
    let fees = [1i128, 100, 1000, 2500, 5000, 9999];
    // Held balance in hundredths of a unit.
    let held_centiunits = [
        50i128,
        100,
        150,
        249,
        250,
        251,
        300,
        790,
        1000,
        1050,
        3330,
        10_000,
        10_040,
        10_060,
        100_007,
        1_234_567,
        100_000_025,
    ];
    // Seizure as a fraction of the held balance, in ten-thousandths.
    let fractions = [
        1i128, 100, 1000, 3000, 5000, 7000, 9000, 9500, 9900, 9990, 9999, 10_000, 10_001, 10_100,
        11_000, 20_000,
    ];
    let mut tally = Tally::default();
    for &dec in &decimals {
        let unit_ray = Ray::from_asset(&env, 1, dec);
        for &index in &indexes {
            let index = Ray::from(index);
            for &held in &held_centiunits {
                let actual_target = Ray::from(mul_div_floor(&env, unit_ray.raw(), held, 100));
                let scaled = actual_target.div_floor(&env, index);
                if scaled <= Ray::ZERO {
                    continue;
                }
                for &frac in &fractions {
                    let seizure_ray =
                        Ray::from(mul_div_floor(&env, actual_target.raw(), frac, 10_000));
                    for &b in &bonuses {
                        for &f in &fees {
                            let i = LegInput {
                                dec,
                                index,
                                scaled,
                                seizure_ray,
                                bonus_bps: b,
                                fees_bps: f,
                            };
                            let m = leg_model(&env, &i);
                            check_leg(&env, &i, &m, &mut tally);
                        }
                    }
                }
            }
        }
    }
    // Fine grid at 3 decimals: every 1/64 of a unit from 0 to 48 units, uncapped.
    let dec = 3u32;
    let unit_ray = Ray::from_asset(&env, 1, dec);
    let scaled = Ray::from(mul_div_floor(&env, unit_ray.raw(), 1_000_000, 1));
    for step in 0..(48 * 64) {
        let seizure_ray = Ray::from(mul_div_floor(&env, unit_ray.raw(), step, 64));
        for &b in &[50i128, 500, 2557, 6666] {
            for &f in &[100i128, 1000, 5000, 9999] {
                let i = LegInput {
                    dec,
                    index: Ray::ONE,
                    scaled,
                    seizure_ray,
                    bonus_bps: b,
                    fees_bps: f,
                };
                let m = leg_model(&env, &i);
                check_leg(&env, &i, &m, &mut tally);
            }
        }
    }
    std::println!(
        "RV-R1R2 model: legs={} omitted={} full={} bumped={} paid_below_principal={} max_fee_over_exact_units={:.6} max_liquidator_shortfall_units={:.6} max_omitted_units={:.6} max_credit_fee_over_exact_raw={}",
        tally.legs,
        tally.omitted,
        tally.full,
        tally.bumped,
        tally.paid_below_principal,
        tally.max_fee_over_exact_units,
        tally.max_shortfall_units,
        tally.max_omitted_units,
        tally.max_credit_fee_over_exact_raw
    );
    std::println!(
        "RV-R1R2 model: five-leg account bound = {:.4} units of liquidator loss and {:.4} units of fee bump over the exact rate",
        5.0 * tally.max_shortfall_units.max(tally.max_omitted_units),
        5.0 * tally.max_fee_over_exact_units
    );
    assert!(tally.bumped > 0, "the bump must be reachable in the grid");
    assert!(
        tally.omitted > 0,
        "omitted legs must be reachable in the grid"
    );
    assert!(tally.max_fee_over_exact_units < 1.0);
    assert!(tally.max_shortfall_units <= 1.0);
    assert!(tally.max_omitted_units < 1.0);
    assert!(tally.max_credit_fee_over_exact_raw <= 1);
}
