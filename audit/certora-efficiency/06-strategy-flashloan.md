# Certora Efficiency — Strategy & Flash Loan

**Files in scope:**
- `controller/certora/spec/strategy_rules.rs` (702 lines, 16 active `#[rule]`s + 4 sanity)
- `controller/certora/spec/flash_loan_rules.rs` (153 lines, 4 active `#[rule]`s + 1 sanity)
- `controller/confs/strategy.conf` (`loop_iter: 12`, `-maxBlockCount 500000`, `-maxCommandCount 2000000`)
- `controller/confs/flash_loan.conf` (`loop_iter: 12`, `-maxBlockCount 300000`)
- Production: `controller/src/strategy.rs:1-1058`, `controller/src/flash_loan.rs:1-77`,
  `controller/src/positions/borrow.rs:26-75`, `controller/src/positions/liquidation.rs:460-528`,
  `controller/src/validation.rs:38-84`
- Compat shims: `controller/certora/spec/compat.rs:34-117`
- Summaries: `controller/certora/spec/summaries/{mod,pool,sac}.rs`

**Soundness audit reference (P1a/P1b):** P1b parameterized `compat::multiply` with
`account_id`, `initial_payment`, `convert_steps` (compat.rs:41-92); P1a flagged
`multiply_creates_both_positions` and `flash_loan_guard_cleared_after_completion`
as unsound. This pass is orthogonal: **"will the prover finish, and is each
rule's PASS load-bearing?"**.

---

## Headline finding (blocks every other recommendation)

**The new `summaries/pool.rs` and `summaries/sac.rs` are not wired into the
production build.** They are referenced by no `summarized!` / `apply_summary!`
call, no `cvlr_mock_client` adornment on `LiquidityPoolClient`, and no
`#[cfg(feature="certora")]` redirect on the SAC `token::Client`. Confirming
greps:

- `pool/src/lib.rs::create_strategy` (line 458), `flash_loan_begin` (line 389),
  `flash_loan_end` (line 415), `seize_position` (line 510), `claim_revenue`
  (line 547) — none wrapped by `summarized!`.
- `pool-interface/src/lib.rs:9` declares `#[contractclient(name = "LiquidityPoolClient")]`
  via `soroban_sdk::contractclient`. That generator emits a real
  cross-contract client; it is **not** the `cvlr_soroban_macros::cvlr_mock_client`
  declared in `vendor/cvlr-soroban/cvlr-soroban-derive/src/mock_client.rs:157`.
  Search for `cvlr_mock_client` in the production tree returns zero hits.
- The summary functions (`create_strategy_summary`, `supply_summary`, …,
  `transfer_summary`, `balance_summary`) are referenced **only** by their own
  module — they are dead code in the certora build.

Consequence for THIS audit: every Strategy and Flash-Loan rule that crosses a
`pool_client.X(...)` call or a `token::Client::X(...)` call is reasoning over
**fully havoced** returns and **fully havoced** side effects. The prover
treats each call as a fresh nondet of the declared return type, with no
post-conditions. Pre/post comparisons across such a call (e.g.
`revenue_after >= revenue_before` in `flash_loan_fee_collected`) are
asserting `nondet1 >= nondet2` — vacuous.

This wiring gap is the **dominant scope-cost-and-soundness defect** in both
files. It dwarfs every per-rule issue below: with summaries unwired, even
"correctly written" rules in this domain do not prove what they claim. Every
recommendation that reads "after the pool summary is wired" is gated on a
build-system fix (wrap `pool::create_strategy` etc. with
`summarized!(spec::summaries::pool::create_strategy_summary, …)` at the
production sites in `pool/src/lib.rs`, and wrap the SAC client in a thin
`#[cfg(feature="certora")] cvlr_mock_client`-style shim).

The audit below assumes the wiring is fixed (as the scope brief states); each
rule is judged on its own merits given that assumption. Where the wiring gap
**also** changes the rule's verdict, it is called out explicitly.

---

## Efficiency rubric (per-rule)

Legend:
- **N**: nondets (count, bounded?)
- **S**: state symbolic? (account-shape, map-shape, storage)
- **A**: single-asset focus?
- **L**: unbounded loops over position maps / asset list?
- **P**: alignment with pool / SAC summaries (assumes wiring)?
- **F**: action-focused vs state-invariant
- **B**: decision branches in the production fn
- **R**: storage read budget
- **V**: actual verification value (post-summary, post-wiring)
- **W**: wiring sensitivity — does PASS depend on the unwired summaries?

---

## Per-rule classification (strategy_rules.rs)

