# Certora suite review, 2026-09-03

A static review of every Sunbeam spec, conf, harness, summary, script and
workflow under `certora/`, made on branch `test/invariant-gap-hunt` at commit
`36a2ae18`. The prover was not run. Every claim carries one of the evidence
labels from the contributing rules: **Observed** (read in the tree or produced
by a command run here), **Inferred** (follows from code but was not reproduced
with the prover), **Unverified** (needs a prover run to settle).

The companion note on the pinned CLI, the open-source prover's defaults and
its Soroban model is [certora-sunbeam-prover-tuning](certora-sunbeam-prover-tuning.md).
Section numbers written as "note §N" point into it.

> **Correction.** An earlier draft of this review, circulated the same day,
> claimed that fifteen assert rules were vacuous and four sanity witnesses
> unsatisfiable because their fixture left the position books empty. The
> companion note established from prover source that Sunbeam havocs contract
> storage at rule start (note §6, `Contract.kt`), so an unseeded book is
> arbitrary, not empty, and those rules are reachable. That claim is
> withdrawn. What remains is the weaker and more important fact in F1: no
> vacuity check runs on most of the suite, so nobody would have known either
> way.

## 1. What was reviewed, and what was run

| Surface | Count | Evidence |
|---|---|---|
| Conf files | 105 at review time, 126 after section 10 | Observed, parsed with a script |
| Rules | 353 at review time (285 assert, 68 satisfy, 0 mixed), 387 after section 10 | Observed, same classifier as `check_orphans.py` |
| Rule modules | 20 controller, 9 pool, 4 common, 4 price-aggregator | Observed |
| Harness and summary files | 12 | Observed |
| Profiles | 7 (`sanity`, `fast`, `core`, `heavy`, `flash-position`, `manual`, `all`) | Observed |
| Workflows | `certora-local.yml`, `certora-verification.yml`, `certora-fastRules.yml` | Observed |
| Prover artifacts | 34 focused WASM files under `artifacts/wasm/certora/` | Observed, custom sections parsed |

Commands run here, all green:

```
python3 certora/scripts/check_orphans.py        # OK: 105 confs, 353 rules, 7 profiles
python3 certora/scripts/sync_wasm_conf.py --check
./certora/compile_all.sh                         # exit 0 in 45 s, 3 warnings
```

Section 10 records a second, larger set of commands run over the repaired
tree.

The three `compile_all.sh` warnings are an unused `simulate_update_indexes_body`
re-export in `common/src/rates/mod.rs` under the `certora` feature, and two
dead items in the pool under `certora` (`Cache::last_timestamp` and
`prepare_with_balance`). They are harmless but they show the cfg gates are
wider than their users.

Not available as evidence: no prover logs exist locally
(`target/certora-local-logs` is absent), and the single retained
`.certora_internal` run from 2026-08-20 holds a conf and a CLI banner but no
verdict. The git history does record measured facts, and those are quoted
where they matter.

## 2. Summary

**The suite is well engineered as infrastructure and weaker as evidence than
the documentation claims.** The build and provenance tooling is better than
most Certora integrations. Four structural facts limit what the green
verdicts mean:

1. **No vacuity check runs on 70 of the 72 assert confs.** The open-source
   WASM flow emits its vacuity sub-rule only at `rule_sanity: advanced`;
   `basic`, which 66 confs set, emits nothing, and 4 heavy assert confs set
   `none` (note §7, Inferred from source). The 68 satisfy witnesses are the
   only guard against a rule that proves nothing, and many assert rules have
   no witness on their own fixture.
2. **The post-pool solvency gate is a havoc summary in every entrypoint
   rule.** The suite proves that the gate ran on a book, not that its
   arithmetic composes with that book. One rule that asserts the composition
   (`ltv_borrow_bound_enforced`) should produce a counterexample as written.
3. **Most entrypoint rules run over arbitrary, unbounded position books.**
   That is the strongest form of a rule and the most expensive: every
   arbitrary `Map` is iterated under a blanket `loop_iter 32`. Only the health
   family bounds its books to one entry.
4. **The prover artifacts were stripped of names, which silenced the
   prover's exact arithmetic.** The release profile sets `strip = "symbols"`
   and no artifact carried a `name` section. The prover matches its exact
   compiler-rt summaries (`__muloti4`, `__multi3`, `__divti3`) and its
   soroban-sdk summaries by function name (note §9), so every native `i128`
   multiply and divide was analysed as inlined limb code under bitwise
   axioms. The `certora-wasm` recipe now keeps names; one focused build
   confirms the `name` section and the four symbols are back (F4).
5. **A real run shows the cost and a false counterexample.** The local CI
   job of 2026-09-02 ran the seven default confs: of 32 pure-arithmetic
   rules, 10 verified, 21 were killed at the 600 s cap and 1 was violated on
   a provably true property. Every rule that verified never touches a
   symbolic multiply-divide. The stripped names, on the native `i128` path
   that `mul_div_*` gained on 2026-08-26, explain both (section 8).

On parameters: the `heavy` profile is a retry list of rules copied from `core`
at larger budgets, two command caps were lowered below expansions the history
records as measured, `smt_timeout 1800` sits three times above the value the
documentation says is already past useful, `precise_bitwise_ops` switches the
solver race to 256-bit bit-vectors on rules with unbounded inputs, and
`multi_assert_check` plus blanket `loop_iter` values multiply work that a
per-conf measurement would remove. Section 6 gives the per-class plan and a
measurement protocol that needs no local prover.

On the branch: none of the four gap-hunt fixes invalidates a rule. The spec
compiles against the fixed code, and every new gate reads a field the harness
leaves nondeterministic. Two of the fixes have no proof coverage at all, and
the prover has not been re-run on the branch (section 5).

## 3. What is good and must be kept

- **Focused WASM per rule module** (`certora/scripts/focused_wasm.py`): one
  artifact per `*_rules.rs`, so the prover transforms only the code a conf
  needs. The README records why the post-link optimizer is disabled.
- **Provenance manifest with source fingerprint**
  (`certora/scripts/write_wasm_manifest.py`, `check_wasm_artifacts.py`,
  `run-rules-local.sh --verify-artifact`): a stale artifact cannot verify
  green against a tree that no longer matches. The single most valuable
  control in the directory.
- **Static integrity gates** (`check_orphans.py`): orphan conf entries, dead
  rules, unknown conf keys, the `optimistic_loop` policy (the CLI marks that
  flag unsound, note §1), mixed assert/satisfy confs and unprofiled confs all
  fail loudly. Extend it rather than replace it (section 7).
- **Rules over arbitrary state.** Because Sunbeam havocs storage at rule
  start, a rule that seeds only what it needs proves its property for every
  reachable and unreachable ledger, which is the strongest statement the tool
  can make. The frame rules in `account_isolation_rules.rs` and the
  `usage_*` family are written this way.
- **Rule idiom**: explicit input domains nearly everywhere, satisfy witnesses
  paired with implication rules (the `post_gate_*` family), ghost state driven
  from production through `spec_hooks::solvency_gate_checked`, and the
  exhaustive `match` coverage guard over `PositionAction` in `spoke_rules.rs`
  that turns a forgotten verb into a compile error.
- **Pool suite shape**: a state-invariant family over all thirteen pool
  operations, an additivity family with the rounding derivation written down,
  token-authority guards, and a two-sided accrual isomorphism. Pool rules
  drive `ops::*` directly on a seeded market, so they carry no authorization
  or cross-contract noise.
- **Summaries with production-faithful growth semantics**
  (`certora/shared/summaries/pool.rs`): positive supply strictly grows the
  scaled record, positive burns strictly shrink it. The commit that tightened
  them records the reachable violation it removed.
- **Documentation of proof boundaries** in `certora/README.md` and
  `certora/pool/spec/README.md`: what a verdict does and does not establish
  is written where a reviewer looks.

## 4. Findings

### F1. No vacuity check runs on most of the suite

**Mechanism** (note §7, Observed in prover source at `0436a658`, Inferred
for the hosted build). The WASM verification flow builds its sanity checks at
level ADVANCED and emits a check only when the configured level is at least
that. `basic` therefore emits nothing on WASM, unlike the Move and Solana
flows. The `CVT_sanity` import that `cvlr::macros::rule` appends to every rule
has no handler in the WASM front-end either; the artifacts here import it
(Observed) and it lowers to a no-op.

| `rule_sanity` | Confs | Vacuity check on WASM |
|---|---|---|
| `basic` | 66 | none (Inferred) |
| `advanced` | 2 (`pool-guards`, `pool-lifecycle`) | yes |
| `none` | 37 (33 sanity confs, 4 heavy assert confs) | none |

Consequences:

