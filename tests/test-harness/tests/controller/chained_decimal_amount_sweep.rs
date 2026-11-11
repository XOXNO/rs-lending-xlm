//! One account walks the controller money endpoints with awkward raw amounts
//! at every decimal bound the protocol lists. The chain is supply, index
//! refresh, borrow, a day of accrual, partial repay, partial withdraw,
//! over-repay, full exit, revenue claim, a flash loan of the remaining cash,
//! and a liquidation after a price crash.
//!
//! A fresh deposit round-trips exactly. Supplier claims stay backed by cash
//! plus debt. A full exit never pays more than the position view. Zero
//! amounts, a zero flash loan, and liquidation of a healthy account stay
//! typed rejections.

use common::constants::{MIN_BORROWABLE_ASSET_DECIMALS, MIN_WHOLE_UNIT_COLLATERAL, RAY};
use common::math::fp::Ray as FpRay;
use common::types::{HubAssetKey, SeizeMode};
use soroban_sdk::{vec, Address, Bytes, Vec};
use test_harness::{
    assert_contract_error, days, errors, hub_asset, map_try_ok_unit, map_try_ok_value, usd,
    AssetConfigPreset, LendingTest, MarketPreset, ALICE, BOB, CAROL, DEFAULT_ASSET_CONFIG,
    DEFAULT_MARKET_PARAMS, HARNESS_SPOKE, LIQUIDATOR,
};

const COLLATERAL_DECIMALS: [u32; 5] = [0, 1, 3, 7, 18];
const DEBT_DECIMALS: [u32; 3] = [3, 7, 18];

fn unit(decimals: u32) -> i128 {
    10i128.pow(decimals)
}

fn price_for(decimals: u32) -> i128 {
    if decimals < MIN_BORROWABLE_ASSET_DECIMALS {
        usd(1_000)
    } else {
        usd(1)
    }
}

fn market(name: &'static str, decimals: u32, price_wad: i128) -> MarketPreset {
    let borrowable = decimals >= MIN_BORROWABLE_ASSET_DECIMALS;
    MarketPreset {
        name,
        decimals,
        price_wad,
        initial_liquidity: 0.0,
        config: AssetConfigPreset {
            is_borrowable: borrowable,
            is_flashloanable: borrowable,
            // A market below the borrowable-decimal floor may be collateral
            // only, and only with a zero liquidation fee.
            liquidation_fees: if borrowable {
                DEFAULT_ASSET_CONFIG.liquidation_fees
            } else {
                0
            },
            ..DEFAULT_ASSET_CONFIG
        },
        params: DEFAULT_MARKET_PARAMS,
    }
}

fn probe_amounts(decimals: u32) -> [i128; 2] {
    if decimals < MIN_BORROWABLE_ASSET_DECIMALS {
        [MIN_WHOLE_UNIT_COLLATERAL, 17]
    } else {
        let one = unit(decimals);
        [1, one.saturating_mul(137).saturating_add(1)]
    }
}

fn leg(t: &LendingTest, asset: &str, amount: i128) -> Vec<(HubAssetKey, i128)> {
    vec![&t.env, (hub_asset(t.resolve_asset(asset)), amount)]
}

fn user_addr(t: &LendingTest, name: &str) -> Address {
    t.users.get(name).unwrap().address.clone()
}

fn collateral_usd_wad(raw: i128, decimals: u32, price_wad: i128) -> i128 {
    let scale = unit(decimals);
    raw / scale * price_wad + raw % scale * price_wad / scale
}

/// About 40% of the collateral's LTV, capped so the debt pool still has cash.
fn affordable_borrow(col_raw: i128, col_dec: u32, col_price: i128, debt_dec: u32) -> Option<i128> {
    let ltv_usd = collateral_usd_wad(col_raw, col_dec, col_price) * 7_500 / 10_000;
    let debt_unit_usd = usd(1) / unit(debt_dec);
    if debt_unit_usd <= 0 {
        return None;
    }
    let want = ltv_usd / debt_unit_usd * 4 / 10;
    let cap = unit(debt_dec).saturating_mul(1_000);
    if want < 1 {
        None
    } else {
        Some(want.min(cap))
    }
}

