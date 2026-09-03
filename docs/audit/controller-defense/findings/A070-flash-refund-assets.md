# A070 — `refund_assets` uniqueness and allowlist in `flash_position`

- Agent: A070
- Theme: T4
- Severity: info
- Status: defended
- Paths:
  - `contracts/controller/src/strategies/flash_position.rs:106-114,228-269,388-400` (`validate_refund_assets`, call site, `refund_listed_assets`)
  - `contracts/controller/src/strategies/flash_position.rs:125-152` (pre-callback snapshot + post-deposit refund order)
  - `contracts/controller/src/strategies/mod.rs:41-55` (`snapshot_balances` de-dup)
  - `contracts/controller/src/strategies/legs.rs:220-233` (`refund_controller_balance_delta`)
  - `contracts/controller/src/context/spoke.rs:76-83` (`require_listed_active_config`)
  - `contracts/controller/src/storage/spoke.rs:27-32` (`ControllerKey::SpokeAsset(spoke_id, HubAssetKey)`)
  - Entrypoint: `contracts/controller/src/lib.rs:195-223`; interface `interfaces/controller/src/lib.rs`
- Defense: Before the flash guard / callback, `validate_refund_assets` enforces (1) `len() ≤ max_supply_positions`, (2) no duplicate `Address`, (3) each address listed under the **debt hub** in the account’s active spoke (`require_listed_active_config` → else `AssetNotInSpoke` #307), (4) no overlap with any declared collateral’s underlying asset. Empty list is allowed. Execution refunds only positive balance deltas vs a pre-callback baseline to `caller`, never gross controller inventory. Unlisted caller-chosen contracts never reach post-guard `token::Client`.
- Gap: (1) Listing key is `HubAssetKey { hub_id: debt.hub_id, asset }` because the vec is bare `Address` — an asset listed only under another hub in the same spoke is rejected (stricter than “any spoke listing”; money-safe UX quirk; `endpoints.md` omits the hub-key detail). (2) Refund allowlist does **not** re-check pause/freeze/`is_collateralizable` (intentional: cash return, not position entry). (3) No dedicated harness for over-length `refund_assets` or multi-hub listing asymmetry (duplicates / overlap / unlisted are covered). (4) Post-guard refund transfers remain a listing-trust residual (A007/A045) — out of uniqueness/allowlist novelty.
- Impact: Duplicate or overlapping declarations fail closed (`InvalidPayments` #100-class). Arbitrary/unlisted token addresses fail closed (`AssetNotInSpoke`) before any refund-leg `balance`/`transfer`. No path found that double-pays a delta, credits unlisted tokens as positions, or sweeps pre-existing controller balances via `refund_assets`. Multi-hub keying can only deny a refund declaration, not invent credit.
- Evidence: ADR-0020; INV-STRAT-04 / INV-ACCT-03 / INV-FLASH-02 (adjacent); `docs/reference/endpoints.md` flash_position declaration lists; errors #307 / `InvalidPayments`; harness `test_flash_position_rejects_duplicate_refund_assets`, `test_flash_position_rejects_refund_overlap`, `test_flash_position_rejects_unlisted_refund_asset`, `test_flash_position_refunds_undeclared_push`, `test_flash_position_returning_debt_token_does_not_repay`; peers A007, A040, A045, A054, A055. Cross-ref A062 (vec bounds/dupes when filed).
- Opinion: Uniqueness and allowlist for `refund_assets` are defended and are the correct trust boundary for post-guard `token::Client` use. Keep Address-level uniqueness + collateral-partition + listed-active checks before the callback. Do not loosen listing to “any hub in spoke” without an explicit product decision; do not add a gross-balance sweep. Optional docs hygiene: state that refund listing is keyed by `debt.hub_id`.

## Scope and method

1. Read `shared/COORDINATION.md`, `SEED.md`, README finding format; confirmed `A070-flash-refund-assets.md` absent.
2. Traced `lib.rs::flash_position` → `process_flash_position` → `validate_refund_assets` → in-guard `snapshot_balances(refund_assets)` → `refund_listed_assets` → `refund_controller_balance_delta`.
3. Compared refund rules to `validate_collaterals` (HubAssetKey + supply flags) and to `require_listed_unhalted_config` / `require_can_supply` used on position entry.
4. Cross-checked A045 §4 (money-flow owner of refunds; defers allowlist detail here), A040 (listing gate), A054 (refund families), A007 §5 (post-guard listed-token residual), ADR-0020, endpoints.md, INV-STRAT-04.
5. Out of primary claim: collateral minima / deposit measure (A045), flash-guard lifecycle (A030/A007), FOT on refund transfer (A055), general amount validators (A061), other strategies’ leftover refunds (A047/A050/A054).

---

## 1. Why `refund_assets` exists

Soroban cannot enumerate token balances on an address. After `execute_flash_position`, the controller can only measure assets it was told to watch. Two declaration lists serve that role (`endpoints.md`):

| List | Type | Role |
|---|---|---|
| `collaterals` | `Vec<(HubAssetKey, i128)>` | Measured Δ → deposit as supply (with floors) |
| `refund_assets` | `Vec<Address>` | Measured Δ → transfer to `caller` (no position credit) |

`refund_assets` recovers leftovers that must **not** become collateral: unspent debt token, swap dust, over-delivery of a non-collateral listed asset. Undeclared pushes stay stranded and unstealable (baseline discipline; A045/A054). ADR-0020: “Caller-listed `refund_assets` can recover them.”

Trust dual-duty (endpoints.md): each entry costs balance reads; both lists are capped at `max_supply_positions`. Each entry is validated against spoke listing **before** the callback so post-guard `token::Client` never targets a caller-chosen arbitrary contract.

---

## 2. Validation surface (`validate_refund_assets`)

Called once, **before** `with_flash_guard`, after account load and `validate_collaterals`:

```228:269:contracts/controller/src/strategies/flash_position.rs
fn validate_refund_assets(
    env: &Env,
    cache: &mut Cache,
    spoke_id: u32,
    hub_id: u32,
    collaterals: &Vec<(HubAssetKey, i128)>,
    refund_assets: &Vec<Address>,
) {
    let limits = storage::get_position_limits(env);
    assert_with_error!(
        env,
        refund_assets.len() <= limits.max_supply_positions,
        GenericError::InvalidPayments
    );

    let mut seen: Map<Address, bool> = Map::new(env);
    for asset in refund_assets.iter() {
        assert_with_error!(
            env,
            !seen.contains_key(asset.clone()),
            GenericError::InvalidPayments
        );
        seen.set(asset.clone(), true);
        // The refund leg hands this address to `token::Client` after the flash
        // guard has closed. Requiring it to be listed keeps that call on a
        // governance-approved contract instead of one the caller chose.
        cache.require_listed_active_config(
            spoke_id,
            &HubAssetKey {
                hub_id,
                asset: asset.clone(),
            },
        );
        for (collateral, _) in collaterals.iter() {
            assert_with_error!(
                env,
                asset != collateral.asset,
                GenericError::InvalidPayments
            );
        }
    }
}
```

Call site passes `account.spoke_id` and `debt.hub_id` (not caller-chosen hub for refunds).

### 2.1 Rule inventory

| # | Rule | Fail | Notes |
|---|---|---|---|
| R1 | `len() ≤ max_supply_positions` | `InvalidPayments` | Same bound as collaterals; `POSITION_LIMIT_MAX` = 5 in constants (governance may set lower) |
| R2 | No duplicate `Address` (`seen` map) | `InvalidPayments` | Address-level, not HubAssetKey |
| R3 | `require_listed_active_config(spoke, {debt.hub_id, asset})` | `SpokeDeprecated` or `AssetNotInSpoke` (#307) | Active spoke + storage listing row |
| R4 | `asset != collateral.asset` for every collateral | `InvalidPayments` | Partition vs deposit list |
| R5 | Empty vec allowed | — | No `require_non_empty`; many happy-path tests pass `Vec::new` |
| R6 | Debt asset **may** be listed | — | Overlap check is collateral-only (INV-STRAT-04: refund ≠ repay) |

### 2.2 What is intentionally **not** checked

| Check used on collaterals / supply entry | On `refund_assets`? | Rationale |
|---|---|---|
| Non-empty / ≥1 positive min | No | Refunds optional |
| `require_can_supply` / `is_collateralizable` | No | Cash return, not supply |
| `require_can_borrow` / `is_borrowable` | No | Not a borrow |
| `enforce_spoke_asset_flags` (paused/frozen) | No | Halted listing may still be refunded as wallet cash; pause blocks **position entry** via collaterals |
| `require_hub_active` on a per-refund hub | Indirect | Uses `debt.hub_id`; debt hub already `require_hub_active` at entry |
| Token implements SAC / non-rebasing | No | Governance listing trust (A055) |

---

## 3. Uniqueness

### 3.1 Primary enforcement

`seen: Map<Address, bool>` rejects a second occurrence of the same address before listing or overlap checks on that element. Harness: `test_flash_position_rejects_duplicate_refund_assets` (ETH twice → `INVALID_PAYMENTS`).

Collaterals use the same Address-level uniqueness (`validate_collaterals` comment: stronger than HubAssetKey uniqueness when two hubs share a token). Refunds only ever carry Address, so the maps are aligned.

### 3.2 Defense in depth if uniqueness were skipped

`snapshot_balances` skips addresses already in the map (one baseline per asset). `refund_listed_assets` would then iterate duplicates with the **same** baseline:

1. First refund: `excess = balance − baseline` → transfer → balance ≈ baseline.
2. Second refund: `excess ≤ 0` → no-op.

So uniqueness is **not** the sole barrier against double-pay; measured-delta semantics already prevent a second full sweep. Uniqueness still matters for:

- Fail-closed input hygiene / integrator feedback (`InvalidPayments`).
- Honest work bound during validation (each entry still runs listing lookup).
- Avoiding ambiguous “declare twice” APIs.

Verdict: uniqueness **defended**; double-pay not achievable even under hypothetical skip (residual only: wasted gas / confusing API).

### 3.3 Overlap with collaterals (partition uniqueness)

Nested loop forbids any refund Address equal to any collateral’s `.asset` (hub id ignored on collateral side). Harness: `test_flash_position_rejects_refund_overlap` (USDC in both → `INVALID_PAYMENTS`).

Order of legs: `process_deposit` **then** `refund_listed_assets`. If overlap were allowed, deposit would consume the positive Δ first and refund would typically see ~0 excess — money-safe but intent-ambiguous. The ban forces a clean partition: an asset is either credited as supply or returned to `caller`, never both in one call.

Same-asset borrow-then-supply (ADR-0020): debt asset may appear in `collaterals`. Then it **cannot** appear in `refund_assets`. Leftover debt tokens pushed as that asset are absorbed into the collateral measure, not wallet-refunded. To wallet-refund unspent debt while collateralizing another asset, list debt in `refund_assets` only.

---

## 4. Allowlist

### 4.1 Mechanism

```76:83:contracts/controller/src/context/spoke.rs
pub(crate) fn require_listed_active_config(
    &mut self,
    spoke_id: u32,
    hub_asset: &HubAssetKey,
) -> AssetConfig {
    self.active_spoke(spoke_id);
    self.require_spoke_asset(spoke_id, hub_asset)
}
```

Storage key is `ControllerKey::SpokeAsset(spoke_id, HubAssetKey)` — **hub_id is part of the key**. Missing row → `AssetNotInSpoke` (#307).

In-code comment at the listing check states the security goal explicitly: post-guard `token::Client` must stay on governance-approved contracts.

### 4.2 Debt-hub keying (A045 residual → owned here)

`refund_assets: Vec<Address>` cannot name a hub. Validator synthesizes:

```text
HubAssetKey { hub_id: debt.hub_id, asset: refund_address }
```

| Scenario | Result |
|---|---|
| Asset listed under `debt.hub_id` in account spoke | Pass R3 |
| Asset listed only under a **different** hub in same spoke | `AssetNotInSpoke` |
| Asset listed in another spoke only | `AssetNotInSpoke` |
| Unlisted / rogue contract (WeirdToken) | `AssetNotInSpoke` (harness) |
| Spoke deprecated | `SpokeDeprecated` via `active_spoke` |

This is **stricter** than “token appears anywhere in the spoke.” It is money-safe (fail closed). UX surprise for multi-hub deployments: integrators must ensure refund tokens are listed under the **debt** hub, not merely under the collateral hub. `endpoints.md` says “listed and active in the account's spoke” without the hub-key detail — documentation gap only.

Attack value of the asymmetry: **none** for theft. Attacker cannot use a foreign-hub listing to admit an unlisted address; they can only be denied a refund declaration.

### 4.3 Timing vs callback

All R1–R4 run before mint/forward/callback. Snapshot and refund `token::Client` calls therefore only see addresses that already cleared listing. Adversarial rationale in `flash_position_adversarial.rs`: unlisted refund would be an arbitrary contract invoked with the flash flag clear while in-memory strategy state is unpersisted — listing closes that vector at the allowlist layer (A007 still notes listed-hook residual).

### 4.4 Contrast with collateral allowlist

| | Collaterals | Refunds |
|---|---|---|
| Identity | `HubAssetKey` (caller picks hub) | `Address` + forced `debt.hub_id` |
| Listing | `require_can_supply` → listed + unhalted + collateralizable | `require_listed_active_config` only |
| Caps / position limits | `validate_position_entry_gates` | Length cap only |
| Post-callback effect | Share credit via `process_deposit` | Wallet transfer via delta |

Asymmetry is appropriate: refunds must not require collateral flags; collaterals must not accept bare untyped addresses without supply gates.

### 4.5 Relationship to A040

A040: position mutations require listed hub assets. A070: **non-position** refund transfers also require listing, for a different reason (post-guard token Client confinement), not because refunds mutate spoke usage or shares.

---

## 5. Execution after validation

```text
with_flash_guard:
  mint_and_forward(debt)
  collateral_before = snapshot(collaterals)
  refund_before     = snapshot(refund_assets)   # listed only
  invoke_receiver(...)
process_deposit(measured collateral Δ)
refund_listed_assets(caller, refund_assets, refund_before)
require_still_open → strategy_finalize → require_still_open
```

```388:400:contracts/controller/src/strategies/flash_position.rs
fn refund_listed_assets(...) {
    for asset in refund_assets.iter() {
        let baseline = before.get(asset)...;
        refund_controller_balance_delta(env, &asset, baseline, caller);
    }
}
```

```220:233:contracts/controller/src/strategies/legs.rs
// excess = balance_delta_since(...); if excess > 0 { transfer(controller → refund_to, excess) }
```

Properties tied to uniqueness/allowlist:

- Only declared listed assets are snapshotted/refunded.
- Baseline includes pre-existing controller inventory → prior stranded dust unstealable.
- Recipient is `caller` (owner or acting delegate), not `receiver`.
- Debt in `refund_assets` returns cash **without** repay (`test_flash_position_returning_debt_token_does_not_repay`) — INV-STRAT-04.
- Zero Δ → silent no-op (over-listing costs budget only).
- Raw `transfer` (not recipient-measured): FOT haircuts caller only (A045/A055).

---

## 6. Negative / attack matrix

| Input abuse | Outcome | Evidence |
|---|---|---|
| Duplicate address in `refund_assets` | `InvalidPayments` | `test_flash_position_rejects_duplicate_refund_assets` |
| Refund address = collateral asset | `InvalidPayments` | `test_flash_position_rejects_refund_overlap` |
| Unlisted / WeirdToken in refunds | `AssetNotInSpoke` before callback | `test_flash_position_rejects_unlisted_refund_asset` |
| `len() > max_supply_positions` | `InvalidPayments` | Code R1; **no dedicated harness found** |
| Empty `refund_assets` | Success (if collateral path OK) | Multiple happy-path tests |
| Debt token in refunds + collateral push | Refund cash; debt shares remain | `test_flash_position_returning_debt_token_does_not_repay` |
| Listed non-collateral extra push | Refunded to caller | `test_flash_position_refunds_undeclared_push` |
| Asset listed only under non-debt hub | `AssetNotInSpoke` | Code path; **no dedicated harness** |
| Paused collateral in collaterals | Rejected pre-callback | `test_flash_position_rejects_paused_collateral_before_callback` (collateral path) |
| Paused asset only in refunds | Allowed by R3 (no flag enforce) | By design; cash return |

No novel critical gap: fail-closed on dupes, overlap, and unlisted; measurement prevents double-sweep.

---

## 7. Docs / product notes

| Source | Accuracy on A070 surface |
|---|---|
| `endpoints.md` constraints table | Correct on len / dupes / overlap / listing; under-specified hub key |
| ADR-0020 | Correct: listed refunds recover leftovers; no auto-repay |
| INV-STRAT-04 | Owns still-open / no repay; not uniqueness per se |
| Skill `writing-flash-position-receivers` | Mentions refund deltas; does not spell uniqueness/allowlist rules |
| Code comment on listing | Accurate security rationale for post-guard Client |

---

## 8. Evidence matrix

| Claim | Evidence |
|---|---|
| Uniqueness enforced Address-level | `validate_refund_assets` `seen` map; harness duplicate test |
| Length capped | `len() <= max_supply_positions`; shared with collaterals |
| Allowlist before callback | Call order in `process_flash_position`; adversarial unlisted test |
| Listing = spoke + debt hub key | `HubAssetKey { hub_id, asset }`; `SpokeAsset` storage key |
| No collateral overlap | Nested assert; overlap harness test |
| Empty OK | No non-empty require; happy paths |
| Debt refund ≠ repay | INV-STRAT-04; returning-debt harness |
| Delta-only / no gross sweep | `refund_controller_balance_delta`; endpoints stranded section; A054 |
| Post-guard Client confined to listed | Code comment + A007 §5 |

---

## 9. Residuals (not undefended fund theft)

| Residual | Severity | Disposition |
|---|---|---|
| Refund listing keyed by `debt.hub_id` | info | Document; optional multi-hub product change |
| No pause/freeze on refund list | info | Intentional; position entry still gated on collaterals |
| Missing over-length / multi-hub harness | info | Coverage gap; code enforced |
| Post-guard listed-token transfer hooks | low | Shared A007/A045; listing trust |
| `endpoints.md` hub-key omission | info | Docs hygiene |
| Certora: no dedicated `validate_refund_assets` rule | info | Harness covers primary negatives |

---

## 10. Cross-refs

| Peer | Relationship |
|---|---|
| A045 | Money-flow owner; defers hub-key allowlist detail to A070 — **agree**, no disagreement file |
| A054 | Flash listed refunds money-safe under A045 — **agree**; A070 owns input validation |
| A007 | Listed-only mitigates post-guard refund window — **agree** |
| A040 | Position listing gate; refunds reuse listing for Client confinement |
| A055 | Lying/FOT listed tokens; allowlist does not prove SAC honesty |
| A062 | Sibling vec length/duplicate theme when filed |
| A018 / A019 | Mode / Wasm receiver — orthogonal gates on same entrypoint |

---

## 11. Verdict

**Status: defended** for A070 scope (`refund_assets` uniqueness and allowlist).

The validator is a complete, fail-closed gate: bounded length, Address uniqueness, debt-hub spoke listing, and collateral-asset partition, all before any untrusted callback or post-guard token Client. Execution preserves measured-delta safety so uniqueness is reinforced rather than sole. The debt-hub listing key is the main semantic quirk — availability/UX, not a theft vector.

Remediation from this audit alone: none required on production Rust. Optional later hygiene: (a) document `debt.hub_id` keying in `endpoints.md` / receiver skill; (b) add harness cases for over-length refunds and multi-hub listing asymmetry; (c) do not widen listing to “any hub” without an explicit ADR.