- A rule whose assumptions cut every path, or whose fixture makes every path
  trap, reports VERIFIED with nothing to distinguish it from a real proof.
  The satisfy witnesses in the `sanity` profile are the only guard, and they
  test a verb's reachability, not the assumption set of the assert rule that
  shares its name.
- The 45 revert-shaped rules (`cvlr_assert!(false)` after a call: 35 in the
  controller modules, 2 in the pool, 8 in the price-aggregator; the count
  comes from parsing every rule body) are vacuous by construction to the TAC
  check: it removes user asserts, asserts `false` at every sink, and reports
  SANITY_FAILED when no sink is reachable. Enabling the check flags all 45,
  correctly, so they must be split into `none` confs first.
- The check does not catch implication vacuity: `post_gate_supply_totals_are_final`
  and `post_gate_repay_totals_are_final` assert `!observed || ...` on paths
  where `observed` is always false, as their own witnesses document, and the
  TAC check reports them VERIFIED because their sinks are reachable. Reading
  the witnesses stays necessary.
- The local CI classifier knows only `Violated:` and `Verified:`; with no
  sanity check running there is nothing for it to miss today, and once one
  runs it must learn the SANITY_FAILED line (F9).

Repair, in order:

1. Confirm on one live job that `advanced` produces the rule_not_vacuous_tac
   sub-rules on the hosted prover and that `basic` does not (Unverified).
2. Set `rule_sanity: advanced` on every assert conf. On WASM the CVL-level
   extras (tautology, redundant require) do not exist, so the cost is one
   re-solve per proved rule, not a multiple.
3. Move the 45 revert-shaped rules into confs with `rule_sanity: none`,
   each paired with a satisfy twin that completes the same fixture with the
   gate condition flipped. `flash_position_sanity` next to the four
   `flash_position_rejects_*` rules is the pattern. The twin is the only
   thing that distinguishes a gate proof from an accidental trap: with traps
   modelled as `assume(false)`, any unrelated overflow, storage miss or
   `unwrap` on the fixture proves a revert rule just as well as the gate
   under test.
4. As an independent cross-check, one build with the `cvlr` `vacuity`
   feature turns every rule into `cvlr_assert!(false)` (note §4): a rule that
   still passes has no reachable assert.

### F2. Arbitrary initial state is unbounded in most entrypoint rules

Storage is two havoced maps, values and existence flags, keyed by key digest
and storage type (note §6). A fixture that writes account metadata and no
positions leaves both position maps arbitrary: any length, any keys, any
scaled amounts and risk parameters. Two effects (Inferred):

- **Cost.** Every read of an arbitrary `Map` and every iteration over it is
  unrolled under the conf's `loop_iter`, 28 or 32 on the 55 host-state confs
  (the pure confs run at 1, 6 or 8). This is the likeliest reason the frame
  rules were measured above a million TAC commands and why the host-state
  confs need caps in the millions.
- **Two rules are violable today because of it** (Inferred).
  `supply_preserves_frozen_valuation_health_components` and
  `unhealthy_supply_improves_frozen_valuation_components` value the pre-book
  with its arbitrary `liquidation_threshold`, and `merge_supply_leg` restamps
  it to the listing's 8 000 through `refresh_supply_risk_params`, so weighted
  collateral can fall and `post_weighted >= pre_weighted` fail.
  `bulk_supply_two_assets_both_persisted` asserts `book.len() == 2` over a
  book that may already hold entries.
- **Exposure to unreachable states.** An arbitrary book may hold a position
  with `liquidation_threshold` above `BPS` or a `loan_to_value` above the
  threshold. Frame and direction rules do not care; any future rule that
  values positions with the real risk formulas will.

