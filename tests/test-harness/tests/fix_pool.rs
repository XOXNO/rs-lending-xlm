//! Pool fixes on the PR #196 branch: regression tests for the revenue claim
//! policy (F8 and the CR-2 siblings), negative scaled positions (F5), the
//! market value ceiling on a capped market's own legs (CR-21) and fresh
//! markets after the share-backed seeding (CR-24). Each test asserts the safe
//! outcome; the ones marked "failed before the fix" reproduced the gap on the
//! PR head (`bbe8459`). Run with `cargo test -p test-harness --test fix_pool`
//! after `make build-one CRATE=pool`.
//!
//! - pool-3b: a market at the value ceiling, its own legs: liquidation that
//!   seizes collateral in it, cleanup of a debt leg in it.
//! - pool-7 (failed before the fix): revenue-only supply, a dust borrow, a
//!   claim that would burn the last share (CR-2 sibling). The claim now pays
//!   only the cash above the liquidation buffer and the batch succeeds.
//! - pool-8 (failed before the fix): the claim batch on a market past its
//!   utilization ceiling (CR-2 / F8 sibling). The market pays 0, the batch
//!   succeeds.
//! - pool-9b: no false refusals on a market seeded by one real supplier or by
//!   revenue shares only; share-less cash is refused.
//! - pool-10 (failed before the fix): negative scaled positions (F5).
//! - pool-11 (failed before the fix): a revenue claim on uncollectible
//!   interest (F8).
//! - claim policy: a zero-payout claim skips both guards; while debt is
//!   outstanding a claim draws only the cash above the liquidation reserve
//!   and pays nothing if that payout would raise utilization past
//!   `min(max_utilization, 95%)` (`MAX_CLAIM_UTILIZATION_RAY`).

use common::constants::{MAX_MARKET_VALUE_RAY, RAY};
use common::errors::{CollateralError, GenericError};
use common::math::fp::Ray;
use common::types::{
    HubAssetKey, PoolAction, PoolBorrowEntry, PoolKey, PoolStateRaw, PoolSupplyEntry,
    PoolWithdrawEntry, ScaledPositionRaw,
};
use common::validation::max_cap_for_decimals;
use controller::types::PositionMode;
use soroban_sdk::testutils::{Address as _, ContractEvents, Events};
use soroban_sdk::xdr::{ContractEventBody, ScVal};
use soroban_sdk::{token, vec, Address, Bytes, Env, Error as SdkError, InvokeError};
use test_harness::{
    apply_flash_fee, assert_contract_error, build_aggregator_swap, days, errors, eth_preset,
    hub_asset, map_try_ok_unit, usd, usdc_preset, wbtc_preset, LendingTest, MarketParamsPreset,
    MarketPreset, ALICE, BOB, CAROL, DEFAULT_ASSET_CONFIG, HARNESS_SPOKE,
};

const UNIT7: i128 = 10_000_000;
const YEAR_SECS: u64 = 31_556_926;
const BILLION: i128 = 1_000_000_000;

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn no_seed(mut preset: MarketPreset) -> MarketPreset {
    preset.initial_liquidity = 0.0;
    preset
}

fn key(t: &LendingTest, asset: &str) -> HubAssetKey {
    hub_asset(t.resolve_asset(asset))
}

fn state(t: &LendingTest, asset: &str) -> PoolStateRaw {
    t.pool_client(asset).get_sync_data(&key(t, asset)).state
}

/// Converts the flat `try_*` result into one `Result`.
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

fn count_topic(events: &ContractEvents, first: &str, second: &str) -> usize {
    events
        .events()
        .iter()
        .filter(|event| {
            let ContractEventBody::V0(body) = &event.body;
            matches!(
                (body.topics.first(), body.topics.get(1)),
                (Some(ScVal::Symbol(a)), Some(ScVal::Symbol(b)))
                    if a.0.to_string() == first && b.0.to_string() == second
            )
        })
        .count()
}

fn lift_caps(t: &LendingTest, asset: &str, decimals: u32) {
    let cap = max_cap_for_decimals(decimals);
    let cfg = t.get_asset_config(asset);
    t.edit_asset_in_spoke_caps(
        asset,
        HARNESS_SPOKE,
        true,
        true,
        cfg.loan_to_value,
        cfg.liquidation_threshold,
        cfg.liquidation_bonus,
        cap,
        cap,
    );
}

