use crate::constants::FEE_CAP;
use crate::errors::Error;
use crate::{Router, RouterClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env, Symbol, Val, Vec};

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
    router.add_to_whitelist(&token_a);
    router.add_to_whitelist(&token_b);
    assert!(router.is_whitelisted(&token_a));
    assert_eq!(router.whitelisted_tokens().len(), 2);
    router.remove_from_whitelist(&token_a);
    assert!(!router.is_whitelisted(&token_a));
    router.remove_from_whitelist(&token_a);
    assert_eq!(router.whitelisted_tokens().len(), 1);

    let owner = Address::generate(&env);
    let id = router.add_referral(&owner, &100, &None);
    assert_eq!(router.referral_counter(), 1);
    router.set_referral_fee(&id, &200, &None);
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
fn legacy_referral_reads_and_updates_in_place() {
    use soroban_sdk::{IntoVal, Map};

    let env = Env::default();
    env.mock_all_auths();
    let owner = Address::generate(&env);
    let addr = env.register(Router, (owner.clone(),));
    let router = RouterClient::new(&env, &addr);
    let id = 7;
    env.as_contract(&addr, || {
        let mut old = Map::<Symbol, Val>::new(&env);
        old.set(Symbol::new(&env, "owner"), owner.clone().into_val(&env));
        old.set(Symbol::new(&env, "active"), true.into_val(&env));
        old.set(Symbol::new(&env, "fee_bps"), 25u32.into_val(&env));
        env.storage()
            .persistent()
            .set(&crate::types::DataKey::Referral(id), &old);
    });
    assert_eq!(router.referral(&id).unwrap().protocol_fee_bps, None);
    for expected in [Some(25), Some(0), Some(FEE_CAP - 25), None] {
        router.set_referral_fee(&id, &25, &expected);
        let cfg = router.referral(&id).unwrap();
        assert_eq!(cfg.protocol_fee_bps, expected);
        assert_eq!(cfg.owner, owner);
        assert_eq!(cfg.fee_bps, 25);
        assert!(cfg.active);
    }
    assert_eq!(
        router
            .try_set_referral_fee(&id, &25, &Some(FEE_CAP - 24))
            .unwrap_err()
            .unwrap(),
        Error::FeeTooHigh.into()
    );
}

#[test]
fn create_referral_with_protocol_rate() {
    let env = Env::default();
    env.mock_all_auths();
    let owner = Address::generate(&env);
    let addr = env.register(Router, (owner.clone(),));
    let router = RouterClient::new(&env, &addr);
    router.set_static_fee(&5);

    for (partner, protocol) in [(25, Some(FEE_CAP - 24)), (996, None), (u32::MAX, Some(1))] {
        let err = router.try_add_referral(&owner, &partner, &protocol);
        assert_eq!(err.unwrap_err().unwrap(), Error::FeeTooHigh.into());
    }
    assert_eq!(router.referral_counter(), 0);
    for rate in [None, Some(0), Some(25), Some(FEE_CAP - 25)] {
        let id = router.add_referral(&owner, &25, &rate);
        let cfg = router.referral(&id).unwrap();
        assert_eq!(cfg.fee_bps, 25);
        assert_eq!(cfg.protocol_fee_bps, rate);
    }
}

