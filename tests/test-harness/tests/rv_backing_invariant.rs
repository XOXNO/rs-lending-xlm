//! RV worker W2: pool backing and accounting invariants.
//!
//! Scope: INV-ACCT-01..10, INV-IDX-03 and INV-FLASH-01 (docs/reference/invariants.md),
//! and the "Backing and cash constraints", "Revenue payout" and "Bad debt" sections
//! of docs/reference/formulas.md.
//!
//! Backing identity, computed with the pool's own `common::rates` helpers from
//! `PoolStateRaw` (the pool's `guards::backing_shortfall` is crate-private):
//!   claims    = unscale_supply_floor(supplied, supply_index, decimals)
//!   backing   = cash + unscale_borrow_ceil(borrowed, borrow_index, decimals)
//!   shortfall = max(0, claims - backing)
//!
//! Fixtures seed no liquidity unless noted. The harness presets seed 1,000,000
//! tokens of cash per market, which leaves every shortfall check about 10^13 base
//! units of slack and cannot detect a one-unit rounding error.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::vec::Vec;

use common::constants::{RAY, SUPPLY_INDEX_FLOOR_RAW, WAD};
use common::math::fp::Ray;
use common::rates::{unscale_borrow_ceil, unscale_supply_floor};
use common::types::{
    AccountPositionRaw, ControllerKey, DebtPositionRaw, HubAssetKey, PoolStateRaw,
};
use position_nft::PositionNftClient;
use proptest::prelude::*;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::token::{self, TokenClient};
use soroban_sdk::{Address, Bytes, Map, I256};
use test_harness::presets::{eth_preset, usdc_preset, wbtc_preset, MarketPreset};
use test_harness::{
    assert_contract_error, days, errors, hub_asset, usd_cents, LendingTest, ALICE, BOB, CAROL,
    DAVE, HARNESS_SPOKE,
};

const ASSETS: [&str; 3] = ["USDC", "ETH", "WBTC"];
const PAYER: &str = "payer";
const DONOR: &str = "donor";
const EVE: &str = "eve";

/// Base units in one operation step per ASSETS entry: 1 USDC, 0.01 ETH, 0.0001 WBTC.
/// Every market uses 7 decimals.
const UNIT_RAW: [i128; 3] = [10_000_000, 100_000, 1_000];

fn zero_seed(mut preset: MarketPreset) -> MarketPreset {
    preset.initial_liquidity = 0.0;
    preset
}

/// The `three_asset_usdc_eth_wbtc` book with no seeded cash.
fn zero_seed_book(max_utilization_disabled: bool) -> LendingTest {
    let builder = LendingTest::new()
        .with_market(zero_seed(usdc_preset()))
        .with_market(zero_seed(eth_preset()))
        .with_market(zero_seed(wbtc_preset()));
    if max_utilization_disabled {
        builder.with_max_utilization_disabled_all_markets().build()
    } else {
        builder.build()
    }
}

/// Registers a fresh accumulator. Revenue claims forward to it.
fn install_accumulator(t: &LendingTest) -> Address {
    let accumulator = Address::generate(&t.env);
    t.set_accumulator(&accumulator);
    accumulator
}

fn pool_state(t: &LendingTest, asset: &str) -> PoolStateRaw {
    let key = hub_asset(t.resolve_asset(asset));
    t.pool_client(asset).get_sync_data(&key).state
}

/// Tokens the pool holds for `asset`, as reported by the token contract.
fn custody(t: &LendingTest, asset: &str) -> i128 {
    let market = t.resolve_market(asset);
    token::Client::new(&t.env, &market.asset).balance(&market.pool)
}

fn balance_of(t: &LendingTest, holder: &Address, asset: &str) -> i128 {
    token::Client::new(&t.env, &t.resolve_asset(asset)).balance(holder)
}

/// `(claims, debt, backing)` in native units, from the pool's own rounding helpers.
fn ledger(t: &LendingTest, asset: &str) -> (i128, i128, i128) {
    let decimals = t.resolve_market(asset).decimals;
    let st = pool_state(t, asset);
    let claims = unscale_supply_floor(
        &t.env,
        Ray::from(st.supplied),
        Ray::from(st.supply_index),
        decimals,
    );
    let debt = unscale_borrow_ceil(
        &t.env,
        Ray::from(st.borrowed),
        Ray::from(st.borrow_index),
        decimals,
    );
    (claims, debt, st.cash.saturating_add(debt))
}

fn shortfall(t: &LendingTest, asset: &str) -> i128 {
    let (claims, _, backing) = ledger(t, asset);
    claims.saturating_sub(backing).max(0)
}

/// Value of the revenue claim at the stored index, floored as the pool pays it.
fn treasury_value(t: &LendingTest, asset: &str) -> i128 {
    let decimals = t.resolve_market(asset).decimals;
    let st = pool_state(t, asset);
    unscale_supply_floor(
        &t.env,
        Ray::from(st.revenue),
        Ray::from(st.supply_index),
        decimals,
    )
}

/// One account's position in one market.
struct Position {
    /// Scaled supply shares.
    supply: i128,
    /// Scaled debt shares.
    debt: i128,
    /// Debt value at the stored borrow index, rounded up as the pool repays it.
    debt_ceil: i128,
}

fn position_of(t: &LendingTest, user: &str, asset: &str) -> Position {
    let account_id = t.resolve_account_id(user);
    let (supplies, borrows) = t.ctrl_client().get_account_positions(&account_id);
    let key = hub_asset(t.resolve_asset(asset));
    let supply = supplies.get(key.clone()).map_or(0, |p| p.scaled_amount);
    let debt = borrows.get(key).map_or(0, |p| p.scaled_amount);
    let st = pool_state(t, asset);
    let decimals = t.resolve_market(asset).decimals;
    let debt_ceil = unscale_borrow_ceil(
        &t.env,
        Ray::from(debt),
        Ray::from(st.borrow_index),
        decimals,
    );
    Position {
        supply,
        debt,
        debt_ceil,
    }
}

fn assert_same_pool_state(label: &str, a: &PoolStateRaw, b: &PoolStateRaw) {
    assert_eq!(a.supplied, b.supplied, "{label}: supplied");
    assert_eq!(a.borrowed, b.borrowed, "{label}: borrowed");
    assert_eq!(a.revenue, b.revenue, "{label}: revenue");
    assert_eq!(a.cash, b.cash, "{label}: cash");
    assert_eq!(a.supply_index, b.supply_index, "{label}: supply index");
    assert_eq!(a.borrow_index, b.borrow_index, "{label}: borrow index");
    assert_eq!(
        a.last_timestamp, b.last_timestamp,
        "{label}: last timestamp"
    );
}

/// Sum of scaled supply and debt over every live position NFT, read from controller
/// storage (the same reconciliation as `astra_audit.rs`).
fn account_totals(t: &LendingTest, key: &HubAssetKey) -> (i128, i128) {
    let nft = PositionNftClient::new(&t.env, &t.position_nft);
    let ids: Vec<u64> = (0..nft.total_supply())
        .map(|i| u64::from(nft.get_token_id(&i)))
        .collect();
    t.env.as_contract(&t.controller, || {
        let storage = t.env.storage().persistent();
        let mut supplied = 0i128;
        let mut borrowed = 0i128;
        for id in ids {
            if let Some(book) = storage
                .get::<_, Map<HubAssetKey, AccountPositionRaw>>(&ControllerKey::SupplyPositions(id))
            {
                supplied += book.get(key.clone()).map_or(0, |p| p.scaled_amount);
            }
            if let Some(book) = storage
                .get::<_, Map<HubAssetKey, DebtPositionRaw>>(&ControllerKey::BorrowPositions(id))
            {
                borrowed += book.get(key.clone()).map_or(0, |p| p.scaled_amount);
            }
        }
        (supplied, borrowed)
    })
}

