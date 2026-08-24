//! Configuration types for the composable price-aggregator oracle: how an asset's price is
//! assembled from one or two underlying sources (direct feeds, factor-scaled feeds, or Aquarius
//! LP token valuations), with independence and sanity-bound checks applied across sources.

use soroban_sdk::{contracttype, Address, String, Symbol, Vec};

use super::oracle::{OracleAssetRef, OracleReadMode, OracleTolerance};

/// Maximum recursion depth allowed while resolving a `PriceKey`, counting nested lookups
/// through `ScaledSource::quote` and `AquariusLpSource` leg keys.
pub const MAX_RESOLUTION_DEPTH: u32 = 3;

/// Minimum number of `PriceSource` entries an `AssetOracle` may declare.
pub const MIN_SOURCES: u32 = 1;
/// Maximum number of `PriceSource` entries an `AssetOracle` may declare.
pub const MAX_SOURCES: u32 = 2;

/// Identifies a priceable asset: either a token contract address or a named synthetic
/// reference used as an intermediate quote in composed sources.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PriceKey {
    Token(Address),

    Ref(Symbol),
}

/// References a price feed on a Reflector oracle contract, together with the read mode
/// (spot or TWAP) used to sample it.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReflectorFeedRef {
    pub contract: Address,
    pub asset: OracleAssetRef,
    pub read_mode: OracleReadMode,
}

/// Classifies a feed as tracking a live market price or a slower-moving fundamental value.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FeedNature {
    Market,

    Fundamental,
}

/// References a feed on a RedStone or Xoxno multi-feed provider contract, identified by
/// feed ID, together with its `FeedNature`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MultiFeedRef {
    pub contract: Address,
    pub feed_id: String,
    pub nature: FeedNature,
}

/// Identifies which provider serves a `FeedSource` and how to read it.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProviderRef {
    Reflector(ReflectorFeedRef),
    RedStone(MultiFeedRef),
    Xoxno(MultiFeedRef),
}

impl ProviderRef {
    /// Returns the address of the provider contract backing this reference.
    pub fn contract(&self) -> &Address {
        match self {
            ProviderRef::Reflector(r) => &r.contract,
            ProviderRef::RedStone(r) | ProviderRef::Xoxno(r) => &r.contract,
        }
    }

    /// Returns true if the reference uses multi-observation (non-spot) sampling
    /// rather than a spot price. Reflector references are smoothed when their
    /// read mode is `Twap` (equal-weight mean of records); RedStone and Xoxno
    /// references are never smoothed.
    fn is_smoothed(&self) -> bool {
        match self {
            ProviderRef::Reflector(r) => matches!(r.read_mode, OracleReadMode::Twap(_)),
            ProviderRef::RedStone(_) | ProviderRef::Xoxno(_) => false,
        }
    }

    /// Returns the feed's nature. Reflector references are always `Market`; RedStone and
    /// Xoxno references carry an explicit `FeedNature`.
    pub fn nature(&self) -> FeedNature {
        match self {
            ProviderRef::Reflector(_) => FeedNature::Market,
            ProviderRef::RedStone(r) | ProviderRef::Xoxno(r) => r.nature,
        }
    }

    /// Returns true if the reference is a market-nature feed read without smoothing.
    pub fn is_unsmoothed_market_leg(&self) -> bool {
        self.nature() == FeedNature::Market && !self.is_smoothed()
    }
}

/// A single price feed source: a provider reference, the decimal scale of the returned
/// price, and how stale a reading may be before it is rejected.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeedSource {
    pub provider: ProviderRef,
    pub decimals: u32,
    pub max_stale_seconds: u64,
}

/// A price derived by reading a factor feed and multiplying it by the price resolved for
/// `quote`. The factor reading is rejected if it falls outside `[min_factor_wad,
/// max_factor_wad]`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScaledSource {
    pub factor: FeedSource,
    pub quote: PriceKey,

    pub min_factor_wad: i128,

    pub max_factor_wad: i128,
}

/// Configuration for pricing an Aquarius liquidity-pool share token from its two underlying
/// reserves. `key_a`/`key_b` resolve the prices of `token_a`/`token_b`, and
/// `reserve_a_decimals`/`reserve_b_decimals` give each reserve's token decimals. A pool
/// whose computed value falls below `min_pool_value_wad` is treated as unpriceable.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AquariusLpSource {
    pub pool: Address,

    pub token_a: Address,
    pub token_b: Address,
    pub key_a: PriceKey,
    pub key_b: PriceKey,

    pub reserve_a_decimals: u32,

    pub reserve_b_decimals: u32,

    pub min_pool_value_wad: i128,
}

/// One input used to price an asset: a direct feed, a factor-scaled feed, or an Aquarius LP
/// share valuation for a constant-product (`AquariusLp`) or stable-swap (`AquariusStableLp`)
/// pool.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum PriceSource {
    Feed(FeedSource),
    Scaled(ScaledSource),
    AquariusLp(AquariusLpSource),

    AquariusStableLp(AquariusLpSource),
}

impl PriceSource {
    /// Returns true for either Aquarius LP variant.
    pub fn is_aquarius_lp(&self) -> bool {
        matches!(
            self,
            PriceSource::AquariusLp(_) | PriceSource::AquariusStableLp(_)
        )
    }
}

/// Governs whether the two sources of a dual-source `AssetOracle` are required to depend on
/// disjoint underlying contracts, or may share a declared, explicit set of addresses.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IndependencePolicy {
    RequireDisjoint,

    AllowShared(Vec<Address>),
}

/// Full pricing configuration for one asset: its price decimals, the maximum age a blended
/// price may have, one or two `PriceSource` entries composed to form the price, the
/// tolerance band checked between dual sources, the independence policy applied to those
/// sources, and the sanity bounds a resolved price must fall within.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssetOracle {
    pub asset_decimals: u32,

    pub max_price_stale_seconds: u64,

    pub sources: Vec<PriceSource>,

    pub tolerance: OracleTolerance,
    pub independence: IndependencePolicy,

    pub min_sanity_price_wad: i128,

    pub max_sanity_price_wad: i128,
}

impl AssetOracle {
    /// Returns true if the oracle composes its price from two sources.
    pub fn is_dual(&self) -> bool {
        self.sources.len() == 2
    }

    /// Returns true if the first configured source is an Aquarius LP valuation.
    pub fn has_aquarius_lp_source(&self) -> bool {
        self.sources
            .get(0)
            .is_some_and(|source| source.is_aquarius_lp())
    }
}

#[cfg(test)]
#[path = "../../tests/types/composable_oracle.rs"]
mod tests;
