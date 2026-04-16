# Certora Spike Outputs — Ground Truth, 2026-04-16

Three spikes executed empirically against the current repo. Every claim below is
from CLI output, not conjecture.

## Spike A: Soroban invocation

**Command that works**: `.certora-venv/bin/certoraSorobanProver <conf>`
- Not `certoraRun` (Soroban support removed per Certora changelog)
- Binary is already installed in `.certora-venv/` — no install step needed

**Accepts**: `.conf` files or raw `.wasm`
**Flags confirmed present**: `--compilation_steps_only`, `--build_script`,
`--cargo_features`, `--server`, and a long menu of others.

**Blocker discovered**: running `certoraSorobanProver confs/math.conf` from
`stellar/controller/` fails with:
```
error[E0463]: can't find crate for `core`
  --> cvlr-spec/src/spec.rs:283:9
    |
283 |         cvlr_log::clog!(ctx);
```
This is a **dependency-resolution failure inside `cvlr-spec`**, not in our
rules. The cvlr toolchain cannot compile its own crate against the wasm
target in this environment. No rule ever runs. Every plan that assumed
`certoraSorobanProver math.conf` would produce verdicts was wrong — the
whole tool chain needs triage before anything else happens.

**Next action required to unblock**:
1. File an issue with Certora about `cvlr-spec` build on Soroban with the
   locally installed prover version.
2. OR upgrade `.certora-venv` and rerun.
3. OR self-host a container with a known-working combo of `cvlr` + prover.

**Planning impact**: the entire Certora workstream is gated on resolving
this build error. Until it is, no rule can be verified. `bugs.md` should
record this as **BLOCKING**, and the success claim for formal verification
is currently ZERO — none of the 183 existing rules has actually been run.

## Spike B: Summary syntax

**Real API** (from `.cargo/git/checkouts/cvlr-soroban-*/cvlr-soroban-macros/src/apply_summary.rs`):

```rust
cvlr_soroban_macros::apply_summary!(path::to::spec_fn,
    pub fn original_fn(env: Env, arg: T) -> R {
        // real implementation lives here
    }
);
```

- A `macro_rules!` macro — **not** an attribute like `#[cvlr::summary]`
- Must **wrap the original function definition in-place** at its source site
- Under `--cfg feature="certora"`, the body becomes the summary; under normal
  build, the body remains the real implementation
- Cannot summarize a function you don't own (e.g., `ReflectorClient::prices`)
  without wrapping a local shim around it

**Consequences for any plan**:
- Summaries live in the contract source files, not in
  `controller/certora/spec/summaries/`
- The existing `spec/summaries/mod.rs` is empty because this crate never
  holds summaries — it's a misnamed module
- To summarize a cross-contract call, you need a local wrapper function in
  the controller that delegates to the external client, then wrap THAT
  with `apply_summary!`

Assertion/assume API:
- `cvlr::cvlr_assert!(cond)`
- `cvlr::cvlr_assume!(cond)`
- `cvlr::cvlr_satisfy!(cond)`
- `cvlr::nondet` module for non-deterministic values

## Spike C: Constructor one-shot

Not executed — Spike A blocker prevents running any rule. Deferred.

## Source / Conf Cross-Check

Against 13 conf files and all `*_rules.rs`:
- **Source rules**: 183 `#[rule]` functions
- **Conf-listed rules**: 155 unique names across all 13 confs
- **Unlisted source rules**: 32 — live rules that no conf references. The
  prover cannot produce a verdict for them even if Spike A were fixed.
  Most are `*_sanity` companions; others include the real rules
  `emode_remove_category` and `emode_add_asset_to_deprecated_category`
  (which the 4 orphan conf entries were trying to reference).
- **Orphan conf entries**: 4 — `derived_price_formula`,
  `claim_revenue_decreases_pool_revenue`, `emode_remove_sets_deprecated`,
  `deprecated_category_blocks_asset_addition`. First two are genuinely
  dead; last two are typos for unlisted source rules. See Phase 1 fix.

## Revised Assessment vs v1 / v2 / v3 Plans

All three prior plans assumed the prover ran. **It does not.** Spike A
reveals the prover cannot even compile cvlr-spec against our build today.
Every earlier plan was written against an assumption that none of us had
verified.

Truthful statement of status:
1. Zero rules have been verified on current tooling.
2. The binary and integration points exist, but are broken at build step.
3. The gap between "we have 193 rules" and "rules pass the prover" is
   **the entire toolchain**, not just CI or orphan cleanup.

## What Should Happen Next

Given these findings, the formal-verification workstream needs:

1. **Stop planning; triage the cvlr-spec build error.** Either with Certora
   support, by upgrading the venv, or by self-hosting a known-good stack.
   Expected effort: **~1 day** of communication + trial.

2. **Only after step 1 returns a single working `math.conf` verdict**:
   - Clean the 4 orphan conf entries
   - Assign the 32 unlisted source rules to confs (or delete them)
   - Generate a real baseline

3. **Summaries** (Phase 3 in prior plans) **cannot exist** in
   `certora/spec/summaries/` — they live at each function's source site via
   `apply_summary!`. Delete `summaries/mod.rs` or document it as a
   misleading leftover.

4. **CI integration** cannot be planned until step 1 works. The workflow
   skeleton in v3 points at `certoraSorobanProver`, which is correct, but
   depends on the broken build.

The honest short-term recommendation is: **keep fuzz/Miri/BigRational as
the current formal-ish safety net, flag the Certora integration as
non-functional in `bugs.md`, and return to Certora when either Certora's
toolchain is upgraded here or a separate engineer owns the build triage.**

Delivered ground-truth artifacts:
- This document
- Working knowledge that the binary exists and accepts confs
- Real summary macro syntax that all prior plans got wrong
- The 32 / 155 / 4 / 183 rule numbers that drive any future plan