fn pool_state(t: &LendingTest, asset: &str) -> common::types::PoolStateRaw {
    let key = hub_asset(t.resolve_asset(asset));
    t.pool_client(asset).get_sync_data(&key).state
}

fn backing_holds(t: &LendingTest, asset: &str, label: &str) {
    let decimals = t.resolve_market(asset).decimals;
    let state = pool_state(t, asset);
    let supplied = FpRay::from(state.supplied).mul_floor(&t.env, FpRay::from(state.supply_index));
    let debt = FpRay::from(state.borrowed).mul_ceil(&t.env, FpRay::from(state.borrow_index));
    let cash = FpRay::from_asset(&t.env, state.cash, decimals);
    assert!(
        state.cash >= 0
            && state.revenue >= 0
            && state.borrow_index >= RAY
            && state.supply_index > 0,
        "{label} {asset}: cash {} revenue {} indexes {}/{}",
        state.cash,
        state.revenue,
        state.borrow_index,
        state.supply_index
    );
    assert!(
        supplied.raw() <= cash.raw().saturating_add(debt.raw()),
        "{label} {asset}: supplier claims {} exceed cash {} plus debt {}",
        supplied.raw(),
        cash.raw(),
        debt.raw()
    );
}

fn setup(col_dec: u32, debt_dec: u32) -> LendingTest {
    LendingTest::new()
        .with_market(market("COL", col_dec, price_for(col_dec)))
        .with_market(market("DEBT", debt_dec, usd(1)))
        .with_min_borrow_collateral_disabled()
        .with_max_utilization_disabled_all_markets()
        .build()
}

fn fresh_round_trip(t: &mut LendingTest, amount: i128, label: &str) {
    t.supply_raw(ALICE, "COL", amount);
    assert_eq!(
        t.token_balance_raw(ALICE, "COL"),
        0,
        "{label}: supply left tokens in the wallet"
    );
    let view = t.supply_balance_raw(ALICE, "COL");
    assert!(
        view >= amount,
        "{label}: supply view {view} is below the deposit {amount}"
    );
    // Zero is the withdraw-all sentinel, not a no-op.
    t.withdraw_raw(ALICE, "COL", 0);
    let paid = t.token_balance_raw(ALICE, "COL");
    assert_eq!(
        paid, amount,
        "{label}: fresh exit paid {paid}, deposit was {amount}"
    );
    assert_eq!(
        t.supply_balance_raw(ALICE, "COL"),
        0,
        "{label}: position remains"
    );
    backing_holds(t, "COL", label);
}

