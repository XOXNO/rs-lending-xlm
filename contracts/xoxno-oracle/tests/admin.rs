#![cfg(test)]
extern crate std;

mod common;
use common::*;

use xoxno_oracle::{Error, XoxnoOracle, XoxnoOracleClient};

use soroban_sdk::testutils::{Address as _, MockAuth, MockAuthInvoke};
use soroban_sdk::{vec, Address, BytesN, Env, IntoVal, String};

#[test]
#[should_panic(expected = "Error(Contract, #3)")]
fn constructor_rejects_threshold_of_zero() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let signers = vec![&env, Address::generate(&env)];
    env.register(XoxnoOracle, (admin, signers, 0u32, TEST_RESOLUTION));
}

#[test]
#[should_panic(expected = "Error(Contract, #3)")]
fn constructor_rejects_threshold_above_signer_count() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let signers = vec![&env, Address::generate(&env)];
    env.register(XoxnoOracle, (admin, signers, 2u32, TEST_RESOLUTION));
}

#[test]
#[should_panic(expected = "Error(Contract, #3)")]
fn constructor_rejects_duplicate_signers() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let dup = Address::generate(&env);

    let signers = vec![&env, dup.clone(), dup];
    env.register(XoxnoOracle, (admin, signers, 1u32, TEST_RESOLUTION));
}

#[test]
fn renounce_ownership_clears_owner() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, _signers) = setup(&env, 1, 1);

    assert!(client.get_owner().is_some());
    client.renounce_ownership();
    assert!(client.get_owner().is_none());
}

#[test]
fn added_signer_can_submit_and_duplicate_add_is_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 1, 1);
    let feed = feed_id(&env);

    let newcomer = Address::generate(&env);
    assert_eq!(
        client.try_submit_price(&newcomer, &feed, &100i128, &1_000u64),
        Err(Ok(Error::NotAuthorizedSigner))
    );

    client.add_signer(&newcomer);
    client.submit_price(&newcomer, &feed, &100i128, &1_000u64);
    assert_eq!(
        client.read_price_data_for_feed(&feed).price.to_u128(),
        Some(100u128)
    );

    assert_eq!(
        client.try_add_signer(&newcomer),
        Err(Ok(Error::SignerAlreadyRegistered))
    );
    assert_eq!(
        client.try_add_signer(&signers[0]),
        Err(Ok(Error::SignerAlreadyRegistered))
    );
}

#[test]
fn set_threshold_boundary_validation() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, _signers) = setup(&env, 2, 1);

    assert_eq!(
        client.try_set_threshold(&0u32),
        Err(Ok(Error::InvalidThreshold))
    );

    assert_eq!(
        client.try_set_threshold(&3u32),
        Err(Ok(Error::InvalidThreshold))
    );

    client.set_threshold(&2u32);
}

#[test]
fn upgrade_rejects_unknown_wasm_hash() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, _signers) = setup(&env, 1, 1);

    let bogus = BytesN::from_array(&env, &[7u8; 32]);
    assert!(client.try_upgrade(&bogus).is_err());
}

#[test]
fn remove_signer_rejected_below_threshold() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 2, 2);

    let result = client.try_remove_signer(&signers[0]);
    assert_eq!(result, Err(Ok(Error::CannotRemoveBelowThreshold)));
}

#[test]
fn remove_signer_succeeds_above_threshold() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 3, 2);

    client.remove_signer(&signers[0]);

    let result = client.try_remove_signer(&signers[1]);
    assert_eq!(result, Err(Ok(Error::CannotRemoveBelowThreshold)));
}

/// `remove_signer_succeeds_above_threshold` never has the signer submit, so the
/// cleanup half of `remove_signer` is unobservable there: with no recorded
/// feeds the loop body never runs and there is no feed list to clear. Submit
/// first, so a de-authorized signer's price and feed list are proven gone.
#[test]
fn remove_signer_clears_its_submission_and_feed_list() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 3, 2);
    let feed = feed_id(&env);

    client.submit_price(&signers[0], &feed, &100i128, &1_000u64);

    let submission_key = MirrorKey::LatestSubmission(feed.clone(), signers[0].clone());
    let feeds_key = MirrorKey::SignerFeeds(signers[0].clone());
    let (had_submission, had_feeds) = env.as_contract(&client.address, || {
        (
            env.storage().persistent().has(&submission_key),
            env.storage().persistent().has(&feeds_key),
        )
    });
    assert!(
        had_submission && had_feeds,
        "precondition: submitting must record both the submission and the signer's feed list"
    );

    client.remove_signer(&signers[0]);

    let (submission_left, feeds_left) = env.as_contract(&client.address, || {
        (
            env.storage().persistent().has(&submission_key),
            env.storage().persistent().has(&feeds_key),
        )
    });
    assert!(
        !submission_left,
        "a de-authorized signer's price must not stay behind to feed future aggregates"
    );
    assert!(
        !feeds_left,
        "a de-authorized signer's feed list must be dropped, not left to accumulate"
    );
}

/// An asset already bound to a feed must not be silently repointed at another
/// one. The second feed id is deliberately different, so `load_feed_owner`
/// cannot be what rejects the call — only the asset-side mapping check can.
#[test]
fn add_feed_rejects_remapping_an_asset_to_a_second_feed() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, _signers) = setup(&env, 1, 1);

    let asset = xlm_asset(&env);
    let first = feed_id(&env);
    let second = String::from_str(&env, "XLM/USD-ALT");

    client.add_feed(&first, &asset);

    let result = client.try_add_feed(&second, &asset);
    assert_eq!(result, Err(Ok(Error::FeedAlreadyMapped)));

    assert_eq!(
        client.assets(),
        vec![&env, asset],
        "a rejected add_feed must not leave a duplicate asset registry entry"
    );
}

