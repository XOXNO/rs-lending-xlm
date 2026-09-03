# A005 — Delegate add/remove + position-manager gating

- Agent: A005
- Theme: T1
- Severity: info
- Status: defended
- Paths:
  - `contracts/controller/src/lib.rs:405-422` (`renew_account`, `add_delegate`, `remove_delegate`)
  - `contracts/controller/src/lib.rs:603-611` (`set_position_manager`)
  - `contracts/controller/src/account.rs:116-143` (`is_owner_or_delegate`, `require_owner_or_delegate`)
  - `contracts/controller/src/account.rs:145-152` (`require_account_owner`)
  - `contracts/controller/src/account.rs:228-291` (`renew_account`, `add_delegate`, `remove_delegate`, `set_account_delegate`)
  - `contracts/controller/src/storage/account.rs:170-245` (`get_delegates`, `add_delegate`, `remove_delegate`)
  - `contracts/controller/src/storage/protocol.rs:135-148` (`get_position_manager`, `set_position_manager`)
  - `contracts/governance/src/op.rs:339-343`, `:455` (`AdminOperation::SetPositionManager` → Sensitive; blocked from immediate path)
  - `scripts/permissionless_entrypoints.txt:53-55`
- Defense: Owner-only grant/revoke/renew; dual gate (account list + active manager) at use time; contemporaneous active-manager check at grant; pause-exempt revoke; NFT `granted_by` lazy revoke; governance-only manager registry on Sensitive delay
- Gap: none that lets a delegate grant or renew its own authority. Residuals are documented design properties (reactivation re-arms stored grants; NFT round-trip can re-arm if new owner never writes delegates; governance deactivate is Sensitive-delayed, not a hot-key kill)
- Impact: Blast radius of a live delegate is the full economic control of that account (borrow/withdraw/strategies to arbitrary `to`) — by design, not an escalation from this surface. Self-grant / self-renew escalation path is closed. Protocol-wide fund risk from this gating alone is none; per-account loss requires a prior owner grant to a governance-approved manager
- Evidence:
  - INV-AUTH-02 (`docs/reference/invariants.md`)
  - STRIDE Spoof.3 / Elevation.4
  - `docs/reference/endpoints.md` (Controller — account and delegation)
  - Unit: `contracts/controller/tests/helpers/account.rs` (`require_account_owner_rejects_active_delegate`, manager inactive / not opted-in rejects, NFT transfer revoke)
  - Unit: `contracts/controller/tests/storage/account.rs` (cap, stale purge, lazy empty)
  - Unit: `contracts/controller/tests/governance/config.rs` (`remove_delegate_reverts_account_not_in_market_for_non_owner`)
  - Harness: `tests/test-harness/tests/composition/delegate_revocation_between_legs.rs`
  - Harness: `tests/test-harness/tests/controller/account.rs` (`test_renew_account_requires_owner`)
  - Fuzz: `tests/test-harness/tests/fuzz/privileged_auth_rejects.rs` (`set_position_manager`)
- Opinion: The three escalation questions in scope all answer **no** for delegate self-service. Gating is coherent with INV-AUTH-02 and the permissionless-entrypoint claims. Treat STRIDE’s “governance kill is immediate” as *effect-immediate after the Sensitive execute*, not *hot-key immediate*; owners must use `remove_delegate` for instant local kill.

---

## Scope questions (direct answers)

