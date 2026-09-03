# A090 — `verified_hubs` memoization correctness

- Agent: A090
- Theme: T7 (also T6 check short-circuit / A099 adjacency)
- Severity: info
- Status: defended
- Paths:
  - `contracts/controller/src/context/mod.rs:25-37,48-61,76-83` (`Cache::verified_hubs`; `Cache::require_hub_active`)
  - `contracts/controller/src/config/spoke.rs:81-96` (`create_hub`; storage `require_hub_active`)
  - `contracts/controller/src/config/mod.rs:9` (re-export)
  - `contracts/controller/src/storage/hub.rs:11-18` (`get_hub` / `set_hub`)
  - `common/src/types/controller.rs:76-81` (`HubConfig { is_active }`)
  - `contracts/controller/src/positions/mod.rs:254-324` (`require_listed_unhalted_config` → `require_can_*` → `validate_position_entry_gates`)
  - `contracts/controller/src/positions/supply.rs:106-120` (`process_deposit` re-gates via Cache)
  - `contracts/controller/src/positions/debt.rs:248-265` (`borrow_into_controller` entry gates)
  - `contracts/controller/src/strategies/{flash_position,flash_loan,swap_debt,swap_collateral,repay_debt_with_collateral,migrate_blend,multiply}.rs` (config vs Cache hub gates)
  - `contracts/controller/src/markets.rs:68-74` (`create_liquidity_pool` config gate)
  - `contracts/controller/tests/governance/config.rs:26-66` (storage gate unit matrix)
  - `contracts/controller/tests/positions/flags.rs:403-433` (`require_can_supply_blocks_inactive_hub`)
- Defense: `verified_hubs` is a **per-invocation presence map** keyed by `hub_id`. `Cache::require_hub_active` inserts **only after** a successful `config::require_hub_active` storage read; a failed check panics with `#43 HubNotActive` and **never** records an entry. Repeat calls for the same id short-circuit on `contains_key` only. No production writer sets `HubConfig.is_active = false` (only `create_hub` → `true`; no public deactivator). Mid-invocation monetary reentry is blocked by the flash session flag (A007). Cache is stack-local to one entrypoint; constructors always start with an empty map.
- Gap: (1) **Latent sticky-success:** after a successful memo, later same-Cache calls do not re-read storage — including `process_deposit` / second `validate_position_entry_gates` passes that run **after** an untrusted flash callback while collateral hubs were memoized **before** the guard. Harmless today (no deactivator; admin cannot flip `is_active` false). Becomes a real kill-switch bypass if a future `set_hub_active(false)` ships without invalidating `verified_hubs` or re-checking after untrusted windows. (2) **Presence/`contains_key` footgun:** the stored `bool` is never read; short-circuit is “key exists,” not “value == true.” Recording `false` while keeping `contains_key` would silently skip the storage check — wrong by construction. (3) **Dual APIs:** many strategies call `config::require_hub_active` before `Cache::new`, so the early check does not populate the memo (extra storage read later; not a security hole). (4) **Test / Certora hole:** no unit asserts success-only insert, memo hit skipping storage, or post-callback re-gate behavior; no CVL mention of `verified_hubs` (A063 residual).
- Impact: **No fund-theft, share-mint, or undercollateralized exit** from this memo under the current call graph and hub writers. Failure cannot become sticky. Success-sticky stale “active” cannot diverge from storage without a same-invocation write of `Hub(hub_id)` to inactive — unreachable in production WASM today. Blast radius if a deactivator is added carelessly: one invocation could complete risk-increasing legs against a hub the operator just killed, until the next fresh Cache. Severity **info** (architecture defended; document + future checklist).
- Evidence: Exhaustive grep of `verified_hubs`, `Cache::require_hub_active`, `config::require_hub_active`, `set_hub` under `contracts/controller`; peers A063 (hub/spoke policy), A099 (opt-skip hunt), A007 (flash guard), A086/A104 (Wave-6 inventory); unit `require_hub_active_*` + `require_can_supply_blocks_inactive_hub`; SEED Cache facts; A104 marks A090 as hole previously covered only inside A099+A063.
- Opinion: **Confirm A099/A063/A104:** success-only hub memo is correct and intentional. Fill A090’s dedicated matrix without elevating severity. When shipping hub deactivation, treat `verified_hubs` like a security check short-circuit: either clear the key on deactivate-in-same-Cache (N/A if admin uses its own Cache), or re-run storage after every untrusted window before further `require_can_*`, or stop memoizing across those windows. Prefer documenting that the map is presence-only (always `true`) so a future “record failure” edit cannot land.

