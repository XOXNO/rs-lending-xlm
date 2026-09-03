# A071 — Blend pool approval check on migrate

- Agent: A071
- Theme: T4 (input validation) / T1 (trust boundary TB11)
- Severity: info
- Status: defended
- Paths:
  - `contracts/controller/src/strategies/migrate_blend.rs:57-63,181-198` (`validate_migration_request` → `is_blend_pool_approved`)
  - `contracts/controller/src/storage/protocol.rs:14-27` (`is_blend_pool_approved` / `set_blend_pool_approved`)
  - `contracts/controller/src/config/registry.rs:38-43` (`set_blend_pool_approval` + event)
  - `contracts/controller/src/lib.rs:342-372` (`migrate_from_blend`), `:555-558` (view), `:613-631` (`approve_blend_pool` / `revoke_blend_pool`)
  - `contracts/controller/src/external/blend.rs` (`blend_repay_all` / `blend_sweep_all` / `guarded_submit` / `authorize_repay_pulls`)
  - `common/src/types/controller.rs:560` (`ControllerKey::BlendPoolAllowed`)
  - `common/src/errors.rs:70` (`BlendPoolNotApproved = 42`)
  - Governance: `contracts/governance/src/op.rs:249-263` (`ApproveBlendPool` / `RevokeBlendPool` + `require_contract_address`); `contracts/governance/src/validate/mod.rs:15-24`
  - Interfaces: `interfaces/controller/src/lib.rs` / `admin.rs`; `interfaces/governance/src/lib.rs`
- Defense: `migrate_from_blend` refuses any `blend_pool` that is not present under `ControllerKey::BlendPoolAllowed` **before** account load, borrow, Blend `submit`, or deposit. Missing key defaults to `false` (fail-closed). Allowlist writes are `#[only_owner]` (governance-owned in production) with typed `ApproveBlendPool` / `RevokeBlendPool` ops on the **Standard** timelock tier; governance proposal validation requires a live Wasm contract address. The only production callers of Blend FFI are `migrate_blend`’s repay/sweep legs. Cap-auth, flash guard, measured Δ, and HF finalize bound damage if an **approved** pool misbehaves (A050 / INV-STRAT-03 / Tamper.8).
- Gap: (1) **Accepted trust residual** — approval does not pin Blend bytecode/hash; an approved pool’s own owner can upgrade behavior until governance revokes (threat-model K12 / A105). (2) **Admission asymmetry** — controller `approve_blend_pool` / `revoke_blend_pool` do **not** re-run `require_contract_address`; only the governance propose path does. Direct owner calls can mark an EOA/non-Wasm address approved (unit test uses `Address::generate`); migrate against a non-contract then fails closed at `BlendPoolClient::submit`. (3) **Defense-in-depth** — `external/blend.rs` helpers do not re-assert the allowlist; safety relies on the single `validate_migration_request` call site (today the sole caller). (4) **TTL** — `BlendPoolAllowed` is shared-persistent (5d threshold / 180d bump); unread expiry reads as unapproved (fail-closed ops availability, not silent approval). (5) Governance `RevokeBlendPool` still requires a live Wasm address — destroyed pools may need direct controller `revoke_blend_pool` as hatch.
- Impact: Unapproved pools cannot run migration money movement (`#42`). No path found that skips the allowlist, treats unset storage as approved, or lets a stranger point `blend_pool` at an arbitrary contract and pull controller-borrowed funds. Blast radius of a **governance-approved** hostile/upgraded Blend is the migrator’s own attempt inside one atomic tx (HF revert or self-harm), not silent third-party share mint — matches A050 / A101 G-BLEND.
- Evidence: INV-STRAT-03; STRIDE TB11 / I9 / Tamper.8 R.1; threat-model “Blend migration” + Known gap “approved Blend pool can be upgraded”; `docs/reference/endpoints.md` migrate; `errors.md` #42; permissionless line for `migrate_from_blend`; harness `test_migrate_unapproved_blend_pool_reverts`; fuzz `migrate_blend_rejects_empty_duplicate_unapproved_zero_cap`; unit `process_migrate_blend_rejects_unapproved_pool`, storage allowlist round-trip, governance config round-trip; integration `blend.sh` allowlist + `blend_unapproved` xfail #42. Peers A001, A009, A029, A034, A050, A062, A101, A105 K12.
- Opinion: The migrate-side approval gate is complete and correctly placed (fail-closed, pre-money). Treat Blend approval as a high-trust governance decision equal in spirit to listing a router venue — Standard delay is acceptable because harm is migrator-local, but operators must monitor Blend upgrades and revoke promptly. Optional hardening (not required for fund safety today): (a) `require_contract_address` inside controller `set_blend_pool_approval(true)`, (b) re-assert allowlist inside `guarded_submit`, (c) document the direct-revoke hatch when Wasm no longer exists.

