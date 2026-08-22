//! Boundary probes for the dust / precision surface the existing suites skip:
//! the `MIN_ASSET_DECIMALS` endpoint (3), and the first depositor into a market
//! drained after its supply index had already grown.
//!
//! The harness mints on `supply_raw`, so supply legs are measured on the
//! position book; token balances are only meaningful across an exit.

use soroban_sdk::Vec;
use test_harness::presets::{MarketPreset, DEFAULT_ASSET_CONFIG, DEFAULT_MARKET_PARAMS};
use test_harness::{helpers::usd, hub_asset, LendingTest, ALICE, BOB, CAROL};

/// `MIN_ASSET_DECIMALS`. No preset anywhere in the harness uses 3 -- the suite
/// only exercises 6, 7, 8, 9, 14 and 18.
fn low3() -> MarketPreset {
    MarketPreset {
        name: "LOW3",
        decimals: 3,
        price_wad: usd(1),
        initial_liquidity: 1_000_000.0,
        config: DEFAULT_ASSET_CONFIG,
        params: DEFAULT_MARKET_PARAMS,
    }
}

fn usdc6() -> MarketPreset {
    MarketPreset {
        name: "USDC6",
        decimals: 6,
        price_wad: usd(1),
        initial_liquidity: 1_000_000.0,
        config: DEFAULT_ASSET_CONFIG,
        params: DEFAULT_MARKET_PARAMS,
    }
}

fn supply_index(t: &LendingTest, name: &str) -> i128 {
    let asset = t.resolve_asset(name);
    let assets = Vec::from_array(&t.env, [hub_asset(asset)]);
    t.ctrl_client()
        .get_market_indexes_detailed(&assets)
        .get(0)
        .unwrap()
        .supply_index
}

/// One raw unit at 3 decimals is 0.001 whole tokens -- the coarsest granularity
/// the protocol admits. Supply / borrow / repay / withdraw must each move the
/// book by exactly that unit and leave nothing stranded.
#[test]
fn three_decimal_market_one_raw_unit_round_trip() {
    let mut t = LendingTest::new()
        .with_market(low3())
        .with_market(usdc6())
        .with_min_borrow_collateral_disabled()
        .with_max_utilization_disabled_all_markets()
        .build();

    t.supply(BOB, "LOW3", 10_000.0);
    t.supply(ALICE, "USDC6", 1_000.0);

    t.supply_raw(ALICE, "LOW3", 1);
    assert_eq!(
        t.supply_balance_raw(ALICE, "LOW3"),
        1,
        "3dec supply of 1 raw unit must credit exactly 1"
    );

    t.borrow_raw(ALICE, "LOW3", 1);
    assert_eq!(
        t.borrow_balance_raw(ALICE, "LOW3"),
        1,
        "3dec borrow of 1 raw unit must record exactly 1"
    );

    t.repay_raw(ALICE, "LOW3", 1);
    assert_eq!(
        t.borrow_balance_raw(ALICE, "LOW3"),
        0,
        "repaying the 1 raw unit must clear the debt leg"
    );

    let tokens_before_exit = t.token_balance_raw(ALICE, "LOW3");
    t.withdraw_raw(ALICE, "LOW3", 1);
    assert_eq!(
        t.supply_balance_raw(ALICE, "LOW3"),
        0,
        "withdrawing the 1 raw unit must clear the supply leg"
    );
    assert_eq!(
        t.token_balance_raw(ALICE, "LOW3"),
        tokens_before_exit + 1,
        "the 3dec exit must pay back exactly the 1 raw unit supplied"
    );
}

