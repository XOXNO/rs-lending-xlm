//! Regression coverage retained from archived agent worktrees.

use common::types::{ControllerKey, HubAssetKey};
use soroban_sdk::testutils::{MockAuth, MockAuthInvoke};
use soroban_sdk::{token, vec, Address, Bytes, IntoVal, Vec};
use test_harness::{
    days, eth_preset, hub_asset, map_try_ok_unit, map_try_ok_value, usd, usdc_preset, LendingTest,
    MarketPreset, ALICE, BOB, CAROL, DEFAULT_ASSET_CONFIG, DEFAULT_MARKET_PARAMS, HARNESS_HUB,
    HARNESS_SPOKE,
};

type Legs = Vec<(HubAssetKey, i128)>;
type TryResult<T, E> = Result<Result<T, E>, Result<soroban_sdk::Error, soroban_sdk::InvokeError>>;
type Books = (
    soroban_sdk::Map<HubAssetKey, common::types::AccountPositionRaw>,
    soroban_sdk::Map<HubAssetKey, common::types::DebtPositionRaw>,
);

const MALLORY: &str = "mallory";
const ROUNDS: u32 = 40;
const HOUR: u64 = 3_600;
const LOAN: i128 = 10_000_000;

fn unseeded(name: &'static str, decimals: u32, price_wad: i128) -> MarketPreset {
    MarketPreset {
        name,
        decimals,
        price_wad,
        initial_liquidity: 0.0,
        config: DEFAULT_ASSET_CONFIG,
        params: DEFAULT_MARKET_PARAMS,
    }
}

fn key(t: &LendingTest, asset: &str, hub_id: u32) -> HubAssetKey {
    HubAssetKey {
        hub_id,
        asset: t.resolve_asset(asset),
    }
}

fn mint(t: &mut LendingTest, user: &str, asset: &str, amount: i128) -> Address {
    let addr = t.get_or_create_user(user);
    t.resolve_market(asset).token_admin.mint(&addr, &amount);
    addr
}

fn scaled_supply(t: &LendingTest, account_id: u64, k: &HubAssetKey) -> i128 {
    let (supplies, _) = t.ctrl_client().get_account_positions(&account_id);
    supplies.get(k.clone()).map_or(0, |p| p.scaled_amount)
}

fn scaled_debt(t: &LendingTest, account_id: u64, k: &HubAssetKey) -> i128 {
    let (_, borrows) = t.ctrl_client().get_account_positions(&account_id);
    borrows.get(k.clone()).map_or(0, |p| p.scaled_amount)
}

fn try_supply(
    t: &LendingTest,
    caller: &Address,
    account_id: u64,
    legs: &Legs,
) -> Result<u64, soroban_sdk::Error> {
    map_try_ok_value(
        t.ctrl_client()
            .try_supply(caller, &account_id, &HARNESS_SPOKE, legs),
    )
}

fn try_borrow(
    t: &LendingTest,
    caller: &Address,
    account_id: u64,
    legs: &Legs,
) -> Result<(), soroban_sdk::Error> {
    map_try_ok_unit(t.ctrl_client().try_borrow(caller, &account_id, legs, &None))
}

