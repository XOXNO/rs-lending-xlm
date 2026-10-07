//! W3 authorization matrix: who may do what over an account and over the controls.
//!
//! Covers INV-AUTH-01 (pool owner gate), INV-AUTH-02 (owner, delegate and NFT-holder
//! spending authority), INV-AUTH-03 (third-party supply), INV-LIQ-01 (credit receivers),
//! and the owner gates of the controller, the price aggregator and two-step ownership.
//!
//! The harness mocks every signature, so a call that reaches the controller's own owner,
//! delegate or NFT check ends in a contract error (`Out::Contract`). A gate that sits in
//! the signature check ends in a host error that is not a contract error (`Out::Host`).
//! `no_signers` and `only_signer` install the auth regime for the next call, and `restore`
//! returns the harness to recording mode.

use common::errors::GenericError;
use common::types::{
    PoolAction, PoolBorrowEntry, PoolNetSettleEntry, PoolSeizeEntry, PoolSupplyEntry,
    PoolWithdrawEntry, ScaledPositionRaw,
};
use controller::constants::RAY;
use controller::types::{
    AssetOracle, ControllerKey, DelegateGrant, InterestRateModel, MarketParamsRaw, OracleTolerance,
    PositionMode, PriceKey, SeizeMode, SpokeAssetArgs,
};
use pool::LiquidityPoolClient;
use soroban_sdk::testutils::{Address as _, AuthorizedFunction, Ledger, MockAuth, MockAuthInvoke};
use soroban_sdk::xdr::{ScErrorType, SorobanAuthorizationEntry};
use soroban_sdk::{token, vec, Address, Bytes, BytesN, Env, IntoVal, Symbol, Val, Vec};
use test_harness::mock_blend::{KIND_COLLATERAL, KIND_LIABILITY};
use test_harness::{
    build_aggregator_swap, errors, f64_to_i128, hub_asset, usd_cents, usdc_preset, HubAssetKey,
    LendingTest, ALICE, BOB, CAROL, DAVE, EVE, HARNESS_HUB, HARNESS_SPOKE, LIQUIDATOR,
    STABLECOIN_SPOKE,
};

/// Codes this file pins that the harness error table does not list.
const REGISTRY_CAP_REACHED: u32 = GenericError::RegistryCapReached as u32;
/// OpenZeppelin `RoleTransferError::TransferExpired`.
const TRANSFER_EXPIRED: u32 = 2203;

const ASSETS: [&str; 3] = ["USDC", "ETH", "WBTC"];

/// Pool mutators in the order `pool_probe` and `pool_mock_args` index them.
const POOL_FNS: [&str; 14] = [
    "create_market",
    "update_params",
    "upgrade",
    "supply",
    "borrow",
    "withdraw",
    "repay",
    "update_indexes",
    "recapitalize",
    "flash_loan",
    "create_strategy",
    "seize_positions",
    "net_settle",
    "claim_revenue",
];

/// Mutators that succeed on the probe input when the controller signs, so the owner recorded
/// for them is readable. The remaining mutators fail after the gate, which drops the record.
const POOL_RECORDED_FNS: [usize; 9] = [3, 4, 5, 6, 7, 8, 11, 12, 13];

// --- helpers ----------------------------------------------------------------

/// Amount in 7-decimal base units (the USDC, ETH and WBTC presets).
fn raw(amount: f64) -> i128 {
    f64_to_i128(amount, 7)
}

fn mint(t: &LendingTest, who: &Address, asset: &str, amount: i128) {
    t.resolve_market(asset).token_admin.mint(who, &amount);
}

fn token_bal(t: &LendingTest, who: &Address, asset: &str) -> i128 {
    token::Client::new(&t.env, &t.resolve_asset(asset)).balance(who)
}

fn pool_bal(t: &LendingTest, asset: &str) -> i128 {
    token_bal(t, &t.resolve_market(asset).pool, asset)
}

fn per_asset(f: impl Fn(&str) -> i128) -> [i128; 3] {
    ASSETS.map(f)
}

/// `[supplied, borrowed, revenue, borrow_index, supply_index, last_timestamp, cash]`.
fn pool_fields(t: &LendingTest, asset: &str) -> [i128; 7] {
    let state = t
        .pool_client(asset)
        .get_sync_data(&hub_asset(t.resolve_asset(asset)))
        .state;
    [
        state.supplied,
        state.borrowed,
        state.revenue,
        state.borrow_index,
        state.supply_index,
        i128::from(state.last_timestamp),
        state.cash,
    ]
}

fn nft_owner(t: &LendingTest, account_id: u64) -> Option<Address> {
    let id = u32::try_from(account_id).ok()?;
    match position_nft::PositionNftClient::new(&t.env, &t.position_nft).try_owner_of(&id) {
        Ok(Ok(owner)) => Some(owner),
        _ => None,
    }
}

/// The delegate grant stored under the account, whether or not it is live for its owner.
fn stored_grant(t: &LendingTest, account_id: u64) -> Option<DelegateGrant> {
    t.env.as_contract(&t.controller, || {
        t.env
            .storage()
            .persistent()
            .get::<_, DelegateGrant>(&ControllerKey::Delegates(account_id))
    })
}

fn scaled_supply(t: &LendingTest, account_id: u64, asset: &str) -> i128 {
    let (supplies, _) = t.ctrl_client().get_account_positions(&account_id);
    supplies
        .get(hub_asset(t.resolve_asset(asset)))
        .map_or(0, |position| position.scaled_amount)
}

/// Result of one call, split by where it failed.
#[derive(Debug, Clone, PartialEq)]
enum Out {
    Ok,
    /// The controller or pool logic rejected the call with this contract error.
    Contract(u32),
    /// A host error that is not a contract error: a `require_auth` gate or a host trap.
    Host(String),
    /// The invocation failed without a host error value.
    Other(String),
}

fn classify(err: soroban_sdk::Error) -> Out {
    if err.is_type(ScErrorType::Contract) {
        Out::Contract(err.get_code())
    } else {
        Out::Host(format!("{err:?}"))
    }
}

fn out<T, E: Into<soroban_sdk::Error>>(
    result: Result<Result<T, E>, Result<soroban_sdk::Error, soroban_sdk::InvokeError>>,
) -> Out {
    match result {
        Ok(Ok(_)) => Out::Ok,
        Ok(Err(err)) => classify(err.into()),
        Err(Ok(err)) => classify(err),
        Err(Err(err)) => Out::Other(format!("{err:?}")),
    }
}

/// Uploads a built contract from the workspace target directory and returns its hash. The
/// test binary runs from the crate directory, so the workspace root is two levels up.
fn upload_wasm(env: &Env, file: &str) -> BytesN<32> {
    let rel = format!("target/wasm32v1-none/release/{file}");
    let bytes = ["", "../", "../../"]
        .iter()
        .find_map(|prefix| std::fs::read(format!("{prefix}{rel}")).ok())
        .unwrap_or_else(|| panic!("{file} not found; run `make build` first"));
    env.deployer()
        .upload_contract_wasm(Bytes::from_slice(env, &bytes))
}

/// Every signature is refused for the next invocation.
fn no_signers(env: &Env) {
    let none: [SorobanAuthorizationEntry; 0] = [];
    env.set_auths(&none);
}

/// Only `who` may sign, and only for the exact invocation `fn_name(args)` on `contract`.
fn only_signer(env: &Env, who: &Address, contract: &Address, fn_name: &str, args: Vec<Val>) {
    let invoke = MockAuthInvoke {
        contract,
        fn_name,
        args,
        sub_invokes: &[],
    };
    env.mock_auths(&[MockAuth {
        address: who,
        invoke: &invoke,
    }]);
}

fn restore(env: &Env) {
    env.mock_all_auths_allowing_non_root_auth();
}

/// One account as the controller and the position NFT report it.
#[derive(Debug, Clone, PartialEq)]
struct AcctView {
    id: u64,
    owner: Option<Address>,
    supply: [i128; 3],
    borrow: [i128; 3],
    positions: (u32, u32),
    collateral_usd: i128,
    debt_usd: i128,
    hf: i128,
    attrs: Option<(u32, PositionMode)>,
}

