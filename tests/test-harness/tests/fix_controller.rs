//! Controller fixes on PR #196: permanent regression tests. Each test is
//! named after the verification check it ports (`controller_<N>`) or after
//! the item it covers, and asserts the safe outcome:
//!
//! * CR-19 (`controller_1`, `controller_2`, `controller_extra_1`,
//!   `controller_extra_2`): the position batch of every account strategy
//!   names the NFT holder at the end of the call. `strategy_finalize`
//!   re-reads the holder, so a caller-selected router hop that moves the NFT
//!   mid-call (`repay_debt_with_collateral`, `multiply`, `swap_collateral`,
//!   `swap_debt`) is covered as well as the `flash_position` callback.
//! * F2 (`controller_7`, `f2_*`): a Long or Short `flash_position` rejects
//!   the debt token among its declared collaterals with `AssetsAreTheSame`,
//!   as `multiply` does, before any debt is minted. The Multiply-mode loop
//!   stays admitted (INV-STRAT-04). A Long open that refund-lists the debt
//!   token instead (the rewritten testnet E2E case `fp_long_multi_collateral`)
//!   is admitted and refunds it to the caller.
//! * F7 (`controller_8a`, `f7_*`): a non-owner of a live account gets
//!   `NotAuthorized` from `add_delegate`, `remove_delegate` and
//!   `renew_account`; a missing account still gets `AccountNotInMarket`.
//! * Unowed liquidation leg (`controller_8b`, control `controller_8c`): a
//!   payment leg for a debt the account does not owe fails with
//!   `DebtPositionNotFound` before the leg is priced, in `liquidate` and in
//!   `get_liquidation_estimate`; the earlier gates keep their order.
//! * F6 (`f6_*`, documented, no code change): the whole-unit collateral floor
//!   counts base units, the token's smallest indivisible amount.
//! * CR-11 residual (`controller_6*_residual_*`, documented in INV-RISK-03):
//!   one refresh that tightens a lower-key leg and loosens a higher-key leg
//!   gates the tightening against the sibling's stored tuple; the next
//!   refresh applies it. These pin the accepted residual, not a fix.

use common::types::SeizeMode;
use controller::types::PositionMode;
use soroban_sdk::testutils::{Address as _, ContractEvents, Events};
use soroban_sdk::xdr::{ContractEventBody, ScVal, ToXdr};
use soroban_sdk::{
    contract, contractclient, contractimpl, contracttype, token, vec, Address, Bytes, Env,
    TryFromVal, Vec,
};
use test_harness::presets::{AssetConfigPreset, MarketPreset, DEFAULT_ASSET_CONFIG};
use test_harness::wasm::release_wasm;
use test_harness::{
    apply_flash_fee, assert_contract_error, errors, eth_preset, hub_asset, map_try_ok_unit,
    map_try_ok_value, usd, usd_cents, usdc_preset, FlashPositionMode, FlashPositionRequest,
    HubAssetKey, LendingTest, ALICE, BOB, CAROL, DAVE, DEFAULT_MARKET_PARAMS, HARNESS_SPOKE,
    LIQUIDATOR,
};

const USDC: i128 = 10_000_000;
const ETH: i128 = 10_000_000;
const GATE: i128 = 1_050_000_000_000_000_000;

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// `"ok"`, or the contract error code, or the raw error.
fn code<T: std::fmt::Debug>(r: Result<T, soroban_sdk::Error>) -> std::string::String {
    match r {
        Ok(_) => "ok".into(),
        Err(e) if e.is_type(soroban_sdk::xdr::ScErrorType::Contract) => e.get_code().to_string(),
        Err(e) => format!("{e:?}"),
    }
}

/// Stored liquidation threshold of the leg keyed `key`.
fn lt_of(t: &LendingTest, account: u64, key: &HubAssetKey) -> u32 {
    let (supplies, _) = t.ctrl_client().get_account_positions(&account);
    supplies
        .get(key.clone())
        .expect("the leg is stored")
        .liquidation_threshold
}

/// Raw ETH held by the ETH pool.
fn eth_pool_cash(t: &LendingTest) -> i128 {
    let pool = t.markets.get("ETH").expect("ETH market").pool.clone();
    token::Client::new(&t.env, &t.resolve_asset("ETH")).balance(&pool)
}

// ---------------------------------------------------------------------------
// CR-19: a caller-chosen router hop that moves the account NFT
// ---------------------------------------------------------------------------

/// Owner named by every `["position", "batch_update"]` event for
/// `account_id` in the last invocation. Event data is
/// `[account_id, (owner, spoke_id, mode), deposits, borrows]`.
fn batch_event_owners(t: &LendingTest, account_id: u64) -> std::vec::Vec<Address> {
    let events: ContractEvents = t.env.events().all();
    events
        .events()
        .iter()
        .filter_map(|event| {
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
            Some(Address::try_from_val(&t.env, owner).expect("owner converts"))
        })
        .collect()
}

/// Byte-for-byte the production router's `StrategyPayload` (see
/// `rv_real_router`).
#[contracttype]
#[derive(Clone)]
pub struct StrategyPayload {
    pub amounts: Vec<i128>,
    pub assets: Vec<Address>,
    pub ops: Bytes,
}

#[contracttype]
#[derive(Clone)]
pub enum MovePoolKey {
    Plan,
}

#[contracttype]
#[derive(Clone)]
pub struct MovePlan {
    pub token_a: Address,
    pub token_b: Address,
    pub num: i128,
    pub den: i128,
    pub nft: Address,
    pub from: Address,
    pub to: Address,
    pub token_id: u32,
}