fn try_withdraw(
    t: &LendingTest,
    caller: &Address,
    account_id: u64,
    legs: &Legs,
) -> Result<Legs, soroban_sdk::Error> {
    match t
        .ctrl_client()
        .try_withdraw(caller, &account_id, legs, &None)
    {
        Ok(Ok(paid)) => Ok(paid),
        Ok(Err(err)) => Err(err.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    }
}

#[test]
fn one_unit_legs_never_extract_value_at_6_7_8_and_18_decimals() {
    let markets: [(&str, u32, i128, f64, &str); 4] = [
        ("USDC6", 6, usd(1), 100_000.0, "user6"),
        ("XLM7", 7, usd(1) / 10, 1_000_000.0, "user7"),
        ("WBTC8", 8, usd(60_000), 2.0, "user8"),
        ("DAI18", 18, usd(1), 100_000.0, "user18"),
    ];
    let mut builder = LendingTest::new()
        .with_market(eth_preset())
        .with_position_limits(4, 4)
        .with_min_borrow_collateral_disabled();
    for (name, decimals, price, _, _) in markets {
        builder = builder.with_market(unseeded(name, decimals, price));
    }
    let mut t = builder.build();

    t.supply(BOB, "ETH", 1_000.0);
    for (name, _, _, depth, _) in markets {
        t.supply(CAROL, name, depth);
        t.borrow(BOB, name, depth * 0.6);
    }
    t.advance_and_sync(days(365));

    for (name, decimals, _, _, user) in markets {
        let k = key(&t, name, HARNESS_HUB);
        let one = 10i128.pow(decimals);
        let sync = t.pool_client(name).get_sync_data(&k).state;
        assert!(
            sync.supply_index > controller::constants::RAY && sync.borrow_index > sync.supply_index,
            "{name}: both indexes must have grown"
        );

        // Fresh-account round trips: deposit then withdraw-all by the zero leg.
        for deposit in [1, 2, 3, 7, one - 1, one, one + 1] {
            let addr = mint(&mut t, user, name, deposit);
            let before = t.token_balance_raw(user, name);
            let id = try_supply(&t, &addr, 0, &vec![&t.env, (k.clone(), deposit)])
                .unwrap_or_else(|e| panic!("{name}: supply {deposit} failed: {e:?}"));
            let paid = try_withdraw(&t, &addr, id, &vec![&t.env, (k.clone(), 0)])
                .unwrap_or_else(|e| panic!("{name}: withdraw-all of {deposit} failed: {e:?}"));
            let out = t.token_balance_raw(user, name) - (before - deposit);
            assert_eq!(paid.get(0).unwrap().1, out, "{name}: reported payout");
            assert!(
                out <= deposit && deposit - out <= 1,
                "{name}: deposit {deposit} returned {out}"
            );
            assert!(!t.account_exists(id), "{name}: the emptied account burns");
        }

        // A borrowed unit books debt and costs at least one unit to clear.
        t.supply(user, "ETH", 1.0);
        let id = t.resolve_account_id(user);
        let addr = t.get_or_create_user(user);
        let wallet0 = t.token_balance_raw(user, name);
        try_borrow(&t, &addr, id, &vec![&t.env, (k.clone(), 1)]).unwrap();
        assert_eq!(t.token_balance_raw(user, name), wallet0 + 1);
        assert!(
            scaled_debt(&t, id, &k) > 0,
            "{name}: 1-unit borrow booked no debt"
        );
        assert!(t.ctrl_client().get_borrow_amount(&id, &k) >= 1);

        // Borrow/repay one unit twenty-five times: wallet unchanged, debt never shrinks.
        let debt_scaled0 = scaled_debt(&t, id, &k);
        for _ in 0..25 {
            try_borrow(&t, &addr, id, &vec![&t.env, (k.clone(), 1)]).unwrap();
            t.ctrl_client()
                .repay(&addr, &id, &vec![&t.env, (k.clone(), 1)]);
        }
        assert_eq!(t.token_balance_raw(user, name), wallet0 + 1);
        assert!(
            scaled_debt(&t, id, &k) >= debt_scaled0,
            "{name}: unit borrow/repay loop reduced debt shares"
        );

        // Close with an overpayment: net cost >= the one unit received.
        let overpay = 1_000;
        mint(&mut t, user, name, overpay);
        let wallet_before_close = t.token_balance_raw(user, name);
        t.ctrl_client()
            .repay(&addr, &id, &vec![&t.env, (k.clone(), overpay)]);
        assert_eq!(scaled_debt(&t, id, &k), 0, "{name}: debt must close");
        let close_cost = wallet_before_close - t.token_balance_raw(user, name);
        assert!(
            close_cost >= 1,
            "{name}: closed 1-unit debt for {close_cost}"
        );

        // Supply/withdraw one unit twenty-five times against a standing position.
        mint(&mut t, user, name, 1_000 * one);
        t.ctrl_client().supply(
            &addr,
            &id,
            &HARNESS_SPOKE,
            &vec![&t.env, (k.clone(), 1_000 * one)],
        );
        let supply_scaled0 = scaled_supply(&t, id, &k);
        let wallet1 = t.token_balance_raw(user, name);
        for _ in 0..25 {
            mint(&mut t, user, name, 1);
            t.ctrl_client()
                .supply(&addr, &id, &HARNESS_SPOKE, &vec![&t.env, (k.clone(), 1)]);
            let paid = try_withdraw(&t, &addr, id, &vec![&t.env, (k.clone(), 1)]).unwrap();
            assert_eq!(paid.get(0).unwrap().1, 1);
        }
        assert_eq!(t.token_balance_raw(user, name), wallet1 + 25);
        assert!(
            scaled_supply(&t, id, &k) <= supply_scaled0,
            "{name}: unit supply/withdraw loop grew supply shares"
        );
    }
    t.assert_spoke_usage_matches_positions();
}

#[test]
fn same_token_on_two_hubs_settles_each_hub_exactly_in_one_batch() {
    let mut t = LendingTest::new()
        .with_market(unseeded("USDC", 7, usd(1)))
        .with_market(eth_preset())
        .with_min_borrow_collateral_disabled()
        .build();
    let hub2 = t.create_hub();
    t.list_market_on_hub(hub2, "USDC", 0.0);
    let h1 = key(&t, "USDC", HARNESS_HUB);
    let h2 = key(&t, "USDC", hub2);
    let unit = 10_000_000i128;

    // Supplier: one batch, duplicate hub-1 legs, one hub-2 leg.
    let carol = mint(&mut t, CAROL, "USDC", 20_000 * unit);
    let cash = |t: &LendingTest, hub: u32| t.pool_state_on_hub(hub, "USDC").cash;
    let (c1, c2) = (cash(&t, HARNESS_HUB), cash(&t, hub2));
    let legs = vec![
        &t.env,
        (h1.clone(), 6_000 * unit),
        (h2.clone(), 10_000 * unit),
        (h1.clone(), 4_000 * unit),
    ];
    let carol_id = try_supply(&t, &carol, 0, &legs).unwrap();
    assert_eq!(t.token_balance_raw(CAROL, "USDC"), 0);
    assert_eq!(cash(&t, HARNESS_HUB) - c1, 10_000 * unit);
    assert_eq!(cash(&t, hub2) - c2, 10_000 * unit);
    let (supplies, _) = t.ctrl_client().get_account_positions(&carol_id);
    assert_eq!(
        supplies.len(),
        2,
        "duplicate hub-1 legs merge into one slot"
    );

    // Borrower: one batch across both hubs, received exactly the sum.
    t.supply(ALICE, "ETH", 10.0);
    let alice_id = t.resolve_account_id(ALICE);
    let alice = t.get_or_create_user(ALICE);
    let legs = vec![
        &t.env,
        (h1.clone(), 3_000 * unit),
        (h2.clone(), 2_000 * unit),
        (h1.clone(), 1_000 * unit),
    ];
    try_borrow(&t, &alice, alice_id, &legs).unwrap();
    assert_eq!(t.token_balance_raw(ALICE, "USDC"), 6_000 * unit);
    t.assert_spoke_usage_matches_positions();

    t.advance_time(days(30));
    t.update_indexes_on_hub(HARNESS_HUB, &["USDC"]);
    t.update_indexes_on_hub(hub2, &["USDC"]);

    // Third-party repay: hub 1 overpaid, hub 2 partial.
    let d1 = t.ctrl_client().get_borrow_amount(&alice_id, &h1);
    let d2 = t.ctrl_client().get_borrow_amount(&alice_id, &h2);
    assert!(d1 > 4_000 * unit && d2 > 2_000 * unit, "interest accrued");
    let pay1 = d1 + 1_000 * unit;
    let pay2 = 500 * unit;
    let bob = mint(&mut t, BOB, "USDC", pay1 + pay2);
    let (c1, c2) = (cash(&t, HARNESS_HUB), cash(&t, hub2));
    let alice_wallet = t.token_balance_raw(ALICE, "USDC");
    t.ctrl_client().repay(
        &bob,
        &alice_id,
        &vec![&t.env, (h1.clone(), pay1), (h2.clone(), pay2)],
    );
    let bob_spent = pay1 + pay2 - t.token_balance_raw(BOB, "USDC");
    let (dc1, dc2) = (cash(&t, HARNESS_HUB) - c1, cash(&t, hub2) - c2);
    assert_eq!(dc2, pay2, "partial leg credits exactly its payment");
    assert!(
        (d1..=d1 + 1).contains(&dc1),
        "hub-1 credit {dc1} must be the ceiled debt ~{d1}"
    );
    assert_eq!(
        bob_spent,
        dc1 + dc2,
        "payer is refunded exactly the overpayment"
    );
    assert_eq!(
        t.token_balance_raw(ALICE, "USDC"),
        alice_wallet,
        "owner untouched"
    );
    assert_eq!(scaled_debt(&t, alice_id, &h1), 0);
    let d2_after = t.ctrl_client().get_borrow_amount(&alice_id, &h2);
    assert!(
        d2_after >= d2 - pay2,
        "partial repay cleared more than it paid"
    );
    t.assert_spoke_usage_matches_positions();

    // Owner clears hub 2 with an overpayment; refund goes to the owner.
    mint(&mut t, ALICE, "USDC", 1_000 * unit);
    let w = t.token_balance_raw(ALICE, "USDC");
    let c2 = cash(&t, hub2);
    t.ctrl_client().repay(
        &alice,
        &alice_id,
        &vec![&t.env, (h2.clone(), d2_after + 100 * unit)],
    );
    assert_eq!(w - t.token_balance_raw(ALICE, "USDC"), cash(&t, hub2) - c2);
    assert_eq!(scaled_debt(&t, alice_id, &h2), 0);

    // Supplier exits both hubs: positive hub-1 leg, zero hub-2 leg, zero hub-1 leg.
    let v1 = t.ctrl_client().get_collateral_amount(&carol_id, &h1);
    let v2 = t.ctrl_client().get_collateral_amount(&carol_id, &h2);
    let legs = vec![
        &t.env,
        (h1.clone(), 1_000 * unit),
        (h2.clone(), 0),
        (h1.clone(), 0),
    ];
    let paid = try_withdraw(&t, &carol, carol_id, &legs).unwrap();
    assert_eq!(paid.len(), 2);
    let out = t.token_balance_raw(CAROL, "USDC");
    assert_eq!(out, paid.get(0).unwrap().1 + paid.get(1).unwrap().1);
    assert!(out >= 20_000 * unit, "suppliers earned interest");
    assert!(
        out <= v1 + v2,
        "payout {out} above the half-up claim {}",
        v1 + v2
    );
    assert!(!t.account_exists(carol_id), "both hubs closed");
    t.assert_spoke_usage_matches_positions();
}

fn flat<T, E: Into<soroban_sdk::Error>>(r: TryResult<T, E>) -> Result<T, soroban_sdk::Error> {
    match r {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(e.into()),
        Err(Ok(e)) => Err(e),
        Err(Err(e)) => panic!("unexpected invoke error {e:?}"),
    }
}

fn one(t: &LendingTest, asset: &str, amount: i128) -> Vec<(HubAssetKey, i128)> {
    vec![&t.env, (hub_asset(t.resolve_asset(asset)), amount)]
}

fn books(t: &LendingTest, id: u64) -> Books {
    t.ctrl_client().get_account_positions(&id)
}

fn alice_with_debt() -> (LendingTest, u64) {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 1.0);
    let id = t.resolve_account_id(ALICE);
    (t, id)
}

