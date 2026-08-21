use crate::reserved_fee_balance;
use crate::storage::accumulate_fee;
use crate::types::{DataKey, SwapVenue};
use crate::{Router, RouterClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{token, vec, Address, Env};

use super::support::{
    aquarius_mock, new_asset, no_transfer_token_mock, one_hop_path, strategy_xdr_with_referral,
};

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

    assert_eq!(
        token::Client::new(&env, &untouched_token).balance(&router_addr),
        500
    );
}

/// A referral that exists but has never accrued anything withholds nothing.
///
/// Existing at all used to matter: the reserve walked `1..=referral_counter()`
/// and an unfunded id contributed an absent slot to skip. The counter made the
/// walk go away, so what is pinned now is the outcome rather than the mechanism
/// — issuing a referral must not, by itself, make a single stray unit
/// unsweepable.
#[test]
fn a_referral_with_no_accrual_withholds_nothing_from_a_sweep() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let router_addr = env.register(Router, (admin.clone(),));
    let router = RouterClient::new(&env, &router_addr);
    let referral_owner = Address::generate(&env);
    let recipient = Address::generate(&env);
    let asset_admin = Address::generate(&env);
    let (stray_token, sac_stray) = new_asset(&env, &asset_admin);

    let id = router.add_referral(&referral_owner, &100);
    assert_eq!(router.referral_fee_balance(&id, &stray_token), 0);

    sac_stray.mint(&router_addr, &777);
    router.sweep_balance(&recipient, &vec![&env, stray_token.clone()]);

    assert_eq!(
        token::Client::new(&env, &stray_token).balance(&router_addr),
        0
    );
    assert_eq!(
        token::Client::new(&env, &stray_token).balance(&recipient),
        777
    );
}

/// Accrual re-arms the bucket it credits, and reading the reserve re-arms the
/// counter.
///
/// `reserved_fee_balance` used to walk every referral bucket and bump each one's
/// TTL as a side effect. It now reads a single `ReservedTotal` entry, so the
/// buckets have to be kept alive where they are actually written -- otherwise a
/// long-lived referral bucket could archive while the reserve still withholds
/// its backing from `sweep_balance`.
#[test]
fn fee_accrual_and_reserve_reads_re_arm_their_ttls() {
    use common::constants::{TTL_BUMP_SHARED, TTL_THRESHOLD_SHARED};
    use soroban_sdk::testutils::storage::Persistent as _;

    let env = Env::default();
    let router_addr = env.register(Router, (Address::generate(&env),));
    let token = Address::generate(&env);

    env.as_contract(&router_addr, || {
        let admin_key = DataKey::AdminFee(token.clone());
        let referral_key = DataKey::ReferralFee(1, token.clone());
        let total_key = DataKey::ReservedTotal(token.clone());

        env.storage().persistent().set(&admin_key, &0i128);
        let aged_admin = env.storage().persistent().get_ttl(&admin_key);
        assert!(
            aged_admin < TTL_THRESHOLD_SHARED,
            "fresh entry must sit below the renewal threshold"
        );

        accumulate_fee(&env, admin_key.clone(), 10);
        accumulate_fee(&env, referral_key.clone(), 7);

        assert_eq!(
            env.storage().persistent().get_ttl(&admin_key),
            TTL_BUMP_SHARED,
            "AdminFee TTL must be re-armed on accrual: aged={aged_admin}"
        );
        assert_eq!(
            env.storage().persistent().get_ttl(&referral_key),
            TTL_BUMP_SHARED,
            "ReferralFee TTL must be re-armed on accrual"
        );

        assert_eq!(reserved_fee_balance(&env, &token), 17);
        assert_eq!(
            env.storage().persistent().get_ttl(&total_key),
            TTL_BUMP_SHARED,
            "ReservedTotal TTL must be re-armed when it is read"
        );
    });
}

