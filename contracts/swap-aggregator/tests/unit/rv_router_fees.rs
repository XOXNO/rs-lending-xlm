//! RV break pass: router fee policy and economics.
//!
//! Each probe states the documented-policy number as an assertion. A failure is
//! a finding; a pass pins the behaviour.

use soroban_sdk::testutils::Address as _;
use soroban_sdk::{contract, contractimpl, contracttype, token, vec, Address, Env, Vec};

use crate::errors::Error;
use crate::reserved_fee_balance;
use crate::types::{SwapHop, SwapVenue};
use crate::{Router, RouterClient};

use super::support::{aquarius_mock, new_asset, one_hop_path, path, strategy_xdr_with_referral};

/// Aquarius-shaped pool that keeps every unit it is handed and pays back
/// exactly one unit of its other token. Models a route tail the sender owns.
mod one_unit_pool {
    use super::*;

    #[contract]
    pub struct OneUnitPool;

    #[contracttype]
    enum Key {
        TokenA,
        TokenB,
    }

    #[contractimpl]
    impl OneUnitPool {
        pub fn init(env: Env, token_a: Address, token_b: Address) {
            env.storage().instance().set(&Key::TokenA, &token_a);
            env.storage().instance().set(&Key::TokenB, &token_b);
        }

        pub fn get_tokens(env: Env) -> Vec<Address> {
            let a: Address = env.storage().instance().get(&Key::TokenA).unwrap();
            let b: Address = env.storage().instance().get(&Key::TokenB).unwrap();
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
            user.require_auth();
            let tokens = Self::get_tokens(env.clone());
            let token_in = tokens.get_unchecked(in_idx);
            let token_out = tokens.get_unchecked(out_idx);
            let pool = env.current_contract_address();
            token::Client::new(&env, &token_in).transfer(&user, &pool, &(in_amount as i128));
            token::Client::new(&env, &token_out).transfer(&pool, &user, &1);
            1
        }
    }
}

const IN: i128 = 1_000_000;

/// R-6: with the output token whitelisted and the input not, the fee base is the
/// final output balance. A sender-owned tail hop that keeps the real fill and
/// returns one unit shrinks that base to 1, so both buckets round to zero while
/// the sender's pool holds the whole intermediate fill. This is the documented
/// "fee on the whitelisted side" policy; fees are opt-in anyway (`referral_id`
/// is the sender's own payload byte), so it is pinned, not reported.
#[test]
fn whitelisted_output_fee_base_is_the_final_balance_only() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let router_addr = env.register(Router, (admin.clone(),));
    let router = RouterClient::new(&env, &router_addr);
    let sender = Address::generate(&env);
    let asset_admin = Address::generate(&env);
    let (a, sac_a) = new_asset(&env, &asset_admin);
    let (b, sac_b) = new_asset(&env, &asset_admin);
    let (c, sac_c) = new_asset(&env, &asset_admin);

    let honest = env.register(aquarius_mock::AqPool, ());
    aquarius_mock::AqPoolClient::new(&env, &honest).init(&a, &b);
    let tail = env.register(one_unit_pool::OneUnitPool, ());
    one_unit_pool::OneUnitPoolClient::new(&env, &tail).init(&b, &c);

    sac_a.mint(&sender, &IN);
    sac_b.mint(&honest, &IN);
    sac_c.mint(&tail, &10);

    router.set_static_fee(&100);
    let id = router.add_referral(&Address::generate(&env), &100);
    router.add_to_whitelist(&c);

    // Reference: A -> B direct with B whitelisted charges 2% of the 1:1 fill.
    {
        let env2 = Env::default();
        env2.mock_all_auths();
        let admin2 = Address::generate(&env2);
        let r2_addr = env2.register(Router, (admin2,));
        let r2 = RouterClient::new(&env2, &r2_addr);
        let s2 = Address::generate(&env2);
        let aa = Address::generate(&env2);
        let (a2, sa2) = new_asset(&env2, &aa);
        let (b2, sb2) = new_asset(&env2, &aa);
        let p2 = env2.register(aquarius_mock::AqPool, ());
        aquarius_mock::AqPoolClient::new(&env2, &p2).init(&a2, &b2);
        sa2.mint(&s2, &IN);
        sb2.mint(&p2, &IN);
        r2.set_static_fee(&100);
        let id2 = r2.add_referral(&Address::generate(&env2), &100);
        r2.add_to_whitelist(&b2);
        let xdr = strategy_xdr_with_referral(
            &env2,
            a2.clone(),
            b2.clone(),
            1,
            alloc::vec![one_hop_path(
                &env2,
                SwapVenue::Aquarius,
                p2,
                a2.clone(),
                b2.clone(),
                1_000_000,
            )],
            id2,
        );
        assert_eq!(r2.execute_strategy(&s2, &IN, &xdr), 980_000);
        assert_eq!(r2.admin_fee_balance(&b2), 10_000);
        assert_eq!(r2.referral_fee_balance(&id2, &b2), 10_000);
        assert_eq!(r2.admin_fee_balance(&a2), 0);
    }

    // Probe: A -> B (honest) -> C (sender-owned tail), C whitelisted.
    let xdr = strategy_xdr_with_referral(
        &env,
        a.clone(),
        c.clone(),
        1,
        alloc::vec![path(
            alloc::vec![
                SwapHop {
                    venue: SwapVenue::Aquarius,
                    pool: honest.clone(),
                    token_in: a.clone(),
                    token_out: b.clone(),
                },
                SwapHop {
                    venue: SwapVenue::Aquarius,
                    pool: tail.clone(),
                    token_in: b.clone(),
                    token_out: c.clone(),
                },
            ],
            1_000_000,
        )],
        id,
    );
    assert_eq!(router.execute_strategy(&sender, &IN, &xdr), 1);

    // Documented-policy numbers: fee base was 1 unit of C, so no bucket moved.
    assert_eq!(router.admin_fee_balance(&a), 0);
    assert_eq!(router.referral_fee_balance(&id, &a), 0);
    assert_eq!(router.admin_fee_balance(&b), 0);
    assert_eq!(router.referral_fee_balance(&id, &b), 0);
    assert_eq!(router.admin_fee_balance(&c), 0);
    assert_eq!(router.referral_fee_balance(&id, &c), 0);
    assert_eq!(token::Client::new(&env, &b).balance(&tail), IN);
    assert_eq!(token::Client::new(&env, &c).balance(&sender), 1);
    // Reserve identity: nothing reserved, nothing stranded in the router.
    for t in [&a, &b, &c] {
        assert_eq!(
            env.as_contract(&router_addr, || reserved_fee_balance(&env, t)),
            0
        );
        assert_eq!(token::Client::new(&env, t).balance(&router_addr), 0);
    }
}

