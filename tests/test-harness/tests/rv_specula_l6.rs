//! Second opinion on Specula lending CR-1 and CR-2, driven through the deployed
//! pool WASM as its Ownable owner (mocked auth stands in for the controller).
//!
//! CR-1, flipped by PR #196 (4e00e09): `guards::backing_shortfall` used to
//! floor the claim side and ceil the debt side to whole units before
//! subtracting, so an exact recapitalization left up to two units of real
//! under-backing and one accrual could reopen a one-unit shortfall. The guard
//! now floors the RAY-precision gap once and subtracts the cash: an exact
//! recapitalization leaves less than one unit of real under-backing.
//!
//! CR-2, flipped by PR #196 (76b5c68): a market whose shares all left but
//! whose cash kept the rounding remainder could be borrowed from, after which
//! `claim_revenue` reverted on `require_supply_for_debt`. `mint_debt` now
//! refuses a debt mint into a market with no supply shares, so the leftover
//! cash stays inert and the revenue claim stays live.
//!
//! `books` recomputes the retired native-unit identity from `PoolStateRaw`;
//! the pool's own guard reports the RAY-precision value.

use common::constants::RAY;
use common::errors::CollateralError;
use common::math::fp::Ray;
use common::rates::{unscale_borrow_ceil, unscale_supply_floor};
use common::types::{
    HubAssetKey, InterestRateModel, PoolAction, PoolBorrowEntry, PoolKey, PoolPositionMutation,
    PoolStateRaw, PoolSupplyEntry, PoolWithdrawEntry, ScaledPositionRaw,
};
use pool::LiquidityPoolClient;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{token, vec, Address, Env, Error as SdkError, InvokeError};
use test_harness::{assert_contract_error, hub_asset, LendingTest};

/// One whole USDC in 7-decimal raw units.
const UNIT: i128 = 10_000_000;
/// RAY per raw unit at 7 decimals (1e27 / 1e7).
const SCALE: i128 = 100_000_000_000_000_000_000;
const DECIMALS: u32 = 7;
const DAY_SECS: u64 = 86_400;

const POOL_INSOLVENT: u32 = CollateralError::PoolInsolvent as u32;

// ---------------------------------------------------------------------------
// Fixture and pool helpers
// ---------------------------------------------------------------------------

fn fixture() -> LendingTest {
    let mut preset = test_harness::usdc_preset();
    preset.initial_liquidity = 0.0;
    let t = LendingTest::new().with_market(preset).build();
    set_model(&t, |m| m.max_utilization = RAY);
    t
}

fn market_key(t: &LendingTest) -> HubAssetKey {
    hub_asset(t.resolve_asset("USDC"))
}

fn pool(t: &LendingTest) -> LiquidityPoolClient<'_> {
    t.pool_client("USDC")
}

fn pool_addr(t: &LendingTest) -> Address {
    t.resolve_market("USDC").pool.clone()
}

fn state(t: &LendingTest) -> PoolStateRaw {
    pool(t).get_sync_data(&market_key(t)).state
}

fn set_model(t: &LendingTest, edit: impl FnOnce(&mut InterestRateModel)) {
    let key = market_key(t);
    let mut model = pool(t).get_sync_data(&key).params.rate_model_view();
    edit(&mut model);
    pool(t).update_params(&key, &model);
}

/// Flat 10% APR at every utilization (base = slopes = 10%, max 100%).
fn ten_percent_flat(m: &mut InterestRateModel) {
    m.base_borrow_rate = RAY / 10;
    m.slope1 = RAY / 10;
    m.slope2 = RAY / 10;
    m.slope3 = RAY / 10;
    m.max_borrow_rate = RAY;
    m.max_utilization = RAY;
}

/// Zero at zero utilization, like the hub-level mainnet models with
/// `base_borrow_rate = 0` (and the hub 2 / hub 3 models with every slope zero).
fn zero_base_rate(m: &mut InterestRateModel) {
    m.base_borrow_rate = 0;
    m.slope1 = 0;
    m.slope2 = 0;
    m.slope3 = 0;
    m.max_borrow_rate = RAY;
    m.max_utilization = RAY;
}

fn act(t: &LendingTest, position: i128, amount: i128) -> PoolAction {
    PoolAction {
        position: ScaledPositionRaw {
            scaled_amount: position,
        },
        amount,
        hub_asset: market_key(t),
    }
}

/// Mints `amount` to `who` and moves it into the pool, as the controller does
/// before supply, repay and recapitalize.
fn pay_in(t: &LendingTest, who: &Address, amount: i128) {
    let market = t.resolve_market("USDC");
    market.token_admin.mint(who, &amount);
    token::Client::new(&t.env, &market.asset).transfer(who, &market.pool, &amount);
}

fn supply(t: &LendingTest, who: &Address, position: i128, amount: i128) -> PoolPositionMutation {
    pay_in(t, who, amount);
    pool(t)
        .supply(&vec![
            &t.env,
            PoolSupplyEntry {
                action: act(t, position, amount),
            },
        ])
        .get(0)
        .unwrap()
}

