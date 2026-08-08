use crate::errors::Error;
use crate::types::SwapVenue;
use crate::{Router, RouterClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{vec, Address, Env};

use super::support::{
    aquarius_mock, new_asset, no_transfer_token_mock, one_hop_path, strategy_xdr_with_referral,
};

#[test]
fn referral_missing_id_is_noop() {
    let env = Env::default();
    env.mock_all_auths();
    let router_addr = env.register(Router, (Address::generate(&env),));
    let router = RouterClient::new(&env, &router_addr);
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (token_a, sac_a) = new_asset(&env, &admin);
    let (token_b, sac_b) = new_asset(&env, &admin);
    let pool = env.register(aquarius_mock::AqPool, ());
    aquarius_mock::AqPoolClient::new(&env, &pool).init(&token_a, &token_b);
    sac_a.mint(&sender, &500);
    sac_b.mint(&pool, &500);
    let xdr = strategy_xdr_with_referral(
        &env,
        token_a.clone(),
        token_b.clone(),
        500,
        alloc::vec![one_hop_path(
            &env,
            SwapVenue::Aquarius,
            pool,
            token_a.clone(),
            token_b.clone(),
            1_000_000,
        ),],
        99,
    );
    assert_eq!(router.execute_strategy(&sender, &500, &xdr), 500);
    assert_eq!(router.admin_fee_balance(&token_a), 0);
}

#[test]
fn referral_inactive_and_zero_combined_bps_noop() {
    {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let router_addr = env.register(Router, (admin.clone(),));
        let router = RouterClient::new(&env, &router_addr);
        let sender = Address::generate(&env);
        let aadmin = Address::generate(&env);
        let (token_a, sac_a) = new_asset(&env, &aadmin);
        let (token_b, sac_b) = new_asset(&env, &aadmin);
        let pool = env.register(aquarius_mock::AqPool, ());
        aquarius_mock::AqPoolClient::new(&env, &pool).init(&token_a, &token_b);
        sac_a.mint(&sender, &500);
        sac_b.mint(&pool, &500);
        router.set_static_fee(&100);
        let id = router.add_referral(&Address::generate(&env), &100);
        router.set_referral_active(&id, &false);
        let xdr = strategy_xdr_with_referral(
            &env,
            token_a.clone(),
            token_b.clone(),
            500,
            alloc::vec![one_hop_path(
                &env,
                SwapVenue::Aquarius,
                pool,
                token_a.clone(),
                token_b.clone(),
                1_000_000,
            ),],
            id,
        );
        assert_eq!(router.execute_strategy(&sender, &500, &xdr), 500);
        assert_eq!(router.admin_fee_balance(&token_a), 0);
    }

    {
        let env = Env::default();
        env.mock_all_auths();
        let router_addr = env.register(Router, (Address::generate(&env),));
        let router = RouterClient::new(&env, &router_addr);
        let sender = Address::generate(&env);
        let aadmin = Address::generate(&env);
        let (token_a, sac_a) = new_asset(&env, &aadmin);
        let (token_b, sac_b) = new_asset(&env, &aadmin);
        let pool = env.register(aquarius_mock::AqPool, ());
        aquarius_mock::AqPoolClient::new(&env, &pool).init(&token_a, &token_b);
        sac_a.mint(&sender, &500);
        sac_b.mint(&pool, &500);
        let id = router.add_referral(&Address::generate(&env), &0);
        let xdr = strategy_xdr_with_referral(
            &env,
            token_a.clone(),
            token_b.clone(),
            500,
            alloc::vec![one_hop_path(
                &env,
                SwapVenue::Aquarius,
                pool,
                token_a.clone(),
                token_b.clone(),
                1_000_000,
            ),],
            id,
        );
        assert_eq!(router.execute_strategy(&sender, &500, &xdr), 500);
        assert_eq!(router.admin_fee_balance(&token_a), 0);
    }

    {
        let env = Env::default();
        env.mock_all_auths();
        let router_addr = env.register(Router, (Address::generate(&env),));
        let router = RouterClient::new(&env, &router_addr);
        let sender = Address::generate(&env);
        let aadmin = Address::generate(&env);
        let (token_a, sac_a) = new_asset(&env, &aadmin);
        let (token_b, sac_b) = new_asset(&env, &aadmin);
        let pool = env.register(aquarius_mock::AqPool, ());
        aquarius_mock::AqPoolClient::new(&env, &pool).init(&token_a, &token_b);
        sac_a.mint(&sender, &1);
        sac_b.mint(&pool, &1);
        router.set_static_fee(&100);
        let id = router.add_referral(&Address::generate(&env), &0);
        let xdr = strategy_xdr_with_referral(
            &env,
            token_a.clone(),
            token_b.clone(),
            1,
            alloc::vec![one_hop_path(
                &env,
                SwapVenue::Aquarius,
                pool,
                token_a.clone(),
                token_b.clone(),
                1_000_000,
            ),],
            id,
        );
        assert_eq!(router.execute_strategy(&sender, &1, &xdr), 1);
        assert_eq!(router.admin_fee_balance(&token_a), 0);
    }
}

#[test]
fn zero_fee_side_creates_no_bucket_entry() {
    use crate::types::DataKey;

    {
        let env = Env::default();
        env.mock_all_auths();
        let router_addr = env.register(Router, (Address::generate(&env),));
        let router = RouterClient::new(&env, &router_addr);
        let sender = Address::generate(&env);
        let asset_admin = Address::generate(&env);
        let (token_a, sac_a) = new_asset(&env, &asset_admin);
        let (token_b, sac_b) = new_asset(&env, &asset_admin);
        let pool = env.register(aquarius_mock::AqPool, ());
        aquarius_mock::AqPoolClient::new(&env, &pool).init(&token_a, &token_b);
        sac_a.mint(&sender, &1_000);
        sac_b.mint(&pool, &1_000);
        let id = router.add_referral(&Address::generate(&env), &100);
        let xdr = strategy_xdr_with_referral(
            &env,
            token_a.clone(),
            token_b.clone(),
            990,
            alloc::vec![one_hop_path(
                &env,
                SwapVenue::Aquarius,
                pool,
                token_a.clone(),
                token_b,
                1_000_000,
            ),],
            id,
        );
        assert_eq!(router.execute_strategy(&sender, &1_000, &xdr), 990);
        assert_eq!(router.referral_fee_balance(&id, &token_a), 10);
        assert_eq!(router.admin_fee_balance(&token_a), 0);
        let has_admin_entry = env.as_contract(&router_addr, || {
            env.storage()
                .persistent()
                .has(&DataKey::AdminFee(token_a.clone()))
        });
        assert!(!has_admin_entry, "zero static fee must not create a bucket");
    }

    {
        let env = Env::default();
        env.mock_all_auths();
        let router_addr = env.register(Router, (Address::generate(&env),));
        let router = RouterClient::new(&env, &router_addr);
        let sender = Address::generate(&env);
        let asset_admin = Address::generate(&env);
        let (token_a, sac_a) = new_asset(&env, &asset_admin);
        let (token_b, sac_b) = new_asset(&env, &asset_admin);
        let pool = env.register(aquarius_mock::AqPool, ());
        aquarius_mock::AqPoolClient::new(&env, &pool).init(&token_a, &token_b);
        sac_a.mint(&sender, &1_000);
        sac_b.mint(&pool, &1_000);
        router.set_static_fee(&100);
        let id = router.add_referral(&Address::generate(&env), &0);
        let xdr = strategy_xdr_with_referral(
            &env,
            token_a.clone(),
            token_b.clone(),
            990,
            alloc::vec![one_hop_path(
                &env,
                SwapVenue::Aquarius,
                pool,
                token_a.clone(),
                token_b,
                1_000_000,
            ),],
            id,
        );
        assert_eq!(router.execute_strategy(&sender, &1_000, &xdr), 990);
        assert_eq!(router.admin_fee_balance(&token_a), 10);
        assert_eq!(router.referral_fee_balance(&id, &token_a), 0);
        let has_referral_entry = env.as_contract(&router_addr, || {
            env.storage()
                .persistent()
                .has(&DataKey::ReferralFee(id, token_a.clone()))
        });
        assert!(
            !has_referral_entry,
            "zero referral fee must not create a bucket"
        );
    }
}

#[test]
fn claim_skips_transfer_when_bucket_is_empty() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let router_addr = env.register(Router, (admin.clone(),));
    let router = RouterClient::new(&env, &router_addr);
    let token = env.register(no_transfer_token_mock::NoTransferToken, ());

    router.claim_admin_fees(&admin, &vec![&env, token.clone()]);
    let id = router.add_referral(&Address::generate(&env), &100);
    router.claim_referral_fees(&id, &vec![&env, token]);
}

