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

/// Registers `feeds` feeds priced by every signer, then returns the
/// (ledger_entries, write_entries) footprint that `set_threshold` consumes.
fn set_threshold_footprint(feeds: u32, signer_count: u32) -> (u32, u32) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let signers: std::vec::Vec<Address> =
        (0..signer_count).map(|_| Address::generate(&env)).collect();
    let mut signers_vec = soroban_sdk::Vec::new(&env);
    for s in signers.iter() {
        signers_vec.push_back(s.clone());
    }
    let id = env.register(
        XoxnoOracle,
        (admin, signers_vec, signer_count, TEST_RESOLUTION),
    );
    let client = XoxnoOracleClient::new(&env, &id);

    let ts_ms = env.ledger().timestamp() * 1_000;
    for i in 0..feeds {
        let fid = String::from_str(&env, &std::format!("FEED{i}"));
        client.register_feed(&fid);
        for signer in signers.iter() {
            client.submit_price(signer, &fid, &1_000_000i128, &ts_ms);
        }
    }

    client.set_threshold(&signer_count);
    let r = env.cost_estimate().resources();
    (
        r.disk_read_entries + r.memory_read_entries + r.write_entries,
        r.write_entries,
    )
}

/// `set_threshold` must cost the same no matter how many feeds are registered.
///
/// It used to recompute every registered feed in the same transaction, so its
/// transaction footprint grew by about one ledger entry per signer plus three,
/// per feed. That is bounded by the network footprint limit, so past a certain
/// feed count the setter becomes permanently uncallable and the threshold can
/// no longer be changed -- exactly when a signer outage requires lowering it.
/// Asserting equality rather than an absolute budget keeps this test honest
/// across protocol versions that revise the limits.
#[test]
fn set_threshold_footprint_does_not_grow_with_feed_count() {
    let (entries_1, writes_1) = set_threshold_footprint(1, 3);
    let (entries_25, writes_25) = set_threshold_footprint(25, 3);

    assert_eq!(
        entries_1, entries_25,
        "set_threshold ledger-entry footprint grew from {entries_1} (1 feed) to {entries_25} (25 feeds)"
    );
    assert_eq!(
        writes_1, writes_25,
        "set_threshold write-entry footprint grew from {writes_1} (1 feed) to {writes_25} (25 feeds)"
    );
}

/// Raising the threshold stores the new value but deliberately leaves existing
/// aggregates alone; `recompute_feeds` is what applies it to a feed that
/// already holds one.
#[test]
fn recompute_feeds_applies_a_raised_threshold_to_an_existing_aggregate() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 3, 1);
    let feed = feed_id(&env);

    client.submit_price(&signers[0], &feed, &100i128, &1_000u64);
    assert_eq!(
        client.read_price_data_for_feed(&feed).price.to_u128(),
        Some(100u128)
    );

    // The setter no longer sweeps every feed, so the aggregate formed under
    // the old threshold of 1 survives the change.
    client.set_threshold(&3);
    assert_eq!(
        client.read_price_data_for_feed(&feed).price.to_u128(),
        Some(100u128)
    );

    // Sweeping applies the new threshold: a single submission is below quorum.
    client.recompute_feeds(&vec![&env, feed.clone()]);
    assert_eq!(
        expect_error(client.try_read_price_data_for_feed(&feed)),
        Error::NoDataForFeed
    );
}

#[test]
fn recompute_feeds_rejects_an_unknown_feed() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, _signers) = setup(&env, 3, 1);

    let unknown = String::from_str(&env, "NOT-A-FEED");
    assert_eq!(
        expect_error(client.try_recompute_feeds(&vec![&env, unknown])),
        Error::FeedNotKnown
    );
}

/// Validation runs over the whole batch before any aggregate is touched, so a
/// single bad id cannot leave the sweep half-applied.
#[test]
fn recompute_feeds_recomputes_nothing_when_any_feed_is_unknown() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 3, 1);
    let feed = feed_id(&env);

    client.submit_price(&signers[0], &feed, &100i128, &1_000u64);
    client.set_threshold(&3);

    let unknown = String::from_str(&env, "NOT-A-FEED");
    assert_eq!(
        expect_error(client.try_recompute_feeds(&vec![&env, feed.clone(), unknown])),
        Error::FeedNotKnown
    );

    // The known feed's aggregate is untouched by the rejected batch.
    assert_eq!(
        client.read_price_data_for_feed(&feed).price.to_u128(),
        Some(100u128)
    );
}

/// `feeds()` enumerates the ids an operator needs to pass to
/// `recompute_feeds` after a configuration change.
#[test]
fn feeds_lists_every_registered_feed() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, _signers) = setup(&env, 3, 1);
    let extra = String::from_str(&env, "BTC/USD");
    client.register_feed(&extra);

    let listed = client.feeds();
    assert_eq!(listed.len(), 2);
    assert!(listed.contains(feed_id(&env)));
    assert!(listed.contains(&extra));
}