/// `lift_caps` with a non-zero liquidation fee: the harness caps helper
/// lists the asset with `liquidation_fees: 0`, which would make the seize
/// withdraw carry no protocol fee.
fn lift_caps_with_fees(t: &LendingTest, asset: &str, decimals: u32, liquidation_fees: u32) {
    let cap = max_cap_for_decimals(decimals);
    let cfg = t.get_asset_config(asset);
    t.ctrl_client()
        .edit_asset_in_spoke(&controller::types::SpokeAssetArgs {
            hub_id: test_harness::HARNESS_HUB,
            asset: t.resolve_asset(asset),
            spoke_id: HARNESS_SPOKE,
            can_collateral: true,
            can_borrow: true,
            paused: false,
            frozen: false,
            no_seize: false,
            ltv: cfg.loan_to_value,
            threshold: cfg.liquidation_threshold,
            bonus: cfg.liquidation_bonus,
            liquidation_fees,
            supply_cap: cap,
            borrow_cap: cap,
        });
}

// ---------------------------------------------------------------------------
// pool-3b: the capped market's own legs
// ---------------------------------------------------------------------------

/// The CR-21 harness fixture's steep XLM stress curve.
fn xlm_curve() -> MarketParamsPreset {
    MarketParamsPreset {
        max_borrow_rate: RAY * 175 / 100,
        base_borrow_rate: RAY / 100,
        slope1: RAY * 4 / 100,
        slope2: RAY * 10 / 100,
        slope3: RAY * 150 / 100,
        mid_utilization: RAY * 50 / 100,
        optimal_utilization: RAY * 75 / 100,
        max_utilization: RAY,
        reserve_factor: 2000,
    }
}

/// The CR-21 harness fixture's steep USDC stress curve.
fn usdc_curve() -> MarketParamsPreset {
    MarketParamsPreset {
        max_borrow_rate: RAY * 125 / 100,
        base_borrow_rate: RAY * 5 / 1000,
        slope1: RAY * 3 / 100,
        slope2: RAY * 95 / 1000,
        slope3: RAY,
        mid_utilization: RAY * 60 / 100,
        optimal_utilization: RAY * 85 / 100,
        max_utilization: RAY,
        reserve_factor: 1500,
    }
}

fn big(name: &'static str, decimals: u32, params: MarketParamsPreset) -> MarketPreset {
    MarketPreset {
        name,
        decimals,
        price_wad: usd(1),
        initial_liquidity: 0.0,
        config: DEFAULT_ASSET_CONFIG,
        params,
    }
}

fn col() -> MarketPreset {
    MarketPreset {
        name: "COL",
        decimals: 7,
        price_wad: usd(1),
        initial_liquidity: 0.0,
        config: DEFAULT_ASSET_CONFIG,
        params: usdc_curve(),
    }
}

fn priced(name: &'static str, price_wad: i128, params: MarketParamsPreset) -> MarketPreset {
    MarketPreset {
        price_wad,
        ..big(name, 7, params)
    }
}

