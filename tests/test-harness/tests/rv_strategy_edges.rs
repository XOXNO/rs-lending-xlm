//! W4 strategy-edge review: `flash_position` (self-referential debt, refunds,
//! minimums, callback reentry), `multiply` initial payments and the router
//! boundary, same-key `swap_collateral` and `repay_debt_with_collateral`,
//! the INV-HALT-01 pause matrix, the delegate close payout, and the router
//! allowance after each strategy.
//!
//! Units: USDC and ETH have 7 decimals, so one whole token is 10_000_000 raw.
//! `apply_flash_fee(x)` is the amount the pool releases after the 9 bps
//! strategy fee that `multiply` withholds (`flash_position` withholds nothing).

use std::collections::BTreeMap;

use controller::constants::{RAY, WAD};
use controller::types::PositionMode;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::xdr::{FromXdr, ToXdr};
use soroban_sdk::{contract, contractimpl, contracttype, token, Address, Bytes, Env, Vec};
use test_harness::mock_aggregator::{BadAggregator, BadMode, ReenterMode, ReenteringAggregator};
use test_harness::{
    apply_flash_fee, assert_contract_error, build_aggregator_swap, errors, hub_asset,
    unconstrained_test_cap, xlm_preset, FlashPositionMode, FlashPositionRequest, HubAssetKey,
    LendingTest, MockSwapPayload, ALICE, BOB, CAROL, DAVE, EVE, HARNESS_HUB, HARNESS_SPOKE,
};

/// One whole ETH or USDC in raw units.
const ETH: i128 = 10_000_000;
const USDC: i128 = 10_000_000;
/// Fee `multiply` withholds on 1 ETH of strategy debt: 9 bps, i.e. 1 ETH - `apply_flash_fee(ETH)`.
const FEE_ON_ONE_ETH: i128 = 9_000;
const YEAR_SECS: u64 = 365 * 86_400;
/// The error the host returns for a contract re-entering a frame already on the stack.
const HOST_REENTRY: &str = "Error(Context, InvalidAction)";

/// Minimal router whose spend and payout are configurable. The harness mocks
/// spend exactly `amount_in`, so a leftover or an overspend needs this double.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Route {
    /// Input the router leaves on the controller: it pulls `total_in - shortfall`.
    pub shortfall: i128,
    /// Input the router pulls beyond `total_in`.
    pub overshoot: i128,
    /// Whether the router pays `min_out` back to the controller.
    pub pay_output: bool,
}

#[contracttype]
#[derive(Clone, Copy)]
enum RouterKey {
    Route,
    LastTotalIn,
}

#[contract]
pub struct RvRouter;

#[contractimpl]
impl RvRouter {
    pub fn __constructor(env: Env, route: Route) {
        env.storage().instance().set(&RouterKey::Route, &route);
    }

    pub fn last_total_in(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&RouterKey::LastTotalIn)
            .unwrap_or(0)
    }

    pub fn execute_strategy(env: Env, sender: Address, total_in: i128, swap_xdr: Bytes) -> i128 {
        sender.require_auth();
        let route: Route = env
            .storage()
            .instance()
            .get(&RouterKey::Route)
            .expect("route is set by the constructor");
        env.storage()
            .instance()
            .set(&RouterKey::LastTotalIn, &total_in);
        let payload =
            MockSwapPayload::from_xdr(&env, &swap_xdr).expect("mock swap payload must decode");
        let router = env.current_contract_address();

        let pull = total_in - route.shortfall + route.overshoot;
        if pull > 0 {
            token::Client::new(&env, &payload.token_in).transfer(&sender, &router, &pull);
        }
        if route.pay_output && payload.min_out > 0 {
            token::Client::new(&env, &payload.token_out).transfer(
                &router,
                &sender,
                &payload.min_out,
            );
        }
        payload.min_out
    }
}

/// Flattens a `try_*` client result into one `soroban_sdk::Error` channel.
fn flatten<T, E>(
    r: Result<Result<T, E>, Result<soroban_sdk::Error, soroban_sdk::InvokeError>>,
) -> Result<T, soroban_sdk::Error>
where
    E: Into<soroban_sdk::Error>,
{
    match r {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(err)) => Err(err.into()),
        Err(Ok(err)) => Err(err),
        Err(Err(invoke)) => panic!("unexpected host invoke failure: {invoke:?}"),
    }
}

fn bal(t: &LendingTest, asset: &Address, who: &Address) -> i128 {
    token::Client::new(&t.env, asset).balance(who)
}

fn key(t: &LendingTest, name: &str) -> HubAssetKey {
    hub_asset(t.resolve_asset(name))
}

fn fund(t: &LendingTest, name: &str, who: &Address, raw: i128) {
    t.resolve_market(name).token_admin.mint(who, &raw);
}

fn install_route(t: &LendingTest, route: Route) -> Address {
    let router = t.env.register(RvRouter, (route,));
    t.ctrl_client().set_swap_aggregator(&router);
    router
}

fn flash_guard_clear(t: &LendingTest) -> bool {
    t.env.as_contract(&t.controller, || {
        !controller::test_support::is_flash_loan_ongoing(&t.env)
    })
}

/// Every balance and pool figure a rejected or reverted strategy must leave
/// unchanged. Pool figures are asset units (`get_supplied_amount` and friends).
fn books(t: &LendingTest, assets: &[&str], parties: &[(&str, Address)]) -> BTreeMap<String, i128> {
    let mut out = BTreeMap::new();
    for name in assets {
        let market = t.resolve_market(name);
        let asset = market.asset.clone();
        let pool = t.pool_client(name);
        let k = hub_asset(asset.clone());
        for (label, who) in parties {
            out.insert(format!("{label}.{name}"), bal(t, &asset, who));
        }
        out.insert(format!("pool.{name}.tokens"), bal(t, &asset, &market.pool));
        out.insert(
            format!("pool.{name}.supplied"),
            pool.get_supplied_amount(&k),
        );
        out.insert(
            format!("pool.{name}.borrowed"),
            pool.get_borrowed_amount(&k),
        );
        out.insert(format!("pool.{name}.cash"), pool.get_reserves(&k));
        out.insert(format!("pool.{name}.revenue"), t.snapshot_revenue(name));
    }
    out.insert(
        "flash_guard_set".to_string(),
        i128::from(!flash_guard_clear(t)),
    );
    out
}

fn payload(
    t: &LendingTest,
    mode: FlashPositionMode,
    collateral: &str,
    collateral_amount: i128,
    extra: &str,
    extra_amount: i128,
) -> Bytes {
    FlashPositionRequest {
        mode,
        collateral: t.resolve_asset(collateral),
        collateral_amount,
        extra_asset: t.resolve_asset(extra),
        extra_amount,
        reenter_spoke_id: HARNESS_SPOKE,
        reenter_account_id: 0,
    }
    .to_xdr(&t.env)
}

fn mins(t: &LendingTest, rows: &[(&str, i128)]) -> Vec<(HubAssetKey, i128)> {
    let mut out = Vec::new(&t.env);
    for (name, min) in rows {
        out.push_back((key(t, name), *min));
    }
    out
}

fn refunds(t: &LendingTest, names: &[&str]) -> Vec<Address> {
    let mut out = Vec::new(&t.env);
    for name in names {
        out.push_back(t.resolve_asset(name));
    }
    out
}

/// Opens a position in `mode`, or extends `account_id` (0 creates one), with the
/// debt given in raw units of `debt` (a `key` name, or a hub-specific key).
#[allow(clippy::too_many_arguments)]
fn flash_mode(
    t: &LendingTest,
    caller: &Address,
    account_id: u64,
    mode: PositionMode,
    debt: &HubAssetKey,
    debt_raw: i128,
    receiver: &Address,
    data: &Bytes,
    collaterals: &Vec<(HubAssetKey, i128)>,
    refund_list: &Vec<Address>,
) -> Result<u64, soroban_sdk::Error> {
    flatten(t.ctrl_client().try_flash_position(
        caller,
        &account_id,
        &HARNESS_SPOKE,
        &mode,
        debt,
        &debt_raw,
        receiver,
        data,
        collaterals,
        refund_list,
    ))
}

/// `flash_mode` in Multiply mode on the base hub.
#[allow(clippy::too_many_arguments)]
fn flash(
    t: &LendingTest,
    caller: &Address,
    account_id: u64,
    debt: &str,
    debt_raw: i128,
    receiver: &Address,
    data: &Bytes,
    collaterals: &Vec<(HubAssetKey, i128)>,
    refund_list: &Vec<Address>,
) -> Result<u64, soroban_sdk::Error> {
    flash_mode(
        t,
        caller,
        account_id,
        PositionMode::Multiply,
        &key(t, debt),
        debt_raw,
        receiver,
        data,
        collaterals,
        refund_list,
    )
}