fn acct_view(t: &LendingTest, id: u64) -> AcctView {
    let ctrl = t.ctrl_client();
    let live = ctrl.account_exists(&id);
    let (supplies, borrows) = ctrl.get_account_positions(&id);
    let attrs = live.then(|| {
        let attributes = ctrl.get_account_attributes(&id);
        (attributes.spoke_id, attributes.mode)
    });
    AcctView {
        id,
        owner: nft_owner(t, id),
        supply: per_asset(|asset| t.supply_balance_raw_for(id, asset)),
        borrow: per_asset(|asset| t.borrow_balance_raw_for(id, asset)),
        positions: (supplies.len(), borrows.len()),
        collateral_usd: if live {
            ctrl.get_total_collateral_usd(&id)
        } else {
            0
        },
        debt_usd: if live {
            ctrl.get_total_borrow_usd(&id)
        } else {
            0
        },
        hf: if live { ctrl.get_health_factor(&id) } else { 0 },
        attrs,
    }
}

/// Everything a refused call must leave alone: wallets, the controller's and pool's token
/// custody, every market's pool state, and the listed accounts.
#[derive(Debug, Clone, PartialEq)]
struct World {
    wallets: std::vec::Vec<[i128; 3]>,
    pool_tokens: [i128; 3],
    controller_tokens: [i128; 3],
    pools: std::vec::Vec<[i128; 7]>,
    accounts: std::vec::Vec<AcctView>,
}

fn world(t: &LendingTest, people: &[Address], accounts: &[u64]) -> World {
    World {
        wallets: people
            .iter()
            .map(|who| per_asset(|asset| token_bal(t, who, asset)))
            .collect(),
        pool_tokens: per_asset(|asset| pool_bal(t, asset)),
        controller_tokens: per_asset(|asset| t.controller_token_balance_raw(asset)),
        pools: ASSETS.iter().map(|asset| pool_fields(t, asset)).collect(),
        accounts: accounts.iter().map(|id| acct_view(t, *id)).collect(),
    }
}

/// Asserts the call was refused with `code` and that the world did not move.
fn rejected(
    label: &str,
    got: Out,
    code: u32,
    t: &LendingTest,
    people: &[Address],
    accounts: &[u64],
    before: &World,
) {
    assert_eq!(got, Out::Contract(code), "{label}: wrong rejection");
    assert_eq!(
        &world(t, people, accounts),
        before,
        "{label}: a rejected call changed balances, pool state or positions"
    );
}

/// Alice: 10 000 USDC supplied and 3 ETH borrowed (health factor 1.33), with an empty
/// Multiply-mode account and a second Normal account. Carol holds 1 000 of each asset.
/// Returns `(world, alice's account, the Multiply account, the second Normal account)`.
fn alice_book() -> (LendingTest, u64, u64, u64) {
    let mut t = LendingTest::new().three_asset_usdc_eth_wbtc().build();
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    let acc = t.account_id(ALICE);
    let mul = t.create_account_full(ALICE, HARNESS_SPOKE, PositionMode::Multiply);
    let other = t.create_account_full(ALICE, HARNESS_SPOKE, PositionMode::Normal);
    let carol = t.get_or_create_user(CAROL);
    for asset in ASSETS {
        mint(&t, &carol, asset, raw(1_000.0));
    }
    (t, acc, mul, other)
}

// --- H1: stranger matrix -------------------------------------------------------

/// INV-AUTH-02, INV-AUTH-03, INV-LIQ-01. Carol is neither owner nor delegate. Every
/// owner-only verb is refused with the code the controller returns, and no balance, pool
/// state or position moves.
#[test]
fn rv_owner_verbs_stranger_carol_rejected_no_side_effects() {
    let (mut t, acc, mul, other) = alice_book();
    let carol = t.get_or_create_user(CAROL);
    let alice = t.get_or_create_user(ALICE);
    let dave = t.get_or_create_user(DAVE);
    t.ctrl_client().set_position_manager(&dave, &true);
    let receiver = t.deploy_flash_position_receiver();
    t.ensure_approved_blend();
    t.seed_blend(CAROL, "USDC", KIND_COLLATERAL, 1_000.0);
    t.seed_blend(CAROL, "ETH", KIND_LIABILITY, 0.1);

    let people = [carol.clone(), alice.clone()];
    let accounts = [acc, mul, other];
    let usdc = hub_asset(t.resolve_asset("USDC"));
    let eth = hub_asset(t.resolve_asset("ETH"));
    let wbtc = hub_asset(t.resolve_asset("WBTC"));
    let steps = build_aggregator_swap(&t, "ETH", "USDC", 0, 1);
    let empty = Bytes::new(&t.env);
    let before = world(&t, &people, &accounts);

    // Spending verbs need the owner or an active delegate.
    let got = out(t.ctrl_client().try_borrow(
        &carol,
        &acc,
        &vec![&t.env, (eth.clone(), raw(0.1))],
        &None,
    ));
    rejected(
        "borrow",
        got,
        errors::NOT_AUTHORIZED,
        &t,
        &people,
        &accounts,
        &before,
    );

    let got = out(t.ctrl_client().try_withdraw(
        &carol,
        &acc,
        &vec![&t.env, (usdc.clone(), raw(1_000.0))],
        &None,
    ));
    rejected(
        "withdraw",
        got,
        errors::NOT_AUTHORIZED,
        &t,
        &people,
        &accounts,
        &before,
    );

    // INV-AUTH-03: a stranger cannot open a new asset slot in someone else's account.
    let got = out(t.ctrl_client().try_supply(
        &carol,
        &acc,
        &HARNESS_SPOKE,
        &vec![&t.env, (wbtc.clone(), raw(0.1))],
    ));
    rejected(
        "supply new WBTC slot",
        got,
        errors::NOT_AUTHORIZED,
        &t,
        &people,
        &accounts,
        &before,
    );

    // INV-AUTH-02: delegate administration is owner-only. The controller reports a
    // non-owner as AccountNotInMarket, not NotAuthorized.
    let got = out(t.ctrl_client().try_add_delegate(&carol, &acc, &dave));
    rejected(
        "add_delegate",
        got,
        errors::ACCOUNT_NOT_IN_MARKET,
        &t,
        &people,
        &accounts,
        &before,
    );

    let got = out(t.ctrl_client().try_remove_delegate(&carol, &acc, &dave));
    rejected(
        "remove_delegate",
        got,
        errors::ACCOUNT_NOT_IN_MARKET,
        &t,
        &people,
        &accounts,
        &before,
    );

    let got = out(t.ctrl_client().try_renew_account(&carol, &acc));
    rejected(
        "renew_account",
        got,
        errors::ACCOUNT_NOT_IN_MARKET,
        &t,
        &people,
        &accounts,
        &before,
    );

    // Strategy verbs check the owner or delegate before any swap runs.
    let got = out(t
        .ctrl_client()
        .try_swap_debt(&carol, &acc, &eth, &raw(1_000.0), &usdc, &steps));
    rejected(
        "swap_debt",
        got,
        errors::NOT_AUTHORIZED,
        &t,
        &people,
        &accounts,
        &before,
    );

    let got =
        out(t
            .ctrl_client()
            .try_swap_collateral(&carol, &acc, &usdc, &raw(1_000.0), &wbtc, &steps));
    rejected(
        "swap_collateral",
        got,
        errors::NOT_AUTHORIZED,
        &t,
        &people,
        &accounts,
        &before,
    );

    let got = out(t.ctrl_client().try_repay_debt_with_collateral(
        &carol,
        &acc,
        &usdc,
        &raw(1_000.0),
        &eth,
        &steps,
        &false,
    ));
    rejected(
        "repay_debt_with_collateral",
        got,
        errors::NOT_AUTHORIZED,
        &t,
        &people,
        &accounts,
        &before,
    );

    // Multiply, flash_position and migrate_from_blend on Alice's existing accounts.
    let got = out(t.ctrl_client().try_multiply(
        &carol,
        &mul,
        &HARNESS_SPOKE,
        &usdc,
        &raw(0.1),
        &eth,
        &PositionMode::Multiply,
        &steps,
        &None,
        &None,
    ));
    rejected(
        "multiply",
        got,
        errors::NOT_AUTHORIZED,
        &t,
        &people,
        &accounts,
        &before,
    );

    let got = out(t.ctrl_client().try_flash_position(
        &carol,
        &mul,
        &HARNESS_SPOKE,
        &PositionMode::Multiply,
        &eth,
        &raw(0.1),
        &receiver,
        &empty,
        &vec![&t.env, (usdc.clone(), raw(100.0))],
        &Vec::new(&t.env),
    ));
    rejected(
        "flash_position",
        got,
        errors::NOT_AUTHORIZED,
        &t,
        &people,
        &accounts,
        &before,
    );

    let got = match t.try_migrate_from_blend(CAROL, acc, &["USDC"], &[], &[("ETH", 0.1)]) {
        Ok(_) => Out::Ok,
        Err(err) => classify(err),
    };
    rejected(
        "migrate_from_blend",
        got,
        errors::NOT_AUTHORIZED,
        &t,
        &people,
        &accounts,
        &before,
    );

    // INV-LIQ-01: Carol cannot credit seized shares into Alice's other account. Alice is
    // made liquidatable first, so a missing check would move value.
    t.set_price("USDC", usd_cents(50));
    assert!(t.can_be_liquidated_by_id(acc), "Alice must be liquidatable");
    let before = world(&t, &people, &accounts);
    let got = out(t.ctrl_client().try_liquidate(
        &carol,
        &acc,
        &vec![&t.env, (eth.clone(), raw(1.0))],
        &SeizeMode::Credit(other),
    ));
    rejected(
        "liquidate Credit(other)",
        got,
        errors::NOT_AUTHORIZED,
        &t,
        &people,
        &accounts,
        &before,
    );
}

