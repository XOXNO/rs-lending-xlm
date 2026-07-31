use soroban_sdk::{contracttype, Address, Env, String, Symbol, Vec};

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
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderKind {
    Reflector,
    RedStone,
    Xoxno,
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

    pub kind: ProviderKind,
    pub nature: FeedNature,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProviderRef {
    Reflector(ReflectorFeedRef),
    MultiFeed(MultiFeedRef),
}

impl ProviderRef {
    pub fn contract(&self) -> &Address {
        match self {
            ProviderRef::Reflector(r) => &r.contract,
            ProviderRef::MultiFeed(m) => &m.contract,
        }
    }

    pub fn kind(&self) -> ProviderKind {
        match self {
            ProviderRef::Reflector(_) => ProviderKind::Reflector,
            ProviderRef::MultiFeed(m) => m.kind,
        }
    }

    pub fn is_smoothed(&self) -> bool {
        match self {
            ProviderRef::Reflector(r) => matches!(r.read_mode, OracleReadMode::Twap(_)),
            ProviderRef::MultiFeed(_) => false,
        }
    }

    pub fn nature(&self) -> FeedNature {
        match self {
            ProviderRef::Reflector(_) => FeedNature::Market,
            ProviderRef::MultiFeed(m) => m.nature,
        }
    }

    pub fn is_unsmoothed_market_leg(&self) -> bool {
        self.nature() == FeedNature::Market && !self.is_smoothed()
    }
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrustDomain {
    pub kind: ProviderKind,
    pub contract: Address,
}

impl TrustDomain {
    pub fn of(provider: &ProviderRef) -> Self {
        TrustDomain {
            kind: provider.kind(),
            contract: provider.contract().clone(),
        }
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
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PoolKind {
    ConstantProduct,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LpShareSource {
    pub pool: Address,
    pub kind: PoolKind,
    pub key_a: PriceKey,
    pub key_b: PriceKey,

    pub reserve_a_decimals: u32,

    pub reserve_b_decimals: u32,

    pub share_decimals: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PriceSource {
    Feed(FeedSource),
    Scaled(ScaledSource),
    LpShare(LpShareSource),
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IndependencePolicy {
    RequireDisjoint,

    AllowShared(Vec<TrustDomain>),
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
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceProperties {
    pub has_unsmoothed_market_leg: bool,

    pub trust: Vec<TrustDomain>,

    pub loosest_max_stale_seconds: u64,

    pub depth: u32,
}

impl SourceProperties {
    pub fn empty(env: &Env) -> Self {
        SourceProperties {
            has_unsmoothed_market_leg: false,
            trust: Vec::new(env),
            loosest_max_stale_seconds: 0,
            depth: 0,
        }
    }

    pub fn of_feed(env: &Env, feed: &FeedSource) -> Self {
        let mut trust = Vec::new(env);
        trust.push_back(TrustDomain::of(&feed.provider));
        SourceProperties {
            has_unsmoothed_market_leg: feed.provider.is_unsmoothed_market_leg(),
            trust,
            loosest_max_stale_seconds: feed.max_stale_seconds,
            depth: 0,
        }
    }

    pub fn join(&self, other: &SourceProperties) -> Self {
        let mut trust = self.trust.clone();
        for domain in other.trust.iter() {
            if !contains_domain(&trust, &domain) {
                trust.push_back(domain);
            }
        }
        SourceProperties {
            has_unsmoothed_market_leg: self.has_unsmoothed_market_leg
                || other.has_unsmoothed_market_leg,
            trust,
            loosest_max_stale_seconds: self
                .loosest_max_stale_seconds
                .max(other.loosest_max_stale_seconds),
            depth: self.depth.max(other.depth),
        }
    }

    pub fn nest(mut self) -> Self {
        self.depth += 1;
        self
    }

    pub fn shared_contracts_with(&self, env: &Env, other: &SourceProperties) -> Vec<Address> {
        let mut shared = Vec::new(env);
        for domain in self.trust.iter() {
            let in_other = other.trust.iter().any(|d| d.contract == domain.contract);
            let already = shared.iter().any(|c| c == domain.contract);
            if in_other && !already {
                shared.push_back(domain.contract.clone());
            }
        }
        shared
    }
}

pub fn contains_domain(haystack: &Vec<TrustDomain>, needle: &TrustDomain) -> bool {
    haystack.iter().any(|d| &d == needle)
}

pub struct LocalProperties {
    pub local: SourceProperties,
    pub dependencies: Vec<PriceKey>,
}

pub fn local_properties(env: &Env, source: &PriceSource) -> LocalProperties {
    match source {
        PriceSource::Feed(feed) => LocalProperties {
            local: SourceProperties::of_feed(env, feed),
            dependencies: Vec::new(env),
        },
        PriceSource::Scaled(scaled) => {
            let mut dependencies = Vec::new(env);
            dependencies.push_back(scaled.quote.clone());
            LocalProperties {
                local: SourceProperties::of_feed(env, &scaled.factor),
                dependencies,
            }
        }
        PriceSource::LpShare(lp) => {
            let mut dependencies = Vec::new(env);
            dependencies.push_back(lp.key_a.clone());
            dependencies.push_back(lp.key_b.clone());
            let mut local = SourceProperties::empty(env);

            local.has_unsmoothed_market_leg = true;
            LocalProperties {
                local,
                dependencies,
            }
        }
    }
}

#[cfg(test)]
#[path = "../../tests/types/composable_oracle.rs"]
mod tests;
