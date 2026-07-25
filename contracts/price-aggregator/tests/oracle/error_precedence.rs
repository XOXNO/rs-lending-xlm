//! Pins which error `price()` raises when a config is broken in *both* legs.
//!
//! The hard path gathers its legs through `compose`, whose per-leg reads are
//! soft but not panic-free: a few config-invariant violations (here, a TWAP
//! source configured for zero records) revert from inside the read itself. So
//! the traversal must reject a broken primary before the anchor is ever
//! touched, or the anchor's error silently replaces the primary's and the
//! error contract in `hard_path_errors.rs` no longer describes real configs.
//!
//! `broken_anchor_alone_reverts_twap_error` is the control: it shows the anchor
//! fault used here really does revert, so the two precedence tests are pinning
//! an ordering rather than passing vacuously.
//!
//! The same ordering has to survive one indirection: a Reflector primary with a
//! quoted base reprices its quote through the *soft* status path, so the
//! quote's own traversal sits underneath a fail-closed `price`. If that
//! traversal reads past a broken quote primary, an anchor revert one level down
//! surfaces as `price`'s error.

use super::*;
use common::constants::WAD;
use common::errors::GenericError;
use common::types::{
    OracleAssetRef, OracleReadMode, OracleSourceConfigOption, ReflectorBase, ReflectorSourceConfig,
};
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{Env, String};

use crate::{PriceAggregator, PriceAggregatorClient};

use crate::test_support::{
    redstone_dual, redstone_primary_reflector_anchor, reflector_quoted, register_redstone_feed,
    EmptyReflector, PricedReflector, RevertingReflector,
};

const NOW: u64 = 1_700_000_000;
const MAX_STALE: u64 = 60;

fn register_agg(env: &Env) -> PriceAggregatorClient<'_> {
    let owner = Address::generate(env);
    let id = env.register(PriceAggregator, (owner,));
    PriceAggregatorClient::new(env, &id)
}

/// Dual config whose anchor is a Reflector TWAP source asking for zero records.
/// `validate_twap_records` rejects that in both read disciplines, so the anchor
/// leg reverts rather than resolving to an unreadable `Leg`.
fn dual_with_unreadable_twap_anchor(
    env: &Env,
    feed: &Address,
    primary_id: &str,
    asset: &Address,
) -> AssetOracleConfig {
    let reflector = env.register(EmptyReflector, ());
    let mut config = redstone_dual(env, feed, primary_id, "ANCHOR", MAX_STALE, 10_500, 9_500);
    config.anchor =
        OracleSourceConfigOption::Some(OracleSourceConfig::Reflector(ReflectorSourceConfig {
            contract: reflector,
            asset: OracleAssetRef::Stellar(asset.clone()),
            read_mode: OracleReadMode::Twap(0),
            decimals: 14,
            resolution_seconds: 300,
            base: ReflectorBase::Usd,
        }));
    config
}

// Control: with a readable, fresh primary the broken anchor is reached and
// raises its own error.
#[test]
fn broken_anchor_alone_reverts_twap_error() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = NOW);
    let client = register_agg(&env);
    let asset = Address::generate(&env);
    let (feed, feed_client) = register_redstone_feed(&env);
    feed_client.set_price(&String::from_str(&env, "PRIMARY"), &WAD);
    client.seed_oracle_config(
        &asset,
        &dual_with_unreadable_twap_anchor(&env, &feed, "PRIMARY", &asset),
    );

    assert_eq!(
        client.try_price(&asset).unwrap_err(),
        Ok(soroban_sdk::Error::from_contract_error(
            OracleError::TwapInsufficientObservations as u32
        ))
    );
}

// Primary unreadable *and* anchor broken: the primary's error wins.
#[test]
fn unreadable_primary_outranks_broken_anchor() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = NOW);
    let client = register_agg(&env);
    let asset = Address::generate(&env);
    // "MISSING" is never populated on the mock feed.
    let (feed, _feed_client) = register_redstone_feed(&env);
    client.seed_oracle_config(
        &asset,
        &dual_with_unreadable_twap_anchor(&env, &feed, "MISSING", &asset),
    );

    assert_eq!(
        client.try_price(&asset).unwrap_err(),
        Ok(soroban_sdk::Error::from_contract_error(
            GenericError::InvalidTicker as u32
        ))
    );
}

// A quote asset broken in both legs, reached through a quoted base: the
// quote's primary error surfaces, not its anchor's read-time revert.
//
// Every config here is one `set_oracle_config` accepts; only the runtime fails.
// The quote's soft traversal runs underneath the hard path, so an anchor read
// it does not need is a revert the hard path cannot catch — `price` would
// report `OracleNotConfigured` (#216) from a contract it merely quotes against,
// rather than `InvalidTicker` (#3) for the feed that is actually missing.
#[test]
fn quoted_base_reports_the_quote_primary_error_not_its_anchor_revert() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = NOW);
    let client = register_agg(&env);
    let asset = Address::generate(&env);
    let quote = Address::generate(&env);

    // Quote: RedStone primary whose feed id is never populated, anchored on a
    // Reflector contract that reverts when read.
    let (feed, _feed_client) = register_redstone_feed(&env);
    let reverting = env.register(RevertingReflector, ());
    client.seed_oracle_config(
        &quote,
        &redstone_primary_reflector_anchor(&env, &feed, "MISSING", &reverting, &quote, MAX_STALE),
    );

    // Asset: a readable Reflector spot priced in `quote`.
    let priced = env.register(PricedReflector, ());
    client.seed_oracle_config(
        &asset,
        &reflector_quoted(&priced, &asset, &quote, MAX_STALE),
    );

    assert_eq!(
        client.try_price(&asset).unwrap_err(),
        Ok(soroban_sdk::Error::from_contract_error(
            GenericError::InvalidTicker as u32
        ))
    );
}

// Primary readable but stale *and* anchor broken: staleness is decided before
// the anchor is read, so the primary's error still wins.
#[test]
fn stale_primary_outranks_broken_anchor() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = NOW);
    let client = register_agg(&env);
    let asset = Address::generate(&env);
    let (feed, feed_client) = register_redstone_feed(&env);
    feed_client.set_price(&String::from_str(&env, "PRIMARY"), &WAD);
    client.seed_oracle_config(
        &asset,
        &dual_with_unreadable_twap_anchor(&env, &feed, "PRIMARY", &asset),
    );

    // Advance well past the primary's max-stale window.
    env.ledger().with_mut(|li| li.timestamp = NOW + 10_000);

    assert_eq!(
        client.try_price(&asset).unwrap_err(),
        Ok(soroban_sdk::Error::from_contract_error(
            OracleError::PriceFeedStale as u32
        ))
    );
}