---

## Method

1. Read `shared/COORDINATION.md`, `SEED.md`, Wave-6 manifest (A090), peers **A063**, **A099**, **A086**, **A088**, **A104**, adjacency **A007**.
2. Read `context/mod.rs` memo primitive; `config/spoke.rs` + `storage/hub.rs` + `HubConfig`.
3. Enumerated **every** production `Cache::require_hub_active` and `config::require_hub_active` site under `contracts/controller/src`; ordered each against Cache construction, flash guards, and post-callback legs.
4. Enumerated every `set_hub` / `create_hub` writer; confirmed no public `is_active=false` path.
5. Traced multi-pass gates (`validate_collaterals` → callback → `process_deposit`) for sticky-success after untrusted code.
6. Checked tests and Certora for memo-specific coverage.
7. No production Rust edited. No git operations (COORDINATION).

No novel Critical/High. Agrees with A099/A063 defense ranking; supplies the dedicated same-tx reentrancy matrix A104 noted as missing for A090.

---

## 1. Memo primitive

### 1.1 Field and constructors

```25:61:contracts/controller/src/context/mod.rs
pub(crate) struct Cache {
    // ...
    verified_hubs: Map<u32, bool>,
    // ...
}

pub(crate) fn new_view(env: &Env) -> Self {
    Cache {
        // ...
        verified_hubs: Map::new(env),
        // ...
    }
}
```

| Property | Behavior |
|---|---|
| Lifetime | Per entrypoint invocation only (stack `Cache`); not durable storage |
| Init | Always empty (`Map::new`) in both `new` and `new_view` |
| Key | `u32` hub id |
| Value | `bool`, but **only `true` is ever written** |
| Cross-invocation | Impossible — new Cache each call |
| Invalidate API | **None** (no `clear_verified_hub`, no spoke-style reset) |

`Cache::new` renews instance TTL then delegates to `new_view` (A093/A034 adjacency). Hub memo does not interact with TTL.

### 1.2 Success-only insert

```76:83:contracts/controller/src/context/mod.rs
pub(crate) fn require_hub_active(&mut self, hub_id: u32) {
    if self.verified_hubs.contains_key(hub_id) {
        return;
    }
    require_hub_active(&self.env, hub_id);
    self.verified_hubs.set(hub_id, true);
}
```

| Step | Outcome |
|---|---|
| Hit (`contains_key`) | Return; **no** storage read; **no** value inspection |
| Miss → storage active | Insert `(hub_id, true)`; return |
| Miss → missing / `is_active=false` | `assert_with_error!(…, HubNotActive)` → panic **before** `set` |

Therefore:

- A failed check **cannot** poison later calls for the same id (panic aborts the invocation; even if it did not, no key would exist).
- A successful check **can** suppress later storage reads for that id for the rest of this Cache’s life.

Underlying storage check:

```93:96:contracts/controller/src/config/spoke.rs
pub(crate) fn require_hub_active(env: &Env, hub_id: u32) {
    let active = storage::get_hub(env, hub_id).is_some_and(|hub| hub.is_active);
    assert_with_error!(env, active, GenericError::HubNotActive);
}
```

Missing key and `is_active == false` share `#43`. `create_hub` always writes `HubConfig { is_active: true }`.

### 1.3 Contrast with sibling memos

| Field | Insert rule | Stale-success risk class |
|---|---|---|
| `token_prices` | Fill-once (ADR-0005) | Intentional freeze |
| `market_indexes` | Simulate + **overwrite** after legs | Footgun if overwrite forgotten (A094) |
| `pool_sync_data` | Fill-once, no invalidate | Latent if post-leg reader appears (A088) |
| `spoke_*` | Pin / `reset_spoke_context` | Spoke-scoped |
| **`verified_hubs`** | **Success-only presence** | Latent only if hub row flips inactive mid-Cache |