/// R-2 / R-6: a referral whose stored owner is the router itself. The
/// permissionless claim self-transfers, clears the bucket and releases the
/// reserve, leaving the backing as sweepable surplus. `ReservedTotal` stays
/// equal to the bucket sum throughout, and only the owner can move the surplus.
#[test]
fn claim_to_router_owner_keeps_reserved_total_equal_to_bucket_sum() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let router_addr = env.register(Router, (admin.clone(),));
    let router = RouterClient::new(&env, &router_addr);
    let sender = Address::generate(&env);
    let asset_admin = Address::generate(&env);
    let (a, sac_a) = new_asset(&env, &asset_admin);
    let (b, sac_b) = new_asset(&env, &asset_admin);
    let pool = env.register(aquarius_mock::AqPool, ());
    aquarius_mock::AqPoolClient::new(&env, &pool).init(&a, &b);
    sac_a.mint(&sender, &IN);
    sac_b.mint(&pool, &IN);

    router.set_static_fee(&100);
    let id = router.add_referral(&Address::generate(&env), &100);
    let xdr = strategy_xdr_with_referral(
        &env,
        a.clone(),
        b.clone(),
        1,
        alloc::vec![one_hop_path(
            &env,
            SwapVenue::Aquarius,
            pool,
            a.clone(),
            b.clone(),
            1_000_000,
        )],
        id,
    );
    assert_eq!(router.execute_strategy(&sender, &IN, &xdr), 980_000);
    assert_eq!(router.admin_fee_balance(&a), 10_000);
    assert_eq!(router.referral_fee_balance(&id, &a), 10_000);
    assert_eq!(
        env.as_contract(&router_addr, || reserved_fee_balance(&env, &a)),
        20_000
    );

    router.set_referral_owner(&id, &router_addr);
    // Permissionless: any caller may trigger the claim; the recipient is fixed.
    router.claim_referral_fees(&id, &vec![&env, a.clone()]);

    let a_client = token::Client::new(&env, &a);
    assert_eq!(router.referral_fee_balance(&id, &a), 0);
    assert_eq!(router.admin_fee_balance(&a), 10_000);
    assert_eq!(
        env.as_contract(&router_addr, || reserved_fee_balance(&env, &a)),
        10_000,
        "reserve must drop by exactly the claimed bucket"
    );
    assert_eq!(a_client.balance(&router_addr), 20_000);

    // Surplus is owner-sweepable; the admin bucket stays backed.
    let recipient = Address::generate(&env);
    router.sweep_balance(&recipient, &vec![&env, a.clone()]);
    assert_eq!(a_client.balance(&recipient), 10_000);
    assert_eq!(a_client.balance(&router_addr), 10_000);
    router.claim_admin_fees(&admin, &vec![&env, a.clone()]);
    assert_eq!(a_client.balance(&admin), 10_000);
    assert_eq!(a_client.balance(&router_addr), 0);
    assert_eq!(
        env.as_contract(&router_addr, || reserved_fee_balance(&env, &a)),
        0
    );
}

