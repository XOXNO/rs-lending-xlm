//! Composable oracle model: symmetric sources, flat source shapes, recursion
//! through the price registry.
//!
//! # Why this exists alongside [`super::oracle`]
//!
//! The older model in [`super::oracle`] fixes two asymmetric roles (`primary` / `anchor`) and hard-wires
//! composition to a single shape (a Reflector feed repriced through a registered
//! token's oracle). That cannot express an asset whose only USD-independent
//! opinion is `ratio x price(reference)` — the reference need not be a token at
//! all — and it decides validation rules by matching provider enum variants
//! rather than by asking what a source actually *is*.
//!
//! Here, a market declares **one or two independent opinions**. Neither has a
//! privileged role: combination is symmetric, so swapping the two changes
//! neither the price nor the accept/reject outcome. How each opinion is formed
//! is that source's own business, described by [`PriceSource`].
//!
//! # Recursion is structural, not syntactic
//!
//! [`PriceSource`] contains no [`PriceSource`]. Nesting is *unrepresentable*,
//! which removes a class of cycle and budget bugs at the type level. Depth comes
//! instead from [`PriceKey`] indirection: a [`ScaledSource`]'s quote and an
//! [`LpShareSource`]'s legs name keys, and the engine re-enters on each key's
//! own [`AssetOracle`]. Two independent bounds apply there — the resolution
//! cycle stack, and [`MAX_RESOLUTION_DEPTH`].

use soroban_sdk::{contracttype, Address, Env, String, Symbol, Vec};

use super::oracle::{OracleAssetRef, OracleReadMode, OracleTolerance};

/// Hard ceiling on composition depth, checked on entry to every resolve.
///
/// Not redundant with the cycle guard: an acyclic graph (an LP whose leg is an
/// LP whose leg is scaled) terminates, but can still exhaust the CPU budget and
/// take the whole protocol's price path with it. Three levels covers every
/// shape we have a use for — `LpShare -> Scaled -> Feed`.
pub const MAX_RESOLUTION_DEPTH: u32 = 3;

/// Supported source count per asset, inclusive.
pub const MIN_SOURCES: u32 = 1;
pub const MAX_SOURCES: u32 = 2;

// ---------------------------------------------------------------------------
// Keys
// ---------------------------------------------------------------------------

/// What a price is *about*.
///
/// Splitting this from the market-asset key space is what makes a reference
/// price expressible. BTC has no Stellar token, but Reflector quotes it and
/// SolvBTC's ratio feed is denominated in it, so it must be nameable and
/// priceable without ever being borrowable, collateralizable, or transferable.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PriceKey {
    /// A real Stellar asset: SAC or SEP-41 contract address.
    Token(Address),
    /// A pure reference price. Registry-only: no market, no spoke listing, no
    /// balance. Written only by the owner, so symbol choice is a governance
    /// question rather than an attack surface.
    Ref(Symbol),
}

// ---------------------------------------------------------------------------
// Providers
// ---------------------------------------------------------------------------

/// Provider family. Identity for trust-domain accounting, not behaviour —
/// `RedStone` and `Xoxno` share a wire ABI but are operated by different
/// parties, so they are distinct domains even at the same contract address.
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

