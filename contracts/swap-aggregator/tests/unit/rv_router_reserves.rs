//! RV break pass: router fee reserves and vault accounting.
//!
//! Every probe parks a real fee reserve in the router, then lets an adversarial
//! venue reach for it from inside a hop while only the sender's own auth tree is
//! mocked. The venue swallows each refusal with `try_invoke_contract`, so the
//! strategy settles and the reserve can be inspected afterwards. Each assertion
//! states the safe number; a failure is a finding.

use soroban_sdk::testutils::{Address as _, MockAuth, MockAuthInvoke};
use soroban_sdk::xdr::ToXdr;
use soroban_sdk::{
    contract, contractimpl, contracttype, token, vec, Address, Bytes, Env, IntoVal, Symbol, Val,
    Vec,
};

use crate::program::encode::{self, RawOp};
use crate::reserved_fee_balance;
use crate::storage::accumulate_fee;
use crate::types::{DataKey, StrategyPayload, SwapVenue};
use crate::{Router, RouterClient};

use super::support::{aquarius_mock, new_asset, one_hop_path, strategy_xdr, Builder, PreSwap};

/// Units of the reserve token the router custodies on behalf of the admin bucket.
const RESERVE: i128 = 5_000;
/// Units the attacker routes through the strategy.
const IN: i128 = 1_000;

#[contracttype]
enum Key {
    TokenA,
    TokenB,
    Share,
    Reserve,
    /// Whether the n-th illegitimate pull succeeded.
    Attempt(u32),
}

fn record(env: &Env, n: u32, ok: bool) {
    env.storage().instance().set(&Key::Attempt(n), &ok);
}

/// `[legit, illegit_1, illegit_2]`: the authorized pull is routed through the
/// same `try_*` helper as the two illegitimate ones, so a `false` means the host
/// refused the call rather than that the helper was malformed.
fn attempt_vec(env: &Env) -> Vec<bool> {
    let mut out = Vec::new(env);
    for n in 0..3u32 {
        let ok: bool = env
            .storage()
            .instance()
            .get(&Key::Attempt(n))
            .unwrap_or(false);
        out.push_back(ok);
    }
    out
}

/// Tries `token.transfer(from, to, amount)` and reports whether it went through.
fn try_transfer(env: &Env, token: &Address, from: &Address, to: &Address, amount: i128) -> bool {
    let args: Vec<Val> = vec![
        env,
        from.into_val(env),
        to.into_val(env),
        amount.into_val(env),
    ];
    env.try_invoke_contract::<Val, soroban_sdk::Error>(token, &Symbol::new(env, "transfer"), args)
        .is_ok()
}

fn try_burn(env: &Env, token: &Address, from: &Address, amount: i128) -> bool {
    let args: Vec<Val> = vec![env, from.into_val(env), amount.into_val(env)];
    env.try_invoke_contract::<Val, soroban_sdk::Error>(token, &Symbol::new(env, "burn"), args)
        .is_ok()
}

fn try_transfer_from(
    env: &Env,
    token: &Address,
    spender: &Address,
    from: &Address,
    to: &Address,
    amount: i128,
) -> bool {
    let args: Vec<Val> = vec![
        env,
        spender.into_val(env),
        from.into_val(env),
        to.into_val(env),
        amount.into_val(env),
    ];
    env.try_invoke_contract::<Val, soroban_sdk::Error>(
        token,
        &Symbol::new(env, "transfer_from"),
        args,
    )
    .is_ok()
}

/// Aquarius-shaped pool: takes its authorized input, then reaches for the
/// reserve token and for a second helping of the input.
#[contract]
pub struct GreedyAqPool;

#[contractimpl]
impl GreedyAqPool {
    pub fn init(env: Env, token_a: Address, token_b: Address, reserve: Address) {
        env.storage().instance().set(&Key::TokenA, &token_a);
        env.storage().instance().set(&Key::TokenB, &token_b);
        env.storage().instance().set(&Key::Reserve, &reserve);
    }

