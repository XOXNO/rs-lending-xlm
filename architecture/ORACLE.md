# Oracle

How the controller prices assets. The protocol relies on Reflector
(SEP-40 oracle network) for spot and TWAP data and layers a two-tier
deviation policy on top to gate risk-increasing operations during market
stress.

## Goals

- **No single point of failure on spot prices.** A single Reflector feed
  suffices in dev; production markets run a spot-vs-TWAP cross-check that
  rejects risk-increasing ops once the divergence exceeds a configured band.
- **Asymmetric risk tolerance.** Supply and repay must always succeed under
  reasonable oracle drift. Borrow and withdraw must fail rather than consume
  bad prices.
- **USD-WAD everywhere.** Reflector reports in 14-decimal USD. The controller
  normalises into WAD (18 decimals) at the edge and then works purely in WAD.

## Reflector integration surface

The interface lives at `controller/src/oracle/reflector.rs` and wraps the
SEP-40 `PriceOracle` contract. The controller calls four methods:

| Method | Purpose |
|---|---|
| `decimals() -> u32` | Feed precision. Reflector returns 14 for USD feeds. |
| `resolution() -> u32` | Sample period in seconds (300 in mainnet Reflector). |
| `lastprice(asset) -> Option<PriceData>` | Spot price with timestamp. |
| `prices(asset, records: u32) -> Vec<Option<PriceData>>` | Historical window used for TWAP. |

The `ReflectorAsset` enum names assets — `Stellar(Address)` for native
assets, `Other(Symbol)` for bridged tickers such as `BTC` or `ETH`.

No provider abstraction trait exists today. The call sites at
`controller/src/oracle/mod.rs:191`, `:214`, `:274`, `:320` construct a
`ReflectorClient::new(env, &oracle_address)` directly. Swapping in a
non-Reflector oracle means refactoring the `find_price_feed` dispatcher
and the four `{cex,dex}_{spot,twap}_price` helpers.

## Oracle configuration

Each market's `MarketConfig` embeds an `OracleProviderConfig`
(`common/src/types.rs:225-232`). The `ORACLE` role configures a market via
`configure_market_oracle` and `edit_oracle_tolerance`
(`controller/src/config.rs:356-451`).

### Fields

| Field | Meaning |
|---|---|
| `base_asset: Address` | Token being priced. |
| `oracle_type: OracleType` | `Normal` enables price resolution; `None` disables the market for oracle-dependent ops. |
| `exchange_source: ExchangeSource` | `SpotOnly` (dev/test), `SpotVsTwap` (default), or `DualOracle` (CEX TWAP as safe anchor vs DEX spot). |
| `asset_decimals: u32` | Discovered from the token contract at config time (`config.rs:321`). |
| `tolerance: OraclePriceFluctuation` | Two-tier deviation bands (see below). |
| `max_price_stale_seconds: u64` | Rejects feeds older than this. Clamped to `[60, 86_400]` in `config.rs:381`. |
| `reflector_*` fields | Reflector-specific metadata: feed addresses, asset kind, symbol, decimals, TWAP sample count. Held in `ReflectorConfig` (`types.rs:199-208`). |

### Tolerance bands

`OraclePriceFluctuation` (`types.rs:216-221`) stores four ratios in
basis points:

```
first_upper_ratio_bps   e.g. 10_200  (+2%)
first_lower_ratio_bps   e.g.  9_800  (−2%)
last_upper_ratio_bps    e.g. 11_000  (+10%)
last_lower_ratio_bps    e.g.  9_000  (−10%)
```

The operator sets one *tolerance magnitude* per tier
(`first_tolerance_bps`, `last_tolerance_bps`); `config.rs:265-294` derives
the asymmetric ratios:

```
upper = BPS + tol
lower = BPS² / (BPS + tol)
```

The asymmetry is deliberate. Expressed as ratios, a 2% upper and a matching
lower deviation must be reciprocals so the band stays symmetric in log-space
(price up 2% vs. down ~1.96%).

### TWAP sample count

`twap_records` caps at **12** samples (`config.rs:312`). At Reflector's
300-second resolution that covers the trailing 60 minutes. The cap is
deliberate: each extra sample costs more host budget per price read, and
Reflector history beyond 12 samples adds little noise reduction in
practice.

## Price resolution pipeline

Entry point: `token_price(env, cache, asset, allow_unsafe) -> Wad`
(`controller/src/oracle/mod.rs:19-56`). Behaviour:

