# Certora CVLR Spec Audit — 2026-06-16

Doc-grounded audit (Certora Prover / Sunbeam-CVLR semantics) of every rule/spec/summary
under `certora/`. Six parallel auditors + live prover diagnostics. Branch
`feat/certora-blend-hardening`.

Doc semantics applied:
- `cvlr_assert!(c)` holds only on **panic-free** paths reaching it → assert after an
  always-reverting op is vacuously satisfied.
- `cvlr_satisfy!(c)` passes iff some panic-free path reaches it with `c` true →
  `cvlr_satisfy!(false)` is the "must-revert" idiom (passes iff unreachable).
- A summary's `cvlr_assume!` bound must hold for **all** real outputs, else unsound.
- `optimistic_loop` silently drops executions exceeding `loop_iter` (unsound coverage).

---

## THE SYSTEMIC FINDING — "assert-on-summarized-value"

The controller summarizes pool calls (indexes, reserves, utilization, totals, bonus)
to **independent nondet draws** (`certora/shared/summaries/*`, harness/storage.rs →
`get_sync_data_summary`). A large class of controller rules then *asserts a property of
those summarized values*. Two failure modes result:

- **Tautology**: the rule asserts exactly what the summary already `cvlr_assume!`s
  (e.g. index ≥ floor). Proves the summary, not production.
- **Unrelated-nondet**: the rule compares two *independent* summary draws (pre vs post,
  or reserves vs amount) with nothing linking them. The assert is not entailed → it
  either genuinely violates or passes by solver luck.

`health_rules` already avoids this by re-deriving math inline against the *unsummarized*
helper layer. That pattern must be applied to the index/solvency/oracle rules, OR those
invariants must move to the **pool** spec where the real math runs.

---

## HIGH findings

### Cluster A — controller index/solvency rules assert on independent summarized nondets
- `index_rules.rs:10-19` `supply_index_above_floor`, `:21-30` `borrow_index_gte_ray` —
  tautologies (re-assert `get_sync_data_summary` assumes).
- `index_rules.rs:32-46/48-62` `*_index_monotonic_after_accrual` — pre/post are two
  independent nondet draws; `after >= before` not entailed.
- `solvency_rules.rs:141-173` `supply_index_grows_slower` — same unrelated-nondet.
- `solvency_rules.rs:10-22` `claim_revenue_bounded_by_reserves` — `claimed <= reserves`
  over two unrelated nondets.
- `solvency_rules.rs:24-34` `utilization_zero_when_supplied_zero` — `supplied_ray==0`
  assume on one summary can't constrain the `capital_utilisation` summary's return.
- `solvency_rules.rs:53-66` `borrow_respects_reserves` — `reserves >= amount` unrelated.
- `solvency_rules.rs:491-525` `supply_/borrow_respects_*_cap` — summaries discard the
  cap arg; post-view is unrelated nondet, cap not enforced.
  FIX: prove these in the **pool** spec (real math); for relations needing pre/post
  linkage use the existing `fresh_monotone_index`/`nondet_market_index_monotone`
  (shared/summaries/pool.rs) so post is constrained `>=` pre.

### Cluster B — oracle rules assert against summaries that don't model the behavior
- `oracle_rules.rs:242-265` `price_cache_consistency` — harness `token_price_summary`
  ignores `prices_cache`; asserts equality vs a value never read.
- `oracle_rules.rs:88-132` `first_tolerance_uses_safe_price` — band selection is a free
  nondet in the harness; the band-named claim is not what's proven (only input-bounding).
- `oracle_rules.rs:71-84` `price_staleness_enforced` — re-asserts the summary's own
  staleness assume (tautology).
  FIX: make the oracle harness read the cache on hit, or re-scope these to the
  input-bound claim and rely on `tolerance_math_rules.rs` (real math) for band semantics.

### Cluster C — liquidation/bonus tautology + mis-encoded must-revert
- `liquidation_rules.rs:92-109/242-254` `bonus_bounded`, `liquidation_bonus_sanity` —
  `calculate_linear_bonus` resolves to its own summary (`calculate_linear_bonus_summary`)
  which assumes exactly the asserted bounds. Tautology; real `calculate_linear_bonus_with_target`
  never run. FIX: repoint to the real math (as `ideal_repayment_targets_102` does).
- `health_rules.rs:96-123` `liquidation_requires_unhealthy_account` — the `satisfy!(false)`
  must-revert is decoupled: the production liquidation gate runs on the nondet
  `calculate_account_totals` summary, unrelated to the rule's inline healthiness assume,
  so the revert isn't forced by the precondition. Proves nothing. FIX: verify against
  real `calculate_account_totals_body`, or tie inline and gate totals to one symbol.

### Cluster D — summary bounds production can violate / unproven
- `shared/summaries/pool.rs:333-334` `capital_utilisation_summary` assumes `util <= RAY`;
  production exceeds RAY under insolvency (`borrowed > supplied`) and the pool view-soundness
  rule deliberately does NOT assert it. UNSOUND for any consumer relying on the upper bound.
  FIX: drop `util <= RAY` (keep `>= 0`).
- `common/spec/summaries.rs:26` `simulate_update_indexes_summary` `supply_out <= MAX_BORROW_INDEX_RAY`
  is not structurally guaranteed (`update_supply_index` has no clamp) and only proven for one
  chunk with small inputs. FIX: drop the supply upper bound (keep monotone `supply_out >= supply_in`)
  or add a real clamp.

