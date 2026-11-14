//! Model of the controller `Context` (`contracts/controller/src/context.rs`)
//! restricted to what the extracted liquidation and risk code reads: the
//! invocation's price cache and market-index cache.
//!
//! * `cached_price` is copied verbatim: it reads the price cache and aborts
//!   (`OracleNotConfigured`) when the asset has no cached price.
//! * `cached_market_index` is the production body with the pool fetch folded
//!   into one call: on a cache miss it asks the pool for the simulated
//!   current index (`external::fetch_market_index`, opaque) and caches it.
//! * `load_markets` is opaque (Charon `--opaque`); its Lean model in
//!   `lean/ControllerAeneas/FunsExternal.lean` fills the missing cache
//!   entries from the price oracle and the pool.
//! * `cached_pool_address`, `require_spoke_asset`, `apply_spoke_exit` and
//!   `record_supply_position_update` (used by `liquidation/apply.rs`) are
//!   opaque: spoke configuration, spoke usage and event buffers are an
//!   uninterpreted part of the state in the Lean model.
//!
//! Spoke usage, configuration caches, event buffers and the pool-address
//! cache of the production `Context` are not read by the extracted functions
//! and are left out. See `../docs/extraction.md`, "Context model".

use crate::errors::OracleError;
use crate::events::PositionAction;
use crate::external::fetch_market_index;
use crate::math::fp::Ray;
use crate::spoke_usage::UsageSide;
use crate::types::{
    AccountPosition, AssetConfig, HubAssetKey, MarketIndex, MarketIndexRaw, PriceFeed,
    PriceFeedRaw,
};
use soroban_sdk::{panic_with_error, Address, Env, Map, Vec};

pub struct Context {
    pub(crate) env: Env,
    pub(crate) token_prices: Map<Address, PriceFeedRaw>,
    pub(crate) market_indexes: Map<HubAssetKey, MarketIndexRaw>,
}

impl Context {
    /// Loads missing prices by token address and missing indexes by hub-asset key.
    /// Already cached values are retained. Opaque; modelled in `FunsExternal.lean`.
    pub(crate) fn load_markets(&mut self, _hub_assets: &Vec<HubAssetKey>) {
        unimplemented!("opaque: modelled in FunsExternal.lean")
    }

    /// Returns a cached index or fetches its simulated current value from the pool.
    pub(crate) fn cached_market_index(&mut self, hub_asset: &HubAssetKey) -> MarketIndex {
        if let Some(index) = self.market_indexes.get(hub_asset.clone()) {
            return (&index).into();
        }
        let index = fetch_market_index(&self.env, hub_asset);
        self.market_indexes.set(hub_asset.clone(), index.clone());
        (&index).into()
    }

    /// Returns the pool address. Opaque.
    pub(crate) fn cached_pool_address(&mut self) -> Address {
        unimplemented!("opaque: modelled in FunsExternal.lean")
    }

    /// Returns the listed asset config; fails if it is unlisted. Opaque.
    pub(crate) fn require_spoke_asset(&mut self, _spoke_id: u32, _hub_asset: &HubAssetKey) -> AssetConfig {
        unimplemented!("opaque: modelled in FunsExternal.lean")
    }

    /// Records a spoke usage exit. Opaque.
    pub(crate) fn apply_spoke_exit(
        &mut self,
        _spoke_id: u32,
        _side: UsageSide,
        _hub_asset: &HubAssetKey,
        _delta_scaled: Ray,
    ) {
        unimplemented!("opaque: modelled in FunsExternal.lean")
    }

    /// Buffers a supply position event delta. Opaque.
    pub(crate) fn record_supply_position_update(
        &mut self,
        _action: PositionAction,
        _hub_asset: &HubAssetKey,
        _index_ray: i128,
        _amount: i128,
        _position: &AccountPosition,
    ) {
        unimplemented!("opaque: modelled in FunsExternal.lean")
    }

    /// Returns a previously loaded price; fails if the cache has no entry.
    pub(crate) fn cached_price(&mut self, asset: &Address) -> PriceFeed {
        let raw = self
            .token_prices
            .get(asset.clone())
            .unwrap_or_else(|| panic_with_error!(&self.env, OracleError::OracleNotConfigured));
        (&raw).into()
    }
}