    pub fn get_tokens(env: Env) -> Vec<Address> {
        let a: Address = env.storage().instance().get(&Key::TokenA).unwrap();
        let b: Address = env.storage().instance().get(&Key::TokenB).unwrap();
        vec![&env, a, b]
    }

    pub fn attempts(env: Env) -> Vec<bool> {
        attempt_vec(&env)
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
        let token_in = tokens.get_unchecked(in_idx);
        let token_out = tokens.get_unchecked(out_idx);
        let reserve: Address = env.storage().instance().get(&Key::Reserve).unwrap();
        let pool = env.current_contract_address();
        let amount = in_amount as i128;

        record(&env, 0, try_transfer(&env, &token_in, &user, &pool, amount));
        record(&env, 1, try_transfer(&env, &reserve, &user, &pool, 1));
        record(&env, 2, try_transfer(&env, &token_in, &user, &pool, 1));
        token::Client::new(&env, &token_out).transfer(&pool, &user, &amount);
        in_amount
    }
}

/// Mint pool whose second constituent is the reserve token the vault holds none of.
#[contract]
pub struct GreedyLpPool;

#[contractimpl]
impl GreedyLpPool {
    pub fn init(env: Env, token_a: Address, reserve: Address, share: Address) {
        env.storage().instance().set(&Key::TokenA, &token_a);
        env.storage().instance().set(&Key::TokenB, &reserve);
        env.storage().instance().set(&Key::Share, &share);
    }

    pub fn get_tokens(env: Env) -> Vec<Address> {
        let a: Address = env.storage().instance().get(&Key::TokenA).unwrap();
        let b: Address = env.storage().instance().get(&Key::TokenB).unwrap();
        vec![&env, a, b]
    }

    pub fn share_id(env: Env) -> Address {
        env.storage().instance().get(&Key::Share).unwrap()
    }

    pub fn attempts(env: Env) -> Vec<bool> {
        attempt_vec(&env)
    }

    pub fn deposit(
        env: Env,
        user: Address,
        desired: Vec<u128>,
        min_shares: u128,
    ) -> (Vec<u128>, u128) {
        let tokens = Self::get_tokens(env.clone());
        let pool = env.current_contract_address();
        let taken = desired.get_unchecked(0) as i128;
        record(
            &env,
            0,
            try_transfer(&env, &tokens.get_unchecked(0), &user, &pool, taken),
        );
        // The vault offered 0 of the reserve constituent; take some anyway.
        record(
            &env,
            1,
            try_transfer(&env, &tokens.get_unchecked(1), &user, &pool, 1),
        );
        record(
            &env,
            2,
            try_transfer(&env, &tokens.get_unchecked(1), &user, &pool, RESERVE),
        );
        assert!(taken as u128 >= min_shares);
        token::StellarAssetClient::new(&env, &Self::share_id(env.clone())).mint(&user, &taken);
        (vec![&env, taken as u128, 0u128], taken as u128)
    }
}

/// Burn pool that declares the reserve token as its share token.
#[contract]
pub struct GreedyBurnPool;

#[contractimpl]
impl GreedyBurnPool {
    pub fn init(env: Env, token_a: Address, token_b: Address, share: Address) {
        env.storage().instance().set(&Key::TokenA, &token_a);
        env.storage().instance().set(&Key::TokenB, &token_b);
        env.storage().instance().set(&Key::Share, &share);
    }

    pub fn get_tokens(env: Env) -> Vec<Address> {
        let a: Address = env.storage().instance().get(&Key::TokenA).unwrap();
        let b: Address = env.storage().instance().get(&Key::TokenB).unwrap();
        vec![&env, a, b]
    }

    pub fn share_id(env: Env) -> Address {
        env.storage().instance().get(&Key::Share).unwrap()
    }

    pub fn attempts(env: Env) -> Vec<bool> {
        attempt_vec(&env)
    }