/// INV-AUTH-03 and INV-LIQ-01. The verbs that need no ownership run for Carol and move
/// exactly the stated amounts. Transfer-mode liquidation pays Carol the
/// seizure net of the protocol fee and retires exactly the debt she paid.
#[test]
fn rv_stranger_topup_repay_liquidate_recapitalize_exact_amounts() {
    let (mut t, acc, _mul, _other) = alice_book();
    let carol = t.get_or_create_user(CAROL);
    let usdc = hub_asset(t.resolve_asset("USDC"));
    let eth = hub_asset(t.resolve_asset("ETH"));

    // Top-up of an asset Alice already holds.
    let supply_before = t.supply_balance_raw_for(acc, "USDC");
    let carol_usdc = token_bal(&t, &carol, "USDC");
    let pool_usdc = pool_bal(&t, "USDC");
    let id = t.ctrl_client().supply(
        &carol,
        &acc,
        &HARNESS_SPOKE,
        &vec![&t.env, (usdc.clone(), raw(500.0))],
    );
    assert_eq!(id, acc, "a top-up returns the same account");
    assert_eq!(
        t.supply_balance_raw_for(acc, "USDC") - supply_before,
        raw(500.0)
    );
    assert_eq!(carol_usdc - token_bal(&t, &carol, "USDC"), raw(500.0));
    assert_eq!(pool_bal(&t, "USDC") - pool_usdc, raw(500.0));

    // Repay Alice's debt: the debt falls by exactly what Carol paid into the pool.
    let debt_before = t.borrow_balance_raw_for(acc, "ETH");
    let carol_eth = token_bal(&t, &carol, "ETH");
    let pool_eth = pool_bal(&t, "ETH");
    t.ctrl_client()
        .repay(&carol, &acc, &vec![&t.env, (eth.clone(), raw(0.5))]);
    assert_eq!(debt_before - t.borrow_balance_raw_for(acc, "ETH"), raw(0.5));
    assert_eq!(carol_eth - token_bal(&t, &carol, "ETH"), raw(0.5));
    assert_eq!(pool_bal(&t, "ETH") - pool_eth, raw(0.5));

    // Permissionless maintenance: no time has passed, so no state moves.
    let pool_state = pool_fields(&t, "USDC");
    t.ctrl_client()
        .update_indexes(&carol, &vec![&t.env, usdc.clone(), eth.clone()]);
    assert_eq!(
        pool_fields(&t, "USDC"),
        pool_state,
        "no elapsed time, no accrual"
    );

    let hf = t.ctrl_client().get_health_factor(&acc);
    t.ctrl_client()
        .update_account_threshold(&carol, &false, &vec![&t.env, acc]);
    assert_eq!(t.ctrl_client().get_health_factor(&acc), hf);

    // Recapitalize: the market is solvent, so nothing is credited and the payment returns.
    let carol_usdc = token_bal(&t, &carol, "USDC");
    let applied = t.ctrl_client().recapitalize(&carol, &usdc, &raw(100.0));
    assert_eq!(applied, 0, "a solvent market has no shortfall to cover");
    assert_eq!(
        token_bal(&t, &carol, "USDC"),
        carol_usdc,
        "the payment is refunded"
    );

    let claimed = t
        .ctrl_client()
        .claim_revenue(&carol, &vec![&t.env, usdc.clone()]);
    assert_eq!(claimed.len(), 1);
    assert_eq!(
        claimed.get(0).unwrap(),
        0,
        "no revenue accrued at zero elapsed time"
    );

    // Transfer-mode liquidation by a stranger once Alice's health factor is below 1.
    t.set_price("USDC", usd_cents(50));
    assert!(t.can_be_liquidated_by_id(acc));
    let payments = vec![&t.env, (eth.clone(), raw(1.0))];
    let estimate = t
        .ctrl_client()
        .get_liquidation_estimate(&acc, &payments, &SeizeMode::Transfer);
    let seized = estimate.seized_collaterals.get(0).unwrap().amount;
    let fee = estimate.protocol_fees.get(0).unwrap().amount;
    let hf_before = t.ctrl_client().get_health_factor(&acc);
    let (debt, supply) = (
        t.borrow_balance_raw_for(acc, "ETH"),
        t.supply_balance_raw_for(acc, "USDC"),
    );
    let (carol_eth, carol_usdc) = (token_bal(&t, &carol, "ETH"), token_bal(&t, &carol, "USDC"));
    let (pool_eth, pool_usdc) = (pool_bal(&t, "ETH"), pool_bal(&t, "USDC"));

    let receiver = t
        .ctrl_client()
        .liquidate(&carol, &acc, &payments, &SeizeMode::Transfer);
    assert_eq!(receiver, 0, "Transfer mode returns no receiver account");

    let paid = carol_eth - token_bal(&t, &carol, "ETH");
    assert!(paid > 0 && paid <= raw(1.0), "paid {paid}");
    assert_eq!(
        pool_bal(&t, "ETH") - pool_eth,
        paid,
        "the pool keeps what Carol paid"
    );
    assert_eq!(
        debt - t.borrow_balance_raw_for(acc, "ETH"),
        paid,
        "debt retired equals paid"
    );

    let gain = token_bal(&t, &carol, "USDC") - carol_usdc;
    assert_eq!(
        gain,
        seized - fee,
        "the liquidator receives the seizure net of the fee"
    );
    assert_eq!(
        pool_usdc - pool_bal(&t, "USDC"),
        gain,
        "the pool pays exactly the gain"
    );
    assert_eq!(
        supply - t.supply_balance_raw_for(acc, "USDC"),
        seized,
        "Alice's collateral falls by the gross seizure"
    );
    // formulas.md, "HF-preserving bonus cap": 1 + b <= C / D, so a liquidation never lowers
    // the health factor. Here the cap binds at b = 5% because LT x (1 + b) = HF = 0.84.
    let hf_after = t.ctrl_client().get_health_factor(&acc);
    assert!(
        hf_after >= hf_before,
        "liquidation lowered the health factor: {hf_before} -> {hf_after}"
    );
}

/// INV-LIQ-04 and INV-AUTH-03 (permissionless cleanup). Carol may socialize an eligible dust
/// account: insolvent, with collateral at or below the $5 cap.
#[test]
fn rv_stranger_clean_bad_debt_socializes_eligible_dust_account() {
    let mut t = LendingTest::new().standard_two_asset_dust_disabled();
    t.supply(ALICE, "USDC", 10.0);
    t.borrow(ALICE, "ETH", 0.003);
    let acc = t.account_id(ALICE);
    // Collateral $1 against $6 of debt: insolvent, and the collateral is under the cap.
    t.set_price("USDC", usd_cents(10));
    let carol = t.get_or_create_user(CAROL);
    let debt = t.borrow_balance_raw_for(acc, "ETH");
    let eth_borrowed = pool_fields(&t, "ETH")[1];

    let got = out(t.ctrl_client().try_clean_bad_debt(&carol, &acc));
    assert_eq!(got, Out::Ok, "a stranger may clean an eligible account");
    assert!(!t.account_exists(acc), "the socialized account is removed");
    assert_eq!(nft_owner(&t, acc), None, "and its NFT is burned");
    // The pool stores borrowed totals as RAY-scaled shares, so the raw debt is scaled by
    // RAY / 10^7 before comparing.
    assert_eq!(
        eth_borrowed - pool_fields(&t, "ETH")[1],
        debt * (RAY / 10_000_000),
        "the socialized ETH debt leaves the pool's borrowed total"
    );
    assert_eq!(
        token_bal(&t, &carol, "USDC"),
        0,
        "the cleaner receives nothing"
    );
}