/// Backing, custody, index and account-book invariants for every market.
/// `last` holds the indexes seen at the previous check, and `donated` the total
/// transferred straight to each pool. Custody must equal cash plus donations.
fn check_books(
    t: &LendingTest,
    last: &mut [(i128, i128); 3],
    donated: &[i128; 3],
) -> Result<(), TestCaseError> {
    for (i, asset) in ASSETS.iter().enumerate() {
        let st = pool_state(t, asset);
        let (claims, debt, backing) = ledger(t, asset);
        let short = claims.saturating_sub(backing).max(0);
        prop_assert_eq!(
            short,
            0,
            "{}: claims {} exceed backing {} (cash {}, debt {})",
            asset,
            claims,
            backing,
            st.cash,
            debt
        );
        prop_assert!(st.cash >= 0, "{}: cash {} is negative", asset, st.cash);
        prop_assert!(
            st.revenue >= 0 && st.revenue <= st.supplied,
            "{}: revenue {} vs supplied {}",
            asset,
            st.revenue,
            st.supplied
        );
        let held = custody(t, asset);
        prop_assert_eq!(
            held,
            st.cash + donated[i],
            "{}: token custody differs from cash book plus donations",
            asset
        );
        prop_assert!(
            st.supply_index >= last[i].0,
            "{}: supply index fell {} -> {}",
            asset,
            last[i].0,
            st.supply_index
        );
        prop_assert!(
            st.borrow_index >= last[i].1,
            "{}: borrow index fell {} -> {}",
            asset,
            last[i].1,
            st.borrow_index
        );
        last[i] = (st.supply_index, st.borrow_index);
        let controller_held = t.controller_token_balance_raw(asset);
        prop_assert!(
            controller_held.abs() <= 4,
            "{}: controller holds {} base units",
            asset,
            controller_held
        );
        let key = hub_asset(t.resolve_asset(asset));
        let (account_supply, account_debt) = account_totals(t, &key);
        prop_assert_eq!(
            account_supply,
            st.supplied - st.revenue,
            "{}: account supply shares vs supplied - revenue",
            asset
        );
        prop_assert_eq!(
            account_debt,
            st.borrowed,
            "{}: account debt shares vs borrowed",
            asset
        );
    }
    Ok(())
}

/// Both users hold accounts with collateral in several markets, so the random
/// operations can reach the borrow, withdraw and netting paths.
fn prime_book(max_utilization_disabled: bool) -> LendingTest {
    let mut t = zero_seed_book(max_utilization_disabled);
    install_accumulator(&t);
    t.supply(ALICE, "USDC", 5_000.0);
    t.supply(ALICE, "ETH", 1.0);
    t.supply(BOB, "USDC", 5_000.0);
    t.supply(BOB, "ETH", 5.0);
    t.supply(BOB, "WBTC", 0.05);
    t
}

/// Attempted and accepted counts per operation kind across the proptest cases.
const KIND_NAMES: [&str; 12] = [
    "supply",
    "borrow",
    "repay_partial",
    "repay_over",
    "withdraw_partial",
    "withdraw_all",
    "advance",
    "claim_revenue",
    "recapitalize",
    "flash_loan",
    "net_settle",
    "donate",
];
static ATTEMPTED: [AtomicUsize; 12] = [const { AtomicUsize::new(0) }; 12];
static ACCEPTED: [AtomicUsize; 12] = [const { AtomicUsize::new(0) }; 12];
static CASES_DONE: AtomicUsize = AtomicUsize::new(0);

/// Proptest cases for the random-sequence property. `RV_W2_CASES` overrides the
/// default of 32 for a longer local run.
fn cases() -> u32 {
    std::env::var("RV_W2_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(32)
}

fn kind_index(op: &Op) -> usize {
    match op {
        Op::Supply { .. } => 0,
        Op::Borrow { .. } => 1,
        Op::RepayPartial { .. } => 2,
        Op::RepayOver { .. } => 3,
        Op::WithdrawPartial { .. } => 4,
        Op::WithdrawAll { .. } => 5,
        Op::Advance { .. } => 6,
        Op::ClaimRevenue { .. } => 7,
        Op::Recapitalize { .. } => 8,
        Op::FlashLoan { .. } => 9,
        Op::NetSettle { .. } => 10,
        Op::Donate { .. } => 11,
    }
}

/// Prints a labelled value when `RV_W2_NUMBERS` is set, so a `--nocapture` run shows
/// the concrete figures behind each assertion.
fn numbers(label: &str, value: impl std::fmt::Debug) {
    if std::env::var_os("RV_W2_NUMBERS").is_some() {
        eprintln!("RV_W2_NUMBERS {label}: {value:?}");
    }
}

fn raw(asset: usize, units: u32) -> i128 {
    i128::from(units) * UNIT_RAW[asset]
}

fn burn(t: &LendingTest, asset: &Address, holder: &Address, amount: i128) {
    TokenClient::new(&t.env, asset).burn(holder, &amount);
}

fn try_supply_raw(t: &LendingTest, user: &str, asset: &str, amount: i128) -> bool {
    let Some(account_id) = t.find_account_id(user) else {
        return false;
    };
    let who = t.users[user].address.clone();
    let market = t.resolve_market(asset);
    market.token_admin.mint(&who, &amount);
    let assets = soroban_sdk::vec![&t.env, (hub_asset(market.asset.clone()), amount)];
    let accepted = matches!(
        t.ctrl_client()
            .try_supply(&who, &account_id, &HARNESS_SPOKE, &assets),
        Ok(Ok(_))
    );
    if !accepted {
        burn(t, &market.asset, &who, amount);
    }
    accepted
}

fn try_borrow_raw(t: &LendingTest, user: &str, asset: &str, amount: i128) -> bool {
    let Some(account_id) = t.find_account_id(user) else {
        return false;
    };
    let who = t.users[user].address.clone();
    let borrows = soroban_sdk::vec![&t.env, (hub_asset(t.resolve_asset(asset)), amount)];
    matches!(
        t.ctrl_client()
            .try_borrow(&who, &account_id, &borrows, &None),
        Ok(Ok(()))
    )
}

fn try_repay_raw(t: &LendingTest, user: &str, asset: &str, amount: i128) -> bool {
    let Some(account_id) = t.find_account_id(user) else {
        return false;
    };
    let who = t.users[user].address.clone();
    let market = t.resolve_market(asset);
    market.token_admin.mint(&who, &amount);
    let payments = soroban_sdk::vec![&t.env, (hub_asset(market.asset.clone()), amount)];
    let accepted = matches!(
        t.ctrl_client().try_repay(&who, &account_id, &payments),
        Ok(Ok(()))
    );
    if !accepted {
        burn(t, &market.asset, &who, amount);
    }
    accepted
}

fn try_net_settle_raw(t: &LendingTest, user: &str, asset: &str, amount: i128) -> bool {
    let Some(account_id) = t.find_account_id(user) else {
        return false;
    };
    let who = t.users[user].address.clone();
    let hub = hub_asset(t.resolve_asset(asset));
    matches!(
        t.ctrl_client().try_repay_debt_with_collateral(
            &who,
            &account_id,
            &hub,
            &amount,
            &hub,
            &Bytes::new(&t.env),
            &false,
        ),
        Ok(Ok(()))
    )
}

/// Donates `amount` through the controller while the book has no shortfall. The
/// controller must refund all of it and the pool must mint nothing.
fn recapitalize_idle(
    t: &mut LendingTest,
    asset: &str,
    amount: i128,
) -> Result<bool, TestCaseError> {
    let payer = t.get_or_create_user(PAYER);
    let market = t.resolve_market(asset);
    let token_addr = market.asset.clone();
    let key = hub_asset(token_addr.clone());
    let before = balance_of(t, &payer, asset);
    market.token_admin.mint(&payer, &amount);
    let applied = match t.ctrl_client().try_recapitalize(&payer, &key, &amount) {
        Ok(Ok(applied)) => applied,
        other => {
            return Err(TestCaseError::fail(format!(
                "recapitalize refused on a zero shortfall: {other:?}"
            )))
        }
    };
    prop_assert_eq!(
        applied,
        0,
        "recapitalize credited {} with zero shortfall",
        applied
    );
    prop_assert_eq!(
        balance_of(t, &payer, asset),
        before + amount,
        "payer was charged for a refunded donation"
    );
    Ok(true)
}

#[derive(Clone, Debug)]
enum Op {
    Supply {
        user: &'static str,
        asset: usize,
        units: u32,
    },
    Borrow {
        user: &'static str,
        asset: usize,
        units: u32,
    },
    RepayPartial {
        user: &'static str,
        asset: usize,
        bps: u16,
    },
    /// Pays the debt plus between 0% and 100% of it again, plus one unit, so the
    /// excess must be refunded.
    RepayOver {
        user: &'static str,
        asset: usize,
        bps: u16,
    },
    WithdrawPartial {
        user: &'static str,
        asset: usize,
        bps: u16,
    },
    WithdrawAll {
        user: &'static str,
        asset: usize,
    },
    Advance {
        secs: u64,
    },
    ClaimRevenue {
        asset: usize,
    },
    Recapitalize {
        asset: usize,
        units: u32,
    },
    FlashLoan {
        user: &'static str,
        asset: usize,
        units: u32,
    },
    /// Same-asset `repay_debt_with_collateral`, which nets supply against debt.
    NetSettle {
        user: &'static str,
        asset: usize,
        bps: u16,
    },
    /// A direct token transfer to the pool, outside the controller.
    Donate {
        asset: usize,
        units: u32,
    },
}

fn user_strat() -> impl Strategy<Value = &'static str> {
    prop_oneof![Just(ALICE), Just(BOB)]
}