fn try_supply(t: &LendingTest, position: i128, amount: i128) -> Result<(), SdkError> {
    flat(pool(t).try_supply(&vec![
        &t.env,
        PoolSupplyEntry {
            action: act(t, position, amount),
        },
    ]))
    .map(|_| ())
}

fn borrow(t: &LendingTest, to: &Address, position: i128, amount: i128) -> PoolPositionMutation {
    pool(t)
        .borrow(
            to,
            &vec![
                &t.env,
                PoolBorrowEntry {
                    action: act(t, position, amount),
                },
            ],
        )
        .get(0)
        .unwrap()
}

fn withdraw_all(t: &LendingTest, to: &Address, position: i128) -> PoolPositionMutation {
    pool(t)
        .withdraw(
            to,
            &false,
            &vec![
                &t.env,
                PoolWithdrawEntry {
                    action: act(t, position, i128::MAX),
                    protocol_fee: 0,
                },
            ],
        )
        .get(0)
        .unwrap()
}

/// Repays `position` in full from `payer`'s wallet, with `amount` paid in and
/// any overpayment refunded by the pool.
fn repay_all(t: &LendingTest, payer: &Address, position: i128, amount: i128) {
    pay_in(t, payer, amount);
    pool(t).repay(payer, &vec![&t.env, act(t, position, amount)]);
}

fn accrue(t: &LendingTest, secs: u64) {
    t.advance_time_no_refresh(secs);
    pool(t).update_indexes(&vec![&t.env, market_key(t)]);
}

/// Pays `offered` in and recapitalizes; returns the amount the pool applied.
fn recapitalize(t: &LendingTest, payer: &Address, offered: i128) -> i128 {
    pay_in(t, payer, offered);
    pool(t)
        .recapitalize(&market_key(t), payer, &offered)
        .actual_amount
}

/// Converts the flat `try_*` result (contract error or conversion error) into one `Result`.
fn flat<T, E: Into<SdkError>>(
    r: Result<Result<T, E>, Result<SdkError, InvokeError>>,
) -> Result<T, SdkError> {
    match r {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(e)) => Err(e.into()),
        Err(Ok(e)) => Err(e),
        Err(Err(e)) => panic!("expected a contract error, got InvokeError {e:?}"),
    }
}

fn inject_state(t: &LendingTest, edit: impl FnOnce(&mut PoolStateRaw)) {
    let env: &Env = &t.env;
    let key = market_key(t);
    let mut injected = state(t);
    edit(&mut injected);
    env.as_contract(&pool_addr(t), || {
        env.storage()
            .persistent()
            .set(&PoolKey::State(key.clone()), &injected);
    });
}

/// `floor(claims)`, `ceil(debt)` and `max(0, floor(claims) - cash - ceil(debt))`,
/// the identity behind `guards::backing_shortfall`.
fn books(t: &LendingTest) -> (i128, i128, i128, i128) {
    let s = state(t);
    let env = &t.env;
    let claims = unscale_supply_floor(
        env,
        Ray::from(s.supplied),
        Ray::from(s.supply_index),
        DECIMALS,
    );
    let debt = unscale_borrow_ceil(
        env,
        Ray::from(s.borrowed),
        Ray::from(s.borrow_index),
        DECIMALS,
    );
    let shortfall = (claims - s.cash - debt).max(0);
    (claims, s.cash, debt, shortfall)
}

// ---------------------------------------------------------------------------
// CR-1: an exact recapitalization is undone by one accrual
// ---------------------------------------------------------------------------

