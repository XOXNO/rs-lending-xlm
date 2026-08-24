use soroban_sdk::{
    contract, contractimpl, contracttype, token, vec, Address, Env, Symbol, Val, Vec, U256,
};

pub mod soroswap_mock {
    use super::*;

    /// Widens one side of the constant-product comparison. A negative value
    /// would mean the pair was drained past its own reserves, so clamp it to
    /// zero and let the invariant assertion fail rather than wrap.
    fn k_term(env: &Env, value: i128) -> U256 {
        U256::from_u128(env, value.max(0) as u128)
    }

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
            // An 18-decimal pair holds reserves around 1e24, and the scaled
            // product of two of those is ~1e55 -- far past `i128`. Compare the
            // invariant in 256-bit space so the mock stays usable at the scales
            // the adapter has to quote.
            let k_after = k_term(&env, balance0_adjusted).mul(&k_term(&env, balance1_adjusted));
            let k_before = k_term(&env, reserve0)
                .mul(&k_term(&env, reserve1))
                .mul(&U256::from_u32(&env, 1_000_000));
            assert!(k_after >= k_before, "soroswap k-invariant violated");

            env.storage()
                .instance()
                .set(&SoroswapKey::Reserve0, &balance0);
            env.storage()
                .instance()
                .set(&SoroswapKey::Reserve1, &balance1);
        }
    }
}

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

pub mod aquarius_lp_mock {
    use super::*;

    #[contract]
    pub struct AqLpPool;

    #[contracttype]
    enum LpKey {
        TokenA,
        TokenB,
        Share,
        TotalShares,
        Stable,
        /// Units withheld from constituent 0 on withdraw, and a signal to skip
        /// the pool's own min_amounts assertion. Models a venue that does not
        /// honour the minimums it was handed. Absent by default, so every
        /// existing test sees the honest pool.
        Shortfall,
    }

    #[contractimpl]
    impl AqLpPool {
        pub fn init(env: Env, token_a: Address, token_b: Address, share: Address) {
            env.storage().instance().set(&LpKey::TokenA, &token_a);
            env.storage().instance().set(&LpKey::TokenB, &token_b);
            env.storage().instance().set(&LpKey::Share, &share);
            env.storage().instance().set(&LpKey::TotalShares, &0i128);
        }

        pub fn get_tokens(env: Env) -> Vec<Address> {
            let token_a: Address = env.storage().instance().get(&LpKey::TokenA).unwrap();
            let token_b: Address = env.storage().instance().get(&LpKey::TokenB).unwrap();
            vec![&env, token_a, token_b]
        }

        pub fn share_id(env: Env) -> Address {
            env.storage().instance().get(&LpKey::Share).unwrap()
        }

        pub fn get_total_shares(env: Env) -> u128 {
            let total: i128 = env.storage().instance().get(&LpKey::TotalShares).unwrap();
            total as u128
        }

        pub fn set_shortfall(env: Env, amount: i128) {
            env.storage().instance().set(&LpKey::Shortfall, &amount);
        }

        pub fn set_stable(env: Env) {
            env.storage().instance().set(&LpKey::Stable, &true);
        }

        pub fn pool_type(env: Env) -> Symbol {
            if env
                .storage()
                .instance()
                .get(&LpKey::Stable)
                .unwrap_or(false)
            {
                Symbol::new(&env, "stable")
            } else {
                Symbol::new(&env, "constant_product")
            }
        }

        pub fn get_reserves(env: Env) -> Vec<u128> {
            let pool = env.current_contract_address();
            let tokens = Self::get_tokens(env.clone());
            let r0 = token::Client::new(&env, &tokens.get(0).unwrap()).balance(&pool);
            let r1 = token::Client::new(&env, &tokens.get(1).unwrap()).balance(&pool);
            vec![&env, r0 as u128, r1 as u128]
        }

        pub fn get_fee_fraction(_env: Env) -> u32 {
            30
        }

        pub fn swap(
            env: Env,
            user: Address,
            in_idx: u32,
            out_idx: u32,
            in_amount: u128,
            out_min: u128,
        ) -> u128 {
            user.require_auth();
            let pool = env.current_contract_address();
            let tokens = Self::get_tokens(env.clone());
            let token_in = tokens.get(in_idx).unwrap();
            let token_out = tokens.get(out_idx).unwrap();
            let r_in = token::Client::new(&env, &token_in).balance(&pool);
            let r_out = token::Client::new(&env, &token_out).balance(&pool);
            let amount = in_amount as i128;
            let net = amount * 9_970 / 10_000;
            let out = net * r_out / (r_in + net);
            assert!(out as u128 >= out_min, "out_min not met");
            token::Client::new(&env, &token_in).transfer(&user, &pool, &amount);
            token::Client::new(&env, &token_out).transfer(&pool, &user, &out);
            out as u128
        }

