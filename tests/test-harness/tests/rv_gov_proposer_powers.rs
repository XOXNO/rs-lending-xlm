//! RV break pass: the documented blast radius of a stolen NON-OWNER PROPOSER
//! key once the timelock delay elapses (12 ledgers on the configured networks).
//!
//! Each test grants `mallory` only the proposer role, drives one operation
//! through `propose` and a permissionless `execute`, and asserts the resulting
//! damage with concrete numbers. These are characterization tests of the trust
//! model in docs/explanation/threat-model.md, not safe expectations: if one of
//! them starts failing, the proposer's powers changed and the threat model
//! must be updated with it.

use controller::constants::{RAY, WAD};
use controller::types::{InterestRateModel, SpokeAssetArgs};
use governance_interface::{
    AdminOperation, RoleArgs, SpokeLiquidationCurveArgs, UpgradePoolParamsArgs,
};
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{vec, Address, BytesN, IntoVal, Symbol, Val, Vec};
use test_harness::{
    assert_contract_error, errors, hub_asset, usd, usd_cents, wad_to_f64, LendingTest, ALICE,
    HARNESS_HUB, HARNESS_SPOKE, UNCONSTRAINED_TEST_CAP,
};

fn salt(env: &soroban_sdk::Env, byte: u8) -> BytesN<32> {
    BytesN::<32>::from_array(env, &[byte; 32])
}

fn zero(env: &soroban_sdk::Env) -> BytesN<32> {
    BytesN::<32>::from_array(env, &[0u8; 32])
}

fn flatten<T, C>(
    r: Result<Result<T, C>, Result<soroban_sdk::Error, soroban_sdk::InvokeError>>,
) -> Result<(), soroban_sdk::Error> {
    match r {
        Ok(_) => Ok(()),
        Err(Ok(e)) => Err(e),
        Err(Err(e)) => panic!("unexpected InvokeError {e:?}"),
    }
}

/// Grants PROPOSER only (owner stays `admin`). Returns the proposer address.
fn grant_proposer(t: &LendingTest) -> Address {
    let mallory = Address::generate(&t.env);
    t.gov_client().execute_immediate(
        &t.admin(),
        &AdminOperation::GrantGovRole(RoleArgs {
            account: mallory.clone(),
            role: Symbol::new(&t.env, "PROPOSER"),
        }),
    );
    assert!(t
        .gov_iface_client()
        .has_role(&mallory, &Symbol::new(&t.env, "PROPOSER")));
    assert_ne!(mallory, t.admin());
    mallory
}

/// Non-owner proposer schedules `op`, the delay passes, anyone executes it
/// against the controller with the raw `(function, args)` tuple.
fn run_controller_op(
    t: &LendingTest,
    proposer: &Address,
    op: &AdminOperation,
    function: &str,
    args: Vec<Val>,
    byte: u8,
) -> Result<(), soroban_sdk::Error> {
    let s = salt(&t.env, byte);
    t.gov_iface_client().propose(proposer, op, &s);
    let delay = t.gov_iface_client().get_min_delay();
    // Sensitive floor is 12 < harness delay 50, so `delay` covers both tiers.
    t.env.ledger().with_mut(|l| l.sequence_number += delay);
    // The ledger jump expires the mock feed's temporary entries (NoLastPrice).
    t.refresh_oracle_prices();
    flatten(t.gov_iface_client().try_execute(
        &None,
        &t.controller,
        &Symbol::new(&t.env, function),
        &args,
        &zero(&t.env),
        &s,
    ))
}

fn eth_args(t: &LendingTest, ltv: u32, threshold: u32, bonus: u32, fees: u32) -> SpokeAssetArgs {
    SpokeAssetArgs {
        hub_id: HARNESS_HUB,
        asset: t.resolve_asset("ETH"),
        spoke_id: HARNESS_SPOKE,
        can_collateral: true,
        can_borrow: true,
        paused: false,
        frozen: false,
        no_seize: false,
        ltv,
        threshold,
        bonus,
        liquidation_fees: fees,
        supply_cap: UNCONSTRAINED_TEST_CAP,
        borrow_cap: UNCONSTRAINED_TEST_CAP,
    }
}

