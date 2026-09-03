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

The hosted prover requires CERTORAKEY and the Certora command-line tool. Build
the focused WASM before submitting directly or through a profile.

## What is covered

| Area | Main question |
|---|---|
| Common | Does fixed-point arithmetic preserve its stated bounds and rounding rules? |
| Pool | Do shares, indexes, cash, revenue, settlement, fees, and flash accounting remain coherent? |
| Controller | Do account actions preserve authorization and solvency requirements? |
| Price system | Do source admission, freshness, tolerance, and fail-closed outcomes hold? |
| Shared summaries | Are cross-contract assumptions explicit and reviewable? |

Each proof area has configuration files, rule modules, fixtures, and
domain-specific documentation. The [pool-core guide](pool/spec/README.md)
explains the pool suite in review terms.

## Artifact integrity

Deployable WASM and prover WASM serve different purposes:

| Artifact | Purpose |
|---|---|
| Deploy artifact | Optimized bytecode for deployment and upgrade |
| Focused Certora artifact | Unoptimized bytecode containing one focused rule module |

Focused artifacts reduce prover transformation cost. They are not a separate
production behavior: production code has no focused-verification branch.

The artifact manifest binds each focused artifact to its source fingerprint and
feature set. Rebuild after changing a contract, rule, fixture, summary, or
relevant dependency. Do not submit a stale artifact.

    make certora-wasm
    python3 certora/scripts/check_wasm_artifacts.py

The focused build intentionally disables the Stellar optimizer. Optimized
bytecode can trigger internal prover transformation failures despite passing
ordinary WASM validation.

### Function names must survive the build

The focused build keeps symbols (`CARGO_PROFILE_RELEASE_STRIP=none`); the
deploy build still strips them. The prover matches its exact compiler-rt
summaries — `__muloti4`, `__multi3`, `__divti3`, `__udivti3`, `__modti3` — and
its soroban-sdk summaries by function name. A stripped module names every
function `FunctionIndex_<n>`, so none of those summaries fire and the prover
analyses inlined 128-bit limb code under bitwise axioms instead. Every
`i128` multiply and divide then costs far more and can produce a spurious
counterexample.

`check_wasm_artifacts.py` now rejects an artifact whose build provenance is
not `"strip": "none"` or that carries no WASM `name` custom section. The
artifacts built before this change fail both checks, and keeping names changes
every artifact's bytes, so its manifest fingerprint changes too. Run
`make certora-wasm` before the next submission; a stale or stripped artifact
cannot reach the prover.

## Sanity checking on WASM

`rule_sanity` does not mean on Sunbeam what it means on CVL. The WASM
verification flow builds its vacuity check at level `advanced` and emits it
only when the configured level is at least that, so **`basic` runs no vacuity
check at all** and is indistinguishable from `none`. The CVL-level extras
(trivial invariant, assert tautology, redundant require) do not exist on WASM,
so `advanced` costs one extra solve per proved rule, not a multiple.

The suite therefore uses two settings and no third:

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

Two classes, set per conf and not by template:

| Class | `smt_timeout` | `prover_args` | `loop_iter` |
|---|---|---|---|
| pure arithmetic and compounding | 600 | `-depth 5 -mediumTimeout 20` | the exact loop count: 1, 8 where `compound_interest` is reached, 6 where `isqrt` or another fixed loop is |
| host state (pool, controller entrypoint and leg, aggregator endpoint) | 900 | `-depth 10 -splitParallel true -mediumTimeout 20` | measured, floor 28 |
| the remaining heavy confs with unique rules | 1800 | as their class | as their class |

The documentation's own guidance is that a rule unsolved in 600 seconds will
not be solved in 2000 either, so a conf that still times out is a shape
problem, not a budget problem. `-dontStopAtFirstSplitTimeout true` belongs on
satisfy confs and on rules with an expected counterexample; it is not set on
any assert conf. `precise_bitwise_ops` stays exactly where the history put it —
`tolerance-math`, `scaled-math`, `math-bv`, `rate-accounting-hard` — because
each was added after an observed spurious counterexample. Do not remove one
without a measurement on a named artifact.

