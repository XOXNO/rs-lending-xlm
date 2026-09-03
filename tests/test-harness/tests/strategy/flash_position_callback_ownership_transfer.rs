//! GH-13. The receiver is also the caller, so it owns the account the call
//! opens and may hand the NFT away from inside its own callback. The call
//! still finalizes under the original id and the new owner holds a solvent
//! position; no value is created.

use common::types::HubAssetKey;
use controller::constants::WAD;
use controller::types::PositionMode;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::xdr::ToXdr;
use soroban_sdk::{vec, Address, Bytes, Vec};
use test_harness::{
    assert_contract_error, errors, f64_to_i128, hub_asset, map_try_ok_unit, FlashPositionMode,
    FlashPositionRequest, FlashPositionTestReceiverClient, LendingTest, ALICE, BOB, HARNESS_SPOKE,
};

#[test]
fn a_self_owned_receiver_can_hand_the_account_away_mid_callback_and_finalize_still_gates() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(BOB, "ETH", 100.0);
    let receiver = t.deploy_flash_position_receiver();
    let bob = t.get_or_create_user(BOB);
    FlashPositionTestReceiverClient::new(&t.env, &receiver)
        .set_nft_transfer_target(&t.position_nft, &bob);
    let usdc = t.resolve_asset("USDC");
    let request = FlashPositionRequest {
        mode: FlashPositionMode::TransferNftMidCallback,
        collateral: usdc.clone(),
        collateral_amount: f64_to_i128(4_000.0, 7),
        extra_asset: Address::generate(&t.env),
        extra_amount: 0,
        reenter_spoke_id: HARNESS_SPOKE,
        reenter_account_id: 0,
    };
    let data: Bytes = request.to_xdr(&t.env);
    let mins: Vec<(HubAssetKey, i128)> = vec![&t.env, (hub_asset(usdc), f64_to_i128(4_000.0, 7))];
    let id = t.ctrl_client().flash_position(
        &receiver,
        &0,
        &HARNESS_SPOKE,
        &PositionMode::Multiply,
        &hub_asset(t.resolve_asset("ETH")),
        &f64_to_i128(1.0, 7),
        &receiver,
        &data,
        &mins,
        &Vec::new(&t.env),
    );
    assert_eq!(t.nft_owner_of(id), bob, "the callback moved the token");
    assert!(t.borrow_balance_raw_for(id, "ETH") > 0);
    assert!(t.supply_balance_raw_for(id, "USDC") > 0);
    assert!(t.health_factor_for_raw(ALICE, id) >= WAD);
    // The old owner lost every owner-gated verb; the new owner can unwind.
    let eth = hub_asset(t.resolve_asset("ETH"));
    let stale = t
        .ctrl_client()
        .try_borrow(&receiver, &id, &vec![&t.env, (eth.clone(), 1)], &None);
    assert_contract_error(map_try_ok_unit(stale), errors::NOT_AUTHORIZED);
    t.resolve_market("ETH")
        .token_admin
        .mint(&bob, &f64_to_i128(1.1, 7));
    t.ctrl_client()
        .repay(&bob, &id, &vec![&t.env, (eth, f64_to_i128(1.1, 7))]);
    let usdc = hub_asset(t.resolve_asset("USDC"));
    t.ctrl_client()
        .withdraw(&bob, &id, &vec![&t.env, (usdc, 0)], &None);
    assert!(
        !t.account_exists(id),
        "the new owner can unwind what it received"
    );
}
