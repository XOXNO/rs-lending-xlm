# Blend migration: support same-asset (looped) positions

**Date:** 2026-06-20
**Status:** design approved, pending implementation
**Component:** `contracts/controller/src/strategies/migrate_blend.rs`
**Builds on:** `docs/superpowers/specs/2026-06-19-blend-v2-migration-design.md`

## Problem

`migrate_from_blend` rejects any asset that appears in **both** a withdraw role
(collateral or supply) and the debt role (`AssetsAreTheSame`). This blocks the
most common Blend pattern: **looping** — supply asset X, borrow X, re-supply,
repeat — which leaves a user with a large collateral *and* debt position in the
**same** token.

### Why it is actually blocked (the precise reason)

The original guard comment attributes the rejection to Blend "netting" the
controller's transfers. That is **inaccurate** for the entrypoint we use. Blend's
plain `submit` (`use_allowance = false`) routes to `handle_transfers`
(`blend-contracts-v2/pool/src/pool/submit.rs:235-245`), which iterates
`spender_transfer` and `pool_transfer` as **separate** maps — no netting. Only
`submit_with_allowance` nets (`handle_transfer_with_allowance`, same file
198-233). And `apply_repay` (`actions.rs:415-441`) always pushes the **full
requested `cap`** into `spender_transfer`, with any over-repay excess returned via
`pool_transfer`. So the controller's `authorize_as_current_contract(transfer,
cap)` matches Blend's pull exactly — **the auth is fine**, including for the
existing different-asset debt path.

The real blocker is in **our** accounting. `deposit_withdrawn` and
`reconcile_debt_refunds` both read `balance_delta(asset, before)`
(`migrate_blend.rs:337-340`, `364-367`) against the **same** pre-submit snapshot.
For a shared asset, a single combined balance delta — collateral withdrawn `K`
plus repay refund `C − D` (cap minus actual debt) — would be consumed by **both**
consumers. One delta, two unknowns: not separable from balances alone.

## Goal

Migrate a same-asset looped Blend position **faithfully**: a Blend position of
`K` collateral + `D` debt in token X becomes `K` collateral + `D` debt in token X
in the controller, preserving the user's exact position and leverage. No silent
net-collapse. No Blend state reads (preserve the "submit-only" design principle).

## Approach: always-split (per-phase) submit

Replace the single combined `submit` with up to two **phase-scoped** submits, each
with its own balance snapshot so the deltas never alias.

```
caller.require_auth(); require_not_flash_loaning()
validate non-empty params; require blend pool approved
load/create account
withdraw_assets = dedup(collateral ∪ supply)        # NO overlap-with-debt assertion
require no duplicate debt assets
prefetch oracles(withdraw ∪ debt)

Phase 1 — REPAY (only if debt_caps non-empty):
  before_debt = snapshot(debt assets)
  for (asset, cap) in debt_caps: require_positive(cap); open_migration_borrow(cap)
  reqs = [Repay(asset, cap) ∀]
  authorize_blend_submit(reqs, debt_caps)     # submit + transfer(ctrl→pool, cap) per debt
  guarded_submit(reqs)
  reconcile_debt_refunds(before_debt)         # refund = Δbal → repay into new debt ⇒ debt = D

Phase 2 — WITHDRAW (only if withdraw_assets non-empty):
  before_withdraw = snapshot(withdraw_assets)
  reqs = [WithdrawCollateral(c, MAX) ∀] + [Withdraw(s, MAX) ∀]
  authorize_blend_submit(reqs, EMPTY)         # submit only; withdraws don't pull from ctrl
  guarded_submit(reqs)
  deposit_withdrawn(before_withdraw)          # Δbal = K → deposit as collateral

