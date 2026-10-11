//! The controller through the production swap-aggregator router.
//!
//! Every other strategy test drives a router double. This binary deploys the real router from
//! its release WASM behind the controller's `multiply` flow, with an Aquarius-shaped one-hop pool,
//! and pins the end-to-end contract between the two:
//!
//! - settlement is measured on both sides: the account's collateral equals the initial payment
//!   plus exactly what the pool paid, the router keeps nothing, and the controller keeps nothing;
//! - the router's referral and static fees come off the routed input, land in backed buckets that
//!   a sweep cannot take, and the account is credited only the net output;
//! - a route whose minimum is not met reverts the whole multiply: no account, no debt, no move;
//! - under enforcing authorization the user signs one `multiply` invocation plus the initial
//!   payment; the router consumes only the controller's single-transfer authority
//!   (INV-STRAT-01) and nothing is attributed to the user below that.
//!
//! Requires `make build` (reads `target/wasm32v1-none/release/swap_aggregator.wasm`).

use controller::types::PositionMode;
use soroban_sdk::testutils::{Address as _, MockAuth, MockAuthInvoke};
use soroban_sdk::xdr::ToXdr;
use soroban_sdk::{
    contract, contractclient, contractimpl, contracttype, token, vec, Address, Bytes, Env, IntoVal,
    Vec,
};
use test_harness::{
    apply_flash_fee, eth_preset, hub_asset, usdc_preset, HubAssetKey, LendingTest, ALICE, BOB,
};

const USDC: i128 = 10_000_000;
const ETH: i128 = 10_000_000;
const DEBT: i128 = 2_000 * USDC;
const PAYMENT: i128 = ETH;
const SPOKE: u32 = test_harness::HARNESS_SPOKE;

/// What the controller hands the router: the borrowed amount net of the flash-loan fee.
fn routed_input() -> i128 {
    apply_flash_fee(DEBT)
}

/// Pool payout for `input` at 5 / 10_000 ETH per USDC base unit.
fn pool_out(input: i128) -> i128 {
    input * 5 / 10_000
}

/// The subset of the router ABI these tests drive.
#[contractclient(name = "RealRouterClient")]
pub trait RealRouter {
    fn add_referral(env: Env, owner: Address, fee_bps: u32) -> u64;
    fn set_static_fee(env: Env, fee_bps: u32);
    fn admin_fee_balance(env: Env, token: Address) -> i128;
    fn referral_fee_balance(env: Env, id: u64, token: Address) -> i128;
    fn sweep_balance(env: Env, recipient: Address, tokens: Vec<Address>);
    fn claim_referral_fees(env: Env, id: u64, tokens: Vec<Address>);
    fn claim_admin_fees(env: Env, recipient: Address, tokens: Vec<Address>);
}

/// Byte-for-byte the router's `StrategyPayload`: same field names and types, so the XDR
/// matches without a dependency on the router crate.
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
    Num,
    Den,
}

/// Aquarius-shaped pool paying `in * num / den` of the other token from its own balance.
/// It pulls its input with `token.transfer(user, pool, amount)`, so the router must have
/// authorized exactly that call.
#[contract]
pub struct FixedRatePool;

#[contractimpl]
impl FixedRatePool {
    pub fn init(env: Env, token_a: Address, token_b: Address, num: i128, den: i128) {
        env.storage().instance().set(&PoolKey::TokenA, &token_a);
        env.storage().instance().set(&PoolKey::TokenB, &token_b);
        env.storage().instance().set(&PoolKey::Num, &num);
        env.storage().instance().set(&PoolKey::Den, &den);
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
        let num: i128 = env.storage().instance().get(&PoolKey::Num).unwrap();
        let den: i128 = env.storage().instance().get(&PoolKey::Den).unwrap();
        let pool = env.current_contract_address();
        let amount_in = in_amount as i128;
        token::Client::new(&env, &token_in).transfer(&user, &pool, &amount_in);
        let out = amount_in * num / den;
        token::Client::new(&env, &token_out).transfer(&pool, &user, &out);
        out as u128
    }
}

/// One Aquarius hop `assets[0] -> assets[1]` through `assets[2]`, spending the whole input,
/// with `amounts[0]` as the strategy minimum, in the router's packed wire layout.
fn one_hop_payload(
    env: &Env,
    token_in: &Address,
    token_out: &Address,
    pool: &Address,
    min_out: i128,
    referral_id: u32,
) -> Bytes {
    let mut ops = Bytes::new(env);
    ops.push_back(1); // wire version
    ops.push_back(0); // token_in  -> assets[0]
    ops.push_back(1); // token_out -> assets[1]
    ops.push_back(0); // min_out   -> amounts[0]
    for byte in referral_id.to_be_bytes() {
        ops.push_back(byte);
    }
    ops.push_back(1); // op_count
    ops.push_back(0); // weight_count
                      // opcode 1 = Aquarius swap, mode 0 = whole balance, pool, token_in, token_out
    for byte in [1u8, 0, 2, 0, 1] {
        ops.push_back(byte);
    }
    StrategyPayload {
        amounts: vec![env, min_out],
        assets: vec![env, token_in.clone(), token_out.clone(), pool.clone()],
        ops,
    }
    .to_xdr(env)
}