```
token_price(asset, allow_unsafe)
│
├─ cache hit? ── yes ──▶ return cached price_wad
│    no
├─ Market(asset).status == PendingOracle | Disabled?
│    yes ──▶ panic PairNotActive
│
├─ OracleType == None?
│    yes ──▶ panic PairNotActive
│
└─ find_price_feed(config, asset)
   │
   ├─ SpotOnly   ── cex_spot_price ──▶ price_wad
   │
   ├─ SpotVsTwap ── cex_spot_and_twap_price
   │                   │
   │                   ├─ lastprice()          → spot_wad
   │                   ├─ prices(N samples)    → twap_wad (avg)
   │                   └─ calculate_final_price(spot, twap, tolerance)
   │
   └─ DualOracle ── cex_twap_price            → safe (CEX TWAP)
                    dex_spot_price (optional) → agg  (DEX spot)
                    calculate_final_price(agg, safe, tolerance)
```

The cache (`controller/src/cache/mod.rs`) holds the resolved
`price_wad` keyed by asset for the remainder of the transaction, so
multi-asset entrypoints (e.g. liquidation) only pay the Reflector cost
once per asset.

### `calculate_final_price`

At `oracle/mod.rs:106-146`. Takes an *aggregator* price (riskier — spot
or DEX) and a *safe* price (anchor — TWAP or CEX TWAP) plus the
tolerance bands, and returns:

```
if is_within_anchor(agg, safe, first_upper, first_lower):
    return safe                     # strict band: use the conservative number
elif is_within_anchor(agg, safe, last_upper, last_lower):
    return (agg + safe) / 2         # relaxed band: midpoint dampens
else:
    if allow_unsafe:
        return safe                 # risk-decreasing op: still allow
    else:
        panic UnsafePriceNotAllowed # risk-increasing op: fail closed
```

`is_within_anchor` (`oracle/mod.rs:338-355`) computes the ratio
`safe / agg` in RAY, rescales to BPS, and checks the band bounds.

### The `allow_unsafe_price` flag

The caller classifies the risk. The controller cache sets `allow_unsafe_price`
per asset-read:

- **Supply, repay** (risk-decreasing) → `true`. Blocking a user who deposits
  or reduces debt during extreme oracle divergence would worsen protocol
  health — let them through on the safe anchor.
- **Borrow, withdraw, collateral decrease, liquidation-target value**
  (risk-increasing) → `false`. A diverged price here would let the user
  borrow against stale collateral or withdraw past the real LTV.

During a Reflector outage or a brief spot/TWAP divergence, supply and repay
still succeed on the safe anchor; new borrows and withdrawals revert with
`UnsafePriceNotAllowed`.

## Staleness

Three checks at `controller/src/oracle/mod.rs:163-178`:

1. **Hard staleness** — if `now - feed_timestamp > max_price_stale_seconds`,
   panic `PriceFeedStale` (error 206). Covers CEX spot reads (`:198`) and the
   **oldest** sample in the TWAP window (`:252`). The oldest sample carries
   the weakest freshness guarantee, so it gates the whole window.
2. **Future-timestamp guard** — tolerates 60 seconds of clock skew; a
   timestamp more than a minute ahead also panics `PriceFeedStale`. This
   blocks a compromised feed from extending its own staleness window.
3. **Soft staleness for DEX** — stale DEX spot in `DualOracle` mode returns
   `None` instead of panicking (`:326-329`). The CEX TWAP anchor then serves
   as the final price; the DEX side only ever tightens.

A valid TWAP needs **≥50%** of `twap_records` populated (`oracle/mod.rs:247`).
Below that, the call panics `TwapInsufficientObservations` (error 219). This
catches a Reflector that responds but has gaps in its window, for example
right after a contract redeploy.

## Zero and missing prices

| Condition | Behaviour |
|---|---|
| `lastprice` returns `None` | panic `NoLastPrice` (error 210). |
| `lastprice` returns a non-positive value | panic `InvalidPrice` (error 217). |
| Every TWAP sample is `None` with `count == 0` | fall back to spot (spot must exist). |
| TWAP count below 50% threshold | panic `TwapInsufficientObservations`. |
| DEX feed unavailable in `DualOracle` | return `None`; the CEX TWAP becomes the final price. |

Supply with a zero-price asset always fails. The retired
`flow_oracle_tolerance` target covered this; `flow_e2e`'s `OracleJitter` op
covers it now.

