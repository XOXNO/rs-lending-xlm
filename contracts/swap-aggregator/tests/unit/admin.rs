//! Owner-gated admin surface: fees, whitelist, ownership, upgrade.

use crate::errors::Error;
use crate::{Router, RouterClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env, Vec};

use super::support::new_asset;
use crate::types::ReferralConfig;

#[test]
fn admin_setters_and_views_surface() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let router_addr = env.register(Router, (admin.clone(),));
    let router = RouterClient::new(&env, &router_addr);

    assert_eq!(router.admin(), admin);
    assert_eq!(router.static_fee_bps(), 0);
    assert_eq!(router.referral_counter(), 0);
    assert_eq!(router.whitelisted_tokens(), Vec::<Address>::new(&env));

    let new_admin = Address::generate(&env);
    let live_until = env.ledger().sequence() + 100;
    router.transfer_ownership(&new_admin, &live_until);
    router.accept_ownership();
    assert_eq!(router.admin(), new_admin);

    let (token_a, _) = new_asset(&env, &admin);
    let (token_b, _) = new_asset(&env, &admin);
    router.add_to_whitelist(&token_a);
    router.add_to_whitelist(&token_a); // dup -> no-op branch
    router.add_to_whitelist(&token_b);
    assert!(router.is_whitelisted(&token_a));
    assert_eq!(router.whitelisted_tokens().len(), 2);
    router.remove_from_whitelist(&token_a);
    assert!(!router.is_whitelisted(&token_a));
    router.remove_from_whitelist(&token_a); // absent -> None branch
    assert_eq!(router.whitelisted_tokens().len(), 1);

    let owner = Address::generate(&env);
    let id = router.add_referral(&owner, &100);
    assert_eq!(router.referral_counter(), 1);
    router.set_referral_fee(&id, &200);
    router.set_referral_active(&id, &false);
    let new_owner = Address::generate(&env);
    router.set_referral_owner(&id, &new_owner);
    let cfg: ReferralConfig = router.referral(&id).unwrap();
    assert_eq!(cfg.fee_bps, 200);
    assert!(!cfg.active);
    assert_eq!(cfg.owner, new_owner);
    assert!(router.referral(&999).is_none());
}

#[test]
fn admin_rejects_fee_over_cap() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let router_addr = env.register(Router, (admin.clone(),));
    let router = RouterClient::new(&env, &router_addr);
    assert_eq!(
        router.try_set_static_fee(&1_001).unwrap_err().unwrap(),
        Error::FeeTooHigh.into()
    );
    assert_eq!(
        router
            .try_add_referral(&admin, &1_001)
            .unwrap_err()
            .unwrap(),
        Error::FeeTooHigh.into()
    );
    let id = router.add_referral(&admin, &10);
    assert_eq!(
        router
            .try_set_referral_fee(&id, &1_001)
            .unwrap_err()
            .unwrap(),
        Error::FeeTooHigh.into()
    );
}

// FEE_CAP is inclusive: setters accept exactly the cap and reject cap + 1
// (the rejection side is covered by `admin_rejects_fee_over_cap`).
#[test]
fn fee_setters_accept_exact_cap() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let router_addr = env.register(Router, (admin.clone(),));
    let router = RouterClient::new(&env, &router_addr);

    router.set_static_fee(&crate::FEE_CAP);
    assert_eq!(router.static_fee_bps(), crate::FEE_CAP);

    let owner = Address::generate(&env);
    let id = router.add_referral(&owner, &crate::FEE_CAP);
    assert_eq!(router.referral(&id).unwrap().fee_bps, crate::FEE_CAP);

    router.set_referral_fee(&id, &0);
    router.set_referral_fee(&id, &crate::FEE_CAP);
    assert_eq!(router.referral(&id).unwrap().fee_bps, crate::FEE_CAP);
}

#[test]
fn ownable_get_owner_and_renounce() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let router_addr = env.register(Router, (admin.clone(),));
    let router = RouterClient::new(&env, &router_addr);

    assert_eq!(router.get_owner(), Some(admin));
    router.renounce_ownership();
    assert_eq!(router.get_owner(), None);
    assert!(router.try_admin().is_err());
}

// Upgrading to a wasm hash that was never uploaded must fail rather than
// silently succeed.
#[test]
fn upgrade_to_unknown_wasm_hash_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let router_addr = env.register(Router, (Address::generate(&env),));
    let router = RouterClient::new(&env, &router_addr);
    let missing = soroban_sdk::BytesN::from_array(&env, &[7u8; 32]);
    assert!(router.try_upgrade(&missing).is_err());
}
