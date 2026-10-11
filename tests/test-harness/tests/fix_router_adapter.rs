//! Fix family `router_adapter`: the production router behind the controller.
//!
//! R-B (third pass of the defensive review): the router's same-token check compared registry
//! indices, so a registry listing the input token twice could declare the input as the output.
//! Behind the controller's `multiply`, the router then paid the unrouted debt token back as the
//! "output", swept the routed hop's output into its admin residual bucket, and the controller
//! rejected the receipt with `NoSwapOutput` (502). The fix compares registry addresses in the
//! decoder, so the router refuses the payload with its own `SameToken` (25) before it pulls the
//! input, and the multiply reverts whole with the error that names the defect.
//!
//! Requires the router's release WASM (`make build-one CRATE=swap-aggregator`): this binary reads
//! `target/wasm32v1-none/release/swap_aggregator.wasm`, like `rv_real_router`.

use controller::types::PositionMode;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::xdr::ToXdr;
use soroban_sdk::{
    contract, contractclient, contractimpl, contracttype, token, vec, Address, Bytes, Env, Error,
    Vec,
};
use test_harness::errors::NO_SWAP_OUTPUT;
use test_harness::{
    eth_preset, hub_asset, usdc_preset, HubAssetKey, LendingTest, ALICE, BOB, HARNESS_SPOKE,
};

const USDC: i128 = 10_000_000;
const ETH: i128 = 10_000_000;
const DEBT: i128 = 2_000 * USDC;
const PAYMENT: i128 = ETH;
/// The router's `SameToken` code (`contracts/swap-aggregator/src/errors.rs`).
const ROUTER_SAME_TOKEN: u32 = 25;
/// USDC routed through the hop by the `Fixed` instruction: 0.1 USDC, which buys 500 ETH base
/// units, inside the router's 1_000-unit residual allowance, so the pre-fix route settles in the
/// router and reaches the controller's receipt check.
const ROUTED: i128 = 1_000_000;

/// The subset of the router ABI these tests read.
#[contractclient(name = "RealRouterClient")]
pub trait RealRouter {
    fn admin_fee_balance(env: Env, token: Address) -> i128;
}

/// Byte-for-byte the router's `StrategyPayload`.
#[contracttype]
#[derive(Clone)]
pub struct StrategyPayload {
    pub amounts: Vec<i128>,
    pub assets: Vec<Address>,
    pub ops: Bytes,
}

#[contracttype]
#[derive(Clone)]
pub enum PoolKey {
    TokenA,
    TokenB,
}

/// Aquarius-shaped pool paying 5 / 10_000 of the other token per input unit.
#[contract]
pub struct FixedRatePool;

#[contractimpl]
impl FixedRatePool {
    pub fn init(env: Env, token_a: Address, token_b: Address) {
        env.storage().instance().set(&PoolKey::TokenA, &token_a);
        env.storage().instance().set(&PoolKey::TokenB, &token_b);
    }

    pub fn get_tokens(env: Env) -> Vec<Address> {
        let a: Address = env.storage().instance().get(&PoolKey::TokenA).unwrap();
        let b: Address = env.storage().instance().get(&PoolKey::TokenB).unwrap();
        vec![&env, a, b]
    }

    pub fn swap(
        env: Env,
        user: Address,
        in_idx: u32,
        out_idx: u32,
        in_amount: u128,
        _out_min: u128,
    ) -> u128 {
        let tokens = Self::get_tokens(env.clone());
        let token_in = tokens.get(in_idx).unwrap();
        let token_out = tokens.get(out_idx).unwrap();
        let pool = env.current_contract_address();
        let amount_in = in_amount as i128;
        token::Client::new(&env, &token_in).transfer(&user, &pool, &amount_in);
        let out = amount_in * 5 / 10_000;
        token::Client::new(&env, &token_out).transfer(&pool, &user, &out);
        out as u128
    }
}

fn router_wasm() -> std::vec::Vec<u8> {
    let rel = "target/wasm32v1-none/release/swap_aggregator.wasm";
    ["", "../", "../../"]
        .iter()
        .find_map(|prefix| std::fs::read(format!("{prefix}{rel}")).ok())
        .unwrap_or_else(|| panic!("swap_aggregator.wasm not found; run `make build` first"))
}

/// Registry `[USDC, ETH, pool, USDC]`: the header declares `token_in = 0` and
/// `token_out = 3`, two indices naming the same address. One Aquarius hop routes `ROUTED` USDC
/// (`amounts[1]`, `Mode::Fixed`) into ETH; `amounts[0] = 1` is the route minimum.
fn input_declared_as_output_payload(
    env: &Env,
    usdc: &Address,
    eth: &Address,
    pool: &Address,
) -> Bytes {
    let mut ops = Bytes::new(env);
    // version, token_in, token_out, min_out, referral (u32 BE), op_count, weight_count
    for byte in [1u8, 0, 3, 0, 0, 0, 0, 0, 1, 0] {
        ops.push_back(byte);
    }
    // opcode 1 = Aquarius swap, mode 3 = Fixed(amounts[1]), pool, token_in, token_out
    for byte in [1u8, 3, 2, 0, 1] {
        ops.push_back(byte);
    }
    StrategyPayload {
        amounts: vec![env, 1_i128, ROUTED],
        assets: vec![env, usdc.clone(), eth.clone(), pool.clone(), usdc.clone()],
        ops,
    }
    .to_xdr(env)
}

