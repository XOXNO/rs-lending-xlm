//! Contract unit tests using `soroban_sdk::testutils`.
//!
//! Covers:
//! - `execute_strategy` happy paths through Soroban mock pools.
//! - Aggregate slippage guard.
//! - PPM split correctness (sum-to-1M, last-path absorbs rounding).
//! - Error paths: empty payload, broken token chain, zero-output-relevant
//!   validation, ppm mismatches.

extern crate std;

use crate::errors::Error;
use crate::types::{StrategyPayload, SwapHop, SwapPath, SwapVenue};
use crate::{Router, RouterClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{
    contract, contractimpl, contracttype, token, vec, xdr::ToXdr, Address, Env, Val, Vec, U256,
};

fn new_asset<'a>(env: &'a Env, admin: &Address) -> (Address, token::StellarAssetClient<'a>) {
    let contract = env.register_stellar_asset_contract_v2(admin.clone());
    let addr = contract.address();
    let sac_admin = token::StellarAssetClient::new(env, &addr);
    (addr, sac_admin)
}

fn one_hop_path(
    env: &Env,
    venue: SwapVenue,
    pool: Address,
    token_in: Address,
    token_out: Address,
    split_ppm: u32,
) -> SwapPath {
    SwapPath {
        split_ppm,
        hops: vec![
            env,
            SwapHop {
                venue,
                amount_out: 0,
                pool,
                token_in,
                token_out,
            },
        ],
    }
}

fn strategy_xdr(
    env: &Env,
    token_in: Address,
    token_out: Address,
    total_min_out: i128,
    paths: Vec<SwapPath>,
) -> soroban_sdk::Bytes {
    strategy_xdr_with_referral(env, token_in, token_out, total_min_out, paths, 0)
}

fn strategy_xdr_with_referral(
    env: &Env,
    token_in: Address,
    token_out: Address,
    total_min_out: i128,
    paths: Vec<SwapPath>,
    referral_id: u64,
) -> soroban_sdk::Bytes {
    StrategyPayload {
        paths,
        referral_id,
        token_in,
        token_out,
        total_min_out,
    }
    .to_xdr(env)
}

mod soroswap_mock {
    use super::*;

    #[contract]
    pub struct SoroswapPair;

    #[contracttype]
    enum SoroswapKey {
        Token0,
        Token1,
        Reserve0,
        Reserve1,
    }

    #[contractimpl]
    impl SoroswapPair {
        pub fn init(env: Env, token0: Address, token1: Address, reserve0: i128, reserve1: i128) {
            env.storage().instance().set(&SoroswapKey::Token0, &token0);
            env.storage().instance().set(&SoroswapKey::Token1, &token1);
            env.storage()
                .instance()
                .set(&SoroswapKey::Reserve0, &reserve0);
            env.storage()
                .instance()
                .set(&SoroswapKey::Reserve1, &reserve1);
        }

        pub fn token_0(env: Env) -> Address {
            env.storage().instance().get(&SoroswapKey::Token0).unwrap()
        }

        pub fn token_1(env: Env) -> Address {
            env.storage().instance().get(&SoroswapKey::Token1).unwrap()
        }

        /// Live reserves, mirroring Soroswap's `get_reserves`. The router reads
        /// these at execution time and sizes the honored output from them.
        pub fn get_reserves(env: Env) -> (i128, i128) {
            (
                env.storage()
                    .instance()
                    .get(&SoroswapKey::Reserve0)
                    .unwrap(),
                env.storage()
                    .instance()
                    .get(&SoroswapKey::Reserve1)
                    .unwrap(),
            )
        }

