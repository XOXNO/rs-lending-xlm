//! Mock contracts for router unit tests.
//!
//! Each mock is a minimal on-chain stand-in for a venue (or a hostile variant)
//! so tests can exercise adapter dispatch without deploying real AMMs.

use soroban_sdk::{
    contract, contractimpl, contracttype, token, vec, Address, Env, Val, Vec, U256,
};

/// Uniswap-v2-style pair: live reserves + k-invariant `swap`.
pub mod soroswap_mock {
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

/// Happy-path Aquarius pool: 1:1 swap after pulling input.
pub mod aquarius_mock {
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

/// Aquarius-ABI pool with configurable report / delivery / input pull.
/// Used to prove the router trusts balance deltas, not pool return values.
pub mod malicious_aquarius_mock {
    use super::*;


    #[contract]
    pub struct MaliciousAqPool;

    #[contracttype]
    enum MalKey {
        TokenA,
        TokenB,
        Report,
        Deliver,
        PullInput,
    }

    #[contractimpl]
    impl MaliciousAqPool {
        pub fn init(env: Env, token_a: Address, token_b: Address, report: u128, deliver: i128) {
            Self::init_with_pull(env, token_a, token_b, report, deliver, false);
        }

        pub fn init_with_pull(
            env: Env,
            token_a: Address,
            token_b: Address,
            report: u128,
            deliver: i128,
            pull_input: bool,
        ) {
            env.storage().instance().set(&MalKey::TokenA, &token_a);
            env.storage().instance().set(&MalKey::TokenB, &token_b);
            env.storage().instance().set(&MalKey::Report, &report);
            env.storage().instance().set(&MalKey::Deliver, &deliver);
            env.storage()
                .instance()
                .set(&MalKey::PullInput, &pull_input);
        }

        pub fn get_tokens(env: Env) -> Vec<Address> {
            let token_a: Address = env.storage().instance().get(&MalKey::TokenA).unwrap();
            let token_b: Address = env.storage().instance().get(&MalKey::TokenB).unwrap();
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
            let token_a: Address = env.storage().instance().get(&MalKey::TokenA).unwrap();
            let token_b: Address = env.storage().instance().get(&MalKey::TokenB).unwrap();
            let token_in = if in_idx == 0 {
                token_a.clone()
            } else {
                token_b.clone()
            };
            let token_out = if out_idx == 0 { token_a } else { token_b };
            let pool = env.current_contract_address();
            if env
                .storage()
                .instance()
                .get(&MalKey::PullInput)
                .unwrap_or(false)
            {
                token::Client::new(&env, &token_in).transfer(&user, &pool, &(in_amount as i128));
            }
            let deliver: i128 = env.storage().instance().get(&MalKey::Deliver).unwrap();
            if deliver > 0 {
                token::Client::new(&env, &token_out).transfer(&pool, &user, &deliver);
            }
            env.storage().instance().get(&MalKey::Report).unwrap()
        }
    }
}

/// Sushi CL-style pool: direction from token0/token1 pair.
pub mod sushi_mock {
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

/// Comet pull-via-allowance pool, plus a no-pull variant.
pub mod comet_mock {
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

    #[contract]
    pub struct NoPullCometPool;

    #[contractimpl]
    impl NoPullCometPool {
        pub fn swap_exact_amount_in(
            env: Env,
            _token_in: Address,
            amount_in: i128,
            token_out: Address,
            _min_out: i128,
            _max_price: i128,
            user: Address,
        ) -> (i128, i128) {
            let pool = env.current_contract_address();
            token::Client::new(&env, &token_out).transfer(&pool, &user, &amount_in);
            (amount_in, 0)
        }
    }
}

/// Phoenix pool: offer-asset swap mirror of the live adapter ABI.
pub mod phoenix_mock {
    use super::*;