#[allow(clippy::too_many_arguments)]
fn multiply(
    t: &LendingTest,
    caller: &Address,
    collateral: &str,
    debt: &str,
    debt_raw: i128,
    swap: &Bytes,
    initial: Option<(&str, i128)>,
    convert: Option<Bytes>,
) -> Result<u64, soroban_sdk::Error> {
    let initial = initial.map(|(name, amount)| (key(t, name), amount));
    flatten(t.ctrl_client().try_multiply(
        caller,
        &0u64,
        &HARNESS_SPOKE,
        &key(t, collateral),
        &debt_raw,
        &key(t, debt),
        &PositionMode::Multiply,
        swap,
        &initial,
        &convert,
    ))
}

/// A Multiply account holding `usdc_units` whole USDC of collateral.
fn base_account(t: &mut LendingTest, user: &str, usdc_units: f64) -> (Address, u64) {
    let acct = t.create_account_full(user, HARNESS_SPOKE, PositionMode::Multiply);
    t.supply_to(user, acct, "USDC", usdc_units);
    (t.get_or_create_user(user), acct)
}

/// ETH market with a 50% utilization ceiling and 1,000 ETH supplied by BOB.
/// The initial harness liquidity is cash, not supply, so utilization starts at 0.
fn util_fixture() -> LendingTest {
    let mut t = LendingTest::new()
        .standard_two_asset()
        .with_market_params("ETH", |p| {
            p.mid_utilization = RAY / 5;
            p.optimal_utilization = RAY * 3 / 10;
            p.max_utilization = RAY / 2;
        })
        .build();
    t.supply(BOB, "ETH", 1_000.0);
    t
}

// ---------------------------------------------------------------------------
// H1: a flash position whose declared collateral is the debt asset itself.
// ---------------------------------------------------------------------------

/// The controller accepts a self-referential open: the receiver returns the 1 ETH
/// it borrowed and that 1 ETH is deposited as collateral. The position mirrors
/// (supply 1 ETH, debt 1 ETH) and the ETH pool's cash does not move.
#[test]
fn rv_flash_position_self_debt_mirrors_positions_cash_unchanged() {
    let mut t = LendingTest::new().standard_two_asset().build();
    let (alice, acct) = base_account(&mut t, ALICE, 4_000.0);
    let receiver = t.deploy_flash_position_receiver();
    let parties = [
        ("alice", alice.clone()),
        ("controller", t.controller.clone()),
        ("receiver", receiver.clone()),
    ];
    let before = books(&t, &["USDC", "ETH"], &parties);

    let data = payload(&t, FlashPositionMode::PushDebtBack, "ETH", 0, "ETH", 0);
    let id = flash(
        &t,
        &alice,
        acct,
        "ETH",
        ETH,
        &receiver,
        &data,
        &mins(&t, &[("ETH", ETH)]),
        &refunds(&t, &[]),
    )
    .expect("self-referential open must be accepted");
    assert_eq!(id, acct);

    assert_eq!(t.supply_balance_raw_for(acct, "ETH"), ETH, "ETH supply 1.0");
    assert_eq!(t.borrow_balance_raw_for(acct, "ETH"), ETH, "ETH debt 1.0");
    assert_eq!(
        t.supply_balance_raw_for(acct, "USDC"),
        4_000 * USDC,
        "USDC base untouched"
    );

    let after = books(&t, &["USDC", "ETH"], &parties);
    let delta = |k: &str| after[k] - before[k];
    assert_eq!(delta("pool.ETH.supplied"), ETH, "deposit leg books supply");
    assert_eq!(delta("pool.ETH.borrowed"), ETH, "mint books debt");
    assert_eq!(
        delta("pool.ETH.cash"),
        0,
        "cash out and back in: no net cash"
    );
    assert_eq!(delta("pool.ETH.tokens"), 0, "pool token balance unchanged");
    assert_eq!(delta("pool.ETH.revenue"), 0, "no fee on a flash position");
    assert_eq!(delta("receiver.ETH"), 0, "receiver keeps nothing");
    assert_eq!(delta("alice.ETH"), 0, "caller receives no ETH");

    // Supply 1 ETH at threshold 0.8 is 1,600 USD; the 4,000 USDC base is 3,200 USD;
    // debt is 2,000 USD. HF = (3,200 + 1,600) / 2,000 = 2.4.
    let hf = t.health_factor_for_raw(ALICE, acct);
    let expected = 24 * WAD / 10;
    assert!(
        (hf - expected).abs() < WAD / 1_000_000,
        "HF should be 2.4, got {hf}"
    );
}

/// The LTV gate is the binding solvency check. With 4,000 USDC of base collateral
/// the self-supplied ETH counts at LTV 0.75, so debt X ETH passes iff
/// 0.75 * (4,000 + 2,000 X) >= 2,000 X, which is X <= 6. Without the base, the
/// loop cannot pass at all (0.75 * 2,000 < 2,000).
#[test]
fn rv_flash_position_self_debt_ltv_boundary_is_six_eth() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(EVE, "ETH", 100_000.0);
    let receiver = t.deploy_flash_position_receiver();

    let (alice, a_alice) = base_account(&mut t, ALICE, 4_000.0);
    let over = flash(
        &t,
        &alice,
        a_alice,
        "ETH",
        61_000_000,
        &receiver,
        &payload(&t, FlashPositionMode::PushDebtBack, "ETH", 0, "ETH", 0),
        &mins(&t, &[("ETH", 61_000_000)]),
        &refunds(&t, &[]),
    );
    assert_contract_error(over, errors::INSUFFICIENT_COLLATERAL);
    assert_eq!(
        t.supply_balance_raw_for(a_alice, "ETH"),
        0,
        "rejected open leaves no ETH"
    );

    let (bob, a_bob) = base_account(&mut t, BOB, 4_000.0);
    let at_six = flash(
        &t,
        &bob,
        a_bob,
        "ETH",
        60_000_000,
        &receiver,
        &payload(&t, FlashPositionMode::PushDebtBack, "ETH", 0, "ETH", 0),
        &mins(&t, &[("ETH", 60_000_000)]),
        &refunds(&t, &[]),
    );
    assert!(
        at_six.is_ok(),
        "exactly 6 ETH sits on the LTV equality: {at_six:?}"
    );

    let (carol, a_carol) = base_account(&mut t, CAROL, 4_000.0);
    flash(
        &t,
        &carol,
        a_carol,
        "ETH",
        59_000_000,
        &receiver,
        &payload(&t, FlashPositionMode::PushDebtBack, "ETH", 0, "ETH", 0),
        &mins(&t, &[("ETH", 59_000_000)]),
        &refunds(&t, &[]),
    )
    .expect("5.9 ETH is inside the LTV bound");

    let dave = t.create_account_full(DAVE, HARNESS_SPOKE, PositionMode::Multiply);
    let dave_addr = t.get_or_create_user(DAVE);
    let no_base = flash(
        &t,
        &dave_addr,
        dave,
        "ETH",
        ETH,
        &receiver,
        &payload(&t, FlashPositionMode::PushDebtBack, "ETH", 0, "ETH", 0),
        &mins(&t, &[("ETH", ETH)]),
        &refunds(&t, &[]),
    );
    assert_contract_error(no_base, errors::INSUFFICIENT_COLLATERAL);
}

/// Spoke caps apply to the self-referential legs. The borrow cap is checked
/// before the callback and the supply cap after it, so each rejection names
/// its own leg. A supply total exactly at the cap is admitted.
#[test]
fn rv_flash_position_self_debt_spoke_caps_reject_each_leg() {
    let mut t = LendingTest::new().standard_two_asset().build();
    let (alice, acct) = base_account(&mut t, ALICE, 4_000.0);
    let receiver = t.deploy_flash_position_receiver();
    let unlimited = unconstrained_test_cap(7);
    let data = payload(&t, FlashPositionMode::PushDebtBack, "ETH", 0, "ETH", 0);
    let parties = [
        ("alice", alice.clone()),
        ("controller", t.controller.clone()),
        ("receiver", receiver.clone()),
    ];

    t.edit_asset_in_spoke_caps(
        "ETH",
        HARNESS_SPOKE,
        true,
        true,
        7_500,
        8_000,
        500,
        unlimited,
        5_000_000,
    );
    let before = books(&t, &["USDC", "ETH"], &parties);
    let borrow_leg = flash(
        &t,
        &alice,
        acct,
        "ETH",
        ETH,
        &receiver,
        &data,
        &mins(&t, &[("ETH", ETH)]),
        &refunds(&t, &[]),
    );
    assert_contract_error(borrow_leg, errors::SPOKE_BORROW_CAP_REACHED);
    assert_eq!(
        books(&t, &["USDC", "ETH"], &parties),
        before,
        "borrow-cap reject rolls back"
    );

    t.edit_asset_in_spoke_caps(
        "ETH",
        HARNESS_SPOKE,
        true,
        true,
        7_500,
        8_000,
        500,
        5_000_000,
        unlimited,
    );
    let just_over = flash(
        &t,
        &alice,
        acct,
        "ETH",
        5_000_001,
        &receiver,
        &payload(&t, FlashPositionMode::PushDebtBack, "ETH", 0, "ETH", 0),
        &mins(&t, &[("ETH", 5_000_001)]),
        &refunds(&t, &[]),
    );
    assert_contract_error(just_over, errors::SPOKE_SUPPLY_CAP_REACHED);
    assert_eq!(
        books(&t, &["USDC", "ETH"], &parties),
        before,
        "supply-cap reject rolls back"
    );

    flash(
        &t,
        &alice,
        acct,
        "ETH",
        5_000_000,
        &receiver,
        &payload(&t, FlashPositionMode::PushDebtBack, "ETH", 0, "ETH", 0),
        &mins(&t, &[("ETH", 5_000_000)]),
        &refunds(&t, &[]),
    )
    .expect("supply landing exactly on the cap is admitted");
    assert_eq!(t.supply_balance_raw_for(acct, "ETH"), 5_000_000);
    assert_eq!(t.borrow_balance_raw_for(acct, "ETH"), 5_000_000);
}

