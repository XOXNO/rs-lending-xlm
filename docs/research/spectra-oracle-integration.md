# Consuming Spectra's oracle in `contracts/price-aggregator`

Research only. No code was changed. Every claim below is tied to a file:line or a permalink.

**Sources read**

- `perspectivefi/spectra-stellar-bridge-public` @ `db966445f93eeec855068ec9346548a2cebc8df6` (2026-08-03) — full shallow clone, 102 tracked paths.
- `perspectivefi/spectra-core` @ `2fa3cbf393acb7b25e20a2aef47aaf7e5681fdb2` (last push 2025-04-25) — read only the `src/spectra-oracles/**` subtree, for context on what "Spectra's oracle" is on EVM.
- Local repo `rs-lending-xlm`, branch `feat/caps-zero-means-zero`.

---

## 0. Bottom line

**The Spectra Stellar bridge repository contains no oracle and no price surface of any kind.** There is nothing on
Stellar to adapt. A `providers/spectra.rs` cannot be written today, because there is no Spectra contract on Stellar
that returns a number a price aggregator could consume.

What Spectra bridges to Stellar is a plain SEP-41 fungible token (`WrappedPT`) carrying three pieces of provenance
metadata — `maturity`, `origin_chain_id`, `origin_address` — and nothing else. It exposes no rate, no redemption
value, no reference to the underlying asset, and no timestamp.

Spectra *does* have PT price oracles, but they live in `spectra-core` on EVM, are `AggregatorV3Interface` shims over
a Curve pool's internal oracle, and — critically — **return `updatedAt = 0`**, i.e. they carry no freshness signal at
all. Even if that design were ported to Soroban verbatim, it would be rejected by our engine on every read (see §5.1).

Consequently:

- **Not consumable as a `PriceSource` today.** Observed, not inferred: the interface does not exist.
- **The composition our engine already supports is the right shape** if Spectra (or a third party) ever publishes a
  PT-to-underlying rate: `PriceSource::Scaled { factor = <PT/underlying rate feed>, quote = <underlying USD price> }`.
  That path needs **zero new provider code** if the rate is published through RedStone or the XOXNO adapter — it is a
  configuration change only.
- A dedicated `ProviderRef::Spectra` variant is only worth building if Spectra deploys a *Soroban* rate contract with
  its own read interface. §4 sketches exactly that, so the work is pre-scoped, but it is contingent.

---

## 1. What Spectra actually exposes

### 1.1 Contract inventory (Stellar side)

From the repository tree at `db96644`:

| Path | Contract | Role |
|---|---|---|
| `contracts/stellar/contracts/bridge/` | `PtBridge` | Core bridge: lock/mint/burn/unlock, roles, fees, rate limits, pause |
| `contracts/stellar/contracts/bridge-axelar/` | Axelar messenger adapter | GMP transport |
| `contracts/stellar/contracts/bridge-traits/` | traits only | `BridgeMessenger`, `BridgeReceiver`, `PrincipalToken`, `WrappedPt`, `PtBridge` |
| `contracts/stellar/contracts/wrapped-pt/` | `WrappedPT` | The bridged token itself (OZ fungible + SEP-41) |
| `contracts/stellar/contracts/fake-pt/` | `FakePT` | Test fixture for a "native Stellar PT" |

There is no fifth contract. A case-insensitive grep for `oracle|price|exchange.?rate|ibt|underlying|maturity|redeem|share`
across `contracts/stellar/` and `contracts/evm/src/` returns **zero hits for `oracle` and zero for `price`**; the only
value-adjacent hits are `maturity` (a `u64` timestamp) and `underlying` (an EVM `address`, only ever carried inside the
bridge message payload).

### 1.2 The complete read interface of the bridged token

`contracts/stellar/contracts/wrapped-pt/src/lib.rs:94-119` — this is the entire non-SEP-41 surface, quoted verbatim:

```rust
    // ── Custom getters ─────────────────────────────────────────────────

    pub fn origin_chain_id(env: Env) -> u32 {
        env.storage().instance().get(&ORIGIN_CHAIN).unwrap()
    }

    pub fn origin_address(env: Env) -> BytesN<32> {
        env.storage().instance().get(&ORIGIN_ADDR).unwrap()
    }

    pub fn maturity(env: Env) -> u64 {
        env.storage().instance().get(&MATURITY).unwrap()
    }

    pub fn bridge(env: Env) -> Address {
        env.storage().instance().get(&BRIDGE).unwrap()
    }

    pub fn admin(env: Env) -> Address {
        env.storage().instance().get(&ADMIN).unwrap()
    }

    pub fn total_supply(env: Env) -> i128 {
        Base::total_supply(&env)
    }
```

Plus the standard `token::TokenInterface` impl at `wrapped-pt/src/lib.rs:122-163` (`allowance`, `approve`, `balance`,
`transfer`, `transfer_from`, `burn`, `burn_from`, `decimals`, `name`, `symbol`).

The declared trait a bridgeable PT must satisfy (`contracts/stellar/contracts/bridge-traits/src/lib.rs:79-89`):

```rust
#[contractclient(name = "PrincipalTokenClient")]
pub trait PrincipalToken {
    /// Returns the token decimals
    fn decimals(env: Env) -> u32;

    /// Returns the token symbol
    fn symbol(env: Env) -> String;

    /// Returns the PT maturity timestamp
    fn maturity(env: Env) -> u64;
}
```

And the bridge-issued wrapper trait (`bridge-traits/src/lib.rs:93-106`):

```rust
#[contractclient(name = "WrappedPtClient")]
pub trait WrappedPt {
    /// Returns the origin EVM PT address (32 bytes, zero-padded)
    fn origin_address(env: Env) -> BytesN<32>;

    /// Returns the origin EVM chain ID
    fn origin_chain_id(env: Env) -> u32;

    /// Mints wrapped PT tokens to a recipient (bridge only)
    fn mint(env: Env, to: Address, amount: i128);

    /// Burns wrapped PT tokens from an address (bridge only)
    fn burn_by_bridge(env: Env, from: Address, amount: i128);
}
```

