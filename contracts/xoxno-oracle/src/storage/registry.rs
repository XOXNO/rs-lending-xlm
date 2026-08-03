use common::oracle::providers::reflector::ReflectorAsset;
use soroban_sdk::{Address, Env, String, Vec};

use crate::storage::config::load_signers;
use crate::storage::index::{
    asset_count, feed_count, feed_index_contains, feed_index_insert, feed_index_remove,
};
use crate::storage::ttl::renew_persistent_key;
use crate::storage::DataKey;
use crate::Error;

pub(crate) fn load_feed_id(env: &Env, asset: &ReflectorAsset) -> Option<String> {
    let key = DataKey::FeedMapping(asset.clone());
    env.storage().persistent().get(&key).inspect(|_| {
        renew_persistent_key(env, &key);
    })
}

pub(crate) fn load_feed_owner(env: &Env, feed_id: &String) -> Option<ReflectorAsset> {
    let key = DataKey::FeedOwner(feed_id.clone());
    env.storage().persistent().get(&key).inspect(|_| {
        renew_persistent_key(env, &key);
    })
}

pub(crate) fn load_all_assets(env: &Env) -> Vec<ReflectorAsset> {
    let count = asset_count(env);
    let mut out = Vec::new(env);
    for i in 0..count {
        let key = DataKey::AssetAt(i);
        if let Some(asset) = env.storage().persistent().get(&key) {
            renew_persistent_key(env, &key);
            out.push_back(asset);
        }
    }
    out
}

pub(crate) fn load_all_feeds(env: &Env) -> Vec<String> {
    let count = feed_count(env);
    let mut out = Vec::new(env);
    for i in 0..count {
        let key = DataKey::FeedAt(i);
        if let Some(feed_id) = env.storage().persistent().get::<DataKey, String>(&key) {
            renew_persistent_key(env, &key);
            out.push_back(feed_id);
        }
    }
    out
}

pub(crate) fn ensure_known_feed(env: &Env, feed_id: &String) {
    if !renew_known_feed(env, feed_id) {
        feed_index_insert(env, feed_id.clone());
    }
}

pub(crate) fn renew_known_feed(env: &Env, feed_id: &String) -> bool {
    let index_key = DataKey::FeedIndex(feed_id.clone());
    match env.storage().persistent().get::<DataKey, u32>(&index_key) {
        Some(slot) => {
            renew_persistent_key(env, &index_key);
            renew_persistent_key(env, &DataKey::FeedAt(slot));
            true
        }
        None => false,
    }
}

pub(crate) fn record_signer_feed(env: &Env, signer: &Address, feed_id: &String) {
    let key = DataKey::SignerFeeds(signer.clone());
    let mut feeds: Vec<String> = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| Vec::new(env));
    if feeds.contains(feed_id) {
        renew_persistent_key(env, &key);
        return;
    }
    feeds.push_back(feed_id.clone());
    env.storage().persistent().set(&key, &feeds);
    renew_persistent_key(env, &key);
}

pub(crate) fn load_signer_feeds(env: &Env, signer: &Address) -> Vec<String> {
    env.storage()
        .persistent()
        .get(&DataKey::SignerFeeds(signer.clone()))
        .unwrap_or_else(|| Vec::new(env))
}

pub(crate) fn remove_signer_feed(env: &Env, signer: &Address, feed_id: &String) {
    let key = DataKey::SignerFeeds(signer.clone());
    let Some(feeds): Option<Vec<String>> = env.storage().persistent().get(&key) else {
        return;
    };
    let mut kept = Vec::new(env);
    for f in feeds.iter() {
        if &f != feed_id {
            kept.push_back(f);
        }
    }
    if kept.is_empty() {
        env.storage().persistent().remove(&key);
    } else {
        env.storage().persistent().set(&key, &kept);
        renew_persistent_key(env, &key);
    }
}

pub(crate) fn clear_feed_state(env: &Env, feed_id: &String) {
    env.storage()
        .persistent()
        .remove(&DataKey::CurrentAggregate(feed_id.clone()));
    env.storage()
        .persistent()
        .remove(&DataKey::History(feed_id.clone()));
    for signer in load_signers(env).iter() {
        env.storage()
            .persistent()
            .remove(&DataKey::LatestSubmission(feed_id.clone(), signer.clone()));
        remove_signer_feed(env, &signer, feed_id);
    }
    env.storage()
        .persistent()
        .remove(&DataKey::FeedOwner(feed_id.clone()));
    feed_index_remove(env, feed_id);
}

pub(crate) fn require_registered_signer(env: &Env, signer: &Address) -> Result<(), Error> {
    let signers = load_signers(env);
    if !signers.contains(signer) {
        return Err(Error::NotAuthorizedSigner);
    }
    Ok(())
}

pub(crate) fn require_known_feed(env: &Env, feed_id: &String) -> Result<(), Error> {
    if !feed_index_contains(env, feed_id) {
        return Err(Error::FeedNotKnown);
    }
    Ok(())
}
