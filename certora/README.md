# Formal verification

This directory contains Certora Sunbeam specifications for the protocol's
highest-risk arithmetic, accounting, solvency, liquidation, oracle, and
strategy properties.

Formal verification complements tests and review. A successful compilation,
submission, or older report is not a proof verdict for the current artifact.
Read the report associated with the exact built WASM and source fingerprint.

## Start here

| Goal | Command |
|---|---|
| Check feature paths, configuration, and rule coverage | ./certora/compile_all.sh |
| Also build and verify Certora WASM provenance | ./certora/compile_all.sh --wasm |
| Build focused prover artifacts | make certora-wasm |
| List available proof profiles | make certora-list |
| Submit the default hosted profile | make certora |
| Submit a chosen profile | make certora CERTORA_PROFILE=fast |

The hosted prover requires `CERTORAKEY` and `certoraSorobanProver` from the
Certora CLI. `make certora` builds the focused WASM first. Build it yourself
before a direct `run_profile.py` or prover call.

## What is covered

| Area | Main question |
|---|---|
| Common | Does fixed-point arithmetic preserve its stated bounds and rounding rules? |
| Pool | Do shares, indexes, cash, revenue, settlement, fees, and flash accounting remain coherent? |
| Controller | Do account actions preserve authorization and solvency requirements? |
| Price system | Do source admission, freshness, tolerance, and fail-closed outcomes hold? |
| Shared summaries | Are cross-contract assumptions explicit and reviewable? |

The common, pool, controller and price-aggregator directories each hold conf
files under `confs/` and rule modules under `spec/`. The shared summaries are
in `shared/summaries/`. The [pool-core guide](pool/spec/README.md) explains the
pool suite in review terms.

## Artifact integrity

Deployable WASM and prover WASM serve different purposes:

| Artifact | Purpose |
|---|---|
| Deploy artifact | Optimized bytecode for deployment and upgrade |
| Focused Certora artifact | Unoptimized bytecode containing one focused rule module |

Focused artifacts reduce prover transformation cost. They do not change
deployed behavior: every focused-verification branch is behind the `certora`
feature, which the deploy build does not enable.

The artifact manifest binds each focused artifact to its source fingerprint and
feature set. Rebuild after changing a contract, rule, fixture, summary, or
relevant dependency. Do not submit a stale artifact.

    make certora-wasm
    python3 certora/scripts/check_wasm_artifacts.py

The focused build disables the Stellar optimizer (`--optimize=false`).
Optimized bytecode can trigger internal prover transformation failures despite
passing ordinary WASM validation.

### Function names must survive the build

The focused build keeps symbols (`CARGO_PROFILE_RELEASE_STRIP=none`); the
deploy build strips them (`strip = "symbols"`). The prover matches its exact
compiler-rt summaries — `__muloti4`, `__multi3`, `__divti3`, `__udivti3`,
`__modti3` — and its soroban-sdk summaries by function name. A stripped module
names every function `FunctionIndex_<n>`, so none of those summaries fire and
the prover analyses inlined 128-bit limb code under bitwise axioms instead.
Every `i128` multiply and divide then costs far more and can produce a spurious
counterexample.

`make certora-wasm` refuses to record an artifact without a WASM `name`
custom section. `check_wasm_artifacts.py` rejects an artifact whose build
provenance is not `"strip": "none"` or that has no `name` section.

## Sanity checking on WASM

`rule_sanity` does not mean on Sunbeam what it means on CVL. The WASM
verification flow builds its vacuity check at level `advanced` and emits it
only when the configured level is at least that, so `basic` runs no vacuity
check and is indistinguishable from `none`. The CVL-level extras
(trivial invariant, assert tautology, redundant require) do not exist on WASM,
so `advanced` costs one extra solve per proved rule, not a multiple.

The suite uses two settings:

| Conf shape | `rule_sanity` | Why |
|---|---|---|
| assert | `advanced` | the only setting that checks the rule is not vacuous |
| satisfy | `none` | a witness is its own reachability evidence |
| revert | `none` | see below |