/// The pool's utilization gate runs on the mint, before the callback. It reads
/// borrowed after the mint over supplied before the self-deposit, so a loop of
/// 600 ETH on 1,000 supplied (0.60 > 0.50) is refused even though the post-loop
/// utilization would be 600 / 1,600 = 0.375. A 400 ETH loop is admitted at 0.40
/// on the mint.
#[test]
fn rv_flash_position_self_debt_mint_gate_uses_pre_deposit_supply() {
    // Empty market: INV-ACCT-08 skips the gate at zero total supply, so the loop's own
    // deposit is the only supply and utilization lands on exactly 1.0. The next borrower
    // is then refused: (6 + 0.1) / 6 > 0.95.
    let mut empty = LendingTest::new().standard_two_asset().build();
    let (e_alice, e_acct) = base_account(&mut empty, ALICE, 4_000.0);
    let e_receiver = empty.deploy_flash_position_receiver();
    flash(
        &empty,
        &e_alice,
        e_acct,
        "ETH",
        6 * ETH,
        &e_receiver,
        &payload(&empty, FlashPositionMode::PushDebtBack, "ETH", 0, "ETH", 0),
        &mins(&empty, &[("ETH", 6 * ETH)]),
        &refunds(&empty, &[]),
    )
    .expect("loop into an empty market is ungated");
    assert_eq!(
        empty
            .pool_client("ETH")
            .get_utilisation(&key(&empty, "ETH")),
        RAY,
        "supplied == borrowed == 6 ETH: utilization exactly 100%"
    );
    let (e_bob, e_bob_acct) = base_account(&mut empty, BOB, 4_000.0);
    let e_refused = flash(
        &empty,
        &e_bob,
        e_bob_acct,
        "ETH",
        ETH / 10,
        &e_receiver,
        &payload(&empty, FlashPositionMode::PushDebtBack, "ETH", 0, "ETH", 0),
        &mins(&empty, &[("ETH", ETH / 10)]),
        &refunds(&empty, &[]),
    );
    assert_contract_error(e_refused, errors::UTILIZATION_ABOVE_MAX);

    let mut t = util_fixture();
    let (alice, acct) = base_account(&mut t, ALICE, 500_000.0);
    let receiver = t.deploy_flash_position_receiver();

    let over = flash(
        &t,
        &alice,
        acct,
        "ETH",
        600 * ETH,
        &receiver,
        &payload(&t, FlashPositionMode::PushDebtBack, "ETH", 0, "ETH", 0),
        &mins(&t, &[("ETH", 600 * ETH)]),
        &refunds(&t, &[]),
    );
    assert_contract_error(over, errors::UTILIZATION_ABOVE_MAX);
    assert_eq!(
        t.borrow_balance_raw_for(acct, "ETH"),
        0,
        "rejected mint leaves no debt"
    );

    // Exactly at the ceiling on the mint: 500 / 1,000 = 0.50 is admitted, one raw unit more is not.
    let one_over = flash(
        &t,
        &alice,
        acct,
        "ETH",
        500 * ETH + 1,
        &receiver,
        &payload(&t, FlashPositionMode::PushDebtBack, "ETH", 0, "ETH", 0),
        &mins(&t, &[("ETH", 500 * ETH + 1)]),
        &refunds(&t, &[]),
    );
    assert_contract_error(one_over, errors::UTILIZATION_ABOVE_MAX);
    flash(
        &t,
        &alice,
        acct,
        "ETH",
        500 * ETH,
        &receiver,
        &payload(&t, FlashPositionMode::PushDebtBack, "ETH", 0, "ETH", 0),
        &mins(&t, &[("ETH", 500 * ETH)]),
        &refunds(&t, &[]),
    )
    .expect("500 ETH mints at exactly 0.50 utilization");

    // The loop's own 500 ETH deposit is not in the mint check: post-loop 500 / 1,500 = 1/3.
    let util = t.pool_client("ETH").get_utilisation(&key(&t, "ETH")) as f64 / RAY as f64;
    assert!(
        (util - 1.0 / 3.0).abs() < 1e-9,
        "post-loop utilization is 500 / 1,500 = 1/3, got {util}"
    );

    // Headroom for a third party: 0.50 x 1,500 - 500 = 250 ETH, down from 500 ETH before the loop.
    let (carol, carol_acct) = base_account(&mut t, CAROL, 1_000_000.0);
    let eth_key = key(&t, "ETH");
    let over_headroom = flatten(t.ctrl_client().try_borrow(
        &carol,
        &carol_acct,
        &Vec::from_array(&t.env, [(eth_key.clone(), 250 * ETH + 1)]),
        &None,
    ));
    assert_contract_error(over_headroom, errors::UTILIZATION_ABOVE_MAX);
    flatten(t.ctrl_client().try_borrow(
        &carol,
        &carol_acct,
        &Vec::from_array(&t.env, [(eth_key, 250 * ETH)]),
        &None,
    ))
    .expect("250 ETH is exactly the remaining headroom");
}

/// The zero-cash griefing question. A loop leaves pool cash unchanged but adds the
/// same notional to borrowed and supplied, so utilization rises from 0 to 400 /
/// 1,400 = 0.2857 and the borrow APR rises from 1.00% to the value below. The cost
/// to the looper is the spread on the notional: borrow APR minus supply APR.
#[test]
fn rv_flash_position_self_debt_zero_cash_loop_costs_rate_spread() {
    let mut t = util_fixture();
    let (alice, acct) = base_account(&mut t, ALICE, 500_000.0);
    let receiver = t.deploy_flash_position_receiver();
    let parties = [
        ("alice", alice.clone()),
        ("controller", t.controller.clone()),
    ];
    let cash_before = books(&t, &["ETH"], &parties);

    assert!(
        (t.pool_borrow_rate("ETH") - 0.01).abs() < 1e-12,
        "base APR at 0 utilization is 1.00%"
    );
    assert_eq!(
        t.pool_client("ETH").get_deposit_rate(&key(&t, "ETH")),
        0,
        "no suppliers' yield at 0 utilization"
    );

    flash(
        &t,
        &alice,
        acct,
        "ETH",
        400 * ETH,
        &receiver,
        &payload(&t, FlashPositionMode::PushDebtBack, "ETH", 0, "ETH", 0),
        &mins(&t, &[("ETH", 400 * ETH)]),
        &refunds(&t, &[]),
    )
    .expect("400 ETH loop");

    let cash_after = books(&t, &["ETH"], &parties);
    assert_eq!(
        cash_after["pool.ETH.cash"], cash_before["pool.ETH.cash"],
        "cash unchanged"
    );
    assert_eq!(
        cash_after["pool.ETH.tokens"], cash_before["pool.ETH.tokens"],
        "tokens unchanged"
    );

    let borrow_apr = t.pool_borrow_rate("ETH");
    let supply_apr = t.pool_client("ETH").get_deposit_rate(&key(&t, "ETH")) as f64 / RAY as f64;
    assert!(
        borrow_apr > supply_apr && supply_apr > 0.0,
        "borrow APR {borrow_apr} must exceed supply APR {supply_apr}"
    );

    let debt_before = t.borrow_balance_raw_for(acct, "ETH");
    let supply_before = t.supply_balance_raw_for(acct, "ETH");
    t.advance_and_sync(YEAR_SECS);
    let debt_growth = (t.borrow_balance_raw_for(acct, "ETH") - debt_before) as f64;
    let supply_growth = (t.supply_balance_raw_for(acct, "ETH") - supply_before) as f64;
    assert!(
        debt_growth > supply_growth,
        "looper pays more than it earns"
    );

    // The per-millisecond compounding of the indexes makes one year of debt growth e^r - 1.
    let notional = 400.0 * ETH as f64;
    let debt_expected = notional * (borrow_apr.exp() - 1.0);
    assert!(
        (debt_growth - debt_expected).abs() / debt_expected < 0.01,
        "debt growth should be e^r - 1 on the notional: expected {debt_expected}, got {debt_growth}"
    );
    // Supply compounds at a rate that drifts up as the loop's debt grows faster than supply,
    // so the one-year net cost sits slightly below the notional x (e^rb - e^rs) bound.
    let net_cost = debt_growth - supply_growth;
    let net_expected = notional * (borrow_apr.exp() - supply_apr.exp());
    assert!(
        net_cost > 0.0 && (net_cost - net_expected).abs() / net_expected < 0.03,
        "one-year net cost should be about notional x (e^rb - e^rs) = {net_expected}, got {net_cost} \
         (borrow APR {borrow_apr}, supply APR {supply_apr}, debt growth {debt_growth}, supply growth {supply_growth})"
    );
}

