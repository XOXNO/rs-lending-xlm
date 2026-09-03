# A039 — Accumulator / revenue claim storage side effects

- Agent: A039
- Theme: T2 (storage mutations); money-path measurement touchpoints owned by A015 / A058
- Severity: info
- Status: defended
- Paths:
  - `contracts/controller/src/keepers.rs:26–38,94–141` (`claim_revenue`, `claim_revenue_for_asset`)
  - `contracts/controller/src/lib.rs:381–387,577–582` (`#[when_not_paused]` claim; `#[only_owner]` `set_accumulator`)
  - `contracts/controller/src/storage/protocol.rs:74–84` (`try_get_accumulator` / `set_accumulator`)
  - `contracts/controller/src/config/registry.rs:32–36` (owner write + `UpdateAccumulatorEvent`)
  - `contracts/controller/src/context/mod.rs:39–44` + `context/pool.rs:9–17` (`Cache::new` instance TTL; pool address memo)
  - `contracts/controller/src/external/pool.rs:126–132` (`pool_claim_revenue_call`)
  - `contracts/controller/src/payments.rs:14–24` + `common/src/token.rs:19–34` (measured inbound Δ / measured forward)
  - `contracts/controller/src/events/revenue.rs:16–24` (`ClaimRevenueEvent`)
  - `contracts/pool/src/lib.rs:236–244` + `ops/revenue.rs:19–55` + `cache/shares.rs:54–75` (pool burn/debit/pay owner)
  - `common/src/types/controller.rs:540–547` (`ControllerKey::Accumulator`)
  - `scripts/permissionless_entrypoints.txt:69`
- Defense: Permissionless `claim_revenue` **reads** the instance `Accumulator` pointer and never writes it. Controller durable value writes on this path are **empty** — no account / supply / debt / delegates / spoke usage / hub / protocol-config mutation. The only controller storage *lifecycle* side effect is instance TTL renew via `Cache::new`. Pool market books (`revenue`, `supplied`, `cash`, indexes via sync) mutate under pool owner-gated `claim_revenue`; cash pays the pool Ownable owner (controller). Controller then forwards **measured** receipt to the stored accumulator address. Unset accumulator fail-closes `#211` before any pool call. Owner-only `set_accumulator` is the sole writer of the pointer.
- Gap: (1) No public clear/unset of `Accumulator` — only overwrite (operational; claim fail-closes if never set). (2) `set_accumulator` accepts any `Address` with no contract-class check (A009 residual; governance Std delay intended). (3) Accumulator is re-read **per asset** inside the loop — correct freshness, but a hypothetical mid-tx owner rewrite (reentrancy + compromised owner) could split a multi-asset batch across two destinations; not reachable by a stranger keeper. (4) Shared A015/A062 — unbounded `assets` Vec; empty Vec still renews instance TTL. (5) Observational — `ClaimRevenueEvent.amount` is controller inbound Δ; forward FoT haircut is not separately persisted (A058).
- Impact: A keeper cannot rewrite protocol config, positions, usage, or the accumulator pointer; cannot redirect revenue; cannot mint controller shares/debt; cannot drain pre-existing controller dust into a claim. Successful claim only (a) renews controller (+ pool) instance TTL, (b) decreases pool revenue/supply shares and cash for listed hubs, (c) moves tokens pool→controller→accumulator, (d) emits events. Blast radius of a wrong accumulator address is **all future claimed revenue** until owner retargets — governance/owner trust, not a claim-path storage hole.
- Evidence: INV-AUTH-03, INV-ACCT-03, INV-ACCT-06, INV-STOR-01; threat-model “Permissionless maintenance” / “Revenue goes only to the configured accumulator”; STRIDE I10; peers A015, A016, A029, A034, A058, A009; unit `claim_revenue_without_accumulator_panics`, storage protocol accumulator round-trip; harness `tests/pool/revenue.rs`, `controller/outbound_transfer_measurement.rs`, `admin_config.rs` permissionless claim.
- Opinion: Wave-2 answer is clean: **claim is a read-pointer + cross-contract burn/pay + token forward, not a controller ledger rewrite.** Keep `ControllerKey::Accumulator` instance-scoped and owner-written only. Do not add a keeper-visible claim counter or per-claim storage row — events are the lifetime sum by design. Optional hygiene only: Vec bounds (A015/A062), propose-time address class on `SetAccumulator` (A009).

---

## Method

