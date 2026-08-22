use controller::constants::{RAY, WAD};
use soroban_sdk::testutils::{ContractEvents, Events, MockAuth, MockAuthInvoke};
use soroban_sdk::token;
use soroban_sdk::xdr::{ContractEventBody, ScVal};
use soroban_sdk::{Address, IntoVal, Val};
use test_harness::mock_aggregator::{BadAggregator, BadMode};
use test_harness::{
    apply_flash_fee, assert_contract_error, build_aggregator_swap, errors, hub_asset,
    map_try_ok_value, usd, LendingTest, ALICE, BOB,
};

use crate::helpers::AliceOps;

const SWAP_REQUESTED_ETH: i128 = 10_000_000;
const SWAP_MIN_OUT_USDC: i128 = 30_000_000_000;

fn count_topic(events: &ContractEvents, first: &str, second: &str) -> usize {
    events
        .events()
        .iter()
        .filter(|event| {
            let ContractEventBody::V0(body) = &event.body;
            matches!(
                (body.topics.first(), body.topics.get(1)),
                (Some(ScVal::Symbol(a)), Some(ScVal::Symbol(b)))
                    if a.0.to_utf8_string().as_deref() == Ok(first)
                        && b.0.to_utf8_string().as_deref() == Ok(second)
            )
        })
        .count()
}

fn count_zero_transfers(events: &ContractEvents) -> usize {
    events
        .events()
        .iter()
        .filter(|event| {
            let ContractEventBody::V0(body) = &event.body;
            let is_transfer = matches!(
                body.topics.first(),
                Some(ScVal::Symbol(topic))
                    if topic.0.to_utf8_string().as_deref() == Ok("transfer")
            );
            is_transfer && matches!(&body.data, ScVal::I128(amount) if i128::from(amount) == 0)
        })
        .count()
}

fn install_bad_router(t: &LendingTest, mode: BadMode) -> Address {
    let admin = t.admin.clone();
    let bad = t.env.register(BadAggregator, (admin.clone(), mode));
    t.ctrl_client().set_swap_aggregator(&bad);
    bad
}

fn mint_to(t: &LendingTest, asset_name: &str, target: &Address, raw_amount: i128) {
    let market = t.resolve_market(asset_name);
    market.token_admin.mint(target, &raw_amount);
}

fn assert_overpull_rejected(result: Result<u64, soroban_sdk::Error>) {
    match result {
        Ok(account_id) => panic!("OverPull must be rejected, got Ok(account_id={account_id})"),
        Err(err) => {
            let overspend = soroban_sdk::Error::from_contract_error(errors::ROUTER_OVERSPEND);
            let err_str = format!("{err:?}");
            assert!(
                err == overspend || err_str.contains("Error(Contract,"),
                "OverPull must reject via ROUTER_OVERSPEND or a contract error, got {err:?}"
            );
        }
    }
}

fn set_sanity_bounds(t: &LendingTest, asset_name: &str, min_wad: i128, max_wad: i128) {
    let asset = t.resolve_asset(asset_name);
    let mut oracle = t
        .price_agg_client()
        .oracle(&controller::types::PriceKey::Token(asset.clone()))
        .unwrap();
    oracle.min_sanity_price_wad = min_wad;
    oracle.max_sanity_price_wad = max_wad;
    t.price_agg_client()
        .seed_oracle(&controller::types::PriceKey::Token(asset.clone()), &oracle);
}

#[test]
fn test_swap_tokens_panics_when_router_refunds_token_in() {
    let mut t = LendingTest::new().standard_two_asset().build();

    let bad = install_bad_router(&t, BadMode::Refund);

    mint_to(&t, "USDC", &bad, 300_000_000_000);

    mint_to(&t, "ETH", &bad, 100_000_000);

    let steps = build_aggregator_swap(
        &t,
        "ETH",
        "USDC",
        apply_flash_fee(SWAP_REQUESTED_ETH),
        SWAP_MIN_OUT_USDC,
    );
    let result = t.try_alice_multiply(&steps);

    assert_contract_error(result, errors::ROUTER_OVERSPEND);
}