// ---------------------------------------------------------------------------
// H2 and H3: refund_assets.
// ---------------------------------------------------------------------------

/// Returned debt tokens that are refund-listed go back to the caller. The
/// account keeps the full debt, the pool books no fee, and HF stays healthy.
#[test]
fn rv_flash_position_refund_debt_token_fee_free_keeps_debt() {
    let mut t = LendingTest::new().standard_two_asset().build();
    let receiver = t.deploy_flash_position_receiver();
    let alice = t.get_or_create_user(ALICE);
    let parties = [
        ("alice", alice.clone()),
        ("controller", t.controller.clone()),
        ("receiver", receiver.clone()),
    ];
    let before = books(&t, &["USDC", "ETH"], &parties);

    let acct = flash(
        &t,
        &alice,
        0,
        "ETH",
        ETH,
        &receiver,
        &payload(
            &t,
            FlashPositionMode::SupplyAndReturnDebt,
            "USDC",
            4_000 * USDC,
            "ETH",
            0,
        ),
        &mins(&t, &[("USDC", 4_000 * USDC)]),
        &refunds(&t, &["ETH"]),
    )
    .expect("minimum met and debt token refunded");

    assert_eq!(
        t.borrow_balance_raw_for(acct, "ETH"),
        ETH,
        "full 1 ETH debt retained"
    );
    assert_eq!(
        t.supply_balance_raw_for(acct, "USDC"),
        4_000 * USDC,
        "declared collateral deposited"
    );
    assert_eq!(
        t.supply_balance_raw_for(acct, "ETH"),
        0,
        "refunded ETH is not collateral"
    );

    let after = books(&t, &["USDC", "ETH"], &parties);
    let delta = |k: &str| after[k] - before[k];
    assert_eq!(
        delta("alice.ETH"),
        ETH,
        "caller gets the full borrowed ETH back"
    );
    assert_eq!(delta("pool.ETH.revenue"), 0, "no origination fee");
    assert_eq!(delta("pool.ETH.borrowed"), ETH);
    assert_eq!(
        delta("pool.ETH.cash"),
        -ETH,
        "cash left the pool as a plain borrow would"
    );
    assert_eq!(delta("pool.ETH.tokens"), -ETH);

    // Multiply on the same 1 ETH books FEE_ON_ONE_ETH (strategy_origination_fee_parity.rs).
    assert_eq!(ETH - apply_flash_fee(ETH), FEE_ON_ONE_ETH);
    // 4,000 USDC at threshold 0.8 is 3,200 USD against 2,000 USD of debt.
    let hf = t.health_factor_for_raw(ALICE, acct);
    assert!(
        (hf - 16 * WAD / 10).abs() < WAD / 1_000_000,
        "HF 1.6, got {hf}"
    );
}

/// A refund-listed asset the caller already supplies is paid to the caller and is
/// never deposited. The existing 2 ETH supply does not move.
#[test]
fn rv_flash_position_refund_supplied_asset_pays_caller_only() {
    let mut t = LendingTest::new().standard_two_asset().build();
    let acct = t.create_account_full(ALICE, HARNESS_SPOKE, PositionMode::Multiply);
    t.supply_to(ALICE, acct, "ETH", 2.0);
    let alice = t.get_or_create_user(ALICE);
    let receiver = t.deploy_flash_position_receiver();
    let parties = [
        ("alice", alice.clone()),
        ("controller", t.controller.clone()),
        ("receiver", receiver.clone()),
    ];
    let before = books(&t, &["USDC", "ETH"], &parties);

    // The receiver pushes 4,000 USDC (declared) and 0.5 ETH (undeclared, refund-listed).
    let id = flash(
        &t,
        &alice,
        acct,
        "ETH",
        ETH,
        &receiver,
        &payload(
            &t,
            FlashPositionMode::Undeclared,
            "USDC",
            4_000 * USDC,
            "ETH",
            ETH / 2,
        ),
        &mins(&t, &[("USDC", 4_000 * USDC)]),
        &refunds(&t, &["ETH"]),
    )
    .expect("undeclared refund-listed push");
    assert_eq!(id, acct);

    let after = books(&t, &["USDC", "ETH"], &parties);
    let delta = |k: &str| after[k] - before[k];
    assert_eq!(
        delta("alice.ETH"),
        ETH / 2,
        "0.5 ETH refunded to the caller"
    );
    assert_eq!(delta("alice.USDC"), 0);
    assert_eq!(
        delta("pool.ETH.supplied"),
        0,
        "refunded ETH never reaches the pool"
    );
    assert_eq!(
        delta("pool.ETH.tokens"),
        -ETH,
        "only the 1 ETH principal left the pool"
    );
    assert_eq!(
        t.supply_balance_raw_for(acct, "ETH"),
        2 * ETH,
        "existing 2 ETH supply unchanged"
    );
    assert_eq!(t.borrow_balance_raw_for(acct, "ETH"), ETH);
}

// ---------------------------------------------------------------------------
// H4: a receiver that delivers less than the declared minimum.
// ---------------------------------------------------------------------------

/// One raw unit short of the minimum reverts with `CollateralMinimumNotMet`. Every
/// balance, the pool's books and the flash guard are as they were before the call,
/// and no account is created.
#[test]
fn rv_flash_position_below_minimum_reverts_all_balances() {
    let mut t = LendingTest::new().standard_two_asset().build();
    let alice = t.get_or_create_user(ALICE);
    let receiver = t.deploy_flash_position_receiver();
    let parties = [
        ("alice", alice.clone()),
        ("controller", t.controller.clone()),
        ("receiver", receiver.clone()),
    ];
    let before = books(&t, &["USDC", "ETH"], &parties);

    let short = flash(
        &t,
        &alice,
        0,
        "ETH",
        ETH,
        &receiver,
        &payload(
            &t,
            FlashPositionMode::BelowMin,
            "USDC",
            4_000 * USDC,
            "ETH",
            0,
        ),
        &mins(&t, &[("USDC", 4_000 * USDC)]),
        &refunds(&t, &[]),
    );
    assert_contract_error(short, errors::COLLATERAL_MINIMUM_NOT_MET);

    assert_eq!(
        books(&t, &["USDC", "ETH"], &parties),
        before,
        "every balance, pool figure and the guard unchanged"
    );
    assert!(!t.account_exists(1), "the reverted open creates no account");
}

// ---------------------------------------------------------------------------
// H5: multiply initial payments and the router boundary.
// ---------------------------------------------------------------------------

/// A debt-token initial payment joins the swap input, so the router pulls
/// `apply_flash_fee(1 ETH) + 0.5 ETH` exactly. The 9,000 raw fee is the only
/// revenue booked.
#[test]
fn rv_multiply_debt_token_payment_swap_input_exact() {
    let mut t = LendingTest::new().standard_two_asset().build();
    let alice = t.get_or_create_user(ALICE);
    let router = install_route(
        &t,
        Route {
            shortfall: 0,
            overshoot: 0,
            pay_output: true,
        },
    );
    fund(&t, "USDC", &router, 4_500 * USDC);
    fund(&t, "ETH", &alice, ETH / 2);
    let swap = build_aggregator_swap(&t, "ETH", "USDC", 0, 4_500 * USDC);
    let revenue_before = t.snapshot_revenue("ETH");

    let id = multiply(
        &t,
        &alice,
        "USDC",
        "ETH",
        ETH,
        &swap,
        Some(("ETH", ETH / 2)),
        None,
    )
    .expect("debt-token initial payment");

    let router_client = RvRouterClient::new(&t.env, &router);
    assert_eq!(
        router_client.last_total_in(),
        apply_flash_fee(ETH) + ETH / 2,
        "swap input is the measured borrow (after fee) plus the 0.5 ETH payment"
    );
    assert_eq!(t.snapshot_revenue("ETH") - revenue_before, FEE_ON_ONE_ETH);
    assert_eq!(t.supply_balance_raw_for(id, "USDC"), 4_500 * USDC);
    assert_eq!(t.borrow_balance_raw_for(id, "ETH"), ETH);
    assert_eq!(
        bal(&t, &t.resolve_asset("ETH"), &alice),
        0,
        "the 0.5 ETH payment is fully consumed"
    );
    assert_eq!(bal(&t, &t.resolve_asset("ETH"), &router), 14_991_000);
}

