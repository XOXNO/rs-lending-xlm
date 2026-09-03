# A035 — Certora harness storage overrides: false-confidence risks

- Agent: A035
- Theme: T2 (storage mutations / verification boundary; overlaps T6 cache, T7 oracle ghosts, A030 flash-guard sibling, A031 NFT, A087 prefetch)
- Severity: info
- Status: partial
- Paths:
  - `contracts/controller/src/storage/mod.rs:1-41` (`verification_storage` mount + re-exports)
  - `certora/controller/harness/storage.rs` (full additive helper surface)
  - `contracts/controller/src/external/mod.rs:1-18` (pool / price / NFT path swap)
  - `certora/controller/harness/external/{pool,price_aggregator,position_nft}.rs`
  - `certora/controller/harness/ghost_prices.rs`
  - `contracts/controller/src/spec_hooks.rs` (`fetch_market_indexes` no-op; solvency ghost hook)
  - `contracts/controller/src/risk/totals.rs:117-159` (risk-totals body vs `apply_summary!`)
  - `certora/controller/spec/fixture.rs` (HUB_ID / SPOKE_ID / empty-book / wellformed assumes)
  - `certora/controller/spec/README.txt` (stated proof assumptions)
  - `certora/README.md` (“How to read a proof result”, Important proof boundaries)
  - `contracts/controller/Cargo.toml:27-42` (certora feature graph; does **not** enable `common/certora`)
- Defense: Durable controller storage accessors (`account` / `hub` / `protocol` / `spoke`) remain the production modules under `feature = "certora"`. The harness **adds** rule-facing helpers and replaces **cross-contract FFI**, not the key families that hold shares, debt, usage, or the flash-guard temp flag. Deployable WASM is built without `certora` (ADR-0017 / `make deploy-artifacts` vs `make certora-wasm`). Suite docs already label pool/oracle summaries as assumptions, not refinements.
- Gap: Several harness helpers and substitutions diverge from production semantics in ways that can make a green Certora report look broader than the model. Highest-signal residuals: (1) `hub0` / fixture pins collapse multi-hub and multi-spoke state spaces; (2) `get_account_attrs` fabricates `SPOKE_ID`+`Normal` when meta is absent; (3) `get_position` Borrow-arm zero-fills supply-only risk fields; (4) `storage::market_index::{get_market_index,get_market_params}` call `LiquidityPoolClient` directly and **bypass** the ghost-memoised `external/pool` summaries; (5) `fetch_market_indexes` is a certora no-op; (6) most rule modules replace `calculate_account_risk_totals` with a havoc summary; (7) NFT ownership is a controller-local ghost map, not the real NFT contract storage layout. None of these are production fund bugs; all are **verification epistemology** hazards.
- Impact: No direct theft, share mint, or flash-guard bypass from the harness itself — certora code is not in deploy WASM. Residual blast radius is **review false confidence**: treating a controller verdict as proof of pool share math, multi-hub isolation, real NFT layout, prefetch batching, or risk-totals arithmetic (outside health/solvency focused builds). A missed production bug in an unmodeled dimension could ship behind “Certora passed.”
- Evidence: Source of `verification_storage` + `external` cfg mounts; A030 note that flash-guard helpers are not replaced; A087 note that index prefetch is no-op under certora; `index_rules.rs` module docs explaining why ABI double-read through the harness proves nothing; `spec/README.txt` lines 11–20; peers A021–A034 (real storage), A065/A086/A087/A094 (oracle/cache), A067 (fixture floor=0).
- Opinion: Treat controller Certora as **conditional evidence on an explicit model**. The load-bearing fact for this scope is that **persistent/temporary controller keys are not swapped**. Read every green rule against the helper/FFI/summary table below before citing it as storage-defense evidence.

## Method