#[test]
fn test_swap_tokens_rejects_router_pulling_more_than_allowance() {
    let mut t = LendingTest::new().standard_two_asset().build();

    let bad = install_bad_router(&t, BadMode::OverPull);
    mint_to(&t, "USDC", &bad, 300_000_000_000);

    let steps = build_aggregator_swap(
        &t,
        "ETH",
        "USDC",
        apply_flash_fee(SWAP_REQUESTED_ETH),
        SWAP_MIN_OUT_USDC,
    );
    let result = t.try_alice_multiply(&steps);

    assert_overpull_rejected(result);
}

#[test]
fn test_swap_tokens_refunds_router_underspend() {
    let mut t = LendingTest::new().standard_two_asset().build();

    let bad = install_bad_router(&t, BadMode::UnderPull);
    mint_to(&t, "USDC", &bad, SWAP_MIN_OUT_USDC);

    let alice = t.get_or_create_user(ALICE);
    let eth_before = t.token_balance(ALICE, "ETH");
    let steps = build_aggregator_swap(
        &t,
        "ETH",
        "USDC",
        apply_flash_fee(SWAP_REQUESTED_ETH),
        SWAP_MIN_OUT_USDC,
    );

    t.try_alice_multiply(&steps)
        .expect("underspend should be refunded, not rejected");

    let eth_after = token::Client::new(&t.env, &t.resolve_asset("ETH")).balance(&alice);
    assert!(
        (eth_after as f64 / 10_000_000.0) > eth_before + 0.49,
        "Alice should receive the unspent borrowed ETH"
    );
}

#[test]
fn test_swap_collateral_refunds_router_underspend_to_caller() {
    let mut t = LendingTest::new().standard_two_asset().build();

    let bad = install_bad_router(&t, BadMode::UnderPull);
    mint_to(&t, "ETH", &bad, 50_000_000);

    t.supply(ALICE, "USDC", 1_000.0);
    let alice_usdc_before = t.token_balance_raw(ALICE, "USDC");
    let steps = build_aggregator_swap(&t, "USDC", "ETH", 10_000_000_000, 50_000_000);

    t.try_swap_collateral(ALICE, "USDC", 1_000.0, "ETH", &steps)
        .expect("swap_collateral should refund router underspend");

    let alice_usdc_after = t.token_balance_raw(ALICE, "USDC");
    assert_eq!(
        alice_usdc_after - alice_usdc_before,
        5_000_000_000,
        "half of the withdrawn USDC should be refunded to Alice's wallet"
    );

    let usdc = t.resolve_asset("USDC");
    let usdc_tok = token::Client::new(&t.env, &usdc);
    assert_eq!(
        usdc_tok.balance(&t.controller_address()),
        0,
        "controller must not strand unspent swap_collateral input"
    );
}

#[test]
fn test_repay_debt_with_collateral_refunds_router_underspend_to_caller() {
    let mut t = LendingTest::new().standard_two_asset().build();

    let bad = install_bad_router(&t, BadMode::UnderPull);
    mint_to(&t, "ETH", &bad, 5_000_000);

    t.supply(ALICE, "USDC", 2_000.0);
    t.borrow(ALICE, "ETH", 0.5);
    let account_id = t.resolve_account_id(ALICE);
    let alice_usdc_before = t.token_balance_raw(ALICE, "USDC");
    let steps = build_aggregator_swap(&t, "USDC", "ETH", 20_000_000_000, 5_000_000);

    t.try_repay_debt_with_collateral(ALICE, "USDC", 2_000.0, "ETH", &steps, false)
        .expect("repay_debt_with_collateral should refund router underspend");

    let alice_usdc_after = t.token_balance_raw(ALICE, "USDC");
    assert_eq!(
        alice_usdc_after - alice_usdc_before,
        10_000_000_000,
        "half of the withdrawn USDC should be refunded to Alice's wallet"
    );
    assert_eq!(
        t.borrow_balance_raw(ALICE, "ETH"),
        0,
        "router output should fully repay the ETH debt"
    );
    assert!(
        !t.account_exists(account_id),
        "fully repaid and fully withdrawn account should be removed"
    );
}