#[test]
fn combined_static_and_referral_fee_cannot_exceed_the_cap() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let router_addr = env.register(Router, (admin.clone(),));
    let router = RouterClient::new(&env, &router_addr);

    router.set_static_fee(&600);
    let id = router.add_referral(&Address::generate(&env), &600);

    let sender = Address::generate(&env);
    let (token_a, sac_a) = new_asset(&env, &admin);
    let (token_b, sac_b) = new_asset(&env, &admin);
    let pool = env.register(aquarius_mock::AqPool, ());
    aquarius_mock::AqPoolClient::new(&env, &pool).init(&token_a, &token_b);
    sac_a.mint(&sender, &1_000);
    sac_b.mint(&pool, &1_000);

    let xdr = strategy_xdr_with_referral(
        &env,
        token_a.clone(),
        token_b.clone(),
        1,
        alloc::vec![one_hop_path(
            &env,
            SwapVenue::Aquarius,
            pool,
            token_a,
            token_b,
            1_000_000
        ),],
        id,
    );

    assert_eq!(
        router
            .try_execute_strategy(&sender, &1_000, &xdr)
            .unwrap_err()
            .unwrap(),
        Error::FeeTooHigh.into()
    );
}

/// A combined fee landing exactly on the cap is allowed, not rejected.
///
/// `apply_fees_on_token` rejects with `FeeTooHigh` when
/// `combined_bps > FEE_CAP`. The neighbouring test only exercises 1200 bps,
/// comfortably above the 1000 bps cap, so nothing pins the boundary itself.
///
/// Break this catches: that `>` becoming `>=`, which would reject a fee
/// configuration sitting exactly at the documented maximum. Surfaced as a
/// surviving mutant by `make mutants-swap-aggregator`
/// (lib.rs:307 `replace > with >= in apply_fees_on_token`).
#[test]
fn combined_fee_exactly_at_the_cap_is_charged_not_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let router_addr = env.register(Router, (admin.clone(),));
    let router = RouterClient::new(&env, &router_addr);

    // 500 + 500 == FEE_CAP (1000 bps).
    router.set_static_fee(&500);
    let referral_owner = Address::generate(&env);
    let id = router.add_referral(&referral_owner, &500);

    let sender = Address::generate(&env);
    let (token_a, sac_a) = new_asset(&env, &admin);
    let (token_b, sac_b) = new_asset(&env, &admin);
    let pool = env.register(aquarius_mock::AqPool, ());
    aquarius_mock::AqPoolClient::new(&env, &pool).init(&token_a, &token_b);
    sac_a.mint(&sender, &1_000);
    sac_b.mint(&pool, &1_000);

    let xdr = strategy_xdr_with_referral(
        &env,
        token_a.clone(),
        token_b.clone(),
        1,
        alloc::vec![one_hop_path(
            &env,
            SwapVenue::Aquarius,
            pool,
            token_a.clone(),
            token_b.clone(),
            1_000_000,
        ),],
        id,
    );

    let out = router.execute_strategy(&sender, &1_000, &xdr);

    // 1:1 fill of 1000, less 5% static and 5% referral.
    // With no whitelist configured the fee is taken on the input token: 5%
    // static + 5% referral of 1000, leaving 900 to swap at the mock's 1:1 rate.
    assert_eq!(out, 900);
    assert_eq!(router.admin_fee_balance(&token_a), 50);
    assert_eq!(router.referral_fee_balance(&id, &token_a), 50);
}