1. Read `shared/COORDINATION.md`, `SEED.md`, A015 (keeper bounds / recipient), A016 (recapitalize measure — adjacent keeper, not claim).
2. Trace `Controller::claim_revenue` → `keepers::claim_revenue` → `claim_revenue_for_asset` end-to-end; inventory every `storage::` / `Cache` / pool FFI / token call.
3. Trace sole writer `set_accumulator` (registry + `#[only_owner]` + `renew_then!`) and confirm no clear/remove API.
4. Trace pool `ops::revenue::{apply,accounting}` durable commits (`burn_claimable_revenue`, `debit_cash`, `commit`, `transfer_out` to owner).
5. Cross-check A029 (protocol keys), A034 (TTL), A058 (measurement), A009 (admin pointer), A022-style non-write discipline.
6. No novel critical controller storage corruption or pointer-hijack via the permissionless claim path.

Out of scope as primary claims: Vec length hygiene (A015/A062), token-lying beyond measured forward (A055/A058), pool IRM/accrual correctness (IDX wave), governance delay floors (A009).

---

## 1. Scope boundary vs peers

| Peer | Owns | This file adds |
|---|---|---|
| A015 | Auth, pause, flash, **who receives** revenue, Vec bounds | Storage write-set of claim + Accumulator key lifecycle |
| A016 | `recapitalize` measured credit into pool | Contrast only — claim does not write payer credit |
| A029 | Protocol instance/persistent key map | Claim does not use any of those writers except **read** Accumulator / Pool |
| A034 | TTL taxonomy; keepers call `Cache::new` | Claim’s sole controller durable *lifecycle* effect is instance renew |
| A058 | Measurement primitives; claim event = inbound Δ | Storage implication: no controller balance ledger key exists |

---

## 2. Call graph

```
Controller::claim_revenue(caller, assets)     #[when_not_paused]  lib.rs:384–387
  └─ keepers::claim_revenue                   keepers.rs:28–38
       ├─ caller.require_auth()
       ├─ require_not_flash_loaning             #400 if flash ongoing
       ├─ Cache::new                            RENEW controller instance TTL
       └─ for hub_asset in assets:
            └─ claim_revenue_for_asset          keepers.rs:96–141
                 ├─ try_get_accumulator         READ instance Accumulator
                 │    └─ None → panic #211      BEFORE pool call
                 ├─ cached_pool_address         READ instance Pool (memoize)
                 ├─ token.balance(controller)   baseline
                 ├─ pool_claim_revenue_call     FFI → pool #[only_owner] claim
                 │    └─ ops::revenue::apply
                 │         ├─ renewed_market    pool instance TTL + sync/accrue
                 │         ├─ burn_claimable_revenue  RAM: revenue/supplied ↓
                 │         ├─ util + solvency guards
                 │         ├─ debit_cash        RAM: cash ↓
                 │         ├─ cache.commit()    WRITE pool market state
                 │         ├─ transfer_out(owner=controller)  if amount ≠ 0
                 │         └─ emit_market_state
                 ├─ balance_delta_since         measured inbound (discard pool report)
                 └─ if received > 0:
                      ├─ transfer_amount_measured  controller → accumulator
                      └─ ClaimRevenueEvent.publish   observational only
```

Return value: `Vec<i128>` of per-asset measured receipts (including zeros). No storage of that Vec.

---

## 3. Durable writes inventory

### 3.1 Controller contract storage

| Phase | Storage class | Key / target | Mutated? | Notes |
|---|---|---|---|---|
| Entry | Instance TTL | entire instance | **extend only** | `Cache::new` → `renew_controller_instance` |
| Per asset | Instance value | `ControllerKey::Accumulator` | **read only** | `try_get_accumulator`; panic if absent |
| Per asset | Instance value | `ControllerKey::Pool` | **read only** (once memoized) | `get_pool` via `cached_pool_address` |
| Per asset | Persistent user | AccountMeta / Supply / Borrow / Delegates | **never** | No account id in ABI |
| Per asset | Persistent shared | Spoke / SpokeAsset / SpokeUsage / Hub / … | **never** | No listing/usage touch |
| Per asset | Instance value | PositionLimits, aggregators, NFT, MinBorrow… | **never** | |
| Per asset | Temp | FlashLoanOngoing | **never set** | Guard read only at entry (A007/A030) |
| Events | — | `revenue:claim` | publish if `received > 0` | Not durable protocol state |
| Token | SAC balances | controller / accumulator | transfer | External to controller keys |