### `multiply_creates_both_positions` — `strategy_rules.rs:31-70`
**Action:** keep but **tighten heavily**; today the heaviest rule in the file by an order of magnitude.
- N: 8 rule params + 3 in compat (`account_id`, `initial_payment`, `convert_steps`) = **11 free symbols** before any pool/SAC nondet.
- S: full symbolic account state for the load-existing branch (`account_id != 0` from compat.rs:61).
- A: no — `collateral_token` and `debt_token` are symbolic.
- L: yes — `collect_initial_multiply_payment` (strategy.rs:669-707) may call `swap_tokens` (4 SAC ops + router); `swap_tokens` itself executes 5 SAC ops + 1 router call (strategy.rs:407-452); `process_deposit` iterates `deposit_assets`; `strategy_finalize` re-traverses both position maps via `require_healthy_account` (validation.rs:75-83).
- P: relies on `pool::create_strategy_summary` (pool.rs:250) **and** `pool::supply_summary` (pool.rs:76) — TWO mutating pool summaries plus the SAC `transfer/balance/approve` set, which is more than any other rule in the file.
- F: action-focused (good).
- B: 4-way branch on `account_id == 0` × `initial_payment.is_some()` × `payment_token == collateral_token | debt_token | other` (strategy.rs:686-703) — the "other" branch nests **another** `swap_tokens`. With `take_initial`/`take_convert` both nondet booleans, the prover explores 4 entry-shape combinations × 4 payment-token shapes × 2 account-id shapes = 32 paths through the entry preamble.
- R: heavy — `cache.cached_asset_config(collateral_token)`, `cached_pool_address`, `cached_price` for both tokens, `storage::get_account` (load branch), `storage::set_supply_positions`, `storage::set_borrow_positions`, then re-priced `cached_price` after `clean_prices_cache` for HF re-check (strategy.rs:973-974).
- V: after wiring, this rule proves a real property (P1b made it sound). But the **path explosion from compat.rs:61-77** is unnecessary — the property "both positions populated" is independent of whether the user supplied an `initial_payment` or which branch the load-vs-create took.
- W: high — without `pool::create_strategy_summary` constraining `position.scaled_amount_ray >= prior` (pool.rs:259-261) and `pool::supply_summary` similarly (pool.rs:84-85), the post-state `scaled_amount_ray` field is fresh nondet i128 and the assert `> 0` fails on the prover's first counter-example.

**Proposed change:** split into 3 rules.
1. `multiply_create_new_creates_both_positions` — pin `account_id == 0`, pin `initial_payment = None`, pin `convert_steps = None`. The happy-path multiply with no initial payment. Removes the load-existing branch and both swap-payment branches.
2. `multiply_load_existing_creates_both_positions` — pin `account_id == 1`, assume an existing account in the right `mode` exists in storage, pin `initial_payment = None`. Verifies the load branch.
3. `multiply_with_initial_payment_in_collateral_creates_both_positions` — pin `account_id == 0`, pin `initial_payment = Some((collateral_token, amount))`. Exercises the `payment_token == collateral_token` branch (strategy.rs:686-687) — the cheap one (no nested `swap_tokens`).

Skip the "other token initial payment" branch from the certora set entirely; verify it via a unit test in `test-harness/tests/multiply_tests.rs` since it requires a fully-functional aggregator mock.

### `multiply_rejects_same_tokens` — `strategy_rules.rs:79-106`
**Action:** keep, but inline the compat shim's nondet inputs to constants.
- N: 7 rule params + 3 compat-shim havocs = **10 free symbols**, almost all ignored after the early panic at strategy.rs:158-160.
- S/A/L/B: minimal — `process_multiply` panics at line 158-160 before any iteration or pool call.
- F: action-focused negative path (good).
- V: high — proves `AssetsAreTheSame` guard.
- W: none — early panic short-circuits before any cross-contract call.

**Proposed change:** the compat shim havocs `account_id`, `take_initial`, `take_convert` (compat.rs:61, 65, 72) — but the panic fires at strategy.rs:158 *before* any of those branches are reached. Wasted nondet draws. Either (a) bypass the compat shim entirely and call `crate::Controller::multiply(...)` directly with `account_id=0, initial_payment=None, convert_steps=None`, or (b) expose a `compat::multiply_minimal` shim that does not havoc the optional fields. Same applies to **every "rejects" rule** that uses the compat shim (rules 3, 14, 14b/c/d): they pay for the nondet branching even though execution panics before it matters.

### `multiply_requires_collateralizable` — `strategy_rules.rs:115-148`
**Action:** keep, same compat-shim tightening as above.
- N: 7 + 3 = 10 free symbols, plus a `cache.cached_asset_config(&collateral_token)` to constrain `is_collateralizable = false`.
- B: panics at strategy.rs:189-191 before any pool / SAC call.
- W: none.

**Proposed change:** same as `multiply_rejects_same_tokens`. The cache lookup at line 131-133 to constrain config is fine — that's a single storage read against a symbolic asset; pin `collateral_token` to a concrete address to drop the symbolic-key-lookup cost.

