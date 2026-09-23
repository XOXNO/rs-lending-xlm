//! A solvency-gate rejection in `flash_position` reverts every token transfer
//! made before it.
//!
//! `process_flash_position` mints debt on the pool, forwards it to the
//! receiver, deposits the measured collateral and refunds listed assets. Only
//! then does `strategy_finalize` run `require_post_pool_risk_gates`. The
//! `refunds` list is empty here, so the refund leg is not exercised.

extern crate std;

use controller::types::PositionMode;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::token;
use soroban_sdk::xdr::ToXdr;
use soroban_sdk::{Address, Bytes, Vec};
use test_harness::{
    assert_contract_error, errors, hub_asset, FlashPositionMode, FlashPositionRequest, HubAssetKey,
    LendingTest, ALICE, HARNESS_SPOKE,
};

fn collaterals(t: &LendingTest, name: &str, raw_min: i128) -> Vec<(HubAssetKey, i128)> {
    let mut out = Vec::new(&t.env);
    out.push_back((hub_asset(t.resolve_asset(name)), raw_min));
    out
}

#[test]
fn solvency_gate_failure_reverts_every_token_transfer() {
    let mut t = LendingTest::new().standard_two_asset().build();

    let alice = t.get_or_create_user(ALICE);
    let receiver = t.deploy_flash_position_receiver();
    let eth = t.resolve_asset("ETH");
    let usdc = t.resolve_asset("USDC");
    let pool = t.ctrl_client().get_pool_address();
    let controller = t.controller.clone();

    // The receiver returns 1 raw unit of USDC against 1.0 ETH of debt. That
    // meets the declared minimum, so the deposit succeeds and the solvency gate
    // rejects the call.
    let request = FlashPositionRequest {
        mode: FlashPositionMode::Success,
        collateral: usdc.clone(),
        collateral_amount: 1,
        extra_asset: Address::generate(&t.env),
        extra_amount: 0,
        reenter_spoke_id: HARNESS_SPOKE,
        reenter_account_id: 0,
    };
    let payload: Bytes = request.to_xdr(&t.env);
    let mins = collaterals(&t, "USDC", 1);
    let refunds = Vec::new(&t.env);

    let eth_client = token::Client::new(&t.env, &eth);
    let usdc_client = token::Client::new(&t.env, &usdc);

    let pool_eth_before = eth_client.balance(&pool);
    let pool_usdc_before = usdc_client.balance(&pool);
    let ctrl_eth_before = eth_client.balance(&controller);
    let ctrl_usdc_before = usdc_client.balance(&controller);
    let recv_eth_before = eth_client.balance(&receiver);
    let revenue_before = t.snapshot_revenue("ETH");
    let alice_eth_before = eth_client.balance(&alice);

    let res = t.try_flash_position(
        ALICE,
        0,
        PositionMode::Multiply,
        "ETH",
        1.0,
        &receiver,
        &payload,
        &mins,
        &refunds,
    );

    std::println!("flash_position result : {res:?}");
    std::println!(
        "pool  ETH  {} -> {}",
        pool_eth_before,
        eth_client.balance(&pool)
    );
    std::println!(
        "pool  USDC {} -> {}",
        pool_usdc_before,
        usdc_client.balance(&pool)
    );
    std::println!(
        "ctrl  ETH  {} -> {}",
        ctrl_eth_before,
        eth_client.balance(&controller)
    );
    std::println!(
        "recv  ETH  {} -> {}",
        recv_eth_before,
        eth_client.balance(&receiver)
    );
    std::println!(
        "ETH revenue {} -> {}",
        revenue_before,
        t.snapshot_revenue("ETH")
    );

    // The gate's error, not any error: a revert before any transfer would make
    // the balance checks below vacuous.
    assert_contract_error(res, errors::INSUFFICIENT_COLLATERAL);

    // The pool paid out ETH and the controller forwarded it to the receiver
    // before the gate ran. Host rollback restores all of it.
    assert_eq!(
        eth_client.balance(&pool),
        pool_eth_before,
        "pool ETH cash must be restored"
    );
    assert_eq!(
        usdc_client.balance(&pool),
        pool_usdc_before,
        "pool USDC must be restored"
    );
    assert_eq!(
        eth_client.balance(&controller),
        ctrl_eth_before,
        "controller ETH must be restored"
    );
    assert_eq!(
        usdc_client.balance(&controller),
        ctrl_usdc_before,
        "controller USDC must be restored"
    );
    assert_eq!(
        eth_client.balance(&receiver),
        recv_eth_before,
        "the receiver must not keep the forwarded principal"
    );
    assert_eq!(
        t.snapshot_revenue("ETH"),
        revenue_before,
        "no revenue may be booked by a reverted strategy"
    );
    assert_eq!(
        eth_client.balance(&alice),
        alice_eth_before,
        "caller balance must be untouched"
    );
}
