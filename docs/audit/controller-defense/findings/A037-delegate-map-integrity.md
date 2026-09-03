# A037 — Delegate map mutation integrity

- Agent: A037
- Theme: T2
- Severity: info
- Status: defended
- Paths:
  - `common/src/types/controller.rs:63-74` (`DelegateGrant` type + lazy-revoke / re-arm contract)
  - `common/src/types/controller.rs:561-564` (`ControllerKey::Delegates(u64)`)
  - `contracts/controller/src/constants.rs:17` (`MAX_DELEGATES = 16`)
  - `contracts/controller/src/storage/account.rs:170-245` (`get_delegates`, `set_delegates`, `add_delegate`, `remove_delegate`)
  - `contracts/controller/src/storage/account.rs:247-270` (`remove_account_entry`, `renew_user_account` include Delegates)
  - `contracts/controller/src/storage/protocol.rs:193-205` (`get_user` / `set_user` user-TTL class)
  - `contracts/controller/src/account.rs:116-143` (`is_owner_or_delegate` / `require_owner_or_delegate` consumers)
  - `contracts/controller/src/account.rs:239-291` (`add_delegate` / `remove_delegate` / `set_account_delegate`)
  - `contracts/controller/src/events/account.rs:8-13` (`AccountDelegateEvent`)
  - `contracts/controller/src/lib.rs:411-423` (entrypoint wiring)
- Defense: Single persistent `DelegateGrant` value per account; private `set_delegates`; owner-stamped atomic write; read-filter + write-side purge/overwrite for stale grants; contains-before-push uniqueness; cap before push; empty → key delete; only owner-gated entrypoints and account cleanup mutate the key; use sites always pass live NFT `account.owner`
- Gap: none that forge, merge, duplicate, or resurrect authority against the live NFT owner without an owner write or documented round-trip re-arm. Residuals: (R1) NFT round-trip re-arm if no interim write; (R2) `get_user` renews zombie TTL on every filtered read; (R3) purge / wholesale overwrite emit no per-address revoke events; (R4) storage API trusts the `owner` argument (encapsulation); (R5) no unit pin that removing the last live delegate deletes the key
- Impact: Compromised map integrity would mean unauthorized borrow/withdraw/strategy control of that account (full economic power — threat-model accepted for *live* delegates). No path found that lets a stranger, former owner’s delegate (while NFT is elsewhere), or inactive manager obtain a live list membership under the current NFT holder. Protocol-wide / cross-account blast radius from this surface: none (keys are per `account_id`). Per-account residual risk is the documented re-arm window and observability of silent purges — not silent fund theft
- Evidence:
  - INV-AUTH-02, INV-RISK-04, INV-STOR-01, INV-STOR-03
  - STRIDE Spoof.3 / Elevation.4
  - `DelegateGrant` / endpoints.md lazy-revoke wording
  - Unit: `contracts/controller/tests/storage/account.rs` (idempotent add, remove, cap, stale empty read, stale purge anti-resurrection, Delegates TTL)
  - Unit: `contracts/controller/tests/helpers/account.rs` (`transfer_revokes_prior_owner_and_delegates`, owner-only gate)
  - Harness: `tests/test-harness/tests/composition/delegate_revocation_between_legs.rs`
  - Peers: A003 (use gate), A005 (grant/revoke gating), A021 (layout), A034 (TTL), A027 / A036 (cleanup clears Delegates)
- Opinion: Mutation integrity is coherent and defensive. The load-bearing properties are atomic stamp+list writes, empty-key deletion, no-merge overwrite of stale grants, and the invariant that every production `owner` argument is the live NFT holder. A005 closed self-service escalation; this file closes map-shape and lifecycle integrity. Treat R1–R3 as documented design / ops residuals, not undefended forgery.

---

## Scope (vs A005 / A021)

