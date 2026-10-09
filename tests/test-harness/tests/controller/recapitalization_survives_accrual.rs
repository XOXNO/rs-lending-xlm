//! An exact recapitalization after a floor-clamped write-down keeps supply
//! open while the market's surviving debt accrues.
//!
//! The write-down clamps the supply index at its floor, so supplier claims
//! exceed cash plus debt. `recapitalize` credits the backing shortfall, and the
//! exact gap left behind must stay below one native unit: accrual adds the same
//! interest to claims and debt, so a smaller gap never reads as a shortfall
//! again and `supply` never reverts `PoolInsolvent`.

use common::constants::{BPS, LIQUIDATION_BUFFER_BPS, RAY, RAY_DECIMALS, SUPPLY_INDEX_FLOOR_RAW};
use common::types::{HubAssetKey, PoolStateRaw, SeizeMode};
use num_bigint::BigInt;
use num_traits::ToPrimitive;
use soroban_sdk::{vec, Address};
use test_harness::errors::POOL_INSOLVENT;
use test_harness::{
    eth_preset, hub_asset, usd, usdc_preset, wbtc_preset, LendingTest, ALICE, BOB, CAROL, EVE,
    LIQUIDATOR,
};

/// Alice's USDC deposit, the market's only lender, in whole tokens.
const LENDER_DEPOSIT_TOKENS: i128 = 1_000;
/// Eve's USDC collateral, seized by the liquidation that drains cash.
const SEIZABLE_COLLATERAL_TOKENS: i128 = 100;
/// Eve's WBTC debt: 0.00123 WBTC.
const SEIZABLE_ACCOUNT_WBTC_DEBT: i128 = 12_300;
/// Carol's healthy USDC debt that keeps accruing after the write-down, in tenths.
const RESIDUAL_DEBT_TENTHS: i128 = 4;
/// Bob stays this far below the borrow limit, in tenths of a token.
const BORROW_MARGIN_TENTHS: i128 = 1;
const WBTC_SPIKE_PRICE: i128 = usd(75_000);
const ETH_COLLAPSED_PRICE: i128 = usd(1);
const PROBE_TOKENS: f64 = 0.001;
const NANO_PER_UNIT: i128 = 1_000_000_000;
const STEP_SECS: u64 = 5;
/// One native unit of interest accrues in roughly 200 seconds, so this spans
/// several unit crossings.
const PROBE_STEPS: u32 = 240;
/// Seconds of accrual before the drain. At each, the fractional parts of the
/// written-down claims and of debt value sum to at least one native unit.
const WARMUPS: [u64; 4] = [3_603, 3_611, 3_641, 3_661];

struct Fixture {
    t: LendingTest,
    unit: i128,
}

fn usdc_key(t: &LendingTest) -> HubAssetKey {
    hub_asset(t.resolve_asset("USDC"))
}

fn usdc_state(t: &LendingTest) -> PoolStateRaw {
    t.pool_client("USDC").get_sync_data(&usdc_key(t)).state
}

/// Exact `claims - cash - debt` of the committed USDC book, in nano native
/// units, truncated.
fn exact_gap_nano(t: &LendingTest) -> i128 {
    let sync = t.pool_client("USDC").get_sync_data(&usdc_key(t));
    let s = sync.state;
    let scale =
        BigInt::from(RAY) * BigInt::from(10i128.pow(RAY_DECIMALS - sync.params.asset_decimals));
    let exact = BigInt::from(s.supplied) * BigInt::from(s.supply_index)
        - BigInt::from(s.borrowed) * BigInt::from(s.borrow_index)
        - BigInt::from(s.cash) * &scale;
    (exact * BigInt::from(NANO_PER_UNIT) / &scale)
        .to_i128()
        .expect("gap fits i128")
}

/// Largest debt the borrow path admits against `supplied` native units.
fn max_debt(supplied: i128, max_utilization: i128) -> i128 {
    let by_cap = supplied * max_utilization / RAY;
    let reserve = (supplied * LIQUIDATION_BUFFER_BPS + BPS - 1) / BPS;
    by_cap.min(supplied - reserve)
}