        /// Uniswap-v2 `swap`: the caller transfers the input BEFORE calling, the
        /// pair sends the requested output, then enforces the constant-product
        /// k-invariant against the 0.3%-fee-adjusted balances. An output larger
        /// than the live reserves permit fails the check here — exactly the
        /// `Error(Contract, #114)` the router avoids by sizing the output from
        /// `get_reserves` rather than trusting a stale quote.
        pub fn swap(env: Env, amount_0_out: i128, amount_1_out: i128, to: Address) {
            let token0: Address = env.storage().instance().get(&SoroswapKey::Token0).unwrap();
            let token1: Address = env.storage().instance().get(&SoroswapKey::Token1).unwrap();
            let reserve0: i128 = env
                .storage()
                .instance()
                .get(&SoroswapKey::Reserve0)
                .unwrap();
            let reserve1: i128 = env
                .storage()
                .instance()
                .get(&SoroswapKey::Reserve1)
                .unwrap();
            let pair = env.current_contract_address();
            let client0 = token::Client::new(&env, &token0);
            let client1 = token::Client::new(&env, &token1);

            if amount_0_out > 0 {
                client0.transfer(&pair, &to, &amount_0_out);
            }
            if amount_1_out > 0 {
                client1.transfer(&pair, &to, &amount_1_out);
            }

            let balance0 = client0.balance(&pair);
            let balance1 = client1.balance(&pair);
            let amount0_in = (balance0 - (reserve0 - amount_0_out)).max(0);
            let amount1_in = (balance1 - (reserve1 - amount_1_out)).max(0);

            let balance0_adjusted = balance0 * 1000 - amount0_in * 3;
            let balance1_adjusted = balance1 * 1000 - amount1_in * 3;
            assert!(
                balance0_adjusted * balance1_adjusted >= reserve0 * reserve1 * 1_000_000,
                "soroswap k-invariant violated"
            );

            env.storage()
                .instance()
                .set(&SoroswapKey::Reserve0, &balance0);
            env.storage()
                .instance()
                .set(&SoroswapKey::Reserve1, &balance1);
        }
    }
}

mod aquarius_mock {
    use super::*;

    #[contract]
    pub struct AqPool;

    #[contracttype]
    enum AqKey {
        TokenA,
        TokenB,
    }

    #[contractimpl]
    impl AqPool {
        pub fn init(env: Env, token_a: Address, token_b: Address) {
            env.storage().instance().set(&AqKey::TokenA, &token_a);
            env.storage().instance().set(&AqKey::TokenB, &token_b);
        }

        pub fn get_tokens(env: Env) -> Vec<Address> {
            let token_a: Address = env.storage().instance().get(&AqKey::TokenA).unwrap();
            let token_b: Address = env.storage().instance().get(&AqKey::TokenB).unwrap();
            vec![&env, token_a, token_b]
        }

        pub fn swap(
            env: Env,
            user: Address,
            in_idx: u32,
            out_idx: u32,
            in_amount: u128,
            _out_min: u128,
        ) -> u128 {
            user.require_auth();
            let token_a: Address = env.storage().instance().get(&AqKey::TokenA).unwrap();
            let token_b: Address = env.storage().instance().get(&AqKey::TokenB).unwrap();
            let token_in = if in_idx == 0 {
                token_a.clone()
            } else {
                token_b.clone()
            };
            let token_out = if out_idx == 0 { token_a } else { token_b };
            let amount = in_amount as i128;
            let pool = env.current_contract_address();
            token::Client::new(&env, &token_in).transfer(&user, &pool, &amount);
            token::Client::new(&env, &token_out).transfer(&pool, &user, &amount);
            in_amount
        }
    }
}

/// Aquarius-ABI pool that lies about its output: it returns `report` and only
/// transfers `deliver` of `token_out`, never pulling `token_in`. Models an
/// untrusted route pool an attacker points the router at to fake swap output.
mod malicious_aquarius_mock {
    use super::*;

    #[contract]
    pub struct MaliciousAqPool;

    #[contracttype]
    enum MalKey {
        TokenA,
        TokenB,
        Report,
        Deliver,
    }

    #[contractimpl]
    impl MaliciousAqPool {
        pub fn init(env: Env, token_a: Address, token_b: Address, report: u128, deliver: i128) {
            env.storage().instance().set(&MalKey::TokenA, &token_a);
            env.storage().instance().set(&MalKey::TokenB, &token_b);
            env.storage().instance().set(&MalKey::Report, &report);
            env.storage().instance().set(&MalKey::Deliver, &deliver);
        }

