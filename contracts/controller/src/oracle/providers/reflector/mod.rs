// Reflector SEP-40 price provider.

mod spot;
mod twap;

use common::errors::OracleError;
use common::math::fp::Wad;
use common::oracle::observation::check_not_future_at;
use common::oracle::providers::reflector::ReflectorPriceData;
use controller_interface::types::{
    MarketStatus, OracleReadMode, OracleSourceConfig, PriceFeedRaw, ReflectorBase,
    ReflectorSourceConfig,
};
use soroban_sdk::{panic_with_error, Address, Env};

use super::super::observation::{build_observation, OracleObservation};
use crate::cache::Cache;

pub(crate) fn read_reflector_source(
    cache: &mut Cache,
    config: &ReflectorSourceConfig,
    max_stale: u64,
    required: bool,
) -> Option<OracleObservation> {
    let observation = match config.read_mode {
        OracleReadMode::Spot => spot::read_spot(cache.env(), config, required),
        OracleReadMode::Twap(records) => {
            twap::read_twap(cache, config, records, max_stale, required)
        }
    };
    observation.map(|obs| reprice_to_usd(cache, &config.base, obs))
}

/// Converts a Reflector observation into USD WAD.
fn reprice_to_usd(
    cache: &mut Cache,
    base: &ReflectorBase,
    obs: OracleObservation,
) -> OracleObservation {
    match base {
        ReflectorBase::Usd => obs,
        ReflectorBase::Quoted(quote) => {
            let env = cache.env().clone();
            let quote_feed = resolve_usd_quote(cache, quote);
            let price_usd = Wad::from(obs.price_wad)
                .mul(&env, Wad::from(quote_feed.price_wad))
                .raw();
            OracleObservation {
                price_wad: price_usd,
                // The composite is only as fresh as its staler leg: bound the
                // token timestamp by the quote's so stale-tolerating policies
                // see the quote's age too.
                observed_at: obs.observed_at.min(quote_feed.timestamp),
                published_at: obs.published_at,
            }
        }
    }
}

/// Resolves the USD price of a quote asset for repricing.
fn resolve_usd_quote(cache: &mut Cache, quote: &Address) -> PriceFeedRaw {
    let env = cache.env().clone();
    let market = cache.cached_market_config(quote);
    if market.status != MarketStatus::Active {
        panic_with_error!(&env, OracleError::InvalidOracleBase);
    }
    match &market.oracle_config.primary {
        // RedStone feeds are USD-denominated by construction.
        OracleSourceConfig::RedStone(_) => {}
        // A Reflector quote source must itself be USD-based (no chaining).
        // Use the base cached at config time; do not call live `base()`.
        OracleSourceConfig::Reflector(r) => match &r.base {
            ReflectorBase::Usd => {}
            _ => panic_with_error!(&env, OracleError::InvalidOracleBase),
        },
    }
    crate::oracle::token_price(cache, quote)
}

pub(crate) fn observation_from_price_data(
    env: &Env,
    pd: &ReflectorPriceData,
    decimals: u32,
) -> OracleObservation {
    check_not_future_at(env, env.ledger().timestamp(), pd.timestamp);
    build_observation(env, pd.price, decimals, pd.timestamp, None)
}