#[contractclient(name = "PositionNftMoveClient")]
pub trait PositionNftMove {
    fn transfer(env: Env, from: Address, to: Address, token_id: u32);
}

/// Aquarius-shaped hop pool chosen by the caller in the route. It pays a fixed
/// rate and, inside the hop, moves the account NFT with the owner's authority
/// (an owner who signs that sub-invocation, as in GH-13).
#[contract]
pub struct NftMovingPool;

#[contractimpl]
impl NftMovingPool {
    pub fn init(env: Env, plan: MovePlan) {
        env.storage().instance().set(&MovePoolKey::Plan, &plan);
    }

    pub fn get_tokens(env: Env) -> Vec<Address> {
        let plan: MovePlan = env.storage().instance().get(&MovePoolKey::Plan).unwrap();
        vec![&env, plan.token_a, plan.token_b]
    }

    pub fn swap(
        env: Env,
        user: Address,
        in_idx: u32,
        out_idx: u32,
        in_amount: u128,
        _out_min: u128,
    ) -> u128 {
        let plan: MovePlan = env.storage().instance().get(&MovePoolKey::Plan).unwrap();
        let tokens = Self::get_tokens(env.clone());
        let token_in = tokens.get(in_idx).unwrap();
        let token_out = tokens.get(out_idx).unwrap();
        let pool = env.current_contract_address();
        let amount_in = in_amount as i128;
        token::Client::new(&env, &token_in).transfer(&user, &pool, &amount_in);
        let out = amount_in * plan.num / plan.den;
        token::Client::new(&env, &token_out).transfer(&pool, &user, &out);
        PositionNftMoveClient::new(&env, &plan.nft).transfer(&plan.from, &plan.to, &plan.token_id);
        out as u128
    }
}

fn one_hop_payload(
    env: &Env,
    token_in: &Address,
    token_out: &Address,
    pool: &Address,
    min_out: i128,
) -> Bytes {
    let mut ops = Bytes::new(env);
    ops.push_back(1); // wire version
    ops.push_back(0); // token_in  -> assets[0]
    ops.push_back(1); // token_out -> assets[1]
    ops.push_back(0); // min_out   -> amounts[0]
    for byte in 0u32.to_be_bytes() {
        ops.push_back(byte); // no referral
    }
    ops.push_back(1); // op_count
    ops.push_back(0); // weight_count
    for byte in [1u8, 0, 2, 0, 1] {
        ops.push_back(byte); // Aquarius swap, whole balance, pool, in, out
    }
    StrategyPayload {
        amounts: vec![env, min_out],
        assets: vec![env, token_in.clone(), token_out.clone(), pool.clone()],
        ops,
    }
    .to_xdr(env)
}

fn router_wasm() -> std::vec::Vec<u8> {
    release_wasm("swap_aggregator.wasm")
}

/// USDC + ETH book with the production router installed. Bob supplies
/// 100 ETH; Carol supplies 10_000 USDC and borrows 1 ETH. A USDC->ETH hop
/// pool at 5/10_000 moves Carol's NFT to Bob inside the swap.
struct RouterNftFixture {
    t: LendingTest,
    carol: Address,
    bob: Address,
    carol_id: u64,
    usdc: Address,
    eth: Address,
    pool: Address,
}

fn router_nft_fixture() -> RouterNftFixture {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();
    t.env.cost_estimate().budget().reset_unlimited();
    t.supply(BOB, "ETH", 100.0);
    t.supply(CAROL, "USDC", 10_000.0);
    t.borrow(CAROL, "ETH", 1.0);
    let carol_id = t.resolve_account_id(CAROL);
    let usdc = t.resolve_asset("USDC");
    let eth = t.resolve_asset("ETH");
    let router_admin = Address::generate(&t.env);
    let router = t.env.register(&router_wasm()[..], (router_admin,));
    t.ctrl_client().set_swap_aggregator(&router);

    let carol = t.get_or_create_user(CAROL);
    let bob = t.get_or_create_user(BOB);
    let pool = t.env.register(NftMovingPool, ());
    NftMovingPoolClient::new(&t.env, &pool).init(&MovePlan {
        token_a: usdc.clone(),
        token_b: eth.clone(),
        num: 5,
        den: 10_000,
        nft: t.position_nft.clone(),
        from: carol.clone(),
        to: bob.clone(),
        token_id: carol_id as u32,
    });
    t.resolve_market("ETH").token_admin.mint(&pool, &(10 * ETH));
    RouterNftFixture {
        t,
        carol,
        bob,
        carol_id,
        usdc,
        eth,
        pool,
    }
}

fn assert_batch_names_holder(t: &LendingTest, id: u64, bob: &Address, carol: &Address) {
    // Read before any other invocation: events are kept for the last call only.
    let owners = batch_event_owners(t, id);
    assert!(
        !owners.is_empty(),
        "the call emitted a batch for the account"
    );
    assert_eq!(&t.nft_owner_of(id), bob, "the hop moved the token");
    for owner in &owners {
        let named = if owner == bob {
            "bob (the holder)"
        } else if owner == carol {
            "carol (the entry owner)"
        } else {
            "another address"
        };
        assert!(
            owner == bob,
            "the batch should name the holder at the end of the call (bob); it names {named}"
        );
    }
}