        pub fn get_tokens(env: Env) -> Vec<Address> {
            let token_a: Address = env.storage().instance().get(&MalKey::TokenA).unwrap();
            let token_b: Address = env.storage().instance().get(&MalKey::TokenB).unwrap();
            vec![&env, token_a, token_b]
        }

        pub fn swap(
            env: Env,
            user: Address,
            _in_idx: u32,
            out_idx: u32,
            _in_amount: u128,
            _out_min: u128,
        ) -> u128 {
            let token_a: Address = env.storage().instance().get(&MalKey::TokenA).unwrap();
            let token_b: Address = env.storage().instance().get(&MalKey::TokenB).unwrap();
            let token_out = if out_idx == 0 { token_a } else { token_b };
            let deliver: i128 = env.storage().instance().get(&MalKey::Deliver).unwrap();
            if deliver > 0 {
                let pool = env.current_contract_address();
                token::Client::new(&env, &token_out).transfer(&pool, &user, &deliver);
            }
            env.storage().instance().get(&MalKey::Report).unwrap()
        }
    }
}

mod sushi_mock {
    use super::*;

    #[contract]
    pub struct SushiPool;

    #[contracttype]
    enum SushiKey {
        Token0,
        Token1,
    }

    #[contractimpl]
    impl SushiPool {
        pub fn init(env: Env, token0: Address, token1: Address) {
            env.storage().instance().set(&SushiKey::Token0, &token0);
            env.storage().instance().set(&SushiKey::Token1, &token1);
        }

        pub fn token0(env: Env) -> Address {
            env.storage().instance().get(&SushiKey::Token0).unwrap()
        }

        pub fn token1(env: Env) -> Address {
            env.storage().instance().get(&SushiKey::Token1).unwrap()
        }

        pub fn get_oracle_hints(env: Env) -> Vec<i128> {
            vec![&env]
        }

        pub fn swap(
            env: Env,
            sender: Address,
            recipient: Address,
            zero_for_one: bool,
            amount_specified: i128,
            _sqrt_price_limit_x96: U256,
            _hints: Val,
        ) -> (i128, i128) {
            sender.require_auth();
            let token0: Address = env.storage().instance().get(&SushiKey::Token0).unwrap();
            let token1: Address = env.storage().instance().get(&SushiKey::Token1).unwrap();
            let token_in = if zero_for_one {
                token0.clone()
            } else {
                token1.clone()
            };
            let token_out = if zero_for_one { token1 } else { token0 };
            let pool = env.current_contract_address();
            token::Client::new(&env, &token_in).transfer(&sender, &pool, &amount_specified);
            token::Client::new(&env, &token_out).transfer(&pool, &recipient, &amount_specified);
            if zero_for_one {
                (amount_specified, -amount_specified)
            } else {
                (-amount_specified, amount_specified)
            }
        }
    }
}

mod comet_mock {
    use super::*;

    #[contract]
    pub struct CometPool;

    #[contractimpl]
    impl CometPool {
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
            token::Client::new(&env, &token_in).transfer_from(&pool, &user, &pool, &amount_in);
            token::Client::new(&env, &token_out).transfer(&pool, &user, &amount_in);
            (amount_in, 0)
        }
    }
}