## USD-WAD normalisation

Reflector returns 14-decimal fixed-point. The controller normalises at
the edge using `Wad::from_token` (`common/src/fp.rs:131-132`), which
calls `rescale_half_up(price, 14, 18)`. After that every downstream
call (HF computation, LTV check, seizure math) works in WAD.

A Reflector tick `100_000_000_000_000` (= $1.00 in 14-dec fixed) becomes
`1_000_000_000_000_000_000` (= 1 WAD). `Wad` is the single source of
truth for "USD value" in the protocol; see
`architecture/INVARIANTS.md §1` for the fixed-point domain map.

The protocol assumes Reflector quotes in USD. Non-USD quote assets would
require double-hop conversion and compounding tolerance bands — complexity
the current design deliberately sidesteps.

## Mock Reflector (test harness)

`test-harness/src/mock_reflector.rs` implements the same interface and keeps
spot and TWAP in separate storage slots, letting tests inject divergence
directly. Surface:

| Method | Purpose |
|---|---|
| `set_price(asset, price_wad)` | Stores spot (14-decimal internally) under `MockKey::Spot`. |
| `set_twap_price(asset, price_wad)` | Stores TWAP under `MockKey::Twap`. |
| `lastprice(asset)` | Returns the spot record with `env.ledger().timestamp()`. |
| `prices(asset, records)` | Returns a vector of TWAP records (falls back to spot if no TWAP set). |
| `decimals()` | Hardcoded `14`. |
| `resolution()` | Hardcoded `300`. |

`flow_e2e::Op::OracleJitter` drives this surface: it pushes the spot by
±500 bps while TWAP stays pinned, then verifies that a brief divergence
trips the risk-increasing guards but leaves the safe-side ops alone. The
retired `fuzz_oracle_tolerance` used the same pattern.

## Events

Oracle config emits `UpdateAssetOracleEvent`
(`common/src/events.rs:286-289`) on every `configure_market_oracle` and
`edit_oracle_tolerance`. Payload `EventOracleProvider` carries the full
tolerance struct, the reflector CEX/DEX oracle addresses, the asset-kind
code, the decimal counts, the TWAP sample count, and the staleness
window — enough for an indexer to reproduce the full price-resolution
shape without reading storage.

**No per-call oracle events.** A tolerance breach or staleness rejection
surfaces as a contract error (`OracleError` variants,
`common/src/errors.rs:84-105`), not an event. Observability layers read
Soroban's diagnostic-event stream, not protocol events, for per-call oracle
behaviour.

## Failure modes and how to debug

| Symptom | Likely cause | Where to look |
|---|---|---|
| `PairNotActive` | Market has `OracleType::None` or `status` is `PendingOracle` / `Disabled`. | `MarketConfig` in controller storage; either `configure_market_oracle` never ran or an admin paused the market. |
| `NoLastPrice` | Reflector lacks a recent sample for this asset, or the symbol is wrong. | Confirm `reflector_cex_symbol` / `reflector_cex_asset_kind` matches the Reflector feed's listing. |
| `PriceFeedStale` | Reflector gap ≥ `max_price_stale_seconds`, or clock-skewed timestamp. | Check Reflector's most-recent sample vs ledger time. |
| `UnsafePriceNotAllowed` | Spot vs TWAP divergence > second tolerance band on a risk-increasing op. | Compare `lastprice` and `prices` window; if legitimate (flash crash), wait for convergence or widen the band via `edit_oracle_tolerance`. |
| `InvalidPrice` | Reflector returned `<= 0`. | Reflector feed misconfiguration; should never happen for a healthy feed. |
| `TwapInsufficientObservations` | TWAP window has too many missing samples. | New market or post-redeploy warm-up; wait for the window to fill. |

## What's not here

- **No oracle-fallback registry.** Each market carries one CEX feed plus
  an optional DEX feed (`DualOracle` mode). An operator swaps feeds
  manually; no automatic failover to a backup Reflector exists.
- **No circuit breakers.** A price jump through both tolerance tiers halts
  risk-increasing ops on one asset; the other markets keep running. No
  oracle-driven switch halts the whole controller.
- **No heterogeneous quote asset.** The protocol assumes Reflector quotes in
  USD. BTC- or ETH-quoted feeds would need double-hop normalisation and
  compounding tolerance bands — unsupported today.