/// EditAssetInSpoke threshold ratchet. Alice: 10 ETH ($20k) collateral, 6000
/// USDC debt, HF 2.67 at the stamped 80% threshold. The proposer edits ETH to
/// (ltv 3100, thr 3200, bonus 21250, fees 0), the largest bonus
/// `validate_risk_bounds` admits at that threshold, then anyone restamps Alice
/// (gate passes: 20000*0.32/6000 = 1.0667 >= 1.05). The curve edit shows the
/// proposer reaches it too, but it is not load-bearing: on the
/// `threshold * (1 + bonus) == 1` boundary the HF-preserving bonus cap sits
/// below the stamped bonus, so every liquidation below HF 1 is a full close.
/// An 8% ETH dip (never liquidatable under the original stamp: HF 2.45) now
/// closes the whole debt and seizes essentially all 10 ETH.
#[test]
fn proposer_threshold_ratchet_and_curve_wipe_alice_equity() {
    let mut t = LendingTest::new().standard_two_asset().build();
    let mallory = grant_proposer(&t);

    t.supply(ALICE, "ETH", 10.0);
    t.borrow(ALICE, "USDC", 6000.0);
    let alice_id = t.resolve_account_id(ALICE);
    let hf0 = t.health_factor(ALICE);
    assert!(hf0 > 2.6 && hf0 < 2.7, "hf0 = {hf0}");

    // Bound check: 3200 * (10000 + 21250) == 10000 * 10000 exactly.
    let args = eth_args(&t, 3100, 3200, 21_250, 0);
    run_controller_op(
        &t,
        &mallory,
        &AdminOperation::EditAssetInSpoke(args.clone()),
        "edit_asset_in_spoke",
        vec![&t.env, args.into_val(&t.env)],
        1,
    )
    .expect("non-owner proposer executes EditAssetInSpoke");

    // Permissionless restamp: gate requires hypothetical HF >= 1.05.
    let keeper = Address::generate(&t.env);
    t.ctrl_client()
        .update_account_threshold(&keeper, &true, &vec![&t.env, alice_id]);
    let hf1 = t.health_factor(ALICE);
    assert!(hf1 > 1.06 && hf1 < 1.07, "hf1 = {hf1}");
    let (supplies, _) = t.ctrl_client().get_account_positions(&alice_id);
    let eth_pos = supplies.get(hub_asset(t.resolve_asset("ETH"))).unwrap();
    assert_eq!(eth_pos.liquidation_threshold, 3200);
    assert_eq!(eth_pos.liquidation_bonus, 21_250);
    assert_eq!(eth_pos.liquidation_fees, 0);

    // Curve: target 10 WAD, max bonus from 9.99.. WAD, factor 100%.
    let curve = SpokeLiquidationCurveArgs {
        spoke_id: HARNESS_SPOKE,
        target_hf_wad: 10 * WAD,
        hf_for_max_bonus_wad: 10 * WAD - 1,
        liquidation_bonus_factor_bps: 10_000,
    };
    run_controller_op(
        &t,
        &mallory,
        &AdminOperation::SetSpokeLiquidationCurve(curve.clone()),
        "set_spoke_liquidation_curve",
        vec![
            &t.env,
            curve.spoke_id.into_val(&t.env),
            curve.target_hf_wad.into_val(&t.env),
            curve.hf_for_max_bonus_wad.into_val(&t.env),
            curve.liquidation_bonus_factor_bps.into_val(&t.env),
        ],
        2,
    )
    .expect("non-owner proposer executes SetSpokeLiquidationCurve");

    // 8% dip. Original stamp would give HF = 18400*0.8/6000 = 2.45.
    t.set_price("ETH", usd(1840));
    let hf2 = t.health_factor(ALICE);
    assert!(hf2 < 1.0, "hf2 = {hf2}");

    let eth_before = t.supply_balance(ALICE, "ETH");
    let debt_before = t.borrow_balance(ALICE, "USDC");
    t.liquidate("mallory_liq", ALICE, "USDC", 6000.0);
    let eth_after = t.supply_balance_raw_for(alice_id, "ETH");
    let liq_eth = t.token_balance("mallory_liq", "ETH");
    let debt_after = t
        .find_account_id(ALICE)
        .map(|_| t.borrow_balance(ALICE, "USDC"))
        .unwrap_or(0.0);
    std::println!(
        "hf0={hf0} hf1={hf1} hf2={hf2} eth_before={eth_before} debt_before={debt_before} \
         eth_after_raw={eth_after} liquidator_eth={liq_eth} debt_after={debt_after}"
    );
    // Whole debt repaid, essentially all collateral gone to the liquidator.
    assert_eq!(debt_after, 0.0);
    assert!(liq_eth > 9.98, "liquidator got {liq_eth} ETH of 10");
    assert!(
        eth_after < 200_000,
        "alice keeps {eth_after} raw ETH (1e7 = 1 ETH)"
    );
}