### `swap_debt_conserves_debt_value` — `strategy_rules.rs:157-201`
**Action:** keep but tighten; second-heaviest rule in the file.
- N: 6 rule params + 1 captured `old_scaled_before` from a symbolic position read = 7 symbols.
- S: full symbolic account loaded via `storage::get_account` at strategy.rs:268.
- A: symbolic both `existing_debt_token` and `new_debt_token`.
- L: `process_swap_debt` calls `open_strategy_borrow` (1 pool call) + `swap_tokens` (5 SAC ops + 1 router) + `repay_debt_from_controller` (1 SAC `transfer_and_measure_received` + `execute_repayment` which does 1 `pool::repay`) + `strategy_finalize` (1 fresh `cache.clean_prices_cache` + `require_healthy_account`).
- P: relies on **3 pool summaries** (`create_strategy`, `repay`, plus internal `update_indexes` paths) plus the full SAC set.
- F: action-focused.
- B: siloed-borrowing rejection (strategy.rs:283-285) and `DebtPositionNotFound` (line 309) are both early-exit branches.
- R: heavy.
- V: "old debt < before, new debt position > 0" is the right shape but does not actually conserve **value** — the rule name overstates what is asserted. The pool returns havoced `scaled_amount_ray` for the new borrow and havoced `actual_amount` for the repay; the rule cannot say anything about USD value parity.
- W: high — same as `multiply_creates_both_positions`.

**Proposed change:** rename to `swap_debt_decreases_old_creates_new` (the actual asserted property). Pin `account_id = 1`, pin both tokens to two distinct concrete addresses. Drop the `cvlr_assume!(old_scaled_before > 0)` and instead pin a concrete value (e.g. `WAD`) for the position scaled amount via a pre-rule storage write — eliminates one symbolic-state dimension at zero verification cost.

### `swap_debt_rejects_same_token` — `strategy_rules.rs:209-232`
**Action:** keep.
- N: 5 free symbols, panics at strategy.rs:264-266 before any work.
- W: none.

**Proposed change:** none.

### `swap_collateral_conserves_collateral` — `strategy_rules.rs:240-284`
**Action:** keep but tighten, twin of `swap_debt_conserves_debt_value`.
- N: 6 rule params + 1 captured = 7.
- L: `process_swap_collateral` runs `validate_swap_new_collateral_preflight` (e-mode + isolation lookup, position-limit check), `withdraw_collateral_to_controller` (1 SAC balance read + `execute_withdrawal` → 1 `pool::withdraw`), `swap_tokens` (5 SAC + 1 router), `process_deposit` (1 `pool::supply`), `strategy_finalize` (HF re-check). 3 pool calls + ~10 SAC ops.
- P: relies on `pool::withdraw_summary` + `pool::supply_summary` + SAC set.
- B: isolation rejection (strategy.rs:349-351), `allow_unsafe_price` toggle on empty borrow (line 355), e-mode validation (line 1035-1043), collateralizable check (line 1045-1047), position-limit branch (line 1050-1056).
- R: heavy.
- V: same shape critique as `swap_debt_conserves_debt_value`.
- W: high.

**Proposed change:** rename to `swap_collateral_decreases_old_creates_new`. Pin `account_id = 1`, pin both assets, pin `from_amount` to a small concrete value (e.g. `1_000_000`) to side-step the `i128` symbolic range that drives prover branch counts in `withdraw::execute_withdrawal`'s Wad/Ray math. The capture of `old_scaled_before` is fine as-is.

### `swap_collateral_rejects_same_token` — `strategy_rules.rs:291-314`
**Action:** keep.
- W: none — panics at strategy.rs:342-344.

### `swap_collateral_rejects_isolated` — `strategy_rules.rs:322-351`
**Action:** keep.
- N: 6 rule params + 1 storage read for `account_attrs`.
- W: none — panics at strategy.rs:349-351 before any pool call.

**Proposed change:** the `cvlr_assume!(attrs.is_isolated)` (line 337) requires the prover to find a storage state where the account exists and is isolated. Pin `account_id = 1` and write the isolation flag directly via a test-only storage helper if one exists, otherwise leave as-is — the assume is cheap.

### `repay_with_collateral_reduces_both` — `strategy_rules.rs:359-411`
**Action:** keep but tighten — heaviest "reduce both sides" rule.
- N: 7 rule params + 2 captured + 1 compat havoc (`close_position`) = 10 free symbols.
- L: `process_repay_debt_with_collateral` runs:
  - `withdraw_collateral_to_controller` (1 SAC balance + `pool::withdraw`)
  - `swap_or_net_collateral_to_debt` (either short-circuit OR `swap_tokens` = 5 SAC + 1 router)
  - `repay_debt_from_controller` (1 SAC `transfer_and_measure_received` = 3 SAC ops + `pool::repay`)
  - `close_remaining_collateral_if_requested` (if `close_position == true` AND borrows empty: `execute_withdraw_all` which **iterates every supply position** — unbounded loop)
  - `strategy_finalize` (HF re-check)
- B: same-asset short-circuit (strategy.rs:474-476), `close_position` 2-way branch with the loop-bearing path inside.
- P: 2 pool summaries + SAC set.
- W: high.