fn asset_strat() -> impl Strategy<Value = usize> {
    0usize..ASSETS.len()
}

fn op_strat() -> impl Strategy<Value = Op> {
    prop_oneof![
        4 => (user_strat(), asset_strat(), 1u32..=2_000)
            .prop_map(|(user, asset, units)| Op::Supply { user, asset, units }),
        3 => (user_strat(), asset_strat(), 1u32..=100)
            .prop_map(|(user, asset, units)| Op::Borrow { user, asset, units }),
        2 => (user_strat(), asset_strat(), 1u16..=10_000)
            .prop_map(|(user, asset, bps)| Op::RepayPartial { user, asset, bps }),
        1 => (user_strat(), asset_strat(), 0u16..=10_000)
            .prop_map(|(user, asset, bps)| Op::RepayOver { user, asset, bps }),
        2 => (user_strat(), asset_strat(), 1u16..=10_000)
            .prop_map(|(user, asset, bps)| Op::WithdrawPartial { user, asset, bps }),
        1 => (user_strat(), asset_strat())
            .prop_map(|(user, asset)| Op::WithdrawAll { user, asset }),
        2 => (60u64..=180 * 86_400).prop_map(|secs| Op::Advance { secs }),
        1 => asset_strat().prop_map(|asset| Op::ClaimRevenue { asset }),
        1 => (asset_strat(), 1u32..=100)
            .prop_map(|(asset, units)| Op::Recapitalize { asset, units }),
        1 => (user_strat(), asset_strat(), 1u32..=500)
            .prop_map(|(user, asset, units)| Op::FlashLoan { user, asset, units }),
        1 => (user_strat(), asset_strat(), 1u16..=10_000)
            .prop_map(|(user, asset, bps)| Op::NetSettle { user, asset, bps }),
        1 => (asset_strat(), 1u32..=100)
            .prop_map(|(asset, units)| Op::Donate { asset, units }),
    ]
}

/// Transfers `amount` straight to the pool, bypassing the controller.
fn donate(t: &mut LendingTest, asset: &str, amount: i128) {
    let donor = t.get_or_create_user(DONOR);
    let market = t.resolve_market(asset);
    market.token_admin.mint(&donor, &amount);
    token::Client::new(&t.env, &market.asset).transfer(&donor, &market.pool, &amount);
}

/// Applies one operation and returns whether the protocol accepted it. Errors only
/// when an operation's own postcondition fails. `donated` accumulates direct
/// transfers to each pool.
fn run_op(t: &mut LendingTest, op: &Op, donated: &mut [i128; 3]) -> Result<bool, TestCaseError> {
    Ok(match *op {
        Op::Supply { user, asset, units } => {
            try_supply_raw(t, user, ASSETS[asset], raw(asset, units))
        }
        Op::Borrow { user, asset, units } => {
            try_borrow_raw(t, user, ASSETS[asset], raw(asset, units))
        }
        Op::RepayPartial { user, asset, bps } => {
            let amount = t.borrow_balance_raw(user, ASSETS[asset]) * i128::from(bps) / 10_000;
            amount > 0 && try_repay_raw(t, user, ASSETS[asset], amount)
        }
        Op::RepayOver { user, asset, bps } => {
            let debt = t.borrow_balance_raw(user, ASSETS[asset]);
            let amount = debt + debt * i128::from(bps) / 10_000 + 1;
            debt > 0 && try_repay_raw(t, user, ASSETS[asset], amount)
        }
        Op::WithdrawPartial { user, asset, bps } => {
            let amount = t.supply_balance_raw(user, ASSETS[asset]) * i128::from(bps) / 10_000;
            amount > 0 && t.try_withdraw_raw(user, ASSETS[asset], amount).is_ok()
        }
        Op::WithdrawAll { user, asset } => t.try_withdraw_raw(user, ASSETS[asset], 0).is_ok(),
        Op::Advance { secs } => {
            t.advance_time(secs);
            let refreshed = t.try_update_indexes_for(&ASSETS);
            prop_assert!(
                refreshed.is_ok(),
                "index update refused after {} s: {:?}",
                secs,
                refreshed
            );
            true
        }
        Op::ClaimRevenue { asset } => t.try_claim_revenue(ASSETS[asset]).is_ok(),
        Op::Recapitalize { asset, units } => {
            recapitalize_idle(t, ASSETS[asset], raw(asset, units))?
        }
        Op::FlashLoan { user, asset, units } => {
            let receiver = t.deploy_flash_loan_receiver();
            t.try_flash_loan_with_data(
                user,
                ASSETS[asset],
                raw(asset, units),
                &receiver,
                &Bytes::new(&t.env),
            )
            .is_ok()
        }
        Op::NetSettle { user, asset, bps } => {
            let supply = t.supply_balance_raw(user, ASSETS[asset]);
            let debt = t.borrow_balance_raw(user, ASSETS[asset]);
            let amount = supply.min(debt) * i128::from(bps) / 10_000;
            amount > 0 && try_net_settle_raw(t, user, ASSETS[asset], amount)
        }
        Op::Donate { asset, units } => {
            let amount = raw(asset, units);
            donate(t, ASSETS[asset], amount);
            donated[asset] += amount;
            true
        }
    })
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: cases(),
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    /// H1: after every one of 25 random operations, every market has zero backing
    /// shortfall, custody equal to cash plus direct donations, non-decreasing indexes,
    /// and account books that reconcile exactly with pool totals. Half the cases run
    /// with the utilization ceiling disabled, which lets cash approach the 2% buffer.
    #[test]
    fn rv_backing_random_ops_keep_shortfall_zero_and_books_exact(
        disable_ceiling in any::<bool>(),
        ops in prop::collection::vec(op_strat(), 25),
    ) {
        let mut t = prime_book(disable_ceiling);
        let mut last = [(RAY, RAY); 3];
        let mut donated = [0i128; 3];
        check_books(&t, &mut last, &donated)?;
        for op in &ops {
            let accepted = run_op(&mut t, op, &mut donated)?;
            let kind = kind_index(op);
            ATTEMPTED[kind].fetch_add(1, Ordering::Relaxed);
            if accepted {
                ACCEPTED[kind].fetch_add(1, Ordering::Relaxed);
            }
            check_books(&t, &mut last, &donated)?;
        }
        if CASES_DONE.fetch_add(1, Ordering::Relaxed) + 1 == cases() as usize
            && std::env::var_os("RV_W2_STATS").is_some()
        {
            for (k, name) in KIND_NAMES.iter().enumerate() {
                eprintln!(
                    "RV_W2_STATS {:<17} attempted {:>5} accepted {:>5}",
                    name,
                    ATTEMPTED[k].load(Ordering::Relaxed),
                    ACCEPTED[k].load(Ordering::Relaxed)
                );
            }
        }
    }
}