| Agent | Question |
|---|---|
| A005 | Who may call grant/revoke, and what dual gates apply at use time? |
| A021 | How does `Delegates` sit in the four-key account layout? |
| **A037** | Can the stored map itself be corrupted, duplicated, merged across owners, left empty-but-present, or resurrected against the protocol’s stated rules? |

Out of scope except as consumers: position-manager registry mutations (A005 / A029), economic power of a *correctly* live delegate (threat-model accepted), pause asymmetry on add vs remove (A001 / A005).

---

## 1. Storage shape

One persistent key per account:

| Item | Detail |
|---|---|
| Key | `ControllerKey::Delegates(account_id: u64)` |
| Value | `DelegateGrant { granted_by: Address, delegates: Vec<Address> }` |
| TTL class | User (`get_user` / `set_user` → `TTL_THRESHOLD_USER` / `TTL_BUMP_USER`) |
| Writer | Private `set_delegates` only (plus unconditional `remove` in cleanup) |
| Cap | `MAX_DELEGATES = 16` (`RegistryCapReached` #45) |

```63:74:common/src/types/controller.rs
/// A delegate list stamped with the owner who granted it. The grant is live only while
/// `granted_by` still owns the account's NFT: transferring the NFT deactivates the prior
/// owner's grant immediately (`get_delegates` reads it as empty for anyone else). The stale
/// entry is purged from storage the next time the new owner writes a delegate (`add_delegate`
/// or `remove_delegate`) — no explicit cleanup call is required. A grant re-arms with its
/// original delegate list only if the NFT returns to `granted_by` before any such write.
#[contracttype]
pub struct DelegateGrant {
    pub granted_by: Address,
    pub delegates: Vec<Address>,
}
```

Atomicity: stamp and list are one `contracttype` value. There is no separate “list without stamp” or “stamp without list” durable state reachable through `set_delegates`.

---

## 2. Mutation surface inventory

### 2.1 Production writers of `Delegates(account_id)`

| Path | Effect |
|---|---|
| `account::set_account_delegate(..., add=true)` → `storage::add_delegate` | Insert address or no-op; may overwrite stale grant wholesale |
| `account::set_account_delegate(..., add=false)` → `storage::remove_delegate` | Remove address, or purge entire stale grant, or no-op |
| `storage::remove_account_entry` (via `remove_account_and_burn_nft`) | Unconditional key delete with meta/supply/debt |

No other crate path calls `storage::add_delegate` / `storage::remove_delegate` outside tests. Position/strategy/liquidation finalize paths **never** rewrite Delegates (A021 matrix; A022–A027).

### 2.2 Private write helper

```181:195:contracts/controller/src/storage/account.rs
fn set_delegates(env: &Env, account_id: u64, owner: &Address, delegates: &Vec<Address>) {
    let key = ControllerKey::Delegates(account_id);
    if delegates.is_empty() {
        env.storage().persistent().remove(&key);
    } else {
        set_user(
            env,
            &key,
            &DelegateGrant {
                granted_by: owner.clone(),
                delegates: delegates.clone(),
            },
        );
    }
}
```

Integrity properties:

1. **Empty ⇒ absent** — durable state never holds `DelegateGrant { delegates: [] }`.
2. **Non-empty ⇒ stamped** — every stored value carries `granted_by =` the `owner` argument at write time.
3. **Single key** — no secondary index, no per-delegate keys, no cross-account fan-out.

### 2.3 Auth wrapper (integrity of *who* may stamp)

```256:290:contracts/controller/src/account.rs
fn set_account_delegate(...) {
    caller.require_auth();
    require_account_owner(env, account_id, caller);
    if add {
        assert_with_error!(
            env,
            storage::get_position_manager(env, delegate).is_some_and(|c| c.is_active),
            GenericError::NotAuthorized
        );
    }
    let changed = if add {
        storage::add_delegate(env, account_id, caller, delegate)
    } else {
        storage::remove_delegate(env, account_id, caller, delegate)
    };
    if changed { AccountDelegateEvent { ... }.publish(env); }
}
```

After `require_account_owner`, `caller` **is** the live NFT owner, so the stamp written into storage matches INV-STOR-03 ownership. Manager-active check on add is gating (A005); map integrity still holds if that check were absent — the list would only admit addresses the owner chose — but contemporaneous activation prevents dormant pre-approval planting.

---

## 3. Read path and the `owner` argument contract

```172:177:contracts/controller/src/storage/account.rs
pub(crate) fn get_delegates(env: &Env, account_id: u64, owner: &Address) -> Vec<Address> {
    get_user::<DelegateGrant>(env, &ControllerKey::Delegates(account_id))
        .filter(|grant| grant.granted_by == *owner)
        .map(|grant| grant.delegates)
        .unwrap_or_else(|| Vec::new(env))
}
```

**Critical encapsulation fact:** `get_delegates` does **not** re-read the NFT. It compares `granted_by` to the `owner` *argument*. Integrity of lazy revoke therefore depends on every use-time caller passing the **current** NFT holder.

Verified production consumers of `is_owner_or_delegate` / `require_owner_or_delegate` all pass `&account.owner` from `get_account` / loaded `Account` (supply, debt, strategies, liquidation Credit receiver, `load_or_create_account` Migrate/Multiply). `Account.owner` is assembled only via `try_account_owner` → NFT `owner_of` (`storage/account.rs:148-157`). No public view exposes raw `get_delegates`.

| If caller passed… | Filter result |
|---|---|
| Live NFT owner Bob, grant stamped by Alice | Empty (lazy revoke) |
| Stale address Alice, grant stamped by Alice | Would return Alice’s list — **but no production path does this** |
| Live owner Bob, grant stamped by Bob | Live list |

**R4 (residual):** storage helpers are trust-the-caller. A future internal misuse that invents an `owner` Address without NFT resolution could mis-interpret or mis-stamp grants. Today’s call graph closes that. Do not weaken to “pass any Address from calldata as owner into `get_delegates`.”

---

## 4. Add integrity

```199:217:contracts/controller/src/storage/account.rs
pub(crate) fn add_delegate(...) -> bool {
    let mut delegates = get_delegates(env, account_id, owner);
    if delegates.contains(delegate) {
        return false;
    }
    assert_with_error!(
        env,
        delegates.len() < MAX_DELEGATES,
        GenericError::RegistryCapReached
    );
    delegates.push_back(delegate.clone());
    set_delegates(env, account_id, owner, &delegates);
    true
}
```

| Property | Mechanism | Evidence |
|---|---|---|
| No duplicates | `contains` before push | `add_delegate_is_idempotent` |
| Cap exact | `len() < 16` then push → max 16 | `add_delegate_accepts_exactly_max_delegates`, `..._rejects_..._past_the_cap` (#45) |
| Idempotent at full cap | contains check **before** cap assert | Re-adding an existing member at 16 returns `false`, does not panic |
| Stale grant not merged | `get_delegates` returns empty for new owner → build list from scratch → `set_user` replaces whole value | `delegates_of_previous_owner_read_as_empty` |
| Stamp always current writer | `set_delegates(..., owner, ...)` | Type + write helper |
| Event only on real insert | `changed == true` | `AccountDelegateEvent` docs |

**Overwrite semantics (anti-merge):** when Bob owns the NFT and Alice’s grant still sits in storage, Bob’s first `add_delegate` does **not** append onto Alice’s `Vec`. It reads empty, pushes Bob’s nominee, and stores `{ granted_by: Bob, delegates: [...] }`, erasing Alice’s list in one write. That prevents cross-owner list contamination and also **prevents Alice’s old list from re-arming** if the NFT later returns to Alice (opposite of R1): the durable stamp is now Bob’s.

---

## 5. Remove integrity

```224:245:contracts/controller/src/storage/account.rs
pub(crate) fn remove_delegate(...) -> bool {
    let key = ControllerKey::Delegates(account_id);
    let Some(grant) = get_user::<DelegateGrant>(env, &key) else {
        return false;
    };
    if grant.granted_by != *owner {
        env.storage().persistent().remove(&key);
        return false;
    }
    let mut delegates = grant.delegates;
    let Some(index) = delegates.first_index_of(delegate) else {
        return false;
    };
    delegates.remove(index);
    set_delegates(env, account_id, owner, &delegates);
    true
}
```

| Case | Storage effect | Return | Event |
|---|---|---|---|
| No key | none | `false` | none |
| Stale (`granted_by != owner`) | **delete entire key** | `false` | none (R3) |
| Live, address absent | none | `false` | none |
| Live, address present | rewrite list or delete if emptied | `true` | `granted=false` |
| Live, last address | `set_delegates` empty → **remove key** | `true` | `granted=false` |

**Stale purge is unconditional** on any `remove_delegate` by the current owner: the named `delegate` need not have been in Alice’s list. One owner revoke call clears the resurrection payload. Pinned by `remove_delegate_purges_stale_grant_preventing_resurrection` (transfer Alice→Bob, Bob removes, transfer Bob→Alice, Alice’s list stays empty).

**Single-index remove:** `first_index_of` + `remove(index)` removes one occurrence. Combined with add’s `contains` gate, duplicates cannot form through the public API. Hypothetical pre-existing duplicate (manual storage / future bug) would require multiple removes — not reachable today (R5-adjacent hygiene).

**R5 (test gap):** empty-key deletion after removing the last *live* member is implied by `set_delegates` and covered for supply/debt maps explicitly; Delegates lacks a twin unit assert. Logic is identical; low verification residual only.

---

## 6. Lifecycle integrity (create → mutate → destroy)

| Phase | Delegates key |
|---|---|
| Account create | Not written (absent ≡ empty) |
| First successful add | Created via `set_user` with stamp |
| Further add/remove | In-place replace or delete |
| NFT transfer | No controller write; reads fail `granted_by` filter for new owner |
| New owner first write | Purge (remove) or overwrite (add) |
| Empty positions cleanup / bad debt | `remove_account_entry` deletes Delegates with siblings |
| `renew_account` / persist TTL | Extends TTL if key `has`; no value mutation |

Account deletion cannot leave an orphaned live grant under a burned id: `remove_account_and_burn_nft` always clears all four keys before burn (INV-STOR-03; A027). Empty-shell rent when `remove_if_empty=false` may leave meta **and** a still-stamped Delegates key until a later cleanup path (A021 / A036) — authority remains owner-gated; residual is rent + possible R1 window while the NFT still exists, not cross-account leakage.

---

## 7. Cross-account and type isolation

- Key material is `Delegates(u64)` — mutating account `N` cannot address account `M`’s grant.
- `ControllerKey` variants are distinct `contracttype` arms; typed `get_user::<DelegateGrant>` fails closed on wrong value shape at decode (storage module comment: wrongly-typed write under a typed key cannot compile at writers).
- No secondary map (e.g. manager→accounts) exists to desync; deactivation of a position manager leaves the per-account list intact but fails the use-time `is_active` leg (A005 R1 reactivation) — list integrity unchanged.

---

## 8. Event vs storage consistency

| Mutation | Durable change? | `AccountDelegateEvent`? |
|---|---|---|
| First add of address | yes | yes (`granted=true`) |
| Re-add same address | no | no |
| Remove present address | yes | yes (`granted=false`) |
| Remove absent (live grant) | no | no |
| Stale purge via remove | yes (key deleted) | **no** (R3) |
| Stale overwrite via add | yes (new stamp+list) | yes only for the *new* address added; prior owner’s addresses get no revoke events (R3) |
| Account cleanup delete | yes | no (account-level cleanup events elsewhere) |

Events are observational (A033 class). Storage is source of truth. Indexers that assume every durable revoke emits `AccountDelegateEvent` will miss purges and wholesale overwrites — ops/observability residual, not an auth hole.

---

## 9. Attack sketches (mutation / integrity)

| Sketch | Result |
|---|---|
| Append onto previous owner’s list after NFT transfer | Blocked: read-as-empty then wholesale write |
| Resurrect Alice’s grant after Bob `remove_delegate` then transfer back | Blocked: purge deleted key |
| Resurrect Alice’s grant after Bob `add_delegate` then transfer back | Blocked: stamp is Bob’s; Alice filter empty |
| Duplicate same address to inflate authority / bypass cap accounting | Blocked: `contains` |
| Add 17th distinct address | Blocked: #45 |
| Store empty `DelegateGrant` accruing rent forever | Blocked: empty → `persistent.remove` |
| Stranger writes foreign `Delegates(id)` | Blocked: `require_account_owner` (A005) |
| Delegate mutates the map | Blocked: owner-only (A005) |
| Position/strategy finalize clobbers Delegates | No write site |
| Cross-account key collision | Distinct `u64` key param |
| Use-time authority with stale stamp while NFT moved | Blocked: filter vs live `account.owner` |
| NFT round-trip to `granted_by` with **no** interim write | **Allowed by design (R1)** — list re-arms |

---

## 10. Residuals

### R1 — Round-trip re-arm (documented design)

If the NFT returns to `granted_by` before the interim owner calls `add_delegate` or `remove_delegate`, the original list becomes live again. Stated on `DelegateGrant`, endpoints.md, threat-model, A005, A021. Mitigation: any new-owner delegate write (including a purge-style remove). Not a map corruption bug.

### R2 — Zombie TTL extended by filtered reads

`get_delegates` → `get_user` renews the persistent key **before** the `granted_by` filter. After transfer, every use-time `is_owner_or_delegate` check (and any other read) that loads the stale grant extends its user TTL, lengthening the R1 window until archive or purge/overwrite. Aligns with INV-STOR-01 “renew when read” but is adverse to “let zombies die.” Severity: info. Mitigation unchanged: new owner should write once.

### R3 — Silent purge / overwrite observability

Stale purge returns `false` and skips events; add-overwrite does not emit revoke events for erased prior addresses. Storage correct; indexers / monitoring incomplete.

### R4 — Storage trusts `owner` parameter

See §3. Encapsulation residual for future call sites.

### R5 — Missing empty-key unit pin for last live remove

Logic in `set_delegates` is clear; twin test exists for supply/debt empty maps, not Delegates. Optional hardening.

### Out of A037 (owned elsewhere)

- Reactivation re-arms **without** map rewrite when governance toggles manager active (A005 R1) — list integrity intact by design.
- Governance Sensitive delay on global manager kill (A005 R3 / A020).
- Formal CVL gap on owner-only add (A005 R5).

---

## 11. Cross-links

- **A003** — use-time triple gate consumes this map’s membership bit.
- **A005** — who may mutate; manager dual switch; pause asymmetry.
- **A021** — four-key layout; delegates column in persist matrix.
- **A027 / A036** — cleanup deletes Delegates with the account.
- **A034 / A017** — Delegates TTL renew via `set_user` / `renew_user_account`; add/remove do not co-renew siblings.
- **A062** — `MAX_DELEGATES` as INV-RISK-04 adjacent bound (this file owns mutation semantics of that bound).

---

## 12. Verdict

**Defended.** The delegate map’s mutation rules preserve a single owner-stamped list per account, refuse duplicates and overflow, delete empty keys, purge or overwrite stale grants without merging, and clear on account destruction. Live authority still requires the A005 dual gate on top of membership. Remaining items are the documented NFT round-trip re-arm, TTL charity to zombie keys, event silence on purge/overwrite, and encapsulation/test hygiene — not undefended forgery or cross-account corruption of delegate storage.
