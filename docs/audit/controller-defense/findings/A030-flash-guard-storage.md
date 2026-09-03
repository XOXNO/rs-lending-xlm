# A030 — Flash guard storage flag lifecycle

- Agent: A030
- Theme: T2 (storage mutations; overlaps T1 flash-guard inventory — see A007)
- Severity: info
- Status: defended
- Paths:
  - `contracts/controller/src/storage/account.rs:276-312` (`SessionKey`, `is_flash_loan_ongoing`, `set_flash_loan_ongoing`, `with_flash_guard`)
  - `contracts/controller/src/storage/mod.rs:21` / `:36-37` (unconditional re-exports of read/RAII; `set_flash_loan_ongoing` feature-gated)
  - `contracts/controller/src/risk/validation.rs:12-24` (checkers)
  - Six production setters: `strategies/flash_loan.rs:35`, `strategies/flash_position.rs:126`, `strategies/swap.rs:89`, `strategies/legs.rs:103`, `positions/debt.rs:275`, `external/blend.rs:91`
  - Unit: `contracts/controller/tests/storage/account.rs:413-436` (GH-14 nesting)
  - Formal: `certora/controller/spec/flash_loan_rules.rs` (`flash_loan_guard_cleared_after_summarized_pool_return` and siblings)
- Defense: Single temporary-storage boolean under a private `SessionKey` enum; set via presence of `true`, clear via **remove** (never store `false`); `with_flash_guard` records prior state and clears only when it opened the window (nesting-safe); production modules cannot call the raw setter (private `mod account` + cfg-gated re-export); panic/failure relies on Soroban tx atomicity to roll the temp write back; checkers read the same key.
- Gap: none that breaks INV-FLASH-02 lifecycle. Residuals: (1) raw `set_flash_loan_ongoing` body is not `cfg`-gated—only the `storage::` re-export is (still unreachable from other controller modules in release builds); (2) no `Drop`/scopeguard—cleanup is sequential after `f()`; correct under Soroban panic=abort + ledger atomicity, fragile if someone later adds recoverable try-semantics around the closure; (3) bool-prev nesting (not a depth counter) assumes the only writer is `with_flash_guard`; (4) a hypothetically stuck `true` temp entry would DoS monetary entrypoints until temp TTL expiry—production paths cannot leave it stuck; (5) post-clear same-invocation legs are intentional and owned by A007, not a storage-lifecycle bug.
- Impact: Lifecycle failure modes that would matter in production are closed. A stuck flag would be protocol-wide availability (all 18 monetary entrypoints return `#400`) until temporary expiry—not theft. Nested clear-too-early would be Critical (open reentrancy during callback); GH-14 + nesting semantics close that. Cross-tx sticky flag from current code: none.
- Evidence: INV-FLASH-02; STRIDE Tamper.5 / R.1–R.2; ADR-0010; ADR-0017 (testing setter surface); `docs/reference/errors.md` #400; unit GH-14; Certora clear-after-flash rule; harness/fuzz `flash_guard_cleared` assertions across flash_loan, flash_position, strategies, migrate_blend, accounting conservation.
- Opinion: The storage lifecycle is the right shape for a Soroban session reentrancy latch: temporary, remove-on-clear, nest-by-prev, production write path funneled through one RAII helper. A007 covers *who must check*; this finding covers *how the bit is born, nests, and dies*. Treat any new writer outside `with_flash_guard`, or any change that clears without consulting `prev`, as a Critical regression.

## Method

1. Read `SessionKey` / get / set / remove / `with_flash_guard` in `storage/account.rs`.
2. Verify crate visibility: private `mod account`, unconditional vs cfg re-exports in `storage/mod.rs`, `test_support` behind `feature = "testing"`.
3. Trace all production `with_flash_guard` call sites and confirm none call the raw setter.
4. Analyse lifecycle states: idle → open → nested → unwind → clear; panic path; sequential multi-window same tx.
5. Cross-check INV-FLASH-02 nesting claim, STRIDE atomicity claim, Certora clear rule, GH-14 unit test, and A007 residuals so this file does not re-litigate entrypoint coverage.

---

## 1. Storage design

```276:312:contracts/controller/src/storage/account.rs
#[contracttype]
#[derive(Clone, Debug)]
enum SessionKey {
    FlashLoanOngoing,
}

pub(crate) fn is_flash_loan_ongoing(env: &Env) -> bool {
    env.storage()
        .temporary()
        .get(&SessionKey::FlashLoanOngoing)
        .unwrap_or(false)
}

pub(crate) fn set_flash_loan_ongoing(env: &Env, ongoing: bool) {
    if ongoing {
        env.storage()
            .temporary()
            .set(&SessionKey::FlashLoanOngoing, &true);
    } else {
        env.storage()
            .temporary()
            .remove(&SessionKey::FlashLoanOngoing);
    }
}

pub(crate) fn with_flash_guard<T>(env: &Env, f: impl FnOnce() -> T) -> T {
    let prev = is_flash_loan_ongoing(env);
    set_flash_loan_ongoing(env, true);
    let out = f();
    if !prev {
        set_flash_loan_ongoing(env, false);
    }
    out
}
```