/// pool-3b. The fixture `a_market_at_the_value_ceiling_does_not_block_
/// other_markets_liquidation_and_cleanup`, extended to the capped market's
/// own legs: gina holds 1,000 BIG18 of collateral against a USDC debt and is
/// liquidated in `SeizeMode::Transfer` (the seize withdraws BIG18 with a
/// protocol fee, accruing the capped market first); hank's only debt is a
/// BIG18 leg and is cleaned up as bad debt.
#[test]
fn pool3b_capped_market_seizure_and_debt_cleanup_stay_live() {
    let mut t = LendingTest::new()
        .with_market(big("BIG18", 18, xlm_curve()))
        .with_market(col())
        .with_market(priced("USDC", usd(1), usdc_curve()))
        .with_market(priced("ETH", usd(2_000), usdc_curve()))
        .with_max_utilization_disabled_all_markets()
        .build();
    lift_caps_with_fees(&t, "BIG18", 18, 1_200);
    lift_caps(&t, "ETH", 7);
    let principal = BILLION * 10i128.pow(18);
    t.supply_raw(BOB, "BIG18", principal);
    t.supply("lp", "USDC", 1_000_000.0);

    // Gina: collateral in the market that will sit at the ceiling.
    t.supply("gina", "BIG18", 1_000.0);
    t.borrow("gina", "USDC", 600.0);
    // Hank: his only debt is a BIG18 leg.
    t.supply("hank", "COL", 10.0);
    t.borrow("hank", "BIG18", 5.0);

    // The whale borrows 98 percent of BIG18 against ETH.
    t.supply_raw(ALICE, "ETH", BILLION * 10_000_000);
    t.borrow_raw(ALICE, "BIG18", principal / 100 * 98 - 10i128.pow(18));

    for _ in 0..6 {
        t.advance_time(YEAR_SECS);
        t.update_indexes_for(&["COL", "USDC", "ETH"]);
    }

    let big = key(&t, "BIG18");
    let indexes = t
        .pool_client("COL")
        .get_bulk_indexes(&vec![&t.env, key(&t, "COL"), big.clone()]);
    let big_index = indexes.get(1).unwrap();
    let s = state(&t, "BIG18");
    let debt_value = Ray::from(s.borrowed)
        .mul_floor(&t.env, Ray::from(big_index.borrow_index))
        .raw();
    let supply_value = Ray::from(s.supplied)
        .mul_floor(&t.env, Ray::from(big_index.supply_index))
        .raw();
    println!(
        "pool-3b: projected BIG18 debt = MAX-{}, supply = MAX-{}",
        MAX_MARKET_VALUE_RAY - debt_value,
        MAX_MARKET_VALUE_RAY - supply_value
    );
    // The stored `supplied` omits the revenue shares the projection mints,
    // so only the debt total is comparable here (as in that fixture).
    assert!(debt_value <= MAX_MARKET_VALUE_RAY && supply_value <= MAX_MARKET_VALUE_RAY);
    assert!(
        debt_value > MAX_MARKET_VALUE_RAY / 100 * 99,
        "BIG18's projected debt sits at the value ceiling"
    );

    // Liquidation that seizes collateral in the capped market. BIG18 has not
    // been synced for six years, so the seize withdraw is its first accrual.
    let stored = state(&t, "BIG18");
    let whole = t.supply_balance_raw("gina", "BIG18") / 10i128.pow(18) * big_index.supply_index
        / stored.supply_index;
    t.set_price("BIG18", usd(700) / whole);
    assert!(t.can_be_liquidated("gina"), "gina is liquidatable");
    let gina_debt = t.borrow_balance_raw("gina", "USDC");
    let revenue_before = stored.revenue;
    let gina = t.resolve_account_id("gina");
    let estimate = t.ctrl_client().get_liquidation_estimate(
        &gina,
        &vec![&t.env, (key(&t, "USDC"), 100 * UNIT7)],
        &common::types::SeizeMode::Transfer,
    );
    println!(
        "pool-3b: estimate seized={:?} protocol_fees={:?} bonus={}",
        estimate.seized_collaterals.get(0).map(|p| p.amount),
        estimate.protocol_fees.get(0).map(|p| p.amount),
        estimate.bonus_rate_bps
    );
    let liquidated = t.try_liquidate("liq", "gina", "USDC", 100.0);
    let ceiling_events = count_topic(&t.env.events().all(), "market", "value_ceiling");
    println!(
        "pool-3b: gina projected BIG18 collateral {whole} tokens, USDC debt {gina_debt}; \
         liquidation -> {liquidated:?}; value_ceiling events in the liquidation = {ceiling_events}"
    );
    assert!(
        liquidated.is_ok(),
        "liquidation seizing BIG18: {liquidated:?}"
    );
    assert!(t.borrow_balance_raw("gina", "USDC") < gina_debt);
    let gina_after = t.supply_balance_raw("gina", "BIG18") / 10i128.pow(18);
    assert!(gina_after < whole, "collateral was seized");
    let after = state(&t, "BIG18");
    println!(
        "pool-3b: BIG18 revenue shares before {revenue_before}, after {} (six years of capped accrual plus the fee: {}); \
         gina BIG18 after {}",
        after.revenue,
        after.revenue - revenue_before,
        t.supply_balance_raw("gina", "BIG18")
    );
    assert!(
        estimate.protocol_fees.get(0).unwrap().amount > 0,
        "the seize carries a protocol fee"
    );
    assert!(after.revenue > revenue_before, "the seize booked revenue");

    // Cleanup of an account whose only debt is a BIG18 leg.
    t.set_price("COL", usd(1) / 10);
    let hank = t.resolve_account_id("hank");
    let cleaned = t.try_clean_bad_debt_by_id(hank);
    println!("pool-3b: hank cleanup (BIG18-only debt) -> {cleaned:?}");
    assert!(cleaned.is_ok(), "cleanup of a BIG18 debt leg: {cleaned:?}");
    assert!(!t.ctrl_client().account_exists(&hank));
    let end = state(&t, "BIG18");
    let supply_end = Ray::from(end.supplied)
        .mul_floor(&t.env, Ray::from(end.supply_index))
        .raw();
    let debt_end = Ray::from(end.borrowed)
        .mul_floor(&t.env, Ray::from(end.borrow_index))
        .raw();
    assert!(supply_end <= MAX_MARKET_VALUE_RAY && debt_end <= MAX_MARKET_VALUE_RAY);
    let supply_after = Ray::from(after.supplied)
        .mul_floor(&t.env, Ray::from(after.supply_index))
        .raw();
    let debt_after = Ray::from(after.borrowed)
        .mul_floor(&t.env, Ray::from(after.borrow_index))
        .raw();
    assert!(supply_after <= MAX_MARKET_VALUE_RAY && debt_after <= MAX_MARKET_VALUE_RAY);
}