// --- H2: delegate lifecycle ------------------------------------------------------

/// INV-AUTH-02. Dave's spending authority follows the stored grant, the active manager flag
/// and the NFT owner, and the balances show each step. Dave borrows to his own wallet.
#[test]
fn rv_delegate_lifecycle_dave_balances_follow_grant_flag_and_nft() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(ALICE, "USDC", 10_000.0);
    let acc = t.account_id(ALICE);
    let other = t.create_account_full(ALICE, HARNESS_SPOKE, PositionMode::Normal);
    let alice = t.get_or_create_user(ALICE);
    let bob = t.get_or_create_user(BOB);
    let dave = t.get_or_create_user(DAVE);
    let eve = t.get_or_create_user(EVE);
    let eth = hub_asset(t.resolve_asset("ETH"));
    let usdc = hub_asset(t.resolve_asset("USDC"));
    t.ctrl_client().set_position_manager(&dave, &true);

    // Grants are per account: Dave has no authority over Alice's second account.
    let got = out(t.ctrl_client().try_borrow(
        &dave,
        &other,
        &vec![&t.env, (eth.clone(), raw(0.1))],
        &Some(dave.clone()),
    ));
    assert_eq!(
        got,
        Out::Contract(errors::NOT_AUTHORIZED),
        "no grant on the other account"
    );

    // (a) Alice grants Dave on `acc`; Dave borrows 0.5 ETH to his own wallet.
    t.ctrl_client().add_delegate(&alice, &acc, &dave);
    let got = out(t.ctrl_client().try_borrow(
        &dave,
        &other,
        &vec![&t.env, (eth.clone(), raw(0.1))],
        &Some(dave.clone()),
    ));
    assert_eq!(
        got,
        Out::Contract(errors::NOT_AUTHORIZED),
        "a grant on acc is not a grant on other"
    );
    t.ctrl_client().borrow(
        &dave,
        &acc,
        &vec![&t.env, (eth.clone(), raw(0.5))],
        &Some(dave.clone()),
    );
    assert_eq!(
        token_bal(&t, &dave, "ETH"),
        raw(0.5),
        "Dave received exactly 0.5 ETH"
    );
    assert_eq!(t.borrow_balance_raw_for(acc, "ETH"), raw(0.5));

    // (b) Alice revokes. Dave's next borrow is refused and moves nothing.
    t.ctrl_client().remove_delegate(&alice, &acc, &dave);
    let got = out(t.ctrl_client().try_borrow(
        &dave,
        &acc,
        &vec![&t.env, (eth.clone(), raw(0.1))],
        &Some(dave.clone()),
    ));
    assert_eq!(got, Out::Contract(errors::NOT_AUTHORIZED));
    assert_eq!(token_bal(&t, &dave, "ETH"), raw(0.5));
    assert_eq!(t.borrow_balance_raw_for(acc, "ETH"), raw(0.5));

    // (c) Alice re-grants; governance deactivates Dave. The stored grant stays, but Dave is
    // refused while inactive.
    t.ctrl_client().add_delegate(&alice, &acc, &dave);
    t.ctrl_client().set_position_manager(&dave, &false);
    let got = out(t.ctrl_client().try_borrow(
        &dave,
        &acc,
        &vec![&t.env, (eth.clone(), raw(0.1))],
        &Some(dave.clone()),
    ));
    assert_eq!(got, Out::Contract(errors::NOT_AUTHORIZED));
    let grant = stored_grant(&t, acc).expect("the grant persists while the manager is inactive");
    assert_eq!(grant.granted_by, alice);
    assert!(grant.delegates.contains(&dave));
    assert_eq!(t.borrow_balance_raw_for(acc, "ETH"), raw(0.5));

    // (d) Governance reactivates Dave. Alice hands the NFT to Bob: Dave's grant is stamped
    // by Alice, so it is dead for Bob.
    t.ctrl_client().set_position_manager(&dave, &true);
    t.nft_transfer(ALICE, BOB, acc);
    assert_eq!(t.nft_owner_of(acc), bob);
    let got = out(t.ctrl_client().try_borrow(
        &dave,
        &acc,
        &vec![&t.env, (eth.clone(), raw(0.1))],
        &Some(dave.clone()),
    ));
    assert_eq!(
        got,
        Out::Contract(errors::NOT_AUTHORIZED),
        "Dave after the NFT moved"
    );
    let got = out(t.ctrl_client().try_withdraw(
        &alice,
        &acc,
        &vec![&t.env, (usdc.clone(), raw(1.0))],
        &None,
    ));
    assert_eq!(
        got,
        Out::Contract(errors::NOT_AUTHORIZED),
        "Alice after the NFT moved"
    );

    // Bob, the owner, withdraws 1 000 USDC to his wallet.
    t.ctrl_client().withdraw(
        &bob,
        &acc,
        &vec![&t.env, (usdc.clone(), raw(1_000.0))],
        &None,
    );
    assert_eq!(token_bal(&t, &bob, "USDC"), raw(1_000.0));
    assert_eq!(t.supply_balance_raw_for(acc, "USDC"), raw(9_000.0));

    // (e) Bob returns the NFT without touching delegates. Dave's grant revives.
    t.nft_transfer(BOB, ALICE, acc);
    assert_eq!(t.nft_owner_of(acc), alice);
    t.ctrl_client().borrow(
        &dave,
        &acc,
        &vec![&t.env, (eth.clone(), raw(0.2))],
        &Some(dave.clone()),
    );
    assert_eq!(
        token_bal(&t, &dave, "ETH"),
        raw(0.7),
        "Dave's second borrow landed"
    );
    assert_eq!(t.borrow_balance_raw_for(acc, "ETH"), raw(0.7));

    // Only the owner grants or revokes: Dave cannot add Eve, nor remove himself.
    let got = out(t.ctrl_client().try_add_delegate(&dave, &acc, &eve));
    assert_eq!(
        got,
        Out::Contract(errors::ACCOUNT_NOT_IN_MARKET),
        "a delegate cannot re-delegate"
    );
    let got = out(t.ctrl_client().try_remove_delegate(&dave, &acc, &dave));
    assert_eq!(
        got,
        Out::Contract(errors::ACCOUNT_NOT_IN_MARKET),
        "a delegate cannot revoke"
    );
    assert!(stored_grant(&t, acc).is_some_and(|g| g.delegates.contains(&dave)));

    // A delegate cannot move the token: no approval exists, and the owner is unchanged.
    let token_id = u32::try_from(acc).expect("small id");
    let nft = position_nft::PositionNftClient::new(&t.env, &t.position_nft);
    let got = out(nft.try_transfer_from(&dave, &alice, &dave, &token_id));
    assert_eq!(
        got,
        Out::Contract(202),
        "an unapproved transfer fails with NonFungibleTokenError::InsufficientApproval"
    );
    assert_eq!(t.nft_owner_of(acc), alice);

    // Payouts go to any recipient the delegate names (documented in INV-AUTH-02).
    t.ctrl_client().withdraw(
        &dave,
        &acc,
        &vec![&t.env, (usdc.clone(), raw(500.0))],
        &Some(eve.clone()),
    );
    assert_eq!(
        token_bal(&t, &eve, "USDC"),
        raw(500.0),
        "Eve received the delegate's payout"
    );
    assert_eq!(t.supply_balance_raw_for(acc, "USDC"), raw(8_500.0));
}