The health family already handles this with `seed_bounded_account`, which
assumes both maps have at most one entry. The table in Appendix A shows which
rules bound their books. Recommended shape for every entrypoint rule: assume
both maps empty, then seed exactly the positions the property needs, and keep
one deliberately unbounded rule per family as the strong form. This is the
same direction as commit `9a97b570` ("concrete position seeding ... so proofs
are model-independent and finite") applied to the rules it did not reach.

One rule stays non-isolating under any book: `multiply_rejects_same_tokens`
passes a non-empty swap with equal tokens, and `swap_tokens_or_passthrough`
rejects that on `InvalidPayments` before the same-token gate is needed.

### F3. The risk gate is a havoc summary in every entrypoint rule

`contracts/controller/src/risk/totals.rs` wraps
`calculate_account_risk_totals` in `apply_summary!` under the plain `certora`
feature, for every focused build. The summary draws four nondeterministic
WAD totals with only sign and ordering constraints and computes the health
factor from them. Consequences (Observed for the mechanism, Inferred for the
proof strength):

- Entrypoint rules prove control flow and bookkeeping conditional on an
  arbitrary risk oracle. That is the right model for frame rules, the
  `usage_*` family, position-direction rules and the `post_gate_*` fence,
  which only need "the gate ran on this book".
- It is the wrong model for solvency claims. `ltv_borrow_bound_enforced`
  recomputes LTV collateral and debt from fresh nondeterministic prices and
  indexes and asserts the gate's inequality over them, while the gate that
  admitted the borrow saw unrelated numbers. Expect a counterexample, not a
  proof; INV-RISK-01 should not cite it.
- The real body is reachable from a rule as
  `crate::risk::totals::calculate_account_risk_totals::calculate_account_risk_totals`,
  and `iso_health_factor_invariant_across_accrual` already uses it on a
  one-asset book.

Recommended repair: gate the summary on the focused feature. Under
`certora-health-rules` and `certora-solvency-rules` compile the real body;
keep the summary for every other module. Single-asset fixtures make the real
body two or three multiply-divides, inside the budgets those confs already
carry. Measure before widening (section 6.5).

The summary is also stronger than production over the prover's storage
model. It assumes `total_collateral > 0` for any non-empty supply map and
`total_debt > 0` for any non-empty debt map, and `weighted <= total`; an
arbitrary book may hold a zero or negative `scaled_amount` and an unclamped
`liquidation_threshold` above `BPS`, for which the real body computes zero
or a weighted value above the total. Every entrypoint proof therefore rests
on an unstated well-formed-book premise. The premise is true of reachable
state, so this is an abstraction to document and enforce (a fixture helper
that assumes it once), not a live bug. The summary also computes the health
factor with `div_floor` where production uses `div_floor_saturating`, so the
saturating branch is unreachable in every proof.

A second inconsistency in the same layer: `bulk_index_summary` (used by
`Cache::cached_market_index`) and `get_sync_data_summary` (used by
`Cache::cached_pool_sync_data`) draw independent indexes for the same market
inside one `Cache`. No current rule compares the two, so nothing is wrong
today; a future rule that reads both will get a spurious counterexample.

### F4. The prover artifacts carried no function names, so the exact arithmetic summaries never fired

Observed: the release profile in `Cargo.toml` sets `strip = "symbols"`, and
every focused artifact under `artifacts/wasm/certora/` had exactly four
custom sections (`contractspecv0`, `contractenvmetav0`, two `contractmetav0`)
and no `name` section. The `certora-wasm` target built with that profile.

Why it matters (note §9, Observed in prover source at `0436a658`):

- The WASM loader names an unnamed function `FunctionIndex_<n>`, and the
  compiler-rt summaries match on the names `__muloti4`, `__multi3`,
  `__divti3`, `__udivti3`, `__modti3`. With names stripped none of them can
  fire, and neither can the soroban-sdk summaries for `Symbol::new` and the
  `TryFromVal` conversions. The prover then inlines compiler-builtins' limb
  code: a 128-bit multiply becomes six 64-bit multiplies with carries, a
  divide becomes `u128_div_rem` with `clz` normalisation and shift helpers.
- Under the default (non-precise) bitwise modelling, `i64.and`/`or`/`xor`
  are uninterpreted functions with bound axioms only, and `i64.shr_s` has no
  axiom at all. The limb code is full of them.
- On the July shape of `fp_core.rs`, arithmetic went through `I256` host
  calls, which the prover models exactly by import name; import names
  survive stripping. So July verdicts were unaffected. The native `i128`
  fast path added on 2026-08-26 (`bb4b1832`) exposed the gap.

Repair, applied: the `certora-wasm` recipe in the `Makefile` now exports
`CARGO_PROFILE_RELEASE_STRIP=none` for the focused builds only; the deploy
build keeps its stripping. Verified here (Observed): a focused build of
`common` with the `certora-rates-rules` feature under that override carries
a `name` section of 3.2 KB and the symbols `__muloti4`, `__multi3`,
`__divti3`, `__udivti3`; the previous artifact carried none of them.
Rebuild every artifact with `make certora-wasm` before the next submission;
the provenance manifest fingerprints the `Makefile`, so stale artifacts fail
the check.

### F5. Dead and bypassed harness code

Observed by grepping every summary for call sites:

- Not mounted anywhere: `certora/controller/harness/external/sac.rs`,
  `certora/controller/harness/views/aggregates.rs`,
  `certora/controller/harness/oracle_tolerance.rs`.
- Compiled but never called: `certora/shared/summaries/sac.rs` (four
  functions), `certora/shared/summaries/reflector.rs` (three functions),
  and in `pool.rs` the view summaries `reserves_summary`,
  `supplied_amount_summary`, `borrowed_amount_summary`,
  `protocol_revenue_summary`, `capital_utilisation_summary`,
  `pool_snapshot_summary`, `fresh_monotone_index`; in `mod.rs`
  `token_price_summary` and the three `*_in_usd_summary` functions.
- Bypassing the summaries: `certora/controller/harness/storage.rs` builds a
  `LiquidityPoolClient` and calls `get_sync_data` directly in
  `market_index::get_market_index` and `market_params::get_market_params`.
  `index_sanity` reaches the first one. Cross-contract `call` is not
  implemented in the prover and returns a havoced value (note §6), so the
  witness passes on an unconstrained pair rather than on
  `get_sync_data_summary`.
- Token calls are not summarized at all: `payments::balance_delta_since`
  and `strategies::swap_tokens` use `token::Client` directly. Every measured
  receipt in the controller rules is therefore a havoced value. That is
  consistent with the README's boundary statement, but the unused `sac.rs`
  summaries suggest the intent was otherwise.

Delete the dead files and the unused summaries; they are inputs to the
artifact fingerprint and to reviewers' mental model, and they cost both.

### F6. Documentation claims to correct

- `docs/reference/invariants.md`, INV-RISK-01, cites
  `ltv_borrow_bound_enforced` as VERIFIED; under F3 that rule cannot be
  proven as written.
- `certora/controller/spec/README.txt` refers to
  `pool/spec/summary_contract_rules.rs` and `pool/confs/summary-contract.conf`;
  neither file exists. Its proof-ordering section is stale for the same reason.
- `certora/README.md` says `certora-local.yml` "records timeouts as
  warnings", which is true, and should add that no controller or pool conf is
  in the PR default set (F9).

### F7. Duplicated rules and the `heavy` profile

Eleven rules appear in two confs, once in `core`/`fast` and once in `heavy`
with larger budgets (Observed):

| Rule | Light conf | Heavy conf |
|---|---|---|
| `supply_withdraw_roundtrip_error_bounded`, `borrow_repay_roundtrip_error_bounded` | solvency-roundtrip | global-solvency-heavy |
| `liquidation_does_not_increase_repaid_debt`, `liquidation_does_not_increase_seized_collateral` | liquidation | liquidation-integrity-heavy |
| `ideal_repayment_targets_curve_hf`, `estimate_leaves_no_sub_threshold_dust` | liquidation-estimation | liquidation-integrity-heavy |
| `bonus_monotone_in_hf` | liquidation-bonus | liquidation-integrity-heavy |
| `no_collateral_account_cannot_borrow` | market-guard | no-collateral-no-debt |
| `controller_supply_persists_pool_returned_position`, `controller_borrow_persists_pool_returned_position` | controller-pool-consistency-light | controller-pool-consistency |

The `all` profile runs each twice and `manual` nine of the ten
(`liquidation-bonus.conf` is in `fast`, not `core`). A third conf-level
overlap is not a duplicate: `deposit_rate_zero_when_no_utilization` names two
different rules in two crates, so any duplicate check must key on the layer
and the rule. More important, the
pattern records which rules timed out at the light budget (`4761d894`:
"so lighter strategy jobs stay reliable"). That is the timeout list the
suite has been carrying, and the documentation's own guidance is that a rule
unsolved in 600 seconds is unlikely to be solved in 2 000 (note §3). Move a
rule, never copy it, and let `check_orphans.py` reject a rule that appears in
two confs unless the conf is a `*-sanity` twin.

The two roundtrip rules in `solvency_rules.rs` are the same computation as
`div_half_up_roundtrip_error_bounded` in `math_rules.rs` with different
bounds. One rule with the tighter bound is enough.

### F8. Conf hygiene

Observed in the confs, with prover defaults from note §3 and the history that
explains them:

- **Command caps below measured expansions.** Commit `9712215b` recorded
  that `swap_debt` expands to 2.04 M commands, the borrow frame rule to
  1.35 M and its reachability rule to 1.59 M, and set `-maxCommandCount
  4000000` on `health.conf` and `account-isolation.conf`. Commit `c0d8d616`
  lowered both to 2 M, and `health-post-gate.conf` (which runs
  `post_gate_swap_debt_totals_are_final`) was created at 2 M. The prover's
  own default is 1 M commands and 100 k blocks. The controller has been
  refactored since, so the numbers are stale in both directions; the point is
  that the cap was tuned once and then overwritten by a template.
- **`smt_timeout` 900 and 1800.** The prover default is 300 s and the
  documentation says a rule unsolved in 600 s "will not be solved in 2,000
  either" and to simplify instead. The host-state template's 1800 s per leaf
  is a way to wait, not a way to prove.
- **`multi_assert_check: true` on every assert conf but one**
  (`pool-lifecycle-revert.conf` sets it false). Each assert becomes its own
  sub-rule. Pool rules carry eight to twelve asserts, so a conf of twelve
  rules runs on the order of a hundred problems; the documentation offers
  the flag as a timeout mitigation, and on those same pool confs nine small
  queries may well beat one large one. It is a per-conf measurement, not a
  default either way; `run-rules-local.sh` strips it locally, so local and
  cloud runs are not the same job.
- **`rule_sanity`.** See F1: `basic` is the wrong setting on WASM for the
  reason opposite to the one the confs assume.
- **`-dontStopAtFirstSplitTimeout true`** on `health-post-gate.conf`, an
  assert conf. The documentation scopes it to rules with an expected
  counterexample or to satisfy rules; on an assert conf it only lengthens a
  job that has already hit a leaf timeout.
- **Blanket `loop_iter 32`.** `check_orphans.py` requires at least 28 on
  every host-state conf; the floor was introduced in `c0d8d616` with no
  recorded measurement, and the prover default is 1. With
  `optimistic_loop: false` an under-unrolled loop fails loudly as an
  unwinding violation, so the sound procedure is to lower the bound per conf
  until that violation appears and then add one. Pure confs are exempt from
  the floor, but `solvency-roundtrip.conf` and `global-solvency-heavy.conf`
  hold pure rules and still run at 32, as does
  `liquidation-accounting-math.conf`. Bounding the arbitrary books (F2)
  lowers the required value directly. Two caveats: the floor of 28 in
  `check_orphans.py` has to move in the same commit as any measured value,
  and on a satisfy rule the prover rewrites every generated assert,
  including the loop-unwinding assertion, into an assume (note §4), so an
  under-unrolled loop silently truncates the witness search instead of
  failing loudly. A `*-sanity` conf must never run below its assert twin's
  `loop_iter`.
- **`precise_bitwise_ops: true` with unbounded inputs** in
  `rate-accounting-hard.conf`: `protocol_fee_shares_bounded_by_headroom`
  assumes only `fee >= 0` and `supplied >= 0`. The flag turns LIA off and
  switches the race to bit-vector solvers, and `i128` multiply and divide
  are already 256-bit nonlinear terms in the prover's encoding (note §6).
  Commit `9712215b` measured the trade on `math.conf`: 8/8 verified in six
  minutes without bit precision, 4/8 with it.
- **`-depth 15` plus `-destructiveOptimizations twostage`** on the heavy
  arithmetic confs (`math-hard`, `rate-accounting-hard`, `math-bv`). The
  documentation's remedy for nonlinear arithmetic is modularisation and
  under-approximation, not deeper splitting; splitting is the remedy for
  path count.
- **`-mediumTimeout 20`** everywhere doubles the prover default of 10 s,
  which is the documented lever for closing subtrees early. Fine as is.
- Minor: `liquidation-additivity.conf` omits `independent_satisfy` (no
  effect, no satisfy rule); `interest.conf` runs at `loop_iter 1`, which is
  correct because none of its seven rules reaches `compound_interest`;
  `check_orphans.py` parses `--exclude_rule` in profile arguments, a key the
  Soroban CLI does not accept (note §1).

### F9. Process gaps

- **PR-time proving covers seven confs**, all common and price-aggregator
  (`run-local-ci.sh` default set, "measured 2h fit"). No controller or pool
  rule is proven on a pull request. The four controller fixes on this branch
  went through CI without a single controller proof.
- **Timeouts are warnings, and a sanity failure would be a warning too.**
  `run-local-ci.sh` greps for `Violated:` and `Verified:`; any log without
  either becomes `TIMEOUT`. Once F1 is repaired the classifier must learn the
  SANITY_FAILED outcome and treat it as a failure (Unverified string).
- **Cloud results are not recorded anywhere in the repository.** There is no
  ledger of run id, artifact hash, per-rule verdict and time. Every tuning
  decision above was made once and lost; the `.certora_internal` directory
  proves that runs happen and that nothing survives them.
- `certora-cli` is pinned at 8.17.1 and the retained run's banner already
  offered 8.18.0. Pinning is right; a monthly bump with a sanity run is the
  cheap way to keep the pin honest.

## 5. Does the gap-hunt branch invalidate a rule?

Branch `test/invariant-gap-hunt` changes controller behaviour in four
commits. Verdict per commit (Observed for the code and the compile, Inferred
for the prover outcome):

| Commit | Change | Rules that cross the new gate | Effect |
|---|---|---|---|
| `de75b717` | `flash_position` rejects `Normal` mode and non-flashloanable markets | all nine `flash_position_*` rules, `flash_position_sanity` | none: every rule passes `Multiply`, and `is_flashloanable` is a nondeterministic field of `get_sync_data_summary`, so the reachable set is unchanged |
| `d32af685` | borrow and withdraw refuse the pool and the controller as recipient | every rule through `compat::borrow_single` and `compat::withdraw_single`, plus `bulk_borrow_*` | none: the recipient is a nondeterministic address or the caller; the two refused values are pruned, the rest survive |
| `5fa73dd5` | a top-up of a held asset no longer counts against a lowered position limit | `supply_position_limit_enforced`, `borrow_position_limit_enforced`, `bulk_supply_duplicate_asset_counted_once`, `bulk_*_exceed_limit_reverts` | none: every existing rule adds a new slot, which is still gated |
| `a27ea69e` | `Credit(0)` liquidation may create its receiver in a deprecated spoke | `liquidation_does_not_change_other_account_positions`, `usage_liq_credit_*` | none: they pass an existing receiver id |

The certora feature builds against the branch (`compile_all.sh`, exit 0).
The prover has not been re-run; a `sanity` submission is the cheapest
confirmation.

Coverage gaps the branch opened, each a small rule on an existing fixture:

- flash_position_rejects_normal_mode and
  flash_position_rejects_non_flashloanable_market (assert form, on the
  `flash_position_sanity` fixture; the second assumes
  `!cached_pool_sync_data(debt).params.is_flashloanable`).
- borrow_rejects_pool_recipient and withdraw_rejects_controller_recipient
  (assert form, on a bounded, seeded book so that only the recipient gate can
  revert), with a satisfy twin using an external recipient.
- supply_topup_survives_lowered_limit (satisfy: seed ten positions, set
  `max_supply_positions` to five, top up a held asset).
- credit_zero_liquidation_creates_receiver_in_deprecated_spoke (satisfy:
  the `usage_liq_credit_seize_reachable` fixture with the spoke deprecated
  and `SeizeMode::Credit(0)`).

## 6. Timeouts: where the time goes and how to get it back

### 6.1 Cost model (Inferred, with the history's measurements and note §6)

Three cost centres dominate, in this order:

1. **Host-state modelling under a blanket unroll bound.** Every Soroban
   `Map`, `Vec` and storage access is a host call the prover models, storage
   starts arbitrary, and every iteration over an arbitrary collection is
   unrolled `loop_iter` times. `seed_protocol` writes eight keys before a
   rule touches an account, and most entrypoint rules leave both position
   books unbounded (F2). This is why the frame rules were measured above a
   million commands.
2. **Nonlinear `i128` and `I256` arithmetic.** Rust `i128` multiply and
   divide compile to compiler-rt calls that the prover summarises as 256-bit
   nonlinear terms; `I256` host operations are the same terms. `mul_div_half_up`
   takes a native fast path and widens on overflow, so the prover carries
   both branches. Rules with two or three such terms on bounded inputs are
   fine; rules with unbounded inputs, or eight products in a row
   (`compound_interest`), are the ones in `heavy`. The documentation's
   remedy is modularisation and under-approximation (note §8).
3. **Multiplied problems.** `multi_assert_check`, duplicate rules across
   confs, and full-entrypoint driving of single-leg verbs multiply the
   problem count without changing what is proven. Unsummarised SDK helpers
   from the stripped names (F4) add code to every rule.

### 6.2 The ladder, in the order to apply it

The documentation's order is settings, then specs, then source (note §8).
Here the spec-shape items come first because they are also correctness
items.

1. **Turn the vacuity check on** (F1) before tuning anything. A fast rule
   that proves nothing is not a result.
2. **Bound the arbitrary books** in every entrypoint rule (F2): assume both
   maps empty, seed what the property needs. This lowers the unrolled size
   directly and lets `loop_iter` drop.
3. **Keep names in the prover artifacts** (F4). Applied in the `Makefile`;
   rebuild with `make certora-wasm` and re-measure one heavy conf. Without
   names no compiler-rt or SDK summary fires.
4. **Only if a named artifact still times out on arithmetic, give the prover
   one path.** Under `cfg(feature = "certora")` compile only the widened
   `I256` branch of `mul_div_half_up`, `mul_div_floor`, `mul_div_ceil` and
   `mul_div_floor_saturating` (section 8.3). The documentation calls this
   munging; it trades fidelity to the native fast path for one exact,
   bitwise-free branch, so it is the fallback, not the first move.
5. **Drive the internal function, not the entrypoint**, when the property is
   about a leg. `spoke_rules.rs` does this for every `usage_*` rule; the
   `positions.conf` rules could call `process_supply` and `process_borrow`
   directly.
6. **Seed the minimum.** Split `seed_protocol` into the four keys every rule
   needs and the four that only strategy rules need (swap aggregator,
   accumulator, min-borrow floor, position limits).
7. **Measure `loop_iter` per conf.** Lower until the unwinding assertion
   fails, then add one. Record the number in the conf's `msg`.
8. **Bound every nondeterministic input** to the production domain
   (under-approximation in the documentation's terms). `MAX_SUPPLY_INDEX_RAY`
   and a cash ceiling bound the fee-share rules without weakening the claim.
9. **Decompose nonlinear rules into lemmas** (modularisation). One rule for
   the native path of `mul_div_half_up`, one for the widened path, each with
   the branch condition assumed. The `split_*` family in `math_rules.rs` is
   the model.
10. **`precise_bitwise_ops` only where the code has bitwise operators, and
    only judged on named artifacts.** On a stripped artifact the inlined limb
    code makes bit precision the sound setting, and the July measurement that
    argued for dropping it predates the native `i128` path. Keep it where it
    is until the named artifacts have been measured; then re-measure
    `rate-accounting-hard.conf` and `math-bv.conf` with bounded inputs.
11. **`multi_assert_check` per conf, from the ledger.** Keep it on the pool
    accounting confs whose rules carry many asserts, turn it off where a rule
    has one or two, and let a measured run decide the rest.
12. **Use the documented splitting recipes by cause**, not one template:
    `-mediumTimeout 30 -depth 5` when there are many medium subproblems,
    `-smt_initialSplitDepth 5 -depth 15` when the code is very large,
    `-dontStopAtFirstSplitTimeout true -depth 15 -mediumTimeout 5` with a
    short `smt_timeout` only on satisfy confs and on rules expected to fail.
    `-splitParallel true` stays on for cloud jobs.
13. **Lower `smt_timeout` to 600 together with `-depth 5`** on the arithmetic
    confs, where leaves are small; keep 900 on host-state confs and 1800 on
    the few heavy confs with unique rules, where fewer, larger leaves need
    it. The two settings are coupled: a smaller leaf budget with a deeper
    split tree only multiplies unsolved leaves.
14. **Set caps from the report.** Every cloud report prints the command count
    after unrolling. Set `-maxCommandCount` and `-maxBlockCount` to 1.5 times
    the measured value, per conf, and stop copying them.

### 6.3 Per-class targets

| Class | Confs | loop_iter | multi_assert | rule_sanity | bitwise | prover_args |
|---|---|---|---|---|---|---|
| Pure arithmetic, no loops | common `math`, `rates`, `lp-math`; controller `math`, `hf-lemmas`, `boundary-math`, `boundary-oracle`, `liquidation-bonus`, `liquidation-accounting-math`, `scaled-reconstruction`; aggregator `scaled-math`, `freshness` | 1 (6 where `isqrt` or a fixed loop exists) | off | advanced | off | `-mediumTimeout 20` only |
| Compounding | `compound-interest`, `interest-compound`, `rate-indexes`, `rate-index-accounting`, `compound-output`, `boundary-compound-sanity` | 8 (the Taylor loop has seven steps) | off | advanced | off | as above |
| Bit-sensitive | `tolerance-math` | 6 | off | advanced | on | as above, inputs bounded to `1_000 * WAD` |
| Pool accounting on a seeded market | all ten `pool/confs` | measured, expect 12 to 20 | off | advanced | off | `-depth 10 -splitParallel true`, caps measured |
| Controller leg rules | `spoke-usage*`, `spoke.conf`, `positions`, `market-guard`, `flash_loan` | measured | off | advanced | off | `-depth 10 -splitParallel true` |
| Controller entrypoint rules | `health*`, `account-isolation`, `solvency-*`, `strategy-*`, `liquidation*`, `consistency*`, `indexes` | measured, after F2 | off | advanced | off | `-depth 10 -splitParallel true`, caps measured, `smt_timeout 600` |
| Revert-shaped assert rules | new confs split out of the above | as their twin | off | none | as twin | as their twin |
| Satisfy and sanity | every `*-sanity` conf, `bulk-borrow-duplicate-leg` | as its assert twin | off | none | as twin | add `-dontStopAtFirstSplitTimeout true` |

The `heavy` profile should shrink to the rules that still time out after the
ladder, each in its own conf with a recorded measurement.

### 6.4 Solver choice

Defaults and flag names are in note §3. The race runs LIA and NIA solvers by
default; `precise_bitwise_ops` turns LIA off and adds bit-vector solvers;
`-backendStrategy singleRace -smt_useLIA false -smt_useNIA true` is the
documented way to run NIA solvers alone. What follows from this suite's
shape: the default race is right for everything in the pure-arithmetic class;
the NIA-only recipe is worth one measured trial on the multiply-divide
identities (`math-hard`, `rate-accounting-hard`) after their inputs are
bounded; bit-vector solving is justified only on `tolerance-math`; and a rule
that needs bit-vectors and nonlinear reasoning at once on `i128` inputs
should be rewritten before it is re-budgeted.

### 6.5 A measurement protocol that needs no local prover

1. Submit one rule per job with `--rule`, so each rule gets the full
   `global_timeout` and its own report. The documentation recommends exactly
   this when many rules share a job (note §8).
2. From each report record: verdict, sanity sub-rule verdict, wall time,
   commands after unrolling, split count, and whether the unwinding assertion
   fired.
3. Write the numbers into a ledger, `certora/tuning-ledger.md`, keyed by
   rule, conf, artifact sha and CLI version.
4. Set the conf's caps and `loop_iter` from the ledger, and let
   `check_orphans.py` refuse a conf whose caps have no ledger row.

The `certora-verification.yml` sanity matrix already runs one conf per job;
extending the matrix to one rule per job for `core` is a small workflow
change and gives the ledger its first hundred rows.

## 7. Prioritised actions

**P0, correctness of the evidence**

0. Re-base the six position-limit rules on `POSITION_LIMIT_MAX` and add the
   compile-time guard (F10); this is the one finding where a proof family
   is silently dead today.
1. Confirm the WASM sanity behaviour on one live job, then set
   `rule_sanity: advanced` on every assert conf and move the twelve
   revert-shaped rules into `none` confs with satisfy twins (F1).
2. Compile the real `calculate_account_risk_totals` body under
   `certora-health-rules` and `certora-solvency-rules` (F3); expect
   `ltv_borrow_bound_enforced` to need a rewrite once it sees real totals.
3. Bound the arbitrary books in the entrypoint rules that do not (F2,
   Appendix A), keeping one unbounded rule per family.
4. Correct INV-RISK-01 in `docs/reference/invariants.md` and the stale
   references in `certora/controller/spec/README.txt` (F6).

5. Triage the local counterexample on
   `utilization_bounded_when_borrowed_lte_supplied` with a `clog!` witness,
   a bit-precise local re-run and one hosted run (8.2).
6. Rebuild every prover artifact with names kept (`make certora-wasm`,
   applied in the `Makefile`) and re-run `math.conf` and `rates.conf` before
   any other tuning; keep the widened-branch munging of 8.3 as the fallback.

**P1, coverage of the branch and cost of the artifact**

7. Add the six rules listed in section 5 and run `flash-position`, `spoke`
   and `liquidation` confs on the branch artifact before the controller
   upgrade.
8. Record the before/after command count of one heavy conf on the named
   artifacts, and add a `name`-section check to `check_wasm_artifacts.py`
   so a stripped artifact can never be submitted again (F4).
9. Install cvc5 and yices on the self-hosted runner and align the local
   per-rule cap with `smt_timeout` (8.3).

**P2, cost**

10. Apply the ladder in section 6.2 to the eight `heavy` confs first; move,
   do not copy, the surviving rules; delete the duplicates (F7).
11. Set `multi_assert_check` false, `smt_timeout` 600, and remove
   `-dontStopAtFirstSplitTimeout` from `health-post-gate.conf` (F8).
12. Start the tuning ledger and the per-rule `core` matrix (6.5).

**P3, hygiene**

13. Delete the dead harness files and unused summaries (F5).
14. Teach `check_orphans.py` three more rules: no rule in two non-sanity
    confs, every `assert!(false)` rule has a satisfy twin by name, and a
    host-state conf's `loop_iter` must cite a ledger row.
15. Teach `run-local-ci.sh` the sanity-failure outcome and promote it to a
    failure; add one controller conf and one pool conf to the PR default set
    even at a 10-minute cap, so a regression in the seam is at least
    attempted.

## 8. Evidence from the local CI run of 2026-09-02

Job 100421733401 of run 33682000735 (`certora-local.yml`, self-hosted runner,
local prover jar, ten JVMs at 8 GB, 600 s per rule) ran the seven default
confs. Observed from the job log:

| Conf | Verified | Killed at 600 s | Violated |
|---|---|---|---|
| `common/math` | 1 | 6 | 0 |
| `common/rates` | 2 | 1 | 1 |
| `common/lp-math` | 1 | 3 | 0 |
| `common/lp-math-stable` | 2 | 2 | 0 |
| `common/compound-interest` | 1 | 0 | 0 |
| `common/rate-indexes` | 1 | 5 | 0 |
| `price-aggregator/scaled-math` | 2 | 4 | 0 |
| Total | 10 | 21 | 1 |

The ten rules that verified share one property: their assertion never
depends on a symbolic multiply-divide. They are the zero cases
(`utilization_zero_when_supplied_zero`, `deposit_rate_zero_when_no_utilization`,
`scaled_price_allows_zero_operands`), the degenerate-input rejections
(`lp_price_rejects_degenerate_inputs`, both `stable_lp_price_rejects_*`,
`scaled_price_rejects_negative_operands`), the constant identity
`wad_to_ray_preserves_one`, and the two early returns
(`compound_interest_identity_at_zero_delta`, `simulate_indexes_no_time_noop`).
Every rule that multiplies or divides symbolic operands was killed, except
the one that was violated.

### 8.1 Why the arithmetic rules time out now

Causes, in order of evidence strength (note §9):

1. **The exact arithmetic summaries were inert.** The artifacts carried no
   `name` section (F4), so `__muloti4`, `__multi3` and `__divti3` were
   analysed as inlined limb code under bitwise axioms instead of as one
   256-bit term each. Commit `bb4b1832` (#129, 2026-08-26) made this
   reachable: before it, `mul_div_half_up` was four `I256` host calls,
   modelled exactly by import name, and commit `9712215b` had measured
   `math.conf` at 8/8 verified in six minutes on that shape. After it, every
   multiply-divide runs the native `i128` path first and widens only on
   overflow, so the prover carries the inlined limb code for the native
   branch plus the host path for the widened one. The rules, their input
   bounds and `math.conf` are unchanged since July (Observed in git).
2. **The local race is z3 alone.** Solvers are discovered by running
   `z3`, `cvc5`, `yices-smt2` and `bitwuzla --version` from PATH, and the
   default solver list is the available subset; the provisioning step
   requires only `z3`. A z3-only NIA race is five z3 configurations; the
   missing cvc5 and yices configurations are the ones that usually close
   nonlinear queries (note §9.6, Observed in prover source).
3. **The per-rule cap is below the per-query budget.** The wrapper killed
   the JVM at 600 s while the conf handed the solver `-t 900` for a single
   leaf. "TIMEOUT" in this log means "killed by `timeout`", not a prover
   verdict. The local runner now clamps the solver budget under the cap and
   reports a killed run as KILLED (section 8.3).

Ten JVMs on one host also contend for cores, which the July measurement
(one rule at a time) did not.

### 8.2 The violated rule

`utilization_bounded_when_borrowed_lte_supplied` asserts that
`utilization(borrowed, supplied)` lies in `[0, RAY]` for
`0 <= borrowed <= supplied <= 100 RAY`. `utilization` returns
`borrowed.div(env, supplied)`, which is `mul_div_half_up(borrowed, RAY, supplied)`.
On either branch of that function the result is
`floor((borrowed * RAY + floor(supplied / 2)) / supplied)`, at most RAY when
`borrowed <= supplied` because the bias is smaller than one divisor. The
rule and its bounds are unchanged since July (Observed).

Verdict from the prover source (note §9.7, Inferred, not reproduced): a
bitwise over-approximation artifact, root-caused by the inert summaries.
The inlined `__udivti3` normalises the divisor with an `i64.or` of two
non-constant words; under the bound axioms that `or` may be modelled below
its true value, and one inflated 64-bit `div_u` step is enough to push the
quotient above RAY. The multiply side can at most lose carries, which cannot
raise the quotient while `borrowed <= supplied`. The July path carried no
such operators around its exact host calls. The `__muloti4` summary's own
gaps (a truncated high word for negative products, no width metadata on
its stores) are unreachable for non-negative operands and never ran. The
solver set changes speed, not satisfiability.

The witness values were not captured: the rule has no `clog!`, and the
local result table shows no local variables.

Confirmation, in order, without weakening the rule:

1. Rebuild the artifacts with `make certora-wasm` (names are now kept) and
   re-run the single rule locally. With the summaries firing, `__muloti4`
   is one exact 256-bit product with an exact overflow flag and `__divti3`
   one exact signed division, and the bitwise limb code leaves the program.
2. If it still fails, add `clog!(borrowed, supplied, util.raw())` and
   re-run once with `precise_bitwise_ops: true` on the same artifact; that
   separates the bitwise axioms from the solver. A hosted run settles the
   rest.
3. If the witness names a product that fits `i128` on a named artifact,
   check the native branch by hand and by a unit test; that would be a real
   defect and the first the suite has found.

### 8.3 What changed and what remains

Applied in this tree:

- **Names kept in the prover artifacts** (`Makefile`, `certora-wasm`
  recipe), verified on one focused build (F4). Every arithmetic rule in the
  suite benefits; this is the highest-leverage single change in the review.
- **Local solver budget clamped under the wrapper cap and killed runs
  reported as KILLED** (`certora/scripts/run-rules-local.sh`,
  `certora/scripts/run-local-ci.sh`), so the ledger can tell a prover
  timeout from a killed JVM.
- **Solver portfolio reported by the workflow**
  (`.github/workflows/certora-local.yml`): the provisioning step prints the
  versions of `cvc5`, `yices-smt2` and `bitwuzla` and warns for each one
  missing, without making them required.

Still to do on the runner: install cvc5 and yices next to z3 (the prover
accepts any solver whose `--version` answers; the runner's PATH already
includes the install directory). Fallback if the arithmetic rules still time
out on a named artifact: compile only the widened `I256` branch of the four
`mul_div_*` primitives under `cfg(feature = "certora")`, the munging the
Certora documentation recommends for hotspots, and cover the native fast
path with `proptest`. Keep the violation red until step 1 above has run.

## 9. Adversarial re-review: what the first pass got wrong or missed

An independent second reading of the suite against this review produced the
corrections applied above (the revert-rule count, the `loop_iter` and
`multi_assert_check` wording, the duplicate that is two rules, the sequencing
of the bitwise and budget recommendations) and the findings below. Evidence
labels as elsewhere.

### F10. The position-limit proof family is dead: the constant moved from 10 to 5 and the fixtures did not

Observed: commit `634dc8f8` (2026-08-14, "Typos and constant change for
limits") set `POSITION_LIMIT_MAX` to 5 in `common/src/constants/shared.rs`.
`seed_protocol` writes that constant into the position limits, but the rules
still seed ten and nine assets:

| Rule | Fixture | Status today |
|---|---|---|
| `supply_position_limit_enforced`, `borrow_position_limit_enforced` | seed ten distinct assets, assume `seeded == limits.max_*_positions` | assumption unsatisfiable (`seeded >= 10`, limit 5): vacuous |
| `bulk_supply_duplicate_asset_counted_once` | seed nine, supply one, assert `len == 10` | `9 + 1 > 5` traps before the assert: vacuous |
| `bulk_borrow_duplicate_leg_not_double_counted` (satisfy) | seed nine, borrow one | traps before the witness: should be VIOLATED today |
| `bulk_supply_distinct_legs_exceed_limit_reverts`, `bulk_borrow_distinct_legs_exceed_limit_reverts` | seed nine, two new legs | pass, but any single new slot already reverts; the boundary the comments describe is no longer exercised |

Six rules, the whole Certora coverage of INV-RISK-04, which survives on unit
and harness tests only. Severity high for proof coverage, no live protocol
bug. The idiom that hid it is `cvlr_assume!(seeded == N)` on a helper that
returns `len >= N` over an arbitrary pre-map: an impossible value produces
silence, not a failure. Repair (in progress on this branch): derive the seed
counts from the constant, start the fixtures from an empty book, and add a
compile-time guard on `POSITION_LIMIT_MAX` in `fixture.rs` so the next
change breaks the build instead of the proof.

### F11. Direction rules that restate their own summary

`supply_does_not_decrease_position` and `borrow_does_not_decrease_debt`
assert exactly what `supply_summary` and `borrow_summary` assume (`amount > 0`
implies a strictly larger scaled record). The abstraction is faithful to the
pool (`SupplyRoundsToZeroShares` and its borrow mirror), so the rules are not
unsound; they have no residual content beyond the summary. The seam they
could cover, the controller persisting the pool's returned mutation, is
already covered by the two `controller_*_persists_pool_returned_position`
rules. Low severity; retire or re-aim.

### F12. `get_sync_data_summary` leaves the whole rate model unconstrained

The eight rate-model fields of `get_sync_data_summary` are drawn with no
assumption at all: negative slopes, unordered kinks and `max_utilization`
above RAY are admitted, where production validates every one on the write
path. No current rule computes a rate from these fields, so today this is a
latent source of spurious counterexamples rather than a defect; it pairs with
the independent index draws noted under F3. Repair (in progress): mirror the
validated domain in the summary.

### F13. The pool suite proves one market configuration

The pool fixture hard-codes `asset_decimals = 7`, `reserve_factor = 1000`
and one rate curve for the state-invariant family (13 rules), the position
accounting family (12), seize/settle (5) and flash accounting (4); only three
call sites vary anything. Production admits decimals from
`MIN_ASSET_DECIMALS` to `MAX_ASSET_DECIMALS` (3 to 18) and any validated
curve. The pool guide already calls decimals 7 "an instance of the params
domain", so this is a review gap, not an undocumented one. Repair (in
progress): symbolic decimals and reserve factor in the state-invariant
fixture; the curve stays fixed and documented.

### F14. Domain audit

Where a rule's assumed domain is narrower than production, a green verdict
does not generalise. The cases worth knowing:

| Rule or family | Assumed | Production | Verdict |
|---|---|---|---|
| `oracle_rules.rs` price family | prices up to `1_000_000 * WAD` | `MAX_REASONABLE_PRICE_WAD` is 1e9 WAD | 1000x narrower |
| `scaled_math_rules.rs` | factor and quote up to `10_000 * WAD` | 1e9 WAD | 1e5x narrower |
| roundtrip rules, `scaled_to_actual_matches_floor_with_rounding` | index in `[RAY, 10 RAY]` | `[SUPPLY_INDEX_FLOOR_RAW, MAX_SUPPLY_INDEX_RAY]` | upper end 1e8x narrower; the below-RAY floor case excluded |
| controller entrypoint `amount` (51 sites) | `amount <= WAD * 1000` | a token amount at symbolic decimals | unit-mixed ceiling; restate in asset units |
| `USAGE_SEED_MAX`, liquidation `MAX_DEBT_AMOUNT_RAW` | 20 RAY, 1e12 raw | `i128` under caps | fine for delta properties, not for overflow properties |
| `invariant_holds_after_market_create` | decimals up to 27 | 3 to 18 | wider than production; a counterexample at 19..27 would be spurious |
| price-aggregator source summaries | `price > 0`, `timestamp <= now + skew` | identical to `OracleObservation::from_*` | faithful, keep |

### What the second pass confirmed

F1's conf counts (66 / 2 / 37), F3's mechanism, F5 in full, F8's conf facts,
F9, the parameter templates of Appendix B, and the fidelity of the
price-aggregator source summaries. The reviewer also confirmed the reach of
F4: because the twenty `mul_div_*` call sites in `fp.rs` sit under every
`Ray`, `Wad` and `Bps` operator, the stripped names affected every conf, and
the ten rules that verified in the 2026-09-02 job are the complete list of
rules in the suite that never depend on a symbolic multiply-divide.

## Appendix A. Fixture audit of the controller assert rules that drive a verb

"Book bound" says what the rule assumes about the position maps its fixture
does not write: **none** means both maps are arbitrary and unbounded,
**≤ 1** means `seed_bounded_account` or an equivalent assumption,
**exact** means the rule assumes the map length after seeding. "Shape"
marks the revert-shaped rules (`cvlr_assert!(false)`) that a vacuity check
flags by construction.

| Rule | Verb | Seeded positions | Book bound | Shape |
|---|---|---|---|---|
| `supply_does_not_change_other_account_positions` | supply | none | none | assert |
| `borrow_does_not_change_other_account_positions` | borrow | none | none | assert |
| `repay_only_changes_target_account_debt` | repay | none | none | assert |
| `liquidation_does_not_change_other_account_positions` | liquidate, Credit | none | none | assert |
| `controller_supply_persists_pool_returned_position` | supply | none | none | assert |
| `controller_borrow_persists_pool_returned_position` | borrow | none | none | assert |
| `supply_preserves_frozen_valuation_health_components` | supply | none | ≤ 1 | assert |
| `borrow_safe_or_health_gated` | borrow | none | ≤ 1 | assert |
| `withdraw_safe_or_health_gated` | withdraw | none | ≤ 1 | assert |
| `unhealthy_repay_improves_frozen_valuation_components` | repay | none | ≤ 1 | assert |
| `unhealthy_supply_improves_frozen_valuation_components` | supply | none | ≤ 1 | assert |
| `post_gate_supply_totals_are_final` | supply | none | ≤ 1 | assert |
| `post_gate_withdraw_totals_are_final` | withdraw | none | ≤ 1 | assert |
| `post_gate_borrow_totals_are_final` | borrow | none | ≤ 1 | assert |
| `post_gate_repay_totals_are_final` | repay | none | ≤ 1 | assert |
| `post_gate_multiply_totals_are_final` | multiply | fresh account, id arbitrary | none | assert |
| `post_gate_repay_with_collateral_totals_are_final` | repay with collateral | supply and debt | none | assert |
| `post_gate_swap_collateral_totals_are_final` | swap collateral | supply | none | assert |
| `post_gate_swap_debt_totals_are_final` | swap debt | debt | none | assert |
| `supply_does_not_decrease_position` | supply | none | none | assert |
| `borrow_does_not_decrease_debt` | borrow | none | none | assert |
| `withdraw_does_not_increase_position` | withdraw | supply | none | assert |
| `repay_does_not_increase_debt` | repay | debt | none | assert |
| `withdraw_after_borrow_preserves_debt_record` | borrow, withdraw | supply | none | assert |
| `ltv_borrow_bound_enforced` | borrow | none | ≤ 1 | assert, see F3 |
| `supply_rejects_zero_amount`, `borrow_rejects_zero_amount`, `repay_rejects_zero_amount` | supply, borrow, repay | none | none | revert |
| `supply_position_limit_enforced`, `borrow_position_limit_enforced` | supply, borrow | ten, against a limit of five | unsatisfiable (F10) | revert |
| `liquidation_does_not_increase_repaid_debt` | liquidate | supply and debt | none | assert |
| `liquidation_does_not_increase_seized_collateral` | liquidate | supply and debt | none | assert |
| `no_collateral_account_cannot_borrow` | borrow | none | exact (0) | revert |
| `disabled_market_blocks_new_supply` | supply | none | none | revert |
| `supply_new_slot_requires_owner_or_delegate` | supply | none | slot absent | revert |
| `spoke_only_registered_assets`, `spoke_borrow_only_registered_assets`, `spoke_only_borrowable_assets`, `spoke_only_collateralizable_assets` | supply, borrow | none | none | revert |
| `deprecated_spoke_blocks_new_supply`, `deprecated_spoke_blocks_new_borrow` | supply, borrow | none | none | revert |
| `deprecated_spoke_withdraw_does_not_increase_supply` | withdraw | supply | none | assert |
| `bulk_supply_duplicate_asset_counted_once`, `bulk_supply_two_assets_both_persisted` | supply | nine against a limit of five, none | all paths trap (F10), none | assert |
| `bulk_supply_distinct_legs_exceed_limit_reverts`, `bulk_borrow_distinct_legs_exceed_limit_reverts` | supply, borrow | nine against a limit of five | boundary no longer exercised (F10) | revert |
| `usage_*_tracks_scaled_delta` (fourteen rules) | various | the consumed side | none | assert |
| `usage_liq_*` (four assert rules) | liquidation legs | supply and debt | none | assert |
| `iso_update_indexes_writes_no_controller_state` | update indexes | supply and debt | none | assert |
| `swap_debt_preserves_directional_bounds` | swap debt | debt | none | assert |
| `swap_collateral_preserves_directional_bounds` | swap collateral | supply | none | assert |
| `swap_debt_rejects_same_token`, `swap_collateral_rejects_same_token` | swap | none | none | revert |
| `repay_with_collateral_never_increases_positions`, `repay_with_collateral_full_close_clears_debt` | repay with collateral | supply and debt | none | assert |
| `net_settle_pivot_never_leaves_zero_scaled_records` | net settle | supply and debt | none | assert |
| `clean_bad_debt_zeros_positions` | bad-debt cleanup | debt | none | assert |
| `multiply_rejects_same_tokens`, `multiply_requires_collateralizable` | multiply | fresh account | none | revert |
| `flash_position_rejects_*` (four rules), `flash_position_guard_blocks_entrypoint` | flash position | fresh account | none | revert |
| `flash_position_success_leaves_debt_and_supply`, `flash_position_does_not_change_other_account` | flash position | fresh account | none | assert |
| `flash_loan_guard_blocks_callers`, `flash_loan_guard_blocks_supply_entrypoint`, `flash_loan_guard_blocks_liquidation_entrypoint` | guard | none | none | revert |

The twelve revert-shaped rules with an unbounded book are the ones to pair
with a satisfy twin on the same fixture when the vacuity check is turned on.

## Appendix B. Conf parameter matrix, condensed

All 105 confs share `server: prover`, `optimistic_loop: false` (except
`lp-math-stable.conf`), `independent_satisfy: true` (except
`liquidation-additivity.conf`) and `-mediumTimeout 20`. The remaining
parameters fall into five templates; prover defaults for comparison are
`smt_timeout` 300, `-depth` 10, `-maxBlockCount` 100 k, `-maxCommandCount`
1 M (note §3).

| Template | smt / global | depth | caps (block / command) | Confs |
|---|---|---|---|---|
| Light | 900 / 3600 | 5 | 150 k / 750 k | 33 pure and aggregator confs |
| Light, wide | 900 / 3600 | 5 | 300 k / 1.5 M | `rate-index-accounting`, `pool-guards` |
| Host | 1800 / 7200 | 10 | 400 k / 2 M | 55 controller, pool and aggregator confs |
| Host, wide | 1800 / 7200 | 10 | 400 k / 4 M | `position-accounting`, `pool-isomorphism` |
| Heavy | 1800 / 7200 | 15 | 800 k / 3 M | the 8 `heavy` confs and `bulk-borrow-duplicate-leg` |

`pool-lifecycle.conf` and `pool-lifecycle-revert.conf` run at 300 / 1800 with
200 k / 1 M caps. `precise_bitwise_ops` is on in `math-bv`, `tolerance-math`,
`scaled-math` and `rate-accounting-hard`.

## 10. What this pass changed, 2026-09-03

Four reviewers worked the findings in parallel over one checkout: the
controller specs and harness, the common/pool/price-aggregator specs and
shared summaries, the confs and profiles, and the scripts and workflows. This
section is the applied state. The prover was still not run.

### 10.1 Commands run over the repaired tree

```
./certora/compile_all.sh
# OK: 126 confs, 387 source rules, 7 profiles, zero orphans, zero dead rules
# OK: all conf files use canonical focused WASM paths and features
make fmt-check                                 # clean after one cargo fmt --all
make docs-check                                # broken links: 0, unknown symbols: 0
make access-control-check                      # OK 207 entrypoints
cargo clippy --all-targets -- -D warnings      # exit 0
cargo test --workspace                         # 2556 passed, 0 failed
make certora-wasm                              # 32 focused artifacts rebuilt
python3 certora/scripts/check_wasm_artifacts.py  # certora wasm artifacts ok
```

The new gate was itself tested by mutation rather than trusted: setting a
revert conf to `advanced`, flipping `optimistic_loop` to true, and deleting a
witness twin each produced the expected failure, and the tree was restored to
`126 confs, 387 source rules` afterwards.

The three `compile_all.sh` warnings noted in section 1 are unchanged and
pre-existing: `git diff` shows none of the three files is touched by this
branch's working tree.

### 10.2 F4, the root cause, is closed and verified

`make certora-wasm` now exports `CARGO_PROFILE_RELEASE_STRIP=none`, and every
artifact was rebuilt. The four compiler-rt names the prover matches on are
present again:

| Artifact | Size | Matches for `__muloti4`, `__multi3`, `__divti3`, `__udivti3` |
|---|---|---|
| `common-math-rules.wasm` | 60 KB | 8 |
| `common-rates-rules.wasm` | 64 KB | 8 |
| `controller-solvency-rules.wasm` | 228 KB | 7 |

`check_wasm_artifacts.py` now parses the module's custom sections itself and
refuses any artifact without a `name` section, and the manifest records
`"strip": "none"` as provenance. A stripped artifact can no longer reach the
prover through the workflow. This closes P0 action 6 and P1 action 8.

### 10.3 F1, the missing vacuity check, is closed by a gate

Every conf is now classified mechanically by the shape of the rules it names,
and `check_orphans.py` enforces the policy:

| Conf kind | `rule_sanity` | Witness requirement |
|---|---|---|
| assert | `advanced` | none |
| revert (`cvlr_assert!(false)`) | `none` | a satisfy rule per rule, by name or allowlist |
| satisfy | `none` | none |

The classifier reads code only, with comments stripped, so a doc comment
naming a macro cannot change a rule's kind. It also rejects a conf that mixes
shapes. All 47 revert-shaped rules now carry a witness: 20 as a generated
`<rule>_fixture_completes` twin in the paired sanity conf, and 27 through an
explicit `EXISTING_WITNESS` map that points a family of guards on one verb at
that verb's success witness. The gate additionally gained: `optimistic_loop`
must stay false outside one allowlisted conf, `loop_iter` must be positive and
at least 28 on any host-state conf, a sanity conf's `loop_iter` may not fall
below its assert twin's, and `-mediumTimeout` and `-maxCommandCount` must be
present. Counts moved from 105 confs / 353 rules to 126 confs / 387 rules,
almost all of the growth being the 21 new completion witnesses and the 22
native and widened lemmas described below, less the four cross-layer
duplicates removed in 10.7.1.

### 10.4 F3, the havoc risk gate, is closed for the two families that need it

`calculate_account_risk_totals` now compiles as its real body under
`certora-health-rules` and `certora-solvency-rules`, and keeps the havoc
summary everywhere else. Those fixtures are single-asset, so the real body is
two or three multiply-divides. `ltv_borrow_bound_enforced` was rewritten to do
what it always claimed: run the verb, reload the persisted book, recompute the
totals with the real function, and assert that weighted collateral covers debt.
It now has a completion witness, so the implication cannot pass by being empty.

### 10.5 F10, the dead position-limit family, is closed by a build break

`certora/controller/spec/fixture.rs` gained
`const _: () = assert!(common::constants::POSITION_LIMIT_MAX == 5);`, with a
doc comment listing the ten fixtures to re-count when it fires. The fixtures
were re-based from ten seeded assets to five and four. This is the finding
where a proof family was silently dead; the failure mode is now a compile
error rather than a vacuous pass.

### 10.6 The native and widened split, and the violated rule

The 2026-08-26 native `i128` fast path in `fp_core` puts two branches and the
overflow test in front of every fixed-point operation. Eleven arithmetic rules
were replaced by 22 lemmas that each assume one branch, with the split
predicate written as the exact branch condition. For utilization, whose
`Ray::div` lowers to `mul_div_half_up(borrowed, RAY, supplied)`, the native
lemma assumes `borrowed <= (i128::MAX - supplied / 2) / RAY` and the widened
lemma assumes the complement, which is precisely
`borrowed * RAY + supplied / 2` overflowing `i128`. Both halves are non-empty
inside the rule's domain, so neither lemma is vacuous. Both now log
`borrowed`, `supplied` and the raw result, so a repeat of the
2026-09-02 counterexample arrives with values instead of bare `VIOLATED`.

Whether the counterexample survives is still **Unverified**: it needs one
prover run on the rebuilt artifacts. The hypothesis of 8.1 says it will not.

### 10.7 Cloud jobs are now per rule

`certora-verification.yml` takes a `profile` input and expands
`profiles.json` into one job per `(conf, rule)` pair rather than one job per
conf, so each rule receives the whole `global_timeout` and its own report
instead of sharing one budget with its conf mates. The workflow refuses a
profile that expands past GitHub's 256-job matrix cap. With no input it still
runs the historical one-job-per-conf sanity sweep.

Parameters were rewritten per class. The suite now uses five recognisable
settings rather than ad-hoc per-conf numbers:

| Class | `-depth` | `-maxBlockCount` | `-maxCommandCount` | Confs |
|---|---|---|---|---|
| entrypoint | 10 | 400,000 | 2,000,000 | 74 |
| pure arithmetic | 5 | 150,000 | 750,000 | 36 |
| liquidation and multiply | 10 | 400,000 | 4,000,000 | 4 |
| widened arithmetic | 5 | 800,000 | 3,000,000 | 3 |
| bulk and light entrypoint | 10 | 150,000 to 300,000 | 750,000 to 1,500,000 | 5 |
| heavy entrypoint | 10 | 800,000 | 3,000,000 | 2 |
| remaining two | 5 | 300,000 to 400,000 | 1,500,000 to 2,000,000 | 2 |

`-splitParallel true` is set on 114 of the 126 confs, the exceptions being
single-rule confs where a parallel split buys nothing.
`-dontStopAtFirstSplitTimeout true` is limited to the 47 confs that need a
full picture rather than a first failure. The `heavy` profile is no longer a
list of duplicated rules at bigger budgets: the surviving rules were moved,
and the duplicates deleted. Ten confs went with them, together with five dead
harness and summary files (F5): the two SAC and Reflector summaries, the
controller's unused aggregate views, and its unused oracle-tolerance harness.

### 10.7.1 Duplicate rules, checked twice

No rule is now named by two confs of the same layer. That much the gate
enforces. A second scan, keyed on rule *name* across layers, found five more
that the layer-keyed check could never see: the controller's interest module
was re-proving five shared-crate lemmas that the common layer already proves,
through a 228 KB artifact at entrypoint budgets rather than a 60 KB artifact at
arithmetic budgets.

| Controller rule | Common rule with the same property | Action |
|---|---|---|
| `deposit_rate_zero_when_no_utilization` | same name | deleted; the extra domain is above the production rate cap |
| compound_interest_identity | `compound_interest_identity_at_zero_delta` | deleted; the common domain is strictly wider |
| update_borrow_index_monotonic | `update_borrow_index_monotonic_when_factor_gte_one` | deleted, after widening the common factor ceiling to 8 ray |
| update_supply_index_monotonic | `update_supply_index_monotonic_when_rewards_positive` | deleted, after admitting `supplied == 0` in the common lemma |
| `supplier_rewards_conservation` | `supplier_rewards_plus_fee_equals_accrued_interest` | **kept**: it draws symbolic market parameters and additionally pins the fee split, neither of which the common rule does |

The two widenings went in before the deletions, so no domain was lost. Rule
count moved 391 to 387.

**A cost finding this exposed, not yet acted on.** 63 controller rules and 1
pool rule reference no `crate::` item at all: they prove shared-crate
arithmetic through the expensive artifact. That is an upper bound rather than a
work order, because a rule that calls a contract-local helper imported at the
top of the file would also match. Moving them is a real refactor: new confs,
feature wiring, and a different artifact under every one of those proofs. It is
the largest remaining cost lever in the suite and it needs its own change.

### 10.8 Still open

1. **One prover run is the only missing evidence.** Nothing in this section
   was proved; it was compiled, gated and reasoned about. The first run
   should be `math.conf` and `rates.conf` on the rebuilt artifacts, because
   they settle 8.1 and 8.2 together.
2. **cvc5 and yices are still not on the self-hosted runner.** The workflow
   now reports which solvers it found, so the next local run says so in its
   log instead of racing z3 against itself.
3. **F2 is partly open.** The health, solvency, spoke and consistency
   families now bound their books; the frame rules deliberately keep theirs
   unbounded with a well-formedness premise instead.
4. **F13 stands.** The pool suite still proves one market configuration.
5. **The section 5 rules for the gap-hunt branch are still unwritten.**