/// The reserve must not grow with the number of referrals ever issued.
///
/// The old `reserved_fee_balance` walked `1..=referral_counter()` on every
/// `sweep_balance`, so a cheap `add_referral` loop could push the sweep past the
/// CPU budget and strand stray tokens in the router permanently.
#[test]
fn reserved_fee_balance_does_not_walk_the_referral_space() {
    let sweep_cpu = |referral_count: u32| -> u64 {
        let env = Env::default();
        env.mock_all_auths();
        env.cost_estimate().budget().reset_unlimited();

        let admin = Address::generate(&env);
        let router_addr = env.register(Router, (admin.clone(),));
        let router = RouterClient::new(&env, &router_addr);
        let referral_owner = Address::generate(&env);
        let recipient = Address::generate(&env);
        let asset_admin = Address::generate(&env);
        let (stray_token, sac_stray) = new_asset(&env, &asset_admin);

        for _ in 0..referral_count {
            router.add_referral(&referral_owner, &10);
        }
        assert_eq!(router.referral_counter(), referral_count as u64);
        sac_stray.mint(&router_addr, &500);

        env.cost_estimate().budget().reset_unlimited();
        router.sweep_balance(&recipient, &vec![&env, stray_token.clone()]);
        let cpu = env.cost_estimate().budget().cpu_instruction_cost();

        assert_eq!(
            token::Client::new(&env, &stray_token).balance(&recipient),
            500
        );
        cpu
    };

    let one = sweep_cpu(1);
    let many = sweep_cpu(64);
    let per_referral = many.saturating_sub(one) / 63;
    // Measured: ~3k per referral with the counter (the ledger map simply holds
    // more entries), ~62k per referral with the walk it replaced. The 10k
    // threshold sits ~3x above the O(1) figure and ~6x below the O(n) one, so
    // it absorbs host-cost drift from an SDK bump without ever admitting a
    // regression back to a per-referral read. If a future SDK pushes the O(1)
    // side past this, re-measure both numbers and keep the band, rather than
    // just raising the constant.
    assert!(
        per_referral < 10_000,
        "sweep cost must not scale with the referral counter \
         (1 referral: {one}, 64 referrals: {many}, {per_referral}/referral)"
    );
}

/// An instance upgraded over funded buckets must be able to rebuild its
/// reserve, and the rebuild must be safe to run twice.
///
/// `upgrade` is a live owner entrypoint and testnet runs a deployed aggregator,
/// so the counter cannot assume it was there from genesis. Upgrading onto a
/// build that reads `ReservedTotal` while the ledger holds only buckets gives
/// `reserved = 0` against real backing: the next `sweep_balance` would hand the
/// fees to the sweep recipient, and every claim afterwards would fail closed
/// because the counter cannot go negative. `migrate_reserved_totals` is the
/// one-shot repair.
#[test]
fn migrate_reserved_totals_rebuilds_the_counter_for_an_upgraded_instance() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let router_addr = env.register(Router, (admin.clone(),));
    let router = RouterClient::new(&env, &router_addr);
    let referral_owner = Address::generate(&env);
    let recipient = Address::generate(&env);
    let asset_admin = Address::generate(&env);
    let (token, sac) = new_asset(&env, &asset_admin);

    let first = router.add_referral(&referral_owner, &100);
    let second = router.add_referral(&referral_owner, &100);

    // Fund the buckets, then delete the counter: that is exactly the ledger an
    // upgrade from a pre-`ReservedTotal` build lands on.
    env.as_contract(&router_addr, || {
        accumulate_fee(&env, DataKey::AdminFee(token.clone()), 40);
        accumulate_fee(&env, DataKey::ReferralFee(first, token.clone()), 25);
        accumulate_fee(&env, DataKey::ReferralFee(second, token.clone()), 35);
        env.storage()
            .persistent()
            .remove(&DataKey::ReservedTotal(token.clone()));

        // The hazard, stated: 100 units are owed and the reserve reads zero.
        assert_eq!(reserved_fee_balance(&env, &token), 0);
    });
    assert_eq!(router.admin_fee_balance(&token), 40);

    // 100 of fee backing plus 123 genuinely stray units.
    sac.mint(&router_addr, &223);

    // Owner-gated like every other admin operation: with no signature in the
    // context the rebuild is refused outright.
    env.set_auths(&[]);
    assert!(
        router
            .try_migrate_reserved_totals(&vec![&env, token.clone()])
            .is_err(),
        "migration must not be reachable without the owner's authorization"
    );
    env.mock_all_auths();

    router.migrate_reserved_totals(&vec![&env, token.clone()]);
    assert_eq!(
        env.as_contract(&router_addr, || reserved_fee_balance(&env, &token)),
        100,
        "the rebuild must recover AdminFee + every ReferralFee"
    );

    // Idempotent: the second call sets the same total, it does not add to it.
    router.migrate_reserved_totals(&vec![&env, token.clone()]);
    assert_eq!(
        env.as_contract(&router_addr, || reserved_fee_balance(&env, &token)),
        100,
        "migration must be safe to run twice"
    );

    router.sweep_balance(&recipient, &vec![&env, token.clone()]);
    let client = token::Client::new(&env, &token);
    assert_eq!(
        client.balance(&recipient),
        123,
        "only the stray units leave"
    );
    assert_eq!(client.balance(&router_addr), 100);

    // And the buckets still pay out in full afterwards.
    let tokens = vec![&env, token.clone()];
    router.claim_admin_fees(&admin, &tokens);
    router.claim_referral_fees(&first, &tokens);
    router.claim_referral_fees(&second, &tokens);

    assert_eq!(client.balance(&admin), 40);
    assert_eq!(client.balance(&referral_owner), 60);
    assert_eq!(client.balance(&router_addr), 0);
    assert_eq!(
        env.as_contract(&router_addr, || reserved_fee_balance(&env, &token)),
        0
    );
}

