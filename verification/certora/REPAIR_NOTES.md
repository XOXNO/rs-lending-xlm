# Certora Spec/Harness Repair — Triage Notes (CR-1)

Read-only error map for CR-2..CR-8. Produced against `feat/governance-split`
at commit `6d1c1a9` (working tree dirty: only `Makefile`, `configs/networks.json`,
`configs/script.sh` modified — unrelated to the certora repair).

Build path used to catalogue: per-crate host `cargo check` (no `--all-features`).

---

## 1. Authoritative gate commands

The certora **compile gate** is `verification/certora/compile_all.sh`:

```
cargo check -p common     --features certora
cargo check -p pool       --features certora --no-default-features
cargo check -p controller --features certora --no-default-features
python3 verification/certora/check_orphans.py
python3 verification/certora/check_invariant_coverage.py
python3 verification/certora/scripts/sync_wasm_conf.py
# --wasm: make certora-wasm + scripts/check_wasm_artifacts.py
```

**What CI actually gates on** (`.github/workflows/certora-verification.yml`):

- Job `compile-check` runs `./verification/certora/compile_all.sh` **without
  `--wasm`** on toolchain 1.95 + `wasm32v1-none` target. → The three host
  `cargo check`s + the two python coverage scripts + `sync_wasm_conf.py` ARE
  the required build half. This is the gate CR-2..CR-6 must turn green.
- Job `sanity` (needs `compile-check`) installs stellar-cli, runs
  `make certora-wasm`, then `certoraSorobanProver` with `CERTORAKEY`. The WASM
  build + hosted prover live here — **Phase 4**, out of scope for the compile
  repair.

### Host-check vs WASM "1 vs ~87" disagreement — RESOLVED

The certora harness is wired via `#[cfg(feature = "certora")] #[path = ...]`
module swaps in `contracts/controller/src/lib.rs` (`mod spec`) and
`contracts/controller/src/external/mod.rs` (`pool`, `sac`). Those swaps replace
the **production** `pool_calls`/`oracle`/`storage` modules with harness stubs
**only when default features are off**. A plain `cargo check -p controller
--features certora` (defaults ON) links the production modules and the certora
path is dead, so it reports ~1 error. With `--no-default-features` the swap
activates and the full set surfaces.

