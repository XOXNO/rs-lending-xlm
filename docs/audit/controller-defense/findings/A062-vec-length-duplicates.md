# A062 — Vec length bounds and duplicate hub-asset rejection
- Agent: A062
- Theme: T4 (untrustworthy input validation); DoS.5 adjacency
- Severity: low (resource / hygiene); no fund-theft via duplicate or oversize payment Vecs
- Status: partial
- Paths:
  - `contracts/controller/src/payments.rs` (`aggregate_payments` / `aggregate_positive_payments`)
  - `common/src/validation.rs` (`require_non_empty_payments`)
  - `contracts/controller/src/risk/validation.rs` (`validate_bulk_position_limits`)
  - `contracts/controller/src/views.rs` (`require_view_inputs_bound`, `MAX_VIEW_INPUTS`)
  - `contracts/controller/src/constants.rs` (`MAX_VIEW_INPUTS = 256`, `MAX_DELEGATES = 16`)
  - `common/src/constants/shared.rs` (`POSITION_LIMIT_MAX = 5`)
  - `contracts/controller/src/strategies/flash_position.rs` (`validate_collaterals`, `validate_refund_assets`)
  - `contracts/controller/src/strategies/migrate_blend.rs` (`require_unique_debt_assets`, soft withdraw dedup)
  - `contracts/controller/src/keepers.rs` (unbounded keeper Vecs)
  - `common/src/collections.rs` (`unique_hub_tokens`, `push_unique_address`, `collect_uncached_keys`)
- Defense: Money-batch paths **sum** duplicate `HubAssetKey` legs (documented design), reject empty/negative, overflow-safe add; durable position cardinality capped at `POSITION_LIMIT_MAX`; flash-position / refund lists **hard-reject** duplicates and length; views hard-capped at 256; migrate debt assets unique.
- Gap: (1) No `MAX_VIEW_INPUTS`-style length cap on mutator payment Vecs or keeper Vecs before iteration. (2) Keepers accept duplicate hubs / account ids with no reject or collapse. (3) `migrate_from_blend` collateral/supply lists soft-dedup only (A050). (4) `liquidate` lacks the view estimate’s 256 cap (mitigated by position limits + budget).
- Impact: Cannot open more than `max_*_positions ≤ 5` slots or double-apply the same hub in one pool batch after aggregation. Residual is CPU/fee DoS from oversized pre-aggregate Vecs or keeper spam — STRIDE DoS.5 residual Low, not silent accounting corruption.
- Evidence: INV-RISK-04; endpoints.md §6 “Batching is uniform”; STRIDE DoS.5; harness duplicate-aggregate / flash / migrate tests; unit `aggregate_payments_dedups_and_preserves_order`; Certora position-limit rules.
- Opinion: Intentional split — **aggregate-and-sum** on user payment batches vs **reject-duplicates** on flash snapshot lists and migrate debt caps — is correct for money safety. The remaining gaps are length hygiene (A015 keepers; uncapped raw payment Vecs), not double-credit bugs.

---

## 1. Scope and judgment criteria

This agent inventories every controller surface that accepts a `Vec` of hub assets, payments, addresses, or account ids, and asks:

1. Is **length** bounded (compile-time constant, governance position limit, or empty-only)?
2. Are **duplicates** rejected, summed, or silently skipped?
3. Does the choice protect **accounting invariants** or only **resource budgets**?

Cross-links: A008 (view bounds), A015 (keeper Vec length), A045 (flash-position uniqueness), A050 (migrate list hygiene), A061 (amount sign/zero — complementary; empty payments via `require_non_empty_payments`).

---

## 2. Shared primitives

### 2.1 `aggregate_payments` — sum by `HubAssetKey`, preserve first-seen order

```47:72:contracts/controller/src/payments.rs
pub(crate) fn aggregate_payments(
    env: &Env,
    payments: &Vec<HubPayment>,
    zero_leg: ZeroLeg,
) -> Vec<HubPayment> {
    require_non_empty_payments(env, payments);
    // ... Map totals; order Vec of first-seen HubAssetKey ...
}
```