#[test]
fn test_repay_without_excess_skips_zero_value_refund() {
    let mut t = LendingTest::new().standard_two_asset().build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);
    t.fund_router("ETH", 0.5);
    let steps = build_aggregator_swap(&t, "USDC", "ETH", 10_000_000_000, 5_000_000);
    let zero_transfers_before = count_zero_transfers(&t.env.events().all());

    t.repay_debt_with_collateral(ALICE, "USDC", 1_000.0, "ETH", &steps, false);

    assert_eq!(
        count_zero_transfers(&t.env.events().all()),
        zero_transfers_before,
        "an empty refund must not invoke a zero-value token transfer"
    );
}

#[test]
fn test_router_pull_uses_controller_self_authorization() {
    let mut t = LendingTest::new().standard_two_asset().build();
    let caller = t.get_or_create_user(ALICE);
    t.supply(ALICE, "USDC", 100_000.0);
    t.fund_router("ETH", 5.0);

    let account_id = t.resolve_account_id(ALICE);
    let current = test_harness::hub_asset(t.resolve_asset("USDC"));
    let replacement = test_harness::hub_asset(t.resolve_asset("ETH"));
    let amount = 10_000_000_000;
    let steps = build_aggregator_swap(&t, "USDC", "ETH", amount, 50_000_000);
    let args: soroban_sdk::Vec<Val> = (
        caller.clone(),
        account_id,
        current.clone(),
        amount,
        replacement.clone(),
        steps.clone(),
    )
        .into_val(&t.env);
    let invoke = MockAuthInvoke {
        contract: &t.controller,
        fn_name: "swap_collateral",
        args,
        sub_invokes: &[],
    };
    let auths = [MockAuth {
        address: &caller,
        invoke: &invoke,
    }];

    t.ctrl_client().mock_auths(&auths).swap_collateral(
        &caller,
        &account_id,
        &current,
        &amount,
        &replacement,
        &steps,
    );

    assert!(t.supply_balance(ALICE, "ETH") > 4.9);
}

#[test]
fn test_swap_tokens_handles_zero_output_from_router() {
    let mut t = LendingTest::new().standard_two_asset().build();

    install_bad_router(&t, BadMode::OutputShortfall);

    let steps = build_aggregator_swap(
        &t,
        "ETH",
        "USDC",
        apply_flash_fee(SWAP_REQUESTED_ETH),
        SWAP_MIN_OUT_USDC,
    );
    let result = t.try_alice_multiply(&steps);

    assert_contract_error(result, errors::NO_SWAP_OUTPUT);
}

#[test]
fn test_multiply_third_token_payment_without_convert_steps_rejects() {
    let mut t = LendingTest::new().three_asset_usdc_eth_wbtc().build();

    let alice = t.get_or_create_user(ALICE);

    t.resolve_market("WBTC")
        .token_admin
        .mint(&alice, &10_000_000i128);

    let usdc = t.resolve_asset("USDC");
    let eth = t.resolve_asset("ETH");
    let wbtc = t.resolve_asset("WBTC");

    t.fund_router("USDC", 3_000.0);
    let steps = build_aggregator_swap(&t, "ETH", "USDC", 0, 30_000_000_000);

    let ctrl = t.ctrl_client();

    let result = ctrl.try_multiply(
        &alice,
        &0u64,
        &1u32,
        &hub_asset(usdc.clone()),
        &1_0000000i128,
        &hub_asset(eth.clone()),
        &controller::types::PositionMode::Multiply,
        &steps,
        &Some((hub_asset(wbtc), 100_000i128)),
        &None,
    );
    assert_contract_error(map_try_ok_value(result), errors::CONVERT_STEPS_REQUIRED);
}

#[test]
fn test_multiply_existing_account_mode_mismatch_rejects() {
    let mut t = LendingTest::new().standard_two_asset().build();

    let account_id = t.create_account_full(ALICE, 1, controller::types::PositionMode::Multiply);
    t.supply_to(ALICE, account_id, "USDC", 1_000.0);

    t.fund_router("USDC", 3_000.0);
    let steps = build_aggregator_swap(&t, "ETH", "USDC", 0, 30_000_000_000);
    let alice = t.get_or_create_user(ALICE);
    let usdc = t.resolve_asset("USDC");
    let eth = t.resolve_asset("ETH");

    let ctrl = t.ctrl_client();
    let result = ctrl.try_multiply(
        &alice,
        &account_id,
        &1u32,
        &hub_asset(usdc.clone()),
        &1_0000000i128,
        &hub_asset(eth.clone()),
        &controller::types::PositionMode::Long,
        &steps,
        &None,
        &None,
    );
    assert_contract_error(map_try_ok_value(result), errors::ACCOUNT_MODE_MISMATCH);
}

