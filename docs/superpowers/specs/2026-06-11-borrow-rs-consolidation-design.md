# borrow.rs structural consolidation — design

**Date:** 2026-06-11
**Scope:** `contracts/controller/src/positions/borrow.rs` (deep restructure) + mechanical convention ripple to `supply.rs`, `withdraw.rs`, `repay.rs`, `validation.rs`, `strategies/helpers.rs`.
**Constraint regime:** pre-mainnet "open season" — benign panic-selection changes and gate additions are allowed; ABI of public contract endpoints is unchanged (`Controller::borrow` signature untouched).

## Problem

`borrow.rs` works but reads poorly and duplicates itself:

1. `process_borrow`'s doc claims "LTV pre-checks"; LTV runs post-pool.
2. Reading order ≠ execution order: the strategy variant sits above the main flow; a validator sits above the stage that calls it.
3. The deduped plan is named `borrows`/`borrow_plan`/`assets`/`new_borrows` across the call chain; `cache` appears at parameter positions 2, 4, and last.
4. The 10-line pool-result merge block is duplicated between `create_borrow_strategy` and `execute_borrow_plan` (only `PositionAction` differs).
5. `create_borrow_strategy` hand-rolls single-asset validation (market-active, e-mode config, siloed, borrowability, isolated debt) duplicating `prepare_borrow_plan` — and **skips the position-limit gate**, so a multiply/debt-swap strategy can push an account past `max_borrow_positions`.
6. `prepare_borrow_plan` walks the plan twice (market-active loop, then config/borrowability/isolated-debt loop) with no semantic justification; supply does it in one pass.
7. `process_borrow` carries an 8-line RedStone prefetch block + comment because `require_within_ltv` prefetches market indexes internally but not RedStone feeds — the gate does not own its data dependencies.
8. `process_borrow_plan` is a 3-line private wrapper with one caller.

## Target shape of borrow.rs (reading order = execution order)

```
//! module doc (updated: gates run post-pool)
#[contractimpl]  borrow → process_borrow

pub fn process_borrow            // full pipeline; process_borrow_plan inlined
fn   prepare_borrow_plan         // all pre-pool gates, single per-asset pass
fn   execute_borrow_plan         // build entries → one pool call → merge loop
fn   merge_borrow_result         // shared pool-result merge (new, flat args)
fn   validate_asset_borrowable   // renamed from validate_borrow_asset_preflight
fn   validate_siloed_borrow_set
pub fn borrow_for_strategy       // renamed from create_borrow_strategy; last
```

`process_borrow` body order: auth → flash-loan guard → account load → owner match → cache (RiskIncreasing) → `plan = aggregate_positive_payments` → `configs = effective_configs_for_plan` → `prepare_borrow_plan` → `execute_borrow_plan` → `require_within_ltv` → `require_healthy_account` → plan-scoped dust gate → `set_debt_positions` → `flush_isolated_debts` → `emit_position_batch` → `emit_market_batch`.

`process_borrow_plan` is deleted (inlined). Supply keeps its analogue `process_deposit` because strategies reuse it; borrow strategies use `borrow_for_strategy` instead.

## Conventions (ripple mechanically to siblings)

- **One name per concept:** the deduped vec is `plan: &Vec<Payment>` in every internal signature; raw entrypoint input stays `borrows`/`withdrawals`/`payments`.
- **Parameter order:** `env` first, domain payload (`account`, `asset`, `config`, …) in the middle, `cache` last — matching `settle_repay_actions`/`settle_withdraw_entries`.
- **Renames:**
  - `validate_borrow_asset_preflight(env, cache, config, asset, account)` → `validate_asset_borrowable(env, account, asset, config, cache)`
  - `create_borrow_strategy(env, cache, account, debt_token, amount)` → `borrow_for_strategy(env, account, debt_token, amount, cache) -> i128` (caller: `strategies/helpers.rs:72`)
- Ripple targets: `supply.rs` (`assets` → `plan`, cache-last in `prepare_deposit_plan`/`execute_deposit_plan`), `withdraw.rs`/`repay.rs` (already cache-last; rename `assets` params; fix the stale "withdrawal loop prices the withdrawn asset" comment block in `process_withdraw`), `positions/mod.rs` helpers already conform.

## Consolidations

**`prepare_borrow_plan(env, account, plan, effective_configs, cache)`:**
`require_non_empty_payments` → `validate_bulk_position_limits` → `validate_siloed_borrow_set` → one per-asset loop: `require_market_active` → `effective_config` → `validate_asset_borrowable` → `add_isolated_debt`. The standalone market-active loop is removed.

**`merge_borrow_result(account, asset, action, result: &PoolPositionMutation, cache)`:**
`record_market_update` → decode `DebtPosition` → `record_debt_position_update(action, asset, result.market_index.borrow_index_ray, result.actual_amount, &position)` → `update_or_remove_debt_position`. Called from `execute_borrow_plan`'s merge loop (`PositionAction::Borrow`) and `borrow_for_strategy` (`PositionAction::Multiply`). Flat arguments — no parameter struct.

**`borrow_for_strategy`:** builds a one-element plan, calls `effective_configs_for_plan` + `prepare_borrow_plan` (gaining the position-limit and non-empty gates), then the strategy-specific tail: flash fee from the effective config, `get_or_create_debt_position`, `pool_create_strategy_call`, `merge_borrow_result(…, Multiply, …)`, return `result.amount_received`. The hand-rolled `new_borrows` vec, duplicate e-mode resolution, and duplicate validation calls are deleted.

**`require_within_ltv` (validation.rs):** gains `crate::oracle::prefetch_redstone_feeds(&union)` immediately before its existing `prefetch_market_indexes(&union)`, after the empty-borrow early return. Borrow's call-site prefetch block (8 lines + comment) is deleted. Withdraw and the strategy health check keep their earlier, deliberately scoped call-site prefetches; the gate's internal prefetch no-ops on already-cached feeds (`prefetch.rs` skips cached entries), so no extra oracle calls anywhere.

## Behavior deltas (intentional)

1. **Panic selection on multi-fault plans:** per-asset checks interleave instead of all-market-active-first; a plan with two different faults may revert with a different (equally valid) error code. All faulty plans still revert.
2. **Strategy borrows gain gates:** `PositionLimitExceeded` becomes reachable from multiply/debt-swap strategy borrows (bug fix), plus the vacuously-true non-empty check. Check order within strategy borrows also changes (limits/siloed before market-active in the loop).
3. **No call-count changes:** oracle and pool cross-contract call counts are identical in all flows; the prefetch move is coverage-neutral for borrow and a no-op for the other two `require_within_ltv` callers.

## Testing & verification

- `cargo check --workspace --all-targets`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cargo test -p controller --lib`.
- Expect a small number of error-code assertions on multi-asset failure tests to need updating (search for `NotBorrowableSiloed`, `AssetNotBorrowable`, `MarketNotActive`-class assertions over multi-asset plans).
- **New test:** strategy borrow (multiply path) blocked with `PositionLimitExceeded` at `max_borrow_positions`.
- WASM size: net change should be ≤ 0 (dedup outweighs the new helper); verify with `stellar contract build` before/after on a stashed baseline.
- Known pre-existing failures, not gates for this work: `budget_withdraw_5_collateral_double_pass` (shadow-budget cliff) and the stale `pool.wasm` fixture requirement (rebuild before running `-p defindex-strategy`).