| Property | Choice | Why it matters |
|---|---|---|
| Storage class | `temporary()` | Session latch for the current invocation tree; not durable account state; cheaper than persistent; expires if ever stranded |
| Key type | Private `SessionKey` enum (single variant) | Separate `#[contracttype]` from `ControllerKey` — distinct XDR type tag; no collision with meta/supply/debt/delegate keys |
| True encoding | `set(..., &true)` | Presence + value both indicate ongoing |
| False encoding | `remove` key | Missing ⇒ `unwrap_or(false)`; avoids a durable “false” temp entry and the footgun of `Some(false)` vs absent |
| Nesting | Save `prev`, clear iff `!prev` | Inner window must not open the outer callback (GH-14 / INV-FLASH-02) |
| TTL bump | None on this key | Same-transaction read/write only; no `extend_ttl` needed for the intended lifecycle |

Read path is total: unset temporary data is idle (`false`), never trap-on-missing.

---

## 2. Write authority (who can mutate the bit)

| Surface | Release / deployable controller | `test` / `testing` / `certora` |
|---|---|---|
| `with_flash_guard` | yes (`storage::` re-export) | yes |
| `is_flash_loan_ongoing` | yes | yes |
| `set_flash_loan_ongoing` via `storage::` | **no** (cfg re-export only) | yes |
| `storage::account::…` from other modules | **no** (`mod account` is private) | n/a |
| `controller::test_support::{set,is}_flash_loan_ongoing` | **no** (`#[cfg(feature = "testing")]` in `lib.rs:33-35`) | harness builds |

Production setters all go through `storage::with_flash_guard` (six sites; matches INV-FLASH-02 inventory and A007 §2). No other `contracts/**/*.rs` production path writes the key.

Residual (style / defense-in-depth): the function `set_flash_loan_ongoing` itself is compiled into the crate unconditionally; only the re-export is gated. Today that is harmless because the submodule is private and the sole in-crate caller is `with_flash_guard`. A future `pub use account::*` or moving the fn would widen the write surface—prefer keeping the funnel single.

ADR-0017 / `make wasm-testing-abi-check`: the harness setter is not a `#[contractimpl]` entrypoint; it must not ride along via the `testing` feature in deploy artifacts. Lifecycle claim for mainnet WASM: only `with_flash_guard` writes.

---

## 3. Lifecycle state machine

### 3.1 Happy path (outer window)

```text
idle (key absent)
  → with_flash_guard: prev=false; set true
  → f() runs (untrusted callback / pool / router / Blend)
  → !prev ⇒ remove key
  → return out
idle
```

Checkers (`require_not_flash_loaning`) see `true` for the entire body of `f()`, including nested cross-contract reentry into the controller.

### 3.2 Nested windows (load-bearing)

```text
outer: prev=false; set true
  inner: prev=true; set true
  inner end: prev true ⇒ do not clear          ← GH-14
  … callback still protected …
outer end: prev false ⇒ remove
```

Concrete composition: `process_flash_position` outer guard wraps mint/forward + `execute_flash_position`; `borrow_into_controller` takes an inner guard around the pool transfer. Without nest-by-prev, the inner clear would leave the receiver callback ungarded.

Unit pin: `nested_flash_guard_windows_keep_the_outer_flag_until_the_outer_window_closes` (`tests/storage/account.rs`, GH-14).

Bool-prev is equivalent to a depth counter for a single boolean latch as long as every writer uses `with_flash_guard`. A manual `set(false)` mid-window would break nesting; production cannot do that (§2).

### 3.3 Sequential windows in one transaction

Strategies often open disjoint windows (e.g. `borrow_into_controller` guard, later `call_router_with_reentrancy_guard`). Each outer window starts from idle, sets, clears. Flag is not “latched for the whole strategy entrypoint”—only for each untrusted handoff. That is intentional (A007 §5 residual: post-guard `process_deposit` / refunds run clear).

### 3.4 Failure / panic path

`with_flash_guard` does not use `Drop` or `defer`. If `f()` panics (`assert_with_error!`, host trap, etc.):

1. The `remove` after `f()` does not run.
2. Soroban aborts the transaction and rolls back **all** ledger writes for the tx, including the temporary `set`.