1. Read `COORDINATION.md`, `SEED.md`, AGENT_MANIFEST A035 row, peer A030/A021/A028/A031/A087.
2. Mapped `storage/mod.rs` re-exports vs `harness/storage.rs` symbols (additive vs shadowing).
3. Traced every harness helper call site under `certora/controller/spec/`.
4. Compared production `external/pool.rs` (LiquidityPoolClient) vs certora harness pool + ghost memoization vs harness `storage::market_index` (client bypass).
5. Catalogued fixture narrowing, risk-totals feature split, NFT ghost keys, and documented suite assumptions.
6. No production Rust edited; findings-only.

---

## 1. Mount architecture (what “override” actually means)

### 1.1 Storage module

```10:41:contracts/controller/src/storage/mod.rs
#[cfg(feature = "certora")]
#[path = "../../../../certora/controller/harness/storage.rs"]
mod verification_storage;
// ... explicit pub(crate) use of account / hub / protocol / spoke ...
#[cfg(feature = "certora")]
pub(crate) use verification_storage::*;
```

| Layer | Under `certora` | Production deploy |
|---|---|---|
| `account.rs` / `hub.rs` / `protocol.rs` / `spoke.rs` | **Compiled and re-exported** | Same |
| Flash guard `with_flash_guard` / `is_flash_loan_ongoing` | Real helpers (A030) | Same |
| `verification_storage` | **Added** on the `storage::` namespace | Absent |
| Name collisions with production accessors | None (certora crate builds; `*` only adds new paths) | N/A |

So “harness storage overrides” is a **misnomer for durable keys**: the suite does not replace `get_supply_positions` / `set_debt_positions` / `get_spoke_usage` / `set_pool` / flash temp storage. It adds rule sugar and redirects **views of external state**.

### 1.2 External FFI (true wholesale swap)

```1:18:contracts/controller/src/external/mod.rs
#[cfg(not(feature = "certora"))]
pub(crate) mod pool;
#[cfg(feature = "certora")]
#[path = ".../harness/external/pool.rs"]
pub(crate) mod pool;
// same pattern for price_aggregator, position_nft
```

Every production call site of `crate::external::pool::*` / price / NFT under the certora feature hits summaries or ghosts. That is intentional tractability (`spec/README.txt`), and it is the dominant reason a controller verdict is **conditional**.

### 1.3 Artifact boundary

- Deploy / optimize builds: no `certora` feature → no `verification_storage`, no ghost statics, real FFI clients.
- Prover builds: `make certora-wasm` → `--features "certora,certora-focused,<rule-module>"`.
- Controller `certora` does **not** pull `common/certora`, so `common::rates::simulate_update_indexes` in controller index rules is the real body (`index_rules.rs` header) — a deliberate split from the pool layer’s monotone havoc summary.

---

## 2. Additive helper inventory (`harness/storage.rs`)

| Symbol | Spec usage | Production analogue | Semantic delta |
|---|---|---|---|
| `hub0(asset)` | Also duplicated locally in almost every `*_rules.rs` | `HubAssetKey { hub_id, asset }` | **Forces `fixture::HUB_ID` (1)**. Multi-hub same-SAC books are invisible to helpers keyed only by `Address`. |
| `get_position(..., Deposit/Borrow, &Address)` | liquidation / strategy / spoke rules | Direct map get on `HubAssetKey` | Always reads `hub0(asset)`. Borrow maps `DebtPositionRaw` → `AccountPositionRaw` with **LTV/threshold/bonus/fees = 0**. |
| `get_position_list` | strategy rules | Map `.keys()` | Returns `Vec<Address>` only — **drops `hub_id`**. |
| `get_account_attrs` | spoke_rules heavily | `try_get_account_meta` / `get_account_meta` | Missing meta → **`{ spoke_id: SPOKE_ID, mode: Normal }`**, not panic/`None`. |
| `asset_pool::get_asset_pool(env, _asset)` | (available; sparse) | `storage::get_pool` | Ignores `asset`; matches today’s single-pool protocol but **looks** like a per-asset pool API. |
| `market_index::get_market_index` | `index_sanity` | `Cache::cached_market_index` → `fetch_pool_bulk_indexes` | Calls **`LiquidityPoolClient::get_sync_data` directly** (see §4). |
| `market_params::get_market_params` | (helper) | sync blob `.params` via external harness | Same direct-client path as market_index. |
| `accounts::get_account_data` | (helper) | meta.spoke_id | Uses panicking `get_account_meta` (unlike attrs default). |
| `positions::get_scaled_amount` / `count_positions` / `get_position_list` | position / consistency / market_guard / spoke | Map len / scaled field | Inherit `hub0` + Borrow field-zeroing. |