fn router_wasm() -> std::vec::Vec<u8> {
    let rel = "target/wasm32v1-none/release/swap_aggregator.wasm";
    ["", "../", "../../"]
        .iter()
        .find_map(|prefix| std::fs::read(format!("{prefix}{rel}")).ok())
        .unwrap_or_else(|| panic!("swap_aggregator.wasm not found; run `make build` first"))
}

struct Rig {
    t: LendingTest,
    router: Address,
    pool: Address,
    carol: Address,
    usdc: Address,
    eth: Address,
}

impl Rig {
    /// Markets, a funded pool paying `num / den` ETH per USDC base unit, the real router behind
    /// the controller, and Carol holding one ETH for her initial payment.
    fn new(num: i128, den: i128) -> Self {
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
        let wasm = router_wasm();
        let router = t.env.register(&wasm[..], (router_admin.clone(),));
        t.ctrl_client().set_swap_aggregator(&router);

        let pool = t.env.register(FixedRatePool, ());
        FixedRatePoolClient::new(&t.env, &pool).init(&usdc, &eth, &num, &den);
        t.resolve_market("ETH")
            .token_admin
            .mint(&pool, &(100 * ETH));

        let carol = t.get_or_create_user("carol");
        t.resolve_market("ETH").token_admin.mint(&carol, &PAYMENT);

        Self {
            t,
            router,
            pool,
            carol,
            usdc,
            eth,
        }
    }

    fn router(&self) -> RealRouterClient<'_> {
        RealRouterClient::new(&self.t.env, &self.router)
    }

    fn usdc_key(&self) -> HubAssetKey {
        hub_asset(self.usdc.clone())
    }

    fn eth_key(&self) -> HubAssetKey {
        hub_asset(self.eth.clone())
    }

    fn balance(&self, token: &Address, of: &Address) -> i128 {
        token::Client::new(&self.t.env, token).balance(of)
    }

    fn multiply(&self, payload: &Bytes) -> Result<u64, soroban_sdk::Error> {
        match self.t.ctrl_client().try_multiply(
            &self.carol,
            &0u64,
            &SPOKE,
            &self.eth_key(),
            &DEBT,
            &self.usdc_key(),
            &PositionMode::Multiply,
            payload,
            &Some((self.eth_key(), PAYMENT)),
            &None,
        ) {
            Ok(Ok(id)) => Ok(id),
            Ok(Err(err)) => Err(err),
            Err(Ok(err)) => Err(err),
            Err(Err(invoke)) => panic!("host-level failure: {invoke:?}"),
        }
    }
}

#[test]
fn multiply_through_the_real_router_settles_exactly_what_the_pool_paid() {
    // 5 / 10_000 ETH per USDC base unit: 2 000 USDC buys 1 ETH.
    let r = Rig::new(5, 10_000);
    let input = routed_input();
    let payload = one_hop_payload(&r.t.env, &r.usdc, &r.eth, &r.pool, pool_out(input), 0);

    let pool_eth_before = r.balance(&r.eth, &r.pool);
    let id = r
        .multiply(&payload)
        .expect("multiply through the real router");

    let pool_paid = pool_eth_before - r.balance(&r.eth, &r.pool);
    assert_eq!(
        pool_paid,
        pool_out(input),
        "the pool pays the fixed rate on the input"
    );
    assert_eq!(
        r.balance(&r.usdc, &r.pool),
        input,
        "the whole net borrowed input reached the pool"
    );
    assert_eq!(
        r.t.ctrl_client().get_collateral_amount(&id, &r.eth_key()),
        PAYMENT + pool_paid,
        "collateral is the payment plus the measured pool payout, nothing else"
    );
    assert!(
        r.t.ctrl_client().get_borrow_amount(&id, &r.usdc_key()) >= DEBT,
        "the account owes at least what was borrowed"
    );

    for (name, token) in [("USDC", &r.usdc), ("ETH", &r.eth)] {
        assert_eq!(r.balance(token, &r.router), 0, "router keeps no {name}");
        assert_eq!(
            r.balance(token, &r.t.controller),
            0,
            "controller keeps no {name}"
        );
        assert_eq!(
            r.balance(token, &r.carol),
            0,
            "nothing leaks to carol as {name}"
        );
    }
}