**Note what is absent:** no `underlying()`, no `get_ibt()`, no `convert_to_underlying()`, no rate, no price, no
`latest_price`, no timestamp accessor. The `underlying` EVM address travels in the bridge message
(`contracts/stellar/contracts/bridge/src/lib.rs:113-122`) but is **discarded** — it is decoded at
`bridge/src/lib.rs:1183` into `PtBridgeInfo.underlying`, and is never written to the wrapper's storage nor passed to
its constructor (`wrapped-pt/src/lib.rs:30-50` takes `admin, bridge, decimals, name, symbol, origin_chain_id,
origin_address, maturity` — no underlying).

`PtBridgeInfo` (`bridge/src/lib.rs:111-122`), for completeness:

```rust
/// PT info from bridging message
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct PtBridgeInfo {
    pub pt_address: BytesN<20>,
    pub amount: i128,
    pub source_chain_id: u32,
    pub stellar_recipient: Address,
    pub maturity: u64,
    pub underlying: BytesN<20>,
    pub decimals: u32,
    pub pt_symbol: String,
}
```

### 1.3 Price semantics — there are none on Stellar

- **Quote currency:** N/A. Nothing is quoted.
- **Decimals/scale:** the token's own `decimals`, taken from the inbound ABI payload byte at
  `bridge/src/lib.rs:1186` (`let decimals = slice.get(255).ok_or(BridgeError::InvalidPayload)? as u32;`) and passed
  straight into the `WrappedPT` constructor at `bridge/src/lib.rs:1407`. **There is no range validation on this
  value on the Stellar side** — it is trusted because the payload came from a trusted remote.
- **Spot vs accruing rate:** N/A.
- **Freshness:** N/A. No timestamp exists other than `maturity`, which is a fixed constant, not an observation time.
- **Behaviour when unavailable:** N/A.

### 1.4 What "bridged by Spectra" means mechanically

Observed from `bridge/src/lib.rs` and `README.md`:

1. On EVM, a Spectra Principal Token (PT) is locked in `PTBridge.sol`.
2. An Axelar (or LayerZero) GMP message carries `{pt_address, amount, source_chain_id, stellar_recipient, maturity,
   underlying, decimals, pt_symbol}` to Stellar.
3. `PtBridge::receive_message` deploys (first time) or looks up a `WrappedPT` keyed by
   `(origin_chain_id, origin_evm_address)` — `create_origin_key`, `bridge/src/lib.rs:1062-1068` — and mints
   `amount` wrapped units to the recipient.

So the bridged asset is a **1:1 wrapper of an EVM Spectra PT**, and a Spectra PT is itself a **zero-coupon claim on an
underlying asset that matures at `maturity`**. Its economic value is a *discount* on the underlying — it approaches
1 underlying at maturity and trades below that before. Therefore:

> **The oracle problem is a composition problem, not a lookup problem.** The correct price is
> `price(wPT) = rate(PT → underlying) × price(underlying)`, and Spectra publishes neither term on Stellar.

Worse, the second term is also awkward: the underlying is an **EVM ERC-20 address** (`BytesN<20>`), so
`PriceKey::Token(underlying)` is generally impossible on Stellar — the underlying often has no Soroban token contract.
The quote leg must therefore be a `PriceKey::Ref(Symbol)` (see §3.3 and §4.3).

### 1.5 Trust model

| Concern | Finding | Evidence |
|---|---|---|
| Who can mint the wrapper | Only the bridge address; `bridge.require_auth()` | `wrapped-pt/src/lib.rs:53-57` |
| Who can burn | Bridge (`burn_by_bridge`) and holder/spender (SEP-41 `burn`/`burn_from`) | `wrapped-pt/src/lib.rs:60-65,144-150` |
| Who can repoint the bridge | Wrapper `admin`, unilaterally, no timelock | `wrapped-pt/src/lib.rs:68-72` |
| Who can replace the wrapper's code | Wrapper `admin`, unilaterally, no timelock | `wrapped-pt/src/lib.rs:82-86` |
| Pause | `PtBridge::pause`/`unpause`, GUARDIAN role only, instance flag | `bridge/src/lib.rs:525-537` |
| Rate limiting | Per-PT volume cap over a window; `0` means blocked | `bridge/src/lib.rs:488-504,977-993` |
| Roles | ADMIN, OPERATOR, GUARDIAN, UPGRADER — all initially the admin | `bridge/src/lib.rs:206-213` |
| Mainnet deployment | **Not deployed.** `deployments/stellar-mainnet.json` has `bridge: null`, `registry: null`, Axelar addresses are `PLACEHOLDER_*`, and a comment reads `"status": "Axelar Stellar mainnet integration expected 2026"` | `deployments/stellar-mainnet.json` |

Two consequences that matter to us regardless of oracle design:

- **The wrapper admin can swap the wrapper's WASM.** Any adapter that reads state from a `WrappedPT` address is
  trusting a mutable, non-timelocked contract. This is a stronger trust assumption than we make of Reflector/RedStone.
- **Pausing the bridge does not freeze the wrapper.** `is_paused` gates bridging, not `transfer`. A paused bridge means
  the wrapper cannot be redeemed for the EVM PT, which is precisely when its fair value should *diverge* from the
  composed price — and nothing on-chain signals that to us.

### 1.6 For context: Spectra's oracle on EVM (`spectra-core`, not the bridge)

Spectra's actual PT oracles are `AggregatorV3Interface` adapters over a Curve pool. The base
(`spectra-core/src/spectra-oracles/oracles/BaseOracle.sol`):

```solidity
    /** @dev See {AggregatorV3Interface-latestRoundData}. */
    function latestRoundData()
        external
        view
        returns (
            uint80 roundId,
            int256 answer,
            uint256 startedAt,
            uint256 updatedAt,
            uint80 answeredInRound
        )
    {
        return (0, int256(_getQuoteAmount()), 0, 0, 0);
    }
```

The PT specialisation (`src/spectra-oracles/oracles/BaseOracleCurvePT.sol`):

```solidity
abstract contract BaseOracleCurvePT is BaseOracle {
    function _getQuoteAmount() internal view override returns (uint256) {
        return _PTPrice();
    }
    /**
     * @dev Depending on the pool you should use:
     * getPTToAssetRate() should be used,
     * or getPTToIBTRate() if the asset is not easily tradable with IBT
     */
    function _PTPrice() internal view virtual returns (uint256);
}
```