**Critical clarification:** production money paths never call these helpers. They exist so rules can assert post-state without re-implementing map keying. False confidence appears when a reader equates “proved via `storage::get_position`” with “proved the production storage API.”

---

## 3. What remains real (and is therefore citeable)

These production storage surfaces run unchanged inside certora-built WASM (rules call them directly):

| Family | Accessors | Peer |
|---|---|---|
| Account meta / supply / debt / delegates | `get_*` / `set_*` / cleanup | A021, A022–A027, A036, A037 |
| Flash temp flag | `with_flash_guard`, `is_flash_loan_ongoing`, `set_flash_loan_ongoing` | A030, A007 |
| Spoke / spoke asset / usage | `get_spoke*`, `set_spoke*`, `try_get_spoke` (certora-visible) | A028, A076–A085 |
| Protocol singletons | pool, oracles, NFT addr, limits, min-borrow floor, hubs | A029, A040 |
| Instance / user TTL renew helpers | `renew_controller_instance`, `renew_user_account` | A034, A017 |

A030’s sibling note is confirmed: harness storage does **not** desync the flash-guard key; rules that exercise `process_flash_loan` hit the real temp latch.

---

## 4. High-signal false-confidence vectors

### 4.1 Direct `LiquidityPoolClient` in storage helpers (bypass of ghosts)

Production Cache / config under certora:

`cached_market_index` / `cached_pool_sync_data` → `external::pool::fetch_*` → `ghost_prices::{market_index,sync_data}` → shared `get_sync_data_summary` (one draw per hub per rule).

Harness helper:

```74:98:certora/controller/harness/storage.rs
pub mod market_index {
    pub fn get_market_index(env: &Env, asset: &Address) -> MarketIndex {
        let pool = crate::storage::get_pool(env);
        let state = LiquidityPoolClient::new(env, &pool)
            .get_sync_data(&hub0(asset))
            .state;
        // ...
    }
}
```

`spec/README.txt`: cross-contract `call` is unimplemented and returns a **havoced** value when a summary is bypassed. Therefore:

- `index_sanity` (`storage::market_index::get_market_index` then `cvlr_satisfy!(indexes > 0)`) is a **reachability witness on unconstrained sync data**, not evidence that production index views are positive or consistent with Cache.
- The suite **already knows** this for the interesting properties: `index_rules.rs` documents that ABI double-read through the harness would compare a nondet to itself, and routes isomorphism/monotonicity through real `simulate_update_indexes` instead.

**False-confidence pattern:** citing `indexes-sanity` / `index_sanity` as “market index storage verified.”

### 4.2 `get_account_attrs` soft-default

```57:64:certora/controller/harness/storage.rs
pub fn get_account_attrs(...) -> AccountAttributes {
    try_get_account_meta(...)
        .map(...)
        .unwrap_or(AccountAttributes {
            spoke_id: fixture::SPOKE_ID,
            mode: PositionMode::Normal,
        })
}
```

Production `get_account_meta` panics `#AccountNotInMarket`. Spoke revert rules today seed accounts before reading attrs, so the default is mostly latent. Residual: a future rule that omits `seed_account` can still “see” spoke 1 / Normal and prove a gate property against a **phantom account**, while production would have failed closed earlier on meta miss (or NFT owner miss). Prefer `try_get_account_meta` / `get_account` in new rules.

### 4.3 Borrow `get_position` field zeroing

Debt storage is `DebtPositionRaw { scaled_amount }` only (A021). The helper widens to `AccountPositionRaw` with zeros. Rules that assert `scaled_amount` (liquidation decrease, strategy refinance) remain meaningful. Any rule that compared Borrow-side LTV/threshold fields through `get_position` would be proving against **fabricated zeros**, not stamped listing risk. Today’s call sites use scaled amounts — keep it that way.