fn chain(t: &mut LendingTest, amount: i128, col_dec: u32, debt_dec: u32, label: &str) {
    t.supply_raw(ALICE, "COL", amount);
    let alice = user_addr(t, ALICE);
    let assets = vec![
        &t.env,
        hub_asset(t.resolve_asset("COL")),
        hub_asset(t.resolve_asset("DEBT")),
    ];
    t.ctrl_client().update_indexes(&alice, &assets);
    t.supply_raw(BOB, "DEBT", unit(debt_dec).saturating_mul(100_000));
    backing_holds(t, "COL", label);
    backing_holds(t, "DEBT", label);

    let borrow_index_before = pool_state(t, "DEBT").borrow_index;
    let borrowed = affordable_borrow(amount, col_dec, price_for(col_dec), debt_dec);
    if let Some(borrow_raw) = borrowed {
        let before = t.token_balance_raw(ALICE, "DEBT");
        map_try_ok_unit(t.ctrl_client().try_borrow(
            &alice,
            &t.account_id(ALICE),
            &leg(t, "DEBT", borrow_raw),
            &None,
        ))
        .unwrap_or_else(|err| panic!("{label}: borrow of {borrow_raw} raw failed: {err:?}"));
        assert_eq!(
            t.token_balance_raw(ALICE, "DEBT") - before,
            borrow_raw,
            "{label}: borrow paid a different amount than requested"
        );
        assert!(
            t.borrow_balance_raw(ALICE, "DEBT") > 0,
            "{label}: debt rounded away"
        );
        t.assert_healthy(ALICE);
        backing_holds(t, "DEBT", label);

        t.advance_and_sync(days(1));
        t.ctrl_client().update_indexes(&alice, &assets);
        let borrow_index_after = pool_state(t, "DEBT").borrow_index;
        assert!(
            borrow_index_after >= borrow_index_before,
            "{label}: borrow index fell {borrow_index_before} -> {borrow_index_after}"
        );
        let debt_after_accrual = t.borrow_balance_raw(ALICE, "DEBT");
        assert!(
            debt_after_accrual >= borrow_raw,
            "{label}: accrual shrank debt {borrow_raw} -> {debt_after_accrual}"
        );

        if debt_after_accrual > 1 {
            t.repay_raw(ALICE, "DEBT", 1);
            assert!(
                t.borrow_balance_raw(ALICE, "DEBT") <= debt_after_accrual,
                "{label}: repaying 1 raw unit increased the debt"
            );
        }

        let debt = t.borrow_balance_raw(ALICE, "DEBT");
        let wallet = t.token_balance_raw(ALICE, "DEBT");
        let extra = 1_000;
        t.repay_raw(ALICE, "DEBT", debt.saturating_add(extra));
        assert_eq!(
            t.borrow_balance_raw(ALICE, "DEBT"),
            0,
            "{label}: over-repay left debt"
        );
        let wallet_after = t.token_balance_raw(ALICE, "DEBT");
        // The debt view is the floor. Closing it can take one more raw unit
        // than that view, and no more of the surplus.
        assert!(
            wallet_after + 1 >= wallet + extra,
            "{label}: over-repay consumed more than 1 raw unit past the debt view: before {wallet}, after {wallet_after}, extra {extra}, view {debt}"
        );
        assert!(
            wallet_after <= wallet + debt + extra,
            "{label}: over-repay refunded more than the payment"
        );
        backing_holds(t, "DEBT", label);
    }

    let account_ids = vec![&t.env, t.account_id(ALICE)];
    t.ctrl_client()
        .update_account_threshold(&alice, &false, &account_ids);
    let wallet_before_partial = t.token_balance_raw(ALICE, "COL");
    let partial = t.try_withdraw_raw(ALICE, "COL", 1);
    let partial_paid = if partial.is_ok() {
        let paid = t.token_balance_raw(ALICE, "COL") - wallet_before_partial;
        assert!(paid <= 1, "{label}: withdrawing 1 raw unit paid {paid}");
        paid
    } else {
        0
    };
    let col_view = t.supply_balance_raw(ALICE, "COL");
    let col_wallet = t.token_balance_raw(ALICE, "COL");
    let paid = if col_view > 0 {
        t.withdraw_all(ALICE, "COL");
        let paid = t.token_balance_raw(ALICE, "COL") - col_wallet;
        assert!(
            paid <= col_view,
            "{label}: exit paid {paid}, above the view {col_view}"
        );
        paid
    } else {
        0
    };
    assert_eq!(
        paid + partial_paid,
        amount,
        "{label}: collateral with no borrowers paid {paid} plus partial {partial_paid}, deposit was {amount}"
    );
    assert_eq!(
        t.supply_balance_raw(ALICE, "COL"),
        0,
        "{label}: collateral remains"
    );

    match t
        .ctrl_client()
        .try_claim_revenue(&alice, &vec![&t.env, hub_asset(t.resolve_asset("DEBT"))])
    {
        Ok(Ok(_)) => {}
        other => panic!("{label}: claim_revenue failed: {other:?}"),
    }
    backing_holds(t, "COL", label);
    backing_holds(t, "DEBT", label);

    let cash = pool_state(t, "DEBT").cash;
    if cash > 0 {
        let receiver = t.deploy_flash_loan_receiver();
        let flash = (cash / 2).max(1);
        map_try_ok_unit(t.ctrl_client().try_flash_loan(
            &user_addr(t, BOB),
            &hub_asset(t.resolve_asset("DEBT")),
            &flash,
            &receiver,
            &Bytes::new(&t.env),
        ))
        .unwrap_or_else(|err| panic!("{label}: flash loan of {flash} failed: {err:?}"));
        let cash_after = pool_state(t, "DEBT").cash;
        assert!(
            cash_after >= cash,
            "{label}: flash loan reduced cash {cash} -> {cash_after}"
        );
        backing_holds(t, "DEBT", label);
    }

    liquidate_after_crash(t, amount, col_dec, debt_dec, label);
}