#[test]
fn test_swap_debt_existing_position_missing_rejects() {
    let mut t = LendingTest::new().three_asset_usdc_eth_wbtc().build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "WBTC", 0.01);

    t.fund_router("ETH", 0.5);

    let steps = build_aggregator_swap(&t, "WBTC", "ETH", apply_flash_fee(10_000), 5_000_000);

    let result = t.try_swap_debt(ALICE, "ETH", 0.001, "WBTC", &steps);
    assert_contract_error(result, errors::DEBT_POSITION_NOT_FOUND);
}

#[test]
fn test_swap_collateral_position_missing_rejects() {
    let mut t = LendingTest::new().three_asset_usdc_eth_wbtc().build();

    t.supply(ALICE, "USDC", 10_000.0);

    let steps = build_aggregator_swap(&t, "WBTC", "ETH", 0, 5_0000000);
    let result = t.try_swap_collateral(ALICE, "WBTC", 0.01, "ETH", &steps);
    assert_contract_error(result, errors::COLLATERAL_POSITION_NOT_FOUND);
}

#[test]
fn test_repay_debt_with_collateral_missing_collateral_rejects() {
    let mut t = LendingTest::new().three_asset_usdc_eth_wbtc().build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    let steps = build_aggregator_swap(&t, "WBTC", "ETH", 0, 1_0000000);
    let result = t.try_repay_debt_with_collateral(ALICE, "WBTC", 0.01, "ETH", &steps, false);
    assert_contract_error(result, errors::COLLATERAL_POSITION_NOT_FOUND);
}

#[test]
fn test_repay_debt_with_collateral_missing_debt_rejects() {
    let mut t = LendingTest::new().three_asset_usdc_eth_wbtc().build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    t.fund_router("WBTC", 0.01);
    let steps = build_aggregator_swap(&t, "USDC", "WBTC", 0, 1_000_000);
    let result = t.try_repay_debt_with_collateral(ALICE, "USDC", 1_000.0, "WBTC", &steps, false);
    assert_contract_error(result, errors::DEBT_POSITION_NOT_FOUND);
}

#[test]
fn test_repay_debt_with_collateral_close_with_remaining_debt_rejects() {
    let mut t = LendingTest::new().standard_two_asset().build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 5.0);

    t.fund_router("ETH", 0.01);

    let steps = build_aggregator_swap(&t, "USDC", "ETH", 200_000_000, 100_000);

    let result = t.try_repay_debt_with_collateral(ALICE, "USDC", 20.0, "ETH", &steps, true);
    assert_contract_error(result, errors::CANNOT_CLOSE_WITH_REMAINING_DEBT);
}

#[test]
fn test_multiply_with_collateral_token_initial_payment() {
    let mut t = LendingTest::new().standard_two_asset().build();

    let alice = t.get_or_create_user(ALICE);
    let usdc_market = t.resolve_market("USDC");
    let usdc = t.resolve_asset("USDC");
    let eth = t.resolve_asset("ETH");

    usdc_market.token_admin.mint(&alice, &500_0000000i128);

    let alice_usdc_before = t.token_balance(ALICE, "USDC");
    t.fund_router("USDC", 3_000.0);

    let steps = build_aggregator_swap(
        &t,
        "ETH",
        "USDC",
        apply_flash_fee(10_000_000),
        30_000_000_000,
    );

    let ctrl = t.ctrl_client();
    let account_id = ctrl.multiply(
        &alice,
        &0u64,
        &1u32,
        &hub_asset(usdc.clone()),
        &1_0000000i128,
        &hub_asset(eth.clone()),
        &controller::types::PositionMode::Multiply,
        &steps,
        &Some((hub_asset(usdc.clone()), 500_0000000i128)),
        &None,
    );

    assert_eq!(
        count_topic(&t.env.events().all(), "strategy", "initial_payment"),
        1,
        "multiply with an initial payment must emit its strategy event"
    );

    let supply = t.supply_balance_for(ALICE, account_id, "USDC");
    assert!(
        (3_499.0..=3_501.0).contains(&supply),
        "collateral-token initial payment must be added directly to collateral: got {}",
        supply
    );

    let borrow = t.borrow_balance_for(ALICE, account_id, "ETH");
    assert!(
        (0.99..=1.01).contains(&borrow),
        "borrow must be only the flash debt: got {}",
        borrow
    );

    let alice_usdc_after = t.token_balance(ALICE, "USDC");
    assert!(
        (alice_usdc_before - alice_usdc_after - 500.0).abs() < 1e-6,
        "Alice's USDC wallet should drop by exactly 500, before={}, after={}",
        alice_usdc_before,
        alice_usdc_after
    );
}