/// controller-1. `repay_debt_with_collateral` on Carol's account: the router
/// hop moves the NFT to Bob mid-swap.
#[test]
fn controller_1_cr19_repay_debt_with_collateral_through_the_router_names_the_holder() {
    let f = router_nft_fixture();
    let amount = 1_000 * USDC;
    // 1% slack on min_out against withdraw rounding; it does not touch the
    // owner path.
    let min_out = amount * 5 / 10_000 * 99 / 100;
    let payload = one_hop_payload(&f.t.env, &f.usdc, &f.eth, &f.pool, min_out);
    f.t.ctrl_client().repay_debt_with_collateral(
        &f.carol,
        &f.carol_id,
        &hub_asset(f.usdc.clone()),
        &amount,
        &hub_asset(f.eth.clone()),
        &payload,
        &false,
    );
    assert_batch_names_holder(&f.t, f.carol_id, &f.bob, &f.carol);
}

/// controller-extra-1. `swap_collateral` USDC -> ETH on Carol's account
/// through the same hop.
#[test]
fn controller_extra_1_cr19_swap_collateral_through_the_router_names_the_holder() {
    let f = router_nft_fixture();
    let amount = 1_000 * USDC;
    let min_out = amount * 5 / 10_000 * 99 / 100;
    let payload = one_hop_payload(&f.t.env, &f.usdc, &f.eth, &f.pool, min_out);
    f.t.ctrl_client().swap_collateral(
        &f.carol,
        &f.carol_id,
        &hub_asset(f.usdc.clone()),
        &amount,
        &hub_asset(f.eth.clone()),
        &payload,
    );
    assert_batch_names_holder(&f.t, f.carol_id, &f.bob, &f.carol);
}

/// controller-extra-2. `swap_debt` ETH -> USDC on Carol's account: borrow
/// USDC, hop USDC -> ETH (moving the NFT), repay ETH.
#[test]
fn controller_extra_2_cr19_swap_debt_through_the_router_names_the_holder() {
    let f = router_nft_fixture();
    let amount = 1_000 * USDC;
    let min_out = apply_flash_fee(amount) * 5 / 10_000 * 99 / 100;
    let payload = one_hop_payload(&f.t.env, &f.usdc, &f.eth, &f.pool, min_out);
    f.t.ctrl_client().swap_debt(
        &f.carol,
        &f.carol_id,
        &hub_asset(f.eth.clone()),
        &amount,
        &hub_asset(f.usdc.clone()),
        &payload,
    );
    assert_batch_names_holder(&f.t, f.carol_id, &f.bob, &f.carol);
}

/// controller-2. `multiply` through the production router: the caller's
/// route names a hop pool that moves the freshly minted account NFT to Bob.
#[test]
fn controller_2_cr19_multiply_through_the_real_router_names_the_holder_after_a_mid_hop_transfer() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();
    t.env.cost_estimate().budget().reset_unlimited();
    t.supply(ALICE, "USDC", 100_000.0);
    t.supply(BOB, "ETH", 100.0);
    let usdc = t.resolve_asset("USDC");
    let eth = t.resolve_asset("ETH");
    let router_admin = Address::generate(&t.env);
    let router = t.env.register(&router_wasm()[..], (router_admin,));
    t.ctrl_client().set_swap_aggregator(&router);

    let carol = t.get_or_create_user(CAROL);
    let bob = t.get_or_create_user(BOB);
    let expected_id = 3u64; // Alice 1, Bob 2, Carol's new account 3.
    assert_eq!(t.resolve_account_id(ALICE), 1);
    assert_eq!(t.resolve_account_id(BOB), 2);
    let pool = t.env.register(NftMovingPool, ());
    NftMovingPoolClient::new(&t.env, &pool).init(&MovePlan {
        token_a: usdc.clone(),
        token_b: eth.clone(),
        num: 5,
        den: 10_000,
        nft: t.position_nft.clone(),
        from: carol.clone(),
        to: bob.clone(),
        token_id: expected_id as u32,
    });
    t.resolve_market("ETH")
        .token_admin
        .mint(&pool, &(100 * ETH));
    t.resolve_market("ETH").token_admin.mint(&carol, &ETH);

    let debt = 2_000 * USDC;
    let out = apply_flash_fee(debt) * 5 / 10_000;
    let payload = one_hop_payload(&t.env, &usdc, &eth, &pool, out);
    let id = t.ctrl_client().multiply(
        &carol,
        &0u64,
        &HARNESS_SPOKE,
        &hub_asset(eth.clone()),
        &debt,
        &hub_asset(usdc.clone()),
        &PositionMode::Multiply,
        &payload,
        &Some((hub_asset(eth), ETH)),
        &None,
    );
    assert_eq!(id, expected_id);
    assert_batch_names_holder(&t, id, &bob, &carol);
}

// ---------------------------------------------------------------------------
// F2
// ---------------------------------------------------------------------------

/// The receiver supplies 4,000 USDC and returns the borrowed ETH.
fn supply_and_return_debt(t: &LendingTest) -> Bytes {
    FlashPositionRequest {
        mode: FlashPositionMode::SupplyAndReturnDebt,
        collateral: t.resolve_asset("USDC"),
        collateral_amount: 4_000 * USDC,
        extra_asset: Address::generate(&t.env),
        extra_amount: 0,
        reenter_spoke_id: HARNESS_SPOKE,
        reenter_account_id: 0,
    }
    .to_xdr(&t.env)
}

/// The receiver supplies 4,000 USDC and keeps the borrowed debt.
fn success_with_usdc(t: &LendingTest) -> Bytes {
    FlashPositionRequest {
        mode: FlashPositionMode::Success,
        collateral: t.resolve_asset("USDC"),
        collateral_amount: 4_000 * USDC,
        extra_asset: Address::generate(&t.env),
        extra_amount: 0,
        reenter_spoke_id: HARNESS_SPOKE,
        reenter_account_id: 0,
    }
    .to_xdr(&t.env)
}