/// ForceSocializeBadDebt is proposer-reachable. Alice: 10,000 USDC
/// collateral, 3 ETH ($6000) debt; USDC -> $0.50 makes C=$5000 < D=$6000.
/// A normal liquidation would repay min(D, C/(1+5%)) = $4761.9 and seize all
/// $5000, leaving $1238 to socialize. The forced path socializes the full
/// $6000 against ETH suppliers and books all 10,000 USDC as protocol revenue.
#[test]
fn proposer_force_socializes_merely_insolvent_account() {
    let mut t = LendingTest::new().standard_two_asset().build();
    let mallory = grant_proposer(&t);

    // Bob is the only ETH share holder (harness seed liquidity is bare cash).
    t.supply("bob", "ETH", 100.0);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    t.set_price("USDC", usd_cents(50));
    let alice_id = t.resolve_account_id(ALICE);
    let coll = t.total_collateral(ALICE);
    let debt = t.total_debt(ALICE);
    assert!(debt > coll, "debt {debt} must exceed collateral {coll}");

    let eth = hub_asset(t.resolve_asset("ETH"));
    let bob_eth_before = t.supply_balance("bob", "ETH");
    let eth_idx_before = t.ctrl_client().get_market_index(&eth).supply_index;
    let usdc_rev_before = t.snapshot_revenue("USDC");
    let eth_rev_before = t.snapshot_revenue("ETH");
    let alice_usdc_scaled = t
        .ctrl_client()
        .get_account_positions(&alice_id)
        .0
        .get(hub_asset(t.resolve_asset("USDC")))
        .unwrap()
        .scaled_amount;

    run_controller_op(
        &t,
        &mallory,
        &AdminOperation::ForceSocializeBadDebt(alice_id),
        "force_socialize_bad_debt",
        vec![&t.env, alice_id.into_val(&t.env)],
        3,
    )
    .expect("non-owner proposer executes ForceSocializeBadDebt");

    assert!(!t.account_exists(alice_id));
    let eth_idx_after = t.ctrl_client().get_market_index(&eth).supply_index;
    let usdc_rev_after = t.snapshot_revenue("USDC");
    let eth_rev_after = t.snapshot_revenue("ETH");
    let bob_eth_after = t.supply_balance("bob", "ETH");
    std::println!(
        "coll={coll} debt={debt} eth_supply_index {eth_idx_before} -> {eth_idx_after} \
         bob_eth {bob_eth_before} -> {bob_eth_after} \
         usdc_revenue {usdc_rev_before} -> {usdc_rev_after} (alice scaled {alice_usdc_scaled}) \
         eth_revenue {eth_rev_before} -> {eth_rev_after}"
    );
    assert!(eth_idx_after < eth_idx_before, "ETH suppliers written down");
    // Bob (100 ETH) eats the whole 3 ETH of debt: 97 ETH left.
    assert!(
        bob_eth_before - bob_eth_after > 2.99,
        "bob lost {}",
        bob_eth_before - bob_eth_after
    );
    // `get_revenue` is in asset units: all 10,000 USDC become protocol revenue.
    assert_eq!(usdc_rev_after - usdc_rev_before, 10_000 * 10i128.pow(7));
    assert_eq!(
        alice_usdc_scaled,
        10_000 * 10i128.pow(7) * (RAY / 10i128.pow(7))
    );
}

/// RemoveSpoke deprecates irreversibly (no AdminOperation re-opens a
/// spoke). After it, an indebted account cannot top up collateral and no new
/// account can be opened in the spoke; withdraw/repay/liquidate still work.
#[test]
fn proposer_deprecates_spoke_blocking_collateral_top_ups() {
    let mut t = LendingTest::new().standard_two_asset().build();
    let mallory = grant_proposer(&t);

    t.supply(ALICE, "ETH", 10.0);
    t.borrow(ALICE, "USDC", 6000.0);

    run_controller_op(
        &t,
        &mallory,
        &AdminOperation::RemoveSpoke(HARNESS_SPOKE),
        "remove_spoke",
        vec![&t.env, HARNESS_SPOKE.into_val(&t.env)],
        4,
    )
    .expect("non-owner proposer executes RemoveSpoke");
    assert!(t.ctrl_client().get_spoke(&HARNESS_SPOKE).is_deprecated);

    // Indebted Alice cannot add collateral to defend her position.
    assert_contract_error(
        t.try_supply(ALICE, "ETH", 1.0).map(|_| ()),
        errors::SPOKE_DEPRECATED,
    );
    // Nobody can open a new account in the spoke.
    assert_contract_error(
        t.try_supply("bob", "USDC", 100.0).map(|_| ()),
        errors::SPOKE_DEPRECATED,
    );
    // Repay and withdraw remain available (exit paths).
    t.repay(ALICE, "USDC", 6000.0);
    t.withdraw_all(ALICE, "ETH");
}