/// Where a feed's number comes from, which decides whether smoothing protects
/// anything.
///
/// Time-averaging defends against *market* manipulation: moving a traded price
/// for one block is cheap, moving a TWAP is not. It does nothing against a
/// publisher reporting a wrong number — that is what trust domains and a second
/// source are for.
///
/// So the rule "at least one source must be smoothed" is really "no
/// market-derived leg may be unsmoothed", and a fundamental feed (a fund NAV, a
/// wrapper's redemption ratio) is exempt because no amount of trading moves it.
/// Declared rather than inferred: nothing on-chain distinguishes
/// `SolvBTC_FUNDAMENTAL` from `BTC` at the same adapter, so governance states it
/// and the declaration rides in the config event.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FeedNature {
    /// Derived from traded prices. Manipulable by trading; needs smoothing.
    Market,
    /// A published fundamental: NAV, redemption ratio, accrual index.
    /// Manipulable only by compromising the publisher.
    Fundamental,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MultiFeedRef {
    pub contract: Address,
    pub feed_id: String,
    /// Which operator stands behind this adapter. Same ABI, different trust.
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

    /// True when this feed is time-averaged.
    ///
    /// Smoothing is a property of the *feed*, not the provider. v1 asserted
    /// "RedStone is always spot" as a variant match; that is a statement about a
    /// company, and it was both too strict and too loose. A multi-feed adapter
    /// serves a single published value with no window, so it is unsmoothed here
    /// — but the reason is the read shape, and a future adapter exposing a
    /// window would answer differently without any rule changing.
    pub fn is_smoothed(&self) -> bool {
        match self {
            ProviderRef::Reflector(r) => matches!(r.read_mode, OracleReadMode::Twap(_)),
            ProviderRef::MultiFeed(_) => false,
        }
    }

    /// Whether trading can move this number. See [`FeedNature`].
    ///
    /// Reflector aggregates traded venues, so every Reflector read is market
    /// derived regardless of window.
    pub fn nature(&self) -> FeedNature {
        match self {
            ProviderRef::Reflector(_) => FeedNature::Market,
            ProviderRef::MultiFeed(m) => m.nature,
        }
    }

    /// The condition the smoothing rule actually cares about: a leg that trading
    /// can move, read without a window.
    pub fn is_unsmoothed_market_leg(&self) -> bool {
        self.nature() == FeedNature::Market && !self.is_smoothed()
    }
}

/// One provider, one contract. The unit of "who can move this number".
///
/// Deliberately excludes the feed id: two feeds on one adapter share an
/// operator, key, and deployment, so a compromise moves both. Distinguishing
/// them here would let a config claim independence it does not have.
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

// ---------------------------------------------------------------------------
// Sources
// ---------------------------------------------------------------------------

/// A single provider read, quoting some unit. Always carries its own staleness
/// bound — there is no market-level default to inherit.
///
/// Implicit staleness is how a fast feed silently acquires a slow feed's
/// tolerance: v1 let a Reflector leg fall back to `max_price_stale_seconds`,
/// so a market holding one 12h-heartbeat feed had to widen that default and
/// thereby widened it for its 5-minute feed too. Every bound is written down.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeedSource {
    pub provider: ProviderRef,
    pub decimals: u32,
    pub max_stale_seconds: u64,
}

/// `factor x price(quote)`, where `factor` is denominated in `quote`'s unit.
///
/// The motivating shape: SolvBTC has no independent USD feed, but RedStone
/// publishes `SolvBTC/BTC` and Reflector publishes BTC. Multiplying yields a
/// USD price whose *only* shared dependency with a direct `SolvBTC/USD` feed is
/// the ratio publisher — which [`IndependencePolicy`] then forces the config to
/// declare.
///
/// `min_factor_wad` / `max_factor_wad` bound the ratio itself. The final sanity
/// band only catches gross moves in the product, and a wrapper ratio is a slow,
/// tightly-known quantity (SolvBTC/BTC sits just above 1.0 and only accrues
/// upward), so bounding it directly is far tighter than anything the output band
/// can express. Without this, a compromised ratio feed prices the asset
/// arbitrarily inside an output band sized for the *quote's* volatility.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScaledSource {
    pub factor: FeedSource,
    pub quote: PriceKey,
    /// Inclusive lower bound on the normalized factor, WAD.
    pub min_factor_wad: i128,
    /// Inclusive upper bound on the normalized factor, WAD.
    pub max_factor_wad: i128,
}

/// Pool shapes this engine knows how to value. An enum rather than a bool so a
/// stable-swap or weighted pool gets its own formula without touching the
/// engine — the constant-product fair-value identity does not generalize.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PoolKind {
    /// `x * y = k`.
    ConstantProduct,
}