    #[contract]
    pub struct PhoenixPool;
    #[contracttype]
    enum PhKey {
        TokenA,
        TokenB,
    }
    #[contractimpl]
    impl PhoenixPool {
        pub fn init(env: Env, token_a: Address, token_b: Address) {
            env.storage().instance().set(&PhKey::TokenA, &token_a);
            env.storage().instance().set(&PhKey::TokenB, &token_b);
        }
        // Adapter calls: swap(router, token_in, amount_in, None, None, None, None) -> i128
        #[allow(clippy::too_many_arguments)] // mirrors the real Phoenix pool ABI
        pub fn swap(
            env: Env,
            sender: Address,
            offer_asset: Address,
            offer_amount: i128,
            _ask_min: Option<i128>,
            _belief_price: Option<i64>,
            _max_spread: Option<u64>,
            _deadline: Option<i64>,
        ) -> i128 {
            let token_a: Address = env.storage().instance().get(&PhKey::TokenA).unwrap();
            let token_b: Address = env.storage().instance().get(&PhKey::TokenB).unwrap();
            let token_out = if offer_asset == token_a {
                token_b
            } else {
                token_a
            };
            let pool = env.current_contract_address();
            token::Client::new(&env, &offer_asset).transfer(&sender, &pool, &offer_amount);
            token::Client::new(&env, &token_out).transfer(&pool, &sender, &offer_amount);
            offer_amount
        }
    }
}

/// Comet pool that pulls input but reports zero output.
pub mod comet_zero_mock {
    use super::*;

    #[contract]
    pub struct ZeroOutComet;
    #[contractimpl]
    impl ZeroOutComet {
        pub fn swap_exact_amount_in(
            env: Env,
            token_in: Address,
            amount_in: i128,
            _token_out: Address,
            _min_out: i128,
            _max_price: i128,
            user: Address,
        ) -> (i128, i128) {
            let pool = env.current_contract_address();
            token::Client::new(&env, &token_in).transfer_from(&pool, &user, &pool, &amount_in);
            (0, 0)
        }
    }
}

/// Token that panics on any `transfer` — proves a path performs no transfer.
pub mod no_transfer_token_mock {
    use super::*;


    #[contract]
    pub struct NoTransferToken;

    #[contracttype]
    enum Key {
        Balance,
    }

    #[contractimpl]
    impl NoTransferToken {
        pub fn init(env: Env, balance: i128) {
            env.storage().instance().set(&Key::Balance, &balance);
        }

        pub fn balance(env: Env, _id: Address) -> i128 {
            env.storage().instance().get(&Key::Balance).unwrap_or(0)
        }

        pub fn transfer(_env: Env, _from: Address, _to: Address, _amount: i128) {
            panic!("transfer must not be called");
        }
    }
}

/// SEP-41-shaped token whose allowance does not auto-decrement on
/// `transfer_from` (infinite-approval style).
pub mod sticky_allowance_token_mock {
    use super::*;


    #[contract]
    pub struct StickyAllowanceToken;

    #[contracttype]
    enum Key {
        Bal(Address),
        Allow(Address, Address),
    }

    #[contractimpl]
    impl StickyAllowanceToken {
        pub fn mint(env: Env, to: Address, amount: i128) {
            env.storage().instance().set(&Key::Bal(to), &amount);
        }

        pub fn balance(env: Env, id: Address) -> i128 {
            env.storage().instance().get(&Key::Bal(id)).unwrap_or(0)
        }

        pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
            let from_bal = Self::balance(env.clone(), from.clone()) - amount;
            let to_bal = Self::balance(env.clone(), to.clone()) + amount;
            env.storage().instance().set(&Key::Bal(from), &from_bal);
            env.storage().instance().set(&Key::Bal(to), &to_bal);
        }

        pub fn approve(
            env: Env,
            from: Address,
            spender: Address,
            amount: i128,
            _expiration_ledger: u32,
        ) {
            env.storage()
                .instance()
                .set(&Key::Allow(from, spender), &amount);
        }

        pub fn allowance(env: Env, from: Address, spender: Address) -> i128 {
            env.storage()
                .instance()
                .get(&Key::Allow(from, spender))
                .unwrap_or(0)
        }

        pub fn transfer_from(
            env: Env,
            _spender: Address,
            from: Address,
            to: Address,
            amount: i128,
        ) {
            Self::transfer(env, from, to, amount);
        }
    }
}