/// Migrating a token that never carried a fee must not invent a reserve.
#[test]
fn migrate_reserved_totals_leaves_an_unused_token_alone() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let router_addr = env.register(Router, (admin,));
    let router = RouterClient::new(&env, &router_addr);
    let recipient = Address::generate(&env);
    let asset_admin = Address::generate(&env);
    let (token, sac) = new_asset(&env, &asset_admin);

    router.migrate_reserved_totals(&vec![&env, token.clone()]);
    env.as_contract(&router_addr, || {
        assert_eq!(reserved_fee_balance(&env, &token), 0);
        assert!(
            !env.storage()
                .persistent()
                .has(&DataKey::ReservedTotal(token.clone())),
            "an empty rebuild must not create a rent-paying entry"
        );
    });

    sac.mint(&router_addr, &500);
    router.sweep_balance(&recipient, &vec![&env, token.clone()]);
    assert_eq!(token::Client::new(&env, &token).balance(&recipient), 500);
}

#[test]
fn sweep_balance_keeps_fee_backing_claimable() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let router_addr = env.register(Router, (admin.clone(),));
    let router = RouterClient::new(&env, &router_addr);
    let sender = Address::generate(&env);
    let referral_owner = Address::generate(&env);
    let sweep_recipient = Address::generate(&env);
    let asset_admin = Address::generate(&env);
    let (token_a, sac_a) = new_asset(&env, &asset_admin);
    let (token_b, sac_b) = new_asset(&env, &asset_admin);
    let pool = env.register(aquarius_mock::AqPool, ());

    aquarius_mock::AqPoolClient::new(&env, &pool).init(&token_a, &token_b);
    router.set_static_fee(&100);
    let referral_id = router.add_referral(&referral_owner, &100);

    sac_a.mint(&sender, &1_000);
    sac_b.mint(&pool, &1_000);
    let swap_xdr = strategy_xdr_with_referral(
        &env,
        token_a.clone(),
        token_b.clone(),
        980,
        alloc::vec![one_hop_path(
            &env,
            SwapVenue::Aquarius,
            pool,
            token_a.clone(),
            token_b,
            1_000_000,
        ),],
        referral_id,
    );

    assert_eq!(router.execute_strategy(&sender, &1_000, &swap_xdr), 980);
    assert_eq!(router.admin_fee_balance(&token_a), 10);
    assert_eq!(router.referral_fee_balance(&referral_id, &token_a), 10);

    sac_a.mint(&router_addr, &123);
    router.sweep_balance(&sweep_recipient, &vec![&env, token_a.clone()]);

    let token_client = token::Client::new(&env, &token_a);
    assert_eq!(token_client.balance(&sweep_recipient), 123);
    assert_eq!(token_client.balance(&router_addr), 20);

    router.claim_admin_fees(&admin, &vec![&env, token_a.clone()]);
    router.claim_referral_fees(&referral_id, &vec![&env, token_a.clone()]);

    assert_eq!(token_client.balance(&admin), 10);
    assert_eq!(token_client.balance(&referral_owner), 10);
    assert_eq!(router.admin_fee_balance(&token_a), 0);
    assert_eq!(router.referral_fee_balance(&referral_id, &token_a), 0);
    assert_eq!(token_client.balance(&router_addr), 0);
}

