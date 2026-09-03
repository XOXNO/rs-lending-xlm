# A064 — Asset listed-in-spoke and FreezePolicy flag checks

- Agent: A064
- Theme: T4 / T1
- Severity: medium (governance footgun on `no_seize`; otherwise defended)
- Status: defended (core gates) / partial (ADR-0008 draft amendment residual)
- Paths:
  - `contracts/controller/src/positions/mod.rs:190-204` (`FreezePolicy`)
  - `contracts/controller/src/positions/mod.rs:254-354` (`require_listed_unhalted_config`, `require_can_supply` / `require_can_borrow`, `validate_position_entry_gates`, `enforce_spoke_asset_flags`)
  - `contracts/controller/src/context/spoke.rs:40-101` (`cached_spoke_asset`, `require_spoke_asset*`, `require_listed_active_config`, `active_spoke`)
  - `contracts/controller/src/positions/supply.rs:113-120`, `:204-210`, `:254-260`
  - `contracts/controller/src/positions/debt.rs:52-59`, `:164-170`, `:229-235`, `:259-265`
  - `contracts/controller/src/positions/liquidation/plan.rs:29-36`, `:84-91`
  - `contracts/controller/src/positions/liquidation/apply.rs:42-48`, `:97-103`, `:149-155`, `:230-251`
  - `contracts/controller/src/strategies/legs.rs:162-168` (`net_settle` exit flags)
  - Callers of `require_can_supply` / `validate_position_entry_gates`: multiply, swap_collateral, migrate_blend, flash_position
- Defense: Entry requires hub active + non-deprecated spoke + spoke listing + `BlockOnEntry` (`paused`/`frozen`) + verb bit (`is_collateralizable` / `is_borrowable`). User exits and liquidation repay use `AllowOnExit` (`paused` only; missing listing = no-op). Seizure uses `SeizureLeg` (`no_seize` only; paused/frozen tolerated; missing listing = no-op). Plan and apply both re-check seizure/repay flags. Guardian flags ratchet one-way (A006).
- Gap: (1) Shipped ADR-0008 residual — `no_seize` does not couple to `frozen` / does not block `require_can_supply`, so guardians can strand liquidations for accounts that hold (or later supply) that collateral; `force_socialize_bad_debt` is the hatch. (2) Credit new receiver slot needs a live listing (`require_spoke_asset`) while victim debit tolerates delist — liveness only (A026/A052). (3) Standalone `enforce_spoke_asset_flags(BlockOnEntry)` no-ops on missing listing; safe today because every entry caller goes through `require_listed_*` first — keep that pairing. (4) `flash_loan` checks hub active only (no spoke listing); it does not mutate controller positions.
- Impact: Unlisted / wrong-spoke / inactive-hub / frozen / paused assets cannot open new controller exposure. Delisted holders stay exitable and seizable. Mis-set `no_seize` (without freeze) can make underwater accounts unliquidatable protocol-wide for holders of that collateral until owner socializes — availability / bad-debt latency, not silent share mint. Compromised guardian can only tighten flags (A006).
- Evidence: INV-HALT-02/03; ADR-0008 (+ draft amendment); STRIDE DoS.2; unit `contracts/controller/tests/positions/flags.rs`; harness `spoke.rs` pause/freeze, `liquidation_seize_modes.rs` pause/`no_seize` split, `liquidation_extreme.rs` paused debt, `deprecated_spoke_*`; peers A001, A006, A013, A026, A040, A051, A052.
- Opinion: The three-policy matrix in `positions/mod.rs` is the correct single source for INV-HALT-02 and is applied consistently on supply/borrow/withdraw/repay/strategies/liquidation. Do not fold seizure back under `paused`. The only substantive open item is the documented `no_seize`↔`frozen` coupling (Option C), not a missing listing check on money paths.

## 1. Method

1. Read COORDINATION + SEED; confirmed no prior `A064-*.md`.
2. Mapped `FreezePolicy` / `require_*` / `enforce_spoke_asset_flags` in `positions/mod.rs` and every production call site (supply, debt, liquidation plan/apply, strategies).
3. Traced listing helpers in `context/spoke.rs` (`require_listed_active_config` = `active_spoke` + `require_spoke_asset`).
4. Cross-checked ADR-0008, INV-HALT-02, STRIDE DoS.2, unit + harness pins, and peer findings A006/A040/A013/A026/A051/A052.
5. Looked for paths that mutate positions without listing or with the wrong policy.

## 2. Policy matrix (INV-HALT-02)

| Policy | Rejects | Tolerates | Missing listing row |
|---|---|---|---|
| `BlockOnEntry` | `paused`, `frozen` | `no_seize` | N/A on real entry paths — `require_listed_*` panics `#307` first |
| `AllowOnExit` | `paused` | `frozen`, `no_seize` | No-op (delisted stay exitable / repayable) |
| `SeizureLeg` | `no_seize` | `paused`, `frozen` | No-op (delisted stay seizable) |

