mod compose;
mod observation;
pub mod policy;
mod providers;
pub mod reflector;
mod tolerance;
pub(crate) mod validation;

use common::errors::{GenericError, OracleError};
use common::rates::simulate_update_indexes;
use common::types::{MarketIndex, MarketStatus, PriceFeed};
use soroban_sdk::{panic_with_error, Address};

use crate::cache::ControllerCache;

#[allow(unused_imports)]
pub(crate) use tolerance::{calculate_final_price, is_within_anchor};

crate::summarized!(
    token_price_summary,
    pub fn token_price(cache: &mut ControllerCache, asset: &Address) -> PriceFeed {
        if let Some(feed) = cache.try_get_price(asset) {
            return feed;
        }

        let market = cache.cached_market_config(asset);
        match market.status {
            MarketStatus::PendingOracle => {
                panic_with_error!(cache.env(), GenericError::PairNotActive);
            }
            MarketStatus::Disabled if !cache.oracle_policy.allows_disabled_market() => {
                panic_with_error!(cache.env(), GenericError::PairNotActive);
            }
            _ => {}
        }

        let config = market.oracle_config;
        let resolved = compose::resolve_price(cache, &config);
        if resolved.price_wad <= 0 {
            panic_with_error!(cache.env(), OracleError::InvalidPrice);
        }
        let feed = PriceFeed {
            price_wad: resolved.price_wad,
            asset_decimals: config.asset_decimals,
            timestamp: resolved.timestamp,
        };

        cache.set_price(asset, &feed);
        feed
    }
);

pub fn price_components(
    cache: &mut ControllerCache,
    asset: &Address,
) -> (Option<i128>, Option<i128>, i128, bool, bool) {
    let market = cache.cached_market_config(asset);
    let configs = market.oracle_config;
    let components = compose::resolve_components(cache, &configs);
    (
        components.anchor_price_wad,
        components.primary_price_wad,
        components.final_price_wad,
        components.within_first_tolerance,
        components.within_second_tolerance,
    )
}

crate::summarized!(
    update_asset_index_summary,
    pub fn update_asset_index(cache: &mut ControllerCache, asset: &Address) -> MarketIndex {
        let env = cache.env().clone();
        let sync_data = cache.cached_pool_sync_data(asset);
        simulate_update_indexes(&env, cache.current_timestamp_ms, &sync_data)
    }
);