/// A third-asset initial payment without `convert_swap` reverts with
/// `ConvertStepsRequired`. Balances and pool figures do not move.
#[test]
fn rv_multiply_third_token_no_convert_reverts_all_balances() {
    let mut t = LendingTest::new()
        .standard_two_asset()
        .with_market(xlm_preset())
        .build();
    let alice = t.get_or_create_user(ALICE);
    fund(&t, "XLM", &alice, 100 * 10_000_000);
    let parties = [
        ("alice", alice.clone()),
        ("controller", t.controller.clone()),
    ];
    let assets = ["USDC", "ETH", "XLM"];
    let before = books(&t, &assets, &parties);

    let swap = build_aggregator_swap(&t, "ETH", "USDC", 0, 4_500 * USDC);
    let res = multiply(
        &t,
        &alice,
        "USDC",
        "ETH",
        ETH,
        &swap,
        Some(("XLM", 10_000_000)),
        None,
    );

    assert_contract_error(res, errors::CONVERT_STEPS_REQUIRED);
    assert_eq!(books(&t, &assets, &parties), before);
    assert!(!t.account_exists(1));
}

/// A router that pulls 1 raw unit less than it was given leaves that unit on the
/// controller. The controller refunds it to the caller on both legs: 1 raw XLM
/// from the third-asset convert and 1 raw ETH from the borrowed-debt swap. The
/// collateral is the 1 USDC convert output plus the 4,500 USDC main output.
#[test]
fn rv_multiply_router_shortfall_refunds_exact_leftover() {
    let mut t = LendingTest::new()
        .standard_two_asset()
        .with_market(xlm_preset())
        .build();
    let alice = t.get_or_create_user(ALICE);
    let router = install_route(
        &t,
        Route {
            shortfall: 1,
            overshoot: 0,
            pay_output: true,
        },
    );
    fund(&t, "USDC", &router, 4_501 * USDC);
    fund(&t, "XLM", &alice, 10_000_000);

    let main = build_aggregator_swap(&t, "ETH", "USDC", 0, 4_500 * USDC);
    let convert = build_aggregator_swap(&t, "XLM", "USDC", 0, USDC);
    let id = multiply(
        &t,
        &alice,
        "USDC",
        "ETH",
        ETH,
        &main,
        Some(("XLM", 10_000_000)),
        Some(convert),
    )
    .expect("shortfall is refunded, not rejected");

    assert_eq!(
        bal(&t, &t.resolve_asset("XLM"), &alice),
        1,
        "10,000,000 paid, 1 raw refunded"
    );
    assert_eq!(
        bal(&t, &t.resolve_asset("ETH"), &alice),
        1,
        "ETH leftover of the borrowed swap refunded"
    );
    assert_eq!(
        t.supply_balance_raw_for(id, "USDC"),
        4_501 * USDC,
        "1 USDC convert + 4,500 USDC main output"
    );
    assert_eq!(bal(&t, &t.resolve_asset("XLM"), &router), 9_999_999);
    assert_eq!(
        bal(&t, &t.resolve_asset("ETH"), &router),
        9_990_999,
        "router pulled amount_received - 1"
    );
    for name in ["XLM", "ETH", "USDC"] {
        assert_eq!(
            bal(&t, &t.resolve_asset(name), &t.controller),
            0,
            "controller keeps no {name}"
        );
    }
}

/// Router output and spend failures revert with no balance movement. `NoSwapOutput`
/// comes from the harness's zero-output mode. `RouterOverspend` comes from its
/// double-pull mode, which needs spare input on the controller so the SAC balance
/// check does not fire first.
#[test]
fn rv_multiply_router_zero_output_and_overspend_revert() {
    let mut t = LendingTest::new().standard_two_asset().build();
    let alice = t.get_or_create_user(ALICE);
    let admin = t.admin.clone();
    let bad = t
        .env
        .register(BadAggregator, (admin.clone(), BadMode::OutputShortfall));
    t.ctrl_client().set_swap_aggregator(&bad);
    fund(&t, "USDC", &bad, 4_500 * USDC);
    fund(&t, "ETH", &t.controller, 20_000_000);
    let parties = [
        ("alice", alice.clone()),
        ("controller", t.controller.clone()),
        ("router", bad.clone()),
    ];
    let before = books(&t, &["USDC", "ETH"], &parties);
    let swap = build_aggregator_swap(&t, "ETH", "USDC", 0, 4_500 * USDC);

    let zero = multiply(&t, &alice, "USDC", "ETH", ETH, &swap, None, None);
    assert_contract_error(zero, errors::NO_SWAP_OUTPUT);
    assert_eq!(
        books(&t, &["USDC", "ETH"], &parties),
        before,
        "zero-output revert is total"
    );

    let overspend = t.env.register(BadAggregator, (admin, BadMode::OverPull));
    t.ctrl_client().set_swap_aggregator(&overspend);
    fund(&t, "USDC", &overspend, 4_500 * USDC);
    let parties = [
        ("alice", alice.clone()),
        ("controller", t.controller.clone()),
        ("router", overspend.clone()),
    ];
    let before = books(&t, &["USDC", "ETH"], &parties);
    let res = multiply(&t, &alice, "USDC", "ETH", ETH, &swap, None, None);
    assert_contract_error(res, errors::ROUTER_OVERSPEND);
    assert_eq!(
        books(&t, &["USDC", "ETH"], &parties),
        before,
        "overspend revert is total"
    );
    assert!(!t.account_exists(1));
}

// ---------------------------------------------------------------------------
// H6: swap_collateral with the same token on a second hub.
// ---------------------------------------------------------------------------

/// The same token on another hub is a passthrough only with an empty route. An
/// empty route moves exactly the requested amount to the new hub and leaves the
/// rest on the old one. A non-empty route is rejected with `InvalidPayments`
/// and leaves both hubs as they were.
#[test]
fn rv_swap_collateral_same_token_cross_hub_partial_and_swap_rejected() {
    let mut t = LendingTest::new().standard_two_asset().build();
    let hub2 = t.create_hub();
    t.list_market_on_hub(hub2, "USDC", 0.0);
    let acct = t.supply_on_hub(HARNESS_HUB, ALICE, "USDC", 1_000.0);
    let alice = t.get_or_create_user(ALICE);
    let current = key(&t, "USDC");
    let new = HubAssetKey {
        hub_id: hub2,
        asset: t.resolve_asset("USDC"),
    };
    let ctrl = t.ctrl_client();
    let usdc = t.resolve_asset("USDC");

    let non_empty = build_aggregator_swap(&t, "USDC", "USDC", 0, 1);
    let rejected =
        flatten(ctrl.try_swap_collateral(&alice, &acct, &current, &(400 * USDC), &new, &non_empty));
    assert_contract_error(rejected, errors::INVALID_PAYMENTS);
    assert_eq!(
        ctrl.get_collateral_amount(&acct, &current),
        1_000 * USDC,
        "hub 1 untouched by the rejected route"
    );
    assert_eq!(ctrl.get_collateral_amount(&acct, &new), 0);

    let empty = Bytes::new(&t.env);
    let wallet_before = bal(&t, &usdc, &alice);
    ctrl.swap_collateral(&alice, &acct, &current, &(400 * USDC), &new, &empty);
    assert_eq!(
        ctrl.get_collateral_amount(&acct, &current),
        600 * USDC,
        "600 USDC stays on hub 1"
    );
    assert_eq!(
        ctrl.get_collateral_amount(&acct, &new),
        400 * USDC,
        "400 USDC moves to hub 2"
    );
    assert_eq!(
        bal(&t, &usdc, &alice),
        wallet_before,
        "no token leaves the wallet"
    );
    assert_eq!(
        bal(&t, &usdc, &t.controller),
        0,
        "controller holds no USDC after the move"
    );
}

// ---------------------------------------------------------------------------
// H7: repay_debt_with_collateral, same-market netting.
// ---------------------------------------------------------------------------