// ---------------------------------------------------------------------------
// pool-7 / pool-9b: revenue-only supply
// ---------------------------------------------------------------------------

/// The fixture of `test_borrow_into_zero_supply_market_rejected` up to
/// `withdraw_all`, without the claim: USDC supply is revenue shares only.
fn revenue_only_usdc() -> LendingTest {
    let mut t = LendingTest::new()
        .with_market(no_seed(usdc_preset()))
        .with_market(no_seed(wbtc_preset()))
        .build();
    let accumulator = t
        .env
        .register(test_harness::mock_reflector::MockReflector, ());
    t.ctrl_client().set_accumulator(&accumulator);

    t.supply(ALICE, "USDC", 1_000.0);
    t.supply(BOB, "WBTC", 1.0);
    t.borrow(BOB, "USDC", 500.0);
    t.advance_time(days(365));
    t.repay(BOB, "USDC", 600.0);
    t.withdraw_all(ALICE, "USDC");
    t
}

/// pool-7 (failed before the fix with #123 for the single claim and the
/// batch). With only revenue shares left, a borrow of exactly the dust above
/// the treasury passes the mint-side `supplied == 0` guard, because revenue
/// shares count as supply. The claim used to pay `min(cash, treasury) =
/// treasury`, burn every revenue share and revert on the supply-for-debt
/// check, failing the controller batch. It now draws only the cash above the
/// liquidation buffer, leaves supply shares under the debt, and the batch
/// succeeds.
#[test]
fn pool7_revenue_only_supply_dust_borrow_does_not_fail_the_claim_batch() {
    let mut t = revenue_only_usdc();
    let usdc = key(&t, "USDC");
    let s = state(&t, "USDC");
    let rev = t.pool_client("USDC").get_revenue(&usdc);
    let dust = s.cash - rev;
    println!(
        "pool-7: supplied={} revenue={} borrowed={} cash={} revenue value={rev} dust={dust}",
        s.supplied, s.revenue, s.borrowed, s.cash
    );
    assert_eq!(s.supplied, s.revenue, "only revenue shares remain");
    assert!(s.revenue > 0);
    assert_eq!(s.borrowed, 0);
    assert!(dust > 0, "rounding leaves dust above the treasury");

    t.supply(CAROL, "WBTC", 0.1);
    let carol = t.get_or_create_user(CAROL);
    let carol_account = t.resolve_account_id(CAROL);
    let borrows = vec![&t.env, (usdc.clone(), dust)];
    let borrowed =
        map_try_ok_unit(
            t.ctrl_client()
                .try_borrow(&carol, &carol_account, &borrows, &None),
        );
    println!("pool-7: dust borrow of {dust} raw -> {borrowed:?}");
    assert!(
        borrowed.is_ok(),
        "the dust borrow passes the mint-side guards: {borrowed:?}"
    );

    let single = t.try_claim_revenue("USDC");
    let batch = flat(
        t.ctrl_client()
            .try_claim_revenue(&t.admin, &vec![&t.env, usdc.clone(), key(&t, "WBTC")]),
    );
    let after = state(&t, "USDC");
    println!(
        "pool-7: after borrow: supplied={} borrowed={} cash={}; single claim -> {single:?}; \
         two-asset batch -> {batch:?}",
        after.supplied, after.borrowed, after.cash
    );
    assert!(single.is_ok(), "single USDC claim: {single:?}");
    assert!(batch.is_ok(), "two-asset claim batch: {batch:?}");
    let end = state(&t, "USDC");
    assert!(
        end.supplied > 0 && end.borrowed > 0,
        "supply shares stay under the dust debt"
    );
    assert!(
        end.cash >= t.liquidation_buffer_raw("USDC"),
        "the claims leave the liquidation buffer in cash"
    );
}