    pub fn withdraw(env: Env, user: Address, shares: u128, _mins: Vec<u128>) -> Vec<u128> {
        let tokens = Self::get_tokens(env.clone());
        let share = Self::share_id(env.clone());
        let pool = env.current_contract_address();
        let amount = shares as i128;
        record(&env, 0, try_burn(&env, &share, &user, amount));
        record(&env, 1, try_burn(&env, &share, &user, 1));
        record(&env, 2, try_transfer(&env, &share, &user, &pool, 1));
        token::Client::new(&env, &tokens.get_unchecked(0)).transfer(&pool, &user, &amount);
        vec![&env, shares, 0u128]
    }
}

/// Comet-shaped pool: spends its allowance, then tries again and reaches for the reserve.
#[contract]
pub struct GreedyComet;

#[contractimpl]
impl GreedyComet {
    pub fn init(env: Env, reserve: Address) {
        env.storage().instance().set(&Key::Reserve, &reserve);
    }

    pub fn attempts(env: Env) -> Vec<bool> {
        attempt_vec(&env)
    }

    pub fn swap_exact_amount_in(
        env: Env,
        token_in: Address,
        amount_in: i128,
        token_out: Address,
        _min_out: i128,
        _max_price: i128,
        user: Address,
    ) -> (i128, i128) {
        let pool = env.current_contract_address();
        let reserve: Address = env.storage().instance().get(&Key::Reserve).unwrap();
        record(
            &env,
            0,
            try_transfer_from(&env, &token_in, &pool, &user, &pool, amount_in),
        );
        record(
            &env,
            1,
            try_transfer_from(&env, &token_in, &pool, &user, &pool, 1),
        );
        record(&env, 2, try_transfer(&env, &reserve, &user, &pool, 1));
        token::Client::new(&env, &token_out).transfer(&pool, &user, &amount_in);
        (amount_in, 0)
    }
}

struct Fixture {
    router_addr: Address,
    sender: Address,
    asset_admin: Address,
    reserve: Address,
}

/// Router with `RESERVE` units of a reserve token backing an admin bucket.
fn fixture(env: &Env) -> Fixture {
    let router_addr = env.register(Router, (Address::generate(env),));
    let sender = Address::generate(env);
    let asset_admin = Address::generate(env);
    let (reserve, sac) = new_asset(env, &asset_admin);
    env.mock_all_auths();
    sac.mint(&router_addr, &RESERVE);
    env.as_contract(&router_addr, || {
        accumulate_fee(env, DataKey::AdminFee(reserve.clone()), RESERVE);
    });
    Fixture {
        router_addr,
        sender,
        asset_admin,
        reserve,
    }
}

/// Drops every mocked auth and mocks exactly the sender's own tree.
fn mock_sender_only(env: &Env, f: &Fixture, token_in: &Address, amount: i128, xdr: &Bytes) {
    env.set_auths(&[]);
    env.mock_auths(&[MockAuth {
        address: &f.sender,
        invoke: &MockAuthInvoke {
            contract: &f.router_addr,
            fn_name: "execute_strategy",
            args: (f.sender.clone(), amount, xdr.clone()).into_val(env),
            sub_invokes: &[MockAuthInvoke {
                contract: token_in,
                fn_name: "transfer",
                args: (f.sender.clone(), f.router_addr.clone(), amount).into_val(env),
                sub_invokes: &[],
            }],
        },
    }]);
}

fn assert_reserve_intact(env: &Env, f: &Fixture) {
    let real = token::Client::new(env, &f.reserve).balance(&f.router_addr);
    let reserved = env.as_contract(&f.router_addr, || reserved_fee_balance(env, &f.reserve));
    let bucket = RouterClient::new(env, &f.router_addr).admin_fee_balance(&f.reserve);
    assert_eq!(
        reserved, RESERVE,
        "ReservedTotal must still claim the reserve"
    );
    assert_eq!(
        bucket, RESERVE,
        "AdminFee bucket must still hold the reserve"
    );
    assert_eq!(real, RESERVE, "the router must still custody the reserve");
}