/// H1 coverage: a fixed script that executes every operation kind at least once,
/// checking the same invariants after each step.
#[test]
fn rv_backing_every_op_kind_keeps_books_exact() -> Result<(), TestCaseError> {
    let mut t = prime_book(false);
    let mut last = [(RAY, RAY); 3];
    let mut donated = [0i128; 3];
    let script = [
        (
            Op::Supply {
                user: BOB,
                asset: 1,
                units: 100,
            },
            true,
        ),
        (
            Op::Borrow {
                user: ALICE,
                asset: 1,
                units: 10,
            },
            true,
        ),
        (
            Op::Borrow {
                user: BOB,
                asset: 0,
                units: 1_000,
            },
            true,
        ),
        (Op::Advance { secs: 30 * 86_400 }, true),
        (
            Op::RepayPartial {
                user: BOB,
                asset: 0,
                bps: 5_000,
            },
            true,
        ),
        (
            Op::RepayOver {
                user: BOB,
                asset: 0,
                bps: 10_000,
            },
            true,
        ),
        (
            Op::WithdrawPartial {
                user: BOB,
                asset: 0,
                bps: 1_000,
            },
            true,
        ),
        // ALICE has no WBTC position, so this must be refused.
        (
            Op::WithdrawAll {
                user: ALICE,
                asset: 2,
            },
            false,
        ),
        (Op::ClaimRevenue { asset: 0 }, true),
        (
            Op::Recapitalize {
                asset: 1,
                units: 50,
            },
            true,
        ),
        (
            Op::FlashLoan {
                user: BOB,
                asset: 0,
                units: 100,
            },
            true,
        ),
        (
            Op::NetSettle {
                user: ALICE,
                asset: 1,
                bps: 5_000,
            },
            true,
        ),
        (
            Op::WithdrawAll {
                user: ALICE,
                asset: 1,
            },
            true,
        ),
        (
            Op::Supply {
                user: ALICE,
                asset: 0,
                units: 500,
            },
            true,
        ),
        (Op::Donate { asset: 1, units: 7 }, true),
    ];
    check_books(&t, &mut last, &donated)?;
    for (op, expect_accepted) in &script {
        let accepted = run_op(&mut t, op, &mut donated)?;
        assert_eq!(accepted, *expect_accepted, "{op:?}");
        check_books(&t, &mut last, &donated)?;
    }
    Ok(())
}

/// H2: with zero shortfall, `recapitalize` refunds the whole amount and leaves every
/// pool field, including the accrual timestamp, unchanged.
#[test]
fn rv_recapitalize_zero_shortfall_refunds_everything_and_mints_nothing() {
    let mut t = zero_seed_book(false);
    t.supply(BOB, "ETH", 5.0);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow_raw(ALICE, "ETH", 10_000_000);
    assert_eq!(shortfall(&t, "ETH"), 0);

    let before = pool_state(&t, "ETH");
    let custody_before = custody(&t, "ETH");
    let payer = t.get_or_create_user(PAYER);
    let eth = t.resolve_asset("ETH");
    t.resolve_market("ETH")
        .token_admin
        .mint(&payer, &20_000_000);

    let applied = t
        .ctrl_client()
        .recapitalize(&payer, &hub_asset(eth), &20_000_000);
    assert_eq!(applied, 0, "no shortfall means nothing is credited");
    assert_same_pool_state("after recapitalize", &before, &pool_state(&t, "ETH"));
    assert_eq!(custody(&t, "ETH"), custody_before);
    assert_eq!(
        balance_of(&t, &payer, "ETH"),
        20_000_000,
        "payer refunded in full"
    );
}

/// Books in the state that a floor-clamped bad-debt write-down leaves: ETH claims
/// of 97.92 ETH against 0.02 ETH of cash and no debt, with the index at RAY/1000.
fn floor_writedown_book() -> LendingTest {
    let mut t = zero_seed_book(true);
    // Suppliers 100 ETH. Borrower posts 300,000 USDC and borrows 97.9 ETH, which
    // passes the 2% liquidation buffer (cash 2.1 >= 2.0 ETH).
    t.supply(BOB, "ETH", 100.0);
    t.supply(ALICE, "USDC", 300_000.0);
    t.borrow_raw(ALICE, "ETH", 979_000_000);
    // A supplier exit ignores the buffer and leaves cash at 0.02 ETH against
    // 97.92 ETH of claims.
    t.withdraw_raw(BOB, "ETH", 20_800_000);
    let before = pool_state(&t, "ETH");
    assert_eq!(before.cash, 200_000);
    assert_eq!(before.borrowed, 979_000_000 * 10i128.pow(20));
    assert_eq!(
        shortfall(&t, "ETH"),
        0,
        "no write-down yet, so no shortfall"
    );

    // USDC at $0.00001 leaves the borrower $3 of collateral against $195,800 of debt.
    // 1e13 WAD = $0.00001 per USDC.
    t.set_price("USDC", 10_000_000_000_000);
    assert!(t.total_collateral_raw(ALICE) <= 5 * WAD);
    assert!(t.total_debt_raw(ALICE) > t.total_collateral_raw(ALICE));
    let alice = t.resolve_account_id(ALICE);
    t.clean_bad_debt_by_id(alice);

    // Debt of 97.9 ETH exceeds 99.9% of the 97.92 ETH claims, so the index clamps
    // to RAY/1000.
    let after = pool_state(&t, "ETH");
    assert_eq!(after.supply_index, SUPPLY_INDEX_FLOOR_RAW);
    assert_eq!(after.borrowed, 0);
    assert_eq!(after.supplied, before.supplied, "shares are not burned");
    assert_eq!(after.cash, 200_000);
    t
}