and a concrete feed (`src/spectra-oracles/chainlinkFeeds/cryptoswap-ng/BaseFeedCurvePTAsset.sol`):

```solidity
abstract contract BaseFeedCurvePTAsset is BaseOracleCurvePT {
    function _PTPrice() internal view override returns (uint256) {
        return CurveOracleLib.getPTToAssetRate(pool);
    }
    function decimals() external view override returns (uint8) {
        return IERC20Metadata(asset).decimals();
    }
}
```

Four properties of this design, all directly relevant:

1. **It is a rate, not a USD price.** The answer is "how many `asset` units one PT is worth", scaled to `asset`'s
   decimals. It must be composed with a USD price for `asset`.
2. **`updatedAt` is hardcoded to `0`.** There is no freshness signal whatsoever. It is a pure view over pool state, so
   "current" means "current ledger", but a consumer cannot distinguish that from "never updated".
3. **It is market-derived, not fundamental.** `CurveOracleLib.getPTToAssetRate(pool)` reads a Curve pool — an AMM whose
   state an attacker can move within a block. (Curve's `price_oracle` is EMA-smoothed; **Unverified** whether
   `getPTToAssetRate` uses the EMA or the spot invariant — I did not read `CurveOracleLib.sol`.)
4. **`decimals` is the *asset's* decimals, not 8 or 18.** So the scale is per-market and must be read, not assumed.

None of this is deployed on Stellar and none of it is in the bridge repo. Treat §1.6 as a description of what a future
Soroban port would most plausibly look like — **Inferred**, not a commitment.

---

## 2. Our extension point

### 2.1 The provider shape every adapter implements

There is **no Rust trait**. The provider "interface" is a convention: two free functions per module, wired into three
`match` sites. This is the exact contract a new provider must satisfy.

**(a) A read function.** Signature, from the two simplest providers:

`contracts/price-aggregator/src/providers/redstone.rs:18-24`

```rust
pub(crate) fn read(
    session: &mut Session,
    feed: &MultiFeedRef,
    decimals: u32,
) -> Option<OracleObservation> {
    multi_feed::read_multi_feed_source(session, feed, decimals)
}
```

`contracts/price-aggregator/src/providers/xoxno.rs:24-30` is identical in shape.
`contracts/price-aggregator/src/providers/reflector.rs:63-73` takes `&ReflectorFeedRef` instead.

The return type is `Option<OracleObservation>`; `None` means "unreadable" and is **not** an error at this layer.

`contracts/price-aggregator/src/observation.rs:6-11`:

```rust
pub(crate) struct OracleObservation {
    pub price_wad: i128,
    pub timestamp: u64,
}
```

**(b) An attest function**, run once at configuration time to bind on-chain facts to the stored config:

`contracts/price-aggregator/src/providers/redstone.rs:10-16` (weakest form — decimals only):

```rust
pub(crate) fn attest(env: &Env, decimals: u32) {
    assert_with_error!(
        env,
        decimals == REDSTONE_DECIMALS,
        OracleError::InvalidOracleDecimals
    );
}
```

`contracts/price-aggregator/src/providers/xoxno.rs:11-22` (decimals + staleness envelope):

```rust
pub(crate) fn attest(env: &Env, feed: &MultiFeedRef, decimals: u32, max_stale: u64) {
    assert_with_error!(
        env,
        reflector_decimals(env, &feed.contract) == decimals,
        OracleError::InvalidOracleDecimals
    );
    assert_with_error!(
        env,
        max_stale >= max_submission_age(env, &feed.contract),
        OracleError::InvalidStalenessConfig
    );
}
```

`contracts/price-aggregator/src/providers/reflector.rs:15-40` additionally asserts the oracle's *base* is USD and that
the feed's resolution fits inside the configured staleness window.

**(c) Three wiring points.** A new `ProviderRef` variant must be added at all three or the code will not compile:

1. `contracts/price-aggregator/src/engine.rs:475-488` — the read dispatch:

```rust
fn read_feed(session: &mut Session, feed: &FeedSource) -> Option<(OracleObservation, bool)> {
    let observation = match &feed.provider {
        ProviderRef::Reflector(r) => reflector::read_reflector_source(session, r, feed.decimals),
        ProviderRef::RedStone(r) => redstone::read(session, r, feed.decimals),
        ProviderRef::Xoxno(x) => xoxno::read(session, x, feed.decimals),
    }?;

    let stale = is_stale(
        session.now_secs(),
        observation.timestamp,
        feed.max_stale_seconds,
    );
    Some((observation, stale))
}
```

2. `contracts/price-aggregator/src/admin.rs:27-37` — the attest dispatch.
3. `contracts/price-aggregator/src/session.rs:163-176` — `collect_provider`, the bulk-prefetch warm path (RedStone and
   XOXNO participate; Reflector opts out with `return`).

Plus the three `match` arms inside `ProviderRef` itself (`common/src/types/composable_oracle.rs:50-75`): `contract()`,
`is_smoothed()`, `nature()`.

### 2.2 Types

`common/src/types/composable_oracle.rs`:

```rust
// :44-48
pub enum ProviderRef {
    Reflector(ReflectorFeedRef),
    RedStone(MultiFeedRef),
    Xoxno(MultiFeedRef),
}

// :36-40
pub struct MultiFeedRef {
    pub contract: Address,
    pub feed_id: String,
    pub nature: FeedNature,
}

// :28-32
pub enum FeedNature {
    Market,
    Fundamental,
}

// :79-83
pub struct FeedSource {
    pub provider: ProviderRef,
    pub decimals: u32,
    pub max_stale_seconds: u64,
}

// :87-94  ← this is the composition primitive
pub struct ScaledSource {
    pub factor: FeedSource,
    pub quote: PriceKey,
    pub min_factor_wad: i128,
    pub max_factor_wad: i128,
}

// :118-124
pub enum PriceSource {
    Feed(FeedSource),
    Scaled(ScaledSource),
    AquariusLp(AquariusLpSource),
    AquariusStableLp(AquariusLpSource),
}

// :145-158
pub struct AssetOracle {
    pub asset_decimals: u32,
    pub max_price_stale_seconds: u64,
    pub sources: Vec<PriceSource>,
    pub tolerance: OracleTolerance,
    pub independence: IndependencePolicy,
    pub min_sanity_price_wad: i128,
    pub max_sanity_price_wad: i128,
}
```