/// The first depositor into a market whose supply index has already grown, after
/// every other supplier has left. The classic inflation-attack shape: if a first
/// deposit could mint shares worth more than it paid, this is where it shows.
#[test]
fn first_depositor_after_index_growth_cannot_mint_free_value() {
    let mut t = LendingTest::new()
        .with_market(usdc6())
        .with_market(low3())
        .with_min_borrow_collateral_disabled()
        .with_max_utilization_disabled_all_markets()
        .build();

    t.supply(BOB, "USDC6", 10_000.0);
    t.supply(CAROL, "LOW3", 100_000.0);
    t.borrow(CAROL, "USDC6", 8_000.0);

    let mut grown = false;
    for _ in 0..400 {
        if supply_index(&t, "USDC6") >= 3 * controller::constants::RAY / 2 {
            grown = true;
            break;
        }
        t.advance_and_sync(30 * 86_400);
    }
    let idx = supply_index(&t, "USDC6");
    assert!(grown, "USDC6 supply index never grew to 1.5x RAY: {idx}");

    // Drain: Carol repays everything, Bob withdraws everything.
    let owed = t.borrow_balance_raw(CAROL, "USDC6");
    t.repay_raw(CAROL, "USDC6", owed);
    t.withdraw_all(BOB, "USDC6");

    let post_drain_idx = supply_index(&t, "USDC6");
    std::println!("post-drain USDC6 supply index = {post_drain_idx} (grew from RAY)");

    // First depositor into the drained market.
    let paid = 1_000_000i128; // 1.0 USDC6
    t.supply(ALICE, "LOW3", 10.0); // creates alice's account
    t.supply_raw(ALICE, "USDC6", paid);

    let credited = t.supply_balance_raw(ALICE, "USDC6");
    assert!(
        credited <= paid,
        "first depositor was credited {credited} for a {paid} deposit -- \
         a deposit must never be worth more than it paid"
    );

    let before_exit = t.token_balance_raw(ALICE, "USDC6");
    t.withdraw_all(ALICE, "USDC6");
    let got_back = t.token_balance_raw(ALICE, "USDC6") - before_exit;
    std::println!("first depositor: paid={paid} credited={credited} got_back={got_back}");
    assert!(
        got_back <= paid,
        "first-deposit-then-withdraw round trip minted value: \
         paid={paid} got_back={got_back}"
    );
}

/// End-to-end sweep of the permissionless `clean_bad_debt` gate across the
/// `BAD_DEBT_USD_THRESHOLD` line, moving the account by **price alone**.
///
/// The gate is WAD USD, so an oracle move -- not a user action -- decides which
/// side of it an account sits on. The property that has to hold is that a price
/// move can never open the gate on an account that is still solvent: every
/// price at which `clean_bad_debt` succeeds must already be a price at which
/// the account was liquidatable. `is_socializable_bad_debt` requires
/// `total_debt > total_collateral`, which is strictly stronger than `HF < 1`
/// for any liquidation threshold below 100%, so the gate should never be the
/// first thing that opens.
///
/// The certora rules and `contracts/controller/tests/positions/liquidation_curve.rs`
/// pin the predicate; nothing drove it end to end through the oracle.
#[test]
fn clean_bad_debt_gate_never_opens_before_liquidation_does() {
    let mut t = LendingTest::new()
        .with_market(usdc6())
        .with_market(low3())
        .with_min_borrow_collateral_disabled()
        .with_max_utilization_disabled_all_markets()
        .build();

    t.supply(BOB, "LOW3", 100_000.0);
    t.supply(ALICE, "USDC6", 20.0);
    t.borrow(ALICE, "LOW3", 10.0);
    let alice_id = t.resolve_account_id(ALICE);

    // Walk the USDC6 price down. Each step re-prices Alice's collateral only.
    let mut opened_at: Option<i128> = None;
    for step in 1..=40i128 {
        let price = usd(1) - (usd(1) * step) / 40; // 1.00 -> 0.025
        if price <= 0 {
            break;
        }
        t.set_price("USDC6", price);

        let collateral = t.total_collateral_raw(ALICE);
        let debt = t.total_debt_raw(ALICE);
        let liquidatable = t.can_be_liquidated(ALICE);
        let gate_open = debt > collateral && collateral <= controller::constants::WAD * 5;

        if gate_open {
            assert!(
                liquidatable,
                "clean_bad_debt gate opened at price={price} \
                 (collateral={collateral} debt={debt}) while the account was NOT \
                 liquidatable -- a price move must never open the dust gate first"
            );
            if opened_at.is_none() {
                opened_at = Some(price);
                std::println!("gate opened at price={price} collateral={collateral} debt={debt}");
            }
        }
    }

    let opened = opened_at.expect("the price walk never crossed the dust gate");
    assert!(opened > 0);

    // At the crossing point the gate really is callable, so the sweep above was
    // not vacuous.
    t.clean_bad_debt_by_id(alice_id);
    assert!(
        !t.try_nft_owner_of(alice_id),
        "a successful clean_bad_debt must delete the account and burn its NFT"
    );
}