/// Same-market netting settles `min(amount, supply_floor, debt_ceil)`: 5 ETH offered
/// against 2 ETH supplied and 1 ETH owed settles 1 ETH. The debt is burned in full
/// and the supply drops by 1 ETH with no token movement. A non-empty route is
/// rejected with `InvalidPayments`.
#[test]
fn rv_repay_with_collateral_netting_clamps_and_burns_both_sides() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(ALICE, "ETH", 2.0);
    t.borrow(ALICE, "ETH", 1.0);
    let acct = t.resolve_account_id(ALICE);
    let alice = t.get_or_create_user(ALICE);
    let eth = key(&t, "ETH");
    let parties = [
        ("alice", alice.clone()),
        ("controller", t.controller.clone()),
    ];
    let before = books(&t, &["ETH"], &parties);

    let nonempty = build_aggregator_swap(&t, "ETH", "ETH", 0, 1);
    let rejected = flatten(t.ctrl_client().try_repay_debt_with_collateral(
        &alice,
        &acct,
        &eth,
        &(5 * ETH),
        &eth,
        &nonempty,
        &false,
    ));
    assert_contract_error(rejected, errors::INVALID_PAYMENTS);
    assert_eq!(books(&t, &["ETH"], &parties), before);

    let empty = Bytes::new(&t.env);
    t.ctrl_client().repay_debt_with_collateral(
        &alice,
        &acct,
        &eth,
        &(5 * ETH),
        &eth,
        &empty,
        &false,
    );

    assert_eq!(
        t.supply_balance_raw_for(acct, "ETH"),
        ETH,
        "supply 2 - settled 1 = 1 ETH"
    );
    assert_eq!(
        t.borrow_balance_raw_for(acct, "ETH"),
        0,
        "debt fully burned"
    );
    let after = books(&t, &["ETH"], &parties);
    let delta = |k: &str| after[k] - before[k];
    assert_eq!(delta("pool.ETH.supplied"), -ETH);
    assert_eq!(delta("pool.ETH.borrowed"), -ETH);
    assert_eq!(delta("pool.ETH.tokens"), 0, "netting moves no tokens");
    assert_eq!(delta("alice.ETH"), 0);
    assert_eq!(delta("pool.ETH.cash"), 0);
}

/// Netting that consumes the whole ETH supply removes that leg. A close then
/// pays every remaining supply position (here 4,000 USDC) to the caller and removes
/// the account. A close while USDC debt remains is refused with
/// `CannotCloseWithRemainingDebt` and changes nothing.
#[test]
fn rv_repay_with_collateral_netting_close_pays_residual() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(EVE, "ETH", 100.0);
    t.supply(ALICE, "USDC", 4_000.0);
    t.supply(ALICE, "ETH", 1.0);
    t.borrow(ALICE, "ETH", 1.0);
    t.borrow(ALICE, "USDC", 100.0);
    let acct = t.resolve_account_id(ALICE);
    let alice = t.get_or_create_user(ALICE);
    let eth = key(&t, "ETH");
    let empty = Bytes::new(&t.env);
    let parties = [
        ("alice", alice.clone()),
        ("controller", t.controller.clone()),
    ];
    let before = books(&t, &["USDC", "ETH"], &parties);

    let blocked = t.try_repay_debt_with_collateral("alice", "ETH", 1.0, "ETH", &empty, true);
    assert_contract_error(blocked, errors::CANNOT_CLOSE_WITH_REMAINING_DEBT);
    assert_eq!(
        books(&t, &["USDC", "ETH"], &parties),
        before,
        "refused close changes nothing"
    );

    t.repay(ALICE, "USDC", 100.0);
    let before = books(&t, &["USDC", "ETH"], &parties);
    t.ctrl_client()
        .repay_debt_with_collateral(&alice, &acct, &eth, &ETH, &eth, &empty, &true);

    assert!(!t.account_exists(acct), "close removes the emptied account");
    let after = books(&t, &["USDC", "ETH"], &parties);
    let delta = |k: &str| after[k] - before[k];
    assert_eq!(
        delta("alice.USDC"),
        4_000 * USDC,
        "residual 4,000 USDC paid to the caller"
    );
    assert_eq!(delta("alice.ETH"), 0, "netted ETH leg moves no tokens");
    assert_eq!(delta("pool.USDC.supplied"), -4_000 * USDC);
    assert_eq!(delta("pool.ETH.supplied"), -ETH);
    assert_eq!(delta("pool.ETH.borrowed"), -ETH);
}

// ---------------------------------------------------------------------------
// H8: INV-HALT-01 pause matrix.
// ---------------------------------------------------------------------------

/// Every entry that INV-HALT-01 lists as blocked by the global pause returns
/// `EnforcedPause` (1000), including the ten the older security test does not
/// cover. Pool figures do not move.
#[test]
fn rv_pause_blocks_every_entry_listed_in_inv_halt_01() {
    let mut t = LendingTest::new().standard_two_asset_dust_disabled();
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 1.0);
    let acct = t.resolve_account_id(ALICE);
    let alice = t.get_or_create_user(ALICE);
    let keeper = t.keeper.clone();
    let admin = t.admin.clone();
    let receiver = t.deploy_flash_position_receiver();
    let loan_receiver = t.deploy_flash_loan_receiver();
    let eth = key(&t, "ETH");
    let usdc = key(&t, "USDC");
    let empty = Bytes::new(&t.env);
    let parties = [
        ("alice", alice.clone()),
        ("controller", t.controller.clone()),
    ];
    let before = books(&t, &["USDC", "ETH"], &parties);
    let carol_addr = t.get_or_create_user(CAROL);
    t.pause();
    let ctrl = t.ctrl_client();

    let no_refunds: Vec<Address> = Vec::new(&t.env);
    let one_usdc = mins(&t, &[("USDC", 1)]);
    let one = Vec::from_array(&t.env, [(usdc.clone(), 1i128)]);
    let mut ids = Vec::new(&t.env);
    ids.push_back(acct);

    assert_contract_error(
        flatten(ctrl.try_supply(&alice, &acct, &HARNESS_SPOKE, &one)),
        errors::CONTRACT_PAUSED,
    );
    assert_contract_error(
        flatten(ctrl.try_borrow(&alice, &acct, &one, &None)),
        errors::CONTRACT_PAUSED,
    );
    assert_contract_error(
        flatten(ctrl.try_flash_loan(&alice, &eth, &1, &loan_receiver, &empty)),
        errors::CONTRACT_PAUSED,
    );
    assert_contract_error(
        flash(
            &t,
            &alice,
            acct,
            "ETH",
            1,
            &receiver,
            &empty,
            &one_usdc,
            &no_refunds,
        ),
        errors::CONTRACT_PAUSED,
    );
    assert_contract_error(
        multiply(&t, &alice, "USDC", "ETH", 1, &empty, None, None),
        errors::CONTRACT_PAUSED,
    );
    assert_contract_error(
        flatten(ctrl.try_swap_debt(&alice, &acct, &eth, &1, &usdc, &empty)),
        errors::CONTRACT_PAUSED,
    );
    assert_contract_error(
        flatten(ctrl.try_swap_collateral(&alice, &acct, &usdc, &1, &eth, &empty)),
        errors::CONTRACT_PAUSED,
    );
    assert_contract_error(
        flatten(
            ctrl.try_repay_debt_with_collateral(&alice, &acct, &usdc, &1, &eth, &empty, &false),
        ),
        errors::CONTRACT_PAUSED,
    );
    let blend = Address::generate(&t.env);
    assert_contract_error(
        flatten(ctrl.try_migrate_from_blend(
            &alice,
            &0u64,
            &HARNESS_SPOKE,
            &HARNESS_HUB,
            &blend,
            &Vec::new(&t.env),
            &Vec::new(&t.env),
            &Vec::new(&t.env),
        )),
        errors::CONTRACT_PAUSED,
    );
    let eth_only = Vec::from_array(&t.env, [eth.clone()]);
    assert_contract_error(
        flatten(ctrl.try_update_indexes(&keeper, &eth_only)),
        errors::CONTRACT_PAUSED,
    );
    assert_contract_error(
        flatten(ctrl.try_claim_revenue(&admin, &eth_only)),
        errors::CONTRACT_PAUSED,
    );
    assert_contract_error(
        flatten(ctrl.try_update_account_threshold(&keeper, &false, &ids)),
        errors::CONTRACT_PAUSED,
    );
    assert_contract_error(
        flatten(ctrl.try_add_delegate(&alice, &acct, &carol_addr)),
        errors::CONTRACT_PAUSED,
    );

    assert_eq!(
        books(&t, &["USDC", "ETH"], &parties),
        before,
        "blocked entries move nothing"
    );
}