| Question | Answer |
|---|---|
| Can a delegate call `add_delegate` (grant itself or others)? | **No.** `set_account_delegate` calls `require_account_owner`, which admits only the live NFT holder. Unit test `require_account_owner_rejects_active_delegate` pins that a fully opted-in active manager still fails. |
| Can a delegate call `remove_delegate`? | **No.** Same owner gate. Non-owner / missing account → `AccountNotInMarket` (#13). |
| Can a delegate call `renew_account` (renew its mandate / account TTL authority path)? | **No.** `renew_account` also goes through `require_account_owner`. Harness `test_renew_account_requires_owner` rejects a non-owner. |
| Can a delegate call `set_position_manager` to (re)activate itself? | **No.** Entrypoint is `#[only_owner]` on the controller. Governance routes `SetPositionManager` as `sensitive_controller_operation` and excludes it from the guardian immediate path. Random callers rejected by `privileged_auth_rejects`. |
| Does use-time authority require a governance-approved active manager? | **Yes.** `is_owner_or_delegate` requires `get_position_manager(caller).is_some_and(|c| c.is_active)` **and** membership in `get_delegates(account_id, owner)`. |
| Does grant-time also require an active manager? | **Yes.** `set_account_delegate` asserts the same active check before writing, so a dormant pre-approval grant cannot be planted. |

“Renew” in INV-AUTH-02 means renewing *authority* (`renew_account` / re-granting). Permissionless `position-nft::renew` is rent charity for the NFT `Owner` TTL only; it does not restore or extend delegate rights.

---

## Defense inventory

### 1. Entrypoint macros and auth order

| Entrypoint | Macro | Body gate | Writes |
|---|---|---|---|
| `add_delegate` | `#[when_not_paused]` | `caller.require_auth()` → `require_account_owner` → active manager assert → storage add | Delegate map + optional `AccountDelegateEvent` |
| `remove_delegate` | none (pause-open) | `caller.require_auth()` → `require_account_owner` → storage remove / stale purge | Delegate map + optional event |
| `renew_account` | none | `caller.require_auth()` → `require_account_owner` → renew controller user keys + NFT renew | TTLs only |
| `set_position_manager` | `#[only_owner]` | Ownable owner (governance in production) | Shared `PositionManager(addr)` key; `is_active=false` deletes the entry |

`require_auth` runs before ownership resolution. Ownership is read live from the position NFT (`storage::account_owner` → `owner_of`), not from a cached controller field (INV-STOR-03).

Declared in `scripts/permissionless_entrypoints.txt` with `caller-auth` and the explicit claim that `require_account_owner` admits the owner alone — matches the body.

### 2. Triple gate at use time (`is_owner_or_delegate`)

```text
caller == owner
  OR (
    position_manager(caller).is_active
    AND get_delegates(account_id, owner).contains(caller)
  )
```

`get_delegates` returns empty unless `DelegateGrant.granted_by == owner` (current NFT holder). That is the third, implicit kill switch on NFT transfer — no separate revoke call required.

Owners short-circuit without a manager check: an owner need not be a position manager to act on their own account.

### 3. Grant-time contemporaneous activation

Comment and assert in `set_account_delegate` (add path): a grant to an address governance has not yet approved must not be stored, because a later `set_position_manager(true)` would otherwise arm it. Rejection uses `GenericError::NotAuthorized` (#44).

Consequence: the only way a manager becomes live on an account is (governance activate) **then** (owner `add_delegate`), in that order for the first arming. See residual on *reactivation* below.

### 4. Revocation paths (ordered by speed)

1. **Owner `remove_delegate`** — per-account, works while globally paused, no manager-active precondition.
2. **NFT transfer** — lazy: grants stamped by the prior owner read as empty for the new owner; prior delegates fail `is_owner_or_delegate` immediately.
3. **Governance `set_position_manager(manager, false)`** — protocol-wide: storage entry removed (`get_position_manager` → `None`); every account that still lists that address fails the active-manager leg at next use. Does **not** clear per-account grant lists.

### 5. Governance-approved manager registry

- Storage: `ControllerKey::PositionManager(Address)` → `PositionManagerConfig { is_active }`. Inactive writes delete the key (absent ≡ inactive).
- Controller mutator: `#[only_owner]` only.
- Governance: `AdminOperation::SetPositionManager` → `DelayTier::Sensitive` (`TIMELOCK_SENSITIVE_MIN_DELAY_LEDGERS = 12` as floor, then `max(min_delay, sensitive_floor)`). Listed in the `resolve` arm that `panic`s for non-timelocked application — guardian / immediate APIs cannot toggle managers.
- Activation and deactivation share the same Sensitive path.

### 6. Cap, events, pause asymmetry

- `MAX_DELEGATES = 16`; overflow → `RegistryCapReached` (#45).
- Idempotent add returns `false` (no event); successful mutate publishes `AccountDelegateEvent`.
- Pause blocks new grants (`#[when_not_paused]` on `add_delegate`) but never blocks revoke — matches endpoints doc and Spoof.3 R.2.

### 7. Stale-grant purge (resurrection defense)

If a prior owner’s grant still sits in storage after transfer, the new owner’s `remove_delegate` (even when the named delegate was never live for them) **deletes** the key. Test `remove_delegate_purges_stale_grant_preventing_resurrection` pins that returning the NFT to the original owner does not re-arm after that purge. `add_delegate` by the new owner also overwrites wholesale via `get_delegates` reading empty then `set_delegates` with the new `granted_by`.

---

## Residuals (not self-grant bugs)

### R1 — Reactivation re-arms stored grants (documented)

`deactivating_the_manager_kills_a_stored_grant_immediately` shows: `set_position_manager(false)` stops borrow/withdraw immediately while the account list still contains the address; `set_position_manager(true)` restores use **without** a new owner `add_delegate`.

This does **not** violate the “no dormant *pre-approval* grant” rule (grant-time still requires active). It means durable local revocation requires `remove_delegate` (or NFT transfer + purge), not reliance on a temporary governance deactivate. Threat-model / endpoints docs describe the dual switch; operators and owners should treat deactivate as a global pause of that manager, not as permanent erasure of every grant.

### R2 — NFT round-trip re-arm without intervening write (documented)

`DelegateGrant` type docs and `docs/reference/endpoints.md`: if the token returns to `granted_by` before the interim owner writes delegates, the original list becomes live again. Intended lazy-revoke property; not delegate self-service. Mitigation for a new owner who wants a clean slate: any delegate write (including a noop-style remove that purges stale storage).

### R3 — Governance “instant” kill vs Sensitive delay

STRIDE Elevation.4 / Spoof.3 call `set_position_manager(false)` an immediate global kill. On-controller effect is immediate once the entrypoint runs. Reaching that entrypoint through production ownership (governance timelock) waits the Sensitive tier. Instant incident response for a single account remains owner `remove_delegate` (pause-open). Cross-check for A020 / STRIDE wording accuracy; not an INV-AUTH-02 break.

### R4 — Economic power of a live delegate (accepted design)

Threat-model “A delegate has complete economic control of the account”: borrow/withdraw/strategies with optional `to`. Gating in this file does not bound that power; it only prevents unapproved or self-extended authority. User-facing docs must stay explicit.

### R5 — Test / formal coverage notes

- Strong unit coverage of owner gate, manager gate, NFT lazy revoke, stale purge, cap.
- Harness covers revoke vs deactivate mid-mandate and owner-only renew.
- No dedicated harness case where a *live delegate* invokes `add_delegate` / `remove_delegate` and expects `#13` (unit gate test is sufficient; optional hardening).
- No dedicated case that `add_delegate` to a never-approved address reverts `#44` at the client layer (logic is in `set_account_delegate`; worth a one-liner harness if expanding evidence).
- Certora: `market_guard_rules.rs` assumes `get_position_manager` none for a caller in one rule; no dedicated CVL rule found for “add_delegate requires owner” or “inactive manager cannot act”. Formal gap only — runtime enforcement is present.

---

## Attack sketches checked and blocked

| Sketch | Result |
|---|---|
| Delegate calls `add_delegate(self)` or adds a peer | Blocked: `require_account_owner` |
| Delegate calls `renew_account` to keep storage/NFT TTL under its control as “authority renew” | Blocked for authority path; NFT `renew` is permissionless rent only |
| Delegate calls `set_position_manager(true)` after governance deactivate | Blocked: `only_owner` / Sensitive governance |
| Stranger grants on foreign `account_id` | Blocked: not NFT owner |
| Owner grants address before governance approval | Blocked: grant-time active assert |
| Listed but deactivated manager borrows | Blocked: use-time `is_active` |
| Active manager never added to account list | Blocked: `get_delegates` membership |
| Former owner’s delegate after NFT transfer | Blocked: `granted_by` mismatch → empty list |
| Former owner’s grant resurrects after new owner `remove_delegate` then transfer back | Blocked: purge |
| Random address `set_position_manager` | Blocked: Ownable |
| Guardian immediate path toggles manager | Blocked: op resolution panics; not in `immediate.rs` |
| Add delegates beyond 16 | Blocked: `RegistryCapReached` |
| Add while paused | Blocked: `when_not_paused` |
| Remove while paused | Allowed (defense) |

---

## Cross-links

- Aligns with A003 (INV-AUTH-02 use paths consume `require_owner_or_delegate` / `is_owner_or_delegate` as mapped here).
- Aligns with A017 on `renew_account` owner-only TTL mutation; this finding adds that delegates cannot use it to renew mandate.
- A021 / A037 own storage-layout and delegate-map integrity depth; this finding only needs the grant stamp + purge behavior above.
- Possible STRIDE wording nit for A020: Sensitive delay on the governance global kill (R3).

---

## Verdict

**Defended.** A delegate cannot grant itself, grant peers, renew controller-side account authority, or (re)approve itself as a position manager. Governance approval is mandatory at grant and at every risk-increasing use; owners and NFT transfer supply instant local kills; pause cannot trap an owner into keeping a bad delegate. Remaining items are documented design residuals, not undefended escalation.