STRIDE Tamper.5 R.2 states this explicitly: the guard defends nested entry, not partial commits. Harness/adversarial tests assert `flash_guard_cleared` after failed flash / strategy / reentry attempts (tx failed ⇒ env shows idle flag in the test host’s post-state model).

Residual: if a future change wrapped `f()` in recoverable host `try_call` semantics and continued the outer Rust function after a failed subcall **without** aborting the tx, missing Drop cleanup could leave the flag set for the rest of a successful transaction. Current call sites do not do that; flag it if try-semantics are introduced around guarded closures.

### 3.5 Cross-transaction stickiness

Temporary entries can survive across ledgers until TTL expiry **if left written**. Current production lifecycle:

| Scenario | Key left set after successful tx? |
|---|---|
| Outer `with_flash_guard` completes | No — `prev=false` ⇒ remove |
| Nested only | No — outer still removes |
| Panic / trap mid-window | N/A — full tx rollback |
| `set_flash_loan_ongoing(true)` then commit without clear | Impossible in release (no reachable raw setter) |

Theoretical sticky-`true` DoS (monetary surface stuck on `#400` until temp expiry) requires a write path that commits `true` without a matching remove. Not present in audited production code. Temp class remains a useful backstop versus persistent.

No `extend_ttl` on `SessionKey` is correct: the protocol never intends this key to outlive the invoking transaction.

---

## 4. Read / check coupling

```12:24:contracts/controller/src/risk/validation.rs
pub(crate) fn require_authorized_caller(env: &Env, caller: &Address) {
    caller.require_auth();
    require_not_flash_loaning(env);
}

pub(crate) fn require_not_flash_loaning(env: &Env) {
    assert_with_error!(
        env,
        !storage::is_flash_loan_ongoing(env),
        FlashLoanError::FlashLoanOngoing
    );
}
```

- Single source of truth: checkers call `is_flash_loan_ongoing` (same temp key).
- Entrypoints that open a window call `require_authorized_caller` **before** `with_flash_guard`, so they cannot start nested from an already-set flag via the public ABI (A007 §4.1).
- Temporary storage is **per contract**: the latch lives on the controller. Pool/receiver/router contracts do not share this key; reentry into the controller observes the controller’s temp map—correct for the threat model.

Views (`get_health_factor`, `is_liquidatable`) intentionally omit the check (threat-model “Risk views are not flash-guarded”). Not a storage lifecycle defect.

`errors.md` #400 wording “except `flash_loan` itself” means the idle entry is allowed to start a flash loan; once the flag is set, nested `flash_loan` is also refused (it uses `require_authorized_caller`). Lifecycle matches that reading.

---

## 5. Formal and test evidence (lifecycle-specific)

| Claim | Evidence |
|---|---|
| Nesting does not clear early | Unit GH-14; INV-FLASH-02 text; flash_position outer+inner composition |
| Flag clear after successful summarized flash | Certora `flash_loan_guard_cleared_after_summarized_pool_return` |
| Flag blocks when set / allows when clear | Certora `flash_loan_guard_blocks_*` / `_allows_when_clear` |
| Failed / adversarial paths leave flag clear (post-tx) | `flash_loan.rs`, `flash_loan_adversarial.rs`, `flash_position_adversarial.rs`, `strategy/adversarial.rs`, fuzz `assert_flash_guard_cleared` / `flash_guard_cleared` |
| Idle default | `unwrap_or(false)`; unit assert before outer guard |

A035 (Certora harness storage overrides) is the sibling scope for whether verification storage could desync from this key; harness `verification_storage` does not replace the flash-guard helpers—rules call real `set`/`is`/`process_flash_loan`.

---

## 6. Deliberate non-goals (owned elsewhere)

| Topic | Owner |
|---|---|
| Which 18 entrypoints must check the flag | A007, STRIDE Tamper.5 |
| Post-guard listed-token transfer hooks during settlement | A007 §5, A055 |
| Ungated `renew_account` / delegate verbs under the flag | A007, GH-28 `reentrancy_matrix` |
| Wasm-receiver / invalid receiver gates | A019 |
| Flash principal+fee pullback economics | A044 / INV-FLASH-01 |

This finding does not re-score those residuals.

---

## 7. Verdict

**Defended.** The flash-guard bit’s storage lifecycle—temporary key, remove-on-clear, nest-by-previous, production writes only via `with_flash_guard`, failure rolled back by ledger atomicity—is coherent and matches INV-FLASH-02 / Tamper.5.

No production code change recommended from A030 alone. Optional hardening if ever scoped: `#[cfg(...)]` the raw setter definition (not only the re-export); or a depth counter / debug assert that `set(false)` is never observed mid-window. Neither is required for the current threat model.