#[test]
fn soroswap_single_hop_derives_output_from_live_reserves() {
    let env = Env::default();
    env.mock_all_auths();

    let router_addr = env.register(Router, (Address::generate(&env),));
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (asset_x, sac_x) = new_asset(&env, &admin);
    let (asset_y, sac_y) = new_asset(&env, &admin);
    // Soroswap's factory stores tokens canonically sorted (`token_0 < token_1`
    // under the host's address ordering). The router now derives the pair's
    // orientation from that invariant instead of reading `token_0`/`token_1`,
    // so the mock must be set up sorted too: smaller address = token_0 = the
    // input token for this hop.
    let ((token_in, sac_in), (token_out, sac_out)) = if asset_x < asset_y {
        ((asset_x, sac_x), (asset_y, sac_y))
    } else {
        ((asset_y, sac_y), (asset_x, sac_x))
    };

    // 2:1 reserves. The router's on-chain Soroswap math (0.3% ceil fee, floor
    // output) honors exactly 995 for amount_in = 500:
    //   fee     = ceil(500 * 3 / 1000)                 = 2
    //   in_less = 500 - 2                               = 498
    //   out     = 498 * 2_000_000 / (1_000_000 + 498)   = 995
    // which is the largest output the pair's k-invariant accepts for this input
    // (the mock asserts that invariant, so an over-sized request would panic).
    let reserve_0: i128 = 1_000_000;
    let reserve_1: i128 = 2_000_000;
    let reserve_derived_out: i128 = 995;
    let pool = env.register(soroswap_mock::SoroswapPair, ());
    soroswap_mock::SoroswapPairClient::new(&env, &pool)
        .init(&token_in, &token_out, &reserve_0, &reserve_1);

    sac_in.mint(&pool, &reserve_0);
    sac_out.mint(&pool, &reserve_1);
    sac_in.mint(&sender, &1_000);

    // `amount_out` carries a deliberately STALE quote (375): the pre-fix router
    // passed this straight to `pool.swap` as the exact output, so any reserve
    // drift between quote and execution tripped the pair's k-check
    // (`Error(Contract, #114)`). The fix derives the output from `get_reserves`
    // on-chain, so this stale value is ignored entirely.
    let stale_quoted_out: i128 = 375;
    // Slippage floor well under the live-reserve output, so the aggregate guard
    // is not what this test exercises — the output derivation is.
    let total_min_out: i128 = 900;

    let swap_xdr = strategy_xdr(
        &env,
        token_in.clone(),
        token_out.clone(),
        total_min_out,
        vec![
            &env,
            SwapPath {
                split_ppm: 1_000_000,
                hops: vec![
                    &env,
                    SwapHop {
                        venue: SwapVenue::Soroswap,
                        amount_out: stale_quoted_out,
                        pool,
                        token_in: token_in.clone(),
                        token_out: token_out.clone(),
                    },
                ],
            },
        ],
    );

    let out = RouterClient::new(&env, &router_addr).execute_strategy(&sender, &500, &swap_xdr);
    assert_eq!(out, reserve_derived_out);
    assert_ne!(out, stale_quoted_out);
    assert_eq!(token::Client::new(&env, &token_in).balance(&sender), 500);
    assert_eq!(
        token::Client::new(&env, &token_out).balance(&sender),
        reserve_derived_out
    );
}

#[test]
fn aquarius_single_hop_happy_path() {
    let env = Env::default();
    env.mock_all_auths();

    let router_addr = env.register(Router, (Address::generate(&env),));
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (token_a, sac_a) = new_asset(&env, &admin);
    let (token_b, sac_b) = new_asset(&env, &admin);
    let pool = env.register(aquarius_mock::AqPool, ());
    aquarius_mock::AqPoolClient::new(&env, &pool).init(&token_a, &token_b);

    sac_a.mint(&sender, &1_000);
    sac_b.mint(&pool, &1_000);

    let swap_xdr = strategy_xdr(
        &env,
        token_a.clone(),
        token_b.clone(),
        500,
        vec![
            &env,
            one_hop_path(
                &env,
                SwapVenue::Aquarius,
                pool,
                token_a.clone(),
                token_b.clone(),
                1_000_000,
            ),
        ],
    );

    let out = RouterClient::new(&env, &router_addr).execute_strategy(&sender, &500, &swap_xdr);
    assert_eq!(out, 500);
    assert_eq!(token::Client::new(&env, &token_a).balance(&sender), 500);
    assert_eq!(token::Client::new(&env, &token_b).balance(&sender), 500);
}

