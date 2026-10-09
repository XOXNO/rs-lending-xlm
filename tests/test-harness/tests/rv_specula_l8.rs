//! Second opinion on Specula lending CR-19 and CR-25. Both tests pin the HEAD
//! behaviour, so each is the assertion that flips once the finding is fixed.
//!
//! CR-19: `process_flash_position` loads the account, and the NFT owner it
//! carries, before the receiver callback and never re-reads it. A receiver
//! that owns the account can move the NFT inside the callback. The
//! `batch_update` event that `strategy_finalize` publishes then names the
//! owner at entry, not the holder at the end of the call. The controller
//! stores no owner, so only the event is stale.
//!
//! CR-25: `get_liquidation_estimate` builds the same plan as `liquidate`, but
//! for `SeizeMode::Credit(id)` it never loads the receiver, so it quotes a
//! credit that `require_credit_position_limit` rejects at execution. The
//! same payments execute in transfer mode, so the plan itself is sound.

use common::types::{HubAssetKey, SeizeMode};
use controller::types::PositionMode;
use soroban_sdk::testutils::{Address as _, ContractEvents, Events};
use soroban_sdk::xdr::{ContractEventBody, ScVal, ToXdr};
use soroban_sdk::{vec, Address, Bytes, TryFromVal, Vec};
use test_harness::{
    assert_contract_error, errors, f64_to_i128, hub_asset, map_try_ok_value, usd_cents,
    FlashPositionMode, FlashPositionRequest, FlashPositionTestReceiverClient, LendingTest, ALICE,
    BOB, CAROL, HARNESS_SPOKE,
};

/// Owner named by the `["position", "batch_update"]` event for `account_id`
/// in the last invocation. The event data is `[account_id, (owner, spoke_id,
/// mode), deposits, borrows]`.
fn batch_event_owner(t: &LendingTest, account_id: u64) -> Address {
    let events: ContractEvents = t.env.events().all();
    let owner = events
        .events()
        .iter()
        .find_map(|event| {
            let ContractEventBody::V0(body) = &event.body;
            let is_batch = matches!(
                (body.topics.first(), body.topics.get(1)),
                (Some(ScVal::Symbol(a)), Some(ScVal::Symbol(b)))
                    if a.0.to_string() == "position" && b.0.to_string() == "batch_update"
            );
            if !is_batch {
                return None;
            }
            let ScVal::Vec(Some(fields)) = &body.data else {
                return None;
            };
            match fields.first() {
                Some(ScVal::U64(id)) if *id == account_id => {}
                _ => return None,
            }
            let ScVal::Vec(Some(attrs)) = fields.get(1)? else {
                return None;
            };
            let ScVal::Address(owner) = attrs.first()? else {
                return None;
            };
            Some(owner.clone())
        })
        .expect("one batch_update event for the account");
    Address::try_from_val(&t.env, &owner).expect("event owner converts to an Address")
}

#[test]
fn cr19_batch_event_names_the_owner_at_entry_after_a_mid_callback_nft_transfer() {
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

    // Read before any other invocation: events are kept for the last call only.
    let event_owner = batch_event_owner(&t, id);
    assert_eq!(t.nft_owner_of(id), bob, "the callback moved the token");
    assert_eq!(
        event_owner, receiver,
        "HEAD: the batch names the owner captured before the callback"
    );
    assert_ne!(
        event_owner,
        t.nft_owner_of(id),
        "HEAD: the batch owner and the NFT holder disagree at the end of the call"
    );
}

#[test]
fn cr25_credit_estimate_ignores_the_receiver_position_limit_that_liquidate_enforces() {
    let mut t = LendingTest::new()
        .standard_two_asset()
        .with_position_limits(1, 1)
        .build();
    t.supply(ALICE, "USDC", 1_000.0);
    t.borrow(ALICE, "ETH", 0.3);
    // The receiver already holds its single permitted supply leg.
    t.supply(CAROL, "ETH", 1.0);
    let alice_id = t.resolve_account_id(ALICE);
    let receiver_id = t.resolve_account_id(CAROL);
    assert_ne!(alice_id, receiver_id);

    // C = $700, weighted = $560 at the 80% threshold, D = $600: solvent and unhealthy.
    t.set_price("USDC", usd_cents(70));

    let carol = t.get_or_create_user(CAROL);
    let offer = f64_to_i128(0.1, 7);
    t.resolve_market("ETH").token_admin.mint(&carol, &offer);
    let payments = vec![&t.env, (hub_asset(t.resolve_asset("ETH")), offer)];
    let credit = SeizeMode::Credit(receiver_id);

    let estimate = t
        .ctrl_client()
        .get_liquidation_estimate(&alice_id, &payments, &credit);
    assert!(
        estimate.max_payment_wad > 0 && !estimate.seized_collaterals.is_empty(),
        "HEAD: the credit estimate quotes a seizure for a receiver that has no room"
    );
    assert_eq!(
        estimate.seized_collaterals.get(0).unwrap().asset,
        t.resolve_asset("USDC"),
        "the credited leg would open a second supply slot on the receiver"
    );

    let rejected = t
        .ctrl_client()
        .try_liquidate(&carol, &alice_id, &payments, &credit);
    assert_contract_error(map_try_ok_value(rejected), errors::POSITION_LIMIT_EXCEEDED);

    // Control: the same payments execute in transfer mode, so only the
    // receiver gate separates the quote from execution.
    let receiver = t
        .ctrl_client()
        .liquidate(&carol, &alice_id, &payments, &SeizeMode::Transfer);
    assert_eq!(receiver, 0);
}