/// H4 (INV-ACCT-04): the only way to create a positive shortfall through normal
/// operations is a bad-debt write-down that hits the supply-index floor RAY/1000.
/// Supply is refused until recapitalization covers the shortfall exactly.
#[test]
fn rv_supply_after_floor_writedown_rejects_until_recapitalized_exactly() {
    let mut t = floor_writedown_book();
    let eth = t.resolve_asset("ETH");
    let (claims, _, backing) = ledger(&t, "ETH");
    assert_eq!((claims, backing), (979_200, 200_000));
    numbers("H4 claims (base units)", claims);
    numbers("H4 backing = cash (base units)", backing);
    assert_eq!(shortfall(&t, "ETH"), 779_200);

    // A direct transfer of 1,000,000 base units (0.1 ETH) is custody, not cash.
    // It leaves the shortfall alone.
    let donor = t.get_or_create_user(DONOR);
    let pool = t.resolve_market("ETH").pool.clone();
    t.resolve_market("ETH").token_admin.mint(&donor, &1_000_000);
    token::Client::new(&t.env, &eth).transfer(&donor, &pool, &1_000_000);
    assert_eq!(
        shortfall(&t, "ETH"),
        779_200,
        "donation does not reduce the shortfall"
    );
    assert_eq!(
        custody(&t, "ETH"),
        1_200_000,
        "cash 200,000 plus the donation"
    );

    // Supply is refused while the book is short.
    assert_contract_error(t.try_supply(CAROL, "ETH", 1.0), errors::POOL_INSOLVENT);

    // Recapitalizing short by one unit keeps supply refused.
    let payer = t.get_or_create_user(PAYER);
    t.resolve_market("ETH")
        .token_admin
        .mint(&payer, &(779_199 + 501));
    assert_eq!(
        t.ctrl_client()
            .recapitalize(&payer, &hub_asset(eth.clone()), &779_199),
        779_199
    );
    assert_eq!(shortfall(&t, "ETH"), 1);
    assert_contract_error(t.try_supply(CAROL, "ETH", 1.0), errors::POOL_INSOLVENT);

    // Covering the last unit with a 501-unit payment credits 1 and refunds 500.
    assert_eq!(
        t.ctrl_client().recapitalize(&payer, &hub_asset(eth), &501),
        1
    );
    assert_eq!(
        balance_of(&t, &payer, "ETH"),
        500,
        "excess refunded exactly"
    );
    assert_eq!(shortfall(&t, "ETH"), 0);
    assert_eq!(
        custody(&t, "ETH"),
        1_979_200,
        "cash 979,200 plus the 1,000,000 donation"
    );

    t.supply(CAROL, "ETH", 1.0);
    assert_eq!(shortfall(&t, "ETH"), 0, "supply succeeds once backed");
}

/// H2/H4 under the default 95% ceiling and with no exits. An insolvent dust account
/// that nobody cleans keeps accruing interest for three years. Accrual is not gated,
/// so utilization passes the ceiling and the supplier balance inflates on interest
/// that will never be paid. The cleanup write-down removes that phantom yield. The
/// index stays far above the RAY/1000 floor, so claims remain backed and supply is
/// not blocked. Before cleanup, the 95% ceiling refuses a revenue claim that would take
/// the cash. A shortfall needs cash below 0.1% of the supply shares, which this path
/// does not reach.
#[test]
fn rv_uncleaned_bad_debt_accrual_writes_off_phantom_yield_without_shortfall() {
    let mut t = zero_seed_book(false);
    install_accumulator(&t);
    t.supply(BOB, "ETH", 100.0);
    t.supply(ALICE, "USDC", 300_000.0);
    // 94 ETH is 94% utilization (under the 95% ceiling) and leaves 6 ETH of cash,
    // which clears the 2% buffer of 2 ETH.
    t.borrow_raw(ALICE, "ETH", 940_000_000);
    let eth = t.resolve_asset("ETH");
    // USDC at $0.00001 leaves the $300k collateral worth $3. Nobody cleans the account.
    t.set_price("USDC", 10_000_000_000_000);
    assert!(t.total_collateral_raw(ALICE) <= 5 * WAD);
    assert!(t.total_debt_raw(ALICE) > t.total_collateral_raw(ALICE));

    // Three years of accrual on the uncollectible debt, in one update (three chunks).
    t.advance_time(days(3 * 365));
    t.update_indexes_for(&["ETH"]);
    let aged = pool_state(&t, "ETH");
    let utilization = t
        .pool_client("ETH")
        .get_utilisation(&hub_asset(eth.clone()));
    let phantom = t.supply_balance_raw(BOB, "ETH");
    numbers("uncleaned: utilization after 3 years (RAY)", utilization);
    numbers("uncleaned: debt value after 3 years", ledger(&t, "ETH").1);
    numbers("uncleaned: claims after 3 years", ledger(&t, "ETH").0);
    numbers("uncleaned: BOB claim before cleanup (base units)", phantom);
    assert!(
        utilization > RAY * 95 / 100,
        "accrual must push utilization past the default ceiling"
    );
    // Protocol revenue accrues on the same uncollectible interest. Before cleanup the
    // 95% ceiling refuses a revenue claim that would take the 6 ETH of cash.
    let revenue_before = aged.revenue;
    assert_contract_error(t.try_claim_revenue("ETH"), errors::UTILIZATION_ABOVE_MAX);
    assert_eq!(
        pool_state(&t, "ETH").cash,
        aged.cash,
        "refused claim moves no cash"
    );
    assert_eq!(pool_state(&t, "ETH").revenue, revenue_before);
    assert!(
        phantom > 50_000_000_000,
        "BOB's 100 ETH claim must inflate more than 50x on uncollectible interest: {phantom}"
    );
    assert_eq!(aged.cash, 60_000_000, "accrual never moves cash");

    let alice = t.resolve_account_id(ALICE);
    t.clean_bad_debt_by_id(alice);
    let after = pool_state(&t, "ETH");
    numbers(
        "uncleaned: supply index after cleanup (RAY)",
        after.supply_index,
    );
    assert!(
        after.supply_index > SUPPLY_INDEX_FLOOR_RAW,
        "the write-down must not reach the floor on this path"
    );
    assert_eq!(after.borrowed, 0);
    assert_eq!(
        shortfall(&t, "ETH"),
        0,
        "claims stay backed after the write-down"
    );

    let bob_after = t.supply_balance_raw(BOB, "ETH");
    numbers("uncleaned: BOB claim after cleanup (base units)", bob_after);
    assert!(
        bob_after < after.cash,
        "BOB's claim falls back below the 6 ETH of cash"
    );
    t.supply(CAROL, "ETH", 1.0);
    assert_eq!(shortfall(&t, "ETH"), 0, "supply stays open");
}

/// H4 without any exit, with the utilization ceiling disabled. Protocol revenue accrues
/// on uncollectible interest, so a permissionless revenue claim can take the real cash
/// (INV-ACCT-06). That leaves zero cash and no debt after cleanup. The write-down then
/// clamps to RAY/1000 and leaves an unbacked residual of 0.001 x shares. Supply is
/// refused until recapitalized. Under the default ceiling the same claim is refused
/// (see the phantom-yield test above), so this path needs the ceiling disabled.
#[test]
fn rv_uncleaned_revenue_claim_drains_cash_and_leaves_floor_residual_when_ceiling_disabled() {
    let mut t = zero_seed_book(true);
    let accumulator = install_accumulator(&t);
    t.supply(BOB, "ETH", 100.0);
    t.supply(ALICE, "USDC", 300_000.0);
    t.borrow_raw(ALICE, "ETH", 940_000_000);
    t.set_price("USDC", 10_000_000_000_000);
    t.advance_time(days(3 * 365));
    t.update_indexes_for(&["ETH"]);

    let aged = pool_state(&t, "ETH");
    let revenue_value = treasury_value(&t, "ETH");
    numbers(
        "floor residual: revenue value before claim (base units)",
        revenue_value,
    );
    numbers(
        "floor residual: real cash before claim (base units)",
        aged.cash,
    );
    assert!(
        revenue_value > aged.cash,
        "revenue on uncollectible interest exceeds cash"
    );

    let acc_before = balance_of(&t, &accumulator, "ETH");
    let paid = t.claim_revenue("ETH");
    assert_eq!(paid, aged.cash, "the claim takes all of the real cash");
    assert_eq!(balance_of(&t, &accumulator, "ETH") - acc_before, paid);
    assert_eq!(pool_state(&t, "ETH").cash, 0);
    assert_eq!(
        shortfall(&t, "ETH"),
        0,
        "before cleanup the claims stay within backing"
    );

    let alice = t.resolve_account_id(ALICE);
    t.clean_bad_debt_by_id(alice);
    let after = pool_state(&t, "ETH");
    assert_eq!(
        after.supply_index, SUPPLY_INDEX_FLOOR_RAW,
        "remaining claims clamp to the floor"
    );
    assert_eq!(after.borrowed, 0);
    // 100 ETH of shares at RAY/1000 is 0.1 ETH, and BOB holds all 100 of them.
    assert_eq!(t.supply_balance_raw(BOB, "ETH"), 1_000_000);
    let (claims, _, backing) = ledger(&t, "ETH");
    let residual = claims - backing;
    numbers("floor residual: claims after cleanup (base units)", claims);
    numbers("floor residual: unbacked residual (base units)", residual);
    assert!(
        residual > 0,
        "the floor clamp must leave an unbacked residual"
    );
    assert_contract_error(t.try_supply(CAROL, "ETH", 1.0), errors::POOL_INSOLVENT);
}