**Controller durable value-write count on `claim_revenue`: zero.**

### 3.2 Owner path that *does* write Accumulator (not the keeper)

| Entrypoint | Gate | Write | Event |
|---|---|---|---|
| `set_accumulator(addr)` | `#[only_owner]` + `renew_then!` | `instance.set(Accumulator, addr)` | `UpdateAccumulatorEvent` (`config`,`accumulator`) |

No `clear_accumulator`, no `persistent().remove(Accumulator)`, no view getter in `ControllerInterface` (readable only via as-contract tests / future admin surfaces). Once set, claim always has a destination until owner overwrites.

### 3.3 Pool contract storage (FFI side effect)

| Step | Effect |
|---|---|
| `renewed_market` | Pool instance TTL renew; load params/state; `global_sync` may advance indexes / mint revenue shares into `revenue`/`supplied` |
| `burn_claimable_revenue` | Burns up to `min(cash, unscaled revenue)` shares from `revenue` and `supplied` (ceil pro-rata if partial) |
| Guards | `require_utilization_below_max`, `require_solvent_withdraw_state` — revert ⇒ no commit |
| `debit_cash` | Accounting cash decreases by net transfer |
| `commit` | `storage::write_state` persists full `PoolStateRaw` for that hub |
| `transfer_out(owner)` | SAC: pool → controller (Ownable owner). Skipped when `actual_amount == 0` |
| Snapshot event | Pool `market` / `batch_state_update` (outstanding revenue field decrements — not a cumulative counter) |

Pool `claim_revenue` is `#[only_owner]`; deploy wiring makes the controller the owner (INV-AUTH-01). The keeper never becomes the pool payee.

### 3.4 Token ledger (not controller keys)

1. Pool → controller: amount decided by pool burn/cash cap; controller measures Δ.
2. Controller → accumulator: `transfer_amount_measured` pushes `received` and returns recipient Δ (discarded); only runs if `received > 0`.
3. Pre-claim controller inventory outside the measured window is untouched (F-8 / harness dust test).

---

## 4. Explicit non-writes (load-bearing absences)

1. **No `storage::set_accumulator` on the claim path** — recipient is not caller-controlled and is not rewritten by success/failure of a claim.
2. **No position / meta / delegate / spoke-usage writes** — claim cannot create foreign risk or restamp LTV/LT (contrast `update_account_threshold` in A015).
3. **No controller-side revenue accrual counter** — lifetime revenue is defined as outstanding pool revenue (index-valued) + Σ `ClaimRevenueEvent.amount` (`events/revenue.rs` docs). Storage does not double-book.
4. **No Cache event buffers / `emit_position_batch`** — claim does not go through position finalize (A033 order rules N/A).
5. **No `put_market_index` / pool sync memo fill required** — claim only needs pool address; does not pollute controller index Cache as SoT (A038/A094).
6. **Pool reported `PoolAmountMutation` discarded** (`let _ = pool_claim_revenue_call`) — prevents trusting intent over measured receipt (INV-ACCT-03).
7. **Zero receipt ⇒ no forward transfer, no `revenue:claim` event** — avoids indexer spam; pool may still have emitted market snapshot / performed sync+commit with zero burn.
8. **Caller address never used as transfer destination** — event metadata only.

---

## 5. Accumulator key lifecycle

### 5.1 Storage class

`ControllerKey::Accumulator` lives in **instance** storage alongside Pool, aggregators, NFT, limits (`protocol.rs:75–84`; A029). It rides the controller instance TTL window (5d threshold / 180d bump). Claim’s `Cache::new` renews that whole instance, so successful (or even empty-loop) claims help keep the Accumulator entry alive with the rest of protocol config — INV-STOR-01 lifecycle, not a value mutate.

### 5.2 Read semantics on claim

```102:103:contracts/controller/src/keepers.rs
    let accumulator = storage::try_get_accumulator(env)
        .unwrap_or_else(|| panic_with_error!(env, OracleError::NoAccumulator));
```

- Fail closed **before** `pool_claim_revenue_call` — no stranded “claimed to controller, nowhere to forward” durable pool burn without a configured destination in the same tx (atomicity: if somehow ordered wrong it would still roll back; the code orders correctly).
- Error `#211` `OracleError::NoAccumulator` (naming quirk; not an oracle fault).
- Unit lock: `contracts/controller/tests/entrypoints.rs::claim_revenue_without_accumulator_panics`.