---

## 2. Who can change hub activity (writer inventory)

| Writer | Path | Effect on `is_active` |
|---|---|---|
| `create_hub` (owner entrypoint → `config::spoke::create_hub`) | `set_hub(…, { is_active: true })` | New id **true** only |
| Unit / harness `storage::set_hub(…, false)` | tests only | Latent bit exercised in tests |
| Public `set_hub_active` / `deactivate_hub` | **Does not exist** | — |
| Upgrade / direct storage | out-of-band / owner trust | Threat-model Sensitive root (A009) |

Production `set_hub` call sites under `contracts/controller/src`: **only** `create_hub`. Position-manager `is_active` is a different type/key (`PositionManagerConfig`) and does not touch `ControllerKey::Hub`.

**Implication for memo correctness:** under shipped WASM, once a hub id is successfully verified, storage for that id cannot become inactive later in the same (or any) invocation without an upgrade or a test harness poke. Sticky success cannot diverge from live storage today.

---

## 3. Call-site inventory

### 3.1 Paths that use `Cache::require_hub_active` (memoized)

Sole definition call into the Cache method:

```260:270:contracts/controller/src/positions/mod.rs
fn require_listed_unhalted_config(...) -> AssetConfig {
    cache.require_hub_active(hub_asset.hub_id);
    let asset_config = cache.require_listed_active_config(spoke_id, hub_asset);
    enforce_spoke_asset_flags(..., FreezePolicy::BlockOnEntry);
    asset_config
}
```

Consumed by:

| Helper | Used by (non-exhaustive of money paths) |
|---|---|
| `require_can_supply` | `validate_position_entry_gates(Deposit)`; multiply / swap_collateral / migrate / flash_position collateral validation; `process_deposit` |
| `require_can_borrow` | `validate_position_entry_gates(Borrow)`; `borrow_into_controller` (strategies) |

`validate_position_entry_gates` loops aggregated payments and calls `require_can_*` per hub asset — **primary consumer of the memo** when the same hub appears twice (bulk supply/borrow, or double validation).

Withdraw / repay / liquidate use `enforce_spoke_asset_flags` **without** hub-active (A063 intentional exit liveness). Those paths never touch `verified_hubs`.

Views build `Cache::new_view` but do not call `require_hub_active` — map stays empty.

### 3.2 Paths that use `config::require_hub_active` (no memo)

| Site | Hub(s) | Cache relation |
|---|---|---|
| `flash_loan` | flash hub | Before `Cache::new` |
| `flash_position` | debt hub | Before `Cache::new`; debt later re-checked via `borrow_into_controller` → Cache |
| `swap_debt` | **existing** debt hub | Before Cache; **new** debt gated via Cache in borrow helper |
| `swap_collateral` | **source** hub | Before Cache; **dest** via `require_can_supply` → Cache |
| `repay_debt_with_collateral` | collateral + debt | Before Cache (both) |
| `migrate_blend` | request hub | Before Cache; per-asset `require_can_supply` may re-read same id via Cache |
| `create_liquidity_pool` | market hub | No user Cache memo involvement |

Early config checks are **fail-closed** and independent of `verified_hubs`. They do **not** insert into the map. Same hub id later on Cache pays a second storage read once, then memos.

### 3.3 Intentional double validation (memo earns its keep)

`flash_position::validate_collaterals`:

1. Per collateral: `require_can_supply` → Cache miss → storage → insert.
2. Then `validate_position_entry_gates(Deposit)` → `require_can_supply` again → **memo hit**.

`process_supply` / strategy deposits: `process_deposit` calls `validate_position_entry_gates` again; if hubs were already verified on the same Cache, hits memo.

This is the optimization A099 described: skip repeat **successful** checks, never skip a failed one.

---

## 4. Same-invocation staleness / reentrancy matrix

Hypothesis under test: after `verified_hubs.set(id, true)`, storage flips to inactive; a later `Cache::require_hub_active(id)` wrongly allows risk-increasing work.

### 4.1 Can storage flip inactive mid-Cache?