## Scope and method

1. Read `shared/COORDINATION.md`, `SEED.md`, AGENT_MANIFEST A071, INV-STRAT-03, threat-model Blend rows, STRIDE TB11 / Tamper.8 / I9, endpoints migrate, errors #42 / #18.
2. Confirmed `A071-blend-pool-approval.md` absent (A102 listed A071 unfiled; A110 backlog “gate inventory still owed”).
3. Traced allowlist **write** path (admin + governance validate + storage + event + TTL) and **read** path (`migrate_from_blend` → `validate_migration_request` → `is_blend_pool_approved`).
4. Enumerated every production `blend_*` / `BlendPoolClient` / `is_blend_pool_approved` call site under `contracts/`.
5. Cross-checked A050 (money-flow owner; defers dedicated approval inventory here), A009 (admin surface), A007 (flash window on Blend submit), A001 (macros), A029/A034 (storage/TTL), A101 G-BLEND, A105 K12.
6. Out of primary claim: leftover repay arithmetic (A050), debt uniqueness / empty-request hygiene (A062), post-pool HF (A072), spoke listing of swept assets (A040/A064), auth owner-or-delegate (A003).

---

## 1. Why this gate exists

`migrate_from_blend` is the only controller path that:

1. Mints hub debt into **controller custody** (`borrow_into_controller`, `charge_fee = false`).
2. Pre-authorizes token `transfer(controller → blend_pool, max)` per `debt_caps`.
3. Invokes an **external** `submit` on a caller-chosen address.
4. Treats subsequent controller balance increases as migrator supply/debt reconciliation.

Without an allowlist, any Wasm implementing a Blend-shaped `submit` could drain the just-borrowed tokens. INV-STRAT-03 / TB11 make that address a **governance** decision, not a user choice.

```
caller-chosen blend_pool
        │
        ▼
is_blend_pool_approved? ──no──► #42 BlendPoolNotApproved (no borrow, no submit)
        │ yes
        ▼
borrow → authorize_repay_pulls → guarded_submit → measure → finalize
```

---

## 2. Gate inventory (migrate read path)

### 2.1 Entrypoint ordering

```342:372:contracts/controller/src/lib.rs
// #[when_not_paused] migrate_from_blend → process_migrate_blend(...)
```

Inside `process_migrate_blend`:

| Step | Check | Error | Money moved? |
|---|---|---|---|
| 1 | `require_authorized_caller` (`require_auth` + not flash-loaning) | auth / `#400` | no |
| 2 | `config::require_hub_active` | `#43 HubNotActive` | no |
| 3 | `validate_migration_request` non-empty vectors | `#16 InvalidPayments` | no |
| 4 | `validate_migration_request` **`is_blend_pool_approved`** | **`#42 BlendPoolNotApproved`** | **no** |
| 5 | `require_unique_debt_assets` | `#7 AssetsAreTheSame` | no |
| 6 | `load_or_create_account` (`AccountGuard::Migrate`) | auth / spoke | no |
| 7 | `require_can_supply` per withdraw asset | listing / freeze | no |
| 8+ | debt leg / sweep / deposit / finalize | various | yes |

