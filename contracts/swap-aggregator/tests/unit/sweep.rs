//! `sweep_balance` and reserved fee-bucket accounting.
//!
//! Sweeps recover stray tokens only above reserved fee backing. Walking fee
//! buckets must skip absent slots (no TTL bump on missing entries) and renew
//! TTLs of positive buckets.

use crate::types::SwapVenue;
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

#[test]
fn reserved_fee_balance_skips_absent_referral_slot() {
    // A registered referral bumps ReferralCounter, so `reserved_fee_balance`
    // iterates that slot even when it never accrued a fee for the swept token.
    // The `amount > 0` presence guard must skip the TTL bump on the absent
    // entry — extending a nonexistent persistent entry panics. This pins the
    // referral half of that guard the way `sweep_balance_recovers_stray_tokens`
    // pins the admin half.
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let router_addr = env.register(Router, (admin.clone(),));
    let router = RouterClient::new(&env, &router_addr);
    let referral_owner = Address::generate(&env);
    let recipient = Address::generate(&env);
    let asset_admin = Address::generate(&env);
    let (stray_token, sac_stray) = new_asset(&env, &asset_admin);

    // Counter goes to 1; no swap runs, so ReferralFee(1, stray_token) is absent.
    router.add_referral(&referral_owner, &100);

    sac_stray.mint(&router_addr, &777);
    router.sweep_balance(&recipient, &vec![&env, stray_token.clone()]);

    // No fees are reserved for this token, so the full stray balance sweeps out.
    // Under a guard mutant that bumps the absent slot the sweep panics instead.
    assert_eq!(
        token::Client::new(&env, &stray_token).balance(&router_addr),
        0
    );
    assert_eq!(
        token::Client::new(&env, &stray_token).balance(&recipient),
        777
    );
}

// `reserved_fee_balance` must renew the shared-tier TTL of every positive fee
// bucket it walks — the guarded `extend_ttl` is what keeps fee backing alive
// between sweeps. Pins the `> 0` guards against a `< 0` swap, which returns
// the same reserved total but silently drops the renewal.
#[test]
fn reserved_fee_balance_renews_positive_fee_bucket_ttls() {
    use crate::types::DataKey;
    use common::constants::{TTL_BUMP_SHARED, TTL_THRESHOLD_SHARED};
    use soroban_sdk::testutils::storage::Persistent as _;

    let env = Env::default();
    let router_addr = env.register(Router, (Address::generate(&env),));
    let token = Address::generate(&env);

    env.as_contract(&router_addr, || {
        let admin_key = DataKey::AdminFee(token.clone());
        let referral_key = DataKey::ReferralFee(1, token.clone());
        env.storage()
            .instance()
            .set(&DataKey::ReferralCounter, &1u64);
        env.storage().persistent().set(&admin_key, &10i128);
        env.storage().persistent().set(&referral_key, &7i128);

        let aged_admin = env.storage().persistent().get_ttl(&admin_key);
        let aged_referral = env.storage().persistent().get_ttl(&referral_key);
        assert!(
            aged_admin < TTL_THRESHOLD_SHARED,
            "fresh entry must sit below the renewal threshold"
        );

        assert_eq!(crate::reserved_fee_balance(&env, &token), 17);

        assert_eq!(
            env.storage().persistent().get_ttl(&admin_key),
            TTL_BUMP_SHARED,
            "AdminFee TTL must be re-armed: aged={aged_admin}"
        );
        assert_eq!(
            env.storage().persistent().get_ttl(&referral_key),
            TTL_BUMP_SHARED,
            "ReferralFee TTL must be re-armed: aged={aged_referral}"
        );
    });
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
        vec![
            &env,
            one_hop_path(
                &env,
                SwapVenue::Aquarius,
                pool,
                token_a.clone(),
                token_b,
                1_000_000,
            ),
        ],
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

// When the router's whole balance is reserved fee backing, a sweep must not
// touch the token at all (not even a zero-value transfer).
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
        env.storage()
            .persistent()
            .set(&crate::types::DataKey::AdminFee(token.clone()), &20_i128);
    });

    router.sweep_balance(&Address::generate(&env), &vec![&env, token.clone()]);
    assert_eq!(router.admin_fee_balance(&token), 20);
}