### NEEDS RECONCILIATION (auditors disagreed)
- `shared/summaries/pool.rs:118` `withdraw_summary` `actual_amount <= amount`: the pool
  auditor flagged it unsound (full-close can exceed by a ulp); the summary auditor verified
  it sound (full-close entered only when `amount >= current_supply_actual`, cache.rs:172-216).
  ACTION: definitively re-read `Cache::resolve_withdrawal` before trusting/weakening.

---

## MY OWN NEW-CODE FINDINGS (this session)
- `health_rules.rs:189-245` `borrow_/withdraw_safe_or_health_gated` (NEW) — **near-vacuous**:
  the skolem `nondet_address()` reserve almost always holds NO position, so
  `post_coll>=pre_coll && post_debt<=pre_debt` is `0>=0 && 0<=0` (trivially true) and the
  ghost is never load-bearing. Blend's skolem ranges over the position vector (real
  positions); ours ranges over all addresses (mostly empty). FIX: constrain `reserve` to an
  asset in `pre_account`'s maps OR `== asset`, or assume `pre_coll>0 || pre_debt>0`.
- `rates_rules.rs:183-217` `simulate_indexes_monotone_one_chunk` (NEW) — **intractable**:
  runs the real degree-8 `compound_interest` with symbolic `delta` → 15-min prover wall
  (confirmed live). FIX: pin `delta_ms` concrete, or drop the lemma and rely on the
  compositional argument (`update_borrow_index_monotonic` + `compound_interest_ge_simple`
  + `update_supply_index_monotonic`).

## MED findings (coverage / tractability)
- `solvency_rules.rs:220-288` position-limit rules iterate a list of length up to `max`
  (≤10) under `loop_iter 3` → executions with >3 existing positions silently dropped.
- `health_rules.rs` inline valuation uses half-up/floor that differs from the production
  gate's floor-collateral/ceil-debt → proves a looser inequality than production enforces.
- `compat.rs:29-37` `withdraw_single` pins `to=None` → the `Some(recipient)` withdraw path
  is unverified in every rule using the shim.
- `pool/spec/integrity_rules.rs:129-144` `pool_state_domain_invariant` — tautological
  (asserts seeded literals; no op runs).
- `solvency_rules.rs:424-457`, `interest_rules.rs` compound_* monotonicity rules — real
  degree-8 Taylor with symbolic operands → timeout-prone (the rates.conf wall).
- `isolation_rules.rs:9-43` `utilization_params_ordered` asserts ordering on fully-nondet
  `get_sync_data_summary` params (no ordering assume) → will not hold as written.
- `emode_rules.rs:296-323` `emode_remove_category` loop bounded by `len()<=5` assume;
  confirm `loop_iter` ≥ 5 in emode.conf.
- 5 controller-side summaries in `shared/summaries/mod.rs` (`token_price`,
  `update_asset_index`, `calculate_account_totals`, `calculate_linear_bonus`, `total_*_in_usd`)
  have NO `*_satisfies_*` soundness lemma (trusted on faith, unlike the pool summaries).

## What is SOUND (no action)
- `common/spec/math_rules.rs`, `controller/spec/math_rules.rs` — real fp_core math.
- `interest_rules.rs`, `tolerance_math_rules.rs` — real rate/ratio math, genuine invariants.
- `pool/spec/summary_contract_rules.rs` + `integrity_rules.rs` — the pool summaries ARE
  backed by executable lemmas running the real `LiquidityPool` (the gold-standard pattern).
- `emode_rules` (negative/governance), `position_rules`, `consistency_rules`,
  `market_guard_rules`, `flash_loan_rules`, `account_isolation_rules`, `oracle_compose_rules`
  — must-revert `satisfy!(false)` idiom correctly encoded; happy-path asserts run on real ops.
- view-soundness rules (added this session) — VERIFIED on the cloud prover.

## Prover diagnostics (live, this session)
- view-soundness.conf — VERIFIED ("No errors found"); confirms the `--optimize=false`
  rebuild is healthy in the cloud (no `FunctionIndex_294`).
- rates.conf, rates_reachability, simulate_indexes_monotone_one_chunk — all hit the CLI's
  15-min no-output wall = degree-8 nonlinearity tractability, not (necessarily) real
  violations. Some "violations" may be the genuinely-unsound Cluster-A/B rules above.

---

## Remediation priority
1. **capital_utilisation_summary**: drop `util <= RAY` (live unsound). [1 line]
2. **simulate summary**: drop `supply_out <= MAX_BORROW_INDEX_RAY`; drop/pin the intractable
   monotone lemma. [few lines]
3. **My health-gated skolem**: constrain `reserve` to a held asset. [few lines]
4. **bonus_bounded / liquidation_requires_unhealthy**: repoint to real math. 
5. **Cluster A (index/solvency view rules)**: move pool-internal invariants to the pool
   spec; for pre/post relations use the monotone-index helper. [largest effort]
6. **Cluster B (oracle)**: fix harness cache-read or re-scope the band rules.
7. Reconcile `withdraw_summary` bound; add `withdraw to=Some` shim; add the 5 missing
   controller-summary `*_satisfies_*` lemmas; fix loop_iter vs position-cap.