#[test]
fn only_the_named_callers_signature_satisfies_the_account_check() {
    let (mut t, id) = alice_with_debt();
    let alice = t.get_or_create_user(ALICE);
    let mallory = t.get_or_create_user(MALLORY);
    let borrows = one(&t, "ETH", 1_000);
    let none: Option<Address> = None;
    let args = (alice.clone(), id, borrows.clone(), none.clone()).into_val(&t.env);
    let invoke = MockAuthInvoke {
        contract: &t.controller,
        fn_name: "borrow",
        args,
        sub_invokes: &[],
    };
    let before = books(&t, id);

    // Mallory signs a call that names ALICE as caller.
    t.env.mock_auths(&[MockAuth {
        address: &mallory,
        invoke: &invoke,
    }]);
    let forged = flat(t.ctrl_client().try_borrow(&alice, &id, &borrows, &none));
    assert!(
        forged.is_err(),
        "a foreign signature must not pass as ALICE's"
    );
    assert_eq!(books(&t, id), before);

    // Positive control: the same tree signed by ALICE succeeds.
    t.env.mock_auths(&[MockAuth {
        address: &alice,
        invoke: &invoke,
    }]);
    flat(t.ctrl_client().try_borrow(&alice, &id, &borrows, &none)).expect("ALICE signed");
    assert_ne!(books(&t, id), before);
}