fn liquidate_after_crash(
    t: &mut LendingTest,
    amount: i128,
    col_dec: u32,
    debt_dec: u32,
    label: &str,
) {
    t.supply_raw(CAROL, "COL", amount);
    let carol = user_addr(t, CAROL);
    let liquidator = t.get_or_create_user(LIQUIDATOR);
    t.resolve_market("DEBT").token_admin.mint(&liquidator, &1);
    let healthy = map_try_ok_value(t.ctrl_client().try_liquidate(
        &liquidator,
        &t.account_id(CAROL),
        &leg(t, "DEBT", 1),
        &SeizeMode::Transfer,
    ));
    assert_contract_error(healthy, errors::HEALTH_FACTOR_TOO_HIGH);

    let Some(borrow_raw) = affordable_borrow(amount, col_dec, price_for(col_dec), debt_dec) else {
        return;
    };
    map_try_ok_unit(t.ctrl_client().try_borrow(
        &carol,
        &t.account_id(CAROL),
        &leg(t, "DEBT", borrow_raw),
        &None,
    ))
    .unwrap_or_else(|err| panic!("{label}: carol borrow failed: {err:?}"));

    let crashed = price_for(col_dec) / 100;
    let crashed = if crashed < 1 { 1 } else { crashed };
    t.set_price("COL", crashed);
    let hf = t.health_factor_raw(CAROL);
    if hf >= usd(1) {
        return;
    }
    let debt = t.borrow_balance_raw(CAROL, "DEBT");
    let collateral_before = t.supply_balance_raw(CAROL, "COL");
    let payment = debt.saturating_add(1_000);
    t.resolve_market("DEBT")
        .token_admin
        .mint(&liquidator, &payment);
    map_try_ok_value(t.ctrl_client().try_liquidate(
        &liquidator,
        &t.account_id(CAROL),
        &leg(t, "DEBT", payment),
        &SeizeMode::Transfer,
    ))
    .unwrap_or_else(|err| panic!("{label}: liquidation of hf {hf} failed: {err:?}"));
    assert!(
        t.borrow_balance_raw(CAROL, "DEBT") < debt,
        "{label}: liquidation did not reduce debt"
    );
    assert!(
        t.supply_balance_raw(CAROL, "COL") <= collateral_before,
        "{label}: liquidation increased collateral"
    );
    backing_holds(t, "COL", label);
    backing_holds(t, "DEBT", label);
}

#[test]
fn chained_actions_hold_for_decimal_amount_grid() {
    for &col_dec in &COLLATERAL_DECIMALS {
        for &debt_dec in &DEBT_DECIMALS {
            for amount in probe_amounts(col_dec) {
                let label = format!("col {col_dec} dec, debt {debt_dec} dec, amount {amount}");
                let mut t = setup(col_dec, debt_dec);
                fresh_round_trip(&mut t, amount, &label);
                chain(&mut t, amount, col_dec, debt_dec, &label);
            }
        }
    }
}