/// Claims 1000.9 units, debt 500.1 units, cash 400 units. The retired
/// native-unit identity reported `1000 - 400 - 501 = 99`; the real gap is
/// `1000.9 - 400 - 500.1 = 100.8` and the guard now reports its floor, 100.
/// Recapitalizing that leaves 0.8 units of real under-backing. One day at 10%
/// APR adds about 0.27 units to both sides, so on this 7-decimal book the gap
/// stays below one unit and supply stays admitted; a second recapitalization
/// has nothing to apply.
#[test]
fn cr1_exact_recapitalization_stays_closed_after_accrual() {
    let t = fixture();
    let env = &t.env;
    let alice = Address::generate(env);
    let bob = Address::generate(env);
    let payer = Address::generate(env);

    // Real custody: 1000 USDC in, 500 USDC out, so every refund below is funded.
    supply(&t, &alice, 0, 1_000 * UNIT);
    borrow(&t, &bob, 0, 500 * UNIT);
    set_model(&t, ten_percent_flat);

    // Fractional books at index 1.0: claims 1000.9, debt 500.1, cash 400 (raw units).
    inject_state(&t, |s| {
        s.supply_index = RAY;
        s.borrow_index = RAY;
        s.supplied = 10_009 * (SCALE / 10);
        s.borrowed = 5_001 * (SCALE / 10);
        s.revenue = 0;
        s.cash = 400;
    });
    let (claims, cash, debt, native) = books(&t);
    assert_eq!((claims, cash, debt), (1_000, 400, 501));
    assert_eq!(
        native, 99,
        "the retired identity under-reported by one unit"
    );
    assert_contract_error(try_supply(&t, 0, 0), POOL_INSOLVENT);

    // Exact recapitalization at RAY precision: the pool applies floor(100.8)
    // and refunds the rest.
    assert_eq!(recapitalize(&t, &payer, 150), 100);
    assert_eq!(state(&t).cash, 500);
    try_supply(&t, 0, 0).expect("supply admitted right after the exact recap");

    // One day of accrual. Interest moves both sides by ~0.27 units; the real
    // gap stays at 0.8 units, so nothing reopens.
    accrue(&t, DAY_SECS);
    let (claims, cash, debt, native) = books(&t);
    std::println!(
        "CR-1 after 1 day: floor(claims)={claims} cash={cash} ceil(debt)={debt} native identity={native}"
    );
    assert_eq!((claims, cash, debt), (1_001, 500, 501));
    try_supply(&t, 0, 0).expect("supply stays admitted after accrual");
    assert_eq!(recapitalize(&t, &payer, 10), 0, "nothing left to apply");

    // On this 7-decimal book no later accrual reports a unit either.
    for _ in 0..12 {
        accrue(&t, 30 * DAY_SECS);
        try_supply(&t, 0, 0).expect("no second recapitalization is needed");
    }
    supply(&t, &alice, 0, 10 * UNIT);
}

// ---------------------------------------------------------------------------
// CR-2: rounding cash with zero supply shares can be borrowed
// ---------------------------------------------------------------------------

/// A normal lifecycle (supply, borrow, accrue, repay, exit, claim revenue)
/// leaves cash with zero supply shares: every exit rounds down and the pool
/// keeps the remainder. Before PR #196 (76b5c68) `borrow` accepted that cash,
/// and the dust debt made `claim_revenue` revert with `PoolInsolvent` (#123)
/// under a zero base rate however much time passed. `mint_debt` now refuses
/// the debt mint, so the remainder stays inert cash, the revenue claim keeps
/// returning zero, and the market reopens with the next supplier.
#[test]
fn cr2_leftover_cash_with_zero_supply_shares_cannot_be_borrowed() {
    let t = fixture();
    let env = &t.env;
    let alice = Address::generate(env);
    let bob = Address::generate(env);
    let carol = Address::generate(env);
    let dave = Address::generate(env);
    let pl = pool(&t);
    let key = market_key(&t);

    // Natural lifecycle under the default curve (1% base, 10% reserve factor).
    let p_alice = supply(&t, &alice, 0, 1_000 * UNIT).position.scaled_amount;
    let p_bob = borrow(&t, &bob, 0, 500 * UNIT).position.scaled_amount;
    accrue(&t, 30 * DAY_SECS);
    repay_all(&t, &bob, p_bob, 600 * UNIT);
    withdraw_all(&t, &alice, p_alice);
    pl.claim_revenue(&key);

    let s = state(&t);
    std::println!(
        "CR-2 after the market empties: supplied={} borrowed={} revenue={} cash={}",
        s.supplied,
        s.borrowed,
        s.revenue,
        s.cash
    );
    assert_eq!((s.supplied, s.borrowed, s.revenue), (0, 0, 0));
    assert!(s.cash > 0, "rounding leaves cash with no owner");
    let leftover = s.cash;

    // Carol (collateral elsewhere, in the controller's view) cannot borrow it.
    let refused = flat(pl.try_borrow(
        &carol,
        &vec![
            env,
            PoolBorrowEntry {
                action: act(&t, 0, leftover),
            },
        ],
    ));
    assert_contract_error(refused, POOL_INSOLVENT);
    let s = state(&t);
    assert_eq!(
        (s.supplied, s.borrowed, s.cash),
        (0, 0, leftover),
        "the refused mint leaves the books unchanged"
    );

    // Zero rate at zero utilization (hub-level mainnet models): nothing
    // accrues, and the revenue claim returns zero instead of reverting.
    set_model(&t, zero_base_rate);
    for _ in 0..12 {
        accrue(&t, 30 * DAY_SECS);
    }
    assert_eq!(state(&t).supplied, 0, "nothing ever mints a share");
    assert_eq!(pl.claim_revenue(&key).actual_amount, 0);

    // The next supplier reopens the market and can leave again at once.
    let p_dave = supply(&t, &dave, 0, UNIT).position.scaled_amount;
    assert_eq!(pl.claim_revenue(&key).actual_amount, 0);
    let out = withdraw_all(&t, &dave, p_dave);
    assert_eq!(out.position.scaled_amount, 0, "dave exits in full");
}
