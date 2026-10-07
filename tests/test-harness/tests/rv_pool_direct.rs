//! W6 (rv_pool_direct): hypotheses H1-H10 of the pool review, driven through the
//! deployed pool WASM as its Ownable owner (the controller). Mocked auth stands in
//! for the owner; what is tested is the pool's own checks and books.
//!
//! Expected values are rebuilt with BigInt from the rounding rules in
//! docs/reference/formulas.md ("Shares and token amounts", "Revenue payout"), not
//! with the contract's fixed-point helpers. Amounts are 7-decimal raw units
//! (1 whole token = 10_000_000), so one raw unit is 1e20 RAY.

use common::constants::{MAX_BORROW_INDEX_RAY, RAY, SUPPLY_INDEX_FLOOR_RAW};
use common::errors::{CollateralError, FlashLoanError, GenericError};
use common::rates::simulate_update_indexes;
use common::types::{
    AccountPositionType, HubAssetKey, InterestRateModel, MarketParamsRaw, PoolAction,
    PoolBorrowEntry, PoolKey, PoolNetSettleEntry, PoolPositionMutation, PoolSeizeEntry,
    PoolStateRaw, PoolSupplyEntry, PoolSyncData, PoolWithdrawEntry, ScaledPositionRaw,
};
use num_bigint::BigInt;
use pool::LiquidityPoolClient;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{
    contract, contractimpl, token, vec, Address, Bytes, Env, Error as SdkError, InvokeError,
};
use test_harness::{assert_contract_error, hub_asset, LendingTest};

const UNIT: i128 = 10_000_000;
/// RAY per raw unit at 7 decimals (1e27 / 1e7).
const SCALE: i128 = 100_000_000_000_000_000_000;
const YEAR_SECS: u64 = 31_556_926;
const MONTH_SECS: u64 = 2_592_000;

const AMOUNT_MUST_BE_POSITIVE: u32 = GenericError::AmountMustBePositive as u32;
const MATH_OVERFLOW: u32 = GenericError::MathOverflow as u32;
const REPAY_ROUNDS_TO_ZERO_SHARES: u32 = GenericError::RepayRoundsToZeroShares as u32;
const INSUFFICIENT_LIQUIDITY: u32 = CollateralError::InsufficientLiquidity as u32;
const UTILIZATION_ABOVE_MAX: u32 = CollateralError::UtilizationAboveMax as u32;
const POOL_INSOLVENT: u32 = CollateralError::PoolInsolvent as u32;
const INVALID_UTIL_RANGE: u32 = CollateralError::InvalidUtilRange as u32;
const OPT_UTIL_TOO_HIGH: u32 = CollateralError::OptUtilTooHigh as u32;
const INVALID_RESERVE_FACTOR: u32 = CollateralError::InvalidReserveFactor as u32;
const INVALID_BORROW_PARAMS: u32 = CollateralError::InvalidBorrowParams as u32;
const MAX_BORROW_RATE_TOO_HIGH: u32 = CollateralError::MaxBorrowRateTooHigh as u32;
const BASE_RATE_NEGATIVE: u32 = CollateralError::BaseRateNegative as u32;
const SLOPE_NON_MONOTONIC: u32 = CollateralError::SlopeNonMonotonic as u32;
const MAX_RATE_BELOW_BASE: u32 = CollateralError::MaxRateBelowBase as u32;
const ASSET_DECIMALS_TOO_HIGH: u32 = CollateralError::AssetDecimalsTooHigh as u32;
const FLASHLOAN_NOT_ENABLED: u32 = FlashLoanError::FlashloanNotEnabled as u32;
const INVALID_FLASHLOAN_REPAY: u32 = FlashLoanError::InvalidFlashloanRepay as u32;

/// Flash-loan receiver used by these probes. `data[0]` picks the callback:
/// 0 approve principal + fee; 1 transfer principal + fee straight to the pool;
/// 2 approve principal + fee and push one raw unit; 3 approve principal + fee + 5;
/// anything else approves nothing.
#[contract]
pub struct RvFlashReceiver;