/// Exits stay open under the same pause: clean bad debt on an insolvent account,
/// recapitalization (which refunds the whole payment when no shortfall exists),
/// account renewal, and delegate revocation. Revocation takes effect at once.
#[test]
fn rv_pause_keeps_bad_debt_recap_renew_revoke_open() {
    let mut t = LendingTest::new().standard_two_asset_dust_disabled();
    // Alice: a delegate on her account, plus a live USDC position.
    t.supply(ALICE, "USDC", 1_000.0);
    let alice_acct = t.resolve_account_id(ALICE);
    t.enable_delegate(ALICE, BOB, alice_acct);
    let alice = t.get_or_create_user(ALICE);
    let bob = t.get_or_create_user(BOB);
    // Carol: 10 USDC against 0.003 ETH, which becomes bad debt when USDC falls to $0.01.
    t.supply(CAROL, "USDC", 10.0);
    t.borrow(CAROL, "ETH", 0.003);
    let carol_acct = t.resolve_account_id(CAROL);
    t.set_price("USDC", test_harness::usd_cents(1));
    assert!(
        t.can_be_liquidated(CAROL),
        "precondition: carol is underwater"
    );
    let dave = t.get_or_create_user(DAVE);
    fund(&t, "ETH", &dave, ETH);

    t.pause();
    let parties = [
        ("alice", alice.clone()),
        ("dave", dave.clone()),
        ("controller", t.controller.clone()),
    ];
    let before = books(&t, &["USDC", "ETH"], &parties);

    t.try_clean_bad_debt_by_id(carol_acct)
        .expect("bad-debt cleanup stays open");
    t.assert_no_positions(CAROL);

    let ctrl = t.ctrl_client();
    let refunded = flatten(ctrl.try_recapitalize(&dave, &key(&t, "ETH"), &ETH))
        .expect("recapitalize stays open");
    assert_eq!(refunded, 0, "no shortfall: nothing is credited");
    assert_eq!(
        bal(&t, &t.resolve_asset("ETH"), &dave),
        ETH,
        "payment refunded in full"
    );

    flatten(ctrl.try_renew_account(&alice, &alice_acct)).expect("renew stays open");

    flatten(ctrl.try_remove_delegate(&alice, &alice_acct, &bob)).expect("revocation stays open");
    let bob_withdraw = flatten(ctrl.try_withdraw(
        &bob,
        &alice_acct,
        &Vec::from_array(&t.env, [(key(&t, "USDC"), 1i128)]),
        &None,
    ));
    assert_contract_error(bob_withdraw, errors::NOT_AUTHORIZED);

    let after = books(&t, &["USDC", "ETH"], &parties);
    assert_eq!(
        after["alice.USDC"], before["alice.USDC"],
        "no payout to the owner"
    );
    assert_eq!(after["pool.USDC.tokens"], before["pool.USDC.tokens"]);
}

// ---------------------------------------------------------------------------
// H9: re-entry from a router and from a flash-position callback.
// ---------------------------------------------------------------------------

/// A router that re-enters the controller is refused by the host with
/// `Error(Context, InvalidAction)` for each mode and each swap strategy. The
/// flash guard clears and no balance moves. Multiply's four modes are covered in
/// strategy/adversarial.rs.
#[test]
fn rv_strategy_router_reentry_all_modes_revert_clean() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);
    let acct = t.resolve_account_id(ALICE);
    let alice = t.get_or_create_user(ALICE);
    let usdc = key(&t, "USDC");
    let eth = key(&t, "ETH");
    let steps = build_aggregator_swap(&t, "USDC", "ETH", 0, ETH / 2);

    for mode in [
        ReenterMode::Supply,
        ReenterMode::Borrow,
        ReenterMode::FlashLoan,
        ReenterMode::Panic,
    ] {
        let router = t
            .env
            .register(ReenteringAggregator, (t.controller.clone(), mode));
        t.ctrl_client().set_swap_aggregator(&router);
        let parties = [
            ("alice", alice.clone()),
            ("controller", t.controller.clone()),
            ("router", router.clone()),
        ];
        let before = books(&t, &["USDC", "ETH"], &parties);

        let swap_debt = flatten(t.ctrl_client().try_swap_debt(
            &alice,
            &acct,
            &eth,
            &(1_000 * USDC),
            &usdc,
            &steps,
        ));
        assert_eq!(
            format!("{:?}", swap_debt.unwrap_err()),
            HOST_REENTRY,
            "swap_debt via {mode:?}"
        );
        assert_eq!(
            books(&t, &["USDC", "ETH"], &parties),
            before,
            "swap_debt {mode:?} moves nothing"
        );

        let swap_col = flatten(t.ctrl_client().try_swap_collateral(
            &alice,
            &acct,
            &usdc,
            &(1_000 * USDC),
            &eth,
            &steps,
        ));
        assert_eq!(
            format!("{:?}", swap_col.unwrap_err()),
            HOST_REENTRY,
            "swap_collateral via {mode:?}"
        );
        assert_eq!(
            books(&t, &["USDC", "ETH"], &parties),
            before,
            "swap_collateral {mode:?} moves nothing"
        );

        let rdwc = flatten(t.ctrl_client().try_repay_debt_with_collateral(
            &alice,
            &acct,
            &usdc,
            &(1_000 * USDC),
            &eth,
            &steps,
            &false,
        ));
        assert_eq!(
            format!("{:?}", rdwc.unwrap_err()),
            HOST_REENTRY,
            "repay_debt_with_collateral via {mode:?}"
        );
        assert_eq!(
            books(&t, &["USDC", "ETH"], &parties),
            before,
            "rdwc {mode:?} moves nothing"
        );
    }
}

/// A flash-position receiver that calls the controller from its callback is refused
/// by the host (`Error(Context, InvalidAction)`) before the flash guard (#400) is
/// reached. In this host the guard fired only under the test-only injection in
/// strategy/flash_position.rs, so the host check is the operative barrier here. The
/// attempted re-entry leaves every balance and the guard unchanged.
#[test]
fn rv_flash_position_callback_reentry_host_refused() {
    let mut t = LendingTest::new().standard_two_asset().build();
    let alice = t.get_or_create_user(ALICE);
    let parties = [
        ("alice", alice.clone()),
        ("controller", t.controller.clone()),
    ];
    let before = books(&t, &["USDC", "ETH"], &parties);

    for mode in [
        FlashPositionMode::ReenterSupply,
        FlashPositionMode::ReenterBorrow,
        FlashPositionMode::ReenterFlashLoan,
        FlashPositionMode::ReenterFlashPosition,
    ] {
        let receiver = t.deploy_flash_position_receiver();
        let data = payload(&t, mode, "USDC", 4_000 * USDC, "ETH", 0);
        let res = flash(
            &t,
            &alice,
            0,
            "ETH",
            ETH,
            &receiver,
            &data,
            &mins(&t, &[("USDC", 4_000 * USDC)]),
            &refunds(&t, &[]),
        );
        assert_eq!(format!("{:?}", res.unwrap_err()), HOST_REENTRY, "{mode:?}");
        assert_eq!(
            books(&t, &["USDC", "ETH"], &parties),
            before,
            "{mode:?} moves nothing"
        );
    }
    assert!(!t.account_exists(1));
}

// ---------------------------------------------------------------------------
// H10: an active delegate closes the account.
// ---------------------------------------------------------------------------

/// A delegate who closes with `repay_debt_with_collateral(close_position = true)`
/// receives the residual collateral. The owner receives none. A delegate can already
/// withdraw collateral to itself, subject to health, so the payout adds no privilege.
#[test]
fn rv_repay_with_collateral_delegate_close_pays_delegate() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 1.0);
    let acct = t.resolve_account_id(ALICE);
    t.enable_delegate(ALICE, BOB, acct);
    let alice = t.get_or_create_user(ALICE);
    let bob = t.get_or_create_user(BOB);
    fund(&t, "ETH", &t.aggregator, ETH);
    let steps = build_aggregator_swap(&t, "USDC", "ETH", 0, ETH);
    let parties = [
        ("alice", alice.clone()),
        ("bob", bob.clone()),
        ("controller", t.controller.clone()),
    ];
    let before = books(&t, &["USDC", "ETH"], &parties);

    t.ctrl_client().repay_debt_with_collateral(
        &bob,
        &acct,
        &key(&t, "USDC"),
        &(4_000 * USDC),
        &key(&t, "ETH"),
        &steps,
        &true,
    );

    assert!(!t.account_exists(acct), "close removes the account");
    let after = books(&t, &["USDC", "ETH"], &parties);
    let delta = |k: &str| after[k] - before[k];
    assert_eq!(
        delta("bob.USDC"),
        6_000 * USDC,
        "delegate receives the 6,000 USDC residual"
    );
    assert_eq!(delta("alice.USDC"), 0, "owner receives nothing");
    assert_eq!(delta("bob.ETH"), 0);
    assert_eq!(delta("alice.ETH"), 0);
    assert_eq!(delta("pool.ETH.borrowed"), -ETH);
    assert_eq!(delta("pool.USDC.supplied"), -10_000 * USDC);
}