#[test]
fn execute_strategy_route_bytes_decode_and_execute() {
    let env = Env::default();
    env.mock_all_auths();

    let router_addr = env.register(Router, (Address::generate(&env),));
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (token_a, sac_a) = new_asset(&env, &admin);
    let (token_b, sac_b) = new_asset(&env, &admin);
    let pool = env.register(aquarius_mock::AqPool, ());
    aquarius_mock::AqPoolClient::new(&env, &pool).init(&token_a, &token_b);

    sac_a.mint(&sender, &1_000);
    sac_b.mint(&pool, &1_000);

    let swap_xdr = strategy_xdr(
        &env,
        token_a.clone(),
        token_b.clone(),
        500,
        vec![
            &env,
            one_hop_path(
                &env,
                SwapVenue::Aquarius,
                pool,
                token_a.clone(),
                token_b.clone(),
                1_000_000,
            ),
        ],
    );

    let out = RouterClient::new(&env, &router_addr).execute_strategy(&sender, &500, &swap_xdr);
    assert_eq!(out, 500);
    assert_eq!(token::Client::new(&env, &token_a).balance(&sender), 500);
    assert_eq!(token::Client::new(&env, &token_b).balance(&sender), 500);
}

// A route pool that reports output it never delivered must not let the caller
// drain the router's own `token_out` balance (e.g. accrued fees). The per-hop
// balance-delta check credits zero and reverts.
#[test]
fn execute_strategy_rejects_fake_venue_output() {
    let env = Env::default();
    env.mock_all_auths();

    let router_addr = env.register(Router, (Address::generate(&env),));
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (token_a, sac_a) = new_asset(&env, &admin);
    let (token_b, sac_b) = new_asset(&env, &admin);

    // Pool claims 700 out but transfers nothing.
    let pool = env.register(malicious_aquarius_mock::MaliciousAqPool, ());
    malicious_aquarius_mock::MaliciousAqPoolClient::new(&env, &pool)
        .init(&token_a, &token_b, &700u128, &0i128);

    // Attacker holds 1 token_a; the router holds 700 token_b of accrued fees.
    sac_a.mint(&sender, &1);
    sac_b.mint(&router_addr, &700);

    let swap_xdr = strategy_xdr(
        &env,
        token_a.clone(),
        token_b.clone(),
        700,
        vec![
            &env,
            one_hop_path(
                &env,
                SwapVenue::Aquarius,
                pool,
                token_a.clone(),
                token_b.clone(),
                1_000_000,
            ),
        ],
    );

    let err = RouterClient::new(&env, &router_addr)
        .try_execute_strategy(&sender, &1, &swap_xdr)
        .unwrap_err();
    assert_eq!(err.unwrap(), Error::ZeroOutput.into());
    // Router fees untouched, attacker gained nothing.
    assert_eq!(token::Client::new(&env, &token_b).balance(&sender), 0);
    assert_eq!(
        token::Client::new(&env, &token_b).balance(&router_addr),
        700
    );
}

// When a pool over-reports, the router credits only what actually arrived.
#[test]
fn execute_strategy_credits_only_delivered_output_not_reported() {
    let env = Env::default();
    env.mock_all_auths();

    let router_addr = env.register(Router, (Address::generate(&env),));
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (token_a, sac_a) = new_asset(&env, &admin);
    let (token_b, sac_b) = new_asset(&env, &admin);

    // Pool reports 700 but only delivers 500.
    let pool = env.register(malicious_aquarius_mock::MaliciousAqPool, ());
    malicious_aquarius_mock::MaliciousAqPoolClient::new(&env, &pool)
        .init(&token_a, &token_b, &700u128, &500i128);
    sac_b.mint(&pool, &500);
    sac_a.mint(&sender, &1);

    let swap_xdr = strategy_xdr(
        &env,
        token_a.clone(),
        token_b.clone(),
        500,
        vec![
            &env,
            one_hop_path(
                &env,
                SwapVenue::Aquarius,
                pool,
                token_a.clone(),
                token_b.clone(),
                1_000_000,
            ),
        ],
    );

    let out = RouterClient::new(&env, &router_addr).execute_strategy(&sender, &1, &swap_xdr);
    // Credited the real 500 delivered, not the 700 claimed.
    assert_eq!(out, 500);
    assert_eq!(token::Client::new(&env, &token_b).balance(&sender), 500);
    assert_eq!(token::Client::new(&env, &token_b).balance(&router_addr), 0);
}