#[test]
fn referral_rates_update_atomically() {
    let env = Env::default();
    env.mock_all_auths();
    let owner = Address::generate(&env);
    let addr = env.register(Router, (owner.clone(),));
    let router = RouterClient::new(&env, &addr);
    router.set_static_fee(&5);
    let id = router.add_referral(&owner, &45, &None);

    router.set_referral_fee(&id, &25, &Some(25));
    let rates = || {
        let cfg = router.referral(&id).unwrap();
        (cfg.fee_bps, cfg.protocol_fee_bps)
    };
    assert_eq!(rates(), (25, Some(25)));
    for (partner, protocol) in [(501, Some(500)), (996, None), (u32::MAX, Some(1))] {
        let err = router.try_set_referral_fee(&id, &partner, &protocol);
        assert_eq!(err.unwrap_err().unwrap(), Error::FeeTooHigh.into());
        assert_eq!(rates(), (25, Some(25)));
    }
    router.set_referral_fee(&id, &10, &None);
    assert_eq!(rates(), (10, None));
    router.set_referral_fee(&id, &10, &Some(0));
    assert_eq!(rates(), (10, Some(0)));
    router.set_referral_fee(&id, &995, &None);
    assert_eq!(rates(), (995, None));
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
            .try_add_referral(&admin, &1_001, &None)
            .unwrap_err()
            .unwrap(),
        Error::FeeTooHigh.into()
    );
    let id = router.add_referral(&admin, &10, &None);
    assert_eq!(
        router
            .try_set_referral_fee(&id, &1_001, &None)
            .unwrap_err()
            .unwrap(),
        Error::FeeTooHigh.into()
    );
}

#[test]
fn fee_setters_accept_exact_cap() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let router_addr = env.register(Router, (admin.clone(),));
    let router = RouterClient::new(&env, &router_addr);

    router.set_static_fee(&FEE_CAP);
    assert_eq!(router.static_fee_bps(), FEE_CAP);

    let owner = Address::generate(&env);
    let id = router.add_referral(&owner, &FEE_CAP, &Some(0));
    assert_eq!(router.referral(&id).unwrap().fee_bps, FEE_CAP);

    router.set_referral_fee(&id, &0, &None);
    router.set_referral_fee(&id, &FEE_CAP, &Some(0));
    assert_eq!(router.referral(&id).unwrap().fee_bps, FEE_CAP);
}

#[test]
fn renounce_ownership_is_not_on_the_abi() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let router_addr = env.register(Router, (admin.clone(),));
    let router = RouterClient::new(&env, &router_addr);

    let called = env.try_invoke_contract::<Val, soroban_sdk::Error>(
        &router_addr,
        &Symbol::new(&env, "renounce_ownership"),
        Vec::new(&env),
    );
    assert!(called.is_err());
    assert_eq!(router.get_owner(), Some(admin.clone()));
    assert_eq!(router.admin(), admin);
}

#[test]
fn upgrade_to_unknown_wasm_hash_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let router_addr = env.register(Router, (Address::generate(&env),));
    let router = RouterClient::new(&env, &router_addr);
    let missing = soroban_sdk::BytesN::from_array(&env, &[7u8; 32]);
    assert!(router.try_upgrade(&missing).is_err());
}

// `renew_instance` sets the router instance TTL to `TTL_BUMP_INSTANCE`, both on
// a fresh instance and after the TTL falls below `TTL_THRESHOLD_INSTANCE`.
#[test]
fn renew_instance_re_extends_router_instance_ttl() {
    use common::constants::{TTL_BUMP_INSTANCE, TTL_THRESHOLD_INSTANCE};
    use soroban_sdk::testutils::storage::Instance as _;
    use soroban_sdk::testutils::Ledger as _;

    let env = Env::default();
    let router_addr = env.register(Router, (Address::generate(&env),));

    env.as_contract(&router_addr, || {
        crate::renew_instance(&env);
        assert_eq!(env.storage().instance().get_ttl(), TTL_BUMP_INSTANCE);
    });

    // Age the ledger so the TTL falls one ledger below the renewal threshold.
    let aged = TTL_BUMP_INSTANCE - TTL_THRESHOLD_INSTANCE + 1;
    env.ledger().with_mut(|l| l.sequence_number += aged);

    env.as_contract(&router_addr, || {
        assert!(env.storage().instance().get_ttl() < TTL_THRESHOLD_INSTANCE);
        crate::renew_instance(&env);
        assert_eq!(env.storage().instance().get_ttl(), TTL_BUMP_INSTANCE);
    });
}