/// INV-AUTH-02. A new owner's write purges the stale grant. Bob grants Eve before the NFT
/// returns, so Alice's live delegates are empty afterwards, and Dave and Eve are both refused.
#[test]
fn rv_delegate_stale_grant_purged_by_new_owner_write() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(ALICE, "USDC", 10_000.0);
    let acc = t.account_id(ALICE);
    let alice = t.get_or_create_user(ALICE);
    let bob = t.get_or_create_user(BOB);
    let dave = t.get_or_create_user(DAVE);
    let eve = t.get_or_create_user(EVE);
    let eth = hub_asset(t.resolve_asset("ETH"));
    t.ctrl_client().set_position_manager(&dave, &true);
    t.ctrl_client().set_position_manager(&eve, &true);
    t.ctrl_client().add_delegate(&alice, &acc, &dave);

    t.nft_transfer(ALICE, BOB, acc);
    // Bob's write overwrites Alice's grant with one stamped by Bob.
    t.ctrl_client().add_delegate(&bob, &acc, &eve);
    let grant = stored_grant(&t, acc).expect("Bob's grant");
    assert_eq!(grant.granted_by, bob);
    assert_eq!(grant.delegates.len(), 1);
    assert!(grant.delegates.contains(&eve));
    assert!(
        !grant.delegates.contains(&dave),
        "Dave's entry is gone from storage"
    );

    t.nft_transfer(BOB, ALICE, acc);
    assert_eq!(t.nft_owner_of(acc), alice);
    for delegate in [&dave, &eve] {
        let got = out(t.ctrl_client().try_borrow(
            delegate,
            &acc,
            &vec![&t.env, (eth.clone(), raw(0.1))],
            &Some(delegate.clone()),
        ));
        assert_eq!(got, Out::Contract(errors::NOT_AUTHORIZED));
    }
    assert_eq!(t.borrow_balance_raw_for(acc, "ETH"), 0, "no borrow landed");
    assert_eq!(
        stored_grant(&t, acc).map(|g| g.granted_by),
        Some(bob.clone()),
        "the stale grant is inert for Alice until she writes"
    );

    // Alice's own remove purges the stale entry and reports no change.
    t.ctrl_client().remove_delegate(&alice, &acc, &dave);
    assert!(
        stored_grant(&t, acc).is_none(),
        "the stale grant is deleted"
    );
    let got = out(t.ctrl_client().try_borrow(
        &eve,
        &acc,
        &vec![&t.env, (eth.clone(), raw(0.1))],
        &Some(eve.clone()),
    ));
    assert_eq!(got, Out::Contract(errors::NOT_AUTHORIZED), "Eve stays out");
}

/// INV-AUTH-02. Grants need an active manager, a dormant grant is refused, and the 17th
/// delegate hits the cap. Re-adding a listed delegate at the cap is a no-op.
#[test]
fn rv_add_delegate_inactive_manager_rejected_cap_at_sixteen() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(ALICE, "USDC", 10_000.0);
    let acc = t.account_id(ALICE);
    let alice = t.get_or_create_user(ALICE);
    let stranger = Address::generate(&t.env);
    let got = out(t.ctrl_client().try_add_delegate(&alice, &acc, &stranger));
    assert_eq!(
        got,
        Out::Contract(errors::NOT_AUTHORIZED),
        "not an active manager"
    );
    assert!(
        stored_grant(&t, acc).is_none(),
        "a refused grant leaves no entry"
    );

    let managers: std::vec::Vec<Address> = (0..17).map(|_| Address::generate(&t.env)).collect();
    for manager in &managers {
        t.ctrl_client().set_position_manager(manager, &true);
    }
    for manager in &managers[..16] {
        t.ctrl_client().add_delegate(&alice, &acc, manager);
    }
    assert_eq!(stored_grant(&t, acc).map(|g| g.delegates.len()), Some(16));
    let got = out(t
        .ctrl_client()
        .try_add_delegate(&alice, &acc, &managers[16]));
    assert_eq!(
        got,
        Out::Contract(REGISTRY_CAP_REACHED),
        "the 17th delegate"
    );

    // A listed delegate is a no-op even at the cap.
    t.ctrl_client().add_delegate(&alice, &acc, &managers[0]);
    assert_eq!(stored_grant(&t, acc).map(|g| g.delegates.len()), Some(16));

    // Removing one frees a slot for the 17th.
    t.ctrl_client().remove_delegate(&alice, &acc, &managers[0]);
    t.ctrl_client().add_delegate(&alice, &acc, &managers[16]);
    let grant = stored_grant(&t, acc).expect("grant");
    assert_eq!(grant.delegates.len(), 16);
    assert!(grant.delegates.contains(&managers[16]));
    assert!(!grant.delegates.contains(&managers[0]));
}

// --- H3: third-party supply --------------------------------------------------------

/// INV-AUTH-03. A stranger's supply is a top-up of a held asset only. It must match the
/// account's spoke, and it obeys the listing's paused and frozen flags.
#[test]
fn rv_third_party_supply_new_slot_spoke_and_halts_rejected() {
    let mut t = LendingTest::new()
        .three_asset_usdc_eth_wbtc()
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .build();
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 1.0);
    let acc = t.account_id(ALICE);
    let carol = t.get_or_create_user(CAROL);
    let alice = t.get_or_create_user(ALICE);
    for asset in ASSETS {
        mint(&t, &carol, asset, raw(1_000.0));
    }
    let usdc = hub_asset(t.resolve_asset("USDC"));
    let wbtc = hub_asset(t.resolve_asset("WBTC"));
    let people = [carol.clone(), alice.clone()];
    let accounts = [acc];

    let supply_before = t.supply_balance_raw_for(acc, "USDC");
    let id = t.ctrl_client().supply(
        &carol,
        &acc,
        &HARNESS_SPOKE,
        &vec![&t.env, (usdc.clone(), raw(100.0))],
    );
    assert_eq!(id, acc);
    assert_eq!(
        t.supply_balance_raw_for(acc, "USDC") - supply_before,
        raw(100.0)
    );
    let before = world(&t, &people, &accounts);

    let got = out(t.ctrl_client().try_supply(
        &carol,
        &acc,
        &HARNESS_SPOKE,
        &vec![&t.env, (wbtc.clone(), raw(0.1))],
    ));
    rejected(
        "new WBTC slot",
        got,
        errors::NOT_AUTHORIZED,
        &t,
        &people,
        &accounts,
        &before,
    );

    // Spoke 2 lists USDC, but Alice's account is bound to spoke 1 (INV-AUTH-06).
    let got =
        out(t
            .ctrl_client()
            .try_supply(&carol, &acc, &2, &vec![&t.env, (usdc.clone(), raw(1.0))]));
    rejected(
        "top-up under spoke 2",
        got,
        errors::SPOKE_MISMATCH,
        &t,
        &people,
        &accounts,
        &before,
    );

    t.set_spoke_asset_flags("USDC", true, false, false);
    let got = out(t.ctrl_client().try_supply(
        &carol,
        &acc,
        &HARNESS_SPOKE,
        &vec![&t.env, (usdc.clone(), raw(1.0))],
    ));
    rejected(
        "top-up while paused",
        got,
        errors::SPOKE_ASSET_PAUSED,
        &t,
        &people,
        &accounts,
        &before,
    );

    t.relax_spoke_asset_flags("USDC", false, true, false);
    let got = out(t.ctrl_client().try_supply(
        &carol,
        &acc,
        &HARNESS_SPOKE,
        &vec![&t.env, (usdc.clone(), raw(1.0))],
    ));
    rejected(
        "top-up while frozen",
        got,
        errors::SPOKE_ASSET_FROZEN,
        &t,
        &people,
        &accounts,
        &before,
    );

    t.relax_spoke_asset_flags("USDC", false, false, false);
    let id = t.ctrl_client().supply(
        &carol,
        &acc,
        &HARNESS_SPOKE,
        &vec![&t.env, (usdc.clone(), raw(1.0))],
    );
    assert_eq!(
        id, acc,
        "the top-up succeeds once the listing is open again"
    );
}

// --- H4: credit receiver through a delegation ------------------------------------------