/// controller-7. Long and Short reject the debt token among the declared
/// collaterals, as `multiply` does, before any debt is minted; Multiply keeps
/// the loop.
#[test]
fn controller_7_f2_long_and_short_flash_positions_reject_the_debt_token_as_collateral() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(BOB, "ETH", 100.0);
    let receiver = t.deploy_flash_position_receiver();
    let data = supply_and_return_debt(&t);
    let both: Vec<(HubAssetKey, i128)> = vec![
        &t.env,
        (hub_asset(t.resolve_asset("USDC")), 4_000 * USDC),
        (hub_asset(t.resolve_asset("ETH")), 0),
    ];
    let pool_cash_before = eth_pool_cash(&t);

    let mut outcomes = std::vec::Vec::new();
    for mode in [PositionMode::Long, PositionMode::Short] {
        let res = t.try_flash_position(
            ALICE,
            0,
            mode,
            "ETH",
            1.0,
            &receiver,
            &data,
            &both,
            &Vec::new(&t.env),
        );
        let opened = res.as_ref().ok().copied();
        let state = opened.map(|id| {
            (
                t.supply_balance_raw_for(id, "ETH"),
                t.borrow_balance_raw_for(id, "ETH"),
            )
        });
        outcomes.push((format!("{mode:?}"), code(res), state));
    }
    let pool_cash_after = eth_pool_cash(&t);
    let expected_code = errors::ASSETS_ARE_THE_SAME.to_string();
    assert!(
        outcomes.iter().all(|(_, c, _)| *c == expected_code),
        "Long/Short with the debt token declared: {outcomes:?} (expected {expected_code}); \
         ETH pool cash before {pool_cash_before}, after {pool_cash_after}"
    );
    assert!(t.find_account_id(ALICE).is_none(), "nothing was opened");
    assert_eq!(pool_cash_after, pool_cash_before, "nothing was minted");

    let id = t
        .try_flash_position(
            ALICE,
            0,
            PositionMode::Multiply,
            "ETH",
            1.0,
            &receiver,
            &data,
            &both,
            &Vec::new(&t.env),
        )
        .expect("the Multiply-mode loop stays admitted (INV-STRAT-04)");
    assert_eq!(t.supply_balance_raw_for(id, "ETH"), ETH);
    assert_eq!(t.borrow_balance_raw_for(id, "ETH"), ETH);
}

/// Mirror of the testnet E2E case `fp_long_multi_collateral`
/// (`tests/integration/flows/flash_position.sh`), rewritten for F2. The old
/// shape, a Long open that declares the debt token next to the real
/// collateral, is now `AssetsAreTheSame` with no state change. The supported
/// shape declares only the real collateral and refund-lists the debt token:
/// the receiver returns the borrowed debt, the controller refunds it to the
/// caller, and the account holds no debt-token supply. The assertion
/// messages carry the E2E predicate labels.
#[test]
fn f2_long_flash_position_refunds_the_returned_debt_token_to_the_caller() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(BOB, "USDC", 10_000.0);
    let receiver = t.deploy_flash_position_receiver();
    let alice = t.get_or_create_user(ALICE);
    let controller = t.controller_address();
    let usdc = t.resolve_asset("USDC");
    let eth = t.resolve_asset("ETH");
    let usdc_pool = t.markets.get("USDC").expect("USDC market").pool.clone();
    let usdc_token = token::Client::new(&t.env, &usdc);
    let eth_token = token::Client::new(&t.env, &eth);
    let data = FlashPositionRequest {
        mode: FlashPositionMode::SupplyAndReturnDebt,
        collateral: eth.clone(),
        collateral_amount: ETH,
        extra_asset: Address::generate(&t.env),
        extra_amount: 0,
        reenter_spoke_id: HARNESS_SPOKE,
        reenter_account_id: 0,
    }
    .to_xdr(&t.env);
    let debt = 100 * USDC;
    let snapshot = || {
        (
            usdc_token.balance(&usdc_pool),
            usdc_token.balance(&alice),
            usdc_token.balance(&receiver),
            usdc_token.balance(&controller),
            eth_token.balance(&controller),
        )
    };

    // The old E2E shape: the debt token declared next to the real collateral.
    let declared: Vec<(HubAssetKey, i128)> = vec![
        &t.env,
        (hub_asset(eth.clone()), ETH),
        (hub_asset(usdc.clone()), debt),
    ];
    let before = snapshot();
    let res = t.try_flash_position(
        ALICE,
        0,
        PositionMode::Long,
        "USDC",
        100.0,
        &receiver,
        &data,
        &declared,
        &Vec::new(&t.env),
    );
    assert_contract_error(res.map(|_| ()), errors::ASSETS_ARE_THE_SAME);
    assert!(t.find_account_id(ALICE).is_none(), "nothing was opened");
    assert_eq!(snapshot(), before, "the rejection moves no token");

    // The rewritten shape: the real collateral declared, the debt refunded.
    let collaterals: Vec<(HubAssetKey, i128)> = vec![&t.env, (hub_asset(eth.clone()), ETH)];
    let refunds: Vec<Address> = vec![&t.env, usdc.clone()];
    let (pool_before, alice_before, receiver_before, ctrl_usdc_before, ctrl_eth_before) =
        snapshot();
    let id = t
        .try_flash_position(
            ALICE,
            0,
            PositionMode::Long,
            "USDC",
            100.0,
            &receiver,
            &data,
            &collaterals,
            &refunds,
        )
        .expect("a Long open that refunds the returned debt token is admitted");
    let (pool_after, alice_after, receiver_after, ctrl_usdc_after, ctrl_eth_after) = snapshot();
    assert_eq!(pool_after - pool_before, -debt, "fp_long_usdc_pool");
    assert_eq!(alice_after - alice_before, debt, "fp_long_usdc_refund");
    assert_eq!(receiver_after, receiver_before, "fp_long_usdc_receiver");
    assert_eq!(ctrl_usdc_after, ctrl_usdc_before, "fp_long_usdc_protected");
    assert_eq!(ctrl_eth_after, ctrl_eth_before, "fp_long_xlm_protected");
    assert_eq!(
        t.supply_balance_raw_for(id, "ETH"),
        ETH,
        "fp_long_xlm_credit"
    );
    assert_eq!(
        t.supply_balance_raw_for(id, "USDC"),
        0,
        "fp_long_usdc_no_credit"
    );
    assert!(t.borrow_balance_raw_for(id, "USDC") >= debt, "fp_long_debt");
    assert_eq!(
        t.ctrl_client().get_account_attributes(&id).mode,
        PositionMode::Long,
        "fp_long_mode"
    );
}