**Authoritative gate = `cargo check -p controller --features certora
--no-default-features`** (exactly what `compile_all.sh` / the CI compile job
run). It reproduces the entire error set on the host; `make certora-wasm` adds
no *additional* compile errors for this repair (it's the same crates + feature,
different target + the stellar optimizer). Measured controller error count =
**91** (plan estimated ~87 — close; the delta is the production-callsite E0061s
the plan's wasm count rolled together).

---

## 2. Per-crate status

| Crate | `cargo check ... --features certora [--no-default-features]` | Errors |
|-------|-------------------------------------------------------------|--------|
| `common` | **GREEN** (exit 0) | 0 |
| `pool` | **GREEN** (exit 0) | 0 |
| `controller` | **FAIL** | **91** (+1 warning) |

> Correction to the plan: `common` and `pool` already compile clean under the
> certora feature on this branch. CR-3 (common) and CR-4 (pool) have **no compile
> work** — the asset-keyed / `pool_address` damage the plan predicted for `pool`
> did not land in the pool spec files (pool specs don't import controller types
> and don't reach through `MarketConfig`). Verify before editing; do not
> "fix" already-green crates. The entire compile repair is in `controller`.

Error-code histogram (controller, 91 total):

| Code | Count | Meaning |
|------|-------|---------|
| E0433 | 51 | `cannot find module or crate controller` — `use controller::…` inside the `controller` crate |
| E0061 | 28 | arg-count mismatch (stale summary sigs vs asset-keyed/batched production callsites) |
| E0609 | 4 | `no field pool_address on MarketConfig` |
| E0599 | 4 | `no method get on PoolPositionMutation` (return shape changed Vec→single) |
| E0432 | 2 | `pool_create_market_call` not in `external::pool`; `common::types::PriceFeedRaw` moved |
| E0308 | 2 | return-type mismatch `Vec<PoolPositionMutation>` vs `PoolPositionMutation` |

---

## 3. The compat/harness cascade — DOES IT HOLD?

**Partly. The 51 path errors are NOT a single-point cascade — they're a
flat sweep, but each is a one-line MECH repoint, so they clear together.**

### 3a. The 51× `cannot find module or crate controller` (E0433)

`controller` is the crate's *own* name. Inside the crate the path must be
`crate::`, not `controller::`. Every spec/harness file opens with
`use controller::types::…` / `controller::constants::…` (a leftover from when
these were a separate crate). The fix is uniform:

```
controller::types::X      → crate::types::X
controller::constants::Y  → crate::constants::Y   (or common::constants::Y)
```

This is sound because `contracts/controller/src/lib.rs` re-exports:
- `pub use controller_interface::types;`  (lib.rs:10) — and
  `controller_interface::types` itself does `pub use common::types::*; pub use
  controller::*; pub use oracle::*;` (interfaces/controller/src/types/mod.rs:4-7).
  → **Every** spec-referenced type (`Payment`, `PositionMode`, `StrategySwap`,
  `MarketParams`, `AccountPositionType`, `MarketIndex`, `InterestRateModel`,
  `MarketParamsRaw`, `AccountPositionRaw`, `AssetConfig`, `OraclePriceFluctuation`,
  `PriceFeedRaw`, `MarketStatus`, `EModeAssetConfig`, `AccountAttributes`) is
  reachable at `crate::types::*`. No type was actually deleted by the crate move.
- `pub mod constants;` (lib.rs:7) where `constants.rs` does
  `pub use common::constants::*;` + `BAD_DEBT_USD_THRESHOLD`. → all of
  `WAD, RAY, BPS, MILLISECONDS_PER_YEAR, SUPPLY_INDEX_FLOOR_RAW,
  MAX_BORROW_RATE_RAY, MIN_DUST_FLOOR_WAD, MAX_FIRST_TOLERANCE` (etc.) resolve
  at `crate::constants::*`. (Tolerance consts live in
  `common/src/constants/shared.rs`.)

These 51 are spread across `compat.rs` (1), every `*_rules.rs` (most), and the
harness files (`external/pool.rs`, `oracle_price.rs`, `oracle_tolerance.rs`,
`storage.rs`). Fixing `compat.rs` alone does NOT clear the others — each file
imports `controller::` independently. But all 51 are the same trivial edit.
**Recommend a single sweep:** `controller::types::` → `crate::types::`,
`controller::constants::` → `crate::constants::` across
`verification/certora/controller/{spec,harness}/**`. ~1 file-wide find/replace.

### 3b. The E0432 `common::types::PriceFeedRaw` (summaries/mod.rs:11)

`PriceFeedRaw` moved `common::types` → `controller_interface::types::oracle`
(interfaces/controller/src/types/oracle.rs:248). From the controller crate use
`crate::types::PriceFeedRaw`. MECH.

### 3c. The 28 E0061 + 4 E0599 + 2 E0308 — the REAL structural break

`external/pool.rs` (harness) re-exports the summaries in
`shared/summaries/pool.rs` under the production wrapper names
(`supply_summary as pool_supply_call`, etc.). **The summaries still have the
pre-single-pool, per-call signatures; the production callsites now batch
entries and take an `asset`.** Concretely the summaries are stale vs the live
ABI (`contracts/controller/src/external/pool.rs` + `interfaces/pool/src/lib.rs`):

| Production wrapper (current) | Summary (`shared/summaries/pool.rs`, stale) | Gap |
|---|---|---|
| `pool_supply_call(env, pool_addr, entries: &Vec<PoolSupplyEntry>) -> Vec<PoolPositionMutation>` | `supply_summary(env, pool_addr, position, amount, supply_cap) -> PoolPositionMutation` | batched Vec; returns Vec |
| `pool_borrow_call(env, pool_addr, receiver, entries: &Vec<PoolBorrowEntry>) -> Vec<…>` | `borrow_summary(env, pool_addr, caller, amount, position, borrow_cap) -> single` | batched Vec; returns Vec |
| `pool_repay_call(env, pool_addr, payer, actions: &Vec<PoolAction>) -> Vec<…>` | `repay_summary(env, pool_addr, caller, amount, position) -> single` | batched Vec; returns Vec |
| `pool_withdraw_call(env, pool_addr, receiver, is_liquidation, entries: &Vec<PoolWithdrawEntry>) -> Vec<…>` | `withdraw_summary(env, pool_addr, caller, amount, position, is_liquidation, protocol_fee) -> single` | batched Vec; returns Vec |
| `pool_create_strategy_call(env, pool_addr, receiver, action: PoolAction, fee, borrow_cap) -> PoolStrategyMutation` | `create_strategy_summary(env, pool_addr, caller, position, amount, fee, borrow_cap)` | takes `PoolAction`, not `position+amount` |
| `pool_seize_position_call(env, pool_addr, asset, side, position)` | `seize_position_summary(env, pool_addr, side, position)` | missing `asset` |
| `pool_flash_loan_call(env, pool_addr, asset, initiator, receiver, amount, fee, data)` | `flash_loan_summary(env, pool_addr, initiator, receiver, amount, fee, data)` | missing `asset` |
| `pool_update_indexes_call(env, pool_addr, asset) -> MarketStateSnapshot` | `update_indexes_summary(env, pool_addr)` | missing `asset` |
| `pool_claim_revenue_call(env, pool_addr, asset) -> PoolAmountMutation` | `claim_revenue_summary(env, pool_addr)` | missing `asset` |
| `pool_add_rewards_call(env, pool_addr, asset, amount) -> MarketStateSnapshot` | `add_rewards_summary(env, pool_addr, amount)` | missing `asset` |
| `fetch_pool_sync_data(env, pool_addr, asset) -> PoolSyncData` | `get_sync_data_summary(env, pool_addr)` | missing `asset` |
| `pool_update_params_call(env, pool_addr, asset, params)` (harness pool.rs:28) | only `(env, pool_addr, params)` | missing `asset` |
| `pool_create_market_call` (router.rs:18 import) | absent from harness — only `create_strategy` re-exported | E0432: add `create_market` void stub |
| `LiquidityPoolClient::*().get_sync_data()` (storage.rs) | `interfaces/pool/src/lib.rs:81 get_sync_data(env, asset)` | method now needs `asset` |
| `reserves()/supplied_amount()/borrowed_amount()/capital_utilisation()` (solvency_rules.rs) | now `(env, asset)` (interfaces/pool/src/lib.rs:73-79) | method now needs `asset` |

These errors land in **production source files** (router.rs, positions/*.rs,
cache/mod.rs, strategies/flash_loan.rs) — NOT the spec files — because production
calls the harness-swapped stubs. The fix is to **rewrite the summaries +
harness `external/pool.rs` to the current batched/asset-keyed ABI** so they
match the production callsites. That is the bulk of CR-2 and is MECH (follow the
production wrapper signatures verbatim) but it is real signature surgery, not a
path repoint. The `PoolPositionMutation::get` (E0599) + `Vec` return (E0308)
errors all dissolve once the summaries return `Vec<PoolPositionMutation>` like
production.

**Cascade verdict: NO single fix clears 51. YES — CR-2 (fix
`shared/summaries/pool.rs` + `harness/external/pool.rs` + `harness/storage.rs`)
+ the global `controller::`→`crate::` sweep together clear ~85 of 91. The
residual ~6 are the `pool_address`/`get_asset_pool` items in §4.**

---

## 4. Per-crate error table

`L:C` = line:col in the source file. Target = current symbol to repoint to.
Fix = MECH (mechanical, follow current ABI) / SEM (needs a decision).

### controller — group A: `controller::`→`crate::` path sweep (51 × E0433) — ALL MECH

All in `verification/certora/controller/`. Same edit everywhere.

| File | Lines | Stale | Target |
|------|-------|-------|--------|
| `spec/compat.rs` | 1 | `controller::types::{Payment,PositionMode,StrategySwap}` | `crate::types::…` |
| `harness/external/pool.rs` | 23 | `controller::types::InterestRateModel` | `crate::types::InterestRateModel` |
| `harness/oracle_price.rs` | 8 | `controller::types::{MarketIndex,PriceFeedRaw}` | `crate::types::…` |
| `harness/oracle_tolerance.rs` | 8,13 | `controller::constants::{BPS,MAX_FIRST_TOLERANCE,…}`, `controller::types::OraclePriceFluctuation` | `crate::constants::…`, `crate::types::…` |
| `harness/storage.rs` | 7 | `controller::types::{AccountAttributes,…,PositionMode}` | `crate::types::…` |
| `spec/account_isolation_rules.rs` | 11,12 | `controller::constants::WAD`, `controller::types::AccountPositionType` | `crate::…` |
| `spec/boundary_rules.rs` | 17,21 | `controller::constants::{…}`, `controller::types::MarketParams` | `crate::…` |
| `spec/consistency_rules.rs` | 11,12 | constants::WAD, types::AccountPositionType | `crate::…` |
| `spec/health_rules.rs` | 16 | constants::WAD | `crate::…` |
| `spec/index_rules.rs` | 10 | constants::{RAY,SUPPLY_INDEX_FLOOR_RAW} | `crate::…` |
| `spec/interest_rules.rs` | 17,24 | constants::{…}, types::MarketParams | `crate::…` |
| `spec/isolation_rules.rs` | 10,66,75,158 | constants::{BPS,RAY}, types::AccountPositionType (×3 inline) | `crate::…` |
| `spec/liquidation_rules.rs` | 15,18 | constants::{BPS,WAD}, types::AccountPositionType | `crate::…` |
| `spec/market_guard_rules.rs` | 11,12 | constants::WAD, types::{AccountPositionType,MarketStatus} | `crate::…` |
| `spec/math_rules.rs` | 10 | constants::{RAY,WAD} | `crate::…` |
| `spec/oracle_compose_rules.rs` | 12 | constants::WAD | `crate::…` |
| `spec/oracle_rules.rs` | 21,24,64,65 | constants::{…}, types::{…}, inline `controller::constants::MIN_DUST_FLOOR_WAD` (×2) | `crate::…` |
| `spec/position_rules.rs` | 11,19,46,87,105,131 | types::AccountPositionType + inline `controller::constants::WAD` (×5) | `crate::…` |
| `spec/solvency_rules.rs` | 10,319,360,578 | constants::{…} + inline types::AccountPositionType (×3) | `crate::…` |
| `spec/strategy_rules.rs` | 17,18 | constants::BAD_DEBT_USD_THRESHOLD, types::{AccountPositionType,StrategySwap} | `crate::…` |
| `spec/tolerance_math_rules.rs` | 10,14 | constants::{BPS,RAY,WAD}, types::OraclePriceFluctuation | `crate::…` |
| `spec/emode_rules.rs` | 245,264,301,432,493 | inline `controller::types::{AccountPositionType,AssetConfig,StrategySwap}` | `crate::types::…` |
| `spec/flash_loan_rules.rs` | 70,106 | inline `controller::types::MarketStatus::Active` | `crate::types::…` |

### controller — group B: stale summary/harness signatures (28 E0061 + 4 E0599 + 2 E0308 + 1 E0432) — MECH (signature surgery in summaries/harness)

Source of truth = `contracts/controller/src/external/pool.rs` (production
wrappers) + `interfaces/pool/src/lib.rs` (client methods). All these errors are
emitted at production callsites but are caused by stale stubs in the certora
harness. Fix the stubs, not production.

| Error site (production) | L | Code | Root cause (stub to fix) |
|---|---|---|---|
| `router.rs` (import) | 18 | E0432 | `pool_create_market_call` missing from `harness/external/pool.rs` — add void stub |
| `cache/mod.rs` | 281 | E0061 | `get_sync_data_summary` needs `asset` arg |
| `positions/borrow.rs` | 108,111,215 | E0061×2,E0599 | `borrow_summary`/`create_strategy_summary` batched-Vec ABI |
| `positions/repay.rs` | 126,128,131 | E0061,E0599,E0308 | `repay_summary` → batched `Vec` |
| `positions/supply.rs` | 164,167 | E0061,E0599 | `supply_summary` → batched `Vec` |
| `positions/withdraw.rs` | 154,166,177 | E0061,E0599,E0308 | `withdraw_summary` → batched `Vec` |
| `positions/liquidation.rs` | 406 | E0061 | `seize_position_summary` needs `asset` |
| `strategies/flash_loan.rs` | 62 | E0061 | `flash_loan_summary` needs `asset` |
| `router.rs` | 157,225 | E0061×2 | `update_indexes_summary` needs `asset` |
| `router.rs` | 229 | E0061 | `pool_update_params_call` (harness) needs `asset` |
| `router.rs` | 254 | E0061 | `claim_revenue_summary` needs `asset` |
| `router.rs` | 299 | E0061 | `add_rewards_summary` needs `asset` |
| `spec/compat.rs` | 27 | E0061 | `Controller::withdraw` gained `to: Option<Address>` 5th arg — pass `None` |
| `spec/emode_rules.rs` | 256 | E0061 | `process_withdraw` gained `to: Option<Address>` — pass `None` |
| `spec/solvency_rules.rs` | 26,48,51,87,160,165,183,187,708,726 | E0061×10 | `pool_client.{reserves,get_sync_data,capital_utilisation,supplied_amount,borrowed_amount}()` now take `asset` (interfaces/pool/src/lib.rs:73-81) — pass `&asset` |

> `compat.rs:27` and `emode_rules.rs:256`: the public `withdraw` /
> `process_withdraw` signatures gained `to: Option<Address>` (the
> withdraw-to-recipient feature). Passing `None` preserves the existing rule
> intent (withdraw to self). MECH, but note it leaves the `Some(recipient)`
> branch unverified — see SEM-3.

### controller — group C: `MarketConfig.pool_address` removed (4 E0609 + downstream) — see SEM-1

| File | L | Code |
|---|---|---|
| `harness/storage.rs` | 76 | E0609 — `asset_pool::get_asset_pool` returns `…pool_address` |
| `harness/storage.rs` | 106,135,150 | E0609 — `market.pool_address` in `get_asset_config`/`get_market_index`/`get_market_params` |
| `harness/storage.rs` | 106,136,151 | E0061 — same lines, `get_sync_data()` then also needs `asset` |

### common / pool — NO ERRORS

Both green. CR-3 and CR-4 are no-ops for the compile. (If `--wasm` later
surfaces a `common`/`pool` issue it is the cvlr/sdk-26 vendoring path, not the
specs — per the plan's self-review note.)

---

## 5. SEM DECISIONS

The triage found **far less semantic damage than the plan predicted**. The
"governance split" did NOT delete config logic from the controller: the
controller still has an internal `mod governance` (`lib.rs:15`) whose
`governance::config::*` functions hold the real admin logic
(`contracts/controller/src/governance/config.rs`); the new separate
`contracts/governance/` contract is a thin *forwarder* on top. So spec rules
that call `crate::governance::config::add_asset_to_e_mode_category`,
`crate::emode::ensure_e_mode_compatible_with_asset`, etc. **still resolve** —
they are MECH path-repoints, not orphaned coverage. **No admin/config rule needs
deletion.** The orphan + invariant-coverage gates already pass (35 confs, 231
rules, zero orphans; 12 invariant modules covered) and will stay green because
no rule is being removed.

The genuine SEM items:

### SEM-1 — `MarketConfig.pool_address` removed → single central pool (RECOMMEND: repoint, MECH-adjacent)

`MarketConfig` is now `{ status, asset_config, oracle_config }` — no per-market
pool. Production resolves the (single) pool via `storage::get_pool(env)`
(instance key, `storage/instance.rs:86`) or `cache.cached_pool_address()`.

Affected: `harness/storage.rs` `asset_pool::get_asset_pool`,
`asset_config::get_asset_config`, `market_index::get_market_index`,
`market_params::get_market_params` (all read `market.pool_address`); and the 7
`solvency_rules.rs` callers of `get_asset_pool`.

- **Option (a) — repoint (RECOMMENDED):** make `get_asset_pool(env, _asset)`
  return `crate::storage::get_pool(env)` (drop the per-asset dimension; keep the
  `_asset` param so the 7 callers stay unchanged). Replace the three
  `LiquidityPoolClient::new(env, &market.pool_address)` sites in storage.rs with
  `…new(env, &get_pool(env))` and add the `&asset` arg to `get_sync_data`. The
  invariant the rules express ("after op on `asset`, the pool views for `asset`
  are consistent") is *preserved* under the central pool — the pool is just
  shared, keyed by `asset` at the view level. This keeps all 7 solvency rules.
- Option (b) — delete: unnecessary; the invariant still holds. Do not delete.

This is the only place `pool_address` removal bites. Classify the harness edits
as **MECH** (clear target `storage::get_pool` + asset-keyed views), but flag for
owner awareness because it silently changes "per-market pool" semantics to
"shared pool, asset-keyed view" — confirm that matches the intended model
(memory note `single-central-pool-migration` says yes: controller +
central pool live on testnet).

### SEM-2 — `seize_position` supply-index drop & summary soundness (RECOMMEND: preserve, re-verify in Phase 4)

When rewriting `seize_position_summary` to the asset-keyed ABI, the existing
postcondition ("supply index MAY drop on the bad-debt branch, floored at
`SUPPLY_INDEX_FLOOR_RAW`; all other paths monotone") must be carried over
verbatim. This is a soundness property, not a signature — do not drop it during
the mechanical sig rewrite. No decision needed now; flagged so CR-2 doesn't lose
it. (Memory: `isolated-debt-counter-asymmetric-drift` FIXED 2026-06-12 — confirm
the summary's monotonicity assumptions still match the new exact-basis model
when the prover runs in Phase 4.)

### SEM-3 — `withdraw`/`process_withdraw` gained `to: Option<Address>` (RECOMMEND: pass None now, add a branch rule in Phase 5)

`compat.rs:27` and `emode_rules.rs:256` pass `None` (withdraw-to-self) to fix
the arg-count. That preserves existing coverage but leaves the new
`Some(recipient)` withdraw-to-recipient branch unverified. **Decision:** pass
`None` for the compile repair (MECH); **record a Phase-5 coverage gap** to add a
`withdraw_to_recipient` rule. Not an orphan (no conf references it yet).

### SEM-4 — batched-entry summaries lose per-entry granularity (RECOMMEND: model single-entry Vec)

Production `supply/borrow/repay/withdraw` now take `Vec<PoolEntry>` and return
`Vec<PoolPositionMutation>`. The cheapest sound summary returns a `Vec` of the
same length as the input with each element bounded as the old single-position
summary did. **Decision for CR-2:** model the Vec element-wise (length-preserving,
each element nondet-bounded per the existing per-op postconditions). This keeps
every downstream rule that indexes `results.get(i)` working. Pure modelling
choice; recommended shape stated — no owner sign-off required, but noted because
it is a judgement about how tightly to bound the batch.

---

## 6. Effort estimate

| Crate / step | Work | Effort |
|---|---|---|
| `common` (CR-3) | none — already green | 0 |
| `pool` (CR-4) | none — already green | 0 |
| controller group A (51 E0433) | global `controller::`→`crate::` sweep across `verification/certora/controller/{spec,harness}` + `common::types::PriceFeedRaw`→`crate::types` | ~30 min (one find/replace + spot-check) |
| controller group B (35 errors) | rewrite `shared/summaries/pool.rs` (11 summaries) + `harness/external/pool.rs` (add `create_market` stub, asset-thread `update_params`) to batched/asset-keyed ABI; thread `&asset` through 10 solvency view calls + 2 `to:None` | ~2–3 h (real signature surgery + keep postconditions) |
| controller group C / SEM-1 (6 errors) | repoint `harness/storage.rs` `pool_address`→`storage::get_pool`, asset-thread `get_sync_data` | ~30 min |
| CR-6 orphan/coverage | **already green; nothing to do** unless a rule is deleted (none planned) | 0 |
| CR-7 `make certora-wasm` | needs stellar-cli; build the 3 crates `--optimize=false` for wasm32v1-none; check artifacts | ~30 min + build time (env-dependent) |
| CR-8 CI confirm + doc | confirm `compile-check` job green; document Phase-4/5 | ~15 min |

**Total controller compile repair: ~4–5 h of focused work, almost entirely
MECH.** The plan's MECH-heavy framing is correct; the only true judgement calls
are SEM-1 (confirm central-pool semantics — recommend repoint) and the
modelling choices SEM-2/3/4 (recommendations given).

---

## 7. Coverage orphaned by the refactors → Phase-5 backlog

Nothing was orphaned by the governance split (config logic stayed in the
controller's `governance::config`). The only *new* unverified surface comes from
ABI additions, not removals:

1. **`withdraw(... to: Option<Address>)` recipient branch** — withdraw-to-other.
   Compile repair pins `to=None`; the `Some(recipient)` path (auth of recipient,
   funds routing) has no rule. → Phase 5: `withdraw_to_recipient` rule.
2. **Batched multi-entry pool ops** — rules exercise single-entry batches via the
   `compat.rs` `*_single` shims; multi-asset atomicity across a `Vec` of
   supply/borrow/repay/withdraw entries (all-or-nothing, per-entry caps) is
   unverified. → Phase 5: a batch-atomicity rule.
3. **Separate `governance` forwarder contract** — `contracts/governance/` (the
   thin admin/timelock forwarder) has **no certora setup at all**. Owner-gating,
   forwarding correctness, and timelock are entirely unverified. → Phase 5
   (matches the plan's "governance/timelock rules" scope).

These are NOT regressions in the existing 231-rule controller suite — that suite
is fully restorable by the MECH repair above. They are net-new surface the
refactors introduced.

---

## 8. Phase-4 / Phase-5 boundary (handoff)

- **Phase 4 (prover):** out of scope here. Needs `CERTORAKEY` + funded Certora +
  stellar-cli + certora-cli; runs via the `sanity` matrix job
  (`certoraSorobanProver <conf> --rule <r>`). Green compile ≠ rules proven —
  whether the (re-signed) summaries are *sound* and the rules still *hold* is a
  Phase-4 question. SEM-2 (seize-position index drop) and SEM-4 (batch bounds)
  are the soundness items to re-check when the prover runs.
- **Phase 5 (new rules):** the three coverage gaps in §7 (withdraw-to-recipient,
  batch atomicity, the `governance` forwarder + timelock suite).
- **Durability follow-up (owner):** once `compile-check` is green on this branch,
  mark the `certora-verification` compile job a **required** check so spec drift
  can't recur silently. Not done here.