/// Fair value of one LP share.
///
/// Priced from the pool invariant, never from the reserve split:
/// `total = 2 * sqrt(r_a * r_b * p_a * p_b)`, `per_share = total / supply`.
/// Reserves enter only through `k = r_a * r_b`, which a swap leaves invariant
/// (fees grow it, never shrink it), so a flash-loan **skew** cannot move the
/// price. The naive `(r_a*p_a + r_b*p_b)/supply` can be moved at will and is
/// not implemented anywhere in this crate.
///
/// # What the formula does not defend
///
/// It immunizes the reserve *ratio*, not the reserve *level* or `total_supply`:
///
/// * **Donation.** A direct transfer into a pool that derives reserves from
///   balances raises `k` with `total_supply` unchanged, so `per_share` jumps
///   with no LP minted. A permissionless `sync`/`skim` restores the same attack
///   even against stored reserves.
/// * **Dust supply.** `per_share = total / supply` is unbounded as `supply`
///   approaches zero - the first-depositor shape.
/// * **Fee-on-transfer or rebasing legs.** `k`-derived value diverges from
///   redeemable value, and redemption is always the smaller number.
///
/// None of these are handled here, which is why `validate_source_shape` refuses
/// this variant outright.
///
/// `key_a` / `key_b` and the decimals are expected to be read back from the pool
/// contract at configuration time rather than supplied by hand: deriving the
/// topology from the integration target is what keeps a config from naming the
/// LP share as its own leg.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LpShareSource {
    pub pool: Address,
    pub kind: PoolKind,
    pub key_a: PriceKey,
    pub key_b: PriceKey,
    /// Token decimals for reserve A, for WAD normalization.
    pub reserve_a_decimals: u32,
    /// Token decimals for reserve B, for WAD normalization.
    pub reserve_b_decimals: u32,
    /// Decimals of the LP share token itself.
    pub share_decimals: u32,
}

/// One independent opinion about an asset's USD price.
///
/// Flat by construction — see the module docs. Adding a shape means adding a
/// variant plus its evaluator and its `local_properties` arm; no validation rule
/// changes, because rules are predicates over [`SourceProperties`].
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PriceSource {
    /// A provider quoting this key directly in USD.
    Feed(FeedSource),
    Scaled(ScaledSource),
    LpShare(LpShareSource),
}

// ---------------------------------------------------------------------------
// Independence
// ---------------------------------------------------------------------------

/// How much trust the two sources are allowed to share.
///
/// v1 asked `(RedStone, RedStone) => reject`, a proxy that was simultaneously
/// too strict (it rejects two genuinely independent RedStone deployments) and
/// too blunt (it never says *what* is shared, so a reviewer cannot weigh it).
///
/// The replacement does not decide for governance; it forces disclosure. The
/// validator computes the actual shared set and requires the config to name it
/// exactly. A config that is effectively single-source can no longer look
/// independent by accident, and the declaration rides along in the config event
/// where an indexer can alarm on it.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IndependencePolicy {
    /// The two sources share no trust domain. Strictest; the default.
    RequireDisjoint,
    /// The sources share exactly these domains, and governance has accepted
    /// that compromising any one of them moves both opinions together.
    ///
    /// Set equality, not subset: a shared domain introduced by a later edit must
    /// be re-declared rather than silently absorbed.
    AllowShared(Vec<TrustDomain>),
}

// ---------------------------------------------------------------------------
// Asset config
// ---------------------------------------------------------------------------

/// Everything needed to price one [`PriceKey`].
///
/// No `primary`, no `anchor`, no `strategy`. "How many independent opinions do
/// we hold" is `sources.len()`; "how is each formed" is each source's own shape.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssetOracle {
    /// Decimals of the priced token, for amount-to-WAD conversion by consumers.
    /// Zero for a [`PriceKey::Ref`], which has no token and no amounts.
    pub asset_decimals: u32,
    /// Ceiling on how stale any composed answer for this key may be.
    ///
    /// Not a default anyone inherits — every feed states its own bound — but a
    /// cap those bounds may not exceed, and the limit the *composite* timestamp
    /// (the min across a source's components) is gated against. Without this
    /// second gate a slow leg that stops publishing rides under a live fast leg
    /// for its whole window; with it, the asset-level answer goes stale on
    /// schedule regardless of which component froze.
    pub max_price_stale_seconds: u64,
    /// One or two. Order indexes errors and events; it never reaches the price.
    pub sources: Vec<PriceSource>,
    /// Agreement band, consulted only when two sources are present.
    pub tolerance: OracleTolerance,
    pub independence: IndependencePolicy,
    /// Inclusive lower sanity bound on the final USD WAD price.
    pub min_sanity_price_wad: i128,
    /// Inclusive upper sanity bound on the final USD WAD price.
    pub max_sanity_price_wad: i128,
}