/// An existing Long account cannot declare the debt token on a second call,
/// cannot relabel itself Multiply to get the loop, and a debt token listed
/// for refund is refunded, never deposited.
#[test]
fn f2_existing_long_account_cannot_bypass_the_rule() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(BOB, "ETH", 100.0);
    let receiver = t.deploy_flash_position_receiver();
    let usdc_only: Vec<(HubAssetKey, i128)> =
        vec![&t.env, (hub_asset(t.resolve_asset("USDC")), 4_000 * USDC)];
    let id = t
        .try_flash_position(
            ALICE,
            0,
            PositionMode::Long,
            "ETH",
            1.0,
            &receiver,
            &success_with_usdc(&t),
            &usdc_only,
            &Vec::new(&t.env),
        )
        .expect("a distinct-token Long opens");

    let with_debt_token: Vec<(HubAssetKey, i128)> = vec![
        &t.env,
        (hub_asset(t.resolve_asset("USDC")), 4_000 * USDC),
        (hub_asset(t.resolve_asset("ETH")), 0),
    ];
    let data = supply_and_return_debt(&t);
    assert_contract_error(
        t.try_flash_position(
            ALICE,
            id,
            PositionMode::Long,
            "ETH",
            1.0,
            &receiver,
            &data,
            &with_debt_token,
            &Vec::new(&t.env),
        )
        .map(|_| ()),
        errors::ASSETS_ARE_THE_SAME,
    );
    assert_contract_error(
        t.try_flash_position(
            ALICE,
            id,
            PositionMode::Multiply,
            "ETH",
            1.0,
            &receiver,
            &data,
            &with_debt_token,
            &Vec::new(&t.env),
        )
        .map(|_| ()),
        errors::ACCOUNT_MODE_MISMATCH,
    );

    // The receiver hands the borrowed ETH back; ETH is a refund asset, not a
    // collateral, so it goes to the caller and the account holds no ETH.
    let alice = t.get_or_create_user(ALICE);
    let eth = t.resolve_asset("ETH");
    let alice_eth_before = token::Client::new(&t.env, &eth).balance(&alice);
    t.try_flash_position(
        ALICE,
        id,
        PositionMode::Long,
        "ETH",
        1.0,
        &receiver,
        &data,
        &usdc_only,
        &vec![&t.env, eth.clone()],
    )
    .expect("a Long top-up refunding its debt token");
    assert_eq!(t.supply_balance_raw_for(id, "ETH"), 0);
    assert_eq!(
        token::Client::new(&t.env, &eth).balance(&alice) - alice_eth_before,
        ETH,
        "the returned debt is refunded to the caller"
    );
    assert_eq!(t.borrow_balance_raw_for(id, "ETH"), 2 * ETH);
}

/// Control (passes before and after the fix): the rule is per call, as in
/// `multiply`. A Long account reaches the same token on both sides with one
/// plain `borrow` or `supply`; the mode carries no risk parameter.
#[test]
fn f2_control_long_account_reaches_the_same_token_by_plain_calls() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(BOB, "ETH", 100.0);
    t.supply(CAROL, "USDC", 100_000.0);
    let receiver = t.deploy_flash_position_receiver();
    let usdc_only: Vec<(HubAssetKey, i128)> =
        vec![&t.env, (hub_asset(t.resolve_asset("USDC")), 4_000 * USDC)];
    let id = t
        .try_flash_position(
            ALICE,
            0,
            PositionMode::Long,
            "ETH",
            1.0,
            &receiver,
            &success_with_usdc(&t),
            &usdc_only,
            &Vec::new(&t.env),
        )
        .expect("a distinct-token Long opens");

    t.try_borrow(ALICE, "USDC", 10.0)
        .expect("a Long account may borrow its collateral token");
    t.try_supply(ALICE, "ETH", 0.1)
        .expect("a Long account may supply its debt token");
    assert_eq!(
        t.ctrl_client().get_account_attributes(&id).mode,
        PositionMode::Long
    );
    assert!(t.supply_balance_raw_for(id, "ETH") > 0);
    assert!(t.borrow_balance_raw_for(id, "ETH") > 0);
    assert!(t.supply_balance_raw_for(id, "USDC") > 0);
    assert!(t.borrow_balance_raw_for(id, "USDC") > 0);
}