/// INV-LIQ-01. A liquidator who is an active delegate of Bob's account may credit seized
/// shares into it. Revoking the grant or deactivating the liquidator as manager refuses the
/// credit with NotAuthorized, which the receiver check raises before any health check.
#[test]
fn rv_credit_receiver_delegate_needs_active_manager() {
    let mut t = LendingTest::new().three_asset_usdc_eth_wbtc().build();
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    let alice_acc = t.account_id(ALICE);
    t.supply(BOB, "USDC", 1_000.0);
    let bob_acc = t.account_id(BOB);
    let bob = t.get_or_create_user(BOB);
    let liquidator = t.get_or_create_user(LIQUIDATOR);
    t.ctrl_client().set_position_manager(&liquidator, &true);
    t.ctrl_client().add_delegate(&bob, &bob_acc, &liquidator);
    mint(&t, &liquidator, "ETH", raw(5.0));
    let eth = hub_asset(t.resolve_asset("ETH"));
    t.set_price("USDC", usd_cents(50));
    assert!(t.can_be_liquidated_by_id(alice_acc));

    // Positive: the delegate credits into Bob's account. Bob's scaled supply rises by the
    // estimate's seized shares net of the fee, and the liquidator takes no USDC.
    let payments = vec![&t.env, (eth.clone(), raw(1.0))];
    let estimate = t.ctrl_client().get_liquidation_estimate(
        &alice_acc,
        &payments,
        &SeizeMode::Credit(bob_acc),
    );
    let shares = estimate.seized_collaterals.get(0).unwrap().amount
        - estimate.protocol_fees.get(0).unwrap().amount;
    let bob_scaled = scaled_supply(&t, bob_acc, "USDC");
    let receiver = t.liquidate_with_mode(LIQUIDATOR, ALICE, "ETH", 1.0, SeizeMode::Credit(bob_acc));
    assert_eq!(receiver, bob_acc, "the credit lands in Bob's account");
    assert_eq!(scaled_supply(&t, bob_acc, "USDC") - bob_scaled, shares);
    assert_eq!(t.nft_owner_of(bob_acc), bob, "Bob still owns the receiver");
    assert_eq!(
        token_bal(&t, &liquidator, "USDC"),
        0,
        "credit mode pays no tokens"
    );

    let people = [liquidator.clone(), bob.clone()];
    let accounts = [alice_acc, bob_acc];
    let credit = |t: &LendingTest| {
        out(t.ctrl_client().try_liquidate(
            &liquidator,
            &alice_acc,
            &vec![&t.env, (eth.clone(), raw(0.1))],
            &SeizeMode::Credit(bob_acc),
        ))
    };

    t.ctrl_client().remove_delegate(&bob, &bob_acc, &liquidator);
    let before = world(&t, &people, &accounts);
    let got = credit(&t);
    rejected(
        "credit after revoke",
        got,
        errors::NOT_AUTHORIZED,
        &t,
        &people,
        &accounts,
        &before,
    );

    t.ctrl_client().add_delegate(&bob, &bob_acc, &liquidator);
    t.ctrl_client().set_position_manager(&liquidator, &false);
    let before = world(&t, &people, &accounts);
    let got = credit(&t);
    rejected(
        "credit with inactive manager",
        got,
        errors::NOT_AUTHORIZED,
        &t,
        &people,
        &accounts,
        &before,
    );
}

// --- H5: pool owner gate ------------------------------------------------------------

/// Pool inputs for the probes. The arguments are harmless: the owner check is the first
/// statement of every mutator, so an unsigned call stops there whatever the arguments.
struct PoolCtx {
    env: Env,
    pool: Address,
    controller: Address,
    carol: Address,
    usdc: HubAssetKey,
    wasm: BytesN<32>,
    params: MarketParamsRaw,
    model: InterestRateModel,
}

fn pool_ctx(t: &mut LendingTest) -> PoolCtx {
    let usdc = t.resolve_asset("USDC");
    PoolCtx {
        env: t.env.clone(),
        pool: t.resolve_market("USDC").pool.clone(),
        controller: t.controller.clone(),
        carol: t.get_or_create_user(CAROL),
        usdc: hub_asset(usdc.clone()),
        // The pool's own code: an accepted `upgrade` probe re-installs identical code.
        wasm: upload_wasm(&t.env, "pool.wasm"),
        params: MarketParamsRaw {
            max_borrow_rate: 0,
            base_borrow_rate: 0,
            slope1: 0,
            slope2: 0,
            slope3: 0,
            mid_utilization: 0,
            optimal_utilization: 0,
            max_utilization: RAY * 95 / 100,
            reserve_factor: 0,
            is_flashloanable: false,
            flashloan_fee: 0,
            asset_id: usdc,
            asset_decimals: 7,
        },
        model: InterestRateModel {
            max_borrow_rate: 0,
            base_borrow_rate: 0,
            slope1: 0,
            slope2: 0,
            slope3: 0,
            mid_utilization: 0,
            optimal_utilization: 0,
            max_utilization: RAY * 95 / 100,
            reserve_factor: 0,
            is_flashloanable: false,
            flashloan_fee: 0,
        },
    }
}

fn dummy_action(c: &PoolCtx) -> PoolAction {
    PoolAction {
        position: ScaledPositionRaw { scaled_amount: 0 },
        amount: 0,
        hub_asset: c.usdc.clone(),
    }
}

fn dummy_settle(c: &PoolCtx) -> PoolNetSettleEntry {
    PoolNetSettleEntry {
        hub_asset: c.usdc.clone(),
        amount: 0,
        supply_position: ScaledPositionRaw { scaled_amount: 0 },
        debt_position: ScaledPositionRaw { scaled_amount: 0 },
    }
}

/// Calls `POOL_FNS[i]` with the probe arguments under the current auth regime.
fn pool_probe(i: usize, c: &PoolCtx) -> Out {
    let e = &c.env;
    let pool = LiquidityPoolClient::new(e, &c.pool);
    let carol = &c.carol;
    let usdc = &c.usdc;
    match i {
        0 => out(pool.try_create_market(&HARNESS_HUB, &c.params)),
        1 => out(pool.try_update_params(usdc, &c.model)),
        2 => out(pool.try_upgrade(&c.wasm)),
        3 => out(pool.try_supply(&Vec::<PoolSupplyEntry>::new(e))),
        4 => out(pool.try_borrow(carol, &Vec::<PoolBorrowEntry>::new(e))),
        5 => out(pool.try_withdraw(carol, &false, &Vec::<PoolWithdrawEntry>::new(e))),
        6 => out(pool.try_repay(carol, &Vec::<PoolAction>::new(e))),
        7 => out(pool.try_update_indexes(&Vec::<HubAssetKey>::new(e))),
        8 => out(pool.try_recapitalize(usdc, carol, &0)),
        9 => out(pool.try_flash_loan(usdc, carol, carol, &0, &Bytes::new(e))),
        10 => out(pool.try_create_strategy(carol, &dummy_action(c), &false)),
        11 => out(pool.try_seize_positions(&Vec::<PoolSeizeEntry>::new(e))),
        12 => out(pool.try_net_settle(&dummy_settle(c))),
        _ => out(pool.try_claim_revenue(usdc)),
    }
}

/// The arguments `POOL_FNS[i]` is called with, in the order the host encodes them, for
/// `only_signer`. Each entry must match the args `pool_probe` passes.
fn pool_mock_args(i: usize, c: &PoolCtx) -> Vec<Val> {
    let e = &c.env;
    let carol: Val = c.carol.clone().into_val(e);
    let usdc: Val = c.usdc.clone().into_val(e);
    match i {
        0 => vec![e, HARNESS_HUB.into_val(e), c.params.clone().into_val(e)],
        1 => vec![e, usdc, c.model.clone().into_val(e)],
        2 => vec![e, c.wasm.clone().into_val(e)],
        3 => vec![e, Vec::<PoolSupplyEntry>::new(e).into_val(e)],
        4 => vec![e, carol, Vec::<PoolBorrowEntry>::new(e).into_val(e)],
        5 => vec![
            e,
            carol,
            false.into_val(e),
            Vec::<PoolWithdrawEntry>::new(e).into_val(e),
        ],
        6 => vec![e, carol, Vec::<PoolAction>::new(e).into_val(e)],
        7 => vec![e, Vec::<HubAssetKey>::new(e).into_val(e)],
        8 => vec![e, usdc, carol, 0i128.into_val(e)],
        9 => vec![
            e,
            usdc,
            carol,
            carol,
            0i128.into_val(e),
            Bytes::new(e).into_val(e),
        ],
        10 => vec![e, carol, dummy_action(c).into_val(e), false.into_val(e)],
        11 => vec![e, Vec::<PoolSeizeEntry>::new(e).into_val(e)],
        12 => vec![e, dummy_settle(c).into_val(e)],
        _ => vec![e, usdc],
    }
}