### Revert-shaped rules and their twins

A revert-shaped rule is `call(...); cvlr_assert!(false);` — it proves that a
gate rejects the call. The TAC vacuity check removes every user assert, asserts
`false` at each sink and re-solves; on this shape no sink is reachable by
construction, so the check reports SANITY_FAILED on a correct rule. Those rules
therefore live in their own `<name>-reverts.conf` at `rule_sanity: none`, with
the same budgets as the conf they were split from.

Turning the check off removes the guard, so each revert-shaped rule is paired
with a satisfy witness that completes the same fixture. Two forms are accepted,
and `check_orphans.py` enforces one of them for every such rule:

- a `<rule>_fixture_completes` twin in the sibling `<name>-reverts-sanity.conf`;
- the module's existing success witness, listed in `EXISTING_WITNESS` in
  `certora/scripts/check_orphans.py`. An entry belongs there only when the
  witness drives the same verb, or is the module's only witness — for example
  `flash_position_sanity` for the `flash_position_rejects_*` family, and
  `flash_loan_guard_allows_when_clear` for `flash_loan_guard_blocks_callers`.

A satisfy conf never runs at a lower `loop_iter` than its assert twin: the
prover rewrites a satisfy rule's generated asserts into assumes, the
loop-unwinding assertion included, so an under-unrolled witness truncates the
search silently instead of failing loudly.

## Budgets

Budgets are set per conf, not by template:

| Class | `smt_timeout` | `prover_args` | `loop_iter` |
|---|---|---|---|
| pure arithmetic and compounding | 600 | `-depth 5 -mediumTimeout 20` | the exact loop count: 1, 8 where `compound_interest` is reached, 6 where `isqrt_of_product` or another fixed loop is |
| host state (pool, controller entrypoint and leg, aggregator endpoint) | 900 | `-depth 10 -splitParallel true -mediumTimeout 20` | measured, floor 28 |
| `math-hard`, `rate-accounting-hard`, `fp-identities-bv`, `strategy-repay-collateral`, `bulk-borrow-duplicate-leg` | 1800 | as their class | as their class |

The `pool-lifecycle` confs are host state at `smt_timeout` 300. Every conf
also sets its own `-maxBlockCount` and `-maxCommandCount`.

Certora's guidance is that a rule unsolved in 600 seconds will not be solved
in 2000 either, so a conf that still times out is a shape problem, not a
budget problem. `-dontStopAtFirstSplitTimeout true` belongs on satisfy confs
and on rules with an expected counterexample; it is not set on any assert conf.
`precise_bitwise_ops` is on only in `fp-identities-bv`, `rate-accounting-hard`,
`scaled-math` and the three `tolerance-math` confs.
Each one answers an observed spurious counterexample. Do not remove one
without a measurement on a named artifact.

`multi_assert_check` is a per-conf choice, not a default. It is on where a rule
carries many asserts and per-assert splitting pays — the pool accounting confs
`pool-state-invariant`, `position-accounting`, `seize-settle-accounting`,
`fee-strategy-accounting`, `flash-loan-accounting`, `pool-guards` — and off
everywhere else, because it makes each assert its own sub-rule and
multiplies a job that proves the same thing. To diagnose which assert of a rule
fails, set it to `true` on that one conf for one run, then set it back.

## Proof profiles

| Profile | Use |
|---|---|
| sanity | Reachability and non-vacuity witnesses, including every `-reverts-sanity` conf |
| fast | Stable math, rate, integrity, and light controller properties, plus the pure-layer `-reverts` confs |
| core | Main audit set: solvency, liquidation, strategies, pool accounting, oracle rules, and the host-state `-reverts` confs |
| heavy | The 1800-second confs outside `core`, plus `lp-math-isqrt` |
| flash-position | Focused flash-position strategy rules: sanity, full, and revert confs |
| manual | Core plus heavy |
| all | Sanity, fast, core, and heavy |

Start with sanity. Run fast or core for a relevant change. Use heavy only for
the targeted surface or an intentional full verification run. A rule runs in
exactly one non-satisfy conf: move a rule between confs, never copy it, or
`check_orphans.py` fails.