### 4.4 Single-hub / single-spoke fixture compression

- `fixture::{HUB_ID,SPOKE_ID} = 1`; `hub_asset` / `hub0` always pin hub 1.
- `seed_protocol` activates hub 1 and one spoke; `seed_market` lists on that spoke only.
- Position-limit / bulk-leg rules still use multiple **assets**, not multiple hubs.

Properties **not** in the controller Certora model via these helpers: same SAC under two hubs consuming two slots; spoke-binding isolation across spoke ids; cross-hub usage caps. Those remain unit/harness / peer-finding territory (A028, A066, A083).

### 4.5 `fetch_market_indexes` no-op (`spec_hooks.rs`)

Under certora, `Cache::fetch_market_indexes` is empty. Production `load_markets` still calls it; indexes arrive via:

- lazy `cached_market_index` → ghost bulk summary, or
- `put_market_index` after mutation summaries (harness rewrites mutation indexes to the ghost snapshot).

A087 already records: prefetch batching is **not** what Certora proves. A green solvency/health rule does not imply the bulk prefetch path is correct.

### 4.6 Risk-totals summary substitution

| Build | `calculate_account_risk_totals` |
|---|---|
| Production / non-certora | Real body |
| `certora-health-rules` / `certora-solvency-rules` | Real body (fixtures kept small) |
| All other controller certora modules | `apply_summary!(..._summary)` — nondet totals with wellformed-book assumes |

Implication: strategy / liquidation / spoke / flash / position / consistency / market-guard / account-isolation / index (except direct body calls) rules that only need “the gate ran” do **not** re-prove USD aggregation, rounding direction, or multi-asset valuation. `shared/summaries/mod.rs` documents the wellformed-book premise explicitly. Citing those modules as “HF math verified” is false confidence — use health/solvency focused artifacts + `hf_lemma_rules` / common math instead.

### 4.7 NFT ghost map ≠ position-nft contract storage

`harness/external/position_nft.rs` stores `GhostNftKey::{Owner,NextId}` in **controller** persistent storage. Production ownership lives in the NFT contract; controller only holds the NFT address (A031). Rules correctly model mint/burn/owner_of **coupling** as the controller sees it, but do not prove NFT contract key layout, authorization, or TTL windows. Do not treat Certora account-create rules as a substitute for position-nft verification / harness TTL suites.

### 4.8 Pool / price mutation summaries

`harness/external/pool.rs` wraps shared nondet summaries and then **forces** `mutation.market_index = ghost_prices::market_index(...)` so post-pool Cache and gate valuation cannot disagree within a rule. That removes an entire class of inconsistent-summary counterexamples (good for INV-ORACLE-03 / single-snapshot modeling) and simultaneously means:

- Controller rules prove **write-through of whatever scaled amount the summary returned**, not pool share conservation.
- Pool accounting belongs to `certora/pool/**` (and is still under its own summaries/assumptions).

Price path: `ghost_prices::price` memoises `price_feed_summary` (positive wad, bounded decimals/timestamp). Fail-closed aggregator bands are proved in `price-aggregator` specs (A065), not by controller ghosts. `fetch_prices_status` nondets flags with `valid ⇒ !stale && !deviation` — soft status, not the hard money path.

### 4.9 Fixture assumptions that shrink the havoced heap

Sunbeam havocs storage at rule start. Fixtures deliberately cut space:

| Helper | Excludes |
|---|---|
| `seed_empty_books` | Pre-existing neighbour positions |
| `assume_books_at_most_one` | Large books (health post-gate family) |
| `assume_wellformed_book` | Illegal LTV/threshold / negative scaled |
| `seed_protocol` min-borrow = 0 | Dust floor (A067 residual) |
| `UNCONSTRAINED_CAP` | Tight spoke caps unless a rule rewrites them |
| `POSITION_LIMIT_MAX == 5` compile assert | Silent vacuity if cap changes without fixture recount |