        pub fn deposit(
            env: Env,
            user: Address,
            desired_amounts: Vec<u128>,
            min_shares: u128,
        ) -> (Vec<u128>, u128) {
            user.require_auth();
            let pool = env.current_contract_address();
            let tokens = Self::get_tokens(env.clone());
            let share: Address = env.storage().instance().get(&LpKey::Share).unwrap();
            let total: i128 = env.storage().instance().get(&LpKey::TotalShares).unwrap();

            let d0 = desired_amounts.get(0).unwrap() as i128;
            let d1 = desired_amounts.get(1).unwrap() as i128;
            let r0 = token::Client::new(&env, &tokens.get(0).unwrap()).balance(&pool);
            let r1 = token::Client::new(&env, &tokens.get(1).unwrap()).balance(&pool);

            let stable: bool = env
                .storage()
                .instance()
                .get(&LpKey::Stable)
                .unwrap_or(false);
            let (used0, used1, shares) = if stable {
                let s = if r0 + r1 > 0 {
                    (d0 + d1) * total / (r0 + r1)
                } else {
                    d0 + d1
                };
                (d0, d1, s)
            } else if total == 0 || r0 == 0 || r1 == 0 {
                let s = if d0 < d1 { d0 } else { d1 };
                (d0, d1, s)
            } else {
                let (u0, u1) = if d0 * r1 <= d1 * r0 {
                    (d0, d0 * r1 / r0)
                } else {
                    (d1 * r0 / r1, d1)
                };
                let by0 = u0 * total / r0;
                let by1 = u1 * total / r1;
                let s = if by0 < by1 { by0 } else { by1 };
                (u0, u1, s)
            };
            assert!(shares as u128 >= min_shares, "min_shares not met");

            for (i, (desired, used)) in [(d0, used0), (d1, used1)].iter().enumerate() {
                let client = token::Client::new(&env, &tokens.get(i as u32).unwrap());
                if *desired > 0 {
                    client.transfer(&user, &pool, desired);
                }
                let refund = desired - used;
                if refund > 0 {
                    client.transfer(&pool, &user, &refund);
                }
            }
            token::StellarAssetClient::new(&env, &share).mint(&user, &shares);
            env.storage()
                .instance()
                .set(&LpKey::TotalShares, &(total + shares));

            (vec![&env, used0 as u128, used1 as u128], shares as u128)
        }

        pub fn withdraw(
            env: Env,
            user: Address,
            share_amount: u128,
            min_amounts: Vec<u128>,
        ) -> Vec<u128> {
            let pool = env.current_contract_address();
            let tokens = Self::get_tokens(env.clone());
            let share: Address = env.storage().instance().get(&LpKey::Share).unwrap();
            let total: i128 = env.storage().instance().get(&LpKey::TotalShares).unwrap();
            let amount = share_amount as i128;

            let r0 = token::Client::new(&env, &tokens.get(0).unwrap()).balance(&pool);
            let r1 = token::Client::new(&env, &tokens.get(1).unwrap()).balance(&pool);
            let shortfall: i128 = env.storage().instance().get(&LpKey::Shortfall).unwrap_or(0);
            let out0 = r0 * amount / total - shortfall;
            let out1 = r1 * amount / total;
            if shortfall == 0 {
                assert!(
                    out0 as u128 >= min_amounts.get(0).unwrap(),
                    "min_amounts[0]"
                );
                assert!(
                    out1 as u128 >= min_amounts.get(1).unwrap(),
                    "min_amounts[1]"
                );
            }

            token::Client::new(&env, &share).burn(&user, &amount);
            token::Client::new(&env, &tokens.get(0).unwrap()).transfer(&pool, &user, &out0);
            token::Client::new(&env, &tokens.get(1).unwrap()).transfer(&pool, &user, &out1);
            env.storage()
                .instance()
                .set(&LpKey::TotalShares, &(total - amount));

            vec![&env, out0 as u128, out1 as u128]
        }
    }
}

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

        #[allow(clippy::too_many_arguments)]
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

/// A token whose `transfer` delivers `amount` minus a basis-point haircut to
/// the recipient while debiting the sender the full `amount`.
///
/// Models the fee-on-transfer / burn-on-transfer contracts that satisfy the
/// SEP-41 interface but break the assumption that a declared transfer amount
/// equals the amount received. Not a SAC.
pub mod fee_on_transfer_token_mock {
    use super::*;

    #[contract]
    pub struct FotToken;

    #[contracttype]
    enum FotKey {
        Balance(Address),
        Allowance(Address, Address),
        FeeBps,
    }

    #[contractimpl]
    impl FotToken {
        /// Sets the transfer haircut in basis points.
        pub fn init(env: Env, fee_bps: i128) {
            env.storage().instance().set(&FotKey::FeeBps, &fee_bps);
        }

        pub fn mint(env: Env, to: Address, amount: i128) {
            let balance = Self::balance(env.clone(), to.clone());
            env.storage()
                .instance()
                .set(&FotKey::Balance(to), &(balance + amount));
        }

        pub fn balance(env: Env, id: Address) -> i128 {
            env.storage()
                .instance()
                .get(&FotKey::Balance(id))
                .unwrap_or(0)
        }

        pub fn decimals(_env: Env) -> u32 {
            7
        }

        /// Debits `from` by `amount`, credits `to` with `amount - fee`. The
        /// difference is burned.
        pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
            from.require_auth();
            let fee_bps: i128 = env.storage().instance().get(&FotKey::FeeBps).unwrap_or(0);
            let fee = amount * fee_bps / 10_000;
            let from_balance = Self::balance(env.clone(), from.clone());
            assert!(from_balance >= amount, "fot: insufficient balance");
            let to_balance = Self::balance(env.clone(), to.clone());
            env.storage()
                .instance()
                .set(&FotKey::Balance(from), &(from_balance - amount));
            env.storage()
                .instance()
                .set(&FotKey::Balance(to), &(to_balance + amount - fee));
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
                .set(&FotKey::Allowance(from, spender), &amount);
        }

        pub fn allowance(env: Env, from: Address, spender: Address) -> i128 {
            env.storage()
                .instance()
                .get(&FotKey::Allowance(from, spender))
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