/// INV-AUTH-01. Each pool mutator is refused with no signature, and refused when only a
/// stranger signs the exact invocation. The pool's state and the token balances stay put.
#[test]
fn rv_pool_mutators_unsigned_or_stranger_rejected() {
    let mut t = LendingTest::new().three_asset_usdc_eth_wbtc().build();
    t.supply(ALICE, "USDC", 1_000.0);
    let c = pool_ctx(&mut t);
    let env = c.env.clone();
    let people = [c.carol.clone()];
    let before = world(&t, &people, &[]);

    for (i, name) in POOL_FNS.iter().enumerate() {
        no_signers(&env);
        let got = pool_probe(i, &c);
        assert!(
            matches!(got, Out::Host(_)),
            "{name} with no signature must fail at the owner gate, got {got:?}"
        );
        only_signer(&env, &c.carol, &c.pool, name, pool_mock_args(i, &c));
        let got = pool_probe(i, &c);
        assert!(
            matches!(got, Out::Host(_)),
            "{name} signed by Carol must fail at the owner gate, got {got:?}"
        );
    }
    restore(&env);
    assert_eq!(
        world(&t, &people, &[]),
        before,
        "refused pool calls changed pool state or balances"
    );
}

/// INV-AUTH-01. The pool's owner is the controller. The recorded signer of an owner-gated
/// call is the controller, and the controller's own signature passes the gate. Views need
/// no signature.
#[test]
fn rv_pool_owner_is_controller_and_views_stay_open() {
    let mut t = LendingTest::new().three_asset_usdc_eth_wbtc().build();
    t.supply(ALICE, "USDC", 1_000.0);
    let c = pool_ctx(&mut t);
    let env = c.env.clone();
    let pool = LiquidityPoolClient::new(&env, &c.pool);

    no_signers(&env);
    assert_eq!(out(pool.try_get_reserves(&c.usdc)), Out::Ok);
    assert_eq!(out(pool.try_get_revenue(&c.usdc)), Out::Ok);
    assert_eq!(out(pool.try_get_sync_data(&c.usdc)), Out::Ok);
    assert_eq!(out(pool.try_get_deposit_rate(&c.usdc)), Out::Ok);
    assert_eq!(
        out(pool.try_get_bulk_indexes(&vec![&env, c.usdc.clone()])),
        Out::Ok
    );

    // The recorded signer of the owner check is the controller.
    restore(&env);
    assert_eq!(out(pool.try_update_indexes(&Vec::new(&env))), Out::Ok);
    let (signer, invocation) = env
        .auths()
        .into_iter()
        .next()
        .expect("the owner must be required");
    assert_eq!(signer, c.controller, "the pool's owner is the controller");
    match invocation.function {
        AuthorizedFunction::Contract((contract, name, _)) => {
            assert_eq!(contract, c.pool);
            assert_eq!(name, Symbol::new(&env, "update_indexes"));
        }
        other => panic!("unexpected authorized function {other:?}"),
    }

    // The controller's exact invocation passes the gate for every mutator. The probe then
    // either succeeds or fails with a contract error from the body. A body trap would be a
    // host error, so `Host` here would mean the gate itself refused the controller.
    for (i, name) in POOL_FNS.iter().enumerate() {
        only_signer(&env, &c.controller, &c.pool, name, pool_mock_args(i, &c));
        let got = pool_probe(i, &c);
        assert!(
            matches!(got, Out::Ok | Out::Contract(_)),
            "{name} signed by the controller must reach its body, got {got:?}"
        );
    }

    // The owner recorded in recording mode for the mutators that succeed on this input.
    // A failed call drops its record, so the other mutators are covered by the check above.
    restore(&env);
    for i in POOL_RECORDED_FNS {
        let name = POOL_FNS[i];
        assert_eq!(
            pool_probe(i, &c),
            Out::Ok,
            "{name} succeeds on the probe input"
        );
        let (signer, invocation) = env
            .auths()
            .into_iter()
            .next()
            .unwrap_or_else(|| panic!("{name}: no owner signature was recorded"));
        assert_eq!(signer, c.controller, "{name}: the recorded owner");
        match invocation.function {
            AuthorizedFunction::Contract((contract, fn_name, _)) => {
                assert_eq!(contract, c.pool, "{name}: the owner-gated contract");
                assert_eq!(
                    fn_name,
                    Symbol::new(&env, name),
                    "{name}: the gated function"
                );
            }
            other => panic!("{name}: unexpected authorized function {other:?}"),
        }
    }
    restore(&env);
}

// --- H6: admin gates -----------------------------------------------------------------

/// Owner-only entrypoints. The flag is true for the price aggregator, false for the
/// controller. Indexed by `admin_probe` and `admin_args`.
const ADMIN_FNS: [(bool, &str); 11] = [
    (false, "pause"),
    (false, "add_spoke"),
    (false, "set_price_aggregator"),
    (false, "add_asset_to_spoke"),
    (false, "upgrade_pool"),
    (false, "force_socialize_bad_debt"),
    (false, "set_accumulator"),
    (true, "set_oracle"),
    (true, "set_sanity_band"),
    (true, "set_tolerance"),
    (true, "upgrade"),
];

struct AdminCtx {
    env: Env,
    controller: Address,
    price_agg: Address,
    treasury: Address,
    key: PriceKey,
    oracle: AssetOracle,
    tolerance: OracleTolerance,
    spoke: SpokeAssetArgs,
    pool_wasm: BytesN<32>,
    pa_wasm: BytesN<32>,
}

fn admin_probe(i: usize, a: &AdminCtx) -> Out {
    let e = &a.env;
    let ctrl = controller::ControllerClient::new(e, &a.controller);
    let pa = price_aggregator::PriceAggregatorClient::new(e, &a.price_agg);
    match i {
        0 => out(ctrl.try_pause()),
        1 => out(ctrl.try_add_spoke()),
        2 => out(ctrl.try_set_price_aggregator(&a.price_agg)),
        3 => out(ctrl.try_add_asset_to_spoke(&a.spoke)),
        4 => out(ctrl.try_upgrade_pool(&a.pool_wasm)),
        5 => out(ctrl.try_force_socialize_bad_debt(&1u64)),
        6 => out(ctrl.try_set_accumulator(&a.treasury)),
        7 => out(pa.try_set_oracle(&a.key, &a.oracle)),
        8 => out(pa.try_set_sanity_band(&a.key, &1, &2)),
        9 => out(pa.try_set_tolerance(&a.key, &a.tolerance)),
        _ => out(pa.try_upgrade(&a.pa_wasm)),
    }
}

fn admin_args(i: usize, a: &AdminCtx) -> Vec<Val> {
    let e = &a.env;
    match i {
        0 | 1 => Vec::new(e),
        2 => vec![e, a.price_agg.clone().into_val(e)],
        3 => vec![e, a.spoke.clone().into_val(e)],
        4 => vec![e, a.pool_wasm.clone().into_val(e)],
        5 => vec![e, 1u64.into_val(e)],
        6 => vec![e, a.treasury.clone().into_val(e)],
        7 => vec![e, a.key.clone().into_val(e), a.oracle.clone().into_val(e)],
        8 => vec![
            e,
            a.key.clone().into_val(e),
            1i128.into_val(e),
            2i128.into_val(e),
        ],
        9 => vec![
            e,
            a.key.clone().into_val(e),
            a.tolerance.clone().into_val(e),
        ],
        _ => vec![e, a.pa_wasm.clone().into_val(e)],
    }
}