Rationale (ADR-0008 / comments on `FreezePolicy`): seizure is pro-rata over **all** collateral; gating it on `paused` would turn one listing halt into a protocol-wide liquidation halt (Aave CS-AAVE4-002 class). Debt repay on liquidate is liquidator-chosen, so `paused` on a named debt is opt-in DoS only (STRIDE DoS.2).

`no_seize` is intentionally ignored by entry and exit policies (`no_seize_does_not_block_entry_or_exit` unit + harness).

## 3. Listing + entry stack

```text
require_can_supply / require_can_borrow
  └─ require_listed_unhalted_config
       ├─ cache.require_hub_active(hub_id)          # #43 HubNotActive
       ├─ cache.require_listed_active_config
       │    ├─ active_spoke(spoke_id)               # #301 SpokeDeprecated
       │    └─ require_spoke_asset → AssetConfig     # #307 AssetNotInSpoke
       └─ enforce_spoke_asset_flags(BlockOnEntry)   # #315 / #316
  └─ is_collateralizable / is_borrowable            # #104 / #107
```

`validate_position_entry_gates` adds bulk position limits, then runs the above for every aggregated hub asset using **`account.spoke_id`** (cross-spoke listing does not count).

Used on:

| Flow | Gate |
|---|---|
| `process_deposit` / ordinary supply | `validate_position_entry_gates(Deposit)` |
| `process_borrow` | `validate_position_entry_gates(Borrow)` |
| `borrow_into_controller` (strategies) | same Borrow gates |
| multiply / swap_collateral / migrate / flash_position collateral | `require_can_supply` (± bulk gates) |

`settle_supply` re-reads `require_spoke_asset` for stamp decimals / position create — listing already proven; flags already enforced. Caps fire later via `apply_spoke_entry` → `require_spoke_asset_config` (INV-HALT-03).

Deprecated spoke: new exposure closed; exits and liquidation remain open (harness `deprecated_spoke_*`; A013 `AllowDeprecated` for Credit(0)).

## 4. Exit and seizure call sites

### 4.1 User / strategy exits — `AllowOnExit`

| Site | Notes |
|---|---|
| `supply::settle_withdraw` | Per aggregated leg before pool withdraw |
| `supply::execute_withdrawal` | Strategy single-leg withdraw |
| `debt::settle_debt` Repay arm | Before measured pull + `apply_repay_batch` |
| `debt::execute_repayment` | Strategy single-leg repay |
| `strategies/legs::net_settle_collateral_against_debt` | Same-asset net settle |
| Liquidation plan repay loop | Named debt assets |
| `apply_liquidation_repayments` | Defense-in-depth re-check |

Frozen listings may exit; paused may not. Delisted: flag helper no-ops; position still required (`get_*_position_or_panic`). Hub inactive is **not** re-checked on bare withdraw/repay — intentional exit liveness when a hub is deactivated (entry still needs hub active).

### 4.2 Seizure — `SeizureLeg`

| Site | Notes |
|---|---|
| `build_liquidation_plan` after `calculate_seized_collateral` | Only non-dropped legs; zero/floor-empty legs `continue` before push, so tiny dust does not trip `#318` |
| `apply_liquidation_seizures` (Transfer) | Re-check before pool withdraw |
| `apply_liquidation_share_credit` (Credit) | Re-check before share move |

Paused/frozen collateral remains seizable (harness `a_paused_collateral_can_still_be_seized*`). `no_seize` blocks both modes (`no_seize_blocks_the_seizure_leg_in_both_modes`). Ordinary withdraw still works under `no_seize`.

Credit credit path **skips** `require_can_supply` / caps / `is_collateralizable` by design (apply.rs comments; seizure ≠ new supply). New empty receiver slot still needs `require_spoke_asset` to stamp risk — delisted collateral cannot open a fresh Credit slot (A052 residual; Transfer unaffected).

## 5. What is not gated here (and why)

| Surface | Behaviour | Verdict |
|---|---|---|
| Global pause | Separate `#[when_not_paused]` (A001 / INV-HALT-01) | Orthogonal |
| Guardian ratchet | `require_flag_ratchet` in `config/asset.rs` (A006) | Complementary |
| `flash_loan` | Hub active only; pool flash; no controller position mutate | Out of position-gate scope; residual note |
| Bad-debt absorb | Owner/keeper paths; not user freeze policies | Expected |
| `restamp_listed_supply_ltv` | Skips missing listing rows | Keep stale stamps; not an entry bypass |

## 6. Gaps and residuals

### G1 — `no_seize` without `frozen` (ADR-0008 draft) — severity: medium / status: partial