`PriceKey` (`composable_oracle.rs:12-16`) is `Token(Address) | Ref(Symbol)`. `OracleTolerance`
(`common/src/types/oracle.rs:15-19`) is `{ upper_ratio_bps: u32, lower_ratio_bps: u32 }`.

Bounds: `MAX_SOURCES = 2`, `MAX_RESOLUTION_DEPTH = 3` (`composable_oracle.rs:5-8`).

### 2.3 How a price is resolved

`resolve` → `compute_hard` → `resolve_outcome` → `compose` → `blend` → `force`
(`contracts/price-aggregator/src/engine.rs:173-226, 291-351, 386-430`).

`compose` reads at most two sources and produces `Legs` (`engine.rs:28-33`):

```rust
enum Legs {
    One(Reading),
    Two { primary: Reading, anchor: Reading },
    Partial { reading: Reading, slot: LegSlot },
    Empty,
}
```

`blend` (`engine.rs:328-351`) for the two-leg case:

```rust
        Legs::Two { primary, anchor } => {
            let stale = primary.stale || anchor.stale;
            let ts = primary.timestamp.min(anchor.timestamp);
            let deviation =
                !within_tolerance_band(env, anchor.price_wad, primary.price_wad, &oracle.tolerance);

            let price_wad = midpoint_price_or_zero(anchor.price_wad, primary.price_wad);
```

So: **staleness is a logical OR, the timestamp is the older of the two, and the blend is an arithmetic midpoint**
(`contracts/price-aggregator/src/tolerance.rs:22-27`), not a min or a conservative pick. Note also that
`within_tolerance_band` (`tolerance.rs:6-20`) compares `max/min` against `tolerance.upper_ratio_bps` only —
`lower_ratio_bps` is not read by that function.

If exactly one of two configured legs is readable, `compose` yields `Legs::Partial`, and `Outcome::partial`
(`engine.rs:81-96`) sets `price_wad = 0` and `deviation = true`. A dual-source asset therefore **cannot** silently
degrade to single-source.

### 2.4 Staleness and sanity enforcement

Two staleness checks, at different levels:

- Per-feed, in `read_feed` (`engine.rs:482-486`) against `FeedSource.max_stale_seconds`.
- Per-asset, in `read_source` (`engine.rs:444-450`) against `AssetOracle.max_price_stale_seconds`, OR-ed with the
  component flag:

```rust
    let timestamp = observation.timestamp;
    let stale = component_stale
        || is_stale(
            session.now_secs(),
            timestamp,
            oracle.max_price_stale_seconds,
        );
```

`is_stale` (`common/src/oracle/observation.rs:27-29`) is `now > ts && (now - ts) > max_stale`. Global bounds:
`MIN_PRICE_STALE_SECONDS = 60`, `MAX_PRICE_STALE_SECONDS = 86_400`
(`common/src/oracle/observation.rs:9-10`). Config-time, `validation::staleness_envelope`
(`contracts/price-aggregator/src/validation.rs:30-40`) forces the asset window to be **at least as loose** as every
leg's window.

The single gate that decides usability, `Outcome::failure` (`engine.rs:98-120`):

```rust
        if self.stale {
            return Some(OracleError::PriceFeedStale);
        }
        if self.deviation {
            return Some(OracleError::UnsafePriceNotAllowed);
        }
        if self.price_wad <= 0 {
            return Some(OracleError::InvalidPrice);
        }
        if self.price_wad < oracle.min_sanity_price_wad
            || self.price_wad > oracle.max_sanity_price_wad
        {
            return Some(OracleError::SanityBoundViolated);
        }
```

`force` (`engine.rs:148-156`) turns any of those into `panic_with_error!`. So `PriceAggregator::prices()` **panics**;
`PriceAggregator::quotes()` returns `PriceStatus { valid: false, .. }` (`engine.rs:158-171`).

The controller uses the panicking path for anything that moves money — `fetch_prices` at
`contracts/controller/src/external/price_aggregator.rs:16-27` calls `.prices(...)`, and additionally panics with
`OracleNotConfigured` if a key is missing from the returned map. **The engine fails closed.**

### 2.5 Config-time guards a new provider inherits

- `feed_shape` (`validation.rs:112-127`): `decimals ∈ [1, 18]`, `max_stale_seconds ∈ [60, 86_400]`.
- `asset_decimals` (`validation.rs:129-137`): `PriceKey::Token` ⇒ decimals ∈ [3, 18]; `PriceKey::Ref` ⇒ **decimals
  must be 0**.
- `factor_bounds` (`validation.rs:139-146`): `min_factor_wad > 0`, `max ≥ min`, `max ≤ MAX_REASONABLE_PRICE_WAD`
  (= `1e9 * WAD`, `common/src/constants/shared.rs:15`).
- `smoothing` (`validation.rs:42-48`): panics with `SpotOnlyNotProductionSafe` if **both** legs are unsmoothed market
  legs. A source is an unsmoothed market leg iff `nature() == Market && !is_smoothed()`
  (`composable_oracle.rs:72-74`). For `RedStone`/`Xoxno`, `is_smoothed()` is hardcoded `false`
  (`composable_oracle.rs:58-63`), so `nature` alone decides.
- `validate_single_source_sanity_band` (`common/src/validation.rs:146-157`): a single-source asset's sanity band may
  span at most `MAX_SINGLE_SOURCE_SANITY_BAND_BPS = 1000` (10%) — `common/src/oracle/observation.rs:17`.
- `independence` (`validation.rs:50-68`): two legs must either share no contract addresses, or the operator must
  enumerate exactly the shared set in `IndependencePolicy::AllowShared`.
- `set_oracle` probes the config live before storing (`admin.rs:39-52`) and revalidates every dependent oracle
  (`admin.rs:54-65`).