/// The O(1) reserve must agree with the sum it replaced, across several
/// referrals and several tokens, and stay in step through claims.
///
/// The old `reserved_fee_balance` recomputed `AdminFee(token) + sum over
/// 1..=referral_counter of ReferralFee(id, token)` on every read. This drives
/// six real `execute_strategy` runs -- three referrals, both directions of a
/// pair -- then checks the counter against that same sum, sweeps to prove no fee
/// backing leaks out as stray dust, and claims everything to prove the counter
/// unwinds to exactly zero.
#[test]
fn reserved_total_matches_bucket_sum() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let router_addr = env.register(Router, (admin.clone(),));
    let router = RouterClient::new(&env, &router_addr);
    let sender = Address::generate(&env);
    let sweep_recipient = Address::generate(&env);
    let asset_admin = Address::generate(&env);
    let (token_a, sac_a) = new_asset(&env, &asset_admin);
    let (token_b, sac_b) = new_asset(&env, &asset_admin);
    let pool = env.register(aquarius_mock::AqPool, ());
    aquarius_mock::AqPoolClient::new(&env, &pool).init(&token_a, &token_b);

    router.set_static_fee(&100);
    let referrals: alloc::vec::Vec<u64> = (0..3)
        .map(|_| router.add_referral(&Address::generate(&env), &100))
        .collect();

    // Three 1_000-unit swaps each way: 2% combined fee off the input side, the
    // remaining 980 swapped 1:1 by the mock.
    sac_a.mint(&sender, &3_000);
    sac_b.mint(&sender, &3_000);
    sac_a.mint(&pool, &3_000);
    sac_b.mint(&pool, &3_000);

    for &referral_id in &referrals {
        for (from, to) in [(&token_a, &token_b), (&token_b, &token_a)] {
            let swap_xdr = strategy_xdr_with_referral(
                &env,
                from.clone(),
                to.clone(),
                980,
                alloc::vec![one_hop_path(
                    &env,
                    SwapVenue::Aquarius,
                    pool.clone(),
                    from.clone(),
                    to.clone(),
                    1_000_000,
                ),],
                referral_id,
            );
            assert_eq!(router.execute_strategy(&sender, &1_000, &swap_xdr), 980);
        }
    }

    for token in [&token_a, &token_b] {
        let expected = referrals
            .iter()
            .fold(router.admin_fee_balance(token), |acc, id| {
                acc + router.referral_fee_balance(id, token)
            });
        // 3 x 10 admin + 3 x 10 referral on each side.
        assert_eq!(expected, 60);
        let counter = env.as_contract(&router_addr, || reserved_fee_balance(&env, token));
        assert_eq!(
            counter, expected,
            "the O(1) reserve must equal the bucket sum it replaced"
        );
    }

    sac_a.mint(&router_addr, &123);
    router.sweep_balance(
        &sweep_recipient,
        &vec![&env, token_a.clone(), token_b.clone()],
    );
    assert_eq!(
        token::Client::new(&env, &token_a).balance(&sweep_recipient),
        123
    );
    assert_eq!(
        token::Client::new(&env, &token_a).balance(&router_addr),
        60,
        "sweep must leave every bucket fully backed"
    );
    assert_eq!(token::Client::new(&env, &token_b).balance(&router_addr), 60);

    let tokens = vec![&env, token_a.clone(), token_b.clone()];
    router.claim_admin_fees(&admin, &tokens);
    for &referral_id in &referrals {
        router.claim_referral_fees(&referral_id, &tokens);
    }

    for token in [&token_a, &token_b] {
        assert_eq!(
            env.as_contract(&router_addr, || reserved_fee_balance(&env, token)),
            0,
            "claiming every bucket must unwind the reserve to zero"
        );
        assert_eq!(token::Client::new(&env, token).balance(&router_addr), 0);
    }
}

#[test]
fn sweep_balance_skips_transfer_when_balance_equals_reserved() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let router_addr = env.register(Router, (admin.clone(),));
    let router = RouterClient::new(&env, &router_addr);
    let token = env.register(no_transfer_token_mock::NoTransferToken, ());
    no_transfer_token_mock::NoTransferTokenClient::new(&env, &token).init(&20);
    env.as_contract(&router_addr, || {
        accumulate_fee(&env, DataKey::AdminFee(token.clone()), 20_i128);
    });

    router.sweep_balance(&Address::generate(&env), &vec![&env, token.clone()]);
    assert_eq!(router.admin_fee_balance(&token), 20);
}