#[test]
fn referral_and_static_fees_come_off_the_input_and_stay_backed_against_a_sweep() {
    let r = Rig::new(5, 10_000);
    let referrer = Address::generate(&r.t.env);
    let treasury = Address::generate(&r.t.env);
    let stranger = Address::generate(&r.t.env);
    r.router().set_static_fee(&50);
    let referral = r.router().add_referral(&referrer, &50);
    assert_eq!(referral, 1);

    // 50 + 50 bps of the net input, each floored; the rest is routed.
    let input = routed_input();
    let fee = input * 50 / 10_000;
    let routed = input - 2 * fee;
    let expected_out = pool_out(routed);
    let payload = one_hop_payload(
        &r.t.env,
        &r.usdc,
        &r.eth,
        &r.pool,
        expected_out,
        referral as u32,
    );

    let id = r.multiply(&payload).expect("multiply with a referral");

    assert_eq!(r.balance(&r.usdc, &r.pool), routed);
    assert_eq!(
        r.t.ctrl_client().get_collateral_amount(&id, &r.eth_key()),
        PAYMENT + expected_out,
        "only the net output is credited as collateral"
    );
    assert_eq!(r.router().admin_fee_balance(&r.usdc), fee);
    assert_eq!(r.router().referral_fee_balance(&referral, &r.usdc), fee);
    assert_eq!(
        r.balance(&r.usdc, &r.router),
        2 * fee,
        "the router holds exactly the two fee buckets"
    );

    let tokens = vec![&r.t.env, r.usdc.clone(), r.eth.clone()];
    r.router().sweep_balance(&stranger, &tokens);
    assert_eq!(
        r.balance(&r.usdc, &stranger),
        0,
        "a sweep cannot take fee backing"
    );
    assert_eq!(r.balance(&r.usdc, &r.router), 2 * fee);

    r.router().claim_referral_fees(&referral, &tokens);
    assert_eq!(r.balance(&r.usdc, &referrer), fee);
    r.router().claim_admin_fees(&treasury, &tokens);
    assert_eq!(r.balance(&r.usdc, &treasury), fee);
    assert_eq!(
        r.balance(&r.usdc, &r.router),
        0,
        "claims drain the buckets exactly"
    );
}

#[test]
fn a_route_minimum_the_pool_cannot_meet_reverts_the_whole_multiply() {
    let r = Rig::new(5, 10_000);
    let payload = one_hop_payload(
        &r.t.env,
        &r.usdc,
        &r.eth,
        &r.pool,
        pool_out(routed_input()) + 1,
        0,
    );

    assert!(
        r.multiply(&payload).is_err(),
        "the router's minimum-output check must fail the strategy"
    );

    assert_eq!(
        r.balance(&r.eth, &r.carol),
        PAYMENT,
        "carol's payment is untouched"
    );
    assert_eq!(r.balance(&r.usdc, &r.pool), 0, "no input reached the pool");
    assert_eq!(r.balance(&r.usdc, &r.router), 0);
    assert_eq!(r.balance(&r.eth, &r.router), 0);
    assert_eq!(r.balance(&r.usdc, &r.t.controller), 0);
}

#[test]
fn enforcing_auth_attributes_only_multiply_and_the_payment_to_the_user() {
    let r = Rig::new(5, 10_000);
    let input = routed_input();
    let payload = one_hop_payload(&r.t.env, &r.usdc, &r.eth, &r.pool, pool_out(input), 0);
    let env = &r.t.env;
    let controller = r.t.controller.clone();

    // Carol signs her own invocation and the initial payment; nothing else is mocked.
    env.set_auths(&[]);
    env.mock_auths(&[MockAuth {
        address: &r.carol,
        invoke: &MockAuthInvoke {
            contract: &controller,
            fn_name: "multiply",
            args: (
                r.carol.clone(),
                0u64,
                SPOKE,
                r.eth_key(),
                DEBT,
                r.usdc_key(),
                PositionMode::Multiply,
                payload.clone(),
                Some((r.eth_key(), PAYMENT)),
                None::<Bytes>,
            )
                .into_val(env),
            sub_invokes: &[MockAuthInvoke {
                contract: &r.eth,
                fn_name: "transfer",
                args: (r.carol.clone(), controller.clone(), PAYMENT).into_val(env),
                sub_invokes: &[],
            }],
        },
    }]);

    let id = r.multiply(&payload).expect("multiply under enforcing auth");

    let auths = env.auths();
    assert_eq!(auths.len(), 1, "only carol is asked to authorize anything");
    let (who, root) = &auths[0];
    assert_eq!(who, &r.carol);
    assert_eq!(
        root.sub_invocations.len(),
        1,
        "carol's tree holds the payment transfer and nothing below it"
    );
    assert!(root.sub_invocations[0].sub_invocations.is_empty());

    assert_eq!(
        r.balance(&r.usdc, &r.pool),
        input,
        "the router pulled exactly the input"
    );
    assert_eq!(
        r.t.ctrl_client().get_collateral_amount(&id, &r.eth_key()),
        PAYMENT + pool_out(input)
    );
}