/// Purging a feed must release the asset that owned it, otherwise the asset is
/// stranded: its feed is gone but it can never be mapped to a replacement.
#[test]
fn purge_feed_frees_the_asset_for_remapping() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, _signers) = setup(&env, 1, 1);

    let asset = xlm_asset(&env);
    let first = feed_id(&env);
    let second = String::from_str(&env, "XLM/USD-ALT");

    client.add_feed(&first, &asset);
    client.purge_feed(&first);

    // Fails with FeedAlreadyMapped if the asset->feed mapping outlived the purge.
    client.add_feed(&second, &asset);

    assert_eq!(
        client.assets(),
        vec![&env, asset],
        "remapping after a purge must leave exactly one registry entry"
    );
}

/// Purging must drop the feed's price history too. Left behind, it would
/// resurface under a feed id that was re-registered later.
#[test]
fn purge_feed_drops_stored_history() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 1, 1);
    let feed = feed_id(&env);

    client.submit_price(&signers[0], &feed, &100i128, &1_000u64);

    let history_key = MirrorKey::History(feed.clone());
    let had_history = env.as_contract(&client.address, || {
        env.storage().persistent().has(&history_key)
    });
    assert!(
        had_history,
        "precondition: a submission at threshold must write a history entry"
    );

    client.purge_feed(&feed);

    let history_left = env.as_contract(&client.address, || {
        env.storage().persistent().has(&history_key)
    });
    assert!(
        !history_left,
        "purged feed history must not survive to be served under a re-registered feed id"
    );
}

#[test]
fn only_owner_can_initiate_ownership_transfer() {
    let env = Env::default();

    let admin = Address::generate(&env);
    let signer = Address::generate(&env);
    let signers = vec![&env, signer.clone()];
    let contract_id = env.register(XoxnoOracle, (admin.clone(), signers, 1u32, TEST_RESOLUTION));
    let client = XoxnoOracleClient::new(&env, &contract_id);

    let non_owner = Address::generate(&env);
    let new_owner = Address::generate(&env);
    let live_until_ledger = 1000u32;
    env.mock_auths(&[MockAuth {
        address: &non_owner,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "transfer_ownership",
            args: (new_owner.clone(), live_until_ledger).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    let result = client.try_transfer_ownership(&new_owner, &live_until_ledger);
    assert!(result.is_err());

    env.mock_auths(&[MockAuth {
        address: &admin,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "transfer_ownership",
            args: (new_owner.clone(), live_until_ledger).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    client.transfer_ownership(&new_owner, &live_until_ledger);
    assert_eq!(client.get_owner(), Some(admin));

    env.mock_auths(&[MockAuth {
        address: &new_owner,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "accept_ownership",
            args: vec![&env],
            sub_invokes: &[],
        },
    }]);
    client.accept_ownership();
    assert_eq!(client.get_owner(), Some(new_owner));
}

#[test]
fn set_max_submission_age_enforces_floor_and_ttl_ceiling() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, _signers) = setup(&env, 1, 1);

    assert_eq!(
        client.try_set_max_submission_age_seconds(&59u64),
        Err(Ok(Error::InvalidSubmissionAge))
    );

    assert_eq!(
        client.try_set_max_submission_age_seconds(&86_401u64),
        Err(Ok(Error::InvalidSubmissionAge))
    );

    client.set_max_submission_age_seconds(&60u64);
    client.set_max_submission_age_seconds(&86_400u64);
}

#[test]
fn set_max_stale_cannot_drop_below_submission_age() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, _signers) = setup(&env, 1, 1);

    assert_eq!(
        client.try_set_max_stale_seconds(&899u64),
        Err(Ok(Error::InvalidSubmissionAge))
    );

    client.set_max_stale_seconds(&900u64);
}

#[test]
fn timing_configuration_getters_and_relative_skew_boundary() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, _signers) = setup(&env, 1, 1);

    client.set_max_stale_seconds(&1_234u64);
    client.set_max_submission_age_seconds(&321u64);

    assert_eq!(client.max_stale_seconds(), 1_234);
    assert_eq!(client.max_submission_age_seconds(), 321);
    assert_eq!(
        client.try_set_max_relative_skew_seconds(&322u64),
        Err(Ok(Error::InvalidRelativeSkew))
    );
    client.set_max_relative_skew_seconds(&321u64);
    assert_eq!(client.max_relative_skew_seconds(), 321);
}

#[test]
fn only_admin_can_call_add_feed() {
    let env = Env::default();
    let (client, admin, _signers) = setup(&env, 1, 1);
    let asset = xlm_asset(&env);
    let not_admin = Address::generate(&env);

    env.mock_auths(&[MockAuth {
        address: &not_admin,
        invoke: &MockAuthInvoke {
            contract: &client.address,
            fn_name: "add_feed",
            args: (feed_id(&env), asset.clone()).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    let result = client.try_add_feed(&feed_id(&env), &asset);
    assert!(result.is_err());

    env.mock_all_auths();
    let _ = admin;
    client.add_feed(&feed_id(&env), &asset);
    assert_eq!(client.assets().len(), 1);
}