**Proposed change:** split into two rules.
1. `repay_with_collateral_reduces_both_no_close` — pin `close_position = false` via a separate compat shim variant. Removes the `execute_withdraw_all` unbounded loop entirely.
2. `repay_with_collateral_close_clears_account` — pin `close_position = true`, pin `borrow_positions.len() == 1` so the rule passes the "no remaining debt" check after the repay, and assert that `account` no longer exists in storage post-call (verifies the `strategy_finalize` deletion branch at strategy.rs:961-962). Tightens `loop_iter` to 1 because the supply map is bounded to a single asset.

### `clean_bad_debt_requires_qualification` — `strategy_rules.rs:423-442`
**Action:** **rewrite — the precondition is not equivalent to the production guard.**
- N: 1 rule param + 1 borrow-list read + 1 HF read = 3 symbols.
- L: `clean_bad_debt_standalone` calls `calculate_account_totals` (summarised) then either panics or runs `execute_bad_debt_cleanup` (loops over both maps + 2 pool calls per asset).
- P: relies on `calculate_health_factor_for_summary` (mod.rs:157-172) + `calculate_account_totals_summary` (mod.rs:180-198).

**The defect:** the production qualification check (liquidation.rs:478) is

```
total_debt_usd > total_collateral_usd && total_collateral_usd <= 5*WAD
```

against **unweighted** USD totals from `calculate_account_totals`. The rule
asserts non-qualification by assuming `hf >= WAD` (line 435). Health factor
uses **weighted** collateral:

```
hf = weighted_coll * WAD / total_borrow,    weighted_coll = Σ value × LT_bps/BPS
```

`hf >= WAD` ⇒ `weighted_coll >= total_borrow` ⇒ `total_coll >= weighted_coll >= total_borrow` (because `weighted <= total`). So the assume *is* sufficient to imply `total_debt <= total_collateral`, hence non-qualification. **It is sound but incomplete:** it misses the *other* non-qualification branch — accounts where `total_collateral_usd > 5*WAD` regardless of HF (i.e. underwater accounts above the dust threshold). Such an account has `hf < WAD` AND `debt > collateral` AND `collateral > 5*WAD`; production rejects it (`!(... && collateral <= 5)`), but this rule doesn't cover it.

Also: the body creates a `ControllerCache` at line 427 but `clean_bad_debt_standalone` creates its own internally at liquidation.rs:463. The rule's cache is unused.

- W: medium — depends on `calculate_account_totals_summary` returning a (totalC, totalD, weightedC) triple consistent with `calculate_health_factor_summary`. The two summaries are independently havoced (mod.rs:131-148 vs mod.rs:180-198), so an HF-summary draw of `WAD` and an account-totals draw of `(0, 100, 0)` are simultaneously satisfiable — making this rule's PASS arguably vacuous, since the prover can pick a witness where the HF assume holds but the account-totals would still trigger the panic. Wait — if the account-totals satisfies the panic condition, the rule PASSES (panic ⇒ unreachable `cvlr_satisfy!(false)` ⇒ rule passes). So the disconnect actually **helps** the rule pass; it doesn't cause a false negative. But it means PASS does not actually exercise the path the rule's name claims. **PASS today proves nothing about the HF-implies-non-qualification implication.**

**Proposed change:** drop the HF lever entirely. Use the production predicate directly:
```rust
let (total_coll, total_debt, _) = crate::helpers::calculate_account_totals(
    &e, &mut cache, &account.supply_positions, &account.borrow_positions);
cvlr_assume!(!(total_debt > total_coll && total_coll <= Wad::from_raw(5 * WAD)));
```
This yokes the rule's precondition directly to the production guard, and the
two summary draws are now constrained to a single triple. Adds one more
storage read (`get_account`) but removes one summary call.

### `clean_bad_debt_zeros_positions` — `strategy_rules.rs:450-465`
**Action:** keep but tighten.
- N: 1 rule param + 1 list read = 2 symbols. Cheapest "zeros" rule.
- L: `clean_bad_debt_standalone` only proceeds if `calculate_account_totals` returns a triple satisfying the production guard at liquidation.rs:478. Inside `execute_bad_debt_cleanup`, both supply and borrow position maps are iterated — 1 SAC + 1 pool `seize_position` per asset. Unbounded.
- A: no — assets are symbolic.
- B: `seize_position` returns a position with `scaled_amount_ray = 0` (per pool.rs:299-300 summary, when wired). After both loops, `remove_account` clears storage.
- W: medium — without `seize_position_summary` wired, the position iteration in `execute_bad_debt_cleanup` runs against havoced returns; that doesn't break the rule (the production code calls `remove_account` at liquidation.rs:520 unconditionally after the loops), but if `loop_iter: 12` is reached on an unbounded `for asset in keys()`, the prover times out before reaching the `remove_account`.

**Proposed change:** pin both `account_id` and a single-asset shape: in a test-only setup, write a single supply position and a single borrow position to storage, then call. With one entry per map and `loop_iter: 1`, the unbounded loop collapses. PASS proves the deletion path on a representative account; the multi-asset case is structurally identical (production loops are not branch-bearing).