/// Negative control for the H1 checker: it must reject a one-unit donation (custody
/// above cash) and the floor-clamped shortfall. Without this, a green H1 run could
/// mean the checker never fires.
#[test]
fn rv_backing_checker_flags_a_unit_donation_and_a_floor_shortfall() {
    let mut t = prime_book(false);
    let mut last = [(RAY, RAY); 3];
    check_books(&t, &mut last, &[0; 3]).expect("clean book must pass");

    let usdc = t.resolve_asset("USDC");
    let pool = t.resolve_market("USDC").pool.clone();
    let donor = t.get_or_create_user(DONOR);
    t.resolve_market("USDC").token_admin.mint(&donor, &1);
    token::Client::new(&t.env, &usdc).transfer(&donor, &pool, &1);
    let mut fresh = [(RAY, RAY); 3];
    let donation_err =
        check_books(&t, &mut fresh, &[0; 3]).expect_err("one donated unit must be flagged");
    assert!(
        donation_err.to_string().contains("custody"),
        "unexpected failure: {donation_err}"
    );

    let floor = floor_writedown_book();
    let mut fresh = [(RAY, RAY); 3];
    let shortfall_err =
        check_books(&floor, &mut fresh, &[0; 3]).expect_err("the floor shortfall must be flagged");
    assert!(
        shortfall_err.to_string().contains("exceed backing"),
        "unexpected failure: {shortfall_err}"
    );
    numbers(
        "negative control: shortfall message",
        shortfall_err.to_string(),
    );
}

/// H6 (INV-IDX-03): a standard bad-debt write-down above the floor lowers the debt
/// market's supply index by at least bad / total, matches the formulas.md write-down
/// exactly, leaves the other markets untouched, and keeps claims within backing.
#[test]
fn rv_clean_bad_debt_standard_writedown_is_bounded_and_isolated() {
    let mut t = zero_seed_book(false);
    t.supply(BOB, "ETH", 100.0);
    t.supply(CAROL, "WBTC", 2.0);
    t.supply(DAVE, "USDC", 200_000.0);
    t.borrow(DAVE, "WBTC", 1.0);
    t.advance_and_sync(days(90));

    // Dust borrower: 8 USDC against 0.002 ETH.
    t.supply(ALICE, "USDC", 8.0);
    t.borrow_raw(ALICE, "ETH", 20_000);
    t.set_price("USDC", usd_cents(5));
    assert!(t.total_collateral_raw(ALICE) <= 5 * WAD);
    assert!(t.total_debt_raw(ALICE) > t.total_collateral_raw(ALICE));

    let eth_before = pool_state(&t, "ETH");
    let wbtc_before = pool_state(&t, "WBTC");
    let usdc_before = pool_state(&t, "USDC");
    let alice_usdc_scaled = position_of(&t, ALICE, "USDC").supply;
    let alice_debt_scaled = position_of(&t, ALICE, "ETH").debt;
    assert!(alice_usdc_scaled > 0 && alice_debt_scaled > 0);

    let env = t.env.clone();
    let bad = Ray::from(alice_debt_scaled)
        .mul_ceil(&env, Ray::from(eth_before.borrow_index))
        .raw();
    let total = Ray::from(eth_before.supplied).mul(&env, Ray::from(eth_before.supply_index));
    let capped = Ray::from(bad).min(total);
    let formula_index = Ray::from(eth_before.supply_index)
        .mul_floor(&env, total.checked_sub(&env, capped).div_floor(&env, total))
        .max(Ray::from(SUPPLY_INDEX_FLOOR_RAW));

    let alice = t.resolve_account_id(ALICE);
    t.clean_bad_debt_by_id(alice);

    let eth_after = pool_state(&t, "ETH");
    numbers("H6 bad debt (RAY)", bad);
    numbers("H6 total supplied value (RAY)", total.raw());
    numbers("H6 supply index before", eth_before.supply_index);
    numbers("H6 supply index after", eth_after.supply_index);
    assert_eq!(
        eth_after.supply_index,
        formula_index.raw(),
        "write-down must match formulas.md#bad-debt"
    );
    assert!(eth_after.supply_index < eth_before.supply_index);
    assert!(
        eth_after.supply_index > SUPPLY_INDEX_FLOOR_RAW,
        "not clamped"
    );

    // INV-IDX-03: SI_after / SI_before <= (total - bad) / total, in exact arithmetic.
    let lhs =
        I256::from_i128(&env, eth_after.supply_index).mul(&I256::from_i128(&env, total.raw()));
    let rhs = I256::from_i128(&env, eth_before.supply_index)
        .mul(&I256::from_i128(&env, total.raw() - capped.raw()));
    assert!(lhs <= rhs, "index fell by less than bad/total");

    // The write-down touches only ETH supply; the debt leaves the borrow book.
    assert_eq!(eth_after.borrow_index, eth_before.borrow_index);
    assert_eq!(eth_after.supplied, eth_before.supplied);
    assert_eq!(eth_after.cash, eth_before.cash);
    assert_eq!(eth_after.borrowed, eth_before.borrowed - alice_debt_scaled);
    let (claims, _, backing) = ledger(&t, "ETH");
    assert!(
        claims <= backing,
        "claims {claims} exceed backing {backing}"
    );

    // WBTC is bit-identical; USDC keeps its indexes and book, and absorbs the dust
    // collateral as revenue.
    assert_same_pool_state("WBTC", &wbtc_before, &pool_state(&t, "WBTC"));
    let usdc_after = pool_state(&t, "USDC");
    assert_eq!(usdc_after.supply_index, usdc_before.supply_index);
    assert_eq!(usdc_after.borrow_index, usdc_before.borrow_index);
    assert_eq!(usdc_after.supplied, usdc_before.supplied);
    assert_eq!(usdc_after.borrowed, usdc_before.borrowed);
    assert_eq!(usdc_after.cash, usdc_before.cash);
    assert_eq!(usdc_after.revenue - usdc_before.revenue, alice_usdc_scaled);
    t.assert_no_positions(ALICE);
}