/// A delegate's withdraw without a recipient pays the delegate. Close-by-repay is
/// therefore no wider a trust grant than withdraw: both pay the caller.
#[test]
fn rv_withdraw_by_delegate_pays_delegate_not_owner() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(ALICE, "USDC", 1_000.0);
    let acct = t.resolve_account_id(ALICE);
    t.enable_delegate(ALICE, BOB, acct);
    let alice = t.get_or_create_user(ALICE);
    let bob = t.get_or_create_user(BOB);
    let parties = [("alice", alice.clone()), ("bob", bob.clone())];
    let before = books(&t, &["USDC"], &parties);

    let usdc = key(&t, "USDC");
    let paid = t.ctrl_client().withdraw(
        &bob,
        &acct,
        &Vec::from_array(&t.env, [(usdc, 400 * USDC)]),
        &None,
    );
    assert_eq!(
        paid.get(0).unwrap().1,
        400 * USDC,
        "pool pays the requested 400 USDC"
    );

    let after = books(&t, &["USDC"], &parties);
    let delta = |k: &str| after[k] - before[k];
    assert_eq!(
        delta("bob.USDC"),
        400 * USDC,
        "delegate receives the withdrawal"
    );
    assert_eq!(delta("alice.USDC"), 0, "owner receives nothing");
    assert_eq!(
        t.supply_balance_raw_for(acct, "USDC"),
        600 * USDC,
        "600 USDC remains"
    );
}

// ---------------------------------------------------------------------------
// H11: no reusable router authority after a strategy.
// ---------------------------------------------------------------------------

/// After each successful strategy the controller holds no residue of the
/// input token, the controller's allowance to the router is zero, and a
/// router-initiated `transfer_from` of spare controller funds fails.
#[test]
fn rv_strategy_router_no_residue_no_reusable_allowance() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(EVE, "ETH", 100.0);
    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);
    let acct = t.resolve_account_id(ALICE);
    let alice = t.get_or_create_user(ALICE);
    let router = t.aggregator.clone();
    t.fund_router("ETH", 2.0);
    t.fund_router("USDC", 100_000.0);

    let steps_usdc_to_eth = build_aggregator_swap(&t, "USDC", "ETH", 0, ETH / 2);
    let controller = t.controller.clone();

    t.ctrl_client().swap_debt(
        &alice,
        &acct,
        &key(&t, "ETH"),
        &(1_000 * USDC),
        &key(&t, "USDC"),
        &steps_usdc_to_eth,
    );
    check_no_residue_no_pull(&t, "USDC", &controller, &router, "swap_debt");

    t.ctrl_client().swap_collateral(
        &alice,
        &acct,
        &key(&t, "USDC"),
        &(1_000 * USDC),
        &key(&t, "ETH"),
        &steps_usdc_to_eth,
    );
    check_no_residue_no_pull(&t, "USDC", &controller, &router, "swap_collateral");

    t.ctrl_client().repay_debt_with_collateral(
        &alice,
        &acct,
        &key(&t, "USDC"),
        &(1_000 * USDC),
        &key(&t, "ETH"),
        &steps_usdc_to_eth,
        &false,
    );
    check_no_residue_no_pull(
        &t,
        "USDC",
        &controller,
        &router,
        "repay_debt_with_collateral",
    );

    let steps_eth_to_usdc = build_aggregator_swap(&t, "ETH", "USDC", 0, 3_000 * USDC);
    let opened = multiply(
        &t,
        &alice,
        "USDC",
        "ETH",
        ETH,
        &steps_eth_to_usdc,
        None,
        None,
    );
    opened.expect("multiply on the same router");
    check_no_residue_no_pull(&t, "ETH", &controller, &router, "multiply");
}

/// RV-FINDING. `multiply` rejects Long mode with debt and collateral in the same market
/// (`AssetsAreTheSame`, multiply.rs `validate_multiply_request`), and docs/reference/errors.md
/// lists `AssetsAreTheSame` for prohibited same-market pairs. `flash_position` has no such
/// check, so the same pair opens a Long account. Expected: rejection, as multiply does.
#[test]
#[ignore = "RV-FINDING: flash_position Long mode accepts debt and collateral in the same market"]
fn rv_finding_flash_position_long_same_market_should_reject() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(BOB, "ETH", 100.0);
    let alice = t.get_or_create_user(ALICE);

    // Control: multiply in Long mode rejects the same market before any transfer.
    let empty = Bytes::new(&t.env);
    let control = flatten(t.ctrl_client().try_multiply(
        &alice,
        &0u64,
        &HARNESS_SPOKE,
        &key(&t, "ETH"),
        &ETH,
        &key(&t, "ETH"),
        &PositionMode::Long,
        &empty,
        &None,
        &None,
    ));
    assert_contract_error(control, errors::ASSETS_ARE_THE_SAME);

    // flash_position: the receiver pushes 4,000 USDC and returns the 1 ETH it borrowed.
    let receiver = t.deploy_flash_position_receiver();
    let data = payload(
        &t,
        FlashPositionMode::SupplyAndReturnDebt,
        "USDC",
        4_000 * USDC,
        "ETH",
        0,
    );
    let collaterals = Vec::from_array(
        &t.env,
        [(key(&t, "USDC"), 4_000 * USDC), (key(&t, "ETH"), 0)],
    );
    let res = flash_mode(
        &t,
        &alice,
        0,
        PositionMode::Long,
        &key(&t, "ETH"),
        ETH,
        &receiver,
        &data,
        &collaterals,
        &refunds(&t, &[]),
    );
    assert_contract_error(res, errors::ASSETS_ARE_THE_SAME);
}

/// RV-FINDING. docs/reference/errors.md states that Long and Short "also reject the same
/// underlying token across hubs". `multiply` in Long mode does so (checked below). The
/// `flash_position` path does not: the same token as debt on hub 1 and collateral on hub 2
/// opens a Long account.
#[test]
#[ignore = "RV-FINDING: flash_position Long mode accepts one token as debt and collateral across hubs"]
fn rv_finding_flash_position_long_cross_hub_token_should_reject() {
    let mut t = LendingTest::new().standard_two_asset().build();
    let hub2 = t.create_hub();
    t.list_market_on_hub(hub2, "ETH", 0.0);
    t.supply(BOB, "ETH", 100.0);
    let alice = t.get_or_create_user(ALICE);
    let hub2_eth = HubAssetKey {
        hub_id: hub2,
        asset: t.resolve_asset("ETH"),
    };

    // Control: multiply in Long mode rejects one token across two hubs.
    let empty = Bytes::new(&t.env);
    let control = flatten(t.ctrl_client().try_multiply(
        &alice,
        &0u64,
        &HARNESS_SPOKE,
        &hub2_eth,
        &ETH,
        &key(&t, "ETH"),
        &PositionMode::Long,
        &empty,
        &None,
        &None,
    ));
    assert_contract_error(control, errors::ASSETS_ARE_THE_SAME);

    // flash_position: debt 1 ETH on hub 1; the receiver pushes 4,000 USDC and returns the
    // 1 ETH, which is declared as hub-2 collateral.
    let receiver = t.deploy_flash_position_receiver();
    let data = payload(
        &t,
        FlashPositionMode::SupplyAndReturnDebt,
        "USDC",
        4_000 * USDC,
        "ETH",
        0,
    );
    let collaterals = Vec::from_array(&t.env, [(key(&t, "USDC"), 4_000 * USDC), (hub2_eth, ETH)]);
    let res = flash_mode(
        &t,
        &alice,
        0,
        PositionMode::Long,
        &key(&t, "ETH"),
        ETH,
        &receiver,
        &data,
        &collaterals,
        &refunds(&t, &[]),
    );
    assert_contract_error(res, errors::ASSETS_ARE_THE_SAME);
}

fn check_no_residue_no_pull(
    t: &LendingTest,
    asset_name: &str,
    controller: &Address,
    router: &Address,
    label: &str,
) {
    let asset = t.resolve_asset(asset_name);
    let tok = token::Client::new(&t.env, &asset);
    assert!(
        tok.balance(controller) <= 4,
        "{label}: controller holds {} base units of {asset_name}, bound is 4",
        tok.balance(controller)
    );
    assert_eq!(
        tok.allowance(controller, router),
        0,
        "{label}: no allowance to the router"
    );

    // Spare funds on the controller that no strategy authorized. The router must not
    // be able to pull them with its own call.
    fund(t, asset_name, controller, 1_000);
    let before = tok.balance(controller);
    // SAC code 9 is insufficient allowance; code 10 is insufficient balance.
    let pulled = flatten(tok.try_transfer_from(router, controller, router, &1_000));
    assert_eq!(
        pulled,
        Err(soroban_sdk::Error::from_contract_error(9)),
        "{label}: router pulled spare {asset_name} without allowance"
    );
    assert_eq!(
        tok.balance(controller),
        before,
        "{label}: controller balance unchanged by the refused pull"
    );
    tok.burn(controller, &1_000);
}