impl AssetOracle {
    /// True when a second opinion exists, so the agreement band applies.
    pub fn is_dual(&self) -> bool {
        self.sources.len() == 2
    }
}

// ---------------------------------------------------------------------------
// Properties
// ---------------------------------------------------------------------------

/// What a source *is*, independent of which variant expresses it.
///
/// Every validation rule is a predicate over this. Adding a provider or a
/// composition shape means implementing its contribution; no rule is edited.
/// That is the whole point of the type — v1's rules read enum variants and so
/// had to be revisited for every new shape.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceProperties {
    /// Some leg that trading can move is read without a window, transitively.
    ///
    /// Deliberately phrased as a defect rather than a virtue. v1's `smoothed`
    /// was combined with OR across components, so a composite counted as
    /// smoothed when *any* part was — which let the manipulable leg go bare. A
    /// defect flag combines with OR correctly: one bad leg taints the whole
    /// source, which is the semantics the rule wants.
    pub has_unsmoothed_market_leg: bool,
    /// Every distinct trust domain this source depends on, transitively.
    pub trust: Vec<TrustDomain>,
    /// The loosest staleness bound among this source's feeds, seconds.
    ///
    /// Used by the config rule that no component may outlive the asset-level
    /// ceiling. That ceiling is what the *composite* timestamp is gated against
    /// at read time; without it, per-leg gating alone lets a frozen slow leg
    /// ride under a live fast one for the slow leg's entire window, which is
    /// exactly when a ratio feed going silent matters most.
    ///
    /// Loosest rather than tightest: gating a composite at its tightest
    /// component would reject every legitimate mixed-cadence source — a 12h
    /// heartbeat ratio combined with a 5-minute quote would need the ratio fresh
    /// to the minute.
    pub loosest_max_stale_seconds: u64,
    /// Composition depth; 0 for a bare [`PriceSource::Feed`].
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

    /// Combines two contributions to one source: a defect anywhere taints the
    /// whole, trust accumulates, freshness takes the looser bound (see
    /// [`SourceProperties::loosest_max_stale_seconds`] for why), depth takes the
    /// deeper branch.
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

    /// Raises depth by one composition level.
    pub fn nest(mut self) -> Self {
        self.depth += 1;
        self
    }

    /// Contract addresses reachable from both sources, deduplicated.
    ///
    /// The unit the independence rule judges on. `TrustDomain`'s `kind` is
    /// declared by the proposer for a multi-feed adapter and cannot be checked
    /// on-chain, so two feeds on one contract could otherwise be labelled as
    /// different providers and read as independent.
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

/// The part of a source's properties knowable without touching the registry,
/// plus the keys whose properties must be folded in to complete it.
///
/// Split this way so `common` stays storage-free: the caller owns recursion,
/// depth accounting, and cycle detection, and this crate owns the shape rules.
pub struct LocalProperties {
    pub local: SourceProperties,
    pub dependencies: Vec<PriceKey>,
}

/// Properties contributed by the source itself, and the keys it depends on.
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
            // A pool contributes no provider and no price opinion — those come
            // from the legs — but its reserves and total supply ARE market
            // state, read at the current ledger with no window available at any
            // price. So an LP share always carries the unsmoothed-market defect
            // no matter how well smoothed its legs are.
            //
            // Do not "fix" this by looking at the legs: TWAP leg prices say
            // nothing about the instantaneous reserve read, which is the
            // dominant manipulation surface for a share price.
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