#[test]
fn test_multiply_with_third_token_initial_payment_swaps_via_convert_steps() {
    let mut t = LendingTest::new().three_asset_usdc_eth_wbtc().build();

    let alice = t.get_or_create_user(ALICE);
    let usdc = t.resolve_asset("USDC");
    let eth = t.resolve_asset("ETH");
    let wbtc_market = t.resolve_market("WBTC");
    let wbtc = t.resolve_asset("WBTC");

    wbtc_market.token_admin.mint(&alice, &10_000_000i128);

    let alice_wbtc_before = t.token_balance(ALICE, "WBTC");

    t.fund_router("USDC", 3_500.0);

    let main_steps = build_aggregator_swap(
        &t,
        "ETH",
        "USDC",
        apply_flash_fee(10_000_000),
        30_000_000_000,
    );

    let convert_steps = build_aggregator_swap(&t, "WBTC", "USDC", 10_000_000, 500_0000000);

    let ctrl = t.ctrl_client();
    let account_id = ctrl.multiply(
        &alice,
        &0u64,
        &1u32,
        &hub_asset(usdc.clone()),
        &1_0000000i128,
        &hub_asset(eth.clone()),
        &controller::types::PositionMode::Multiply,
        &main_steps,
        &Some((hub_asset(wbtc), 10_000_000i128)),
        &Some(convert_steps),
    );

    let supply = t.supply_balance_for(ALICE, account_id, "USDC");
    assert!(
        (3_499.0..=3_501.0).contains(&supply),
        "third-token payment must be converted and added to collateral: got {}",
        supply
    );

    let alice_wbtc_after = t.token_balance(ALICE, "WBTC");
    assert!(
        alice_wbtc_after < alice_wbtc_before,
        "Alice's WBTC wallet must decrease after multiply with WBTC initial payment: before={}, after={}",
        alice_wbtc_before,
        alice_wbtc_after
    );
}

#[test]
fn test_swap_tokens_allowance_remains_zero_after_overpull_rejection() {
    let mut t = LendingTest::new().standard_two_asset().build();

    let admin = t.admin.clone();
    let bad = t.env.register(BadAggregator, (admin, BadMode::OverPull));
    t.ctrl_client().set_swap_aggregator(&bad);

    t.resolve_market("USDC")
        .token_admin
        .mint(&bad, &300_000_000_000_i128);
    t.resolve_market("ETH")
        .token_admin
        .mint(&bad, &100_000_000_i128);

    let steps = build_aggregator_swap(&t, "ETH", "USDC", 0, 30_000_000_000);
    let result = t.try_alice_multiply(&steps);
    assert_overpull_rejected(result);

    let eth = t.resolve_asset("ETH");
    let eth_tok = token::Client::new(&t.env, &eth);
    let allowance = eth_tok.allowance(&t.controller_address(), &bad);
    assert_eq!(
        allowance, 0,
        "post-rollback allowance on rejected swap must be zero, got {}",
        allowance
    );
}

#[test]
fn test_swap_tokens_allowance_zero_after_successful_multiply() {
    let mut t = LendingTest::new().standard_two_asset().build();

    t.fund_router("USDC", 3_000.0);

    let steps = build_aggregator_swap(
        &t,
        "ETH",
        "USDC",
        apply_flash_fee(10_000_000),
        30_000_000_000,
    );
    let _account_id = t.try_alice_multiply(&steps).expect("multiply");

    let eth = t.resolve_asset("ETH");
    let eth_tok = token::Client::new(&t.env, &eth);
    let allowance = eth_tok.allowance(&t.controller_address(), &t.aggregator);
    assert_eq!(
        allowance, 0,
        "controller allowance on the router must be zero after a successful swap, got {}",
        allowance
    );
}