/// R-6 rounding: the static and referral components floor independently, so a
/// balance whose combined fee would be 1 unit pays 0 when each component rounds
/// to zero. The forgiveness is below one unit per component and never favours
/// the router.
#[test]
fn fee_components_floor_independently_and_never_overcharge() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let router_addr = env.register(Router, (admin.clone(),));
    let router = RouterClient::new(&env, &router_addr);
    let sender = Address::generate(&env);
    let asset_admin = Address::generate(&env);
    let (a, sac_a) = new_asset(&env, &asset_admin);
    let (b, sac_b) = new_asset(&env, &asset_admin);
    let pool = env.register(aquarius_mock::AqPool, ());
    aquarius_mock::AqPoolClient::new(&env, &pool).init(&a, &b);

    // 1_999 * 5 / 10_000 = 0.9995 -> 0 per component; 1_999 * 10 / 10_000 = 1.999 -> 1 combined.
    let amount = 1_999_i128;
    sac_a.mint(&sender, &amount);
    sac_b.mint(&pool, &amount);
    router.set_static_fee(&5);
    let id = router.add_referral(&Address::generate(&env), &5);

    let xdr = strategy_xdr_with_referral(
        &env,
        a.clone(),
        b.clone(),
        1,
        alloc::vec![one_hop_path(
            &env,
            SwapVenue::Aquarius,
            pool,
            a.clone(),
            b.clone(),
            1_000_000,
        )],
        id,
    );
    assert_eq!(router.execute_strategy(&sender, &amount, &xdr), amount);
    assert_eq!(router.admin_fee_balance(&a), 0);
    assert_eq!(router.referral_fee_balance(&id, &a), 0);
    assert_eq!(
        env.as_contract(&router_addr, || reserved_fee_balance(&env, &a)),
        0
    );
}

/// R-6: `min_out` is compared after the fee. A route whose gross fill equals
/// `min_out` fails once the input-side fee shrinks the fill, so the sender is
/// never paid less than the declared minimum.
#[test]
fn min_out_is_enforced_net_of_fee() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let router_addr = env.register(Router, (admin.clone(),));
    let router = RouterClient::new(&env, &router_addr);
    let sender = Address::generate(&env);
    let asset_admin = Address::generate(&env);
    let (a, sac_a) = new_asset(&env, &asset_admin);
    let (b, sac_b) = new_asset(&env, &asset_admin);
    let pool = env.register(aquarius_mock::AqPool, ());
    aquarius_mock::AqPoolClient::new(&env, &pool).init(&a, &b);
    sac_a.mint(&sender, &IN);
    sac_b.mint(&pool, &IN);
    router.set_static_fee(&100);
    let id = router.add_referral(&Address::generate(&env), &100);

    let build = |min_out: i128| {
        strategy_xdr_with_referral(
            &env,
            a.clone(),
            b.clone(),
            min_out,
            alloc::vec![one_hop_path(
                &env,
                SwapVenue::Aquarius,
                pool.clone(),
                a.clone(),
                b.clone(),
                1_000_000,
            )],
            id,
        )
    };
    assert_eq!(
        router
            .try_execute_strategy(&sender, &IN, &build(980_001))
            .unwrap_err()
            .unwrap(),
        Error::SlippageExceeded.into()
    );
    assert_eq!(
        router.execute_strategy(&sender, &IN, &build(980_000)),
        980_000
    );
}