---

## 3. Can Spectra be consumed as a `PriceSource`?

### 3.1 Directly: no

There is no callable read interface. This is not a limitation of our engine; there is nothing to call.

### 3.2 The composition, if a rate ever exists

The economically correct composition, and the one our engine already implements, is:

```
price_usd(wPT) = rate(PT → underlying)  ×  price_usd(underlying)
                 └── ScaledSource.factor ┘   └─ ScaledSource.quote ─┘
```

executed by `read_scaled` (`contracts/price-aggregator/src/engine.rs:490-518`):

```rust
    let Some((factor, factor_stale)) = read_feed(session, &scaled.factor) else {
        return Ok(None);
    };

    if factor.price_wad < scaled.min_factor_wad || factor.price_wad > scaled.max_factor_wad {
        return Err(OracleError::FactorOutOfBounds);
    }

    let quote = resolve_nested(session, &scaled.quote, depth + 1)?;

    let Some(price_wad) = Wad::from(factor.price_wad).try_mul(&env, Wad::from(quote.price_wad))
    else {
        return Err(OracleError::InvalidPrice);
    };

    Ok(Some((
        OracleObservation {
            price_wad: price_wad.raw(),
            timestamp: factor.timestamp.min(quote.timestamp),
        },
        factor_stale,
    )))
```

Note the three properties this gives us for free:

- `min_factor_wad`/`max_factor_wad` is the natural place to bound a PT discount. For a PT that can never exceed
  1 underlying, `max_factor_wad = 1.02e18` and `min_factor_wad = 0.50e18` (illustrative) makes an oracle that reports a
  PT worth 3× the underlying a hard `FactorOutOfBounds` failure rather than a mispriced position.
- `timestamp = min(factor, quote)` — the composed observation inherits the older of the two, so a stale underlying
  price cannot be laundered by a fresh rate.
- The quote's own staleness is separately enforced inside `resolve_nested` (`engine.rs:241-245`), which returns the
  nested `Outcome::failure` as an `Err`, propagating out of `read_scaled` via `?`.

### 3.3 The quote leg must usually be `PriceKey::Ref`

The underlying is an EVM address that typically has no Soroban token contract, so `PriceKey::Token(underlying)` is
unavailable. Use `PriceKey::Ref(symbol_short!("USDC"))` and register a normal `AssetOracle` for it with
`asset_decimals: 0` (mandated by `validation.rs:129-137`). This is what `PriceKey::Ref` exists for.

### 3.4 Verdict

| Route | Feasible today | New code required |
|---|---|---|
| `ProviderRef::Spectra` reading a Soroban Spectra oracle | **No** — no such contract | n/a |
| `PriceSource::Scaled` with the rate published via RedStone or the XOXNO adapter | **Yes, if the feed is published** | **None.** Configuration only. |
| `PriceSource::Scaled` with a new `ProviderRef::Spectra` factor leg | Only if Spectra ships a Soroban rate contract | ~120 LOC, §4 |
| Deriving the discount on-chain from `maturity` + a configured yield curve | Would need a new `PriceSource` variant | Substantial; not recommended (§5.6) |

**Strong recommendation: prefer the middle row.** If a PT/underlying rate can be delivered through a feed provider we
already trust, it inherits our attestation, staleness, warm-batching and independence machinery at zero marginal code
and zero marginal audit surface. Do not build a bespoke provider to read a number that a generic feed can carry.

---

## 4. Sketch: `contracts/price-aggregator/src/providers/spectra.rs`

**This is contingent on Spectra deploying a Soroban PT rate oracle.** It does not exist today. The sketch is written so
the work is pre-scoped if it ever does, and so the decimal handling is settled in advance.

### 4.1 Assumed counterparty interface

**Unverified — this contract does not exist.** Modelled on the EVM `BaseFeedCurvePTAsset` shape (§1.6), with the two
changes our engine requires (a timestamp, and an explicit failure mode). Would live in
`common/src/oracle/providers/spectra.rs` alongside the other client definitions:

```rust
use soroban_sdk::{contractclient, contracttype, Address, Env};

#[contracttype]
#[derive(Clone, Debug)]
pub struct SpectraRateData {
    /// PT units → underlying units, scaled by 10^decimals.
    pub rate: i128,
    /// Ledger time at which `rate` was last observed. MUST be a real
    /// observation time, never `env.ledger().timestamp()` at read time.
    pub timestamp: u64,
}

#[contractclient(name = "SpectraRateOracleClient")]
#[allow(dead_code)]
pub trait SpectraRateOracle {
    /// Rate + observation timestamp for one PT.
    fn pt_rate(env: Env, pt: Address) -> SpectraRateData;
    /// Scale of `SpectraRateData.rate`. Equals the *underlying's* decimals.
    fn decimals(env: Env) -> u32;
    /// Longest interval the publisher guarantees between updates.
    fn max_submission_age_seconds(env: Env) -> u64;
    /// Contract address of the underlying, for attestation.
    fn underlying(env: Env, pt: Address) -> Address;
}

pub fn read_rate_uncached(env: &Env, contract: &Address, pt: &Address) -> Option<SpectraRateData> {
    match SpectraRateOracleClient::new(env, contract).try_pt_rate(pt) {
        Ok(Ok(data)) => Some(data),
        _ => None,
    }
}
```

The `try_*` + `Ok(Ok(_))` pattern is mandatory — it matches `read_price_data_uncached`
(`common/src/oracle/providers/redstone.rs:20-29`) and converts a trapping counterparty into `None` rather than
unwinding our whole transaction.

### 4.2 Type additions

```rust
// common/src/types/composable_oracle.rs
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SpectraRateRef {
    pub contract: Address,
    pub pt: Address,
    pub nature: FeedNature,
}

pub enum ProviderRef {
    Reflector(ReflectorFeedRef),
    RedStone(MultiFeedRef),
    Xoxno(MultiFeedRef),
    Spectra(SpectraRateRef),      // NEW
}
```

and the three `ProviderRef` methods (`composable_oracle.rs:50-75`):