`multi_assert_check` is a per-conf choice, not a default. It is on where a rule
carries many asserts and per-assert splitting pays — the pool accounting confs
`pool-state-invariant`, `position-accounting`, `seize-settle-accounting`,
`fee-strategy-accounting`, `flash-loan-accounting`, `pool-guards` — and off
everywhere else, because each assert otherwise becomes its own sub-rule and
multiplies a job that proves the same thing. To diagnose which assert of a rule
fails, set it to `true` on that one conf for one run and set it back; the
tuning ledger, not a template, settles the steady-state value.

## Proof profiles

| Profile | Use |
|---|---|
| sanity | Reachability and non-vacuity witnesses, including every `-reverts-sanity` conf |
| fast | Stable math, rate, integrity, and light controller properties, plus the pure-layer `-reverts` confs |
| core | Main audit set: solvency, liquidation, strategies, pool accounting, oracle rules, and the host-state `-reverts` confs |
| heavy | The confs whose rules are unique to them and still need larger budgets |
| flash-position | Focused flash-position strategy rules: sanity, full, and revert confs |
| manual | Core plus heavy |
| all | Sanity, fast, core, and heavy |

Start with sanity. Run fast or core for a relevant change. Use heavy only for
the targeted surface or an intentional full verification run. A rule runs in
exactly one non-satisfy conf: move a rule between confs, never copy it, or
`check_orphans.py` fails.

The repository holds 389 rules across 126 conf files and 7 profiles. Reproduce
those numbers with:

    grep -rh '#\[rule\]' certora --include='*.rs' | wc -l
    find certora -name '*.conf' -not -path '*/.certora_internal/*' | wc -l

The `-not -path` filter skips the gitignored `.certora_internal/` prover build
directory, which holds copies of confs that are not part of the suite.

`certora/scripts/check_orphans.py` confirms that confs, rules and profiles stay
in sync. It reports, in one pass, every rule that no conf runs, every conf that
names a rule which no longer exists, every rule that runs in more than one
non-satisfy conf of its layer, every conf whose `rule_sanity` does not match its
shape, and every revert-shaped rule without a witness:

    python3 certora/scripts/check_orphans.py

Extra prover flags follow a double dash:

    ./certora/scripts/run_profile.py fast -- --dry-run

`run_profile.py` passes those flags to every conf in the profile and stops at
the first non-zero exit. To prove one rule, run its own conf directly rather
than a whole profile, because the rule exists in only some confs:

    certoraSorobanProver interest.conf --rule borrow_rate_capped
      # from certora/controller/confs/

## What runs on a pull request

`certora-local.yml` runs on pull requests that touch `certora/**` or
`common/src/**`. It invokes the local prover over a fixed set of confs with a
per-rule time cap, and treats proved violations and tooling errors as failures
while recording timeouts as warnings.

`certora-verification.yml` and `certora-fastRules.yml` submit hosted jobs and
run only on manual dispatch. No profile runs automatically on every pull
request.

`certora-verification.yml` takes an optional `profile` dispatch input. Left
empty it runs the historical sanity sweep, one job per conf. Given a profile
name it derives one job per (conf, rule) pair and runs
`certoraSorobanProver <conf> --rule <rule>`, so every rule gets the whole
`global_timeout` and its own report; each job appends a conf/rule/outcome/report
row to the run summary, which is what the tuning ledger is filled from. A
GitHub matrix caps at 256 jobs, so `all` (389 rules) is refused with a message;
dispatch a narrower profile.

## Local and hosted execution

The profile runner can submit hosted jobs or invoke a local prover installation.
For local execution, provide a compatible Java runtime, Certora CLI
dependencies, and a local prover binary. Build focused WASM first.

    ./certora/scripts/run_profile.py sanity --local

For expensive local rules, use the dedicated local runner. It isolates temporary
prover state and retains logs under the build directory.

    ./certora/scripts/run-rules-local.sh certora/pool/confs/position-accounting.conf

The local runner defaults to conservative parallelism and an 8 GiB Java heap.
Increase CERTORA_JAVA_HEAP, enable split parallelism, or enable per-assert
diagnostics only after measuring available host and solver capacity.

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
independently prove the summarized pool, oracle, token, or external-call
behavior.

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