Shipped semantics: `no_seize` blocks seizure only. Guardians can set it independently of `frozen` (`set_spoke_asset_flags_tightens_no_seize_independently`). Because seizure is all-or-nothing over collateral, one `no_seize` leg reverts the whole liquidation. Users can still **supply** that asset (`BlockOnEntry` ignores `no_seize`), growing the unliquidatable set. Existing holders are already stuck.

Documented options A/B/C are not implemented; recommended Option C couples setter so `no_seize ⇒ frozen`. Until then, `force_socialize_bad_debt` is the operator hatch. This matches STRIDE DoS.2 residual and threat-model listing-flag note — **not** a missing `enforce_spoke_asset_flags` call.

### G2 — Credit delist new-slot liveness — severity: low / status: residual

Victim debit: listing optional. Receiver new slot: listing required. Workarounds: Transfer, or Credit to a receiver that already holds the hub. Agrees with A026/A052.

### G3 — Helper footgun if listing check is omitted — severity: info

`enforce_spoke_asset_flags(..., BlockOnEntry)` alone allows a missing listing (unit `missing_spoke_asset_is_noop`). Production entry always pairs with `require_listed_unhalted_config`. Future callers must keep that pairing; do not “optimize” by calling the flag helper alone for entries.

### G4 — Flash loan listing — severity: info

No spoke listing on `process_flash_loan`. Does not write account positions; pool market params still apply. Track under flash/pool scopes if desired, not as a supply/borrow freeze bypass.

## 7. Attack / misuse sketches (expected outcomes)

| Attempt | Outcome |
|---|---|
| Supply/borrow unlisted hub asset | `#307 AssetNotInSpoke` |
| Supply/borrow on deprecated spoke | `#301 SpokeDeprecated` |
| Supply/borrow inactive hub | `#43 HubNotActive` |
| Supply/borrow frozen | `#316`; withdraw/repay OK |
| Supply/borrow/withdraw/repay paused | `#315` |
| Liquidate naming paused debt | `#315` (opt-in) |
| Liquidate account whose collateral is paused | Seizure proceeds |
| Liquidate when any planned seize leg has `no_seize` | `#318`; whole tx reverts |
| Withdraw after delist (usage drained → remove listing) | Flags no-op; exit works if position remains |
| Credit seize delisted onto empty receiver | `#307` on stamp |
| Cross-spoke listed asset on account spoke N | `#307` against N’s registry |

## 8. Evidence map

| Claim | Pin |
|---|---|
| Policy matrix | `positions/mod.rs` `FreezePolicy` + `enforce_spoke_asset_flags`; INV-HALT-02; ADR-0008 |
| Entry stack | `require_listed_unhalted_config`; unit `require_can_supply_*`, `require_can_borrow_*`, inactive hub |
| Pause/freeze e2e | harness `test_paused_spoke_asset_blocks_supply_and_withdraw`, `test_frozen_spoke_asset_blocks_entries_but_allows_exit` |
| Seizure split | harness `a_paused_collateral_can_still_be_seized*`, `no_seize_blocks_*`, `no_seize_does_not_block_ordinary_withdrawal`; unit pause/freeze/`no_seize` matrix |
| Paused debt opt-in | harness `a_paused_debt_asset_is_opt_in_*`, `test_paused_debt_leg_rejects_liquidation` |
| Delist exit/seize | unit `missing_spoke_asset_is_noop*` |
| Deprecated liveness | harness `deprecated_spoke_*`; A013 |
| Ratchet | A006; `asset_flags.rs` |
| Allowed tokens | A040 (this file deepens freeze/seize matrix) |

No Certora rules call `enforce_spoke_asset_flags` by name; coverage is unit + harness + docs. That is acceptable for discrete flag logic; G1 is a product decision, not a prover hole.

## 9. Peer alignment

| Peer | Relation |
|---|---|
| A001 | Global pause orthogonal; liquidate/withdraw/repay stay open |
| A006 | Flag ratchet complements these read-side gates |
| A013 | Credit receiver / deprecated spoke create |
| A026 / A051 / A052 | Liquidation apply order; Transfer/Credit; SeizureLeg + Credit listing residual |
| A040 | High-level allowed-token claim; A064 owns FreezePolicy thoroughness |
| A024 / A025 / A022 / A023 | Storage peers cite these gates; no contradiction |

## 10. Opinion

Treat `FreezePolicy` as load-bearing API: any new monetary verb must pick exactly one policy and, for entries, go through `require_listed_unhalted_config` (or `require_can_supply` / `require_can_borrow`). Do not reintroduce `paused` on seizure. Prioritize deciding ADR-0008 Option C (`no_seize` ⇒ `frozen`) over adding more call-site checks — the call graph for listing and flags on supply/borrow/seize is already complete and correctly asymmetric.