| Mechanism | Reachable in production? | Notes |
|---|---|---|
| Public deactivate | No | No entrypoint |
| `create_hub` | Only sets **true** on a **new** id | Cannot flip an already-memoized id to false |
| Cross-contract write of controller persistent Hub key | No | Only this contract’s storage API |
| Reenter monetary entrypoint to mutate hub | No | `require_not_flash_loaning` / `require_authorized_caller` (A007) |
| Reenter owner admin during flash callback | Admin **does not** check flash flag (A007 intentional) | Still no deactivate; `create_hub` only adds active hubs |
| Listed-token hook after guard clears | Residual A007 | Still no hub writer |

**Verdict:** sticky-success divergence is **unreachable** on current WASM.

### 4.2 Post-untrusted-window re-gate (latent design)

`flash_position` ordering (simplified):

1. `config::require_hub_active(debt)` (no Cache).
2. `Cache::new`.
3. `validate_collaterals` → memoizes **collateral** hub ids (and double-gates via `validate_position_entry_gates`).
4. `with_flash_guard` → `mint_and_forward` → `borrow_into_controller` → Cache-memos **debt** hub → pool + **receiver callback**.
5. After guard: `process_deposit` → `validate_position_entry_gates` → `require_can_supply` → **memo hit** for collateral hubs (no storage re-read).

So hub-active for collaterals is **not** re-fetched after the callback. That matches price/spoke memo philosophy for the invocation. With a future deactivate-in-callback (owner receiver + new admin API), step 5 could admit supply against a just-killed hub for that one flash completion.

Mitigations already present for money safety: flash flag, measured deposits, `require_flash_position_still_open`, listing flags (`paused`/`frozen`) which live in spoke-asset memo (A089) — separate from hub memo. Hub kill is not the primary halt today (A063: spoke deprecation + asset flags).

### 4.3 Multi-hub isolation

Keys are per `hub_id`. Verifying hub `1` never marks hub `2`. Bulk payments across hubs miss independently. Same hub with two assets (two `HubAssetKey`s, shared `hub_id`) correctly shares one memo entry — desired.

### 4.4 Config-then-Cache same id

Example: `migrate_blend` / `flash_position` debt path.

| Time | Check | Memo effect |
|---|---|---|
| T0 | `config::require_hub_active(H)` | None |
| T1 | `Cache::require_hub_active(H)` | Storage read again; insert |
| T2+ | `Cache::require_hub_active(H)` | Hit |

Redundant read only; never skips an unchecked hub.

### 4.5 Failure-then-retry within one Cache

Not observable in one Soroban invocation: first failure panics the call. Across invocations, each `Cache::new*` starts empty — a previously failed hub is fully re-checked. **No sticky fail.**

---

## 5. `contains_key` / `bool` API footgun

The map type is `Map<u32, bool>`, but the gate is:

```rust
if self.verified_hubs.contains_key(hub_id) { return; }
```

not

```rust
if self.verified_hubs.get(hub_id) == Some(true) { return; }
```

| Future edit | Result |
|---|---|
| `set(id, false)` to “remember failure” then continue | **Critical incorrect skip** of storage check |
| `set(id, true)` only on success (today) | Correct |
| Switch to a set / unit-valued map | Clearer intent |

**Recommendation (docs / hygiene, not a live bug):** comment at the field that presence means “verified active”; value is vestigial always-true; never insert false. Optional harden: `get(hub_id).unwrap_or(false)` or drop the bool.

This is the main **engineering** residual unique to A090 vs A099’s one-liner.

---

## 6. Interaction with entry/exit policy (A063)

Memo correctness is orthogonal to **where** hub-active is required:

| Verb class | Hub-active? | Uses `verified_hubs`? |
|---|---|---|
| Supply / borrow / strategy entry / flash open | Yes | Yes (via Cache) or config pre-check |
| Withdraw / repay / liquidate | No (by design) | No |
| Keepers | No | No |
| `add_asset_to_spoke` | No hub check | No |

A memo bug could only weaken checks on paths that already call `Cache::require_hub_active`. It cannot open withdraw/repay. Conversely, fixing memo would not add hub checks to exits — and must not (INV-LIQ-01 / A063).

---

## 7. Tests and formal coverage