`certora/scripts/check_orphans.py` confirms that confs, rules and profiles stay
in sync. It reports, in one pass, every rule that no conf runs, every conf that
names a rule that does not exist, every rule that runs in more than one
non-satisfy conf of its layer, every conf whose `rule_sanity` does not match its
shape, and every revert-shaped rule without a witness. It also rejects a conf
that no profile runs, a host-state conf below `loop_iter` 28, a conf
without `-mediumTimeout` or `-maxCommandCount`, and an allowlist entry that
names no conf. On success it prints the conf, rule and profile counts:

    python3 certora/scripts/check_orphans.py

Extra prover flags follow a double dash. `run_profile.py` passes them to every
conf in the profile and stops at the first non-zero exit. Its own `--dry-run`
flag prints each command and runs nothing:

    ./certora/scripts/run_profile.py fast --dry-run

To prove one rule, run the conf that lists it from that conf's directory:

    cd certora/common/confs
    certoraSorobanProver interest-curve.conf --rule borrow_rate_capped

## What runs on a pull request

`certora-local.yml` runs on pull requests that touch `certora/**`,
`common/src/**`, `contracts/**/src/**`, `Cargo.toml` or `Cargo.lock`. It runs
the local prover on the self-hosted runner over a default set of eight confs,
one of them the pool's `pool-lifecycle`, with a per-rule time cap (900 s by
default). A proved violation, a loop-unwind failure, `SANITY_FAILED`, an empty
or missing rule log, a log with no prover verdict (`ERROR`: a prover, CLI or JVM
error), a runner that stops before the provers, a dispatch `rules` name the
conf does not list, or a missing conf fails the job. A prover-reported timeout
(`<rule>: Solver timed out`), a solver unknown (`<rule>: Solver failed`) and a
wrapper kill are warnings. Each rule log is deleted before its run, so an older
verdict is never read. `certora/scripts/test-run-local-ci.sh` checks this
classification against a stub prover in the build job. A runner without the
local prover install skips the proof with a warning and the job stays green,
so read the job log for the verdict
summary. When the prove step fails, `target/certora-local-logs` is uploaded as
`certora-local-prover-logs-<sha>`. It keeps the prover working directory of each
failed rule. The concrete counterexample, with its `clog!` values, is in
`Reports/Report-<rule>-Assertions-example1.html` under that directory.

`isqrt_is_the_integer_floor_of_the_root` runs from its own `lp-math-isqrt.conf`,
outside the default set, because it cannot be proved under the current model.
The prover treats `context/obj_cmp` on two U256 objects as "equal digests give
0, otherwise a havoc in {1, -1}", so every ordering comparison inside
`isqrt_of_product` is nondeterministic. The reported counterexample
(a = 0x55555555555556, b = 3) does not reproduce against the Rust function. The
conf is in the `heavy` profile because `check_orphans.py` rejects a conf that no
profile runs. Revisit it when the prover models U256 ordering.

`certora-verification.yml` and `certora-fastRules.yml` submit hosted jobs and
run only on manual dispatch. No profile runs automatically on every pull
request.

`certora-fastRules.yml` passes `--wait_for_results ALL`, so each conf's job
blocks until the cloud returns and a violated, vacuous or timed-out rule fails
the workflow. The profile's confs prove one after another and the run stops at
the first failing conf, so a dispatch takes the sum of its confs' prover times.
The `job_timeout_minutes` input caps it (720 by default).

`certora-verification.yml` takes an optional `profile` dispatch input. Left
empty, it runs the `sanity` profile with one job per conf. Given a profile
name, it derives one job per (conf, rule) pair and runs
`certoraSorobanProver <conf> --rule <rule>`, so every rule gets the whole
`global_timeout` and its own report. Each job appends a conf, rule, outcome,
verdict and report row to the run summary. A GitHub matrix caps at 256 jobs, so
the workflow refuses a profile that expands past the cap, such as `all`;
dispatch a narrower profile.

## Local and hosted execution