/// H1 under long accrual (INV-IDX-04): three years at 97% utilization in one update
/// call, which `interest::global_sync` splits into one-year steps. Backing stays
/// exact, the indexes compound, accrual moves no cash, and a cash-limited claim
/// keeps the books exact.
#[test]
fn rv_accrual_three_years_at_high_utilization_keeps_shortfall_zero() -> Result<(), TestCaseError> {
    let mut t = zero_seed_book(true);
    install_accumulator(&t);
    t.supply(BOB, "USDC", 10_000.0);
    t.supply(ALICE, "ETH", 20.0);
    t.borrow(ALICE, "USDC", 9_700.0);
    let start = pool_state(&t, "USDC");
    let mut last = [(RAY, RAY); 3];
    check_books(&t, &mut last, &[0; 3])?;

    t.advance_time(days(3 * 365));
    t.update_indexes_for(&ASSETS);
    check_books(&t, &mut last, &[0; 3])?;
    let grown = pool_state(&t, "USDC");
    assert!(
        grown.borrow_index > 2 * RAY,
        "debt index must compound over three years: {}",
        grown.borrow_index
    );
    assert!(grown.supply_index > RAY, "supplier index must grow");
    assert_eq!(grown.cash, start.cash, "accrual never moves cash");
    assert!(grown.revenue > 0, "protocol fee accrues as revenue");

    let value = treasury_value(&t, "USDC");
    assert!(
        value > grown.cash,
        "fixture must be cash-limited: value {value}"
    );
    let paid = t.claim_revenue("USDC");
    assert_eq!(paid, grown.cash, "cash-limited claim pays exactly the cash");
    check_books(&t, &mut last, &[0; 3])?;
    Ok(())
}

/// H3 (INV-ACCT-02, INV-FLASH-01): a direct token transfer to the pool is not cash.
/// It raises custody only. Flash loans, withdrawals and revenue claims are sized from
/// the cash book, so the donation is never paid out.
#[test]
fn rv_donation_is_not_cash_and_is_never_paid_out() -> Result<(), TestCaseError> {
    let mut t = zero_seed_book(true);
    let accumulator = install_accumulator(&t);
    t.supply(BOB, "USDC", 10_000.0);
    t.supply(ALICE, "ETH", 20.0);
    t.borrow(ALICE, "USDC", 5_000.0);
    t.advance_time(days(365));
    t.update_indexes_for(&["USDC"]);

    let usdc = t.resolve_asset("USDC");
    let pool = t.resolve_market("USDC").pool.clone();
    let before = pool_state(&t, "USDC");
    assert_eq!(custody(&t, "USDC"), before.cash);
    let bob_supply = t.supply_balance_raw(BOB, "USDC");
    let alice_debt = t.borrow_balance_raw(ALICE, "USDC");

    // 1. Donate 123.456789 USDC straight to the pool.
    let gift: i128 = 1_234_567_890;
    let donor = t.get_or_create_user(DONOR);
    t.resolve_market("USDC").token_admin.mint(&donor, &gift);
    token::Client::new(&t.env, &usdc).transfer(&donor, &pool, &gift);
    let donated = pool_state(&t, "USDC");
    assert_eq!(custody(&t, "USDC") - before.cash, gift);
    assert_same_pool_state("after donation", &before, &donated);
    assert_eq!(t.supply_balance_raw(BOB, "USDC"), bob_supply);
    assert_eq!(t.borrow_balance_raw(ALICE, "USDC"), alice_debt);

    // 2. A 100 USDC flash loan still succeeds. Its 9 bps fee is cash and balance alike.
    let receiver = t.deploy_flash_loan_receiver();
    assert_eq!(t.try_flash_loan(EVE, "USDC", 100.0, &receiver), Ok(()));
    let after_loan = pool_state(&t, "USDC");
    let fee = after_loan.cash - donated.cash;
    assert_eq!(fee, 900_000, "9 bps of 1e9 base units");
    numbers("H3 flash fee (base units)", fee);
    assert_eq!(custody(&t, "USDC") - after_loan.cash, gift);

    // 3. A loan above cash is refused even though custody covers it.
    let too_big = after_loan.cash + gift / 2;
    assert_contract_error(
        t.try_flash_loan_with_data(EVE, "USDC", too_big, &receiver, &Bytes::new(&t.env)),
        errors::INSUFFICIENT_LIQUIDITY,
    );

    // 4. So is a withdrawal that only the donation would cover.
    assert_contract_error(
        t.try_withdraw_raw(BOB, "USDC", after_loan.cash + gift / 2),
        errors::INSUFFICIENT_LIQUIDITY,
    );

    // 5. A revenue claim pays out of cash and leaves the donation in custody.
    let acc_before = balance_of(&t, &accumulator, "USDC");
    let paid = t.claim_revenue("USDC");
    let claimed = pool_state(&t, "USDC");
    assert!(paid > 0 && paid <= after_loan.cash, "paid {paid}");
    assert_eq!(claimed.cash, after_loan.cash - paid);
    assert_eq!(custody(&t, "USDC") - claimed.cash, gift);
    assert_eq!(balance_of(&t, &accumulator, "USDC") - acc_before, paid);

    // 6. Withdrawing exactly the remaining cash pays from cash and nothing else.
    let cash_now = claimed.cash;
    let bob_wallet = t.token_balance_raw(BOB, "USDC");
    t.withdraw_raw(BOB, "USDC", cash_now);
    assert_eq!(pool_state(&t, "USDC").cash, 0);
    assert_eq!(t.token_balance_raw(BOB, "USDC") - bob_wallet, cash_now);
    assert_eq!(custody(&t, "USDC"), gift, "only the donation remains");
    Ok(())
}

/// H5 (INV-ACCT-06, full branch): with cash above the revenue value, one claim pays
/// exactly the floored revenue value, burns every revenue share, and forwards the
/// measured receipt. A second claim pays zero.
#[test]
fn rv_claim_revenue_full_payout_pays_floor_value_and_burns_all_shares() {
    let mut t = zero_seed_book(false);
    let accumulator = install_accumulator(&t);
    t.supply(BOB, "USDC", 10_000.0);
    t.supply(ALICE, "ETH", 2.0);
    t.borrow(ALICE, "USDC", 1_000.0);
    t.advance_time(days(365));
    t.update_indexes_for(&["USDC"]);

    let before = pool_state(&t, "USDC");
    let value = treasury_value(&t, "USDC");
    assert!(before.revenue > 0, "fixture must accrue revenue");
    assert!(
        value > 0 && value < before.cash,
        "fixture must be a full payout: value {value}, cash {}",
        before.cash
    );

    let acc_before = balance_of(&t, &accumulator, "USDC");
    assert_eq!(t.claim_revenue("USDC"), value);
    let after = pool_state(&t, "USDC");
    assert_eq!(after.revenue, 0, "full payout burns every revenue share");
    assert_eq!(after.supplied, before.supplied - before.revenue);
    assert_eq!(after.cash, before.cash - value);
    assert_eq!(balance_of(&t, &accumulator, "USDC") - acc_before, value);

    assert_eq!(t.claim_revenue("USDC"), 0, "second claim pays nothing");
    assert_eq!(balance_of(&t, &accumulator, "USDC") - acc_before, value);
}

/// H5 (INV-ACCT-06, cash-limited branch, utilization ceiling disabled): the claim
/// pays exactly the cash, burns the pro-rata ceiling of revenue shares, and leaves
/// the rest claimable.
#[test]
fn rv_claim_revenue_cash_limited_pays_cash_and_burns_pro_rata_shares() {
    let mut t = zero_seed_book(true);
    let accumulator = install_accumulator(&t);
    t.supply(BOB, "USDC", 10_000.0);
    t.supply(ALICE, "ETH", 20.0);
    t.borrow(ALICE, "USDC", 9_700.0);
    t.advance_time(days(365));
    t.update_indexes_for(&["USDC"]);

    let before = pool_state(&t, "USDC");
    let value = treasury_value(&t, "USDC");
    assert_eq!(before.cash, 3_000_000_000, "300 USDC left after the borrow");
    assert!(
        value > before.cash,
        "fixture must be cash-limited: value {value}, cash {}",
        before.cash
    );

    let acc_before = balance_of(&t, &accumulator, "USDC");
    let paid = t.claim_revenue("USDC");
    assert_eq!(paid, before.cash, "pays exactly the cash");
    let after = pool_state(&t, "USDC");
    assert_eq!(after.cash, 0);
    assert!(after.revenue > 0, "revenue remains claimable later");
    let burned = before.revenue - after.revenue;
    let env = t.env.clone();
    let expected_burn = Ray::from(before.revenue)
        .mul_ratio_ceil(&env, paid, value)
        .raw();
    numbers("H5 cash-limited: paid", paid);
    numbers("H5 cash-limited: revenue shares burned", burned);
    assert_eq!(
        burned, expected_burn,
        "burn is ceil(revenue * paid / value)"
    );
    assert_eq!(after.supplied, before.supplied - burned);
    assert_eq!(balance_of(&t, &accumulator, "USDC") - acc_before, paid);
    assert_eq!(t.claim_revenue("USDC"), 0, "no cash left, so nothing paid");
}