#[contractimpl]
impl RvFlashReceiver {
    pub fn execute_flash_loan(
        env: Env,
        _initiator: Address,
        asset: Address,
        amount: i128,
        fee: i128,
        pool: Address,
        data: Bytes,
    ) {
        let me = env.current_contract_address();
        let tok = token::Client::new(&env, &asset);
        let total = amount + fee;
        let expiration = env.ledger().sequence() + 100;
        match data.get(0).unwrap_or(0) {
            0 => tok.approve(&me, &pool, &total, &expiration),
            1 => tok.transfer(&me, &pool, &total),
            2 => {
                tok.approve(&me, &pool, &total, &expiration);
                tok.transfer(&me, &pool, &1);
            }
            3 => tok.approve(&me, &pool, &(total + 5), &expiration),
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Fixture and client helpers
// ---------------------------------------------------------------------------

fn fixture() -> LendingTest {
    let mut preset = test_harness::usdc_preset();
    preset.initial_liquidity = 0.0;
    let t = LendingTest::new().with_market(preset).build();
    set_model(&t, |m| {
        m.max_utilization = RAY;
        m.is_flashloanable = true;
        m.flashloan_fee = 100;
    });
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

fn token_client(t: &LendingTest) -> token::Client<'_> {
    token::Client::new(&t.env, &t.resolve_asset("USDC"))
}

fn balance(t: &LendingTest, who: &Address) -> i128 {
    token_client(t).balance(who)
}

fn sync(t: &LendingTest) -> PoolSyncData {
    pool(t).get_sync_data(&market_key(t))
}

fn state(t: &LendingTest) -> PoolStateRaw {
    sync(t).state
}

fn set_model(t: &LendingTest, edit: impl FnOnce(&mut InterestRateModel)) {
    let key = market_key(t);
    let mut model = sync(t).params.rate_model_view();
    edit(&mut model);
    pool(t).update_params(&key, &model);
}

/// Flat 200% rate from 50% utilization up, so any book at or above that accrues at the cap.
fn cap_rate_curve(m: &mut InterestRateModel) {
    m.max_borrow_rate = 2 * RAY;
    m.base_borrow_rate = 0;
    m.slope1 = 2 * RAY;
    m.slope2 = 2 * RAY;
    m.slope3 = 2 * RAY;
    m.mid_utilization = RAY / 2;
    m.optimal_utilization = RAY * 8 / 10;
    m.max_utilization = RAY;
}

/// Flat annual rate `rate` from 50% utilization up.
fn flat_rate_curve(rate: i128) -> impl FnOnce(&mut InterestRateModel) {
    move |m: &mut InterestRateModel| {
        m.max_borrow_rate = rate;
        m.base_borrow_rate = 0;
        m.slope1 = rate;
        m.slope2 = rate;
        m.slope3 = rate;
        m.mid_utilization = RAY / 2;
        m.optimal_utilization = RAY * 8 / 10;
        m.max_utilization = RAY;
    }
}

fn act(t: &LendingTest, position: i128, amount: i128) -> PoolAction {
    PoolAction {
        position: pos(position),
        amount,
        hub_asset: market_key(t),
    }
}

fn pos(scaled_amount: i128) -> ScaledPositionRaw {
    ScaledPositionRaw { scaled_amount }
}

fn mint(t: &LendingTest, who: &Address, amount: i128) {
    t.resolve_market("USDC").token_admin.mint(who, &amount);
}

/// Mints `amount` to `who` and moves it into the pool, as the controller does
/// before supply, repay and recapitalize. Returns `who`'s balance afterwards.
fn pay_in(t: &LendingTest, who: &Address, amount: i128) -> i128 {
    mint(t, who, amount);
    let market = t.resolve_market("USDC");
    token::Client::new(&t.env, &market.asset).transfer(who, &market.pool, &amount);
    balance(t, who)
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

fn withdraw(t: &LendingTest, to: &Address, position: i128, amount: i128) -> PoolPositionMutation {
    pool(t)
        .withdraw(
            to,
            &false,
            &vec![
                &t.env,
                PoolWithdrawEntry {
                    action: act(t, position, amount),
                    protocol_fee: 0,
                },
            ],
        )
        .get(0)
        .unwrap()
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Book {
    supplied: i128,
    borrowed: i128,
    revenue: i128,
    supply_index: i128,
    borrow_index: i128,
    cash: i128,
    pool_balance: i128,
}

fn book(t: &LendingTest) -> Book {
    let s = state(t);
    Book {
        supplied: s.supplied,
        borrowed: s.borrowed,
        revenue: s.revenue,
        supply_index: s.supply_index,
        borrow_index: s.borrow_index,
        cash: s.cash,
        pool_balance: balance(t, &pool_addr(t)),
    }
}

// ---------------------------------------------------------------------------
// Exact rounding rules, recomputed with BigInt (formulas.md)
// ---------------------------------------------------------------------------

fn bi(x: i128) -> BigInt {
    BigInt::from(x)
}

fn ray() -> BigInt {
    BigInt::from(RAY)
}

fn i(b: &BigInt) -> i128 {
    b.to_string().parse().expect("value fits i128")
}

fn ceil_div(a: &BigInt, d: &BigInt) -> BigInt {
    (a + d - BigInt::from(1)) / d
}

/// `Ray::mul`: half-up multiply-divide by RAY.
fn mul_ray_half(x: &BigInt, y: &BigInt) -> BigInt {
    (x * y + BigInt::from(RAY / 2)) / ray()
}

/// Half-up, floor and ceil conversions from RAY to 7-decimal raw units.
fn raw_half(v: &BigInt) -> BigInt {
    (v + BigInt::from(SCALE / 2)) / BigInt::from(SCALE)
}

fn raw_floor(v: &BigInt) -> BigInt {
    v / BigInt::from(SCALE)
}

fn raw_ceil(v: &BigInt) -> BigInt {
    ceil_div(v, &BigInt::from(SCALE))
}

/// Scaled shares for `amount` raw units at `index`: floor for supply mints and
/// partial repayments, ceil for partial withdrawals and debt mints.
fn shares(amount: i128, index: i128, round_up: bool) -> BigInt {
    let num = bi(amount) * BigInt::from(SCALE) * ray();
    if round_up {
        ceil_div(&num, &bi(index))
    } else {
        num / bi(index)
    }
}

/// `unscale_supply` / `unscale_borrow`: half-up at both steps.
fn value_half(shares: i128, index: i128) -> BigInt {
    raw_half(&mul_ray_half(&bi(shares), &bi(index)))
}

/// `unscale_supply_floor`: floor at both steps.
fn value_floor(shares: i128, index: i128) -> BigInt {
    raw_floor(&(bi(shares) * bi(index) / ray()))
}

/// `unscale_borrow_ceil`: ceil at both steps.
fn value_ceil(shares: i128, index: i128) -> BigInt {
    raw_ceil(&ceil_div(&(bi(shares) * bi(index)), &ray()))
}

/// `guards::backing_shortfall`, recomputed from the stored books.
fn shortfall(t: &LendingTest) -> BigInt {
    let s = state(t);
    let claims = value_floor(s.supplied, s.supply_index);
    let backing = bi(s.cash) + value_ceil(s.borrowed, s.borrow_index);
    let gap = claims - backing;
    if gap > BigInt::from(0) {
        gap
    } else {
        BigInt::from(0)
    }
}

fn now_ms(t: &LendingTest) -> u64 {
    t.env.ledger().timestamp() * 1000
}

// ---------------------------------------------------------------------------
// H1: input validation
// ---------------------------------------------------------------------------

#[test]
fn rv_negative_amounts_rejected_with_amount_must_be_positive() {
    let t = fixture();
    let env = &t.env;
    let alice = Address::generate(env);
    let bob = Address::generate(env);
    let payer = Address::generate(env);
    let receiver = env.register(RvFlashReceiver, ());
    let key = market_key(&t);
    let p_alice = supply(&t, &alice, 0, 1_000 * UNIT).position.scaled_amount;
    let p_bob = borrow(&t, &bob, 0, 500 * UNIT).position.scaled_amount;
    let before = book(&t);
    let pl = pool(&t);

    assert_contract_error(
        flat(pl.try_supply(&vec![
            env,
            PoolSupplyEntry {
                action: act(&t, 0, -1),
            },
        ])),
        AMOUNT_MUST_BE_POSITIVE,
    );
    assert_contract_error(
        flat(pl.try_borrow(
            &bob,
            &vec![
                env,
                PoolBorrowEntry {
                    action: act(&t, 0, -1),
                },
            ],
        )),
        AMOUNT_MUST_BE_POSITIVE,
    );
    assert_contract_error(
        flat(pl.try_borrow(
            &bob,
            &vec![
                env,
                PoolBorrowEntry {
                    action: act(&t, 0, 0),
                },
            ],
        )),
        AMOUNT_MUST_BE_POSITIVE,
    );
    assert_contract_error(
        flat(pl.try_withdraw(
            &alice,
            &false,
            &vec![
                env,
                PoolWithdrawEntry {
                    action: act(&t, p_alice, -1),
                    protocol_fee: 0,
                },
            ],
        )),
        AMOUNT_MUST_BE_POSITIVE,
    );
    assert_contract_error(
        flat(pl.try_withdraw(
            &alice,
            &true,
            &vec![
                env,
                PoolWithdrawEntry {
                    action: act(&t, p_alice, UNIT),
                    protocol_fee: -1,
                },
            ],
        )),
        AMOUNT_MUST_BE_POSITIVE,
    );
    assert_contract_error(
        flat(pl.try_repay(&bob, &vec![env, act(&t, p_bob, -1)])),
        AMOUNT_MUST_BE_POSITIVE,
    );

    let settle = |amount: i128, supply_pos: i128, debt_pos: i128| PoolNetSettleEntry {
        hub_asset: key.clone(),
        amount,
        supply_position: pos(supply_pos),
        debt_position: pos(debt_pos),
    };
    assert_contract_error(
        flat(pl.try_net_settle(&settle(-1, p_alice, p_bob))),
        AMOUNT_MUST_BE_POSITIVE,
    );
    assert_contract_error(
        flat(pl.try_net_settle(&settle(UNIT, -1, p_bob))),
        AMOUNT_MUST_BE_POSITIVE,
    );
    assert_contract_error(
        flat(pl.try_net_settle(&settle(UNIT, p_alice, -1))),
        AMOUNT_MUST_BE_POSITIVE,
    );

    let seize = |side: AccountPositionType, scaled: i128| PoolSeizeEntry {
        hub_asset: key.clone(),
        side,
        position: pos(scaled),
    };
    assert_contract_error(
        flat(pl.try_seize_positions(&vec![env, seize(AccountPositionType::Deposit, -1)])),
        AMOUNT_MUST_BE_POSITIVE,
    );
    assert_contract_error(
        flat(pl.try_seize_positions(&vec![env, seize(AccountPositionType::Borrow, -1)])),
        AMOUNT_MUST_BE_POSITIVE,
    );
    assert_contract_error(
        flat(pl.try_recapitalize(&key, &payer, &-1)),
        AMOUNT_MUST_BE_POSITIVE,
    );
    assert_contract_error(
        flat(pl.try_create_strategy(&bob, &act(&t, 0, -1), &true)),
        AMOUNT_MUST_BE_POSITIVE,
    );
    assert_contract_error(
        flat(pl.try_create_strategy(&bob, &act(&t, 0, 0), &false)),
        AMOUNT_MUST_BE_POSITIVE,
    );
    assert_contract_error(
        flat(pl.try_flash_loan(&key, &alice, &receiver, &-1, &Bytes::new(env))),
        AMOUNT_MUST_BE_POSITIVE,
    );
    assert_contract_error(
        flat(pl.try_flash_loan(&key, &alice, &receiver, &0, &Bytes::new(env))),
        AMOUNT_MUST_BE_POSITIVE,
    );

    // A negative position is not rejected as AmountMustBePositive on the exits. Withdraw
    // fails in Ray::mul on the negative operand; repay's ceiled debt goes negative and
    // the net-repay check fails.
    assert_contract_error(
        flat(pl.try_withdraw(
            &alice,
            &false,
            &vec![
                env,
                PoolWithdrawEntry {
                    action: act(&t, -1, UNIT),
                    protocol_fee: 0,
                },
            ],
        )),
        MATH_OVERFLOW,
    );
    // Repay: a debt of -1 raw shares ceils to zero value, so the net-repay reaches
    // checked_sub (33). A debt of -1e27 shares ceils to -1e7 raw and fails the
    // net-repay share check (52). The code depends on magnitude, not on a sign check.
    assert_contract_error(
        flat(pl.try_repay(&bob, &vec![env, act(&t, -1, UNIT)])),
        MATH_OVERFLOW,
    );
    assert_contract_error(
        flat(pl.try_repay(&bob, &vec![env, act(&t, -RAY, UNIT)])),
        REPAY_ROUNDS_TO_ZERO_SHARES,
    );
    assert_eq!(
        book(&t),
        before,
        "rejected calls must leave the books unchanged"
    );
}

#[test]
#[ignore = "RV-FINDING: pool accepts a negative position.scaled_amount on supply, borrow and create_strategy"]
fn rv_finding_negative_scaled_position_accepted_on_supply_borrow_strategy() {
    let t = fixture();
    let env = &t.env;
    let alice = Address::generate(env);
    let bob = Address::generate(env);
    supply(&t, &alice, 0, 1_000 * UNIT);
    let pl = pool(&t);
    let mut failures: Vec<String> = Vec::new();

    match flat(pl.try_supply(&vec![
        env,
        PoolSupplyEntry {
            action: act(&t, -5, UNIT),
        },
    ])) {
        Err(e) if e == SdkError::from_contract_error(AMOUNT_MUST_BE_POSITIVE) => {}
        other => failures.push(format!("supply(position=-5, amount=1 unit): {other:?}")),
    }
    match flat(pl.try_borrow(
        &bob,
        &vec![
            env,
            PoolBorrowEntry {
                action: act(&t, -5, UNIT),
            },
        ],
    )) {
        Err(e) if e == SdkError::from_contract_error(AMOUNT_MUST_BE_POSITIVE) => {}
        other => failures.push(format!("borrow(position=-5, amount=1 unit): {other:?}")),
    }
    match flat(pl.try_create_strategy(&bob, &act(&t, -5, UNIT), &false)) {
        Err(e) if e == SdkError::from_contract_error(AMOUNT_MUST_BE_POSITIVE) => {}
        other => failures.push(format!(
            "create_strategy(position=-5, amount=1 unit): {other:?}"
        )),
    }
    assert!(
        failures.is_empty(),
        "negative scaled positions were not rejected:\n{}",
        failures.join("\n")
    );
}

#[test]
fn rv_zero_amounts_are_noop_or_close_without_moving_tokens() {
    let t = fixture();
    let env = &t.env;
    let alice = Address::generate(env);
    let bob = Address::generate(env);
    let p_alice = supply(&t, &alice, 0, 1_000 * UNIT).position.scaled_amount;
    let p_bob = borrow(&t, &bob, 0, 500 * UNIT).position.scaled_amount;
    let before = book(&t);
    let pl = pool(&t);

    // supply(0) mints no shares and credits no cash.
    let minted = pl
        .supply(&vec![
            env,
            PoolSupplyEntry {
                action: act(&t, 0, 0),
            },
        ])
        .get(0)
        .unwrap();
    assert_eq!(minted.position.scaled_amount, 0);
    assert_eq!(book(&t), before);

    // borrow(0) is rejected.
    assert_contract_error(
        flat(pl.try_borrow(
            &bob,
            &vec![
                env,
                PoolBorrowEntry {
                    action: act(&t, p_bob, 0),
                },
            ],
        )),
        AMOUNT_MUST_BE_POSITIVE,
    );

    // repay(0) against a live debt is a no-op: no shares burned, no cash moved.
    let repaid = pl
        .repay(&bob, &vec![env, act(&t, p_bob, 0)])
        .get(0)
        .unwrap();
    assert_eq!(
        (repaid.position.scaled_amount, repaid.actual_amount),
        (p_bob, 0)
    );
    assert_eq!(book(&t), before);

    // withdraw(0) against a live position (half-up value >= 1 raw) is a no-op.
    let w = pl
        .withdraw(
            &alice,
            &false,
            &vec![
                env,
                PoolWithdrawEntry {
                    action: act(&t, p_alice, 0),
                    protocol_fee: 0,
                },
            ],
        )
        .get(0)
        .unwrap();
    assert_eq!((w.position.scaled_amount, w.actual_amount), (p_alice, 0));
    assert_eq!(book(&t), before);

    // withdraw(0) against a position worth 0.1 raw (1e19 shares at RAY, half-up 0)
    // closes it and burns the shares for nothing. Built from a controller-supplied
    // position: no authentic position with half-up value 0 exists.
    let dust = 10_000_000_000_000_000_000i128;
    let d = pl
        .withdraw(
            &alice,
            &false,
            &vec![
                env,
                PoolWithdrawEntry {
                    action: act(&t, dust, 0),
                    protocol_fee: 0,
                },
            ],
        )
        .get(0)
        .unwrap();
    assert_eq!((d.position.scaled_amount, d.actual_amount), (0, 0));
    let after = book(&t);
    assert_eq!(after.supplied, before.supplied - dust);
    assert_eq!(after.cash, before.cash);
    assert_eq!(after.pool_balance, before.pool_balance);
}

// ---------------------------------------------------------------------------
// H2: the pool trusts the controller's scaled position (INV-ACCT-10)
// ---------------------------------------------------------------------------

#[test]
fn rv_position_above_book_panics_on_full_exit_and_is_absorbed_on_partial() {
    let t = fixture();
    let env = &t.env;
    let alice = Address::generate(env);
    let bob = Address::generate(env);
    let p_a = supply(&t, &alice, 0, 1_000 * UNIT).position.scaled_amount;
    let p_b = borrow(&t, &bob, 0, 500 * UNIT).position.scaled_amount;
    let extra = RAY; // 1e27 shares = 1 whole token at index RAY
    let pl = pool(&t);
    let before = book(&t);

    // Full exit: the burn exceeds total supplied shares and the subtraction panics.
    assert_contract_error(
        flat(pl.try_withdraw(
            &alice,
            &false,
            &vec![
                env,
                PoolWithdrawEntry {
                    action: act(&t, p_a + extra, i128::MAX),
                    protocol_fee: 0,
                },
            ],
        )),
        MATH_OVERFLOW,
    );
    assert_eq!(book(&t), before);

    // Partial exit of one raw unit with the same phantom: accepted. Only 1e20 shares
    // burn, so the returned position stays above the market's supplied shares.
    let alice_before = balance(&t, &alice);
    let w = withdraw(&t, &alice, p_a + extra, 1);
    assert_eq!(w.actual_amount, 1);
    assert_eq!(w.position.scaled_amount, p_a + extra - SCALE);
    let after_partial = book(&t);
    assert_eq!(after_partial.supplied, before.supplied - SCALE);
    assert_eq!(after_partial.cash, before.cash - 1);
    assert_eq!(balance(&t, &alice) - alice_before, 1);
    assert!(
        w.position.scaled_amount > after_partial.supplied,
        "returned phantom position {} should exceed the market's supplied shares {}",
        w.position.scaled_amount,
        after_partial.supplied
    );

    // Full repay with a phantom debt: the burn exceeds borrowed shares.
    let phantom_debt = p_b + extra;
    let owed = i(&value_ceil(phantom_debt, RAY));
    assert_eq!(owed, 5_010_000_000, "ceil value of the phantom debt");
    assert_contract_error(
        flat(pl.try_repay(&bob, &vec![env, act(&t, phantom_debt, owed)])),
        MATH_OVERFLOW,
    );
    let before_repay = book(&t);

    // Partial repay of one raw unit: accepted, one scaled unit burns.
    pay_in(&t, &bob, 1);
    let r = pl
        .repay(&bob, &vec![env, act(&t, phantom_debt, 1)])
        .get(0)
        .unwrap();
    assert_eq!(r.actual_amount, 1);
    assert_eq!(r.position.scaled_amount, phantom_debt - SCALE);
    let after_repay = book(&t);
    assert_eq!(after_repay.borrowed, before_repay.borrowed - SCALE);
    assert_eq!(after_repay.cash, before_repay.cash + 1);

    // Seize on the borrow side with a position above borrowed: the burn panics.
    let over = after_repay.borrowed + extra;
    assert_contract_error(
        flat(pl.try_seize_positions(&vec![
            env,
            PoolSeizeEntry {
                hub_asset: market_key(&t),
                side: AccountPositionType::Borrow,
                position: pos(over),
            },
        ])),
        MATH_OVERFLOW,
    );
    assert_eq!(
        book(&t),
        after_repay,
        "a panicking seize must not write the index"
    );
}

// ---------------------------------------------------------------------------
// H3: rounding at the boundaries (INV-ACCT-05)
// ---------------------------------------------------------------------------

#[test]
fn rv_partial_withdraw_burns_ceil_and_full_close_pays_floor() {
    let t = fixture();
    let env = &t.env;
    let alice = Address::generate(env);
    let bob = Address::generate(env);
    let dave = Address::generate(env);
    let erin = Address::generate(env);
    let carol = Address::generate(env);
    let key = market_key(&t);
    supply(&t, &alice, 0, 1_000 * UNIT);
    borrow(&t, &bob, 0, 500 * UNIT);
    t.advance_time_no_refresh(YEAR_SECS);
    let pl = pool(&t);
    pl.update_indexes(&vec![env, key.clone()]);
    let index = state(&t).supply_index;
    assert!(
        index > RAY,
        "fixture must accrue supply interest, index {index}"
    );

    // H3a: at this index one raw unit still mints shares. Zero shares needs
    // index > 1e47 for a raw unit at 7 decimals, above MAX_SUPPLY_INDEX_RAY (1e36).
    let one = supply(&t, &carol, 0, 1);
    assert_eq!(bi(one.position.scaled_amount), shares(1, index, false));

    println!(
        "H3 index={index} one_raw_shares={}",
        shares(1, index, false)
    );
    // H3c: find a supply amount whose half-up display value exceeds its floor value.
    let x = (1_000_000i128..)
        .find(|&x| {
            let p = i(&shares(x, index, false));
            value_half(p, index) > value_floor(p, index)
        })
        .expect("a half-up/floor gap exists at this index");
    let p = i(&shares(x, index, false));
    let half = i(&value_half(p, index));
    let floor = value_floor(p, index);

    println!("H3 x={x} shares={p} half_up={half} floor={floor}");
    // Full close requested at the half-up balance: burns every share, pays the floor.
    supply(&t, &dave, 0, x);
    let dave_before = balance(&t, &dave);
    let full = withdraw(&t, &dave, p, half);
    assert_eq!(
        bi(full.actual_amount),
        floor,
        "pays floor value, not half-up"
    );
    assert_eq!(full.position.scaled_amount, 0, "every share burned");
    assert_eq!(bi(balance(&t, &dave) - dave_before), floor);

    // H3b: one unit below the half-up balance is a partial exit: burns ceil shares.
    supply(&t, &erin, 0, x);
    let partial = half - 1;
    let before = book(&t);
    let part = withdraw(&t, &erin, p, partial);
    assert_eq!(bi(part.actual_amount), bi(partial));
    let burned = shares(partial, index, true);
    assert_eq!(bi(part.position.scaled_amount), bi(p) - &burned);
    let after = book(&t);
    assert_eq!(bi(before.supplied - after.supplied), burned);
    assert_eq!(after.cash, before.cash - partial);
}

#[test]
fn rv_repay_exact_ceil_debt_burns_all_and_refunds_overpayment() {
    let t = fixture();
    let env = &t.env;
    let alice = Address::generate(env);
    let bob = Address::generate(env);
    let carol = Address::generate(env);
    let key = market_key(&t);
    supply(&t, &alice, 0, 1_000 * UNIT);
    let y1 = 3_333_333_333i128;
    let p1 = borrow(&t, &bob, 0, y1).position.scaled_amount;
    t.advance_time_no_refresh(YEAR_SECS);
    let pl = pool(&t);
    pl.update_indexes(&vec![env, key.clone()]);
    let index = state(&t).borrow_index;
    assert!(index > RAY);
    let owed1 = i(&value_ceil(p1, index));
    assert!(
        owed1 > y1,
        "interest must raise the ceiled debt above principal"
    );

    // Exactly ceil(debt): burns all shares, no refund.
    let wallet = pay_in(&t, &bob, owed1);
    let pre = book(&t);
    let r1 = pl
        .repay(&bob, &vec![env, act(&t, p1, owed1)])
        .get(0)
        .unwrap();
    assert_eq!(r1.actual_amount, owed1);
    assert_eq!(r1.position.scaled_amount, 0);
    assert_eq!(balance(&t, &bob), wallet, "no refund at exactly ceil(debt)");
    let post = book(&t);
    assert_eq!(post.borrowed, pre.borrowed - p1);
    assert_eq!(post.cash, pre.cash + owed1);
    assert_eq!(post.pool_balance, pre.pool_balance);

    // A fresh debt at the same index: ceil(debt) + 5 refunds exactly 5.
    let p2 = borrow(&t, &carol, 0, 1_234_567_891).position.scaled_amount;
    assert_eq!(
        bi(p2),
        shares(1_234_567_891, index, true),
        "debt mint rounds up"
    );
    let owed2 = i(&value_ceil(p2, index));
    let wallet2 = pay_in(&t, &carol, owed2 + 5);
    let pre2 = book(&t);
    let r2 = pl
        .repay(&carol, &vec![env, act(&t, p2, owed2 + 5)])
        .get(0)
        .unwrap();
    assert_eq!(r2.actual_amount, owed2);
    assert_eq!(r2.position.scaled_amount, 0);
    assert_eq!(
        balance(&t, &carol),
        wallet2 + 5,
        "overpayment of exactly 5 refunded"
    );
    let post2 = book(&t);
    assert_eq!(post2.cash, pre2.cash + owed2);
    assert_eq!(post2.pool_balance, pre2.pool_balance - 5);
}

#[test]
fn rv_net_settle_burns_min_overlap_and_closes_only_exhausted_side() {
    let t = fixture();
    let env = &t.env;
    let alice = Address::generate(env);
    let frank = Address::generate(env);
    let dave = Address::generate(env);
    let gina = Address::generate(env);
    let key = market_key(&t);
    supply(&t, &alice, 0, 1_000 * UNIT);
    let fs = supply(&t, &frank, 0, 300 * UNIT).position.scaled_amount;
    let fd = borrow(&t, &frank, 0, 290 * UNIT).position.scaled_amount;
    let ds = supply(&t, &dave, 0, 300 * UNIT).position.scaled_amount;
    let dd = borrow(&t, &dave, 0, 250 * UNIT).position.scaled_amount;
    let gs = supply(&t, &gina, 0, 100 * UNIT).position.scaled_amount;
    let gd = borrow(&t, &gina, 0, 99 * UNIT).position.scaled_amount;
    t.advance_time_no_refresh(YEAR_SECS);
    let pl = pool(&t);
    pl.update_indexes(&vec![env, key.clone()]);
    let s = state(&t);
    let (si, bi_idx) = (s.supply_index, s.borrow_index);
    let settle_entry = |amount: i128, sp: i128, dp: i128| PoolNetSettleEntry {
        hub_asset: key.clone(),
        amount,
        supply_position: pos(sp),
        debt_position: pos(dp),
    };

    println!("H3 net-settle: supply_floor(F)={} debt_ceil(F)={} supply_floor(D)={} debt_ceil(D)={} supply_floor(G)={} debt_ceil(G)={}", value_floor(fs, si), value_ceil(fd, bi_idx), value_floor(ds, si), value_ceil(dd, bi_idx), value_floor(gs, si), value_ceil(gd, bi_idx));
    // Case 1, requested amount below both sides: burns ceil supply and floor debt.
    let before = book(&t);
    let amount1 = 100 * UNIT;
    let r1 = pl.net_settle(&settle_entry(amount1, fs, fd));
    assert_eq!(bi(r1.settled_amount), bi(amount1));
    assert_eq!(
        bi(r1.supply_position.scaled_amount),
        bi(fs) - shares(amount1, si, true)
    );
    assert_eq!(
        bi(r1.debt_position.scaled_amount),
        bi(fd) - shares(amount1, bi_idx, false)
    );
    assert_eq!(book(&t).cash, before.cash, "net settle moves no cash");
    assert_eq!(book(&t).pool_balance, before.pool_balance);

    // Case 2, debt is the smaller side: the debt closes fully, supply burns ceil(debt value).
    let debt_ceil = value_ceil(dd, bi_idx);
    let supply_floor = value_floor(ds, si);
    assert!(
        debt_ceil < supply_floor,
        "fixture: debt side must be the exhausted one"
    );
    let r2 = pl.net_settle(&settle_entry(10_000 * UNIT, ds, dd));
    assert_eq!(bi(r2.settled_amount), debt_ceil);
    assert_eq!(r2.debt_position.scaled_amount, 0, "debt side fully closed");
    assert_eq!(
        bi(r2.supply_position.scaled_amount),
        bi(ds) - shares(i(&debt_ceil), si, true)
    );

    // Case 3, supply is the smaller side: supply closes fully, debt burns floor(supply value).
    let debt_ceil_g = value_ceil(gd, bi_idx);
    let supply_floor_g = value_floor(gs, si);
    assert!(
        supply_floor_g < debt_ceil_g,
        "fixture: supply side must be the exhausted one"
    );
    let r3 = pl.net_settle(&settle_entry(10_000 * UNIT, gs, gd));
    assert_eq!(bi(r3.settled_amount), supply_floor_g);
    assert_eq!(
        r3.supply_position.scaled_amount, 0,
        "supply side fully closed"
    );
    assert_eq!(
        bi(r3.debt_position.scaled_amount),
        bi(gd) - shares(i(&supply_floor_g), bi_idx, false)
    );
}

// ---------------------------------------------------------------------------
// H4: flash-loan exactness (INV-FLASH-01)
// ---------------------------------------------------------------------------

#[test]
fn rv_flash_loan_repayment_is_exact_and_books_only_the_fee() {
    let t = fixture();
    let env = &t.env;
    let alice = Address::generate(env);
    supply(&t, &alice, 0, 1_000 * UNIT);
    let key = market_key(&t);
    let pl = pool(&t);
    let receiver = env.register(RvFlashReceiver, ());
    let data = |mode: u8| Bytes::from_slice(env, &[mode]);
    let principal = 100 * UNIT;
    let fee = principal / 100; // 100 bps, exact
    mint(&t, &receiver, 10 * fee);
    let receiver_start = balance(&t, &receiver);

    // (a) Principal + fee transferred straight to the pool, no allowance: the
    // post-callback balance check fails.
    let before = book(&t);
    assert_contract_error(
        flat(pl.try_flash_loan(&key, &alice, &receiver, &principal, &data(1))),
        INVALID_FLASHLOAN_REPAY,
    );
    assert_eq!(book(&t), before);
    assert_eq!(balance(&t, &receiver), receiver_start);

    // (b) Approve principal + fee and also push one raw unit: the balance after the
    // callback is B - P + 1, not B - P.
    assert_contract_error(
        flat(pl.try_flash_loan(&key, &alice, &receiver, &principal, &data(2))),
        INVALID_FLASHLOAN_REPAY,
    );
    assert_eq!(book(&t), before);
    assert_eq!(balance(&t, &receiver), receiver_start);

    // Approving nothing fails the allowance check.
    assert_contract_error(
        flat(pl.try_flash_loan(&key, &alice, &receiver, &principal, &data(9))),
        INVALID_FLASHLOAN_REPAY,
    );
    assert_eq!(book(&t), before);

    // (c) Approve principal + fee + 5: the pool pulls exactly principal + fee.
    let rev_asset_before = pl.get_revenue(&key);
    let got = pl.flash_loan(&key, &alice, &receiver, &principal, &data(3));
    assert_eq!(got, fee);
    let after = book(&t);
    assert_eq!(after.cash, before.cash + fee);
    assert_eq!(after.pool_balance, before.pool_balance + fee);
    assert_eq!(after.borrowed, before.borrowed);
    let delta_shares = fee * SCALE; // floor(fee * RAY / supply_index) at index RAY
    assert_eq!(after.revenue, before.revenue + delta_shares);
    assert_eq!(after.supplied, before.supplied + delta_shares);
    assert_eq!(
        pl.get_revenue(&key),
        rev_asset_before + fee,
        "revenue grows by the fee in asset units"
    );
    assert_eq!(
        balance(&t, &receiver),
        receiver_start - fee,
        "receiver paid principal + fee, got principal"
    );
    assert_eq!(token_client(&t).allowance(&receiver, &pool_addr(&t)), 5);

    // (d) Tiny principals: the fee is at least one raw unit, then half-up.
    for (principal, expected_fee) in [(1i128, 1i128), (49, 1), (150, 2)] {
        let before = book(&t);
        let got = pl.flash_loan(&key, &alice, &receiver, &principal, &data(0));
        assert_eq!(got, expected_fee, "fee on principal {principal}");
        let after = book(&t);
        assert_eq!(after.cash, before.cash + expected_fee);
        assert_eq!(after.pool_balance, before.pool_balance + expected_fee);
        assert_eq!(after.revenue, before.revenue + expected_fee * SCALE);
    }

    // (e) Market not flashloanable: rejected before any token call.
    set_model(&t, |m| m.is_flashloanable = false);
    let before = book(&t);
    assert_contract_error(
        flat(pl.try_flash_loan(&key, &alice, &receiver, &principal, &data(0))),
        FLASHLOAN_NOT_ENABLED,
    );
    assert_eq!(book(&t), before);
    set_model(&t, |m| m.is_flashloanable = true);

    // (f) Amount above cash: rejected at the reserve check.
    let cash = state(&t).cash;
    assert_contract_error(
        flat(pl.try_flash_loan(&key, &alice, &receiver, &(cash + 1), &data(0))),
        INSUFFICIENT_LIQUIDITY,
    );
    assert_eq!(book(&t), before);
}

// ---------------------------------------------------------------------------
// H5: utilization gate (INV-ACCT-08)
// ---------------------------------------------------------------------------

#[test]
fn rv_utilization_gate_at_80pct_blocks_borrow_withdraw_and_claim() {
    let t = fixture();
    let env = &t.env;
    let alice = Address::generate(env);
    let bob = Address::generate(env);
    let key = market_key(&t);
    set_model(&t, |m| m.max_utilization = RAY * 8 / 10);
    let pl = pool(&t);
    let p_alice = supply(&t, &alice, 0, 1_000 * UNIT).position.scaled_amount;
    // Debt exactly 80% of floored supply passes; one raw unit more is above the ceiling.
    borrow(&t, &bob, 0, 800 * UNIT);
    assert_contract_error(
        flat(pl.try_borrow(
            &bob,
            &vec![
                env,
                PoolBorrowEntry {
                    action: act(&t, 0, 1),
                },
            ],
        )),
        UTILIZATION_ABOVE_MAX,
    );

    // A non-liquidation withdraw of one token takes utilization to 800/999: rejected.
    let before = book(&t);
    assert_contract_error(
        flat(pl.try_withdraw(
            &alice,
            &false,
            &vec![
                env,
                PoolWithdrawEntry {
                    action: act(&t, p_alice, UNIT),
                    protocol_fee: 0,
                },
            ],
        )),
        UTILIZATION_ABOVE_MAX,
    );
    assert_eq!(book(&t), before);

    // The same withdraw as a liquidation skips the gate and burns ceil shares.
    let liq = pl
        .withdraw(
            &alice,
            &true,
            &vec![
                env,
                PoolWithdrawEntry {
                    action: act(&t, p_alice, UNIT),
                    protocol_fee: 0,
                },
            ],
        )
        .get(0)
        .unwrap();
    assert_eq!(liq.actual_amount, UNIT);
    assert_eq!(
        bi(liq.position.scaled_amount),
        bi(p_alice) - shares(UNIT, RAY, true)
    );
    assert_eq!(book(&t).cash, before.cash - UNIT);

    // Accrual alone lifts utilization past 80%: the revenue claim is rejected.
    t.advance_time_no_refresh(YEAR_SECS);
    let before_claim = book(&t);
    assert_contract_error(flat(pl.try_claim_revenue(&key)), UTILIZATION_ABOVE_MAX);
    assert_eq!(book(&t), before_claim);

    // Gate lifted: the same claim pays min(cash, floor(revenue)).
    set_model(&t, |m| m.max_utilization = RAY);
    let s = state(&t);
    let payout = bi(s.cash).min(value_floor(s.revenue, s.supply_index));
    assert!(
        payout > BigInt::from(0),
        "fixture: revenue must be claimable"
    );
    let owner = t.controller.clone();
    let owner_before = balance(&t, &owner);
    assert_eq!(bi(pl.claim_revenue(&key).actual_amount), payout);
    assert_eq!(bi(balance(&t, &owner) - owner_before), payout);
}

// ---------------------------------------------------------------------------
// H6: liquidation cash buffer (INV-ACCT-07)
// ---------------------------------------------------------------------------

#[test]
fn rv_liquidation_buffer_rounds_up_and_strategy_checks_gross_principal() {
    // Borrow path: 2% of 100.0000001 USDC is 2_000_000.02 raw, so the reserve rounds up to 20_000_001.
    let t = fixture();
    let env = &t.env;
    let alice = Address::generate(env);
    let bob = Address::generate(env);
    let supplied = 1_000_000_001i128;
    supply(&t, &alice, 0, supplied);
    let reserve = i(&ceil_div(&(bi(supplied) * bi(200)), &bi(10_000)));
    assert_eq!(reserve, 20_000_001);
    let pl = pool(&t);
    let draw = supplied - reserve;
    assert_eq!(draw, 980_000_000);
    borrow(&t, &bob, 0, draw);
    assert_eq!(state(&t).cash, reserve, "cash lands exactly on the reserve");
    assert_contract_error(
        flat(pl.try_borrow(
            &bob,
            &vec![
                env,
                PoolBorrowEntry {
                    action: act(&t, 0, 1),
                },
            ],
        )),
        INSUFFICIENT_LIQUIDITY,
    );

    // Strategy path with a 1% fee: the gate uses gross principal, not principal minus fee.
    let t2 = fixture();
    let env2 = &t2.env;
    let alice2 = Address::generate(env2);
    let carol = Address::generate(env2);
    supply(&t2, &alice2, 0, supplied);
    let pl2 = pool(&t2);
    let before = book(&t2);
    // Net-of-fee check would pass (cash after net = 29_800_000 >= 20_000_001); gross fails.
    assert_contract_error(
        flat(pl2.try_create_strategy(&carol, &act(&t2, 0, 980_000_001), &true)),
        INSUFFICIENT_LIQUIDITY,
    );
    assert_eq!(book(&t2), before);

    let receiver_before = balance(&t2, &carol);
    let s = pl2.create_strategy(&carol, &act(&t2, 0, 980_000_000), &true);
    assert_eq!(s.actual_amount, 980_000_000);
    assert_eq!(
        s.amount_received, 970_200_000,
        "fee 1% = 9_800_000 withheld"
    );
    assert_eq!(balance(&t2, &carol) - receiver_before, 970_200_000);
    let after = book(&t2);
    assert_eq!(after.cash, before.cash - 970_200_000);
    assert_eq!(after.revenue, before.revenue + 9_800_000 * SCALE);
    assert_eq!(after.borrowed, 980_000_000 * SCALE);
}

// ---------------------------------------------------------------------------
// H7: write-down (INV-IDX-02, INV-IDX-03)
// ---------------------------------------------------------------------------

#[test]
fn rv_seize_writedown_matches_formula_and_backing_after() {
    let t = fixture();
    let env = &t.env;
    let alice = Address::generate(env);
    let bob = Address::generate(env);
    let key = market_key(&t);
    let p_a = supply(&t, &alice, 0, 1_000 * UNIT).position.scaled_amount;
    let p_b = borrow(&t, &bob, 0, 500 * UNIT).position.scaled_amount;
    t.advance_time_no_refresh(YEAR_SECS);
    pool(&t).update_indexes(&vec![env, key.clone()]);
    let s0 = state(&t);
    let short_before = shortfall(&t);

    // Expected index from the write-down rule, rebuilt in BigInt: bad debt ceiled, total
    // supplied value half-up, remaining/total floored, index floored, RAY/1000 clamp.
    let bad_ray = ceil_div(&(bi(p_b) * bi(s0.borrow_index)), &ray());
    let total = mul_ray_half(&bi(s0.supplied), &bi(s0.supply_index));
    let bad_capped = bad_ray.clone().min(total.clone());
    let remaining = &total - &bad_capped;
    let reduction = (&remaining * ray()) / &total;
    let expected =
        ((bi(s0.supply_index) * reduction) / ray()).max(BigInt::from(SUPPLY_INDEX_FLOOR_RAW));

    seize_borrow(&t, p_b);
    let s1 = state(&t);
    assert_eq!(bi(s1.supply_index), expected);
    // Lower bound: the index falls by at least bad / total (S1 * total <= S0 * (total - bad)).
    assert!(bi(s1.supply_index) * &total <= bi(s0.supply_index) * (&total - &bad_ray));
    assert_eq!(s1.borrowed, 0);
    assert_eq!(
        s1.supplied, s0.supplied,
        "borrow-side seize leaves supplied shares"
    );
    assert_eq!(s1.revenue, s0.revenue);
    assert_eq!(s1.cash, s0.cash);
    println!(
        "H7 write-down: S0={} S1={} bad_ray={} total={} shortfall_before={} shortfall_after={}",
        s0.supply_index,
        s1.supply_index,
        bad_ray,
        total,
        short_before,
        shortfall(&t)
    );
    let short_after = shortfall(&t);
    assert_eq!(
        short_after,
        BigInt::from(0),
        "backing shortfall before {short_before}, after {short_after}"
    );

    // Deposit-side seize: Alice's shares move into revenue; supplied and index unchanged.
    let before = state(&t);
    seize_deposit(&t, p_a);
    let after = state(&t);
    assert_eq!(after.supplied, before.supplied);
    assert_eq!(after.revenue, before.revenue + p_a);
    assert_eq!(after.supply_index, before.supply_index);
    assert_eq!(after.cash, before.cash);
}

fn seize_borrow(t: &LendingTest, scaled: i128) {
    pool(t).seize_positions(&vec![
        &t.env,
        PoolSeizeEntry {
            hub_asset: market_key(t),
            side: AccountPositionType::Borrow,
            position: pos(scaled),
        },
    ]);
}

fn seize_deposit(t: &LendingTest, scaled: i128) {
    pool(t).seize_positions(&vec![
        &t.env,
        PoolSeizeEntry {
            hub_asset: market_key(t),
            side: AccountPositionType::Deposit,
            position: pos(scaled),
        },
    ]);
}

#[test]
fn rv_seize_borrow_zero_supplied_and_floor_clamp() {
    // (i) Zero supplied value with debt. Unreachable through public paths (withdraw,
    // net settle and revenue claims all require supply for debt), so the state is injected.
    let t = fixture();
    let env = &t.env;
    let alice = Address::generate(env);
    let bob = Address::generate(env);
    supply(&t, &alice, 0, 1_000 * UNIT);
    let p_b = borrow(&t, &bob, 0, 500 * UNIT).position.scaled_amount;
    let mut injected = state(&t);
    injected.supplied = 0;
    let key = market_key(&t);
    env.as_contract(&pool_addr(&t), || {
        env.storage()
            .persistent()
            .set(&PoolKey::State(key.clone()), &injected);
    });
    let before = book(&t);
    assert_eq!(before.supplied, 0);
    seize_borrow(&t, p_b);
    let after = book(&t);
    assert_eq!(
        after.supply_index, before.supply_index,
        "zero supplied value: index unchanged"
    );
    assert_eq!(after.borrowed, 0, "debt burned");
    assert_eq!(after.cash, before.cash);

    // (ii) Floor clamp: bad debt wipes the supply value down to a sliver of cash.
    let t2 = fixture();
    let env2 = &t2.env;
    let alice2 = Address::generate(env2);
    let bob2 = Address::generate(env2);
    let key2 = market_key(&t2);
    supply(&t2, &alice2, 0, 1_000 * UNIT);
    let p_b2 = borrow(&t2, &bob2, 0, 980 * UNIT).position.scaled_amount;
    t2.advance_time_no_refresh(YEAR_SECS);
    let pl2 = pool(&t2);
    pl2.update_indexes(&vec![env2, key2.clone()]);
    // Alice takes back all but 0.1 USDC of the 20 USDC cash.
    let p_a2 = i(&shares(1_000 * UNIT, RAY, false));
    let w = withdraw(&t2, &alice2, p_a2, 199_000_000);
    assert_eq!(w.actual_amount, 199_000_000);
    let pre = state(&t2);
    assert_eq!(pre.cash, 1_000_000);
    let bad_ray = ceil_div(&(bi(p_b2) * bi(pre.borrow_index)), &ray());
    let total = mul_ray_half(&bi(pre.supplied), &bi(pre.supply_index));
    assert!(bad_ray < total, "bad debt stays under total supply value");
    let remaining = &total - &bad_ray;
    let rf = &remaining * ray() / &total;
    let raw_index = (bi(pre.supply_index) * &rf) / ray();
    assert!(
        raw_index < BigInt::from(SUPPLY_INDEX_FLOOR_RAW),
        "pre-clamp index below the floor"
    );

    seize_borrow(&t2, p_b2);
    let post = state(&t2);
    assert_eq!(
        post.supply_index, SUPPLY_INDEX_FLOOR_RAW,
        "clamped to RAY/1000"
    );
    assert_eq!(post.borrowed, 0);

    println!(
        "H7 floor: pre_index={} post_index={} residual_shortfall={}",
        pre.supply_index,
        post.supply_index,
        shortfall(&t2)
    );
    // Residual claims at the floor exceed cash: shortfall, supply rejected, recap fills it.
    let shortfall_now = shortfall(&t2);
    assert!(
        shortfall_now > BigInt::from(0),
        "floor leaves residual claims"
    );
    assert_contract_error(
        flat(pool(&t2).try_supply(&vec![
            env2,
            PoolSupplyEntry {
                action: act(&t2, 0, UNIT),
            },
        ])),
        POOL_INSOLVENT,
    );
    let payer = Address::generate(env2);
    let payer_before = pay_in(&t2, &payer, UNIT);
    let rc = pool(&t2).recapitalize(&key2, &payer, &UNIT);
    println!(
        "H7 recap: applied={} refund={}",
        rc.actual_amount,
        UNIT - rc.actual_amount
    );
    assert_eq!(bi(rc.actual_amount), shortfall_now);
    assert_eq!(
        bi(balance(&t2, &payer) - payer_before),
        bi(UNIT) - shortfall_now
    );
    assert_eq!(shortfall(&t2), BigInt::from(0));
}

// ---------------------------------------------------------------------------
// H8: rate-model changes (update_params / create_market)
// ---------------------------------------------------------------------------

#[test]
fn rv_update_params_accrues_under_old_model_before_switch() {
    let t = fixture();
    let env = &t.env;
    let alice = Address::generate(env);
    let bob = Address::generate(env);
    let key = market_key(&t);
    supply(&t, &alice, 0, 1_000 * UNIT);
    borrow(&t, &bob, 0, 500 * UNIT);
    t.advance_time_no_refresh(30 * 86_400);
    let pl = pool(&t);

    // Expected indexes at the switch time, under the OLD model (stored sync data, pre-call).
    let pre = sync(&t);
    let expected = simulate_update_indexes(env, now_ms(&t), &pre);
    let mut fast = sync(&t).params.rate_model_view();
    fast.base_borrow_rate = RAY / 10;
    fast.slope1 = RAY / 2;
    fast.slope2 = RAY * 8 / 10;
    fast.slope3 = RAY * 3 / 2;
    // What the new curve would have accrued over the same window, for contrast.
    let mut new_params = pre.params.clone();
    new_params.base_borrow_rate = fast.base_borrow_rate;
    new_params.slope1 = fast.slope1;
    new_params.slope2 = fast.slope2;
    new_params.slope3 = fast.slope3;
    let under_new = simulate_update_indexes(
        env,
        now_ms(&t),
        &PoolSyncData {
            params: new_params,
            state: pre.state.clone(),
        },
    );
    assert_ne!(
        under_new.borrow_index.raw(),
        expected.borrow_index.raw(),
        "fixture: the new curve must accrue differently"
    );

    pl.update_params(&key, &fast);
    let at_switch = state(&t);
    assert_eq!(
        at_switch.borrow_index,
        expected.borrow_index.raw(),
        "borrow index is the old-model accrual"
    );
    assert_eq!(
        at_switch.supply_index,
        expected.supply_index.raw(),
        "supply index is the old-model accrual"
    );
    assert_eq!(at_switch.last_timestamp, now_ms(&t));

    // After the switch, the new curve governs the next window.
    t.advance_time_no_refresh(30 * 86_400);
    pl.update_indexes(&vec![env, key.clone()]);
    let next = state(&t);
    let switched = sync(&t);
    let expected_next = simulate_update_indexes(
        env,
        now_ms(&t),
        &PoolSyncData {
            params: switched.params.clone(),
            state: PoolStateRaw {
                supplied: at_switch.supplied,
                borrowed: at_switch.borrowed,
                revenue: at_switch.revenue,
                borrow_index: at_switch.borrow_index,
                supply_index: at_switch.supply_index,
                last_timestamp: at_switch.last_timestamp,
                cash: at_switch.cash,
            },
        },
    );
    assert_eq!(next.borrow_index, expected_next.borrow_index.raw());
    assert_eq!(next.supply_index, expected_next.supply_index.raw());
    assert!(next.borrow_index > at_switch.borrow_index);
}

#[test]
fn rv_model_validation_rejects_each_invalid_model_with_its_code() {
    let t = fixture();
    let key = market_key(&t);
    let pl = pool(&t);
    let base_model = sync(&t).params.rate_model_view();
    let base_debug = format!("{:?}", base_model);

    type Edit = fn(&mut InterestRateModel);
    let cases: [(&str, Edit, u32); 10] = [
        (
            "mid_utilization = 0",
            |m: &mut InterestRateModel| m.mid_utilization = 0,
            INVALID_UTIL_RANGE,
        ),
        (
            "optimal = mid",
            |m: &mut InterestRateModel| m.optimal_utilization = m.mid_utilization,
            INVALID_UTIL_RANGE,
        ),
        (
            "optimal = RAY",
            |m: &mut InterestRateModel| {
                m.optimal_utilization = RAY;
                m.max_utilization = RAY;
            },
            OPT_UTIL_TOO_HIGH,
        ),
        (
            "max_utilization < optimal",
            |m: &mut InterestRateModel| m.max_utilization = m.optimal_utilization - 1,
            INVALID_UTIL_RANGE,
        ),
        (
            "reserve_factor = BPS",
            |m: &mut InterestRateModel| m.reserve_factor = 10_000,
            INVALID_RESERVE_FACTOR,
        ),
        (
            "flashloan_fee = 501",
            |m: &mut InterestRateModel| m.flashloan_fee = 501,
            INVALID_BORROW_PARAMS,
        ),
        (
            "max_borrow_rate = 2 RAY + 1",
            |m: &mut InterestRateModel| m.max_borrow_rate = 2 * RAY + 1,
            MAX_BORROW_RATE_TOO_HIGH,
        ),
        (
            "base_borrow_rate < 0",
            |m: &mut InterestRateModel| m.base_borrow_rate = -1,
            BASE_RATE_NEGATIVE,
        ),
        (
            "slope2 < slope1",
            |m: &mut InterestRateModel| m.slope2 = m.slope1 - 1,
            SLOPE_NON_MONOTONIC,
        ),
        (
            "max_borrow_rate = base",
            |m: &mut InterestRateModel| {
                m.base_borrow_rate = RAY / 100;
                m.slope1 = RAY / 100;
                m.slope2 = RAY / 100;
                m.slope3 = RAY / 100;
                m.max_borrow_rate = RAY / 100;
            },
            MAX_RATE_BELOW_BASE,
        ),
    ];
    for (label, edit, code) in cases {
        let mut model = base_model.clone();
        edit(&mut model);
        assert_contract_error(flat(pl.try_update_params(&key, &model)), code);
        assert_eq!(
            format!("{:?}", sync(&t).params.rate_model_view()),
            base_debug,
            "{label}: stored model changed"
        );
    }

    // create_market: a 2-decimal market cannot be flash-loanable; decimals above 18 are rejected.
    let mut params: MarketParamsRaw = sync(&t).params;
    params.asset_decimals = 2;
    params.is_flashloanable = true;
    assert_contract_error(
        flat(pl.try_create_market(&2u32, &params)),
        INVALID_BORROW_PARAMS,
    );
    let mut params19: MarketParamsRaw = sync(&t).params;
    params19.asset_decimals = 19;
    params19.is_flashloanable = false;
    assert_contract_error(
        flat(pl.try_create_market(&3u32, &params19)),
        ASSET_DECIMALS_TOO_HIGH,
    );
}

// ---------------------------------------------------------------------------
// H9: index ceiling and horizon (INV-IDX-01)
// ---------------------------------------------------------------------------

#[test]
fn rv_borrow_index_hits_ceiling_after_100y_and_accrual_is_inert() {
    let t = fixture();
    let env = &t.env;
    let alice = Address::generate(env);
    let bob = Address::generate(env);
    let key = market_key(&t);
    set_model(&t, cap_rate_curve);
    let p_a = supply(&t, &alice, 0, 10 * UNIT).position.scaled_amount;
    // 9.8 of 10 tokens: cash lands exactly on the 2% reserve.
    let p_b = borrow(&t, &bob, 0, 98_000_000).position.scaled_amount;
    let pl = pool(&t);

    t.advance_time_no_refresh(100 * YEAR_SECS);
    pl.update_indexes(&vec![env, key.clone()]);
    let capped = state(&t);
    assert_eq!(
        capped.borrow_index, MAX_BORROW_INDEX_RAY,
        "borrow index sits at 1e36"
    );
    println!(
        "100y single call: borrow_index={} supply_index={} supplied={} borrowed={} revenue={} cash={}",
        capped.borrow_index, capped.supply_index, capped.supplied, capped.borrowed, capped.revenue, capped.cash
    );
    let at_cap = book(&t);

    // A further year adds nothing.
    t.advance_time_no_refresh(YEAR_SECS);
    pl.update_indexes(&vec![env, key.clone()]);
    assert_eq!(book(&t), at_cap, "accrual past the ceiling is inert");

    // Repay at the ceiling: the exact ceiled debt, no refund.
    let owed = i(&value_ceil(p_b, MAX_BORROW_INDEX_RAY));
    let wallet = pay_in(&t, &bob, owed);
    let r = pl
        .repay(&bob, &vec![env, act(&t, p_b, owed)])
        .get(0)
        .unwrap();
    assert_eq!(r.actual_amount, owed);
    assert_eq!(r.position.scaled_amount, 0);
    assert_eq!(balance(&t, &bob), wallet);
    assert_eq!(state(&t).borrowed, 0);
    assert_eq!(state(&t).cash, at_cap.cash + owed);

    // Withdraw at the ceiling: the full claim, floor-valued.
    let s = state(&t);
    let claim = value_floor(p_a, s.supply_index);
    let alice_before = balance(&t, &alice);
    let w = withdraw(&t, &alice, p_a, i128::MAX);
    assert_eq!(bi(w.actual_amount), claim);
    assert_eq!(w.position.scaled_amount, 0);
    assert_eq!(bi(balance(&t, &alice) - alice_before), claim);
}

#[test]
fn rv_value_overflow_horizon_for_billion_token_book() {
    // Realistic-scale check: 1e9 whole tokens supplied, 98% borrowed. Debt value in RAY is
    // shares * index, so i128 overflows once the borrow index passes ~173x.
    let build = || {
        let t = fixture();
        let env = &t.env;
        let alice = Address::generate(env);
        let bob = Address::generate(env);
        supply(&t, &alice, 0, 1_000_000_000 * UNIT);
        borrow(&t, &bob, 0, 980_000_000 * UNIT);
        t
    };

    // 20% APR for 5 years in one call.
    let t = build();
    set_model(&t, flat_rate_curve(RAY / 5));
    t.advance_time_no_refresh(5 * YEAR_SECS);
    pool(&t).update_indexes(&vec![&t.env, market_key(&t)]);
    let idx = state(&t).borrow_index as f64 / RAY as f64;
    let e5 = (1.0f64).exp();
    println!("20% APR, 5y: borrow index x{idx:.9} (e = {e5:.9})");
    assert!((idx - e5).abs() < 1e-6, "5y at 20% APR must track e^1");

    // 200% APR in 30-day steps: record the first failing step, if any.
    let t = build();
    set_model(&t, flat_rate_curve(2 * RAY));
    let pl = pool(&t);
    let key = market_key(&t);
    let mut last_ok_days = 0u64;
    let mut failure: Option<(u64, SdkError, Book)> = None;
    for step in 1..=40u64 {
        t.advance_time_no_refresh(MONTH_SECS);
        let before = book(&t);
        match flat(pl.try_update_indexes(&vec![&t.env, key.clone()])) {
            Ok(()) => last_ok_days = step * 30,
            Err(e) => {
                assert_eq!(
                    book(&t),
                    before,
                    "a failed accrual leaves the books unchanged"
                );
                failure = Some((step * 30, e, before));
                break;
            }
        }
    }
    let idx2 = state(&t).borrow_index as f64 / RAY as f64;
    match failure {
        Some((day, err, _)) => {
            println!(
                "200% APR, 1e9 book: last accrual OK at day {last_ok_days} (index x{idx2:.3}); day {day} fails with {err:?}"
            );
            assert!(last_ok_days >= 730, "2 years at 200% APR must still accrue");
            assert_eq!(err, SdkError::from_contract_error(MATH_OVERFLOW));
            // Once accrual overflows, every entrypoint that accrues first fails as well,
            // including the rate change that would stop the growth.
            let stuck = sync(&t).params.rate_model_view();
            assert_contract_error(flat(pl.try_update_params(&key, &stuck)), MATH_OVERFLOW);
            let who = Address::generate(&t.env);
            let alice_pos = i(&shares(1_000_000_000 * UNIT, RAY, false));
            let bob_pos = i(&shares(980_000_000 * UNIT, RAY, true));
            assert_contract_error(
                flat(pl.try_withdraw(
                    &who,
                    &false,
                    &vec![
                        &t.env,
                        PoolWithdrawEntry {
                            action: act(&t, alice_pos, UNIT),
                            protocol_fee: 0,
                        },
                    ],
                )),
                MATH_OVERFLOW,
            );
            assert_contract_error(
                flat(pl.try_repay(&who, &vec![&t.env, act(&t, bob_pos, UNIT)])),
                MATH_OVERFLOW,
            );
            println!("stuck market: update_params, withdraw and repay all fail with MathOverflow");
        }
        None => panic!("no failure within 40 months, last OK day {last_ok_days}"),
    }
}

// ---------------------------------------------------------------------------
// H10: revenue claim (INV-ACCT-06)
// ---------------------------------------------------------------------------

#[test]
fn rv_revenue_claim_cash_limited_burns_ceil_then_zero_claimable() {
    let t = fixture();
    let env = &t.env;
    let alice = Address::generate(env);
    let bob = Address::generate(env);
    let key = market_key(&t);
    supply(&t, &alice, 0, 1_000 * UNIT);
    // 98% borrowed: cash is 20 tokens, far below the accrued revenue.
    borrow(&t, &bob, 0, 980 * UNIT);
    t.advance_time_no_refresh(YEAR_SECS);
    let pl = pool(&t);
    pl.update_indexes(&vec![env, key.clone()]);
    let s = state(&t);
    let treasury = value_floor(s.revenue, s.supply_index);
    assert!(treasury > bi(s.cash), "fixture: revenue must exceed cash");
    let payout = bi(s.cash);
    let burned = ceil_div(&(bi(s.revenue) * &payout), &treasury);
    let owner = t.controller.clone();
    let owner_before = balance(&t, &owner);

    let claimed = pl.claim_revenue(&key).actual_amount;
    assert_eq!(bi(claimed), payout, "cash-limited payout");
    assert_eq!(bi(balance(&t, &owner) - owner_before), payout);
    let after = state(&t);
    assert_eq!(
        bi(s.revenue - after.revenue),
        burned,
        "ceil of revenue * payout / treasury"
    );
    assert_eq!(
        bi(s.supplied - after.supplied),
        burned,
        "revenue shares leave total supply"
    );
    assert_eq!(after.cash, 0);
    assert_eq!(balance(&t, &pool_addr(&t)), 0);

    // Nothing left to claim: returns zero, moves nothing, burns nothing.
    let before = book(&t);
    let owner_mid = balance(&t, &owner);
    assert_eq!(pl.claim_revenue(&key).actual_amount, 0);
    assert_eq!(balance(&t, &owner), owner_mid);
    assert_eq!(book(&t), before);

    // A full claim on a separate book: payout equals floor(revenue), every revenue share burns.
    let t2 = fixture();
    let env2 = &t2.env;
    let alice2 = Address::generate(env2);
    let bob2 = Address::generate(env2);
    let key2 = market_key(&t2);
    supply(&t2, &alice2, 0, 1_000 * UNIT);
    borrow(&t2, &bob2, 0, 100 * UNIT);
    t2.advance_time_no_refresh(YEAR_SECS);
    pool(&t2).update_indexes(&vec![env2, key2.clone()]);
    let s2 = state(&t2);
    let treasury2 = value_floor(s2.revenue, s2.supply_index);
    assert!(treasury2 > BigInt::from(0) && treasury2 < bi(s2.cash));
    let owner2 = t2.controller.clone();
    let owner2_before = balance(&t2, &owner2);
    assert_eq!(bi(pool(&t2).claim_revenue(&key2).actual_amount), treasury2);
    assert_eq!(bi(balance(&t2, &owner2) - owner2_before), treasury2);
    let s2_after = state(&t2);
    assert_eq!(s2_after.revenue, 0);
    assert_eq!(bi(s2.supplied - s2_after.supplied), bi(s2.revenue));
}