Approval is checked **after** pause + hub-active + non-empty request, and **before** account mutation, borrow, or any Blend FFI. Empty-request fails with `#16` first (so an empty call against an unapproved pool never reaches `#42`) — test hygiene only; not a bypass.

### 2.2 Exact assertion

```181:198:contracts/controller/src/strategies/migrate_blend.rs
fn validate_migration_request(
    env: &Env,
    blend_pool: &Address,
    collateral_assets: &Vec<Address>,
    supply_assets: &Vec<Address>,
    debt_caps: &Vec<(Address, i128)>,
) {
    assert_with_error!(
        env,
        !collateral_assets.is_empty() || !supply_assets.is_empty() || !debt_caps.is_empty(),
        GenericError::InvalidPayments
    );

    assert_with_error!(
        env,
        storage::is_blend_pool_approved(env, blend_pool),
        GenericError::BlendPoolNotApproved
    );
}
```

### 2.3 Storage semantics (fail-closed)

```14:27:contracts/controller/src/storage/protocol.rs
pub(crate) fn is_blend_pool_approved(env: &Env, pool: &Address) -> bool {
    get_shared(env, &ControllerKey::BlendPoolAllowed(pool.clone())).unwrap_or(false)
}

pub(crate) fn set_blend_pool_approved(env: &Env, pool: &Address, approved: bool) {
    let key = ControllerKey::BlendPoolAllowed(pool.clone());
    if approved {
        set_shared(env, &key, &true);
    } else {
        env.storage().persistent().remove(&key);
    }
}
```

| Property | Behavior |
|---|---|
| Default | unset → `false` |
| Approve | persist `true` under `BlendPoolAllowed(addr)` + shared TTL bump |
| Revoke | **remove** key (do not store `false`) |
| Key family | per-address; no global “all Blend pools” bit |
| Read side effect | `get_shared` renews TTL when present (A034) |

Unit: `blend_pool_allowlist_approve_then_revoke` — double-approve and double-revoke stay idempotent; revoke leaves `is_blend_pool_approved == false`.

### 2.4 Single consumer of Blend FFI

Production graph under `contracts/`:

| Symbol | Callers |
|---|---|
| `blend_repay_all` / `blend_sweep_all` | **only** `migrate_blend.rs` |
| `BlendPoolClient::submit` | **only** `guarded_submit` in `external/blend.rs` |
| `is_blend_pool_approved` (mutator) | **only** `validate_migration_request` |
| `is_blend_pool_approved` (view) | `lib.rs` view + tests |

There is no alternate migrate, keeper, or strategy path that can invoke Blend without passing the allowlist. Residual: helpers themselves do not re-check — a future internal caller would need the same gate (defense-in-depth Gap 3).

### 2.5 Complementary controls (not substitutes)

These do **not** replace the allowlist; they bound an **approved** pool:

| Control | Where | Role |
|---|---|---|
| `authorize_repay_pulls` caps | `external/blend.rs` | Blend cannot pull more than each `debt_caps` max |
| `with_flash_guard` | `guarded_submit` | Blocks monetary reentry during `submit` (A007) |
| Measured leftovers / deposits | `migrate_blend.rs` | No credit from Blend return values (A050) |
| `strategy_finalize` HF / min collateral | shared | Unhealthy end-state reverts (A072) |
| Spoke listing / `require_can_supply` | pre-sweep | Swept assets must be supplyable on the target hub |

---

## 3. Allowlist write path (governance / admin)

### 3.1 Controller admin surface

