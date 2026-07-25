//! Pins the exact error `price()` raises for each failure mode.
//!
//! These are characterization tests: they encode today's behavior so the
//! hard/soft path unification can be proven not to change it. If one of these
//! changes, the ABI's error contract changed and downstream consumers break.
//!
//! One test per row of the hard-path failure table in
//! `docs/superpowers/plans/2026-07-25-price-aggregator-unify-price-paths.md`.
//! Rows 2 and 5 each split into two sub-cases (Reflector vs RedStone/Xoxno
//! provider family), so there are more tests than table rows.

use super::*;
use common::constants::WAD;
use common::errors::GenericError;
use common::oracle::providers::reflector::{ReflectorAsset, ReflectorOracle, ReflectorPriceData};
use common::types::{
    OracleAssetRef, OracleReadMode, OracleSourceConfig, OracleSourceConfigOption, OracleStrategy,
    OracleTolerance, RedStoneSourceConfig, ReflectorBase, ReflectorSourceConfig,
};
use mock_redstone::{MockRedStonePriceFeed, MockRedStonePriceFeedClient};
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{contract, contractimpl, Env, String, Symbol, Vec};

use crate::{PriceAggregator, PriceAggregatorClient};

/// Reflector-shaped stub that always reports no last price. The row-2
/// Reflector-unreadable case needs a genuinely registered contract: an
/// unregistered address traps with a host `InvalidAction` error, not the
/// provider's own `NoLastPrice`.
#[contract]
struct EmptyReflector;

#[contractimpl]
impl ReflectorOracle for EmptyReflector {
    fn base(env: Env) -> ReflectorAsset {
        ReflectorAsset::Other(Symbol::new(&env, "USD"))
    }

    fn decimals(_env: Env) -> u32 {
        14
    }

    fn resolution(_env: Env) -> u32 {
        300
    }

    fn lastprice(_env: Env, _asset: ReflectorAsset) -> Option<ReflectorPriceData> {
        None
    }

    fn prices(_env: Env, _asset: ReflectorAsset, _records: u32) -> Option<Vec<ReflectorPriceData>> {
        None
    }
}

fn register_agg(env: &Env) -> (Address, PriceAggregatorClient<'_>) {
    let owner = Address::generate(env);
    let id = env.register(PriceAggregator, (owner.clone(),));
    (owner, PriceAggregatorClient::new(env, &id))
}

fn register_redstone_feed(env: &Env) -> (Address, MockRedStonePriceFeedClient<'_>) {
    let id = env.register(MockRedStonePriceFeed, ());
    (id.clone(), MockRedStonePriceFeedClient::new(env, &id))
}

fn redstone_single(env: &Env, feed: &Address, feed_id: &str, max_stale: u64) -> AssetOracleConfig {
    AssetOracleConfig {
        asset_decimals: 7,
        max_price_stale_seconds: max_stale,
        tolerance: OracleTolerance {
            upper_ratio_bps: 10_000,
            lower_ratio_bps: 10_000,
        },
        strategy: OracleStrategy::Single,
        primary: OracleSourceConfig::RedStone(RedStoneSourceConfig {
            contract: feed.clone(),
            feed_id: String::from_str(env, feed_id),
            decimals: 8,
            max_stale_seconds: max_stale,
        }),
        anchor: OracleSourceConfigOption::None,
        min_sanity_price_wad: WAD - WAD / 20,
        max_sanity_price_wad: WAD + WAD / 20,
    }
}

fn redstone_dual(
    env: &Env,
    feed: &Address,
    primary_id: &str,
    anchor_id: &str,
    max_stale: u64,
    upper_bps: u32,
    lower_bps: u32,
) -> AssetOracleConfig {
    AssetOracleConfig {
        asset_decimals: 7,
        max_price_stale_seconds: max_stale,
        tolerance: OracleTolerance {
            upper_ratio_bps: upper_bps,
            lower_ratio_bps: lower_bps,
        },
        strategy: OracleStrategy::PrimaryWithAnchor,
        primary: OracleSourceConfig::RedStone(RedStoneSourceConfig {
            contract: feed.clone(),
            feed_id: String::from_str(env, primary_id),
            decimals: 8,
            max_stale_seconds: max_stale,
        }),
        anchor: OracleSourceConfigOption::Some(OracleSourceConfig::RedStone(
            RedStoneSourceConfig {
                contract: feed.clone(),
                feed_id: String::from_str(env, anchor_id),
                decimals: 8,
                max_stale_seconds: max_stale,
            },
        )),
        min_sanity_price_wad: WAD / 2,
        max_sanity_price_wad: WAD * 2,
    }
}