/// pool-9b. No false refusals on fresh markets: a market seeded only by one
/// real supplier admits a borrow, a flash loan of the whole cash, a multiply
/// opening debt and a same-asset repay-with-collateral; a revenue-only market
/// admits a one-unit borrow; a market holding only share-less cash refuses a
/// borrow with PoolInsolvent (#123).
#[test]
fn pool9b_fresh_markets_admit_every_production_path() {
    let mut t = LendingTest::new()
        .standard_two_asset()
        .without_initial_liquidity()
        .with_min_borrow_collateral_disabled()
        .build();
    t.supply(ALICE, "USDC", 10.0);
    t.supply(BOB, "ETH", 1.0);
    let borrowed = t.try_borrow(BOB, "USDC", 5.0);
    assert!(
        borrowed.is_ok(),
        "borrow on a one-supplier market: {borrowed:?}"
    );

    let receiver = t.deploy_flash_loan_receiver();
    let cash = state(&t, "USDC").cash;
    let flashed = t.try_flash_loan(CAROL, "USDC", cash as f64 / UNIT7 as f64, &receiver);
    assert!(flashed.is_ok(), "flash loan of the whole cash: {flashed:?}");

    t.fund_router("ETH", 0.002);
    let steps = build_aggregator_swap(&t, "USDC", "ETH", apply_flash_fee(20_000_000), 20_000);
    let multiplied = t.try_multiply("dave", "ETH", 2.0, "USDC", PositionMode::Multiply, &steps);
    assert!(
        multiplied.is_ok(),
        "multiply opening USDC debt: {multiplied:?}"
    );

    t.supply(ALICE, "ETH", 1.0);
    t.borrow(ALICE, "USDC", 1.0);
    let settled =
        t.try_repay_debt_with_collateral(ALICE, "USDC", 0.5, "USDC", &Bytes::new(&t.env), false);
    assert!(
        settled.is_ok(),
        "same-asset repay with collateral: {settled:?}"
    );

    // A revenue-only market admits a one-unit borrow.
    let mut r = revenue_only_usdc();
    r.supply(CAROL, "WBTC", 0.1);
    let carol = r.get_or_create_user(CAROL);
    let carol_account = r.resolve_account_id(CAROL);
    let one_unit = map_try_ok_unit(r.ctrl_client().try_borrow(
        &carol,
        &carol_account,
        &vec![&r.env, (key(&r, "USDC"), 1i128)],
        &None,
    ));
    assert!(
        one_unit.is_ok(),
        "one-unit borrow from revenue-only supply: {one_unit:?}"
    );

    // Share-less cash only: the borrow is refused.
    let mut s = LendingTest::new()
        .standard_two_asset()
        .without_initial_liquidity()
        .with_min_borrow_collateral_disabled()
        .build();
    s.inject_shareless_cash_raw("USDC", 1_000 * UNIT7);
    s.supply(BOB, "ETH", 1.0);
    let refused = s.try_borrow(BOB, "USDC", 5.0);
    assert_contract_error(refused, errors::POOL_INSOLVENT);
}

// ---------------------------------------------------------------------------
// pool-8: the claim batch on a market past its utilization ceiling
// ---------------------------------------------------------------------------

/// pool-8 (failed before the fix with #127 for the whole batch). An
/// uncleaned insolvency pushes ETH past its default 95% ceiling. The
/// two-asset claim batch succeeds: ETH pays 0 and moves nothing (the
/// liquidation reserve on the inflated supply exceeds its cash), and USDC
/// pays its revenue.
#[test]
fn pool8_claim_batch_survives_a_market_past_its_utilization_ceiling() {
    let mut t = LendingTest::new()
        .with_market(no_seed(usdc_preset()))
        .with_market(no_seed(eth_preset()))
        .with_market(no_seed(wbtc_preset()))
        .build();
    let accumulator = Address::generate(&t.env);
    t.set_accumulator(&accumulator);
    t.supply(BOB, "ETH", 100.0);
    t.supply(ALICE, "USDC", 300_000.0);
    t.borrow_raw(ALICE, "ETH", 940_000_000);
    t.supply(CAROL, "WBTC", 1.0);
    t.borrow(CAROL, "USDC", 1_000.0);
    t.set_price("USDC", 10_000_000_000_000);
    t.advance_time(days(3 * 365));
    t.update_indexes_for(&["ETH", "USDC"]);

    let eth = key(&t, "ETH");
    let utilization = t.pool_client("ETH").get_utilisation(&eth);
    let eth_before = state(&t, "ETH");
    println!(
        "pool-8: ETH utilization {:.4}, cash {}, revenue value {}",
        utilization as f64 / RAY as f64,
        eth_before.cash,
        t.pool_client("ETH").get_revenue(&eth)
    );
    assert!(
        utilization > RAY * 95 / 100,
        "ETH sits above its 95% ceiling"
    );

    let batch = vec![&t.env, eth.clone(), key(&t, "USDC")];
    let paid = flat(t.ctrl_client().try_claim_revenue(&t.admin, &batch));
    println!("pool-8: claim batch [ETH, USDC] -> {paid:?}");
    let paid = paid.expect("the claim batch succeeds");
    assert_eq!(paid.get(0).unwrap(), 0, "the ETH claim pays nothing");
    assert!(paid.get(1).unwrap() > 0, "USDC revenue is swept");
    let after = state(&t, "ETH");
    assert_eq!(
        (after.cash, after.revenue),
        (eth_before.cash, eth_before.revenue),
        "the zero ETH claim moves nothing"
    );
}