#[test]
fn same_asset_loop_returns_no_more_than_the_deposit() {
    for decimals in [3u32, 7, 12, 18] {
        for deposit in probe_amounts(decimals) {
            let label = format!("{decimals} decimals, deposit {deposit}");
            let mut t = LendingTest::new()
                .with_market(market("ASSET", decimals, usd(1)))
                .with_min_borrow_collateral_disabled()
                .with_max_utilization_disabled_all_markets()
                .build();
            // A second supplier keeps the pool solvent when the borrower exits.
            let liquidity = unit(decimals).saturating_mul(100_000).max(deposit);
            t.supply_raw(BOB, "ASSET", liquidity);
            t.supply_raw(ALICE, "ASSET", deposit);
            let loop_borrow = deposit * 3_000 / 10_000;
            if loop_borrow >= 1 {
                let alice = user_addr(&t, ALICE);
                map_try_ok_unit(t.ctrl_client().try_borrow(
                    &alice,
                    &t.account_id(ALICE),
                    &leg(&t, "ASSET", loop_borrow),
                    &None,
                ))
                .unwrap_or_else(|err| panic!("{label}: same-asset borrow failed: {err:?}"));
                t.advance_and_sync(days(30));
                let debt = t.borrow_balance_raw(ALICE, "ASSET");
                assert!(
                    debt >= loop_borrow,
                    "{label}: accrual shrank same-asset debt"
                );
                t.repay_raw(ALICE, "ASSET", debt.saturating_add(1));
                assert_eq!(
                    t.borrow_balance_raw(ALICE, "ASSET"),
                    0,
                    "{label}: debt remains"
                );
            }
            let view = t.supply_balance_raw(ALICE, "ASSET");
            let wallet = t.token_balance_raw(ALICE, "ASSET");
            t.withdraw_all(ALICE, "ASSET");
            let paid = t.token_balance_raw(ALICE, "ASSET") - wallet;
            assert!(
                paid >= deposit,
                "{label}: same-asset exit {paid} is below the deposit {deposit}"
            );
            assert!(
                paid <= view,
                "{label}: same-asset exit paid {paid}, above the view {view}"
            );
            backing_holds(&t, "ASSET", &label);
        }
    }
}

#[test]
fn zero_amounts_and_zero_flash_loan_are_rejected() {
    let mut t = setup(7, 7);
    t.supply_raw(ALICE, "COL", unit(7) * 100);
    t.supply_raw(BOB, "DEBT", unit(7) * 100_000);
    let alice = user_addr(&t, ALICE);
    let id = t.account_id(ALICE);
    let ctrl = t.ctrl_client();

    assert_contract_error(
        map_try_ok_value(ctrl.try_supply(&alice, &id, &HARNESS_SPOKE, &leg(&t, "COL", 0))),
        errors::AMOUNT_MUST_BE_POSITIVE,
    );
    assert_contract_error(
        map_try_ok_unit(ctrl.try_borrow(&alice, &id, &leg(&t, "DEBT", 0), &None)),
        errors::AMOUNT_MUST_BE_POSITIVE,
    );
    assert_contract_error(
        map_try_ok_unit(ctrl.try_repay(&alice, &id, &leg(&t, "DEBT", 0))),
        errors::AMOUNT_MUST_BE_POSITIVE,
    );
    let receiver = t.deploy_flash_loan_receiver();
    assert_contract_error(
        map_try_ok_unit(ctrl.try_flash_loan(
            &user_addr(&t, BOB),
            &hub_asset(t.resolve_asset("DEBT")),
            &0,
            &receiver,
            &Bytes::new(&t.env),
        )),
        errors::AMOUNT_MUST_BE_POSITIVE,
    );
}