```rust
    pub fn contract(&self) -> &Address {
        match self {
            ProviderRef::Reflector(r) => &r.contract,
            ProviderRef::RedStone(r) | ProviderRef::Xoxno(r) => &r.contract,
            ProviderRef::Spectra(s) => &s.contract,
        }
    }

    pub fn is_smoothed(&self) -> bool {
        match self {
            ProviderRef::Reflector(r) => matches!(r.read_mode, OracleReadMode::Twap(_)),
            ProviderRef::RedStone(_) | ProviderRef::Xoxno(_) => false,
            ProviderRef::Spectra(_) => false,   // see §5.4
        }
    }

    pub fn nature(&self) -> FeedNature {
        match self {
            ProviderRef::Reflector(_) => FeedNature::Market,
            ProviderRef::RedStone(r) | ProviderRef::Xoxno(r) => r.nature,
            ProviderRef::Spectra(s) => s.nature,
        }
    }
```

`ProviderRef` is `#[contracttype]` and is reachable from stored `AssetOracle` values, so **adding a variant changes the
persisted enum discriminant space**. Appending at the end is the only safe position; existing entries keep their tags.
Verify against the actual SDK encoding before shipping — flagged as an open question (§6).

### 4.3 The adapter

```rust
// contracts/price-aggregator/src/providers/spectra.rs
use common::errors::OracleError;
use common::oracle::observation::is_future_at;
use common::oracle::providers::spectra::{read_rate_uncached, SpectraRateOracleClient};
use common::types::SpectraRateRef;
use soroban_sdk::{assert_with_error, Env};

use crate::observation::OracleObservation;
use crate::session::Session;

/// Config-time binding. Mirrors xoxno::attest (providers/xoxno.rs:11-22).
pub(crate) fn attest(env: &Env, feed: &SpectraRateRef, decimals: u32, max_stale: u64) {
    let client = SpectraRateOracleClient::new(env, &feed.contract);

    // The configured scale must equal the oracle's own scale, which is the
    // UNDERLYING's decimals — not the PT's, and not a constant.
    assert_with_error!(
        env,
        client.decimals() == decimals,
        OracleError::InvalidOracleDecimals
    );

    // Our staleness window must be at least as loose as the publisher's
    // guaranteed update interval, or every read is stale by construction.
    assert_with_error!(
        env,
        max_stale >= client.max_submission_age_seconds(),
        OracleError::InvalidStalenessConfig
    );

    // Bind the PT identity: the oracle must actually know this PT and agree
    // on which asset the rate is denominated in.
    assert_with_error!(
        env,
        read_rate_uncached(env, &feed.contract, &feed.pt).is_some(),
        OracleError::NoLastPrice
    );
}

pub(crate) fn read(
    session: &mut Session,
    feed: &SpectraRateRef,
    decimals: u32,
) -> Option<OracleObservation> {
    let env = session.env();
    let now_secs = session.now_secs();

    let data = read_rate_uncached(env, &feed.contract, &feed.pt)?;

    // Reject future-dated observations exactly as the other providers do
    // (observation.rs:21-23, 36-38). MAX_FUTURE_SKEW_SECONDS = 60.
    if is_future_at(now_secs, data.timestamp) {
        return None;
    }

    // Scale conversion. `data.rate` is PT->underlying at 10^decimals.
    // try_normalize_positive_price rejects rate <= 0 and decimals > 18,
    // then multiplies by 10^(18 - decimals) to land on WAD.
    Some(OracleObservation {
        price_wad: common::oracle::observation::try_normalize_positive_price(
            data.rate, decimals,
        )?,
        timestamp: data.timestamp,
    })
}
```

Then the three wiring edits of §2.1(c), and — because a Spectra oracle is not a RedStone-style multi-feed —
`session.rs:163-176` `collect_provider` gets `ProviderRef::Spectra(_) => return,` (opt out of bulk warming, like
Reflector).

### 4.4 Decimal and scale conversion, spelled out

Let `d` = the oracle's declared `decimals` (= the underlying's decimals, e.g. 6 for USDC, 7 for a Stellar-native
asset, 18 for an 18-dec ERC-20), and `r` = the raw `i128` rate.

1. **Meaning:** `1 PT = r / 10^d` units of underlying.
2. **To WAD** (`common/src/oracle/observation.rs:19-25`):

```rust
pub fn try_normalize_positive_price(price: i128, decimals: u32) -> Option<i128> {
    if price <= 0 || decimals > WAD_DECIMALS {
        return None;
    }
    let factor = 10i128.checked_pow(WAD_DECIMALS - decimals)?;
    price.checked_mul(factor)
}
```

   ⇒ `factor_wad = r × 10^(18 − d)`. `WAD_DECIMALS = 18`, `WAD = 1e18` (`common/src/constants/shared.rs:3,9`).
   Rate 0.95 with `d = 6` ⇒ `r = 950_000` ⇒ `factor_wad = 950_000 × 10^12 = 0.95e18`. ✓
3. **Bounds check** (`engine.rs:500-502`): `min_factor_wad ≤ factor_wad ≤ max_factor_wad`, else `FactorOutOfBounds`.
4. **Compose** (`engine.rs:506`): `price_wad = Wad(factor_wad) × Wad(quote_price_wad)`, i.e.
   `factor_wad × quote_wad / 1e18` — dimensionally `(underlying/PT) × (USD/underlying) = USD/PT`. ✓
5. **The wrapper's own `decimals` never enters the price.** `AssetOracle.asset_decimals` is carried alongside the price
   in `PriceFeedRaw` (`engine.rs:139-145`) and applied by the consumer at
   `common/src/types/oracle.rs:79-81`:

```rust
    pub fn usd_value_wad(self, env: &Env, token_amount: i128) -> crate::math::fp::Wad {
        crate::math::fp::Wad::from_token(token_amount, self.asset_decimals).mul(env, self.price)
    }
```

   `AssetOracle.asset_decimals` **must** equal `WrappedPT::decimals()`, and must lie in `[3, 18]`
   (`validation.rs:14-15,129-137`). Since `bridge/src/lib.rs:1186` accepts any `u8` from the payload, a PT with
   decimals outside `[3, 18]` simply cannot be listed. That is a correct fail-closed outcome, not a bug to work around.

