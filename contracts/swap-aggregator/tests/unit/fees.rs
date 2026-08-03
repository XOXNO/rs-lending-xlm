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