#[test]
fn a_full_third_party_repay_keeps_the_supply_side_and_the_account() {
    // Repay loads debt only; it must never persist or prune the unloaded supply side.
    let (mut t, id) = alice_with_debt();
    let supply_before = books(&t, id).0;
    let mallory = t.get_or_create_user(MALLORY);
    t.resolve_market("ETH")
        .token_admin
        .mint(&mallory, &10_000_000_000);
    t.ctrl_client()
        .repay(&mallory, &id, &one(&t, "ETH", 2_000_000_000));

    let (supply, debt) = books(&t, id);
    assert!(debt.is_empty());
    assert_eq!(supply, supply_before);
    assert!(t.account_exists(id));
    assert_eq!(t.nft_owner_of(id), t.get_or_create_user(ALICE));
    t.withdraw_all(ALICE, "USDC");
    assert!(!t.account_exists(id));
}

fn account_keys(account_id: u64) -> [ControllerKey; 4] {
    [
        ControllerKey::AccountMeta(account_id),
        ControllerKey::SupplyPositions(account_id),
        ControllerKey::BorrowPositions(account_id),
        ControllerKey::Delegates(account_id),
    ]
}

#[test]
fn auto_removed_account_leaves_no_key_and_id_is_not_reused() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 0.5);
    let old_id = t.resolve_account_id(ALICE);
    t.enable_delegate(ALICE, CAROL, old_id);
    t.env.as_contract(&t.controller, || {
        let p = t.env.storage().persistent();
        assert!(account_keys(old_id).iter().all(|k| p.has(k)));
    });

    t.repay(ALICE, "ETH", 1.0);
    t.withdraw_all(ALICE, "USDC");

    t.env.as_contract(&t.controller, || {
        let p = t.env.storage().persistent();
        for key in account_keys(old_id) {
            assert!(!p.has(&key), "{key:?} survived account removal");
        }
    });
    assert!(!t.try_nft_owner_of(old_id));
    assert!(!t.ctrl_client().account_exists(&old_id));

    t.supply(ALICE, "USDC", 100.0);
    let new_id = t.resolve_account_id(ALICE);
    assert!(
        new_id > old_id,
        "burned id {old_id} was reissued as {new_id}"
    );
    t.env.as_contract(&t.controller, || {
        assert!(!t
            .env
            .storage()
            .persistent()
            .has(&ControllerKey::Delegates(new_id)));
    });
}