### 4.5 Example configuration

```rust
AssetOracle {
    asset_decimals: 18,                       // == WrappedPT::decimals()
    max_price_stale_seconds: 3_600,
    sources: vec![&env, PriceSource::Scaled(ScaledSource {
        factor: FeedSource {
            provider: ProviderRef::Spectra(SpectraRateRef {
                contract: spectra_rate_oracle,
                pt: wrapped_pt_address,
                nature: FeedNature::Market,   // pool-derived ⇒ Market. See §5.4.
            }),
            decimals: 6,                      // == underlying's decimals
            max_stale_seconds: 1_800,
        },
        quote: PriceKey::Ref(symbol_short!("USDC")),
        min_factor_wad:   500_000_000_000_000_000,   // 0.50
        max_factor_wad: 1_020_000_000_000_000_000,   // 1.02
    })],
    tolerance: OracleTolerance { upper_ratio_bps: 200, lower_ratio_bps: 200 },
    independence: IndependencePolicy::RequireDisjoint,
    min_sanity_price_wad: /* ... */,
    max_sanity_price_wad: /* ... */,
}
```

Two config-time traps in that snippet:

- With `nature: Market` and `is_smoothed() == false`, this is a lone unsmoothed market leg, so
  `validation::smoothing` (`validation.rs:42-48`) **panics with `SpotOnlyNotProductionSafe`** unless a second,
  smoothed source is configured. That guard is doing exactly its job here; see §5.4.
- Single-source assets are additionally capped to a 10% sanity band
  (`common/src/validation.rs:146-157`). A PT whose fair value legitimately drifts from 0.80 to 1.00 over its life
  cannot satisfy a 10% band, which is another reason a PT should not be a single-source listing.

---

## 5. Risks

### 5.1 Staleness — a `updatedAt = 0` port would be dead on arrival, and that is the good outcome

If a Soroban port copies the EVM `latestRoundData` shape (`return (0, answer, 0, 0, 0)`), the observation timestamp is
`0`. `is_stale(now, 0, max_stale)` is `true` for any `now > max_stale`, so `Outcome::failure` returns
`PriceFeedStale` and `force` panics. **Fail-closed. Correct behaviour, permanently unusable feed.**

The dangerous mitigation is the obvious one: having the adapter substitute `env.ledger().timestamp()` for a missing
timestamp. That converts a pool-state view into a permanently "fresh" price and **defeats every staleness control in
the engine simultaneously** — `read_feed`'s per-leg check, `read_source`'s per-asset check, and the
`min(factor, quote)` propagation in `read_scaled`. The `read` sketch in §4.3 deliberately propagates
`data.timestamp` unmodified. **This must not be relaxed.**

### 5.2 Manipulation surface

A Curve-pool-derived PT rate is a market price on a venue whose state an attacker can move. Three compounding factors:

- **Thin PT liquidity.** PT pools are small relative to the spot markets we normally price against; the cost of moving
  the rate is correspondingly small.
- **Cross-chain lag.** The rate would be observed on EVM and relayed to Stellar. During the relay window, the on-chain
  rate is by definition an EVM-history value. Any liquidation or borrow priced off it is priced off the past.
- **The wrapper is not redeemable atomically.** Unlike an LP share, an arbitrageur on Stellar cannot burn a mispriced
  wPT and instantly realise the underlying — bridging is asynchronous and can be paused
  (`bridge/src/lib.rs:525-537`) or rate-limited (`bridge/src/lib.rs:488-504`). The usual "arbitrage corrects the
  mispricing" argument does not hold, so the price can stay wrong for as long as the bridge is impaired.

Mitigation that already exists: `min_factor_wad`/`max_factor_wad` (`engine.rs:500-502`) caps the *magnitude* of a
manipulated rate. It does not cap the *direction* — an attacker who wants the PT cheap (to liquidate) is bounded only
by `min_factor_wad`, so that bound is the risk parameter that matters, and it should be set from the PT's worst
plausible discount, not from a comfortable round number.

### 5.3 Decimals mismatch — three independent places to get it wrong