/// SetPositionManager(manager, true) by a proposer re-arms every delegate
/// grant stored under a manager the owner had deactivated. Grants survive
/// deactivation by design (the NFT owner's `remove_delegate` is the
/// immediate remedy); this pins that a proposer can flip the switch back.
#[test]
fn proposer_reactivates_deactivated_manager_and_revives_grants() {
    let mut t = LendingTest::new().standard_two_asset().build();
    let mallory = grant_proposer(&t);

    t.supply(ALICE, "ETH", 10.0);
    let alice_id = t.resolve_account_id(ALICE);
    t.enable_delegate(ALICE, "mgr", alice_id);
    let mgr = t.get_or_create_user("mgr");
    let eth = hub_asset(t.resolve_asset("ETH"));

    // Owner (governance, mocked) deactivates the manager: emergency response.
    t.ctrl_client().set_position_manager(&mgr, &false);
    let blocked = flatten(t.ctrl_client().try_withdraw(
        &mgr,
        &alice_id,
        &vec![&t.env, (eth.clone(), 0i128)],
        &Some(mallory.clone()),
    ));
    assert_contract_error(blocked, errors::NOT_AUTHORIZED);

    run_controller_op(
        &t,
        &mallory,
        &AdminOperation::SetPositionManager(mgr.clone(), true),
        "set_position_manager",
        vec![&t.env, mgr.clone().into_val(&t.env), true.into_val(&t.env)],
        5,
    )
    .expect("non-owner proposer executes SetPositionManager");

    // The dormant grant is live again: the manager drains Alice to mallory.
    t.ctrl_client().withdraw(
        &mgr,
        &alice_id,
        &vec![&t.env, (eth.clone(), 0i128)],
        &Some(mallory.clone()),
    );
    let mallory_eth =
        soroban_sdk::token::Client::new(&t.env, &t.resolve_asset("ETH")).balance(&mallory);
    std::println!("mallory_eth_raw={mallory_eth}");
    assert_eq!(mallory_eth, 10 * 10i128.pow(7));
    assert!(t.find_account_id(ALICE).is_none() || t.supply_balance(ALICE, "ETH") == 0.0);
}

/// UpgradeLiquidityPoolParams extremes `InterestRateModel::verify`
/// admits: 200% APR at any utilisation and a 99.99% reserve factor.
#[test]
fn proposer_sets_200_percent_apr_and_9999_reserve_factor() {
    let mut t = LendingTest::new().standard_two_asset().build();
    let mallory = grant_proposer(&t);

    t.supply(ALICE, "ETH", 10.0);
    t.borrow(ALICE, "USDC", 6000.0);
    let usdc = hub_asset(t.resolve_asset("USDC"));

    let model = InterestRateModel {
        max_borrow_rate: 2 * RAY,
        base_borrow_rate: 2 * RAY - 1,
        slope1: 2 * RAY - 1,
        slope2: 2 * RAY - 1,
        slope3: 2 * RAY - 1,
        mid_utilization: 1,
        optimal_utilization: 2,
        max_utilization: RAY,
        reserve_factor: 9_999,
        is_flashloanable: true,
        flashloan_fee: 0,
    };
    let args = UpgradePoolParamsArgs {
        hub_asset: usdc.clone(),
        params: model.clone(),
    };
    run_controller_op(
        &t,
        &mallory,
        &AdminOperation::UpgradeLiquidityPoolParams(args),
        "upgrade_liquidity_pool_params",
        vec![
            &t.env,
            usdc.clone().into_val(&t.env),
            model.into_val(&t.env),
        ],
        6,
    )
    .expect("non-owner proposer executes UpgradeLiquidityPoolParams");

    let rate = t.pool_borrow_rate("USDC");
    let debt0 = t.borrow_balance(ALICE, "USDC");
    let rev0 = t.snapshot_revenue("USDC");
    t.advance_and_sync(86_400);
    let debt1 = t.borrow_balance(ALICE, "USDC");
    let rev1 = t.snapshot_revenue("USDC");
    let supply_idx = t.ctrl_client().get_market_index(&usdc).supply_index;
    std::println!(
        "borrow_rate={rate} debt {debt0} -> {debt1} (+{}%) revenue_scaled {rev0} -> {rev1} \
         supply_index={supply_idx} ({})",
        (debt1 - debt0) / debt0 * 100.0,
        wad_to_f64(supply_idx / 1_000_000_000)
    );
    assert!(rate > 1.99, "rate {rate}");
    assert!(
        debt1 - debt0 > 32.0,
        "one day at 200% APR on 6000 adds > 32"
    );
}
