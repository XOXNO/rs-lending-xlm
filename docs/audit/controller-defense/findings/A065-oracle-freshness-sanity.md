# A065 — Oracle price freshness / sanity band usage on risk paths

- Agent: A065
- Theme: T4 / T8 (untrustworthy input + undefended-gap quantification at TB3)
- Severity: low (residual config / availability); core fail-closed path: **defended**
- Status: defended (mutations); partial (intentional non-pricing paths + config residuals)
- Paths:
  - `contracts/controller/src/context/oracle.rs` (`fetch_prices`, `cached_price`)
  - `contracts/controller/src/context/mod.rs` (`load_markets` → prices + indexes)
  - `contracts/controller/src/external/price_aggregator.rs` (`fetch_prices` hard / `fetch_prices_status` soft)
  - `contracts/controller/src/risk/totals.rs` (valuation; uses `feed.price` only)
  - `contracts/controller/src/risk/validation.rs` (`require_post_pool_risk_gates`)
  - `contracts/controller/src/strategies/mod.rs` (`prefetch_strategy_prices`)
  - `contracts/controller/src/views.rs` (`get_all_market_indexes_detailed` soft quotes)
  - Upstream SoT: `contracts/price-aggregator/src/engine.rs` (`Outcome::failure`, `force`, `to_status`)
  - Shared helpers: `common/src/oracle/observation.rs`, `common/src/validation.rs` (`validate_sanity_bounds*`)
- Defense: Mutating risk paths obtain prices only via aggregator `prices()` (hard). Aggregator `force`/`failure` rejects stale, deviant, non-positive, and out-of-sanity-band outcomes before a `PriceFeedRaw` is returned. Controller Cache memoizes that snapshot for the invocation (INV-ORACLE-03). Soft `quotes()` / `PriceStatus` is view-only.
- Gap: Controller never re-checks `feed.timestamp`, staleness windows, or sanity bands itself — it trusts the aggregator pointer and the hard API. Supply / repay / debt-free exits intentionally skip pricing. Dual-source prices still inside configured stale windows can skew blends (config residual, not a missing panic). Poisoned stale legs can block liquidation until the feed recovers (fail-closed availability).
- Impact: Wrong prices cannot silently pass a gated mutation if the live aggregator enforces `failure`. Blast radius of a compromised / mis-pointed aggregator or of over-wide per-asset stale/sanity config is protocol-wide valuation for every HF/LTV/liquidation decision (see A009, STRIDE Tamper.1 / TB3). Stale-leg planting turns undercollateralized accounts temporarily unliquidatable (DoS on recovery, not mispricing).
- Evidence: INV-ORACLE-01..04; ADR-0004, ADR-0005, ADR-0014; STRIDE TB3 / Tamper.1 / Info.4 / I25; harness `audit_supply_stale_shield`, `oracle/tolerance/staleness`, `oracle/tolerance/dual_source` sanity tests, `strategy` sanity bound edges, `governance/immediate` tighten-only band; Certora `freshness*` + `price_cache_consistency`; peers A072, A094, A086, A009, A029.
- Opinion: Correct trust split — freshness and sanity belong in the aggregator; the controller’s job is fail-closed consumption + one snapshot. Do not add redundant controller-side stale/sanity checks unless the aggregator pointer is no longer the single SoT. Track residuals as ops/config (stale windows, single-source band width, ORACLE-role tighten) and the known plant-stale-leg recovery DoS.

---

## 1. Scope and trust boundary

A065 covers how **controller risk paths** obtain and use oracle prices for USD valuation (HF, LTV collateral, liquidation sizing, keeper FullTuple HF), specifically:

1. Whether **freshness** (`max_price_stale_seconds` / per-source `max_stale_seconds`, future skew) is enforced before money-moving risk decisions.
2. Whether **sanity bands** (`min_sanity_price_wad` / `max_sanity_price_wad`) gate those same decisions.
3. Whether the controller **re-validates** or merely **consumes** aggregator output.
4. Which entrypoints **skip** pricing by design.