fn seeded() -> LendingTest {
    let mut t = LendingTest::new().standard_two_asset_dust_disabled();
    t.supply(BOB, "ETH", 100.0);
    t.supply(ALICE, "USDC", 400_000.0);
    t.borrow(ALICE, "ETH", 60.0);
    t
}

struct Outcome {
    bob_supply: i128,
    alice_debt: i128,
    mallory_net: i128,
}

fn run(attack: bool, donate: bool) -> Outcome {
    let mut t = seeded();
    let mut minted = 0i128;
    let unit = 10i128.pow(t.resolve_market("ETH").decimals);
    for round in 0..ROUNDS {
        t.advance_time(HOUR + u64::from(round));
        t.update_indexes_for(&["ETH", "USDC"]);
        if donate && round == 0 {
            let mallory = t.get_or_create_user(MALLORY);
            let market = t.resolve_market("ETH");
            market.token_admin.mint(&mallory, &(500 * unit));
            minted += 500 * unit;
            token::Client::new(&t.env, &market.asset).transfer(
                &mallory,
                &market.pool,
                &(500 * unit),
            );
        }
        if attack {
            t.supply_raw(MALLORY, "ETH", 1_000 * unit);
            minted += 1_000 * unit;
            t.withdraw_all(MALLORY, "ETH");
            t.supply_raw(MALLORY, "ETH", 1);
            minted += 1;
            t.withdraw_all(MALLORY, "ETH");
        }
    }
    t.update_indexes_for(&["ETH", "USDC"]);
    let mallory_net = if attack {
        t.token_balance_raw(MALLORY, "ETH") - minted
    } else {
        0
    };
    Outcome {
        bob_supply: t.supply_balance_raw(BOB, "ETH"),
        alice_debt: t.borrow_balance_raw(ALICE, "ETH"),
        mallory_net,
    }
}