// ---------------------------------------------------------------------------
// F7 and the unowed liquidation leg
// ---------------------------------------------------------------------------

/// controller-8a. Owner-only account administration refuses any other caller
/// of an existing account with `NotAuthorized`; a missing account stays
/// `AccountNotInMarket`.
#[test]
fn controller_8a_f7_non_owner_delegate_and_renew_calls_are_not_authorized() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(ALICE, "USDC", 1_000.0);
    let acc = t.account_id(ALICE);
    let alice = t.get_or_create_user(ALICE);
    let carol = t.get_or_create_user(CAROL);
    let dave = t.get_or_create_user(DAVE);
    t.ctrl_client().set_position_manager(&dave, &true);
    let ctrl = t.ctrl_client();
    let mut got = std::vec::Vec::new();

    got.push((
        "stranger add_delegate",
        code(map_try_ok_unit(ctrl.try_add_delegate(&carol, &acc, &dave))),
    ));
    got.push((
        "stranger remove_delegate",
        code(map_try_ok_unit(
            ctrl.try_remove_delegate(&carol, &acc, &dave),
        )),
    ));
    got.push((
        "stranger renew_account",
        code(map_try_ok_unit(ctrl.try_renew_account(&carol, &acc))),
    ));
    ctrl.add_delegate(&alice, &acc, &dave);
    got.push((
        "delegate add_delegate",
        code(map_try_ok_unit(ctrl.try_add_delegate(&dave, &acc, &carol))),
    ));
    got.push((
        "delegate remove_delegate",
        code(map_try_ok_unit(
            ctrl.try_remove_delegate(&dave, &acc, &dave),
        )),
    ));
    got.push((
        "delegate renew_account",
        code(map_try_ok_unit(ctrl.try_renew_account(&dave, &acc))),
    ));
    got.push((
        "missing renew_account",
        code(map_try_ok_unit(ctrl.try_renew_account(&alice, &999_999))),
    ));
    got.push((
        "missing add_delegate",
        code(map_try_ok_unit(
            ctrl.try_add_delegate(&alice, &999_999, &dave),
        )),
    ));

    // Previous owner after an NFT transfer.
    t.nft_transfer(ALICE, BOB, acc);
    t.adopt_account(BOB, acc, HARNESS_SPOKE, PositionMode::Normal);
    let ctrl = t.ctrl_client();
    got.push((
        "previous owner renew_account",
        code(map_try_ok_unit(ctrl.try_renew_account(&alice, &acc))),
    ));
    got.push((
        "previous owner remove_delegate",
        code(map_try_ok_unit(
            ctrl.try_remove_delegate(&alice, &acc, &dave),
        )),
    ));

    let na = errors::NOT_AUTHORIZED.to_string();
    let anim = errors::ACCOUNT_NOT_IN_MARKET.to_string();
    let expected: std::vec::Vec<(&str, std::string::String)> = std::vec![
        ("stranger add_delegate", na.clone()),
        ("stranger remove_delegate", na.clone()),
        ("stranger renew_account", na.clone()),
        ("delegate add_delegate", na.clone()),
        ("delegate remove_delegate", na.clone()),
        ("delegate renew_account", na.clone()),
        ("missing renew_account", anim.clone()),
        ("missing add_delegate", anim),
        ("previous owner renew_account", na.clone()),
        ("previous owner remove_delegate", na),
    ];
    assert_eq!(got, expected);
}

/// F7 changes the code, not the authorization: the previous owner changes
/// nothing, the new owner administers, an owner granting an inactive manager
/// is `NotAuthorized`, and once the account is closed the holder's calls are
/// `AccountNotInMarket` again.
#[test]
fn f7_previous_owner_new_owner_and_closed_account() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(ALICE, "USDC", 1_000.0);
    let acc = t.account_id(ALICE);
    let alice = t.get_or_create_user(ALICE);
    let bob = t.get_or_create_user(BOB);
    let dave = t.get_or_create_user(DAVE);
    let carol = t.get_or_create_user(CAROL);
    let ctrl = t.ctrl_client();
    ctrl.set_position_manager(&dave, &true);
    ctrl.add_delegate(&alice, &acc, &dave);

    t.nft_transfer(ALICE, BOB, acc);
    t.adopt_account(BOB, acc, HARNESS_SPOKE, PositionMode::Normal);
    let ctrl = t.ctrl_client();
    assert_contract_error(
        map_try_ok_unit(ctrl.try_remove_delegate(&alice, &acc, &dave)),
        errors::NOT_AUTHORIZED,
    );
    assert_contract_error(
        map_try_ok_unit(ctrl.try_renew_account(&alice, &acc)),
        errors::NOT_AUTHORIZED,
    );
    assert_contract_error(
        map_try_ok_unit(ctrl.try_add_delegate(&alice, &acc, &dave)),
        errors::NOT_AUTHORIZED,
    );

    ctrl.renew_account(&bob, &acc);
    assert_contract_error(
        map_try_ok_unit(ctrl.try_add_delegate(&bob, &acc, &carol)),
        errors::NOT_AUTHORIZED,
    );
    ctrl.add_delegate(&bob, &acc, &dave);

    t.withdraw_all(BOB, "USDC");
    assert!(!t.account_exists(acc), "the empty account is removed");
    let ctrl = t.ctrl_client();
    assert_contract_error(
        map_try_ok_unit(ctrl.try_renew_account(&bob, &acc)),
        errors::ACCOUNT_NOT_IN_MARKET,
    );
    assert_contract_error(
        map_try_ok_unit(ctrl.try_remove_delegate(&bob, &acc, &dave)),
        errors::ACCOUNT_NOT_IN_MARKET,
    );
}

