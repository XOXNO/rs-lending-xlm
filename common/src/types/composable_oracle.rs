use soroban_sdk::{contracttype, Address, String, Symbol, Vec};

use super::oracle::{OracleAssetRef, OracleReadMode, OracleTolerance};

pub const MAX_RESOLUTION_DEPTH: u32 = 3;

pub const MIN_SOURCES: u32 = 1;
pub const MAX_SOURCES: u32 = 2;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PriceKey {
    Token(Address),

    Ref(Symbol),
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReflectorFeedRef {
    pub contract: Address,
    pub asset: OracleAssetRef,
    pub read_mode: OracleReadMode,
}

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FeedNature {
    Market,

    Fundamental,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MultiFeedRef {
    pub contract: Address,
    pub feed_id: String,
    pub nature: FeedNature,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProviderRef {
    Reflector(ReflectorFeedRef),
    RedStone(MultiFeedRef),
    Xoxno(MultiFeedRef),
}

impl ProviderRef {
    pub fn contract(&self) -> &Address {
        match self {
            ProviderRef::Reflector(r) => &r.contract,
            ProviderRef::RedStone(r) | ProviderRef::Xoxno(r) => &r.contract,
        }
    }

    pub fn is_smoothed(&self) -> bool {
        match self {
            ProviderRef::Reflector(r) => matches!(r.read_mode, OracleReadMode::Twap(_)),
            ProviderRef::RedStone(_) | ProviderRef::Xoxno(_) => false,
        }
    }

    pub fn nature(&self) -> FeedNature {
        match self {
            ProviderRef::Reflector(_) => FeedNature::Market,
            ProviderRef::RedStone(r) | ProviderRef::Xoxno(r) => r.nature,
        }
    }

    pub fn is_unsmoothed_market_leg(&self) -> bool {
        self.nature() == FeedNature::Market && !self.is_smoothed()
    }
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeedSource {
    pub provider: ProviderRef,
    pub decimals: u32,
    pub max_stale_seconds: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScaledSource {
    pub factor: FeedSource,
    pub quote: PriceKey,

    pub min_factor_wad: i128,

    pub max_factor_wad: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AquariusLpSource {
    pub pool: Address,
    /// The pool's reserve-mirror plane (`pool.get_pools_plane()`), captured at
    /// listing; reserves are read from here so the pricing path stays read-only.
    pub plane: Address,
    /// Reserve-token identities in the exact order returned by `pool.get_tokens()`.
    pub token_a: Address,
    pub token_b: Address,
    pub key_a: PriceKey,
    pub key_b: PriceKey,

    pub reserve_a_decimals: u32,

    pub reserve_b_decimals: u32,

    /// Minimum manipulation-resistant pool value, in USD WAD.
    pub min_pool_value_wad: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum PriceSource {
    Feed(FeedSource),
    Scaled(ScaledSource),
    AquariusLp(AquariusLpSource),
    /// Stableswap (Curve-style) LP share. Same binding payload as
    /// [`PriceSource::AquariusLp`]; the variant selects the invariant-`D`
    /// pricing path, and the amplification is read live from the pool.
    AquariusStableLp(AquariusLpSource),
}

impl PriceSource {
    /// Either Aquarius LP-share pricing path (constant-product or stableswap).
    /// Both are sole-source oracles priced from pool reserves against two
    /// independently-banded underlyings.
    pub fn is_aquarius_lp(&self) -> bool {
        matches!(
            self,
            PriceSource::AquariusLp(_) | PriceSource::AquariusStableLp(_)
        )
    }
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IndependencePolicy {
    RequireDisjoint,

    AllowShared(Vec<Address>),
}

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
    pub fn is_dual(&self) -> bool {
        self.sources.len() == 2
    }

    /// LP-share pricing derives from two independently-banded underlyings, so —
    /// like a dual-source oracle — its own sanity band is a wide backstop, not a
    /// tight single-feed guard, and is exempt from the single-source width cap.
    pub fn has_aquarius_lp_source(&self) -> bool {
        self.sources.get(0).is_some_and(|source| source.is_aquarius_lp())
    }
}

#[cfg(test)]
#[path = "../../tests/types/composable_oracle.rs"]
mod tests;