#[test]
fn execute_strategy_rejects_wrong_token_in_endpoint() {
    let env = Env::default();
    env.mock_all_auths();

    let router_addr = env.register(Router, (Address::generate(&env),));
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (token_a, sac_a) = new_asset(&env, &admin);
    let (token_b, _) = new_asset(&env, &admin);
    let pool = env.register(aquarius_mock::AqPool, ());

    sac_a.mint(&sender, &1_000);

    let swap_xdr = strategy_xdr(
        &env,
        token_b.clone(),
        token_b.clone(),
        1,
        vec![
            &env,
            one_hop_path(
                &env,
                SwapVenue::Aquarius,
                pool,
                token_a.clone(),
                token_b.clone(),
                1_000_000,
            ),
        ],
    );

    let err = RouterClient::new(&env, &router_addr)
        .try_execute_strategy(&sender, &500, &swap_xdr)
        .unwrap_err();
    assert_eq!(err.unwrap(), Error::BrokenTokenChain.into());
    assert_eq!(token::Client::new(&env, &token_a).balance(&sender), 1_000);
}

#[test]
fn sushi_single_hop_happy_path() {
    let env = Env::default();
    env.mock_all_auths();

    let router_addr = env.register(Router, (Address::generate(&env),));
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (token_a, sac_a) = new_asset(&env, &admin);
    let (token_b, sac_b) = new_asset(&env, &admin);
    let pool = env.register(sushi_mock::SushiPool, ());
    sushi_mock::SushiPoolClient::new(&env, &pool).init(&token_a, &token_b);

    sac_a.mint(&sender, &1_000);
    sac_b.mint(&pool, &1_000);

    let swap_xdr = strategy_xdr(
        &env,
        token_a.clone(),
        token_b.clone(),
        300,
        vec![
            &env,
            one_hop_path(
                &env,
                SwapVenue::Sushi,
                pool,
                token_a.clone(),
                token_b.clone(),
                1_000_000,
            ),
        ],
    );

    let out = RouterClient::new(&env, &router_addr).execute_strategy(&sender, &300, &swap_xdr);
    assert_eq!(out, 300);
    assert_eq!(token::Client::new(&env, &token_a).balance(&sender), 700);
    assert_eq!(token::Client::new(&env, &token_b).balance(&sender), 300);
}

#[test]
fn comet_single_hop_happy_path() {
    let env = Env::default();
    env.mock_all_auths();

    let router_addr = env.register(Router, (Address::generate(&env),));
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (token_a, sac_a) = new_asset(&env, &admin);
    let (token_b, sac_b) = new_asset(&env, &admin);
    let pool = env.register(comet_mock::CometPool, ());

    sac_a.mint(&sender, &1_000);
    sac_b.mint(&pool, &1_000);

    let swap_xdr = strategy_xdr(
        &env,
        token_a.clone(),
        token_b.clone(),
        250,
        vec![
            &env,
            one_hop_path(
                &env,
                SwapVenue::CometDex,
                pool,
                token_a.clone(),
                token_b.clone(),
                1_000_000,
            ),
        ],
    );

    let out = RouterClient::new(&env, &router_addr).execute_strategy(&sender, &250, &swap_xdr);
    assert_eq!(out, 250);
    assert_eq!(token::Client::new(&env, &token_a).balance(&sender), 750);
    assert_eq!(token::Client::new(&env, &token_b).balance(&sender), 250);
}

#[test]
fn execute_strategy_errors_on_empty_payload() {
    let env = Env::default();
    env.mock_all_auths();
    let router_addr = env.register(Router, (Address::generate(&env),));
    let sender = Address::generate(&env);
    let token_a = Address::generate(&env);
    let token_b = Address::generate(&env);
    let swap_xdr = strategy_xdr(&env, token_a, token_b, 1, vec![&env]);
    let err = RouterClient::new(&env, &router_addr)
        .try_execute_strategy(&sender, &1, &swap_xdr)
        .unwrap_err();
    assert_eq!(err.unwrap(), Error::EmptyBatch.into());
}