/// controller-8b. A payment leg naming a listed, priced asset the account
/// neither owes nor holds is refused as `DebtPositionNotFound`, alone or next
/// to a valid leg, by `liquidate` and by the estimate. Before the fix both
/// failed with `OracleNotConfigured`, because the leg was priced from the
/// account's portfolio cache before its position was looked up.
#[test]
fn controller_8b_unowed_liquidation_payment_leg_is_debt_position_not_found() {
    let mut t = LendingTest::new().three_asset_usdc_eth_wbtc().build();
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    t.set_price("USDC", usd_cents(50));
    t.assert_liquidatable(ALICE);
    let acc = t.account_id(ALICE);
    let liquidator = t.get_or_create_user(LIQUIDATOR);
    let wbtc = hub_asset(t.resolve_asset("WBTC"));
    let eth = hub_asset(t.resolve_asset("ETH"));
    let ctrl = t.ctrl_client();

    let only_wbtc: Vec<(HubAssetKey, i128)> = vec![&t.env, (wbtc.clone(), 1_000)];
    let mixed: Vec<(HubAssetKey, i128)> = vec![&t.env, (eth, ETH), (wbtc, 1_000)];
    let mut got = std::vec::Vec::new();
    for (label, payments) in [("only_wbtc", &only_wbtc), ("mixed", &mixed)] {
        let liq = code(
            map_try_ok_value(ctrl.try_liquidate(&liquidator, &acc, payments, &SeizeMode::Transfer))
                .map(|_| ()),
        );
        let estimate = match ctrl.try_get_liquidation_estimate(&acc, payments, &SeizeMode::Transfer)
        {
            Ok(Ok(_)) => "ok".to_string(),
            Ok(Err(conversion)) => panic!("estimate conversion failed: {conversion:?}"),
            Err(Ok(err)) => code::<()>(Err(err)),
            Err(Err(invoke)) => panic!("estimate invoke failed: {invoke:?}"),
        };
        got.push((label, liq, estimate));
    }
    let dpnf = errors::DEBT_POSITION_NOT_FOUND.to_string();
    assert_eq!(
        got,
        std::vec![
            ("only_wbtc", dpnf.clone(), dpnf.clone()),
            ("mixed", dpnf.clone(), dpnf),
        ],
        "(label, liquidate, estimate); OracleNotConfigured is {}",
        errors::ORACLE_NOT_CONFIGURED
    );
    assert_eq!(
        t.borrow_balance_raw_for(acc, "ETH"),
        3 * ETH,
        "nothing was repaid"
    );
}

/// controller-8c, control. The health gate and the pause flag still run
/// before the position check; a leg held as collateral but not owed is
/// `DebtPositionNotFound`.
#[test]
fn controller_8c_earlier_liquidation_gates_keep_their_order() {
    let mut t = LendingTest::new().three_asset_usdc_eth_wbtc().build();
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    let acc = t.account_id(ALICE);
    let liquidator = t.get_or_create_user(LIQUIDATOR);
    let wbtc = hub_asset(t.resolve_asset("WBTC"));
    let usdc = hub_asset(t.resolve_asset("USDC"));
    let only_wbtc: Vec<(HubAssetKey, i128)> = vec![&t.env, (wbtc, 1_000)];
    let held_not_owed: Vec<(HubAssetKey, i128)> = vec![&t.env, (usdc, 1_000)];

    assert_contract_error(
        map_try_ok_value(t.ctrl_client().try_liquidate(
            &liquidator,
            &acc,
            &only_wbtc,
            &SeizeMode::Transfer,
        ))
        .map(|_| ()),
        errors::HEALTH_FACTOR_TOO_HIGH,
    );

    t.set_price("USDC", usd_cents(50));
    t.assert_liquidatable(ALICE);
    assert_contract_error(
        map_try_ok_value(t.ctrl_client().try_liquidate(
            &liquidator,
            &acc,
            &held_not_owed,
            &SeizeMode::Transfer,
        ))
        .map(|_| ()),
        errors::DEBT_POSITION_NOT_FOUND,
    );

    t.set_spoke_asset_paused("WBTC", true);
    assert_contract_error(
        map_try_ok_value(t.ctrl_client().try_liquidate(
            &liquidator,
            &acc,
            &only_wbtc,
            &SeizeMode::Transfer,
        ))
        .map(|_| ()),
        errors::SPOKE_ASSET_PAUSED,
    );
    assert_eq!(t.borrow_balance_raw_for(acc, "ETH"), 3 * ETH);
}

// ---------------------------------------------------------------------------
// F6 (documented)
// ---------------------------------------------------------------------------

/// A 2-decimal collateral-only listing at $10 per whole token.
fn two_decimal_preset() -> MarketPreset {
    MarketPreset {
        name: "TWO",
        decimals: 2,
        price_wad: usd(10),
        initial_liquidity: 0.0,
        config: AssetConfigPreset {
            is_borrowable: false,
            is_flashloanable: false,
            flashloan_fee: 0,
            liquidation_fees: 0,
            ..DEFAULT_ASSET_CONFIG
        },
        params: DEFAULT_MARKET_PARAMS,
    }
}