### `claim_revenue_transfers_to_accumulator` — `strategy_rules.rs:474-484`
**Action:** **delete or merge with sanity.**
- N: 2 rule params + 1 `claim_revenue` return = 3 symbols.
- F: state-invariant. Asserts `amount >= 0`, then `cvlr_satisfy!(amount >= 0)` — both the same expression.
- V: zero. The `claim_revenue_summary` (pool.rs:319-323) constrains `amount >= 0` by construction; the rule is just re-stating the summary's post-condition. The `cvlr_satisfy!` line additionally is reachable iff a single-asset `claim_revenue` doesn't panic, which is already covered by the sanity rule.
- W: medium — without the summary, this is asserting that a fully-havoced i128 is `>= 0`, which fails trivially.

**Proposed change:** delete. The pool-level `claim_revenue_bounded_by_reserves` lives in the solvency suite (per the prior audit). Nothing in this rule is strategy-domain-specific.

### `strategy_blocked_during_flash_loan_multiply` — `strategy_rules.rs:503-532`
**Action:** keep — but consider the duplication.
- N: 7 rule params + 3 compat havocs.
- B: panics at strategy.rs:156 (via `validation::require_not_flash_loaning`) before any meaningful work.
- W: none — panic is in pure controller code reading instance storage; no cross-contract call.

### `strategy_blocked_during_flash_loan_swap_debt` / `swap_collateral` / `repay_with_collateral` — `strategy_rules.rs:535-616`
**Action:** **collapse into one parameterised guard rule.**
- The four "blocked during flash loan" rules each pay the full call-machinery cost (compat shim havocs, parameter symbols) for the same property: `require_not_flash_loaning` panics before any other work. The companion rule `flash_loan_rules::flash_loan_guard_blocks_callers` (flash_loan_rules.rs:80-90) already proves this directly against the helper; *every* mutating endpoint that calls the helper first inherits it.

**Proposed change:** delete all four (`strategy_blocked_during_flash_loan_*`). The single helper-level rule in `flash_loan_rules.rs` covers the shared property. Replace with **one** "structural guard" rule per file that asserts the helper is the *first* statement in each mutating path — but that's a static-analysis check, not a Certora rule. Better: a doc-as-code unit test in `test-harness/tests/` that uses `#[should_panic]` to verify each entrypoint panics when the flag is set.

### Sanity rules — `strategy_rules.rs:622-701`
**Action:** keep.
- All four sanity rules (`multiply_sanity`, `swap_debt_sanity`, `swap_collateral_sanity`, `clean_bad_debt_sanity`) are reachability checks. Cheap; valuable for catching broken summary chains where a path becomes infeasible.

---

## Per-rule classification (flash_loan_rules.rs)

### `flash_loan_fee_collected` — `flash_loan_rules.rs:42-64`
**Action:** **rewrite** — the assertion is vacuous in the current build.
- N: 5 rule params + 2 protocol-revenue snapshots = 7 symbols.
- B: `process_flash_loan` runs `require_not_flash_loaning`, `require_amount_positive`, `require_market_active`, `is_flashloanable` check (flash_loan.rs:40-44), then `pool::flash_loan_begin` + `env.invoke_contract::<()>(receiver, ...)` + `pool::flash_loan_end` (flash_loan.rs:50-64). The receiver invocation is a third-party callback — fully havoced.
- P: relies on `pool::flash_loan_end_summary` (pool.rs:238) — but that summary is empty (no return, no side effect modelled). The summary cannot enforce the revenue-monotonicity post-condition because the summary signature hides the pool's `protocol_revenue` storage.
- V: even with the summary wired, **`revenue_after >= revenue_before`** compares two independent calls to `pool_client.protocol_revenue()` (flash_loan_rules.rs:57, 61). With no `protocol_revenue` summary and no shared per-tx snapshot, both reads return independent havoced i128s. PASS does not prove the bound.
- W: high — and additionally **the rule cannot be made meaningful without joint-summary wiring** (a `protocol_revenue` summary that draws from the same per-tx snapshot as `flash_loan_end_summary` would mutate).

**Proposed change:** the right home for this property is in the **pool** crate's spec, not the controller's. The pool already has unit tests (`pool/src/lib.rs:1181-1186`) for the negative-fee rejection. Add a Certora rule against the pool crate that asserts `protocol_revenue` after `flash_loan_end` is `>=` before, using the pool's *own* storage (no cross-contract layer). Delete this rule from the controller spec; replace with a structural rule that asserts the controller calls `flash_loan_begin` then the callback then `flash_loan_end` in order — covered today by the pool's own internal `FLASH_LOAN_PRE_BALANCE_KEY` invariant (pool.rs:24-42 in summary docs).

If the rule must stay in the controller spec, replace the protocol_revenue
read with a dedicated joint-snapshot summary in `summaries/pool.rs` of the
shape suggested in the solvency-domain audit's *Cross-cutting recommendation
1* (joint pool-views summary).