Trust boundary **TB3** (STRIDE): controller ↔ price aggregator. The controller must consume complete, validated snapshots; any missing, stale, out-of-band, or disagreeing required price must fail closed (INV-ORACLE-01, ADR-0005).

Out of scope for depth (peer agents): aggregator Wasm/admin surface alone (A009/A029), mid-tx market-index cache footguns (A094/A086), post-pool gate arithmetic (A072), lying tokens (A055).

---

## 2. Architecture: who enforces what

### 2.1 Controller surface (thin client)

| Symbol | File | Role |
|---|---|---|
| `Cache::fetch_prices` | `context/oracle.rs:20–28` | Bulk-fetch missing assets; write `token_prices` |
| `Cache::cached_price` | `context/oracle.rs:32–38` | Panic `OracleNotConfigured` (#216) if uncached; **no** timestamp/sanity inspect |
| `Cache::load_markets` | `context/mod.rs:70–74` | Dedup tokens → `fetch_prices` + indexes |
| `external::fetch_prices` | `external/price_aggregator.rs:20–30` | Calls aggregator **`prices`**; missing key → `#216` |
| `external::fetch_prices_status` | `external/price_aggregator.rs:35–45` | Calls aggregator **`quotes`**; missing → `PriceStatus::unusable()` |
| `prefetch_strategy_prices` | `strategies/mod.rs:59–65` | Prefetch account + extras before strategy legs |
| `account_price_assets` | `risk/totals.rs:24–39` | Dedup supply/borrow/extra underlyings for prefetch |
| `calculate_account_risk_totals*` | `risk/totals.rs:169–229` | Loads markets; values with **`feed.price` only** |
| `require_post_pool_risk_gates` | `risk/validation.rs:31–60` | Debt-free skip; else LTV / HF / min collateral on totals |

`PriceFeed` / `PriceFeedRaw` carry `price_wad`, `asset_decimals`, and `timestamp`. On risk paths the controller uses **`price` only**. Grep across `contracts/controller/src` shows **zero** reads of `feed.timestamp` for gating; `price_timestamp` appears only when forwarding soft status in `views.rs`.

### 2.2 Aggregator SoT (hard vs soft)

```text
Mutations / gates          Views (observability)
─────────────────          ─────────────────────
prices() ──► resolve ──►   quotes() ──► resolve_status
              force()                    to_status()
              Outcome::failure()         valid/stale/deviation flags
              panic on fail              never panics per-key
```

`Outcome::failure` (`engine.rs:126–147`) checks, in order:

1. Carried resolution error  
2. Missing oracle config  
3. **`stale` → `PriceFeedStale` (#206)**  
4. **`deviation` → `UnsafePriceNotAllowed` (#205)**  
5. Non-positive price → `InvalidPrice`  
6. **Outside `[min_sanity_price_wad, max_sanity_price_wad]` → `SanityBoundViolated` (#223)**

`force` panics on any failure; only then emits `PriceFeedRaw`. Soft `to_status` reports the same classification as `valid` / `error_code` without panicking — STRIDE Info.4: controllers must not value from `quotes()`; they do not.

### 2.3 Freshness primitives (common)

| Constant / helper | Location | Meaning |
|---|---|---|
| `is_stale(now, feed_ts, max_stale)` | `observation.rs:61–62` | Age strictly greater than budget |
| `is_future_at` / `MAX_FUTURE_SKEW_SECONDS=60` | `observation.rs:13,77–81` | Future drops leg (INV-ORACLE-04) |
| `MIN/MAX_PRICE_STALE_SECONDS` | `60` / `93_600` (26h) | Envelope for configured budgets |
| `MAX_LEG_AGE_SPREAD_SECONDS` | `3_600` | Dual-leg age gap |

Sanity admission (`common/src/validation.rs`):

- `validate_sanity_bounds` — positive, ordered, ≤ `MAX_REASONABLE_PRICE_WAD`, half-width ≥ `MIN_SANITY_BAND_BPS` (50)
- `validate_single_source_sanity_band` — width ≤ `MAX_SINGLE_SOURCE_SANITY_BAND_BPS` (1000) when not dual
- `validate_lp_sanity_band` — LP backstop ≤ `MAX_LP_SANITY_BAND_BPS` (8182 ≈ 10× fair-value range)

Runtime: sanity is **pass/fail on the blended (or single) price**, never a clamp (ADR-0004). Immediate `set_sanity_band` may **only tighten** (threat-model; `admin.rs:197–213`; INV-AUTH-04).

---

## 3. How risk totals consume prices

`calculate_account_risk_totals_body` (`totals.rs:169–229`):

1. `cache.load_markets(portfolio_hub_keys(...))` → hard prices for every supply/borrow underlying.
2. Per supply position: `cached_price` → `position_value` / `position_value_floor` with `feed.price`.
3. Per debt position: `position_value_ceil` with `feed.price`.
4. HF = weighted_collateral / total_debt (floor, saturating), or `i128::MAX` if debt-free.

No path in totals inspects freshness or sanity. If `load_markets` / `fetch_prices` returned, the aggregator already accepted the quote. If any required asset fails hard resolution, the whole mutation panics inside the cross-contract `prices` call (or `#216` on a missing map entry).

`require_post_pool_risk_gates` is a pure consumer of those totals. Debt-free accounts return early **without** calling totals — so a debt-free account never needs a live oracle for that gate (withdraw full exit with broken oracle is intentional; harness `test_withdraw_full_exit_works_with_broken_oracle`).

---

## 4. Entrypoint matrix (valuation vs skip)

| Path | Prices fetched? | Via | Fail-closed on stale/sanity? | Notes |
|---|---|---|---|---|
| `borrow` | Yes (post-pool solvency → totals → `load_markets`) | hard `prices` | Yes | Also any pre-gate that loads markets |
| `withdraw` (with debt) | Yes (`enforce_post_pool_solvency`) | hard | Yes | |
| `withdraw` (debt-free) | Gate skipped | — | N/A | Broken oracle OK |
| `supply` | **No** (no solvency gate) | — | N/A | HF non-decreasing; can plant stale-priced legs |
| `repay` | **No** | — | N/A | HF non-decreasing; `test_full_repay_fires_zero_redstone_calls` |
| Strategies (`multiply`, swaps, flash_position, migrate, repay-with-collateral) | Yes (`prefetch_strategy_prices` + finalize gates) | hard | Yes | Prefetch before callback/legs; finalize reuses Cache (INV-ORACLE-03 / ADR-0020 flash snapshot) |
| `liquidate` / estimate / plan | Yes (`build_liquidation_plan` → totals) | hard | Yes | Every supply+debt asset must resolve |
| `clean_bad_debt` / socialize | Yes (risk totals / plan) | hard | Yes | Stale leg blocks cleanup (`audit_supply_stale_shield`) |
| Keeper FullTuple HF check | Yes (`keepers.rs:222–233`) | hard | Yes | Fail closed before adverse param commit |
| `get_market_indexes_detailed` | Soft status | `quotes` | No panic; flags exposed | Observability only |
| Account HF / risk views that call totals | Hard via `load_markets` | `prices` | Yes | View can revert on bad oracle |

**ADR-0005 auditor focus** is met for liquidation and keepers: both take the hard path. Supply/repay skips are deliberate non-valuation mutations, not permissive fallbacks on a failed price.

---

## 5. INV-ORACLE mapping (controller-visible)

| Invariant | Controller enforcement | Aggregator enforcement |
|---|---|---|
| **INV-ORACLE-01** fail closed | Only hard `prices` on mutations; panic on missing cache/key | `failure` + `force` |
| **INV-ORACLE-02** dual legs | None (trust aggregator) | `blend` / deviation / both legs |
| **INV-ORACLE-03** one snapshot | `Cache.token_prices` memo; `collect_uncached_keys`; strategy prefetch once | Session cache inside aggregator call |
| **INV-ORACLE-04** future skew | None | Observation drop via `is_future_at` |

Certora: aggregator `freshness_rules` / `freshness.conf`; controller harness ghosts replay one price per asset per rule (`certora/controller/harness/ghost_prices.rs`, `external/price_aggregator.rs`) modeling INV-ORACLE-03. Ghost `fetch_prices_status` nondets flags but assumes `valid ⇒ !stale && !deviation` — production mutations do not read that path.

---

## 6. Defenses that work (evidence)

### 6.1 Hard API on every valuation mutation

- `fetch_prices` → `PriceAggregatorClient::prices` only.
- Unit killer: `contracts/controller/tests/context/oracle.rs` (cache populate / skip-if-cached).
- Aggregator audit tests: `audit_hard_price_reverts_stale_while_status_soft_flags`, `audit_hard_price_reverts_outside_sanity_band`, inclusive band edges (`contracts/price-aggregator/tests/surface.rs`).

### 6.2 Stale blocks risk-increasing / risk-gated exits

Harness `tests/test-harness/tests/oracle/tolerance/staleness.rs`:

- Supply after aging: **allowed** (no price read).
- Borrow after aging: **`PriceFeedStale`**.
- Withdraw with borrows after aging: **`PriceFeedStale`**.
- Dual-source stale anchor / missing TWAP: strict borrow blocked (`UnsafePrice` / stale as applicable).

### 6.3 Sanity band blocks valuations

- `test_sanity_bound_blocks_price_above_ceiling` / `below_floor` / tampered zero state (`oracle/tolerance/dual_source.rs`).
- Strategy inclusive edge then one-over reject (`strategy/router.rs`).
- Governance immediate path: tighten-only; band excluding live price fails closed at read (`governance/immediate.rs`).

### 6.4 Liquidation / cleanup fail closed on any required stale leg

`audit_supply_stale_shield.rs`:

1. Debted account can **supply** a dust WBTC leg while WBTC feed is stale (supply skips oracle).
2. Twin account liquidates fine with fresh feeds.
3. Poisoned account: liquidate / clean bad debt / withdraw collateral → **`PRICE_FEED_STALE`**.
4. After WBTC refreshed, identical liquidation succeeds.

This is fail-closed **safety** with an **availability** cost (see §7).

### 6.5 Soft quotes isolated to views

`get_all_market_indexes_detailed` uses `fetch_prices_status` and forwards `stale` / `deviation` / `valid` / `error_code`. Mutations never call it for HF. Matches STRIDE Info.4 residual (off-chain consumers must read flags).

### 6.6 Snapshot coherence

- First successful hard fetch pins `token_prices` for the rest of the invocation.
- Strategy finalize and post-pool gates reuse the same Cache instance → same prices (A045/A094: price prefetch intentional; index refresh is separate).
- Flash callback cannot refresh prices through controller mutators (flash guard); finalize uses pre-callback snapshot (ADR-0020).

---

## 7. Gaps and residuals (not novel criticals vs SEED)

### 7.1 No controller-side re-validation of timestamp / sanity — **accepted design**

`cached_price` would happily return a hand-seeded `timestamp: 0` feed in tests. Production integrity depends entirely on:

1. Correct `storage::get_price_aggregator` pointer (owner / timelock; A009).
2. Aggregator `prices()` implementing `failure` (immutable Wasm / replacement via `SetPriceAggregator`).

Adding duplicate stale/sanity checks in the controller would not help a malicious aggregator that lies about `price_wad` while forging a fresh timestamp. **Defense in depth that matters is pointer governance + aggregator correctness**, not a second timestamp compare.

**Status:** defended by TB3 design. Severity if broken: critical, but the break is aggregator compromise / mis-deploy, already tracked.

### 7.2 Supply (and repay) can proceed without fresh prices — **intentional**

Allows:

- Liveness when oracles are down for pure collateral top-ups / debt reduction.
- **Planting** a position whose asset later cannot be priced → blocks liquidation/cleanup until the feed recovers (`audit_supply_stale_shield`).

Impact: temporary **liquidation DoS** on the affected account (and any socialize path that must price all legs), not silent under-collateralized borrow. Protocol prefers stuck over wrong (ADR-0005). Ops mitigation: listing policy / dust controls / avoiding long single-source stale windows on fringe assets.

Related harness: `audit_liquidate_and_clean_stale_leg.rs`.

### 7.3 Dual-source skew **inside** configured stale windows — **config residual**

`audit_borrow_withdraw_liquidate_stale_anchor_blend.rs` demonstrates: with a wide RedStone anchor window (e.g. 86_400s) and a lagging but still “fresh” anchor at a frozen high price, the midpoint blend can inflate collateral enough to pass a borrow that fails under an honest fresh anchor (~5% skew in the fixture, within 1000 bps tolerance).

This is **not** a missing `PriceFeedStale` panic — both legs pass freshness and tolerance by configuration. Residuals:

- Per-asset `max_stale_seconds` / `max_price_stale_seconds` sizing (STRIDE Tamper.1 residual Low with ✅).
- Admission attestation point-in-time for XOXNO `max_submission_age` (STRIDE Tamper.10 Medium ⚙) — outside controller.
- Single-source / LP reliance on sanity band as sole backstop (threat-model § sanity band).

Controller cannot fix this without changing blend policy or re-reading soft status (which would break INV-ORACLE / ADR-0004).

### 7.4 Debt-free oracle skip

`require_post_pool_risk_gates` no-ops when `account.debt_free()`. Full collateral exit with broken oracle is desired. A path that **opens debt** without going through totals would be critical; current borrow/strategy/liquidation paths do not.

### 7.5 `asset_decimals` unused on risk totals

`PriceFeed.asset_decimals` is stored and returned by the aggregator but risk valuation uses share×index→WAD×`price` (`common/src/rates/value.rs`) without `from_token`. Decimal consistency is an aggregator listing / price-dimension assumption, not a freshness/sanity bypass. Flagged only so A065 does not imply the controller “uses the full feed.” Cross-check with numeric-bounds / listing runbooks if a separate decimals audit is scoped.

### 7.6 View soft-status misuse

Off-chain / keepers reading `price_wad` from `get_market_indexes_detailed` without `valid` can act on unusable quotes (STRIDE Info.4). On-chain mutations are unaffected.

---

## 8. Call-graph summary (mutation valuation)

```text
borrow / withdraw(+debt) / strategy_finalize / liquidate.plan / keeper FullTuple
  └─ require_post_pool_risk_gates | build_liquidation_plan | HF assert
       └─ calculate_account_risk_totals
            └─ load_markets
                 └─ fetch_prices (Cache)
                      └─ external::fetch_prices
                           └─ PriceAggregator.prices
                                └─ engine::resolve → force → failure
                                     ├─ stale?
                                     ├─ deviation?
                                     ├─ price_wad > 0?
                                     └─ sanity band?
```

Prefetch shortcut (strategies): `prefetch_strategy_prices` → same `fetch_prices` before legs; later `load_markets` hits cache (INV-ORACLE-03).

---

## 9. Peer cross-links

| Peer | Relationship |
|---|---|
| **A072** | Post-pool gates defended; explicitly defers oracle fail-closed to INV-ORACLE — this file is that deferral. |
| **A094 / A086** | Cache staleness for **indexes / sync**, not oracle freshness windows. Price memoization here is a feature (one snapshot). |
| **A009 / A029** | Aggregator pointer / protocol instance storage — outer trust root for this defense. |
| **A045** | Flash finalize uses pre-callback price snapshot — consistent with INV-ORACLE-03. |
| **A025** | Repay skips oracle — agrees; HF non-decreasing. |
| **A024** | Debt-free withdraw + broken oracle — agrees with gate skip. |
| **SEED / PRELIMINARY** | Aggregator + XOXNO owners as trust roots — unchanged; A065 does not claim a novel critical inside controller risk math. |

No disagreement file warranted: peers correctly treat oracle policy as upstream of controller gates.

---

## 10. Impact quantification

| Failure mode | Who can cause | Funds / safety effect | Availability |
|---|---|---|---|
| Aggregator correctly rejects stale/sanity | Market / feed outage | None (revert) | Gated actions pause per asset |
| Aggregator bug skips `failure` | Deploy/upgrade of aggregator | Full mis-valuation on all HF paths | — |
| Controller pointed at hostile aggregator | Owner / Sensitive `SetPriceAggregator` | Same | — |
| Plant stale-priced supply leg | Anyone who can supply that asset into a debted account | Cannot steal via wrong price; blocks liq/clean | Per-account recovery DoS until feed fresh |
| Over-wide stale window + dual blend skew | Governance listing config | Over-borrow within tolerance band | — |
| ORACLE role tightens sanity to exclude price | Compromised ORACLE key | Fail closed (kill switch) | Asset unusable for valuation |

---

## 11. Tests & formal evidence checklist

| Layer | Artifact |
|---|---|
| Unit (controller) | `tests/context/oracle.rs` |
| Unit (common) | `tests/oracle/observation.rs`, `tests/validation.rs` (sanity bounds) |
| Aggregator | `tests/surface.rs` hard vs soft; `tests/oracle/engine.rs` status flag semantics |
| Harness | `oracle/tolerance/staleness.rs`, `dual_source` sanity, `controller/audit_supply_stale_shield.rs`, `audit_*_stale_leg`, `security_audit*poc_stale_oracle*`, `strategy/router` sanity edges, `governance/immediate` ratchet |
| Certora | `certora/price-aggregator/spec/freshness_rules.rs`, `oracle_rules.rs` `price_cache_consistency`; controller ghost price harness |
| Docs | `invariants.md` INV-ORACLE-01..04; ADR-0004/0005/0014; `errors.md` #205/#206/#216/#223; threat-model oracle rows; STRIDE TB3/Tamper.1/Info.4 |

---

## 12. Opinion / recommendations

1. **Verdict:** Controller risk paths are **defended** for freshness and sanity **by fail-closed hard consumption** of the price aggregator. The controller correctly does not re-implement stale/sanity arithmetic.
2. **Do not** “fix” by having `cached_price` or `totals` compare timestamps — that adds WASM cost and false confidence if the aggregator is wrong.
3. **Keep** soft `quotes` off mutation paths (already true).
4. **Ops / listing:** treat per-asset stale ceilings and single-source sanity width as security parameters equal to LTV; document the plant-stale-leg recovery DoS in liquidation runbooks (partially present for force-socialize).
5. **Tracking only:** dual-source in-window skew (harness fixture) and Tamper.10 attestation drift remain governance/aggregator residuals, not controller holes.
6. Optional hardening (non-blocking): integration assert that every `#[contractimpl]` mutator which calls `calculate_account_risk_totals` / `require_post_pool_risk_gates` never calls `fetch_prices_status` — today grep-clean for mutations.

**Bottom line:** Freshness and sanity bands are enforced on controller risk paths **indirectly but reliably** through `prices()` → `Outcome::failure`. The controller’s defensive job — hard API only, one Cache snapshot, debt-free/supply/repay skips that cannot worsen solvency — matches ADR-0005 and INV-ORACLE-01..03.
)
