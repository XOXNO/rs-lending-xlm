//! Second opinion on Specula lending CR-1 and CR-2, driven through the deployed
//! pool WASM as its Ownable owner (mocked auth stands in for the controller).
//!
//! CR-1: `guards::backing_shortfall` floors the claim side and ceils the debt
//! side to whole units before subtracting, so an exact recapitalization can
//! leave up to two units of real under-backing. One accrual can then move the
//! floor of the claims across a unit boundary while the ceiling of the debt
//! stays put, and `supply` reverts with `PoolInsolvent` until one more unit is
//! recapitalized.
//!
//! CR-2: a market whose shares all left but whose cash kept the rounding
//! remainder can be borrowed from. `mint_debt` has no supply-for-debt check,
//! the liquidation buffer is zero on zero supply, and the utilization gate
//! returns early on zero supply. `claim_revenue` then reverts on
//! `require_supply_for_debt`, and under a zero base rate (every hub-level
//! mainnet rate model in `configs/ops/mainnet`) no interest ever mints the
//! revenue share that would clear the state.
//!
//! Shortfall is recomputed with the `common::rates` helpers from `PoolStateRaw`,
//! exactly as `guards::backing_shortfall` does (the guard is crate-private).

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

/// Claims 1000.9 units, debt 500.1 units, cash 400 units. The guard reports
/// `1000 - 400 - 501 = 99` while the real gap is `1000.9 - 400 - 500.1 = 100.8`.
/// Recapitalizing the reported 99 leaves 1.8 units of real under-backing.
/// One day at 10% APR adds about 0.27 units to both sides: the claim floor
/// steps 1000 -> 1001, the debt ceiling stays at 501, and supply is refused
/// until one more unit is paid in. After that the real gap is below one unit
/// and no later accrual can reopen it.
#[test]
fn cr1_exact_recapitalization_reopens_after_one_accrual() {
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
    let (claims, cash, debt, shortfall) = books(&t);
    assert_eq!((claims, cash, debt), (1_000, 400, 501));
    assert_eq!(shortfall, 99, "guard: floor(claims) - cash - ceil(debt)");
    assert_contract_error(try_supply(&t, 0, 0), POOL_INSOLVENT);

    // Exact recapitalization: the pool applies the reported 99 and nothing more.
    assert_eq!(recapitalize(&t, &payer, 99), 99);
    let (_, cash, _, shortfall) = books(&t);
    assert_eq!((cash, shortfall), (499, 0));
    try_supply(&t, 0, 0).expect("supply admitted right after the exact recap");

    // One day of accrual. Interest moves both sides by ~0.27 units; only the
    // claim side crosses a unit boundary.
    accrue(&t, DAY_SECS);
    let (claims, cash, debt, shortfall) = books(&t);
    std::println!(
        "CR-1 after 1 day: floor(claims)={claims} cash={cash} ceil(debt)={debt} shortfall={shortfall}"
    );
    assert_eq!((claims, cash, debt), (1_001, 499, 501));
    assert_eq!(shortfall, 1, "the reported shortfall reopened by one unit");
    // Supply is blocked again although the market was recapitalized exactly.
    assert_contract_error(try_supply(&t, 0, 0), POOL_INSOLVENT);
    assert_contract_error(try_supply(&t, 0, 10 * UNIT), POOL_INSOLVENT);

    // A second recapitalization of one unit (nine refunded) closes it for good.
    assert_eq!(recapitalize(&t, &payer, 10), 1);
    assert_eq!(books(&t).3, 0);
    try_supply(&t, 0, 0).expect("supply admitted after the second recap");

    // Real under-backing is now below one unit; accrual is rounded in the
    // protocol's favour, so the reported shortfall can never reach one again.
    for _ in 0..12 {
        accrue(&t, 30 * DAY_SECS);
        assert_eq!(books(&t).3, 0, "no third recapitalization is ever needed");
    }
    supply(&t, &alice, 0, 10 * UNIT);
}

// ---------------------------------------------------------------------------
// CR-2: rounding cash with zero supply shares can be borrowed
// ---------------------------------------------------------------------------

/// A normal lifecycle (supply, borrow, accrue, repay, exit, claim revenue)
/// leaves cash with zero supply shares: every exit rounds down and the pool
/// keeps the remainder. `borrow` accepts that cash because no debt-mint guard
/// looks at `supplied`. `claim_revenue` then reverts with `PoolInsolvent`
/// (#123), stays reverted under a zero base rate however much time passes,
/// and clears only when a share is minted: by any supply, or by interest once
/// the rate at zero utilization is positive.
#[test]
fn cr2_leftover_cash_is_borrowable_against_zero_supply_and_blocks_claim_revenue() {
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

    // Carol (collateral elsewhere, in the controller's view) borrows the remainder.
    let p_carol = borrow(&t, &carol, 0, leftover).position.scaled_amount;
    assert!(p_carol > 0);
    let s = state(&t);
    assert_eq!((s.supplied, s.cash), (0, 0));
    assert!(s.borrowed > 0, "debt now exists against zero supply shares");

    // The revenue claim for this market is refused.
    assert_contract_error(flat(pl.try_claim_revenue(&key)), POOL_INSOLVENT);

    // Zero rate at zero utilization (hub-level mainnet models): no interest, no
    // revenue share, and the refusal persists through a year of accrual.
    set_model(&t, zero_base_rate);
    for _ in 0..12 {
        accrue(&t, 30 * DAY_SECS);
    }
    assert_eq!(state(&t).supplied, 0, "nothing ever mints a share");
    assert_contract_error(flat(pl.try_claim_revenue(&key)), POOL_INSOLVENT);

    // Any positive rate at zero utilization clears it: the next accrual mints
    // a revenue share, `supplied` turns positive and the claim passes.
    set_model(&t, ten_percent_flat);
    accrue(&t, DAY_SECS);
    let s = state(&t);
    assert!(s.supplied > 0 && s.supplied == s.revenue);
    assert_eq!(
        pl.claim_revenue(&key).actual_amount,
        0,
        "nothing claimable, no revert"
    );

    // Back under a zero rate, the state can be re-entered and is also cleared
    // by one raw unit of supply from anyone. A one-unit supplier is then the
    // last supplier: `require_supply_for_debt` keeps that unit locked until the
    // dust debt is repaid.
    set_model(&t, zero_base_rate);
    inject_state(&t, |s| {
        s.supplied = 0;
        s.revenue = 0;
    });
    assert_contract_error(flat(pl.try_claim_revenue(&key)), POOL_INSOLVENT);
    let p_dave = supply(&t, &dave, 0, 1).position.scaled_amount;
    assert_eq!(pl.claim_revenue(&key).actual_amount, 0);
    assert_contract_error(
        flat(pl.try_withdraw(
            &dave,
            &false,
            &vec![
                env,
                PoolWithdrawEntry {
                    action: act(&t, p_dave, i128::MAX),
                    protocol_fee: 0,
                },
            ],
        )),
        POOL_INSOLVENT,
    );
}