fn reflector_single(reflector: &Address, asset: &Address, max_stale: u64) -> AssetOracleConfig {
    AssetOracleConfig {
        asset_decimals: 7,
        max_price_stale_seconds: max_stale,
        tolerance: OracleTolerance {
            upper_ratio_bps: 10_000,
            lower_ratio_bps: 10_000,
        },
        strategy: OracleStrategy::Single,
        primary: OracleSourceConfig::Reflector(ReflectorSourceConfig {
            contract: reflector.clone(),
            asset: OracleAssetRef::Stellar(asset.clone()),
            read_mode: OracleReadMode::Spot,
            decimals: 14,
            resolution_seconds: 300,
            base: ReflectorBase::Usd,
        }),
        anchor: OracleSourceConfigOption::None,
        min_sanity_price_wad: WAD - WAD / 20,
        max_sanity_price_wad: WAD + WAD / 20,
    }
}

// Row 1: config is the `pending_for` self-pointer sentinel.
#[test]
fn pending_config_reverts_oracle_not_configured() {
    let env = Env::default();
    env.mock_all_auths();
    let (_owner, client) = register_agg(&env);
    let asset = Address::generate(&env);
    client.seed_oracle_config(&asset, &AssetOracleConfig::pending_for(asset.clone(), 7));

    assert_eq!(
        client.try_price(&asset).unwrap_err(),
        Ok(soroban_sdk::Error::from_contract_error(
            OracleError::OracleNotConfigured as u32
        ))
    );
}

// Row 2 (Reflector): primary source unreadable.
#[test]
fn primary_reflector_unreadable_reverts_no_last_price() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.timestamp = 1_700_000_000;
    });
    let (_owner, client) = register_agg(&env);
    let asset = Address::generate(&env);
    let reflector = env.register(EmptyReflector, ());
    client.seed_oracle_config(&asset, &reflector_single(&reflector, &asset, 900));

    assert_eq!(
        client.try_price(&asset).unwrap_err(),
        Ok(soroban_sdk::Error::from_contract_error(
            OracleError::NoLastPrice as u32
        ))
    );
}

// Row 2 (RedStone/Xoxno): primary source unreadable.
#[test]
fn primary_redstone_unreadable_reverts_invalid_ticker() {
    let env = Env::default();
    env.mock_all_auths();
    let (_owner, client) = register_agg(&env);
    let asset = Address::generate(&env);
    let (feed, _feed_client) = register_redstone_feed(&env);
    // Config points at a feed id that was never populated.
    client.seed_oracle_config(&asset, &redstone_single(&env, &feed, "MISSING", 900));

    assert_eq!(
        client.try_price(&asset).unwrap_err(),
        Ok(soroban_sdk::Error::from_contract_error(
            GenericError::InvalidTicker as u32
        ))
    );
}

// Row 3: primary older than its max-stale.
#[test]
fn primary_stale_reverts_price_feed_stale() {
    let env = Env::default();
    env.mock_all_auths();
    let now: u64 = 1_000;
    env.ledger().with_mut(|li| {
        li.timestamp = now;
    });
    let (_owner, client) = register_agg(&env);
    let asset = Address::generate(&env);
    let (feed, feed_client) = register_redstone_feed(&env);
    feed_client.set_price(&String::from_str(&env, "BTC/USD"), &WAD);
    client.seed_oracle_config(&asset, &redstone_single(&env, &feed, "BTC/USD", 60));

    // Advance well past max_stale_seconds.
    env.ledger().with_mut(|li| {
        li.timestamp = now + 10_000;
    });

    assert_eq!(
        client.try_price(&asset).unwrap_err(),
        Ok(soroban_sdk::Error::from_contract_error(
            OracleError::PriceFeedStale as u32
        ))
    );
}

// Row 4: strategy is dual but `config.anchor` is `None`.
#[test]
fn dual_missing_anchor_reverts_no_last_price() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.timestamp = 1_700_000_000;
    });
    let (_owner, client) = register_agg(&env);
    let asset = Address::generate(&env);
    let (feed, feed_client) = register_redstone_feed(&env);
    feed_client.set_price(&String::from_str(&env, "BTC/USD"), &WAD);

    let mut cfg = redstone_single(&env, &feed, "BTC/USD", 900);
    cfg.strategy = OracleStrategy::PrimaryWithAnchor;
    cfg.anchor = OracleSourceConfigOption::None;
    client.seed_oracle_config(&asset, &cfg);

    assert_eq!(
        client.try_price(&asset).unwrap_err(),
        Ok(soroban_sdk::Error::from_contract_error(
            OracleError::NoLastPrice as u32
        ))
    );
}

// Row 5: anchor unreadable — same error as row 2's RedStone/Xoxno sub-case,
// raised from the anchor leg instead of the primary leg.
#[test]
fn anchor_unreadable_reverts_invalid_ticker() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.timestamp = 1_700_000_000;
    });
    let (_owner, client) = register_agg(&env);
    let asset = Address::generate(&env);
    let (feed, feed_client) = register_redstone_feed(&env);
    feed_client.set_price(&String::from_str(&env, "PRIMARY"), &WAD);
    // Anchor feed id never populated.
    client.seed_oracle_config(
        &asset,
        &redstone_dual(&env, &feed, "PRIMARY", "ANCHOR", 900, 10_500, 9_500),
    );

    assert_eq!(
        client.try_price(&asset).unwrap_err(),
        Ok(soroban_sdk::Error::from_contract_error(
            GenericError::InvalidTicker as u32
        ))
    );
}