// ---------------------------------------------------------------------------
// pool-10: negative scaled positions (F5)
// ---------------------------------------------------------------------------

fn act(t: &LendingTest, position: i128, amount: i128) -> PoolAction {
    PoolAction {
        position: ScaledPositionRaw {
            scaled_amount: position,
        },
        amount,
        hub_asset: key(t, "USDC"),
    }
}

/// pool-10 (failed before the fix: `supply(-5)` booked `1e27 - 5` shares,
/// exits failed with #33 or #52). Every pool leg rejects a negative scaled
/// position with AmountMustBePositive (#14) and leaves the books unchanged.
#[test]
fn pool10_negative_scaled_position_is_rejected_with_the_typed_error_on_every_leg() {
    let mut t = LendingTest::new()
        .with_market(no_seed(usdc_preset()))
        .build();
    t.supply_raw(BOB, "USDC", 1_000 * UNIT7);
    let env: &Env = &t.env;
    let who = Address::generate(env);
    let pl = t.pool_client("USDC");
    let code = GenericError::AmountMustBePositive as u32;
    let before = state(&t, "USDC");

    let supply = flat(pl.try_supply(&vec![
        env,
        PoolSupplyEntry {
            action: act(&t, -5, UNIT7),
        },
    ]));
    let supplied_book = state(&t, "USDC");
    let borrow = flat(pl.try_borrow(
        &who,
        &vec![
            env,
            PoolBorrowEntry {
                action: act(&t, -5, UNIT7),
            },
        ],
    ));
    let strategy = flat(pl.try_create_strategy(&who, &act(&t, -5, UNIT7), &false));
    let withdraw = flat(pl.try_withdraw(
        &who,
        &false,
        &vec![
            env,
            PoolWithdrawEntry {
                action: act(&t, -1, UNIT7),
                protocol_fee: 0,
            },
        ],
    ));
    let repay_small = flat(pl.try_repay(&who, &vec![env, act(&t, -1, UNIT7)]));
    let repay_ray = flat(pl.try_repay(&who, &vec![env, act(&t, -RAY, UNIT7)]));
    let after = state(&t, "USDC");
    println!(
        "pool-10: supply -> {:?} (supplied {} -> {}); borrow -> {:?}; strategy -> {:?}; \
         withdraw -> {:?}; repay(-1) -> {:?}; repay(-RAY) -> {:?}",
        supply
            .as_ref()
            .map(|m| m.get(0).unwrap().position.scaled_amount),
        before.supplied,
        supplied_book.supplied,
        borrow
            .as_ref()
            .map(|m| m.get(0).unwrap().position.scaled_amount),
        strategy.as_ref().map(|m| m.position.scaled_amount),
        withdraw
            .as_ref()
            .map(|m| m.get(0).unwrap().position.scaled_amount),
        repay_small
            .as_ref()
            .map(|m| m.get(0).unwrap().position.scaled_amount),
        repay_ray
            .as_ref()
            .map(|m| m.get(0).unwrap().position.scaled_amount),
    );
    for (leg, result) in [
        ("supply", supply.map(|_| ())),
        ("borrow", borrow.map(|_| ())),
        ("create_strategy", strategy.map(|_| ())),
        ("withdraw", withdraw.map(|_| ())),
        ("repay(-1)", repay_small.map(|_| ())),
        ("repay(-RAY)", repay_ray.map(|_| ())),
    ] {
        assert_eq!(
            result,
            Err(SdkError::from_contract_error(code)),
            "{leg} with a negative scaled position"
        );
    }
    assert_eq!(
        (after.supplied, after.borrowed, after.cash),
        (before.supplied, before.borrowed, before.cash),
        "rejected calls leave the books unchanged"
    );
}

// ---------------------------------------------------------------------------
// pool-11: a revenue claim on uncollectible interest (F8)
// ---------------------------------------------------------------------------

struct F8Book {
    claimed: Result<i128, SdkError>,
    revenue_before: i128,
    bob_after_cleanup: i128,
}