/// `MIN_WHOLE_UNIT_COLLATERAL` (2) counts base units: 0.02 of a 2-decimal
/// token is two units and clears the floor with debt, 0.01 is one unit and
/// does not. LTV cover and health factor hold throughout, so the floor is the
/// only gate that refuses the last withdrawal.
#[test]
fn f6_whole_unit_collateral_floor_counts_base_units() {
    let mut t = LendingTest::new()
        .standard_two_asset()
        .with_market(two_decimal_preset())
        .with_min_borrow_collateral_disabled()
        .build();
    t.supply(ALICE, "TWO", 0.03);
    t.borrow(ALICE, "USDC", 0.05);
    assert_eq!(t.supply_balance_raw_for(t.account_id(ALICE), "TWO"), 3);

    t.try_withdraw(ALICE, "TWO", 0.01)
        .expect("two base units remain with debt: the floor holds");
    assert_contract_error(
        t.try_withdraw(ALICE, "TWO", 0.01),
        errors::MIN_BORROW_COLLATERAL_NOT_MET,
    );
    assert_eq!(t.supply_balance_raw_for(t.account_id(ALICE), "TWO"), 2);

    t.repay(ALICE, "USDC", 0.05);
    t.try_withdraw(ALICE, "TWO", 0.02)
        .expect("a debt-free account may exit completely");
}

// ---------------------------------------------------------------------------
// CR-11 residual (documented in INV-RISK-03): mixed tighten / loosen
// ---------------------------------------------------------------------------

/// Alice: 10_000 USDC + 5 ETH (10_000 USD each) against 0.2416 WBTC
/// (14_496 USD; HF 1.104). The first-key asset A is relisted LTV 6_500 /
/// LT 7_000 (bonus and fees kept: favours the liquidator); the second-key
/// asset B is relisted LT 8_500 (LTV, bonus and fees kept: does not).
/// A's gate with B at its stored 8_000: (0.70 + 0.80) * 10_000 / 14_496 =
/// 1.035 < 1.05. Final with B loosened: (0.70 + 0.85) * 10_000 / 14_496 =
/// 1.069 >= 1.05.
fn mixed_pending_account() -> (LendingTest, u64, HubAssetKey, HubAssetKey) {
    let mut t = LendingTest::new().three_asset_usdc_eth_wbtc().build();
    t.supply(BOB, "WBTC", 1.0);
    t.supply(ALICE, "USDC", 10_000.0);
    t.supply(ALICE, "ETH", 5.0);
    t.borrow(ALICE, "WBTC", 0.2416);
    let account = t.account_id(ALICE);
    let hf = t.health_factor_for_raw(ALICE, account);
    assert!(hf > GATE, "starts healthy: {hf}");
    let (supplies, _) = t.ctrl_client().get_account_positions(&account);
    let keys = supplies.keys();
    assert_eq!(keys.len(), 2);
    let a = keys.get(0).unwrap();
    let b = keys.get(1).unwrap();
    let (a_name, b_name) = (one_dollar(&t, &a).0, one_dollar(&t, &b).0);
    t.edit_asset_config(a_name, |c| {
        c.loan_to_value = 6_500;
        c.liquidation_threshold = 7_000;
    });
    t.edit_asset_config(b_name, |c| {
        c.liquidation_threshold = 8_500;
    });
    (t, account, a, b)
}

/// About one dollar of the leg keyed `key`, for a top-up that touches it.
fn one_dollar(t: &LendingTest, key: &HubAssetKey) -> (&'static str, f64) {
    if key.asset == t.resolve_asset("USDC") {
        ("USDC", 1.0)
    } else {
        ("ETH", 0.0005)
    }
}

/// controller-6a, residual pin. A $1 supply to both legs, in either leg
/// order, applies B's loosening and keeps A's stale, looser threshold,
/// because A's gate reads B at its stored 8_000. One keeper refresh then
/// applies the tightening, which the finished state allows.
#[test]
fn controller_6a_cr11_residual_supply_batch_gates_the_tightening_against_the_stored_sibling() {
    for reversed in [false, true] {
        let (mut t, account, a, b) = mixed_pending_account();
        let mut legs = [one_dollar(&t, &a), one_dollar(&t, &b)];
        if reversed {
            legs.reverse();
        }
        t.supply_bulk(ALICE, &legs);
        assert_eq!(
            (lt_of(&t, account, &a), lt_of(&t, account, &b)),
            (8_000, 8_500),
            "after the supply batch (legs {legs:?}): B loosened, A kept its stale tuple"
        );
        t.update_account_threshold(true, &[account]);
        assert_eq!(
            (lt_of(&t, account, &a), lt_of(&t, account, &b)),
            (7_000, 8_500),
            "one more refresh applies the tightening"
        );
        let hf = t.health_factor_for_raw(ALICE, account);
        assert!(hf >= GATE, "the finished state clears the gate: {hf}");
    }
}

/// controller-6b, residual pin. The same order in `update_account_threshold`:
/// the first keeper call loosens B and keeps A; the second applies A.
#[test]
fn controller_6b_cr11_residual_second_keeper_call_applies_the_tightening() {
    let (t, account, a, b) = mixed_pending_account();
    t.update_account_threshold(true, &[account]);
    assert_eq!(
        (lt_of(&t, account, &a), lt_of(&t, account, &b)),
        (8_000, 8_500),
        "first keeper call: B loosened, A kept its stale tuple"
    );
    t.update_account_threshold(true, &[account]);
    assert_eq!(
        (lt_of(&t, account, &a), lt_of(&t, account, &b)),
        (7_000, 8_500),
        "second keeper call: A tightened"
    );
    let hf = t.health_factor_for_raw(ALICE, account);
    assert!(hf >= GATE, "the finished state clears the gate: {hf}");
}