#[test]
fn execute_strategy_errors_on_aggregate_slippage() {
    let env = Env::default();
    env.mock_all_auths();

    let router_addr = env.register(Router, (Address::generate(&env),));
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (token_a, sac_a) = new_asset(&env, &admin);
    let (token_b, sac_b) = new_asset(&env, &admin);
    let pool = env.register(aquarius_mock::AqPool, ());
    aquarius_mock::AqPoolClient::new(&env, &pool).init(&token_a, &token_b);

    sac_a.mint(&sender, &1_000);
    sac_b.mint(&pool, &1_000);

    let swap_xdr = strategy_xdr(
        &env,
        token_a.clone(),
        token_b.clone(),
        1_000,
        vec![
            &env,
            one_hop_path(&env, SwapVenue::Aquarius, pool, token_a, token_b, 1_000_000),
        ],
    );
    let err = RouterClient::new(&env, &router_addr)
        .try_execute_strategy(&sender, &100, &swap_xdr)
        .unwrap_err();
    assert_eq!(err.unwrap(), Error::SlippageExceeded.into());
}

#[test]
fn execute_strategy_errors_on_broken_token_chain() {
    let env = Env::default();
    env.mock_all_auths();
    let router_addr = env.register(Router, (Address::generate(&env),));
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (token_a, sac_a) = new_asset(&env, &admin);
    let (token_b, _) = new_asset(&env, &admin);
    let pool = env.register(aquarius_mock::AqPool, ());
    sac_a.mint(&sender, &1_000);

    let hops = vec![
        &env,
        SwapHop {
            venue: SwapVenue::Aquarius,
            amount_out: 0,
            pool: pool.clone(),
            token_in: token_a.clone(),
            token_out: token_a.clone(),
        },
        SwapHop {
            venue: SwapVenue::Aquarius,
            amount_out: 0,
            pool,
            token_in: token_b.clone(),
            token_out: token_b.clone(),
        },
    ];
    let swap_xdr = strategy_xdr(
        &env,
        token_a,
        token_b.clone(),
        1,
        vec![
            &env,
            SwapPath {
                split_ppm: 1_000_000,
                hops,
            },
        ],
    );
    let err = RouterClient::new(&env, &router_addr)
        .try_execute_strategy(&sender, &100, &swap_xdr)
        .unwrap_err();
    assert_eq!(err.unwrap(), Error::BrokenTokenChain.into());
}

#[test]
fn execute_strategy_rejects_same_token_in_and_out() {
    let env = Env::default();
    env.mock_all_auths();
    let router_addr = env.register(Router, (Address::generate(&env),));
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (token_a, sac_a) = new_asset(&env, &admin);
    let pool = env.register(aquarius_mock::AqPool, ());
    sac_a.mint(&sender, &1_000);

    let swap_xdr = strategy_xdr(
        &env,
        token_a.clone(),
        token_a.clone(),
        1,
        vec![
            &env,
            one_hop_path(
                &env,
                SwapVenue::Aquarius,
                pool,
                token_a.clone(),
                token_a,
                1_000_000,
            ),
        ],
    );
    let err = RouterClient::new(&env, &router_addr)
        .try_execute_strategy(&sender, &100, &swap_xdr)
        .unwrap_err();
    assert_eq!(err.unwrap(), Error::SameToken.into());
}