// Row 5: anchor stale — same error as row 3, raised from the anchor leg.
#[test]
fn anchor_stale_reverts_price_feed_stale() {
    let env = Env::default();
    env.mock_all_auths();
    let now: u64 = 1_700_000_000;
    env.ledger().with_mut(|li| {
        li.timestamp = now;
    });
    let (_owner, client) = register_agg(&env);
    let asset = Address::generate(&env);
    let (feed, feed_client) = register_redstone_feed(&env);
    feed_client.set_price(&String::from_str(&env, "PRIMARY"), &WAD);
    let stale_ms = (now - 10_000) * 1_000;
    feed_client.set_price_data(
        &String::from_str(&env, "ANCHOR"),
        &WAD,
        &stale_ms,
        &stale_ms,
    );
    client.seed_oracle_config(
        &asset,
        &redstone_dual(&env, &feed, "PRIMARY", "ANCHOR", 900, 10_500, 9_500),
    );

    assert_eq!(
        client.try_price(&asset).unwrap_err(),
        Ok(soroban_sdk::Error::from_contract_error(
            OracleError::PriceFeedStale as u32
        ))
    );
}

// Row 6: primary vs anchor outside the tolerance band.
#[test]
fn dual_out_of_band_reverts_unsafe_price_not_allowed() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.timestamp = 1_700_000_000;
    });
    let (_owner, client) = register_agg(&env);
    let asset = Address::generate(&env);
    let (feed, feed_client) = register_redstone_feed(&env);
    feed_client.set_price(&String::from_str(&env, "PRIMARY"), &WAD);
    feed_client.set_price(&String::from_str(&env, "ANCHOR"), &(WAD * 2));
    client.seed_oracle_config(
        &asset,
        &redstone_dual(&env, &feed, "PRIMARY", "ANCHOR", 900, 10_500, 9_500),
    );

    assert_eq!(
        client.try_price(&asset).unwrap_err(),
        Ok(soroban_sdk::Error::from_contract_error(
            OracleError::UnsafePriceNotAllowed as u32
        ))
    );
}

// Row 7: final price `<= 0`.
#[test]
fn non_positive_price_reverts_invalid_price() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.timestamp = 1_700_000_000;
    });
    let (_owner, client) = register_agg(&env);
    let asset = Address::generate(&env);
    let (feed, feed_client) = register_redstone_feed(&env);
    feed_client.set_price_data(
        &String::from_str(&env, "BTC/USD"),
        &0i128,
        &(1_700_000_000u64 * 1_000),
        &(1_700_000_000u64 * 1_000),
    );
    client.seed_oracle_config(&asset, &redstone_single(&env, &feed, "BTC/USD", 900));

    assert_eq!(
        client.try_price(&asset).unwrap_err(),
        Ok(soroban_sdk::Error::from_contract_error(
            OracleError::InvalidPrice as u32
        ))
    );
}

// Row 8: final price outside the sanity band.
#[test]
fn outside_sanity_band_reverts_sanity_bound_violated() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.timestamp = 1_700_000_000;
    });
    let (_owner, client) = register_agg(&env);
    let asset = Address::generate(&env);
    let (feed, feed_client) = register_redstone_feed(&env);
    feed_client.set_price(&String::from_str(&env, "BTC/USD"), &WAD);

    let mut cfg = redstone_single(&env, &feed, "BTC/USD", 900);
    cfg.min_sanity_price_wad = WAD * 10;
    cfg.max_sanity_price_wad = WAD * 20;
    client.seed_oracle_config(&asset, &cfg);

    assert_eq!(
        client.try_price(&asset).unwrap_err(),
        Ok(soroban_sdk::Error::from_contract_error(
            OracleError::SanityBoundViolated as u32
        ))
    );
}

// Row 8 (sub-case): sanity band disabled (`max_sanity_price_wad <= 0`).
#[test]
fn sanity_band_disabled_reverts_sanity_bound_violated() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.timestamp = 1_700_000_000;
    });
    let (_owner, client) = register_agg(&env);
    let asset = Address::generate(&env);
    let (feed, feed_client) = register_redstone_feed(&env);
    feed_client.set_price(&String::from_str(&env, "BTC/USD"), &WAD);

    let mut cfg = redstone_single(&env, &feed, "BTC/USD", 900);
    cfg.max_sanity_price_wad = 0;
    client.seed_oracle_config(&asset, &cfg);

    assert_eq!(
        client.try_price(&asset).unwrap_err(),
        Ok(soroban_sdk::Error::from_contract_error(
            OracleError::SanityBoundViolated as u32
        ))
    );
}