/// R-1: an Aquarius-shaped venue holding the router's single transfer authority
/// cannot pull a second unit of the input, nor one unit of the reserve token.
#[test]
fn aquarius_hop_cannot_reach_reserve_or_double_pull_input() {
    let env = Env::default();
    let f = fixture(&env);
    let (token_a, sac_a) = new_asset(&env, &f.asset_admin);
    let (token_b, sac_b) = new_asset(&env, &f.asset_admin);
    let pool = env.register(GreedyAqPool, ());
    GreedyAqPoolClient::new(&env, &pool).init(&token_a, &token_b, &f.reserve);
    sac_a.mint(&f.sender, &IN);
    sac_b.mint(&pool, &IN);

    let xdr = strategy_xdr(
        &env,
        token_a.clone(),
        token_b.clone(),
        IN,
        alloc::vec![one_hop_path(
            &env,
            SwapVenue::Aquarius,
            pool.clone(),
            token_a.clone(),
            token_b.clone(),
            1_000_000,
        )],
    );
    mock_sender_only(&env, &f, &token_a, IN, &xdr);

    let out = RouterClient::new(&env, &f.router_addr).execute_strategy(&f.sender, &IN, &xdr);
    assert_eq!(out, IN);
    assert_eq!(
        GreedyAqPoolClient::new(&env, &pool).attempts(),
        vec![&env, true, false, false],
        "neither the reserve pull nor the second input pull may succeed"
    );
    assert_reserve_intact(&env, &f);
    assert_eq!(
        token::Client::new(&env, &token_a).balance(&f.router_addr),
        0
    );
}

/// R-1: a mint whose pool lists the reserve token as a constituent the vault
/// holds none of gets no authority over it, in any amount.
#[test]
fn mint_cannot_pull_an_unfunded_reserve_constituent() {
    let env = Env::default();
    let f = fixture(&env);
    let (token_a, sac_a) = new_asset(&env, &f.asset_admin);
    let pool = env.register(GreedyLpPool, ());
    let share = env
        .register_stellar_asset_contract_v2(pool.clone())
        .address();
    GreedyLpPoolClient::new(&env, &pool).init(&token_a, &f.reserve, &share);
    sac_a.mint(&f.sender, &IN);

    let xdr = Builder::new(&env, token_a.clone(), share.clone(), IN, 0)
        .mint(pool.clone(), share.clone(), IN, PreSwap::None, true)
        .build();
    mock_sender_only(&env, &f, &token_a, IN, &xdr);

    let out = RouterClient::new(&env, &f.router_addr).execute_strategy(&f.sender, &IN, &xdr);
    assert_eq!(out, IN);
    assert_eq!(
        GreedyLpPoolClient::new(&env, &pool).attempts(),
        vec![&env, true, false, false],
        "an unfunded constituent carries no authority, for 1 unit or the whole reserve"
    );
    assert_reserve_intact(&env, &f);
    assert_eq!(token::Client::new(&env, &share).balance(&f.sender), IN);
}

/// R-1: a burn whose pool declares the reserve token as its share token can burn
/// only the in-flight shares, never a unit of the reserve behind them.
#[test]
fn burn_of_reserve_token_shares_cannot_overreach_into_the_reserve() {
    let env = Env::default();
    let f = fixture(&env);
    let (token_x, sac_x) = new_asset(&env, &f.asset_admin);
    let (token_y, _sac_y) = new_asset(&env, &f.asset_admin);
    let pool = env.register(GreedyBurnPool, ());
    GreedyBurnPoolClient::new(&env, &pool).init(&token_x, &token_y, &f.reserve);
    token::StellarAssetClient::new(&env, &f.reserve).mint(&f.sender, &IN);
    sac_x.mint(&pool, &IN);

    let xdr = Builder::new(&env, f.reserve.clone(), token_x.clone(), IN, 0)
        .burn(pool.clone(), f.reserve.clone(), alloc::vec![0, 0])
        .build();
    let reserve = f.reserve.clone();
    mock_sender_only(&env, &f, &reserve, IN, &xdr);

    let out = RouterClient::new(&env, &f.router_addr).execute_strategy(&f.sender, &IN, &xdr);
    assert_eq!(out, IN);
    assert_eq!(
        GreedyBurnPoolClient::new(&env, &pool).attempts(),
        vec![&env, true, false, false],
        "a second burn and a transfer of the share token must both be refused"
    );
    assert_reserve_intact(&env, &f);
    assert_eq!(token::Client::new(&env, &token_x).balance(&f.sender), IN);
}