fn f8_book(days_aged: u64, claim_first: bool, default_ceiling: bool) -> F8Book {
    let builder = LendingTest::new()
        .with_market(no_seed(usdc_preset()))
        .with_market(no_seed(eth_preset()));
    let mut t = if default_ceiling {
        builder.build()
    } else {
        builder.with_max_utilization_disabled_all_markets().build()
    };
    let accumulator = Address::generate(&t.env);
    t.set_accumulator(&accumulator);
    t.supply(BOB, "ETH", 100.0);
    t.supply(ALICE, "USDC", 300_000.0);
    t.borrow_raw(ALICE, "ETH", 940_000_000);
    t.set_price("USDC", 10_000_000_000_000);
    t.advance_time(days(days_aged));
    t.update_indexes_for(&["ETH"]);
    let eth = key(&t, "ETH");
    let revenue_before = t.pool_client("ETH").get_revenue(&eth);
    let claimed = if claim_first {
        t.try_claim_revenue("ETH")
    } else {
        Ok(0)
    };
    let alice = t.resolve_account_id(ALICE);
    t.clean_bad_debt_by_id(alice);
    F8Book {
        claimed,
        revenue_before,
        bob_after_cleanup: t.supply_balance_raw(BOB, "ETH"),
    }
}

/// pool-11 (failed before the fix: the ceiling-disabled claim paid 9,737,069
/// raw and BOB's post-cleanup claim fell to 50,262,931 from 59,467,615; the
/// default-ceiling claim reverted #127). The ceiling-disabled claim pays 0,
/// BOB's post-cleanup claim is the same with and without it, and the
/// default-ceiling claim returns Ok(0) without reverting. Residual, by
/// design: claims small enough to fit under the 95% line still pay revenue
/// accrued on uncollectible interest; prompt cleanup is the remedy.
#[test]
fn pool11_revenue_claim_does_not_pay_out_uncollectible_interest() {
    let with_claim = f8_book(30, true, false);
    let without_claim = f8_book(30, false, false);
    let under_ceiling = f8_book(30, true, true);
    println!(
        "pool-11 (30 days): revenue={} claimed(ceiling off)={:?} claimed(default ceiling)={:?} \
         BOB after cleanup: with claim={}, without={}",
        with_claim.revenue_before,
        with_claim.claimed,
        under_ceiling.claimed,
        with_claim.bob_after_cleanup,
        without_claim.bob_after_cleanup
    );
    assert!(
        with_claim.revenue_before > 0,
        "revenue accrued on the bad debt"
    );
    assert_eq!(
        with_claim.claimed,
        Ok(0),
        "the claim pays nothing on uncollectible interest"
    );
    assert_eq!(
        with_claim.bob_after_cleanup, without_claim.bob_after_cleanup,
        "the claim moves no write-down onto the suppliers"
    );
    assert_eq!(
        under_ceiling.claimed,
        Ok(0),
        "under the default ceiling the claim pays zero without reverting"
    );
}

/// The error codes the checks name.
#[test]
fn error_codes_are_the_ones_the_checks_name() {
    assert_eq!(GenericError::MathOverflow as u32, 33);
    assert_eq!(CollateralError::PoolInsolvent as u32, 123);
    assert_eq!(CollateralError::UtilizationAboveMax as u32, 127);
    assert_eq!(GenericError::AmountMustBePositive as u32, 14);
}

// ---------------------------------------------------------------------------
// Claim policy: zero-payout claims and the liquidation reserve
// ---------------------------------------------------------------------------

fn inject_usdc_state(t: &LendingTest, edit: impl FnOnce(&mut PoolStateRaw)) {
    let env: &Env = &t.env;
    let usdc = key(t, "USDC");
    let pool = t.resolve_market("USDC").pool.clone();
    let mut injected = state(t, "USDC");
    edit(&mut injected);
    env.as_contract(&pool, || {
        env.storage()
            .persistent()
            .set(&PoolKey::State(usdc.clone()), &injected);
    });
}