/// Characterization (INV-ACCT-08): with the default 95% ceiling, a claim that would
/// pay out all remaining cash pushes utilization to 100% and is refused. Revenue
/// stays claimable only once cash covers it.
#[test]
fn rv_claim_revenue_cash_limited_reverts_under_default_utilization_ceiling() {
    let mut t = zero_seed_book(false);
    install_accumulator(&t);
    t.supply(BOB, "USDC", 10_000.0);
    t.supply(ALICE, "ETH", 20.0);
    t.borrow(ALICE, "USDC", 9_400.0);
    t.advance_time(days(365));
    t.update_indexes_for(&["USDC"]);

    let before = pool_state(&t, "USDC");
    let value = treasury_value(&t, "USDC");
    assert_eq!(before.cash, 6_000_000_000, "600 USDC left after the borrow");
    assert!(
        value > before.cash,
        "fixture must be cash-limited: value {value}, cash {}",
        before.cash
    );
    assert_contract_error(t.try_claim_revenue("USDC"), errors::UTILIZATION_ABOVE_MAX);
    let after = pool_state(&t, "USDC");
    assert_eq!(after.cash, before.cash);
    assert_eq!(after.revenue, before.revenue);
    assert_eq!(after.supplied, before.supplied);
}

/// H7 (INV-ACCT-05, exact close): repaying more than the ceiled debt burns every debt
/// share and refunds exactly `amount - ceil(debt)`.
#[test]
fn rv_repay_above_ceiled_debt_burns_all_shares_and_refunds_exact_excess() {
    let mut t = zero_seed_book(false);
    t.supply(BOB, "USDC", 10_000.0);
    t.supply(ALICE, "ETH", 2.0);
    t.borrow(ALICE, "USDC", 1_000.0);
    t.advance_and_sync(days(30));

    let debt_ceil = position_of(&t, ALICE, "USDC").debt_ceil;
    assert!(debt_ceil > 1_000_000_000, "interest must have accrued");
    let overpay: i128 = 1_234_567;
    let wallet = t.token_balance_raw(ALICE, "USDC");
    t.repay_raw(ALICE, "USDC", debt_ceil + overpay);

    assert_eq!(
        t.token_balance_raw(ALICE, "USDC") - wallet,
        overpay,
        "refund is amount - ceil(debt)"
    );
    assert_eq!(
        position_of(&t, ALICE, "USDC").debt,
        0,
        "every debt share is burned"
    );
    assert_eq!(pool_state(&t, "USDC").borrowed, 0);
    assert_eq!(t.borrow_balance_raw(ALICE, "USDC"), 0);
    assert_eq!(
        t.supply_balance_raw(ALICE, "ETH"),
        20_000_000,
        "collateral untouched"
    );
}

/// H7 (INV-ACCT-05, one unit short): repaying `ceil(debt) - 1` burns exactly the
/// floored shares, leaves a positive residual, and paying that residual's ceiling
/// closes the position with no refund.
#[test]
fn rv_repay_one_unit_below_ceiled_debt_leaves_positive_residual() {
    let mut t = zero_seed_book(false);
    t.supply(BOB, "USDC", 10_000.0);
    t.supply(ALICE, "ETH", 2.0);
    t.borrow(ALICE, "USDC", 1_000.0);
    t.advance_and_sync(days(30));

    let before = position_of(&t, ALICE, "USDC");
    let (scaled_before, debt_ceil) = (before.debt, before.debt_ceil);
    let borrow_index = pool_state(&t, "USDC").borrow_index;
    let env = t.env.clone();
    let burned = Ray::from_asset(&env, debt_ceil - 1, 7)
        .div_floor(&env, Ray::from(borrow_index))
        .raw();

    t.repay_raw(ALICE, "USDC", debt_ceil - 1);
    let after = position_of(&t, ALICE, "USDC");
    let (scaled_left, residual_ceil) = (after.debt, after.debt_ceil);
    assert_eq!(
        scaled_left,
        scaled_before - burned,
        "partial burn is floor(amount / index)"
    );
    assert!(scaled_left > 0, "a positive residual remains");
    assert!(
        residual_ceil >= 1,
        "residual debt is at least one base unit"
    );
    assert_eq!(pool_state(&t, "USDC").borrowed, scaled_left);

    let wallet = t.token_balance_raw(ALICE, "USDC");
    t.repay_raw(ALICE, "USDC", residual_ceil);
    assert_eq!(
        t.token_balance_raw(ALICE, "USDC"),
        wallet,
        "exact close, no refund"
    );
    assert_eq!(t.borrow_balance_raw(ALICE, "USDC"), 0);
    assert_eq!(pool_state(&t, "USDC").borrowed, 0);
}

/// Same-asset netting (INV-ACCT-05, INV-ACCT-02): `repay_debt_with_collateral` with
/// collateral equal to the debt burns matched supply and debt shares, moves no
/// tokens and leaves cash unchanged, capped at the ceiled debt.
#[test]
fn rv_repay_debt_with_collateral_same_asset_nets_without_moving_cash() {
    let mut t = zero_seed_book(false);
    t.supply(ALICE, "ETH", 10.0);
    t.supply(BOB, "ETH", 20.0);
    t.borrow(ALICE, "ETH", 2.0);
    let before = pool_state(&t, "ETH");
    let custody_before = custody(&t, "ETH");
    let wallet_before = t.token_balance_raw(ALICE, "ETH");

    // Index is still RAY, so 1 ETH = 1e7 base units = RAY scaled shares.
    assert_eq!(
        t.try_repay_debt_with_collateral(ALICE, "ETH", 1.0, "ETH", &Bytes::new(&t.env), false),
        Ok(())
    );
    let mid = pool_state(&t, "ETH");
    assert_eq!(
        before.supplied - mid.supplied,
        RAY,
        "1 ETH of supply shares burned"
    );
    assert_eq!(
        before.borrowed - mid.borrowed,
        RAY,
        "1 ETH of debt shares burned"
    );
    assert_eq!(mid.cash, before.cash);
    assert_eq!(custody(&t, "ETH"), custody_before);
    assert_eq!(t.token_balance_raw(ALICE, "ETH"), wallet_before);
    assert_eq!(t.supply_balance_raw(ALICE, "ETH"), 90_000_000);
    assert_eq!(t.borrow_balance_raw(ALICE, "ETH"), 10_000_000);

    // Asking for 50 ETH settles only the remaining 1 ETH of debt.
    assert_eq!(
        t.try_repay_debt_with_collateral(ALICE, "ETH", 50.0, "ETH", &Bytes::new(&t.env), false),
        Ok(())
    );
    let after = pool_state(&t, "ETH");
    assert_eq!(after.borrowed, 0);
    assert_eq!(t.borrow_balance_raw(ALICE, "ETH"), 0);
    assert_eq!(t.supply_balance_raw(ALICE, "ETH"), 80_000_000);
    assert_eq!(after.cash, before.cash);
    assert_eq!(custody(&t, "ETH"), custody_before);
    assert_eq!(shortfall(&t, "ETH"), 0);
}