### `flash_loan_guard_blocks_callers` — `flash_loan_rules.rs:79-90`
**Action:** **keep — best-in-class rule design.**
- N: 1 rule param.
- B: writes the flag, calls the helper, asserts unreachable.
- L: none.
- V: high — the doc comment correctly notes that any mutating endpoint inheriting `require_not_flash_loaning` is covered transitively. This is the right granularity.
- W: none — no cross-contract call.

### `flash_loan_guard_allows_when_clear` — `flash_loan_rules.rs:96-103`
**Action:** keep.
- Same shape as above with the flag cleared. Catches the regression where the helper panics unconditionally. Cheap, valuable.

### `flash_loan_guard_cleared_after_completion` — `flash_loan_rules.rs:117-135`
**Action:** **rewrite — vacuously satisfied on revert paths.**
- N: 5 rule params.
- B: `process_flash_loan` has many revert paths (`require_amount_positive`, `require_market_active`, `is_flashloanable`, the receiver-invocation panic, `flash_loan_end`'s short-repay panic). On any revert, the rule's `cvlr_assert!(!is_flash_loan_ongoing(&e))` is unreachable but Soroban rolls back state — so the guard is implicitly cleared by rollback, not by the production code. The rule passes vacuously on every revert path.

The original P1a flag was on this rule: "vacuously satisfied on revert paths". The current implementation does **not** address this — there is no `cvlr_assume!` constraining the call to a successful path. The doc comment at lines 113-116 says "if the guard remains true after completion, all mutating endpoints would be permanently locked," but Soroban's transactional rollback covers the revert paths automatically; the rule needs to assert the property on the **non-revert** path to be load-bearing.

- W: medium — without `flash_loan_begin_summary` and `flash_loan_end_summary` wired, every successful path also goes through havoced cross-contract calls. The receiver invocation `env.invoke_contract::<()>(...)` at flash_loan.rs:56-60 is a true third-party call; it can panic or return arbitrarily. The summary cannot constrain it.

**Proposed change:** add an assume that excludes the revert paths:
```rust
cvlr_assume!(amount > 0);
cvlr_assume!(!crate::storage::is_flash_loan_ongoing(&e));
// Constrain the asset to a flashloanable, active market with sufficient
// reserves; otherwise the rule is satisfied vacuously by the `is_flashloanable`
// or `require_market_active` panic.
let mut cache = crate::cache::ControllerCache::new(&e, false);
let cfg = cache.cached_asset_config(&asset);
cvlr_assume!(cfg.is_flashloanable);
cvlr_assume!(crate::storage::get_market_config(&e, &asset).status == MarketStatus::Active);
crate::flash_loan::process_flash_loan(&e, &caller, &asset, amount, &receiver, &data);
cvlr_assert!(!crate::storage::is_flash_loan_ongoing(&e));
```
This still passes vacuously when the receiver invocation or `flash_loan_end` panics, but those are **third-party** revert paths that cannot be constrained from the controller's side. Document explicitly that the rule covers the case "callback returned successfully and pool accepted the repayment". Pair with a `flash_loan_guard_cleared_sanity` reachability check that asserts `cvlr_satisfy!(true)` after the call to confirm the non-revert path is actually reached by the prover (it will not be unless wiring + summaries make the callback path feasible).

### `flash_loan_sanity` — `flash_loan_rules.rs:141-152`
**Action:** keep — one-liner reachability check.

---

## Cross-cutting recommendations

### 1. Wire the pool & SAC summaries (P0; gates everything else)

This is restated from the **Headline finding**. Concretely:

- **Pool side:** for each function in `pool/src/lib.rs` that has a summary in `summaries/pool.rs` (`supply`, `borrow`, `withdraw`, `repay`, `update_indexes`, `add_rewards`, `flash_loan_begin`, `flash_loan_end`, `create_strategy`, `seize_position`, `claim_revenue`, `get_sync_data`), wrap the production fn body with `crate::summarized!(controller_path::summary_fn, pub fn original(...) {...})`. The macro expansion (vendor/cvlr-soroban/cvlr-soroban-macros/src/apply_summary.rs:15-33) replaces the body with a forwarding call to the summary under `feature="certora"`, leaving the original body intact otherwise.

  Concern: pool is a separate crate; controller's `summarized!` macro lives in `controller/src/lib.rs:13-17` and references `crate::spec::summaries`. The pool crate would need its own `summarized!` macro pointing at `controller_certora::spec::summaries::pool` — implies a pool→controller dev-time dependency under `feature="certora"`. Acceptable; mirror the pattern established for SAC.

- **SAC side:** the `soroban_sdk::token::Client` is generated; cannot wrap its definition. The closest wiring is to introduce a thin internal `controller::utils::sac::Sac` wrapper that delegates to the SAC client in production and `apply_summary!`s to `summaries::sac::*` under `feature="certora"`. All strategy / flash-loan code paths that use `token::Client::new(...)` should route through this wrapper. ~5-10 call sites; mechanical refactor.

Without this, every PASS in this domain that crosses a `pool_client.X(...)` or `token_client.X(...)` is suspect.

### 2. Compat shim "minimal mode" for negative-path rules

`compat::multiply` (compat.rs:41-92) havocs `account_id`, `initial_payment`, `convert_steps`. That's correct for **positive-path** rules where every branch must be reachable. For **negative-path** rules (`multiply_rejects_same_tokens`, `multiply_requires_collateralizable`, `strategy_blocked_during_flash_loan_multiply`) the panic fires before any of those branches matter — paying for the symbolic exploration is pure overhead.

Add `compat::multiply_minimal(env, caller, e_mode, coll, debt_amt, debt, mode, steps)` that pins `account_id = 0`, `initial_payment = None`, `convert_steps = None` and forwards. Use it in every rule where the property is "reverts at the entry guard". Same pattern for `compat::repay_debt_with_collateral_minimal` (no `close_position` havoc) used by `strategy_blocked_during_flash_loan_repay_with_collateral`.

Rules requiring branch reachability (`multiply_creates_both_positions`, `repay_with_collateral_reduces_both`) keep the full shim — and split per-branch as recommended above.

### 3. Concrete `account_id` and concrete asset addresses

Every strategy rule that reads or writes account state should pin `account_id = 1` instead of accepting it symbolically. Affected rules in this file: `swap_debt_conserves_debt_value`, `swap_collateral_conserves_collateral`, `swap_collateral_rejects_isolated`, `repay_with_collateral_reduces_both`, `clean_bad_debt_*`. Same recommendation applies to assets: `collateral_token`, `debt_token`, `current_collateral`, `new_collateral` should be pinned to fixed addresses in the rule body.

Rationale: the symbolic `account_id: u64` adds a 64-bit dimension to the storage map symbolic state. Pinning it to a concrete value collapses every storage lookup keyed by account-id from "search the whole map" to "single-cell read" in the prover's representation.

### 4. Collapse the four "blocked during flash loan" rules into one

Per the per-rule note on `strategy_blocked_during_flash_loan_*`: these four rules each pay a full mutating-endpoint setup cost to verify the same single-line guard. Since `flash_loan_rules::flash_loan_guard_blocks_callers` already verifies the helper directly, the four endpoint-level rules add zero verification value beyond "the helper is actually called first". The right way to verify "the helper is called first" is structural (a clippy lint, an attribute macro, a compile-time check) — not Certora.

Delete strategy_rules.rs:503-616 (74 lines). Save the prover budget for the heavy positive-path rules.

### 5. Bonus: `clean_bad_debt_requires_qualification` precondition

Replace the HF-based assume (strategy_rules.rs:435) with a direct `calculate_account_totals` call and a negation of the production predicate. Spelled out in the per-rule entry. This eliminates a summary-coupling defect where the HF and account-totals summaries are independently havoced.

### 6. `flash_loan_fee_collected` is mis-located

The protocol-revenue monotonicity property belongs in the **pool** spec, not the controller spec. The controller cannot enforce it without a joint summary on `pool::protocol_revenue`. Move the rule to a future `pool/certora/spec/flash_loan_rules.rs` against the pool crate and delete it here, OR add a joint pool-views summary (per *Cross-cutting #1* of the solvency audit) and rewrite the rule to draw both reads from the same per-tx snapshot.

### 7. `flash_loan_guard_cleared_after_completion` revert-vacuity fix

The rule still has the P1a defect. The proposed assume-based constraint above (per-rule entry) bounds the rule to the non-revert path within the controller's local code, but cannot constrain the third-party callback path. Document this limitation; pair with a sanity rule that confirms reachability.

---

## Multi-step flow strategy

For multi-step entry points (multiply, swap_debt), should we:
(a) Have a single end-to-end rule per entry point (slow, comprehensive)
(b) Split into per-leg rules (flash leg, swap leg, supply leg) — fast, partial
(c) Both — comprehensive rule for the happy path, leg-specific rules for edge cases

**Recommendation: (c), but heavily skewed toward (b) for this protocol.**

The case for (c) over pure (a) is the headline rubric in this audit: **every
end-to-end rule today pays the full call-tree cost** (4 cross-contract calls
+ ~10 SAC ops + 2-3 cache rebuilds + HF re-check) for a single property. With
the prover budget capped at `-maxBlockCount 500000` / `-maxCommandCount
2000000` (strategy.conf:34-35), three or four end-to-end rules saturate the
budget — and most of the time the property under test is something a leg-level
rule could prove cheaply.

The case against pure (b) — leg-only rules — is that the **composition
property** is the one that matters most. "Multiply atomically opens debt AND
deposits collateral" is the real safety property; verifying flash-leg and
deposit-leg separately doesn't prove they happen in the same atomic
transaction. Soroban's transactional rollback gives you that for free as long
as you can prove the entry point doesn't have an early-return path between
the legs — which is itself a structural property, not a Certora-shaped one.

**Concrete split for this protocol:**

| Entry point | End-to-end (slow) rule | Leg-level (fast) rules |
| --- | --- | --- |
| `multiply` | `multiply_create_new_creates_both_positions` (single happy path: account_id=0, no initial payment) | `open_strategy_borrow_creates_borrow_position`, `swap_tokens_returns_at_least_min_out`, `process_deposit_creates_supply_position`, `strategy_finalize_clears_isolated_debt` |
| `swap_debt` | `swap_debt_decreases_old_creates_new` (single happy path) | `open_strategy_borrow_creates_borrow_position` (shared), `repay_debt_from_controller_decreases_debt`, `swap_tokens_returns_at_least_min_out` (shared) |
| `swap_collateral` | `swap_collateral_decreases_old_creates_new` | `withdraw_collateral_to_controller_decreases_supply`, `swap_tokens_returns_at_least_min_out` (shared), `process_deposit_creates_supply_position` (shared) |
| `repay_with_collateral` | `repay_with_collateral_reduces_both_no_close` (split out from current monster rule) | `withdraw_collateral_to_controller_decreases_supply` (shared), `swap_or_net_collateral_to_debt_returns_min_out`, `repay_debt_from_controller_decreases_debt` (shared) |

Notes:

- The leg-level rules are written against the **internal** strategy helpers (`open_strategy_borrow`, `swap_tokens`, `withdraw_collateral_to_controller`, etc., at strategy.rs:407, 745, 908). These are private but `pub(crate)`-accessible from the spec module since `controller/certora/spec/mod.rs` lives inside the crate. Leg-level rules avoid the entry-preamble explosion (auth, flash-loan guard, mode validation, e-mode, isolation) that adds 4-8 branches to the entry-level rule's exploration.

- The shared leg rules (`swap_tokens_returns_at_least_min_out`, etc.) are written **once** and verified **once**, then reused as building blocks. The strategy budget reduces from "4 entry-level rules × full call tree each" to "4 entry-level rules × happy path only + 6 shared leg rules × small call subtree each". Net prover work drops by an estimated 40-60%.

- The end-to-end rule per entry point is kept as the **last line of defense**: it's the only rule that proves "the legs compose atomically." Pin every available concrete value in that rule (account_id, both tokens, amounts) so the symbolic state is minimal — its only job is to verify the composition, not the leg semantics.

- Edge-case branches (`account_id != 0` load-existing branch in multiply, `close_position = true` in repay-with-collateral) get their own dedicated end-to-end rules at the same minimal-symbolic-state shape. **Do not** havoc the optional-input axes inside a single composite rule (current compat shim does this; it's the wrong default for tight budgets).

**Anti-pattern to avoid:** verifying *value conservation* (e.g. "swap_debt
preserves USD debt within slippage tolerance") at the leg level. That property
needs the joint pre/post snapshot the end-to-end rule provides; leg-level
rules can only verify shape (decreases / increases / not-empty). The current
"conserves" rules in this file actually verify shape, not value — rename to
match.

---

## Severity-tagged action items

### P0 (build-system blocker; everything below assumes this is fixed)
- **Wire the pool & SAC summaries** at production sites in `pool/src/lib.rs` and via a `controller::utils::sac` wrapper. Without this, every PASS in this domain that crosses a cross-contract call is non-load-bearing. Per the headline finding, this is the dominant defect in the audit; **do not approve any other recommendation until this lands**.

### P1 (rule-level soundness; high-impact)
- **Fix `flash_loan_guard_cleared_after_completion`** revert-vacuity per the per-rule entry. The P1a flag is unaddressed.
- **Fix `clean_bad_debt_requires_qualification`** precondition — replace the HF-based assume with a direct `calculate_account_totals` predicate negation. The current rule passes vacuously due to the disconnect between the HF and account-totals summary draws.
- **Rewrite or relocate `flash_loan_fee_collected`** — vacuous in the current build; either move to the pool crate's spec or wire a joint protocol-revenue summary.

### P2 (efficiency / budget reduction; high-impact)
- **Split `multiply_creates_both_positions`** into 3 rules (per branch). Removes the 32-path entry-preamble explosion.
- **Split `repay_with_collateral_reduces_both`** into 2 rules (close-position true/false). Removes the unbounded `execute_withdraw_all` loop from the no-close case.
- **Adopt the leg-level rule split** for the four heavy entry points per the *Multi-step flow strategy* table above. Estimated 40-60% prover-work reduction in the strategy domain.

### P2b (efficiency; mechanical wins)
- **Add `compat::multiply_minimal` + `compat::repay_debt_with_collateral_minimal`** for negative-path rules. Removes wasted nondet draws in 5 rules.
- **Pin `account_id = 1` and concrete asset addresses** in every rule that doesn't specifically test the `account_id == 0` create branch. Affects 6 rules in this file.

### P3 (housekeeping)
- **Delete `claim_revenue_transfers_to_accumulator`** — duplicates the summary's own post-condition; covered by sanity.
- **Collapse the four `strategy_blocked_during_flash_loan_*` rules** into the existing helper-level rule in `flash_loan_rules.rs`. The structural "guard called first" property is not a Certora-shaped property.
- **Rename `*_conserves_*` → `*_decreases_old_creates_new_*`** to reflect what the rules actually assert (shape, not value).
