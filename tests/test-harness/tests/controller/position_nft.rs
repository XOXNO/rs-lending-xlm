use common::types::SeizeMode;

use test_harness::{
    assert_contract_error, asset_payment_vec, errors, eth_preset, f64_to_i128, usd_cents,
    usdc_preset, LendingTest, ALICE, BOB, HARNESS_SPOKE, LIQUIDATOR,
};

// --- identity & lifecycle -------------------------------------------------

#[test]
fn supply_mints_nft_with_token_id_equal_account_id() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    t.supply(ALICE, "USDC", 1_000.0);
    let account_id = t.account_id(ALICE);
    assert!(
        account_id >= 1,
        "id 0 is the create sentinel and is never minted"
    );
    assert_eq!(t.nft_owner_of(account_id), t.get_or_create_user(ALICE));
}

#[test]
fn emptying_account_burns_nft_and_resupply_mints_fresh_id() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    t.supply(ALICE, "USDC", 1_000.0);
    let first_id = t.account_id(ALICE);
    t.withdraw_all(ALICE, "USDC");
    assert!(
        !t.try_nft_owner_of(first_id),
        "NFT must burn with the account"
    );
    assert!(
        !t.account_exists(first_id),
        "controller meta must die with the NFT -- meta-without-NFT must be unrepresentable \
         through public flows"
    );

    t.supply(ALICE, "USDC", 500.0);
    let second_id = t.account_id(ALICE);
    assert!(second_id > first_id, "ids are never reused");
}

// --- transfer semantics ---------------------------------------------------

#[test]
fn transfer_hands_full_control_to_new_owner() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    t.supply(ALICE, "USDC", 1_000.0);
    let account_id = t.account_id(ALICE);
    let attrs = t.get_account_attributes(ALICE);

    t.nft_transfer(ALICE, BOB, account_id);
    t.adopt_account(BOB, account_id, attrs.spoke_id, attrs.mode);

    assert_eq!(t.nft_owner_of(account_id), t.get_or_create_user(BOB));

    // New owner withdraws.
    t.withdraw(BOB, "USDC", 100.0);
    let bob_supply = t.supply_balance(BOB, "USDC");
    assert!(
        bob_supply <= 900.01,
        "new owner's withdraw must have reduced the transferred collateral, got {}",
        bob_supply
    );
}

#[test]
fn old_owner_is_rejected_after_transfer() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    t.supply(ALICE, "USDC", 1_000.0);
    let account_id = t.account_id(ALICE);
    t.nft_transfer(ALICE, BOB, account_id);

    // ALICE's registered account id still points at the transferred account;
    // an owner-gated withdraw must now fail NotAuthorized.
    let raw = f64_to_i128(100.0, t.resolve_market("USDC").decimals);
    let result = t.try_withdraw_raw(ALICE, "USDC", raw);
    assert_contract_error(result, errors::NOT_AUTHORIZED);
}

#[test]
fn transfer_revokes_old_owners_delegates() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    t.supply(ALICE, "USDC", 1_000.0);
    let account_id = t.account_id(ALICE);
    t.enable_delegate(ALICE, "MANAGER", account_id);

    t.nft_transfer(ALICE, BOB, account_id);

    // The manager's grant was stamped by ALICE; with BOB as owner it is dead.
    let result = t.try_borrow_as_to("MANAGER", account_id, "USDC", 10.0, "MANAGER");
    assert_contract_error(result, errors::NOT_AUTHORIZED);
}

#[test]
fn new_owner_grant_overwrites_stale_grant() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    t.supply(ALICE, "USDC", 1_000.0);
    let account_id = t.account_id(ALICE);
    let attrs = t.get_account_attributes(ALICE);
    t.enable_delegate(ALICE, "MANAGER", account_id);
    t.nft_transfer(ALICE, BOB, account_id);
    t.adopt_account(BOB, account_id, attrs.spoke_id, attrs.mode);

    // BOB re-grants; the stale ALICE grant is overwritten wholesale.
    t.enable_delegate(BOB, "MANAGER", account_id);
    t.borrow_as_to("MANAGER", account_id, "USDC", 10.0, BOB); // must succeed now

    assert!(
        t.borrow_balance_for(BOB, account_id, "USDC") > 9.0,
        "manager's borrow-as-to-BOB under the fresh grant must have landed"
    );
}

#[test]
fn debt_travels_with_the_token() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    t.supply(ALICE, "USDC", 1_000.0);
    t.borrow(ALICE, "USDC", 200.0);
    let account_id = t.account_id(ALICE);
    let attrs = t.get_account_attributes(ALICE);

    t.nft_transfer(ALICE, BOB, account_id);
    t.adopt_account(BOB, account_id, attrs.spoke_id, attrs.mode);

    // BOB repays the debt he acquired and can then withdraw the collateral.
    t.repay(BOB, "USDC", 250.0);
    t.withdraw_all(BOB, "USDC");
    assert!(!t.try_nft_owner_of(account_id));
}

// --- guards & boundaries --------------------------------------------------