Universal rules that keep books unbounded (frame / isolation) are stronger and costlier; empty-book rules are not “full portfolio” proofs.

### 4.10 Health ghost empty-default

`health_ghost::observed_supply/debt` return **empty maps** if the gate never ran. Post-gate implications correctly key on `gate_observed()`. A careless assert that “observed books equal storage” without the flag would pass on the debt-free skip path — the existing health rules treat this as implication, not absolute equality.

---

## 5. Reading guide: which verdicts transfer to production storage?

| Claim class | Transfers? | Why |
|---|---|---|
| Controller wrote the scaled amount returned by pool FFI into supply/debt maps | **Conditional yes** | Real storage setters; pool side summarized |
| Flash flag set/cleared around summarized flash | **Yes** (flag) | Real temp storage (A030); callback/pool fee economics summarized |
| Spoke usage delta tracking for seeded rows | **Mostly yes** | Real usage keys; exit-missing-row carve-out documented in fixture |
| Multi-hub / multi-spoke isolation | **No** (via helpers) | `hub0` / SPOKE_ID compression |
| Prefetch / `fetch_market_indexes` batching | **No** | No-op under certora (A087) |
| Risk USD / HF arithmetic in non-health/solvency modules | **No** | Havoc summary |
| Oracle fail-closed / freshness | **No** (controller) | Ghost accepts feeds; see aggregator specs (A065) |
| NFT contract storage / TTL | **No** | Ghost map on controller |
| `index_sanity` positive indexes | **No meaningful transfer** | Direct client havoc (§4.1) |
| Index isomorphism via `simulate_update_indexes` | **Yes for projection math** | Real common rates path; not a storage-layout proof |

---

## 6. Cross-links

| Peer | Relationship |
|---|---|
| A030 | Sibling: confirms flash-guard helpers not replaced by `verification_storage` |
| A021–A029, A032–A034, A036 | Real storage mutation defense; cite those for layout, not A035 helpers |
| A031 | Production NFT coupling; Certora uses ghost owner map |
| A065 | Oracle freshness owned by aggregator; controller ghosts model snapshot reuse |
| A067 | Fixture forces min-borrow floor 0 |
| A087 / A094 / A086 | Cache prefetch / put_market_index; certora no-op prefetch |
| A088 | Certora sync vs bulk nondet note — mitigated for controller by `ghost_prices`, still summary-level |
| `certora/controller/spec/README.txt` | Canonical assumption list for reviewers |

---

## 7. Remediation / hygiene (verification-only; no production change required)

1. **Document in `harness/storage.rs` rustdoc** (top of file): “additive rule helpers; durable keys are production modules; `market_index::*` bypasses ghosts — prefer `Cache` / `ghost_prices` / `simulate_update_indexes`.”
2. **Stop new rules from calling `LiquidityPoolClient` via storage helpers**; route index reads through `ghost_prices` or the real simulate path (as `iso_*` already does).
3. **Prefer `try_get_account_meta`** over `get_account_attrs` in rules, or `cvlr_assume!(try_get_account_meta(...).is_some())` before attrs.
4. **Never assert Borrow risk fields** through `get_position`; use `get_debt_positions` / scaled helpers only.
5. When citing Certora in audits/ADRs, name the **feature artifact** (`certora-health-rules` vs summary modules) and the **summary boundary**.
6. Optional: delete or rewire `index_sanity` so it cannot be misread as index integrity evidence.

---

## 8. Verdict

Persistent and temporary **controller** storage used by production money paths is not replaced under Certora; flash-guard, account maps, spoke usage, and protocol singletons remain the real accessors. False confidence comes from the **additive helpers**, **FFI summaries**, **ghost memoization**, **risk-totals substitution**, and **fixture compression** that wrap those accessors. Status **partial**: storage-key integrity for verification is defended; transfer of “Certora green” to unmodeled dimensions (multi-hub, real pool math, prefetch, NFT contract, soft attrs default, direct-client index helper) is not. Severity **info** — epistemology / review hazard, not a deployable fund-safety defect.
