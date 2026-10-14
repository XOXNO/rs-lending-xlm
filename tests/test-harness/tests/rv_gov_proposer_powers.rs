//! RV break pass: the documented blast radius of a stolen NON-OWNER PROPOSER
//! key once the timelock delay elapses (12 ledgers on the configured networks).
//!
//! Each test grants `mallory` only the proposer role and drives one operation
//! through `propose` and a permissionless `execute`. The proposer-powers fix
//! (gov_b G10, INV-AUTH-09) moved liquidation terms to the owner: a threshold
//! cut, a bonus raise, a fee cut, the spoke liquidation curve, forced
//! socialization and position-manager activation are refused at proposal for
//! a non-owner proposer, and those three tests were flipped from the earlier
//! characterization; the owner path in each keeps the earlier numbers. Spoke
//! removal and the interest-rate model stay proposer powers and remain
//! characterization tests of docs/explanation/threat-model.md: if one of them
//! starts failing, the proposer's powers changed and the threat model must be
//! updated with it.

use controller::constants::{RAY, WAD};
use controller::types::{InterestRateModel, SpokeAssetArgs};
use governance_interface::{
    AdminOperation, OperationState, RoleArgs, SpokeLiquidationCurveArgs, UpgradePoolParamsArgs,
};
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{vec, Address, BytesN, IntoVal, Symbol, Val, Vec};
use test_harness::{
    assert_contract_error, errors, hub_asset, usd_cents, wad_to_f64, LendingTest, ALICE,
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

/// Proposes `op` as `proposer`; the contract error if the proposal is refused.
fn try_propose(
    t: &LendingTest,
    proposer: &Address,
    op: &AdminOperation,
    byte: u8,
) -> Result<(), soroban_sdk::Error> {
    flatten(
        t.gov_iface_client()
            .try_propose(proposer, op, &salt(&t.env, byte)),
    )
}

/// `proposer` schedules `op`, the delay passes, anyone executes it against
/// the controller with the raw `(function, args)` tuple.
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

/// Liquidation terms are the owner's (gov_b G10, INV-AUTH-09; was
/// `proposer_threshold_ratchet_and_curve_wipe_alice_equity`). Alice: 10 ETH
/// ($20k) collateral, 6000 USDC debt, HF 2.67 at the stamped 80% threshold.
/// The proposer's edit to (ltv 3100, thr 3200, bonus 21250, fees 0), which
/// earlier restamped her to HF 1.07 so that an 8% ETH dip closed the whole
/// position, is refused at proposal (44); so are a one-point threshold cut, a
/// one-point bonus raise, a one-point fee cut and the curve edit. An edit
/// that keeps the terms stays with the proposer and executes. The owner's
/// authority is unchanged: its cut executes and the permissionless restamp
/// lands exactly the earlier numbers.
#[test]
fn proposer_cannot_worsen_liquidation_terms_or_set_the_curve() {
    let mut t = LendingTest::new().standard_two_asset().build();
    let mallory = grant_proposer(&t);

    t.supply(ALICE, "ETH", 10.0);
    t.borrow(ALICE, "USDC", 6000.0);
    let alice_id = t.resolve_account_id(ALICE);
    let hf0 = t.health_factor(ALICE);
    assert!(hf0 > 2.6 && hf0 < 2.7, "hf0 = {hf0}");
    let eth = hub_asset(t.resolve_asset("ETH"));
    let live = t.ctrl_client().get_spoke_asset(&HARNESS_SPOKE, &eth);
    let (ltv, thr, bonus, fees) = (
        live.loan_to_value,
        live.liquidation_threshold,
        live.liquidation_bonus,
        live.liquidation_fees,
    );

    // Bound check of the review's row: 3200 * (10000 + 21250) == 10000 * 10000.
    let worse = [
        eth_args(&t, 3100, 3200, 21_250, 0),
        eth_args(&t, ltv, thr - 1, bonus, fees),
        eth_args(&t, ltv, thr, bonus + 1, fees),
        eth_args(&t, ltv, thr, bonus, fees - 1),
    ];
    for (i, args) in worse.iter().enumerate() {
        assert_contract_error(
            try_propose(
                &t,
                &mallory,
                &AdminOperation::EditAssetInSpoke(args.clone()),
                10 + i as u8,
            ),
            errors::NOT_AUTHORIZED,
        );
    }
    let curve = SpokeLiquidationCurveArgs {
        spoke_id: HARNESS_SPOKE,
        target_hf_wad: 10 * WAD,
        hf_for_max_bonus_wad: 10 * WAD - 1,
        liquidation_bonus_factor_bps: 10_000,
    };
    assert_contract_error(
        try_propose(
            &t,
            &mallory,
            &AdminOperation::SetSpokeLiquidationCurve(curve),
            14,
        ),
        errors::NOT_AUTHORIZED,
    );
    assert_eq!(t.health_factor(ALICE), hf0, "nothing was scheduled");

    // Kept terms with a tighter cap: routine listing power, still the proposer's.
    let mut kept = eth_args(&t, ltv, thr, bonus, fees);
    kept.supply_cap = UNCONSTRAINED_TEST_CAP / 2;
    run_controller_op(
        &t,
        &mallory,
        &AdminOperation::EditAssetInSpoke(kept.clone()),
        "edit_asset_in_spoke",
        vec![&t.env, kept.into_val(&t.env)],
        15,
    )
    .expect("a proposer may keep the terms and tighten a cap");
    assert_eq!(
        t.ctrl_client()
            .get_spoke_asset(&HARNESS_SPOKE, &eth)
            .supply_cap,
        UNCONSTRAINED_TEST_CAP / 2
    );

    // The owner's cut: the earlier characterization, now an owner-only power.
    let args = eth_args(&t, 3100, 3200, 21_250, 0);
    run_controller_op(
        &t,
        &t.admin(),
        &AdminOperation::EditAssetInSpoke(args.clone()),
        "edit_asset_in_spoke",
        vec![&t.env, args.into_val(&t.env)],
        16,
    )
    .expect("the owner executes EditAssetInSpoke");
    let keeper = Address::generate(&t.env);
    t.ctrl_client()
        .update_account_threshold(&keeper, &true, &vec![&t.env, alice_id]);
    let hf1 = t.health_factor(ALICE);
    assert!(hf1 > 1.06 && hf1 < 1.07, "hf1 = {hf1}");
    let (supplies, _) = t.ctrl_client().get_account_positions(&alice_id);
    let eth_pos = supplies.get(eth).unwrap();
    assert_eq!(eth_pos.liquidation_threshold, 3200);
    assert_eq!(eth_pos.liquidation_bonus, 21_250);
    assert_eq!(eth_pos.liquidation_fees, 0);
}

/// `ForceSocializeBadDebt` is owner-only (gov_b G10, INV-AUTH-09; was
/// `proposer_force_socializes_merely_insolvent_account`). Alice: 10,000 USDC
/// collateral, 3 ETH ($6000) debt; USDC -> $0.50 makes C=$5000 < D=$6000. The
/// proposer's proposal is refused (44) and the account stays. The owner's
/// executes with the earlier outcome: a normal liquidation would repay
/// min(D, C/(1+5%)) = $4761.9 and seize all $5000, leaving $1238 to
/// socialize; the forced path socializes the full $6000 against ETH
/// suppliers and books all 10,000 USDC as protocol revenue.
#[test]
fn proposer_cannot_force_socialize_a_merely_insolvent_account() {
    // No ETH seed: the harness seed now mints supply shares, which would
    // share the write-down with Bob.
    let mut t = LendingTest::new()
        .standard_two_asset()
        .with_initial_liquidity("ETH", 0.0)
        .build();
    let mallory = grant_proposer(&t);

    // Bob is the only ETH share holder.
    t.supply("bob", "ETH", 100.0);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    t.set_price("USDC", usd_cents(50));
    let alice_id = t.resolve_account_id(ALICE);
    let coll = t.total_collateral(ALICE);
    let debt = t.total_debt(ALICE);
    assert!(debt > coll, "debt {debt} must exceed collateral {coll}");

    assert_contract_error(
        try_propose(
            &t,
            &mallory,
            &AdminOperation::ForceSocializeBadDebt(alice_id),
            3,
        ),
        errors::NOT_AUTHORIZED,
    );
    assert!(
        t.account_exists(alice_id),
        "the proposer's operation was refused"
    );

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
        &t.admin(),
        &AdminOperation::ForceSocializeBadDebt(alice_id),
        "force_socialize_bad_debt",
        vec![&t.env, alice_id.into_val(&t.env)],
        4,
    )
    .expect("the owner executes ForceSocializeBadDebt");

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

/// `SetPositionManager(manager, true)` is owner-only (gov_b G10,
/// INV-AUTH-09; was `proposer_reactivates_deactivated_manager_and_revives_grants`):
/// it re-arms every delegate grant stored under a manager the owner had
/// deactivated. The proposer's re-activation is refused at proposal (44) and
/// the dormant grant stays dormant; deactivating a manager, a safety action,
/// stays a proposer power. The owner's re-activation executes with the
/// earlier outcome: the manager drains Alice to mallory.
#[test]
fn proposer_cannot_reactivate_a_deactivated_manager() {
    let mut t = LendingTest::new().standard_two_asset().build();
    let mallory = grant_proposer(&t);

    t.supply(ALICE, "ETH", 10.0);
    let alice_id = t.resolve_account_id(ALICE);
    t.enable_delegate(ALICE, "mgr", alice_id);
    let mgr = t.get_or_create_user("mgr");
    let eth = hub_asset(t.resolve_asset("ETH"));

    // Owner (governance, mocked) deactivates the manager: emergency response.
    t.ctrl_client().set_position_manager(&mgr, &false);
    let blocked = |t: &LendingTest| {
        flatten(t.ctrl_client().try_withdraw(
            &mgr,
            &alice_id,
            &vec![&t.env, (eth.clone(), 0i128)],
            &Some(mallory.clone()),
        ))
    };
    assert_contract_error(blocked(&t), errors::NOT_AUTHORIZED);

    assert_contract_error(
        try_propose(
            &t,
            &mallory,
            &AdminOperation::SetPositionManager(mgr.clone(), true),
            5,
        ),
        errors::NOT_AUTHORIZED,
    );
    let off = t.gov_iface_client().propose(
        &mallory,
        &AdminOperation::SetPositionManager(mgr.clone(), false),
        &salt(&t.env, 6),
    );
    assert_eq!(
        t.gov_iface_client().get_operation_state(&off),
        OperationState::Waiting,
        "deactivation stays a proposer power"
    );
    assert_contract_error(blocked(&t), errors::NOT_AUTHORIZED);

    run_controller_op(
        &t,
        &t.admin(),
        &AdminOperation::SetPositionManager(mgr.clone(), true),
        "set_position_manager",
        vec![&t.env, mgr.clone().into_val(&t.env), true.into_val(&t.env)],
        7,
    )
    .expect("the owner executes SetPositionManager");

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