1. **Oracle scale vs configured `FeedSource.decimals`.** Guarded by `attest` (§4.3) and re-checked nowhere at read
   time. If the counterparty can change its `decimals()` after attestation, every subsequent read is silently
   mis-scaled by a power of ten. `xoxno::attest` has the same exposure today; a Spectra oracle would be worse because
   its scale is per-market (the underlying's decimals), not a protocol constant like `REDSTONE_DECIMALS = 8`.
   *Consider re-reading `decimals()` inside `read` and returning `None` on mismatch — a real hardening delta versus the
   existing providers, and cheap.*
2. **Rate scale vs the underlying's decimals.** `getPTToAssetRate` is denominated in the asset, and
   `BaseFeedCurvePTAsset.decimals()` returns `IERC20Metadata(asset).decimals()`. Someone who assumes 18 (because PTs
   are 18-dec) and configures `decimals: 18` for a USDC-denominated rate will be off by `10^12`. That error passes
   `feed_shape` (1..=18 is satisfied) and is caught only by `factor_bounds` — which is exactly why
   `min_factor_wad`/`max_factor_wad` must be set tightly.
3. **`AssetOracle.asset_decimals` vs `WrappedPT::decimals()`.** Nothing in the aggregator reads the token's `decimals()`
   for a `Feed`/`Scaled` source — only the Aquarius path does that (`providers/aquarius.rs:250-267`). A mismatch here
   mis-scales every position value by a power of ten and is caught by no automated check. *An `attest`-time
   `TokenClient::new(env, wrapped_pt).try_decimals() == asset_decimals` assertion would close this and is worth doing
   for all providers, not just Spectra.*

### 5.4 `FeedNature` is a load-bearing, operator-supplied claim

`nature` on `MultiFeedRef` (and on the proposed `SpectraRateRef`) is **config data, not an attested fact**. Declaring a
Curve-pool-derived PT rate as `Fundamental` bypasses `validation::smoothing` (`validation.rs:42-48`) and permits a
single-source, unsmoothed, market-derived listing — the exact configuration `SpotOnlyNotProductionSafe` exists to
prevent. This is a governance/runbook risk, not a code defect: the guard works, but it can be talked out of by
mislabelling. A PT rate read off an AMM is `Market`. Write that down in the listing runbook.

### 5.5 Availability and zero

| Counterparty behaviour | Path | Outcome |
|---|---|---|
| Contract traps / not deployed | `try_pt_rate` ⇒ `Err` ⇒ `read_rate_uncached` ⇒ `None` | `read_scaled` returns `Ok(None)` (`engine.rs:496-498`) ⇒ `Legs::Empty` ⇒ `Outcome::unreadable()` = `NoLastPrice` ⇒ **panic** |
| Returns `rate == 0` or negative | `try_normalize_positive_price` returns `None` (`observation.rs:20`) | same as above ⇒ **panic** |
| Returns a stale but non-zero rate | `is_stale` ⇒ `PriceFeedStale` | **panic** |
| Returns a future-dated timestamp | `is_future_at` ⇒ `None` (§4.3) | **panic** |
| Rate outside the configured band | `FactorOutOfBounds` (`engine.rs:500-502`) | **panic** |
| One of two legs unreadable | `Legs::Partial` ⇒ `deviation = true` (`engine.rs:81-96`) | `UnsafePriceNotAllowed` ⇒ **panic** |
| Underlying's own price stale | `resolve_nested` returns `Err` (`engine.rs:241-245`) | propagated via `?` ⇒ **panic** |

**The engine fails closed in every case**, and the controller uses the panicking `prices()` path
(`contracts/controller/src/external/price_aggregator.rs:16-27`). There is no fail-open branch to worry about.

The corresponding *liveness* risk is real and should be stated plainly: an unavailable Spectra oracle **freezes every
operation touching an account that holds the wPT**, including liquidation of that account. A PT listed against a
single Spectra source converts an oracle outage into an inability to liquidate, which is how solvent-but-unliquidatable
bad debt is created. This is an argument for dual-sourcing a PT, or for not listing it as collateral at all.

### 5.6 Deriving the discount from `maturity` on-chain — do not

`maturity` is the only value-bearing datum Spectra actually exposes on Stellar. It is tempting to model
`rate = 1 / (1 + y)^((T − t)/year)` for a governance-set `y`. Two objections:

- The PT's market value can and does deviate from any fixed-`y` curve; a model price that ignores the market is not an
  oracle, it is a governance-controlled valuation of collateral. It would sail through every check in §2.4 because it
  is arithmetically well-behaved and perfectly "fresh".
- It requires a new `PriceSource` variant, a fixed-point exponential, and new `properties`/`validation`/`session`
  handling — substantially more surface than the `Scaled` route, for a worse price.

Mentioned only to record that it was considered and rejected.

### 5.7 Wrapper mutability and bridge liveness (independent of oracle design)

- `WrappedPT::upgrade` and `set_bridge` are admin-only with **no timelock** (`wrapped-pt/src/lib.rs:68-86`). If we ever
  read state from the wrapper (e.g. the `attest`-time decimals check in §5.3), we are trusting mutable code.
- `pause` freezes bridging but not transfers (`bridge/src/lib.rs:525-537`). A paused bridge is exactly when the
  wrapper's fair value decouples from the composed price, and nothing surfaces that on-chain.
- `set_rate_limit_for_pt` with `0` blocks a PT entirely (`bridge/src/lib.rs:488-504`; documented at
  `bridge/src/lib.rs:970` as "A return value of 0 means this PT is blocked (rate limit always exceeded)"). Same
  decoupling.

**Any listing of a Spectra-bridged asset should carry an off-chain monitor on `PtBridge::is_paused()` and
`get_rate_limit_for_pt()`, with a governance runbook to freeze the market when either trips.** The price aggregator
cannot see these and should not be extended to — they are a market-status concern, not a price concern.

---

## 6. Open questions

1. **Does Spectra intend to publish any price surface on Stellar at all?** Nothing in the public bridge repo suggests
   it. The bridge's own scope is transport. Ask Spectra directly.
2. **Is the Stellar bridge even live?** `deployments/stellar-mainnet.json` is entirely `null`/`PLACEHOLDER_*` with the
   comment `"status": "Axelar Stellar mainnet integration expected 2026"`. `deployments/stellar-testnet.json` was not
   read. If mainnet is unlaunched, this is pre-planning, not integration work.
3. **Does `CurveOracleLib.getPTToAssetRate(pool)` read Curve's EMA `price_oracle` or the instantaneous invariant?**
   I did not read `spectra-core/src/libraries/CurveOracleLib.sol`. This determines whether `is_smoothed()` should be
   `false` (§4.2) and whether §5.2 is a one-block or multi-block attack.
4. **What decimals do live Spectra PTs actually carry?** `PtBridgeInfo.decimals` comes from the EVM `ptToken.decimals()`
   with no Stellar-side range check (`bridge/src/lib.rs:1186`). Our `[3, 18]` bound (`validation.rs:14-15`) would
   silently exclude anything outside it at listing time. Sample the real deployments before promising a listing.
5. **Is appending a `ProviderRef` variant wire-compatible with stored `AssetOracle` values?** `ProviderRef` is
   `#[contracttype]` and reachable from persistent storage (`registry.rs:35-47`). Appending should preserve existing
   discriminants, but this must be verified against the pinned `soroban-sdk` encoding, with a round-trip test over a
   pre-upgrade stored value, before any such change ships.
6. **Which underlyings would we need `PriceKey::Ref` entries for, and do we already price them?** The composition in
   §3.2 is only as good as the quote leg. If the underlying is an exotic yield-bearing ERC-20 with no Stellar price
   source, the whole route is moot regardless of what Spectra publishes.
7. **Redemption path on Stellar.** Nothing in the bridge lets a wPT holder redeem at maturity on Stellar; they must
   bridge back to EVM and redeem there. **Inferred** from the absence of any redeem entry point in
   `bridge/src/lib.rs` and `wrapped-pt/src/lib.rs`. If correct, a matured wPT is worth 1 underlying *minus bridging
   friction and bridge-liveness risk*, which is not what a `getPTToAssetRate`-derived oracle would report, and is a
   permanent basis our `max_factor_wad` should account for.