| Coverage | Status |
|---|---|
| Storage `require_hub_active` unknown / zero / deactivated | Unit `governance/config.rs` |
| `require_can_supply` + inactive/missing hub | `flags.rs::require_can_supply_blocks_inactive_hub` |
| Success-only insert / no false sticky | **Missing** dedicated assert |
| Second call skips storage (behavioral) | **Missing** (would need storage spy or counter) |
| Post-flash `process_deposit` uses memo | **Missing** explicit matrix |
| Certora `verified_hubs` / hub memo | **None** found under `certora/` |

A063 already noted optional CVL for entry vs exit hub assumptions. A090 adds: if a rule models Cache, it must encode **success-only** insert (failure not in map).

---

## 8. Threat scenarios

| Scenario | Possible today? | Outcome |
|---|---|---|
| Failed hub check memoized; later bypass | No | Panic before insert |
| Successful hub memo; second asset same hub skips storage | Yes (intended) | Still active in storage |
| Callback deactivates hub; post-callback deposit skips re-check | Deactivate unreachable | Latent if API added |
| Attacker forces sticky fail across txs | No | Fresh Cache each call |
| `contains_key` after `set(..., false)` | Not in code | Would be Critical if introduced |
| View Cache leaks verified hubs into mutator | No | Separate values |
| Wrong hub allowed because another hub verified | No | Keyed by id |

**Single-account max (current code):** none from this memo.

**Single-account max (future misuse of sticky success + deactivate):** one invocation completes entry/deposit legs operator intended to kill; next invocation fails closed.

**Protocol SoT:** hub rows unchanged by the memo; next tx always re-reads storage on first miss.

---

## 9. Cross-links

| Peer | Relation |
|---|---|
| **A063** | Policy: when hub-active is required; documents success-only memo in passing |
| **A099** | Opt-skip hunt: hub memo is the exemplar of safe short-circuit |
| **A007** | Bounds mid-callback monetary reentry that could otherwise race admin |
| **A086 / A104** | Inventory; A104 marked A090 as unfilled hole covered only by A099+A063 |
| **A088** | Sibling fill-once correctness pattern (pool address / sync) |
| **A064** | Entry flag stack sits **after** hub-active in `require_listed_unhalted_config` |
| **A047–A050** | Strategy early `config::require_hub_active` vs Cache entry gates |

No disagreement file needed: aligned with A099/A063/A104 on **defended / info**.

---

## 10. Remediation notes (for A110; docs-only unless call graph changes)

| P | Action | Closes |
|---|---|---|
| P3 | Document on `verified_hubs` / `Cache::require_hub_active`: success-only; presence == active; never insert false; no invalidate API because no mid-tx deactivate | Docs gap A086/A090 |
| P3 | When adding `set_hub_active(false)`: checklist — admin uses own Cache; any shared Cache must `remove` the hub key or re-check after untrusted windows before further `require_can_*` | Future kill-switch |
| P4 | Optional unit: two `Cache::require_hub_active` calls; poke storage to false between them only if testing sticky behavior is desired as a **negative** harness under `#[cfg(test)]` storage poke — document expected sticky allow to lock the contract | Test hole |
| P4 | Optional harden: gate on `get == Some(true)` or unit map | Footgun |
| Anti-fix | Do not add hub-active to withdraw/repay/liquidate to “fix” memo fears | A063 / INV-LIQ-01 |
| Anti-fix | Do not memoize failures | Would create sticky deny across… (still one tx) or confuse contains_key | 

---

## 11. Verdict

1. **`verified_hubs` memoization is correct for shipped code** — success-only insert, fail-closed storage check, per-invocation empty map, per-hub keys, no production inactive writer.
2. **The optimization does not skip a security check that has not already passed** in this invocation; A099’s claim holds under full call-graph review.
3. **Residuals are documentation / future-API / API-shape footguns**, not live fund-theft: sticky success across untrusted windows if deactivate ships; `contains_key` ignoring bool; thin tests/Certora.
4. **Severity info, status defended.** Fills A104’s A090 hole with an explicit same-tx matrix; does not change Wave-7 ranking versus A094 index overwrite or A080 exit no-op.

**Final judgment:** Treat `verified_hubs` as a defended, intentional short-circuit of a read-only existence/liveness bit. Keep success-only. Re-audit this memo the day hub deactivation becomes an on-chain verb.