#[test]
fn fix_rb_multiply_with_the_input_declared_as_the_output_is_refused_by_the_router() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();
    t.env.cost_estimate().budget().reset_unlimited();
    t.supply(ALICE, "USDC", 100_000.0);
    t.supply(BOB, "ETH", 100.0);

    let usdc = t.resolve_asset("USDC");
    let eth = t.resolve_asset("ETH");
    let wasm = router_wasm();
    let router = t.env.register(&wasm[..], (Address::generate(&t.env),));
    t.ctrl_client().set_swap_aggregator(&router);
    let pool = t.env.register(FixedRatePool, ());
    FixedRatePoolClient::new(&t.env, &pool).init(&usdc, &eth);
    t.resolve_market("ETH")
        .token_admin
        .mint(&pool, &(100 * ETH));
    let carol = t.get_or_create_user("carol");
    t.resolve_market("ETH").token_admin.mint(&carol, &PAYMENT);

    let eth_key: HubAssetKey = hub_asset(eth.clone());
    let usdc_key: HubAssetKey = hub_asset(usdc.clone());
    let payload = input_declared_as_output_payload(&t.env, &usdc, &eth, &pool);
    let result = t.ctrl_client().try_multiply(
        &carol,
        &0u64,
        &HARNESS_SPOKE,
        &eth_key,
        &DEBT,
        &usdc_key,
        &PositionMode::Multiply,
        &payload,
        &Some((eth_key.clone(), PAYMENT)),
        &None,
    );
    let code = match result {
        Err(Ok(err)) | Ok(Err(err)) => err,
        other => panic!("multiply must fail, got {other:?}"),
    };
    assert_ne!(
        code,
        Error::from_contract_error(NO_SWAP_OUTPUT),
        "pre-fix outcome: the router settled and the controller found no ETH receipt"
    );
    assert_eq!(
        code,
        Error::from_contract_error(ROUTER_SAME_TOKEN),
        "the router refuses the payload in decode with SameToken"
    );

    // Nothing moved anywhere.
    let bal = |token: &Address, of: &Address| token::Client::new(&t.env, token).balance(of);
    assert_eq!(bal(&eth, &carol), PAYMENT, "carol keeps her payment");
    assert_eq!(bal(&usdc, &pool), 0, "no input reached the hop");
    assert_eq!(bal(&eth, &pool), 100 * ETH, "the hop paid nothing");
    for token in [&usdc, &eth] {
        assert_eq!(bal(token, &router), 0, "router holds nothing");
        assert_eq!(bal(token, &t.controller), 0, "controller holds nothing");
    }
    assert_eq!(
        RealRouterClient::new(&t.env, &router).admin_fee_balance(&eth),
        0,
        "no residual booked"
    );
}

/// Skeptic (router_adapter, R-B), sibling entry point: every controller strategy reaches the
/// router through the one call site in `contracts/controller/src/strategies/swap.rs`, so the
/// decoder's address check covers `swap_collateral` as it covers `multiply`. The same
/// input-declared-as-output payload, sent through `swap_collateral`, is refused by the router's
/// `SameToken` (25) and leaves the account, the pool and the router exactly as they were.
#[test]
fn skeptic_rb_swap_collateral_with_the_input_declared_as_the_output_is_refused_by_the_router() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();
    t.env.cost_estimate().budget().reset_unlimited();
    t.supply("carol", "USDC", 10_000.0);
    t.supply(BOB, "ETH", 100.0);

    let usdc = t.resolve_asset("USDC");
    let eth = t.resolve_asset("ETH");
    let wasm = router_wasm();
    let router = t.env.register(&wasm[..], (Address::generate(&t.env),));
    t.ctrl_client().set_swap_aggregator(&router);
    let pool = t.env.register(FixedRatePool, ());
    FixedRatePoolClient::new(&t.env, &pool).init(&usdc, &eth);
    t.resolve_market("ETH")
        .token_admin
        .mint(&pool, &(100 * ETH));

    let carol = t.get_or_create_user("carol");
    let account_id = t.resolve_account_id("carol");
    let (supply_before, borrow_before) = t.ctrl_client().get_account_positions(&account_id);
    let usdc_pool_before =
        token::Client::new(&t.env, &usdc).balance(&t.resolve_market("USDC").pool);

    let eth_key: HubAssetKey = hub_asset(eth.clone());
    let usdc_key: HubAssetKey = hub_asset(usdc.clone());
    let payload = input_declared_as_output_payload(&t.env, &usdc, &eth, &pool);
    let result = t.ctrl_client().try_swap_collateral(
        &carol,
        &account_id,
        &usdc_key,
        &(100 * USDC),
        &eth_key,
        &payload,
    );
    let code = match result {
        Err(Ok(err)) => err,
        other => panic!("swap_collateral must fail with a contract error, got {other:?}"),
    };
    assert_eq!(code, Error::from_contract_error(ROUTER_SAME_TOKEN));

    let (supply_after, borrow_after) = t.ctrl_client().get_account_positions(&account_id);
    assert_eq!(supply_after, supply_before, "collateral untouched");
    assert_eq!(borrow_after, borrow_before, "no debt appeared");
    let bal = |token: &Address, of: &Address| token::Client::new(&t.env, token).balance(of);
    assert_eq!(bal(&usdc, &t.resolve_market("USDC").pool), usdc_pool_before);
    assert_eq!(bal(&usdc, &pool), 0, "no input reached the hop");
    for token in [&usdc, &eth] {
        assert_eq!(bal(token, &router), 0, "router holds nothing");
        assert_eq!(bal(token, &t.controller), 0, "controller holds nothing");
    }
    assert_eq!(
        RealRouterClient::new(&t.env, &router).admin_fee_balance(&eth),
        0
    );
}