| Entrypoint | Macros | Body |
|---|---|---|
| `approve_blend_pool(pool)` | `#[only_owner]` only (not pause-gated) | `renew_then!` → `set_blend_pool_approval(..., true)` |
| `revoke_blend_pool(pool)` | `#[only_owner]` only | `renew_then!` → `set_blend_pool_approval(..., false)` |
| `is_blend_pool_approved(pool)` | neither (view) | storage read |

Pause asymmetry is intentional and useful: migrate is `#[when_not_paused]`, but revoke remains available while paused so operators can cut migration sources before unpause. A007 notes `ControllerAdmin` does not check the flash flag — acceptable because admin requires owner auth; a Blend callback cannot forge owner authorization to flip the allowlist mid-`submit`.

### 3.2 Registry + event

```38:43:contracts/controller/src/config/registry.rs
pub(crate) fn set_blend_pool_approval(env: &Env, pool: Address, approved: bool) {
    storage::set_blend_pool_approved(env, &pool, approved);
    ApproveBlendPoolEvent { pool, approved }.publish(env);
}
```

Both approve and revoke emit `ApproveBlendPoolEvent` with `approved: bool` (topics `["config", "approve_blend_pool"]`) — indexers can track the allowlist without separate revoke topic (events.md).

### 3.3 Governance typed ops

| Op | Propose validation | Delay tier | Executes |
|---|---|---|---|
| `ApproveBlendPool(pool)` | `require_contract_address` → else `#18 NotSmartContract` | **Standard** (`controller_operation`) | `approve_blend_pool` |
| `RevokeBlendPool(pool)` | same Wasm existence check | **Standard** | `revoke_blend_pool` |

```15:24:contracts/governance/src/validate/mod.rs
// exists() && Executable::Wasm(_) required
```

Harness `ensure_approved_blend` uses `gov_client().execute_immediate(ApproveBlendPool)` — production path is timelocked Standard delay (A009). Contrast: `SetPositionManager` is **Sensitive**; Blend approval is Standard because residual harm is migrator-scoped (A101), not protocol-wide manager registry.

### 3.4 Admission asymmetry (Gap 2)

| Path | Wasm/contract check? |
|---|---|
| Governance `ApproveBlendPool` / `RevokeBlendPool` at propose | **yes** |
| Controller `approve_blend_pool` / `revoke_blend_pool` | **no** |
| Unit `blend_pool_approval_entrypoints_round_trip` | approves bare `Address::generate` successfully |

In the intended deployment, controller owner is governance and mutations go through typed ops, so production admits only Wasm. Direct `only_owner` bypass (compromised owner key, test harness, or mis-wired ownership) can pollute the allowlist with non-contracts; migrate then fails at invoke time (`MissingValue` / invoke error), not with `#42`. Fund-safe, ops-noisy. Optional fix: mirror `require_contract_address` on approve (and consider relaxing it on revoke so destroyed pools stay revocable via governance).

### 3.5 No Blend bytecode attestation

Approval stores a boolean keyed by address. There is no hash pin, interface probe, or “is this the Blend factory pool” check. That is the documented Known gap (threat-model / A105 K12): **approval trusts the pool’s upgrade authority forever until revoke**.

---

## 4. Attack / bypass analysis

| Hypothesis | Result |
|---|---|
| Caller passes unapproved MockBlend / arbitrary Wasm | `#42` before borrow — harness + fuzz + integration `blend_unapproved` |
| Unset storage treated as approved | No — `unwrap_or(false)` |
| Skip allowlist via empty collateral but non-empty debt | Still checked; debt-only migrate must be approved |
| Call `blend_repay_all` from another strategy | No other callers |
| Reenter during `submit` to flip allowlist | Admin needs owner auth; monetary reentry blocked by flash guard; even a mid-call revoke would not redirect the already-bound `blend_pool` address |
| Concurrent tx approves then migrator uses stale deny | Soroban reads current persistent state per invocation; each migrate re-reads allowlist |
| TTL expiry of `BlendPoolAllowed` | Reads as unapproved — **fail-closed**; migrators DoS until re-approve / renew via view/migrate |
| Approve self (controller) or lending pool | Allowed if governance chooses; `submit` would need to implement Blend ABI; still migrator-local + HF |
| Hostile approved pool consumes full pull | A050 residual — HF revert or migrator self-harm; baselines protect stuck controller inventory / other users |
| Privilege: stranger `approve_blend_pool` | Rejected — fuzz `privileged_auth_rejects`; `#[only_owner]` |
| Pause then migrate unapproved | Pause rejects first; after unpause, `#42` still holds |