### 5.3 Write semantics (owner only)

```32:36:contracts/controller/src/config/registry.rs
pub(crate) fn set_accumulator(env: &Env, addr: Address) {
    storage::set_accumulator(env, &addr);
    UpdateAccumulatorEvent { accumulator: addr }.publish(env);
}
```

- No validation that `addr` is a contract, treasury multisig, or non-banned address (A009 G4).
- Overwrite is silent w.r.t. in-flight claims in other txs; same-tx reentrancy would require owner auth on `set_accumulator`.
- Governance maps this to Standard-delay `SetAccumulator` when owner = governance; controller itself has no delay (A009).

### 5.4 Per-asset re-read

`claim_revenue` loops assets and calls `claim_revenue_for_asset` each time; each iteration re-reads Accumulator. Properties:

| Property | Verdict |
|---|---|
| Stale memo of accumulator across assets? | No — fresh read each asset |
| Keeper can change pointer between assets? | No — no writer on path |
| Owner mid-batch retarget via reentrancy? | Requires owner-auth `set_accumulator` during a token hook; exotic; would split destinations across assets in one tx |
| Snapshot-once at loop start better? | Optional hardening; current design prefers latest governance pointer |

Not a stranger-storage bug.

---

## 6. Ordering, atomicity, and partial batches

### 6.1 Single-asset happy path

1. Renew controller instance TTL.
2. Require Accumulator present.
3. Snapshot controller token balance.
4. Pool sync → burn → guard → debit → **commit** → transfer to controller.
5. Measure Δ; if `> 0`, measured forward to Accumulator + event.

If step 5’s forward panics (e.g. measured push overflow hygiene), Soroban aborts the transaction and rolls back pool commit + inbound transfer. No “burn without payout” durable residue across txs.

### 6.2 Multi-asset batch

Assets are claimed **in order**. If asset *k* reverts (unknown market, util ceiling, insolvency, missing accumulator on a later iteration after a hypothetical clear — clear does not exist), the **entire** invocation reverts: prior assets’ pool commits and token hops unwind.

Duplicates: first claim drains claimable cash/shares; later duplicate typically returns `0` (no second forward, no second `revenue:claim`). No double storage burn beyond what pool state already reflects after the first leg in the same successful tx.

### 6.3 Zero-revenue markets

Pool `accounting` still syncs, commits (possibly index-only updates), and emits market state; `transfer_out` skipped; controller sees `received == 0` → no Accumulator transfer, no claim event. Controller storage still unchanged aside from the entry instance renew.

### 6.4 Pause / flash

- Global pause: `#[when_not_paused]` blocks claim (A001/A015). Accumulator pointer remains writable by owner during pause (`set_accumulator` has no pause gate — intentional admin liveness).
- Flash: entry `require_not_flash_loaning`; claim does not set the temp flag. Listed-token reentrancy into other mutators is the shared A007 residual; it still cannot rewrite Accumulator without owner.

---

## 7. Side-effect classes (what “storage side effects” means here)

| Class | Present on claim? | Risk if mis-designed |
|---|---|---|
| Protocol pointer rewrite | **No** | Redirect treasury |
| Account position rewrite | **No** | Foreign risk / HF |
| Spoke usage rewrite | **No** | Cap desync |
| Pool market rewrite | **Yes** (intended) | Insolvent claim — blocked by INV-ACCT-06 guards |
| Instance TTL bump | **Yes** | Fee grief via empty Vec — low (A015/A034) |
| Event emission | **Yes** (conditional) | Indexer over-count — mitigated by measured amount + zero silence |
| Token balances | **Yes** | Redirect — blocked by fixed Accumulator + measure |

The Wave-2 question “does claim smuggle controller ledger writes?” → **No.**

---

## 8. Attack / confusion scenarios (storage-focused)