strategy_finalize(account)                    # single end-state health gate
emit BlendMigrationEvent
```

### Why this is correct for the same-asset case

Trace token X with cap `C`, actual Blend debt `D` (`D ≤ C`), collateral `K`:

- Phase 1: snapshot `B`; borrow `C` (`B+C`); repay submit pulls `C`, refunds
  `C−D` (`B + (C−D)`). `reconcile` reads `Δ = C−D`, repays it into the new debt →
  net new debt `= C − (C−D) = D`, balance back to `B`.
- Phase 2: snapshot `B` (refund already removed); withdraw submit pays `K`
  (`B+K`). `deposit` reads `Δ = K`, deposits `K` as collateral.

End state: collateral `K`, debt `D`. Refund and collateral deltas are measured
against **different** baselines (phase-2 snapshot is taken after phase-1
reconcile), so they never alias — even when the asset is identical.

### Blast radius

- **Collateral-only** migration: `debt_caps` empty → phase 1 skipped → a single
  withdraw submit. Structurally **identical** to today's collateral-only path
  (already live-verified on testnet 2026-06-20). Unchanged.
- **Debt-only** migration: `withdraw_assets` empty → phase 2 skipped → a single
  repay submit. (End state needs pre-existing collateral to pass the health gate,
  same as before.)
- **Combined** debt + withdraw (different-asset OR same-asset loop): two submits.
  The different-asset combined path was only ever harness-tested, never live, so
  restructuring it carries no proven-behavior regression on-chain.

### Auth model (the #1 live risk, now ×2 for the combined case)

The combined case nests **two** `submit(from = user)` calls. Each requires its own
`authorize_as_current_contract` emitted **immediately before** its submit, with no
intervening cross-call (notably no `token.balance()` between the phase-2 snapshot
and the phase-2 authorize/submit — order snapshot → authorize → submit). The user
signs both nested submits through the transaction auth tree.

## Code changes (`migrate_blend.rs`)

- `unique_withdraw_assets`: drop the `debt_set` parameter and the
  `AssetsAreTheSame` assertion; keep the collateral∪supply dedup.
- Keep the duplicate-debt rejection (rename `debt_asset_set` to a `()`-returning
  `require_unique_debt_assets` since its set return is no longer consumed).
- Split `build_blend_requests` into `build_repay_requests(debt_caps)` and
  `build_withdraw_requests(collateral, supply)`.
- Generalize `snapshot_balances` to take a single `Vec<Address>` of assets.
- Add `guarded_submit(env, pool, from, requests)` wrapping the
  `FlashLoanOngoing` set/restore around `blend_submit_call`.
- Reuse `authorize_blend_submit` unchanged: phase 2 passes an empty `debt_caps`
  so no transfer legs are emitted.
- Rewrite `process_migrate_blend` to the two-phase orchestration above.

## Tests (`tests/test-harness/tests/strategy/migrate_blend.rs`, `mock_blend.rs`)

- `test_migrate_role_overlap_rejected` → repurpose to assert a same-asset loop
  **migrates faithfully** (`K` collateral + `D` debt), not rejects.
- Add an explicit same-asset loop test (collateral X + debt X, distinct `K`, `D`).
- Confirm `mock_blend.rs` correctly serves two sequential submits (repay then
  withdraw) and that intermediate state is consistent.
- Re-run the remaining migrate tests (collateral-only, supply-only, debt-only,
  debt+collateral different-asset, unhealthy revert, cap-too-low, unapproved
  pool, into-existing-account).
- Gate: `cargo test -p controller --lib`, full harness, `cargo clippy
  --workspace --all-targets -- -D warnings`, `make wasm-size-check`.

## Live verification (final, optional phase — confirm before deploying)

A same-asset loop is XLM-only, so it is testable on the current testnet, but it
requires the upgraded controller wasm on-chain first:

1. Build + `upgrade_controller` via governance (timelocked ~1 min on testnet).
2. With the deployer wallet: Blend `submit([SupplyCollateral(XLM, 500),
   Borrow(XLM, 200)])` to open a real XLM loop.
3. `migrate_from_blend(collateral_assets=[XLM], debt_caps=[(XLM, cap)])` into a
   fresh account (or account #1).
4. Assert: Blend position cleared, controller account holds ≈500 XLM collateral +
   ≈200 XLM debt, health factor healthy.

## Out of scope

- Net-collapse semantics (rejected in favour of faithful preservation).
- A caller-selectable per-asset faithful/collapse flag (YAGNI).
- Adding new controller markets to enable different-asset (e.g. XLM/USDC) debt
  migration live — a separate effort.
```