#[test]
fn split_ppm_must_sum_to_one_million() {
    let env = Env::default();
    env.mock_all_auths();
    let router_addr = env.register(Router, (Address::generate(&env),));
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (token_a, sac_a) = new_asset(&env, &admin);
    let (token_b, _) = new_asset(&env, &admin);
    let pool = env.register(aquarius_mock::AqPool, ());
    sac_a.mint(&sender, &1_000);

    let swap_xdr = strategy_xdr(
        &env,
        token_a.clone(),
        token_b.clone(),
        1,
        vec![
            &env,
            one_hop_path(
                &env,
                SwapVenue::Aquarius,
                pool.clone(),
                token_a.clone(),
                token_b.clone(),
                600_000,
            ),
            one_hop_path(&env, SwapVenue::Aquarius, pool, token_a, token_b, 200_000),
        ],
    );
    let err = RouterClient::new(&env, &router_addr)
        .try_execute_strategy(&sender, &100, &swap_xdr)
        .unwrap_err();
    assert_eq!(err.unwrap(), Error::SplitPpmMismatch.into());
}

#[test]
fn split_ppm_zero_path_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let router_addr = env.register(Router, (Address::generate(&env),));
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (token_a, sac_a) = new_asset(&env, &admin);
    let (token_b, _) = new_asset(&env, &admin);
    let pool = env.register(aquarius_mock::AqPool, ());
    sac_a.mint(&sender, &1_000);

    let swap_xdr = strategy_xdr(
        &env,
        token_a.clone(),
        token_b.clone(),
        1,
        vec![
            &env,
            one_hop_path(
                &env,
                SwapVenue::Aquarius,
                pool.clone(),
                token_a.clone(),
                token_b.clone(),
                1_000_000,
            ),
            one_hop_path(&env, SwapVenue::Aquarius, pool, token_a, token_b, 0),
        ],
    );
    let err = RouterClient::new(&env, &router_addr)
        .try_execute_strategy(&sender, &100, &swap_xdr)
        .unwrap_err();
    assert_eq!(err.unwrap(), Error::ZeroSplitPpm.into());
}

#[test]
fn two_path_split_consumes_full_total_in_with_rounding_absorbed() {
    let env = Env::default();
    env.mock_all_auths();
    let router_addr = env.register(Router, (Address::generate(&env),));
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (token_a, sac_a) = new_asset(&env, &admin);
    let (token_b, sac_b) = new_asset(&env, &admin);
    let pool = env.register(aquarius_mock::AqPool, ());
    aquarius_mock::AqPoolClient::new(&env, &pool).init(&token_a, &token_b);
    sac_a.mint(&sender, &1_000);
    sac_b.mint(&pool, &1_000);

    let swap_xdr = strategy_xdr(
        &env,
        token_a.clone(),
        token_b.clone(),
        7,
        vec![
            &env,
            one_hop_path(
                &env,
                SwapVenue::Aquarius,
                pool.clone(),
                token_a.clone(),
                token_b.clone(),
                333_333,
            ),
            one_hop_path(
                &env,
                SwapVenue::Aquarius,
                pool,
                token_a,
                token_b.clone(),
                666_667,
            ),
        ],
    );
    let out = RouterClient::new(&env, &router_addr).execute_strategy(&sender, &7, &swap_xdr);
    assert_eq!(out, 7, "last path must absorb PPM rounding");
    assert_eq!(token::Client::new(&env, &token_b).balance(&sender), 7);
}

#[test]
fn sweep_balance_recovers_stray_tokens_to_recipient() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let router_addr = env.register(Router, (admin.clone(),));
    let asset_admin = Address::generate(&env);
    let (stray_token, sac_stray) = new_asset(&env, &asset_admin);
    let (untouched_token, sac_untouched) = new_asset(&env, &asset_admin);
    let recipient = Address::generate(&env);

    // Simulate dust: a direct transfer to the router outside `execute_strategy`.
    sac_stray.mint(&router_addr, &1_234);
    sac_untouched.mint(&router_addr, &500);

    RouterClient::new(&env, &router_addr)
        .sweep_balance(&recipient, &vec![&env, stray_token.clone()]);

    assert_eq!(
        token::Client::new(&env, &stray_token).balance(&router_addr),
        0
    );
    assert_eq!(
        token::Client::new(&env, &stray_token).balance(&recipient),
        1_234
    );
    // Tokens not passed in `tokens` are left alone.
    assert_eq!(
        token::Client::new(&env, &untouched_token).balance(&router_addr),
        500
    );
}