| Scenario | Storage outcome | Verdict |
|---|---|---|
| Stranger keeper claims | Reads Accumulator; no controller value write; pool burns; tokens → Accumulator | Defended |
| Stranger tries to pass recipient | No ABI field | Defended |
| Accumulator unset | `#211` before pool | Defended |
| Claim with empty `assets` | Only instance TTL renew | Low residual |
| Duplicate hubs in one Vec | Second leg ~0; no double treasury credit beyond first burn | Defended |
| Under-delivering FoT token | Forward measured Δ; dust not raided; event = inbound Δ | Defended (A058) |
| Compromised owner `set_accumulator(attacker)` | Instance pointer overwrite; future claims follow | Owner trust (A009/A015) |
| Keeper hopes claim writes a “paid” flag to block others | No such key — claims are race-on-pool-state only | By design |
| Mid-batch failure after asset 1 success | Full tx rollback | Defended |
| Reenter claim from token hook | May attempt nested claim; still pays Accumulator; cannot set pointer | No storage hijack |
| Confuse pool owner payee with Accumulator | Pool pays controller; controller forwards | Two-hop defended |

---

## 9. Invariant / threat-model mapping

| Claim | Mapping | Live verdict on this path |
|---|---|---|
| Permissionless maintenance cannot create foreign risk | INV-AUTH-03; threat-model row; permissionless_entrypoints.txt:69 | Holds — no account/usage writes; recipient fixed |
| Revenue claims remain solvent | INV-ACCT-06 | Holds — pool guards + cash-capped burn before commit |
| Credit / forward equals measured receipt | INV-ACCT-03 / F-8 | Holds — discard pool report; measure then forward |
| Persistent/instance lifecycle discipline | INV-STOR-01 | Holds — claim renews instance; does not orphan user keys (touches none) |
| Protocol config single-writer | A029 / INV-AUTH-01 | Holds — Accumulator owner-only; claim is reader |
| STRIDE I10 | Keeper ↔ Controller ↔ Pool | Holds — auth + destination binding + pool owner payee |

---

## 10. Tests / evidence pointers

| Lock | Location |
|---|---|
| Unset Accumulator panics `#211` | `contracts/controller/tests/entrypoints.rs::claim_revenue_without_accumulator_panics` |
| Accumulator set/get round-trip | `contracts/controller/tests/storage/protocol.rs`; `entrypoints.rs::admin_setters_persist_and_are_readable` |
| Route pool → controller → Accumulator; controller retains nothing | `tests/test-harness/tests/pool/revenue.rs::test_claim_revenue_routes_through_controller_to_accumulator` |
| Measured forward; dust intact; event = measured | `outbound_transfer_measurement.rs::claim_revenue_forwards_the_measured_amount_and_leaves_controller_dust_intact` |
| Zero claim silent on `revenue:claim` | `claim_revenue_emits_nothing_when_there_is_no_revenue` |
| Third-party caller may invoke | `revenue.rs::test_permissionless_revenue_endpoints`; `admin_config.rs` |
| Flash reentry refused | `meta/reentrancy_matrix.rs` |
| Pool util / insolvency / partial cash burn | `pool/tests/flows.rs`, `pool_revenue_edge.rs`, `pool_coverage.rs` |

---

## 11. Residuals (accepted / low)

1. **Owner-trusted Accumulator pointer** — retarget steals future revenue; Standard gov delay when wired correctly; no controller-side address class check (A009).
2. **No unset** — cannot return to `#211` without overwrite to a new address; operational, not a claim bug.
3. **Instance TTL on empty/no-op claims** — permissionless rent bump; fee-funded (A034/A015).
4. **Event vs forward FoT** — storage-free observational asymmetry (A058); indexers must not treat event as Accumulator final balance under FoT listings.
5. **Pool sync commit on zero claim** — pool storage may advance indexes without controller Accumulator movement; expected accrual maintenance coupled into claim.
6. **Unbounded assets Vec** — resource/UX; not a storage corruption vector (A062).

---

## 12. Verdict

| Question | Answer |
|---|---|
| Does `claim_revenue` write `ControllerKey::Accumulator`? | **No** — read-only; sole writer is owner `set_accumulator` |
| Does it write any controller account / usage / config value keys? | **No** |
| What controller storage side effect remains? | **Instance TTL renew** via `Cache::new` (+ event publish, non-durable) |
| What external durable effects remain? | Pool market state burn/debit/commit; SAC balances pool→controller→accumulator |
| Can a keeper redirect or strand revenue via storage tricks? | **No** — unset fail-closes; recipient fixed; measured forward; tx atomicity |
| Novel critical Wave-2 gap? | **None** |

Overall: **defended**. Accumulator configuration is a narrow owner-written instance pointer; the permissionless claim path is intentionally storage-light on the controller and correctly pushes all entitlement mutation into the pool’s revenue books plus token transfers to that pointer.