The profile runner can submit hosted jobs or invoke a local prover installation.
For local execution, provide a compatible Java runtime, Certora CLI
dependencies, and a local prover binary. Build focused WASM first.

    ./certora/scripts/run_profile.py sanity --local

For expensive local rules, use the dedicated local runner. It isolates temporary
prover state, keeps logs under `target/certora-local-logs/`, and refuses a stale
artifact.

    ./certora/scripts/run-rules-local.sh certora/pool/confs/position-accounting.conf

The local runner defaults to one rule at a time and a `-Xmx8g` Java heap. It
drops `-splitParallel true` and sets `multi_assert_check` to false. Raise
`CERTORA_JAVA_HEAP`, pass `-j <n>`, or set `CERTORA_LOCAL_SPLIT_PARALLEL=true`
or `CERTORA_LOCAL_MULTI_ASSERT=true` only after you measure host and solver
capacity.

## How to read a proof result

A verdict applies only within its model.

- Check the artifact fingerprint and configuration used by the report.
- Read assumptions, fixtures, summaries, loop bounds, and rule preconditions.
- Treat a reached cross-contract summary as conditional on that summary.
- Distinguish a counterexample, timeout, loop-unwind failure, and transformation
  error before changing a rule or production code.
- Keep universal assertions and satisfy witnesses separate; reachability is not
  a substitute for a universal property.

## Important proof boundaries

Pool rules directly exercise the accounting transitions used before token
transfers. They do not model arbitrary token behavior, flash callbacks,
allowances, reentrancy, or transaction rollback.

Controller rules use explicit summaries for cross-contract work where a full
composition would be intractable. A controller verdict therefore does not
independently prove the summarized pool, oracle, or position NFT behavior.
Token, swap aggregator, Blend, and flash-position receiver calls have no harness
summary, and a verdict proves nothing about those contracts either.

Price-system rules separate success-path properties from fail-closed outcomes.
Controller valuation rules assume an accepted price unless the rule explicitly
models price failure.

Long unbounded batch processing and arbitrary multi-year accrual loops remain
outside the current proof model when no suitable induction invariant exists.

## Adding or changing a proof

1. State the invariant and threat first.
2. Identify whether it belongs in common, pool, controller, or price-system
   verification.
3. Add a fixture that makes the relevant state reachable.
4. Add a focused rule and a configuration with appropriate sanity policy.
5. Build artifacts and run the static checks.
6. Run the smallest relevant profile or rule, then record the exact report
   and artifact identity.
7. Update the domain guide when the proven boundary or residual assumption
   changes.

Prefer a small lemma before a large stateful rule. Do not hide a timeout by
loosening a property or increasing resource limits without explaining the
change.

## Troubleshooting

| Symptom | First action |
|---|---|
| Artifact-provenance failure | Rebuild focused WASM and run the artifact checker |
| Optimizer-related transformation error | Rebuild with make certora-wasm; it uses unoptimized prover WASM |
| Rule is unreachable or vacuous | Add or repair a satisfy witness before trusting a universal result |
| Expanded-command limit | Review the modeled surface and raise the relevant limit only when justified |
| Local host runs out of memory | Run one rule, keep split parallelism off, and lower Java heap before increasing it |
| Counterexample appears bitwise-spurious | Re-run the targeted rule with precise bitwise modeling |
| SANITY_FAILED on a revert-shaped rule | Expected on that shape; the rule belongs in a `-reverts` conf at `rule_sanity: none` |
| Arithmetic rule times out on every operand | Confirm the artifact carries a `name` section; without it no compiler-rt summary fires |
| Unclear which assert of a rule fails | Set `multi_assert_check: true` on that one conf for one run, then set it back |

## References

- [Certora Sunbeam documentation](https://docs.certora.com/en/latest/docs/sunbeam/index.html)
- [Sunbeam tutorials](https://certora-sunbeam-tutorials.readthedocs-hosted.com/en/latest/)
- [Protocol invariants](../docs/reference/invariants.md)
- [Threat model](../docs/explanation/threat-model.md)