/// R-1: a Comet-shaped venue spends exactly its allowance; a second
/// `transfer_from` and a reserve pull are refused, and the allowance ends at 0.
#[test]
fn comet_hop_cannot_spend_allowance_twice_or_reach_reserve() {
    let env = Env::default();
    let f = fixture(&env);
    let (token_a, sac_a) = new_asset(&env, &f.asset_admin);
    let (token_b, sac_b) = new_asset(&env, &f.asset_admin);
    let pool = env.register(GreedyComet, ());
    GreedyCometClient::new(&env, &pool).init(&f.reserve);
    sac_a.mint(&f.sender, &IN);
    sac_b.mint(&pool, &IN);

    let xdr = strategy_xdr(
        &env,
        token_a.clone(),
        token_b.clone(),
        IN,
        alloc::vec![one_hop_path(
            &env,
            SwapVenue::CometDex,
            pool.clone(),
            token_a.clone(),
            token_b.clone(),
            1_000_000,
        )],
    );
    mock_sender_only(&env, &f, &token_a, IN, &xdr);

    let out = RouterClient::new(&env, &f.router_addr).execute_strategy(&f.sender, &IN, &xdr);
    assert_eq!(out, IN);
    assert_eq!(
        GreedyCometClient::new(&env, &pool).attempts(),
        vec![&env, true, false, false]
    );
    assert_eq!(
        token::Client::new(&env, &token_a).allowance(&f.router_addr, &pool),
        0
    );
    assert_reserve_intact(&env, &f);
}

/// Decode compares `token_in`/`token_out` by registry index, so a registry that
/// lists the same address twice is accepted. Pins that this is harmless for the
/// fee reserve: one input-side fee, a round trip, and backed buckets.
#[test]
fn duplicate_registry_address_in_and_out_round_trips_with_one_input_fee() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let router_addr = env.register(Router, (admin.clone(),));
    let router = RouterClient::new(&env, &router_addr);
    let sender = Address::generate(&env);
    let asset_admin = Address::generate(&env);
    let (token_a, sac_a) = new_asset(&env, &asset_admin);
    let (token_b, sac_b) = new_asset(&env, &asset_admin);
    let pool = env.register(aquarius_mock::AqPool, ());
    aquarius_mock::AqPoolClient::new(&env, &pool).init(&token_a, &token_b);
    router.set_static_fee(&100);
    let referral = router.add_referral(&Address::generate(&env), &100);
    sac_a.mint(&sender, &IN);
    sac_a.mint(&pool, &IN);
    sac_b.mint(&pool, &IN);

    // assets: [A, A, B, pool]; in = 0, out = 1 (same address, different index).
    let ops = encode::program(
        &env,
        0,
        1,
        0,
        referral as u32,
        &[
            RawOp {
                opcode: 1,
                mode: encode::ALL,
                idx_a: 3,
                idx_b: 0,
                idx_c: 2,
            },
            RawOp {
                opcode: 1,
                mode: encode::PREV,
                idx_a: 3,
                idx_b: 2,
                idx_c: 1,
            },
        ],
        &[],
    );
    let mut assets = Vec::new(&env);
    for a in [&token_a, &token_a, &token_b, &pool] {
        assets.push_back(a.clone());
    }
    let xdr = StrategyPayload {
        amounts: vec![&env, 980],
        assets,
        ops,
    }
    .to_xdr(&env);

    let out = router.execute_strategy(&sender, &IN, &xdr);
    assert_eq!(out, 980, "round trip after a 2% input fee");
    assert_eq!(token::Client::new(&env, &token_a).balance(&sender), 980);
    assert_eq!(router.admin_fee_balance(&token_a), 10);
    assert_eq!(router.referral_fee_balance(&referral, &token_a), 10);
    let reserved = env.as_contract(&router_addr, || reserved_fee_balance(&env, &token_a));
    assert_eq!(reserved, 20);
    assert_eq!(token::Client::new(&env, &token_a).balance(&router_addr), 20);
}