/// Governance owns the controller and the price aggregator. With no signature each
/// owner-only entrypoint fails at the gate. Carol's signature for the exact invocation fails
/// at the gate. Governance's exact invocation reaches the body, which returns Ok or a
/// contract error.
#[test]
fn rv_admin_entrypoints_owner_gate_governance() {
    let mut t = LendingTest::new().three_asset_usdc_eth_wbtc().build();
    let env = t.env.clone();
    let gov = t.governance.clone();
    let carol = t.get_or_create_user(CAROL);
    let usdc = t.resolve_asset("USDC");
    let ctx = AdminCtx {
        env: env.clone(),
        controller: t.controller.clone(),
        price_agg: t.price_aggregator.clone(),
        treasury: Address::generate(&env),
        key: PriceKey::Token(usdc.clone()),
        oracle: t.market_oracle_config(&usdc),
        tolerance: OracleTolerance {
            upper_ratio_bps: 500,
            lower_ratio_bps: 500,
        },
        spoke: SpokeAssetArgs {
            hub_id: HARNESS_HUB,
            asset: usdc.clone(),
            spoke_id: HARNESS_SPOKE,
            can_collateral: true,
            can_borrow: true,
            paused: false,
            frozen: false,
            no_seize: false,
            ltv: 7_500,
            threshold: 8_000,
            bonus: 500,
            liquidation_fees: 0,
            supply_cap: 0,
            borrow_cap: 0,
        },
        pool_wasm: upload_wasm(&env, "pool.wasm"),
        pa_wasm: upload_wasm(&env, "price_aggregator.wasm"),
    };

    // Identity: governance is the controller's recorded signer and the oracle's owner.
    t.ctrl_client().pause();
    assert_eq!(
        env.auths()[0].0,
        gov,
        "the controller's owner is governance"
    );
    t.ctrl_client().unpause();
    assert_eq!(
        t.price_agg_client().get_owner(),
        Some(gov.clone()),
        "the price aggregator's owner is governance"
    );

    let contract_of = |is_pa: bool| {
        if is_pa {
            ctx.price_agg.clone()
        } else {
            ctx.controller.clone()
        }
    };

    no_signers(&env);
    for (i, (_, name)) in ADMIN_FNS.iter().enumerate() {
        let got = admin_probe(i, &ctx);
        assert!(
            matches!(got, Out::Host(_)),
            "{name} without a signature must fail at the gate, got {got:?}"
        );
    }

    for (i, (is_pa, name)) in ADMIN_FNS.iter().enumerate() {
        only_signer(
            &env,
            &carol,
            &contract_of(*is_pa),
            name,
            admin_args(i, &ctx),
        );
        let got = admin_probe(i, &ctx);
        assert!(
            matches!(got, Out::Host(_)),
            "{name} signed by Carol must fail at the gate, got {got:?}"
        );
    }

    for (i, (is_pa, name)) in ADMIN_FNS.iter().enumerate() {
        only_signer(&env, &gov, &contract_of(*is_pa), name, admin_args(i, &ctx));
        let got = admin_probe(i, &ctx);
        assert!(
            matches!(got, Out::Ok | Out::Contract(_)),
            "{name} signed by governance must reach its body, got {got:?}"
        );
    }
    restore(&env);
}

// --- H7: two-step ownership -----------------------------------------------------------

/// INV-AUTH-01 (controller ownership). Until accept, governance still administers and Carol
/// cannot. Accept needs Carol's signature. After accept the roles flip.
#[test]
fn rv_two_step_old_owner_until_accept_then_flip() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    let env = t.env.clone();
    let ctrl_addr = t.controller.clone();
    let gov = t.governance.clone();
    let carol = t.get_or_create_user(CAROL);
    let live = env.ledger().sequence() + 1_000;
    t.ctrl_client().transfer_ownership(&carol, &live);

    only_signer(&env, &gov, &ctrl_addr, "pause", Vec::new(&env));
    assert_eq!(
        out(t.ctrl_client().try_pause()),
        Out::Ok,
        "old owner still administers"
    );
    only_signer(&env, &gov, &ctrl_addr, "unpause", Vec::new(&env));
    assert_eq!(out(t.ctrl_client().try_unpause()), Out::Ok);

    only_signer(&env, &carol, &ctrl_addr, "pause", Vec::new(&env));
    assert!(
        matches!(out(t.ctrl_client().try_pause()), Out::Host(_)),
        "the pending owner cannot administer before accepting"
    );
    only_signer(&env, &gov, &ctrl_addr, "accept_ownership", Vec::new(&env));
    assert!(
        matches!(out(t.ctrl_client().try_accept_ownership()), Out::Host(_)),
        "only the pending owner can accept"
    );
    only_signer(&env, &carol, &ctrl_addr, "accept_ownership", Vec::new(&env));
    assert_eq!(out(t.ctrl_client().try_accept_ownership()), Out::Ok);

    only_signer(&env, &gov, &ctrl_addr, "pause", Vec::new(&env));
    assert!(
        matches!(out(t.ctrl_client().try_pause()), Out::Host(_)),
        "the old owner is out after accept"
    );
    only_signer(&env, &carol, &ctrl_addr, "pause", Vec::new(&env));
    assert_eq!(
        out(t.ctrl_client().try_pause()),
        Out::Ok,
        "the new owner administers"
    );
    restore(&env);

    t.ctrl_client().unpause();
    assert_eq!(env.auths()[0].0, carol, "the recorded owner is Carol");
}

/// INV-AUTH-01 (controller ownership). A pending grant is accepted through its live_until
/// ledger and refused after it. The refusal code depends on the temporary-storage entry. The
/// harness sets the minimum temporary TTL to 10 ledgers. A 5-ledger grant's entry therefore
/// outlives live_until, and accept reports TransferExpired. A 20-ledger grant's entry lapses
/// at live_until, and accept reports NoPendingTransfer.
#[test]
fn rv_two_step_grant_refused_after_live_until() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    let env = t.env.clone();
    let ctrl_addr = t.controller.clone();
    let carol = t.get_or_create_user(CAROL);
    let bob = t.get_or_create_user(BOB);

    // Accepting on the last live ledger still works.
    let start = env.ledger().sequence();
    t.ctrl_client().transfer_ownership(&carol, &(start + 5));
    env.ledger().set_sequence_number(start + 5);
    only_signer(&env, &carol, &ctrl_addr, "accept_ownership", Vec::new(&env));
    assert_eq!(out(t.ctrl_client().try_accept_ownership()), Out::Ok);
    restore(&env);

    // Carol grants Bob for five ledgers and accepts one ledger late.
    let now = env.ledger().sequence();
    t.ctrl_client().transfer_ownership(&bob, &(now + 5));
    env.ledger().set_sequence_number(now + 6);
    only_signer(&env, &bob, &ctrl_addr, "accept_ownership", Vec::new(&env));
    assert_eq!(
        out(t.ctrl_client().try_accept_ownership()),
        Out::Contract(TRANSFER_EXPIRED),
        "one ledger past live_until"
    );
    restore(&env);

    // A 20-ledger grant lapses through the temporary-storage TTL.
    let now = env.ledger().sequence();
    t.ctrl_client().transfer_ownership(&bob, &(now + 20));
    env.ledger().set_sequence_number(now + 21);
    only_signer(&env, &bob, &ctrl_addr, "accept_ownership", Vec::new(&env));
    assert_eq!(
        out(t.ctrl_client().try_accept_ownership()),
        Out::Contract(errors::NO_PENDING_TRANSFER),
        "the entry has lapsed"
    );
    restore(&env);

    t.ctrl_client().pause();
    assert_eq!(
        env.auths()[0].0,
        carol,
        "a refused grant leaves the owner unchanged"
    );
    t.ctrl_client().unpause();
}

// --- H8: NFT approval ---------------------------------------------------------------------

/// Threat model "Account authority". An approved NFT operator can take the whole account:
/// the token, the collateral and the debt, which stays on the account. Alice loses access.
#[test]
fn rv_nft_operator_approval_moves_the_whole_account() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 1.0);
    let acc = t.account_id(ALICE);
    let alice = t.get_or_create_user(ALICE);
    let mallory = t.get_or_create_user("mallory");
    let usdc = hub_asset(t.resolve_asset("USDC"));
    let token_id = u32::try_from(acc).expect("small id");

    let nft = position_nft::PositionNftClient::new(&t.env, &t.position_nft);
    let until = t.env.ledger().sequence() + 1_000;
    nft.approve(&alice, &mallory, &token_id, &until);
    assert_eq!(nft.get_approved(&token_id), Some(mallory.clone()));
    nft.transfer_from(&mallory, &alice, &mallory, &token_id);
    assert_eq!(
        t.nft_owner_of(acc),
        mallory,
        "Mallory now owns the account token"
    );

    // Mallory withdraws 7 000 USDC to her wallet. The withdrawal passes the solvency gate.
    t.ctrl_client().withdraw(
        &mallory,
        &acc,
        &vec![&t.env, (usdc.clone(), raw(7_000.0))],
        &None,
    );
    assert_eq!(
        token_bal(&t, &mallory, "USDC"),
        raw(7_000.0),
        "Alice's tokens reach Mallory"
    );
    assert_eq!(
        token_bal(&t, &alice, "USDC"),
        0,
        "Alice's wallet gets nothing"
    );
    assert_eq!(t.supply_balance_raw_for(acc, "USDC"), raw(3_000.0));
    assert_eq!(
        t.borrow_balance_raw_for(acc, "ETH"),
        raw(1.0),
        "the debt travels with the token"
    );

    let got =
        out(t
            .ctrl_client()
            .try_withdraw(&alice, &acc, &vec![&t.env, (usdc, raw(1.0))], &None));
    assert_eq!(
        got,
        Out::Contract(errors::NOT_AUTHORIZED),
        "Alice has no authority left"
    );
}