#[test]
fn unknown_and_unmintable_account_ids_are_account_not_found() {
    // `try_supply_to` isn't used here: it pre-resolves the target account's
    // spoke id via `get_account_attributes`, which for an unknown id fails
    // with AccountNotInMarket -- a harness-internal artifact, not the
    // controller's real answer. The real `supply` entry point only reaches
    // AccountNotFound inside `load_or_create_account`, after that spoke
    // lookup would already have failed differently, so we call `try_supply`
    // directly to observe the controller's actual error.
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    t.supply(ALICE, "USDC", 1_000.0);

    let alice_addr = t.get_or_create_user(ALICE);
    let asset_addr = t.resolve_asset("USDC");

    for bad_id in [9_999u64, u64::from(u32::MAX) + 1, u64::MAX] {
        let assets = asset_payment_vec(&t.env, asset_addr.clone(), 10_000_000i128);
        let result = match t
            .ctrl_client()
            .try_supply(&alice_addr, &bad_id, &HARNESS_SPOKE, &assets)
        {
            Ok(Ok(id)) => Ok(id),
            Ok(Err(err)) => Err(err),
            Err(e) => Err(e.expect("expected contract error, got InvokeError")),
        };
        assert_contract_error(result, errors::ACCOUNT_NOT_FOUND);
    }
}

#[test]
fn self_liquidation_is_allowed() {
    // NFT-angle variant of liquidation.rs's `test_self_liquidation_allowed`
    // (Task 5): the same unhealthy-position setup, but the account is
    // transferred to BOB before he liquidates himself as its new owner.
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    let account_id = t.account_id(ALICE);
    let attrs = t.get_account_attributes(ALICE);
    t.set_price("USDC", usd_cents(50));
    t.assert_liquidatable(ALICE);

    t.nft_transfer(ALICE, BOB, account_id);
    t.adopt_account(BOB, account_id, attrs.spoke_id, attrs.mode);

    t.liquidate(BOB, BOB, "ETH", 0.5);

    assert!(
        t.borrow_balance(BOB, "ETH") < 3.0,
        "self-liquidation as the NFT's new owner must reduce the account's own debt"
    );
}

#[test]
fn underwater_position_transfers_freely_and_stays_liquidatable() {
    // Same unhealthy-position setup as liquidation.rs; the account is
    // transferred but never adopted into a harness user's bookkeeping, then
    // a third party liquidates it keyed directly by account_id -- owner
    // identity is irrelevant to solvency machinery.
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    let account_id = t.account_id(ALICE);
    t.set_price("USDC", usd_cents(50));
    t.assert_liquidatable(ALICE);

    t.nft_transfer(ALICE, BOB, account_id);
    assert_eq!(t.nft_owner_of(account_id), t.get_or_create_user(BOB));

    let debt_before = t.borrow_balance_for(BOB, account_id, "ETH");

    let liquidator_addr = t.get_or_create_user(LIQUIDATOR);
    let eth_decimals = t.resolve_market("ETH").decimals;
    let eth_asset = t.resolve_asset("ETH");
    let raw_amount = f64_to_i128(0.5, eth_decimals);
    t.resolve_market("ETH")
        .token_admin
        .mint(&liquidator_addr, &raw_amount);

    let ctrl = t.ctrl_client();
    let payments = asset_payment_vec(&t.env, eth_asset, raw_amount);
    ctrl.liquidate(
        &liquidator_addr,
        &account_id,
        &payments,
        &SeizeMode::Transfer,
    );

    assert!(
        t.borrow_balance_for(BOB, account_id, "ETH") < debt_before,
        "third-party liquidation keyed by account_id must reduce debt regardless of NFT owner"
    );
}

#[test]
fn deploy_position_nft_is_set_once() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    // The builder already deployed it; a second deploy must reject.
    let ctrl = t.ctrl_client();
    let result = ctrl.try_deploy_position_nft(
        &t.position_nft_wasm_hash,
        &soroban_sdk::String::from_str(&t.env, "u"),
        &soroban_sdk::String::from_str(&t.env, "n"),
        &soroban_sdk::String::from_str(&t.env, "s"),
    );
    let mapped: Result<soroban_sdk::Address, soroban_sdk::Error> = match result {
        Ok(res) => res.map_err(|e| e.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    };
    assert_contract_error(mapped, errors::POSITION_NFT_ALREADY_DEPLOYED);
}

#[test]
fn renew_account_follows_current_owner() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    t.supply(ALICE, "USDC", 1_000.0);
    let account_id = t.account_id(ALICE);
    t.nft_transfer(ALICE, BOB, account_id);

    let alice = t.get_or_create_user(ALICE);
    let bob = t.get_or_create_user(BOB);

    let result = t.ctrl_client().try_renew_account(&alice, &account_id);
    let mapped: Result<(), soroban_sdk::Error> = match result {
        Ok(res) => res.map_err(|e| e.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    };
    assert_contract_error(mapped, errors::ACCOUNT_NOT_IN_MARKET);

    t.ctrl_client().renew_account(&bob, &account_id); // succeeds
}