/// CR-2 sibling (failed before the fix with #123). A market whose supply
/// shares all left while two raw units of rounding dust were borrowed: the
/// claim returns zero instead of reverting, and a claim never pays out the
/// last share under debt.
#[test]
fn claim_with_nothing_to_pay_does_not_fail_on_orphan_dust_debt() {
    let t = LendingTest::new()
        .with_market(no_seed(usdc_preset()))
        .build();
    let scale = RAY / UNIT7;
    inject_usdc_state(&t, |s| {
        s.supplied = 0;
        s.revenue = 0;
        s.borrowed = 2 * scale;
        s.cash = 0;
    });
    assert_eq!(t.claim_revenue("USDC"), 0, "nothing to pay, no revert");

    // Every remaining share is revenue and one raw unit of debt is open, with
    // a sub-unit supply remainder. The whole floored revenue claim exceeds the
    // claim utilization room (two units short of the floored supply value),
    // so the claim pays nothing and the shares stay under the debt.
    inject_usdc_state(&t, |s| {
        s.supplied = 5 * RAY + scale - 1;
        s.revenue = 5 * RAY + scale - 1;
        s.borrowed = 1;
        s.cash = 100 * UNIT7;
    });
    assert_eq!(t.claim_revenue("USDC"), 0, "nothing payable, no revert");
    assert_eq!(
        state(&t, "USDC").supplied,
        5 * RAY + scale - 1,
        "the shares stay"
    );
}

/// F8 (failed before the fix: the claim took all 300 USDC of cash). 10,000
/// USDC supplied, 9,700 borrowed, the utilization ceiling disabled and a year
/// of accrual: the treasury outgrows the cash. While debt is outstanding a
/// claim draws only the cash above the 2% liquidation reserve, and pays
/// nothing if that payout would raise utilization past 95%
/// (`MAX_CLAIM_UTILIZATION_RAY`), whatever the market's ceiling. Here the
/// reserve exceeds the cash, so the claim pays nothing; after a 3,000 USDC
/// repayment the cash above the reserve would cross the 95% line, so it still
/// pays nothing; after 2,000 more the whole revenue fits and is paid.
#[test]
fn revenue_claim_keeps_the_liquidation_reserve_while_debt_is_outstanding() {
    let mut t = LendingTest::new()
        .with_market(no_seed(usdc_preset()))
        .with_market(no_seed(eth_preset()))
        .with_max_utilization_disabled_all_markets()
        .build();
    let accumulator = Address::generate(&t.env);
    t.set_accumulator(&accumulator);
    t.supply(BOB, "USDC", 10_000.0);
    t.supply(ALICE, "ETH", 20.0);
    t.borrow(ALICE, "USDC", 9_700.0);
    t.advance_time(days(365));
    t.update_indexes_for(&["USDC"]);

    let aged = state(&t, "USDC");
    let reserve = t.liquidation_buffer_raw("USDC");
    assert!(
        reserve > aged.cash,
        "the reserve on the inflated supply exceeds the cash"
    );
    assert_eq!(
        t.claim_revenue("USDC"),
        0,
        "nothing above the reserve to pay"
    );
    assert_eq!(
        state(&t, "USDC").cash,
        aged.cash,
        "the cash stays for liquidations"
    );
    let usdc = t.resolve_asset("USDC");
    let paid_out = |t: &LendingTest| token::Client::new(&t.env, &usdc).balance(&accumulator);
    assert_eq!(paid_out(&t), 0);

    // 3,000 USDC repaid: cash rises above the reserve, but paying it would
    // carry utilization past 95%, so the claim is refused whole.
    t.repay(ALICE, "USDC", 3_000.0);
    let before = state(&t, "USDC");
    let above_reserve = before.cash - t.liquidation_buffer_raw("USDC");
    let revenue = t.pool_client("USDC").get_revenue(&key(&t, "USDC"));
    let room = t.claim_utilization_room_raw("USDC");
    assert!(
        0 < room && room < above_reserve.min(revenue),
        "the payout would cross the 95% line: {room} {above_reserve} {revenue}"
    );
    assert_eq!(
        t.claim_revenue("USDC"),
        0,
        "refused whole, without a revert"
    );
    assert_eq!(state(&t, "USDC").cash, before.cash);
    assert_eq!(state(&t, "USDC").revenue, before.revenue);
    assert_eq!(paid_out(&t), 0);

    // 2,000 more: the whole revenue fits under the line and is paid.
    t.repay(ALICE, "USDC", 2_000.0);
    let before = state(&t, "USDC");
    let reserve = t.liquidation_buffer_raw("USDC");
    let revenue = t.pool_client("USDC").get_revenue(&key(&t, "USDC"));
    let claimable = (before.cash - reserve).min(revenue);
    assert!(0 < claimable && claimable <= t.claim_utilization_room_raw("USDC"));
    assert_eq!(
        t.claim_revenue("USDC"),
        claimable,
        "pays the claimable cash"
    );
    assert!(
        state(&t, "USDC").cash >= reserve,
        "the reserve stays in the pool"
    );
    assert!(
        t.pool_client("USDC").get_utilisation(&key(&t, "USDC")) <= RAY * 95 / 100,
        "the claim leaves utilization within the 95% claim bound"
    );
    assert_eq!(paid_out(&t), claimable);
}