No novel critical bypass of INV-STRAT-03 found.

---

## 5. Auth / pause / permissionless claims

| Claim source | Statement | Live code |
|---|---|---|
| `scripts/permissionless_entrypoints.txt` | `migrate_from_blend` \| caller-auth \| INV-AUTH-02, INV-STRAT-02 \| “governance-approved pool only” | Matches: pause + auth + allowlist + Migrate guard |
| STRIDE I9 | `require_auth` + approved Blend + reentrancy gate | Matches (auth via `require_authorized_caller`; reentrancy via guard on submit) |
| STRIDE Tamper.8 R.1 | Governance allowlist + measured flows + risk gates | Matches |
| endpoints.md | `blend_pool` must be approved; approval is governance | Matches |

`migrate_from_blend` is permissionless **to invoke**, not permissionless to choose the Blend target.

---

## 6. Test / evidence matrix

| Layer | Coverage |
|---|---|
| Unit strategy | `process_migrate_blend_rejects_unapproved_pool` expects `#42` |
| Unit storage | approve/revoke/idempotent false |
| Unit governance config | entrypoint round-trip; registry helper reflects storage |
| Unit external blend | non-live pool submit panics (MissingValue) — orthogonal to `#42` |
| Harness | `test_migrate_unapproved_blend_pool_reverts` (registered MockBlend, **not** approved) |
| Fuzz | `migrate_blend_rejects_empty_duplicate_unapproved_zero_cap` includes unapproved `#42` + flash guard cleared |
| Integration | `blend.sh` approve/revoke/reapprove views; `xfail blend_unapproved` `#42` after revoke |
| Invariants doc | INV-STRAT-03 cites harness unapproved test |
| Certora | No dedicated allowlist rule found under `certora/` (enforcement is storage+assert; not a numeric invariant) |

Gaps in tests (info): no harness that asserts **direct** controller approve of an EOA then migrate fail-mode; no test that `guarded_submit` alone without prior validate is unreachable (structural today).

---

## 7. Peer cross-refs and disagreement

| Peer | Relationship |
|---|---|
| A050 | Owns money-flow; states allowlist once; defers deep gate inventory → **this file** |
| A101 G-BLEND | Synthesis of approved-pool residual — **accepted**; A071 confirms the gate side |
| A105 K12 | Threat-model Known gap accurate |
| A009 | Admin inventory + Standard tier + gov Wasm check |
| A001 | Macro inventory: approve/revoke owner-only; view ungated |
| A007 | Blend submit under flash guard; admin not flash-checked |
| A029 / A034 | Persistent shared key + TTL class |
| A062 | Empty/duplicate migrate inputs adjacent; approval orthogonal |
| A102 | Listed A071 unfiled — **now filed** |

No disagreement file needed: A050’s “allowlist before borrow” claim matches live code; residual trust is explicitly accepted, not a missed `#42` hole.

---

## 8. Verdict

**Defended.** The Blend pool approval check on migrate is a complete, fail-closed, pre-money allowlist enforcing INV-STRAT-03 / TB11. Unapproved addresses cannot reach borrow or `submit`. Remaining items are accepted governance trust (approved-pool upgrades), optional admission hygiene on the direct admin path, and defense-in-depth re-assert inside Blend FFI — none open a silent protocol theft path beyond what A050 / A101 already bound to the migrator’s own transaction.