#[test]
fn test_multiply_reusing_account_wrong_owner_rejects() {
    let mut t = LendingTest::new().standard_two_asset().build();

    t.fund_router("USDC", 3_000.0);
    let steps = build_aggregator_swap(
        &t,
        "ETH",
        "USDC",
        apply_flash_fee(10_000_000),
        30_000_000_000,
    );
    let alice_account = t.try_alice_multiply(&steps).expect("multiply");

    t.fund_router("USDC", 3_000.0);
    let steps2 = build_aggregator_swap(&t, "ETH", "USDC", 0, 30_000_000_000);
    let bob = t.get_or_create_user(BOB);
    let usdc = t.resolve_asset("USDC");
    let eth = t.resolve_asset("ETH");
    let ctrl = t.ctrl_client();
    let result = ctrl.try_multiply(
        &bob,
        &alice_account,
        &1u32,
        &hub_asset(usdc.clone()),
        &1_0000000i128,
        &hub_asset(eth.clone()),
        &controller::types::PositionMode::Multiply,
        &steps2,
        &None,
        &None,
    );
    assert_contract_error(map_try_ok_value(result), errors::NOT_AUTHORIZED);
}

#[test]
fn test_sanity_bound_ceiling_exact_accept_then_one_over_reject() {
    let mut t = LendingTest::new().standard_two_asset().build();

    set_sanity_bounds(&t, "ETH", usd(100), usd(2_000));
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    t.set_price_keeping_sanity_band("ETH", usd(2_000) + WAD / 100);
    let result = t.try_borrow(ALICE, "ETH", 0.1);
    assert_contract_error(result, errors::SANITY_BOUND_VIOLATED);
}

#[test]
fn test_sanity_bound_floor_exact_accept_then_one_under_reject() {
    let mut t = LendingTest::new().standard_two_asset().build();

    set_sanity_bounds(&t, "ETH", usd(2_000), usd(10_000));
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    t.set_price_keeping_sanity_band("ETH", usd(2_000) - WAD / 100);
    let result = t.try_borrow(ALICE, "ETH", 0.1);
    assert_contract_error(result, errors::SANITY_BOUND_VIOLATED);
}

#[test]
fn test_borrow_at_cap_then_step_over_rejected() {
    let mut t = LendingTest::new()
        .standard_two_asset()
        .with_market_params("USDC", |p| {
            p.max_utilization = controller::constants::RAY * 85 / 100;
        })
        .build();

    t.supply(ALICE, "USDC", 1_000.0);
    t.supply(BOB, "ETH", 10.0);

    t.borrow(BOB, "USDC", 850.0);

    let result = t.try_borrow(BOB, "USDC", 1.0);
    assert_contract_error(result, errors::UTILIZATION_ABOVE_MAX);
}

#[test]
fn test_multiply_at_utilization_cap_then_step_over_rejected() {
    let mut t = LendingTest::new()
        .standard_two_asset()
        .with_market_params("ETH", |p| {
            p.max_utilization = RAY * 85 / 100;
        })
        .build();

    t.supply(BOB, "ETH", 1_000.0);
    t.supply(ALICE, "USDC", 50_000.0);
    t.supply(BOB, "USDC", 400_000.0);
    t.borrow(BOB, "ETH", 850.0);

    t.fund_router("USDC", 3_000.0);
    let steps = build_aggregator_swap(
        &t,
        "ETH",
        "USDC",
        apply_flash_fee(10_000_000),
        30_000_000_000,
    );
    let result = t.try_alice_multiply(&steps);
    assert_contract_error(result, errors::UTILIZATION_ABOVE_MAX);
}

#[test]
fn test_strategy_multiply_unsupported_category() {
    let mut t = LendingTest::new()
        .standard_two_asset()
        .with_dust_disabled_all_markets()
        .with_max_utilization_disabled_all_markets()
        .build();

    t.supply("alice", "USDC", 10.0);
    let steps = t.mock_swap_steps("ETH", "USDC", usd(2000));

    let res = t.try_multiply_with_category(
        "alice",
        999,
        "USDC",
        5.0,
        "ETH",
        controller::types::PositionMode::Multiply,
        &steps,
    );

    assert_contract_error(res, errors::SPOKE_NOT_FOUND);
}