/// Liquidates Eve in Transfer mode for the largest WBTC repayment whose net
/// USDC seizure still fits in pool cash.
fn liquidate_down_to_cash(t: &mut LendingTest) {
    let eve = t.account_id(EVE);
    let wbtc = hub_asset(t.resolve_asset("WBTC"));
    let cash = usdc_state(t).cash;
    let net_seized = |t: &LendingTest, repay: i128| {
        let estimate = t.ctrl_client().get_liquidation_estimate(
            &eve,
            &vec![&t.env, (wbtc.clone(), repay)],
            &SeizeMode::Transfer,
        );
        let gross: i128 = estimate.seized_collaterals.iter().map(|p| p.amount).sum();
        let fee: i128 = estimate.protocol_fees.iter().map(|p| p.amount).sum();
        gross - fee
    };
    let (mut lo, mut hi) = (1, t.borrow_balance_raw(EVE, "WBTC"));
    while lo < hi {
        let mid = (lo + hi + 1) / 2;
        if net_seized(t, mid) <= cash {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    let liquidator = t.get_or_create_user(LIQUIDATOR);
    t.resolve_market("WBTC").token_admin.mint(&liquidator, &lo);
    t.ctrl_client().liquidate(
        &liquidator,
        &eve,
        &vec![&t.env, (wbtc, lo)],
        &SeizeMode::Transfer,
    );
}

/// Drains USDC cash through a liquidation, then cleans Bob's collapsed
/// account, which writes the supply index down to its floor.
fn clamped_market(warmup: u64) -> Fixture {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .with_initial_liquidity("USDC", 0.0)
        .build();
    let anyone = t.get_or_create_user("anyone");
    let params = t.pool_client("USDC").get_sync_data(&usdc_key(&t)).params;
    let unit = 10i128.pow(params.asset_decimals);

    let lender_deposit = LENDER_DEPOSIT_TOKENS * unit;
    let seizable = SEIZABLE_COLLATERAL_TOKENS * unit;
    t.supply_raw(ALICE, "USDC", lender_deposit);
    t.supply_raw(EVE, "USDC", seizable);
    t.borrow_raw(EVE, "WBTC", SEIZABLE_ACCOUNT_WBTC_DEBT);
    t.supply(BOB, "ETH", 1.0);
    t.supply(CAROL, "WBTC", 0.01);
    let residual = RESIDUAL_DEBT_TENTHS * unit / 10;
    t.borrow_raw(CAROL, "USDC", residual);
    let bob_debt = max_debt(lender_deposit + seizable, params.max_utilization)
        - residual
        - BORROW_MARGIN_TENTHS * unit / 10;
    t.borrow_raw(BOB, "USDC", bob_debt);
    t.advance_time(warmup);

    t.set_price("WBTC", WBTC_SPIKE_PRICE);
    liquidate_down_to_cash(&mut t);

    t.set_price("ETH", ETH_COLLAPSED_PRICE);
    let bob = t.account_id(BOB);
    t.ctrl_client().clean_bad_debt(&anyone, &bob);
    Fixture { t, unit }
}

/// Permissionless recapitalization of `amount` by a funded payer.
fn recapitalize(fx: &mut Fixture, amount: i128) -> i128 {
    let payer: Address = fx.t.get_or_create_user("treasury");
    let market = fx.t.resolve_market("USDC");
    market.token_admin.mint(&payer, &amount);
    let key = hub_asset(market.asset.clone());
    fx.t.ctrl_client().recapitalize(&payer, &key, &amount)
}

/// Seconds after which a fresh depositor's supply first reverts `PoolInsolvent`.
fn first_insolvent_supply(fx: &mut Fixture) -> Option<u64> {
    (1..=PROBE_STEPS).find_map(|step| {
        fx.t.advance_time(STEP_SECS);
        let result = fx.t.try_supply("dave", "USDC", PROBE_TOKENS);
        let insolvent =
            matches!(&result, Err(e) if *e == soroban_sdk::Error::from_contract_error(POOL_INSOLVENT));
        assert!(insolvent || result.is_ok(), "unexpected supply error {result:?}");
        insolvent.then_some(u64::from(step) * STEP_SECS)
    })
}

#[test]
fn exact_recapitalization_is_not_undone_by_accrual() {
    for warmup in WARMUPS {
        let mut fx = clamped_market(warmup);
        assert_eq!(usdc_state(&fx.t).supply_index, SUPPLY_INDEX_FLOOR_RAW);
        assert!(
            exact_gap_nano(&fx.t) >= NANO_PER_UNIT,
            "warmup {warmup}: the write-down leaves a shortfall"
        );

        let unit = fx.unit;
        let applied = recapitalize(&mut fx, unit);
        let gap = exact_gap_nano(&fx.t);

        assert!(applied > 0, "warmup {warmup}: recapitalization applies");
        assert_eq!(
            first_insolvent_supply(&mut fx),
            None,
            "warmup {warmup}: accrual reopened a shortfall after recapitalizing {applied} \
             (exact gap left: {gap} nano units)"
        );
        assert!(
            (0..NANO_PER_UNIT).contains(&gap),
            "warmup {warmup}: exact recapitalization leaves a gap below one unit, got {gap} nano"
        );
    }
}