#[test]
fn just_in_time_supply_and_donation_cycles_do_not_move_the_exchange_rate() {
    let control = run(false, false);
    let cycled = run(true, false);
    let donated = run(true, true);

    assert!(control.bob_supply > 100 * 10i128.pow(7));
    assert_eq!(cycled.alice_debt, control.alice_debt);
    assert!(cycled.bob_supply >= control.bob_supply);
    assert!(cycled.bob_supply - control.bob_supply <= i128::from(2 * ROUNDS));
    assert!(cycled.mallory_net <= 0);

    assert_eq!(donated.alice_debt, control.alice_debt);
    assert_eq!(donated.bob_supply, cycled.bob_supply);
    assert!(donated.mallory_net <= -500 * 10i128.pow(7));
}

fn attempt(t: &mut LendingTest, receiver: &Address) -> bool {
    let caller = t.get_or_create_user(BOB);
    let asset = hub_asset(t.resolve_asset("ETH"));
    t.ctrl_client()
        .try_flash_loan(&caller, &asset, &LOAN, receiver, &Bytes::new(&t.env))
        .is_ok_and(|inner| inner.is_ok())
}

#[test]
fn controller_and_pool_cannot_be_flash_loan_receivers() {
    let mut t = LendingTest::new().standard_two_asset_dust_disabled();
    let pool_addr = t.resolve_market("ETH").pool.clone();
    let asset_addr = t.resolve_market("ETH").asset.clone();
    let controller = t.controller_address();
    let key = hub_asset(asset_addr.clone());
    let asset = token::Client::new(&t.env, &asset_addr);

    let pool_before = asset.balance(&pool_addr);
    let controller_before = asset.balance(&controller);
    let state_before = t.pool_client("ETH").get_sync_data(&key).state;

    assert!(!attempt(&mut t, &controller));
    assert!(!attempt(&mut t, &pool_addr));

    assert_eq!(asset.balance(&pool_addr), pool_before);
    assert_eq!(asset.balance(&controller), controller_before);
    let state_after = t.pool_client("ETH").get_sync_data(&key).state;
    assert_eq!(state_after.cash, state_before.cash);
    assert_eq!(state_after.supplied, state_before.supplied);
    assert_eq!(state_after.revenue, state_before.revenue);

    let honest = t.deploy_flash_loan_receiver();
    assert!(attempt(&mut t, &honest));
}