| Rule | Behavior | Error |
|---|---|---|
| Empty Vec | Reject | `InvalidPayments` (#16) |
| Negative amount | Reject (before sticky-zero arms) | `AmountMustBePositive` (#14) |
| Zero + `ZeroLeg::Rejected` | Reject | `AmountMustBePositive` |
| Zero + `ZeroLeg::MeansAll` | Withdraw-all sentinel; sticky zero | — |
| Duplicate `HubAssetKey` | **Sum** with `checked_add` | `MathOverflow` (#33) on wrap |
| Key identity | Full `HubAssetKey` (`hub_id` + `asset`) | Same SAC under two hubs = two legs |

`aggregate_positive_payments` = `ZeroLeg::Rejected`. Used by supply, borrow, repay, liquidation repayment math. Withdraw uses `MeansAll`.

**Design note (not a bug):** Duplicates are not rejected on these paths. `docs/reference/endpoints.md` §6: “Multi-asset calls sum duplicate legs per asset and preserve first-appearance ordering.” Unit proof: `contracts/controller/tests/helpers/utils.rs::aggregate_payments_dedups_and_preserves_order`.

After aggregation, each hub appears **once** in the pool batch. That is the accounting defense against double-apply.

### 2.2 `validate_bulk_position_limits` — slot cardinality, with in-list dedup

```63:116:contracts/controller/src/risk/validation.rs
// Counts only hub assets not already on the account, deduplicated within aggregated.
// Top-ups of held assets are free even if current_count > max_allowed after a governance cut.
```

Governance stores `PositionLimits` with both sides in `1..=POSITION_LIMIT_MAX` (`POSITION_LIMIT_MAX = 5`). Constructor defaults both to 5 (`governance.rs`). INV-RISK-04.

The in-function `seen` Map is belt-and-suspenders: callers normally pass already-aggregated Vecs. Credit-mode liquidation builds a fresh aggregated list from seize entries (`require_credit_position_limit`).

### 2.3 View length cap

```19:26:contracts/controller/src/views.rs
fn require_view_inputs_bound<T>(env: &Env, values: &Vec<T>) {
    assert_with_error!(
        env,
        values.len() <= MAX_VIEW_INPUTS, // 256
        GenericError::InvalidPayments
    );
}
```

Applied to `get_market_indexes_detailed` and `liquidation_estimations_detailed` / `get_liquidation_estimate`. Empty Vecs are allowed on views (no `require_non_empty`). Duplicates are **not** rejected: market-index view emits one row per input entry; prices are fetched via `unique_hub_tokens` only.

### 2.4 Collection helpers

| Helper | Role |
|---|---|
| `unique_hub_tokens` | Distinct underlying `Address` from hub keys (oracle prefetch) |
| `push_unique_address` | Soft append-if-absent (migrate withdraw list, risk price assets) |
| `collect_uncached_keys` | Missing-key collect with O(n²) dedup; **callers must be position-limit bounded** (comment in `collections.rs`) |

---

## 3. Surface inventory

### 3.1 Core position mutators (payment batches)

| Entrypoint | Aggregate mode | Empty | Max len | Duplicate policy | Downstream bound |
|---|---|---|---|---|---|
| `supply` | Positive sum | reject | **none** | sum by hub | `validate_bulk_position_limits` Deposit + listing |
| `borrow` | Positive sum | reject | **none** | sum by hub | Borrow position limit + solvency |
| `repay` | Positive sum | reject | **none** | sum by hub | Existing debt map (unknown hub panics later) |
| `withdraw` | MeansAll sum | reject | **none** | sum by hub; zero sticky | Existing supply map |
| `liquidate` | Positive sum (inside plan) | reject (double-checked) | **none** (unlike estimate) | sum by hub | Debt map ≤ `max_borrow_positions`; seize ≤ supply map |

**Pre-aggregate pause walk (liquidation):** `build_liquidation_plan` iterates **raw** `debt_payments` for `AllowOnExit` flags before merge. Duplicate legs re-check the same hub; oversized lists burn budget before aggregation.

**Estimate asymmetry:** `get_liquidation_estimate` enforces `len ≤ 256`; `liquidate` does not. Practical unique hubs after aggregate still cannot exceed the account’s borrow map (≤ 5 under current `POSITION_LIMIT_MAX`). Residual is raw-Vec DoS only.

Harness evidence of intentional sum behavior:

- `tests/test-harness/tests/controller/supply.rs` — `test_supply_duplicate_asset_bulk_*`, overflow revert
- `borrow.rs` — `test_borrow_duplicate_asset_bulk_accumulates_single_position`
- `repay.rs` — `test_repay_duplicate_asset_payments_aggregate`
- `withdraw.rs` — `test_withdraw_aggregates_duplicate_assets`, sticky zero
- `liquidation_coverage.rs` — `test_liquidation_aggregates_duplicate_debt_payments`

### 3.2 Flash position — hard reject (strongest list hygiene)

| Input | Non-empty | Max len | Duplicate policy | Other |
|---|---|---|---|---|
| `collaterals: Vec<(HubAssetKey, i128)>` | yes | `≤ max_supply_positions` | **Reject** on underlying `Address` (`InvalidPayments`) | ≥1 positive min; listing/caps |
| `refund_assets: Vec<Address>` | no (empty OK) | `≤ max_supply_positions` | **Reject** duplicates | Listed on debt hub; no overlap with collateral assets |

Comment in `validate_collaterals`: uniqueness on **asset** is strictly stronger than full `HubAssetKey`, so two hubs sharing a SAC cannot both be declared (would double-snapshot the same token balance).

Why reject here but sum elsewhere: flash snapshot / refund delta math must map **one baseline → one Δ**. Summing duplicate declarations would be ambiguous; rejecting is the money-safe choice (A045).

Tests: `test_flash_position_rejects_duplicate_collateral`, `test_flash_position_rejects_duplicate_refund_assets`; integration `flash_position.sh` xfail `#16`.

### 3.3 `migrate_from_blend`

| List | Empty rule | Max len | Duplicate policy |
|---|---|---|---|
| Union of collateral/supply/debt | At least one non-empty overall | **none** | — |
| `debt_caps: Vec<(Address, i128)>` | may be empty if others present | **none** | **Hard reject** — `AssetsAreTheSame` (#7) via `require_unique_debt_assets` |
| `collateral_assets` / `supply_assets` | — | **none** | **Soft dedup** into `withdraw_assets` via `push_unique_address`; raw lists still passed to Blend sweep |

Debt uniqueness prevents double full-cap mint of the same asset. Collateral/supply duplicates are hygiene-only (sweep no-ops after empty) — agrees with A050 residual (4).

Tests: `test_migrate_duplicate_debt_rejected`; fuzz `migrate_blend_rejects_empty_duplicate_unapproved_zero_cap`; integration `blend.sh` xfail `#7`.

### 3.4 Single-leg strategies (no multi-hub payment Vec)

`multiply`, `swap_debt`, `swap_collateral`, `repay_debt_with_collateral`, `flash_loan` take discrete hub keys / amounts, not payment batches. Swap **bytes** emptiness is enforced (`InvalidPayments`) — A047/A049/A056 territory, not hub-duplicate inventory.

`snapshot_balances` skips repeat addresses when building maps (safe soft dedup for measurement).

### 3.5 Views

| View | Length | Empty | Duplicates |
|---|---|---|---|
| `get_market_indexes_detailed(hub_assets)` | `≤ 256` | allowed | Allowed; one output row per input; prices unique’d |
| `get_liquidation_estimate(debt_payments, …)` | `≤ 256` | rejected later by plan/aggregate | Summed inside shared plan path |

Unit: `view_input_bound_rejects_oversized_asset_vectors`. A008: defended for views.

### 3.6 Keepers (permissionless Vecs) — gap aligned with A015

| Entrypoint | Length cap | Empty check | Duplicate policy |
|---|---|---|---|
| `update_indexes(assets)` | **none** | no (no-op loop) | none — pool may accrue same hub twice |
| `claim_revenue(assets)` | **none** | no | none — second claim of same hub typically returns 0 |
| `update_account_threshold(account_ids)` | **none** | no | none — same id refreshed twice |

Money destination for revenue is still the accumulator (A015). Gap is resource/UX only. STRIDE DoS.5 claims batch sizes are “bounded by caller input” — true for work proportional to Vec length, but there is **no protocol max** analogous to `MAX_VIEW_INPUTS` on these three.

### 3.7 Adjacent non-payment Vec bounds (completeness)

| Surface | Bound |
|---|---|
| Account delegates | `MAX_DELEGATES = 16` (`RegistryCapReached`) — INV-RISK-04 adjacent |
| `set_position_limits` | both sides `1..=POSITION_LIMIT_MAX` |
| Governance `AddAssetToSpoke` / validate | same position-limit domain |

---

## 4. Defense vs gap matrix

| Concern | User payment batches | Flash collaterals/refunds | Migrate debt | Migrate coll/supply | Views | Keepers |
|---|---|---|---|---|---|---|
| Empty reject | yes | collaterals yes | request-level yes | soft | no | no |
| Hard max length | no | `max_supply_positions` | no | no | 256 | **no** |
| Duplicate → unique pool/account effect | yes (sum) | N/A (reject) | reject | soft unique withdraw | N/A | **no** |
| Durable slot overflow | position limits | position limits | supply/borrow gates | same | N/A | N/A |
| Fund double-credit via dups | **defended** | **defended** | **defended** | soft OK | N/A | revenue not caller-directed |

---

## 5. Impact quantification

**What an attacker cannot do with duplicate / long Vecs**

- Open more than `max_supply_positions` / `max_borrow_positions` (≤ 5) new slots in one call or cumulatively past the limit (except top-ups of held assets).
- Cause the pool batch for supply/borrow/repay/withdraw/liquidate to process the same `HubAssetKey` twice after aggregation.
- Double-snapshot or double-refund the same token in `flash_position`.
- Mint migrate debt twice for one SAC in `debt_caps`.

**What remains**

- Fee/CPU exhaustion by submitting a huge Vec of repeated hubs on mutators (aggregate still walks every leg) or keepers (no cap). Soroban budget reverts — availability annoyance, not theft. Same residual class as A015.
- `liquidate` accepting `debt_payments.len() > 256` while the estimate view refuses — UX asymmetry; unique post-aggregate set still ≤ account debt cardinality.
- Migrate collateral/supply duplicate entries — no accounting bug (A050).

Blast radius: **accounts** bounded by position limits; **markets** untouched by duplicate payment shape; **governance** only via raising `POSITION_LIMIT_MAX` (requires code + Certora fixture assert currently pinned at 5).

---

## 6. Evidence index

| Kind | Location |
|---|---|
| Aggregate sum + overflow | `payments.rs`; unit helpers tests; harness supply overflow |
| Position limits | `risk/validation.rs`; INV-RISK-04; Certora `solvency_rules` / `spoke_rules`; harness `position_limit_lowering_keeps_topups.rs` |
| Flash reject | `flash_position.rs:186–268`; harness + `flash_position.sh` |
| Migrate debt unique | `migrate_blend.rs:201–213`; `#7 AssetsAreTheSame` |
| View cap | `views.rs` + `MAX_VIEW_INPUTS`; unit panic `#16` |
| Docs | endpoints.md supply batch + §6; errors.md `#16`; STRIDE DoS.5 |
| Peer | A008 defended views; A015 partial keepers; A045 flash; A050 migrate soft lists; A061 amounts |

---

## 7. Remediation notes (non-blocking; audit-only)

If a later fix wave scopes hygiene:

1. Reuse `MAX_VIEW_INPUTS` (or `MAX_KEEPER_INPUTS`) on keeper Vecs and optionally on mutator payment Vecs **before** aggregation / pause walks.
2. Optionally reject (not only soft-dedup) migrate `collateral_assets` / `supply_assets` duplicates for fail-loud UX.
3. Align `liquidate` with estimate’s 256 cap for API symmetry (defense-in-depth; not required for INV-RISK-04).

Do **not** change aggregate-and-sum semantics on supply/borrow/repay/withdraw/liquidate without an intentional API break — that behavior is documented and harness-pinned.

---

## 8. Opinion

Controller Vec hygiene is **intentionally tiered**: payment batches collapse duplicates into one monetary leg; flash and migrate-debt lists that drive snapshots or full-cap borrows hard-reject; views and position maps bound read/write cardinality. The undefended corner is the same class A015 already flagged for keepers — missing hard length caps on permissionless maintenance Vecs and on raw payment Vecs before fold — with **no** demonstrated path from duplicates to double-credit or slot exhaustion beyond `POSITION_LIMIT_MAX`. Status **partial**, severity **low**.
