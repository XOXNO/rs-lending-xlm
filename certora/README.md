# Formal verification

Certora Sunbeam rules for the protocol's arithmetic, accounting, solvency,
liquidation, oracle and strategy properties.

A verdict is valid only for the WASM it ran on. A compile, a submission or an
older report is not a proof of the current code.

| Directory | Rules for |
|---|---|
| `common/` | Fixed-point math, rate curve, indexes, LP pricing. See [common/spec/README.md](common/spec/README.md) |
| `pool/` | Shares, indexes, cash, revenue, bad debt, fees, flash loans. See [pool/spec/README.md](pool/spec/README.md) |
| `controller/` | Entrypoint gates, authorization, solvency, liquidation, strategies. See [controller/spec/README.md](controller/spec/README.md) |
| `price-aggregator/` | Source admission, freshness, tolerance, fail-closed pricing |
| `shared/summaries/` | Cross-contract summaries the controller rules assume |

Verdicts are conditional; see [What a verdict does not prove](#what-a-verdict-does-not-prove).

Each contract directory has confs in `confs/` and rules in `spec/`. To find
the conf that runs a rule:

    grep -l <rule_name> certora/*/confs/*.conf

## Run

| Goal | Command |
|---|---|
| Static checks: feature paths, conf wiring, rule coverage | `./certora/compile_all.sh` |
| Same, plus build and check the prover WASM | `./certora/compile_all.sh --wasm` |
| Build the prover WASM | `make certora-wasm` |
| List profiles | `make certora-list` |
| Submit a profile to the hosted prover | `make certora CERTORA_PROFILE=fast` |
| Print the commands of a profile | `./certora/scripts/run_profile.py fast --dry-run` |
| Prove one rule | `cd certora/common/confs && certoraSorobanProver interest-curve.conf --rule borrow_rate_capped` |

The hosted prover needs `CERTORAKEY` and `certoraSorobanProver` from the
Certora CLI. `make certora` builds the WASM first; a direct `run_profile.py` or
prover call does not. Arguments after `--` go to every conf of the profile.

A local prover install (Java, Certora CLI dependencies, prover binary) can run
a profile with `run_profile.py <profile> --local`, or one conf with
`./certora/scripts/run-rules-local.sh <conf>`. The local runner proves one rule
at a time with an 8 GB heap, turns off `-splitParallel` and
`multi_assert_check`, writes logs to `target/certora-local-logs/`, and refuses
a stale artifact. `CERTORA_JAVA_HEAP`, `-j <n>`,
`CERTORA_LOCAL_SPLIT_PARALLEL` and `CERTORA_LOCAL_MULTI_ASSERT` change this.

## Profiles

| Profile | Contents |
|---|---|
| `sanity` | Reachability witnesses, including every `-reverts-sanity` conf. Run this first |
| `fast` | Stable math, rate and light controller rules, plus the pure-layer `-reverts` confs |
| `core` | Main audit set: solvency, liquidation, strategies, pool accounting, oracle, host-state `-reverts` confs |
| `heavy` | The 1800 s confs outside `core`, plus `lp-math-isqrt` (expected to fail; see Known prover limits) |
| `flash-position` | Flash-position strategy rules |
| `manual` | `core` + `heavy` |
| `all` | `sanity` + `fast` + `core` + `heavy` |

`certora-fastRules.yml` offers `fast`, `core`, `heavy`, `sanity` and `all`. The
`flash-position` and `manual` profiles run only through
`certora-verification.yml` or `run_profile.py`. A `heavy` or `all` run always has
one expected failure: `lp-math-isqrt`.

`certora/scripts/check_orphans.py` keeps confs, rules and profiles in sync. It
fails when:

- no conf runs a rule, or a conf names a rule that does not exist;
- a rule runs in more than one non-satisfy conf of its layer (move rules, do
  not copy them);
- a conf is in no profile;
- `rule_sanity` does not match the rule shape, or a revert-shaped rule has no
  witness;
- a host-state conf has `loop_iter` below 28, or a conf has no
  `-mediumTimeout` or `-maxCommandCount`.

## CI

**`certora-local.yml`** runs on pull requests that touch `certora/**`,
`common/src/**`, `contracts/**/src/**`, `Cargo.toml` or `Cargo.lock`. It proves
a default set of six confs on the self-hosted runner, 900 s per rule.
Five of them are common confs and one is a price-aggregator conf. A green run
says nothing about the controller or the pool. Dispatch `certora-fastRules.yml`
or `certora-verification.yml` for those.

- Fails on: a violation, a loop-unwind failure, `SANITY_FAILED`, a missing or
  empty rule log, a log with no verdict, a missing conf, or a dispatch rule the
  conf does not list.
- Warns on: solver timeout, solver unknown, wrapper kill.
- **Stays green with only a warning when the runner has no prover install.**
  Read the verdict summary in the job log.
- On failure it uploads `certora-local-prover-logs-<sha>`. The counterexample
  with its `clog!` values is in `Reports/Report-<rule>-Assertions-example1.html`.

**`certora-fastRules.yml`** and **`certora-verification.yml`** submit hosted
jobs on manual dispatch only.

- `certora-fastRules.yml` proves the confs of a profile one after another and
  stops at the first failure. `job_timeout_minutes` caps it (default 720).
- `certora-verification.yml` runs `sanity`, one job per conf, when `profile` is
  empty. With a profile it runs one job per (conf, rule) pair and writes a row
  per rule to the run summary. A GitHub matrix holds at most 256 jobs, so `all`
  is refused.

## Build artifacts

| Artifact | Build |
|---|---|
| Deploy | Optimized, symbols stripped |
| Prover | `--optimize=false`, one rule module, `certora` feature on |

The `certora` feature is off in the deploy build, so prover-only code never
ships. The optimizer is off because optimized bytecode can crash the prover's
transformations.

`make certora-wasm` writes a manifest that binds each artifact to its source
fingerprint and features. Rebuild after any change to a contract, rule,
fixture, summary or dependency, then check:

    python3 certora/scripts/check_wasm_artifacts.py

### Function names must survive the build

The prover build keeps symbols (`CARGO_PROFILE_RELEASE_STRIP=none`). The prover
applies its compiler-rt summaries (`__muloti4`, `__multi3`, `__divti3`,
`__udivti3`, `__modti3`) and its soroban-sdk summaries by function name. In a
stripped module every function is `FunctionIndex_<n>`, no summary applies, and
each `i128` multiply or divide becomes slow bit-level code that can give a
false counterexample.

`make certora-wasm` refuses a module without a `name` custom section.
`check_wasm_artifacts.py` rejects one without that section or without
`"strip": "none"` provenance.

## Sanity checking on WASM

On Sunbeam, `rule_sanity: basic` does not run a vacuity check; it is the same
as `none`. Only `advanced` runs it, at the cost of one extra solve per proved
rule.

| Rule shape | `rule_sanity` |
|---|---|
| assert | `advanced` |
| satisfy | `none` (the witness proves reachability) |
| revert | `none` |

A revert-shaped rule is `call(...); cvlr_assert!(false);`. It proves that a
gate rejects the call. The vacuity check reports `SANITY_FAILED` on this shape
even when the rule is correct, so these rules live in a separate
`<name>-reverts.conf` with the same budgets as the conf they came from.

Without the vacuity check, each revert rule needs a satisfy witness that
completes the same fixture. `check_orphans.py` accepts one of:

- a `<rule>_fixture_completes` twin in the sibling `<name>-reverts-sanity.conf`;
- an existing witness of the module, listed in `EXISTING_WITNESS` in
  `check_orphans.py`. Add an entry only when the witness drives the same verb
  or is the only witness of the module.

A satisfy conf must not use a lower `loop_iter` than its assert twin. The
prover turns a satisfy rule's asserts into assumes, the loop-unwind assert
included, so a low bound cuts the search without an error.

## Budgets

| Class | `smt_timeout` | `prover_args` | `loop_iter` |
|---|---|---|---|
| Pure arithmetic | 600 | `-depth 5 -mediumTimeout 20` | the exact loop count: 1, or 8 where `compound_interest` is reached, or 6 where `isqrt_of_product` or another fixed loop is |
| Host state (pool, controller, aggregator) | 900 | `-depth 10 -splitParallel true -mediumTimeout 20` | measured, at least 28 |
| `math-hard`, `rate-accounting-hard`, `fp-identities-bv`, `strategy-repay-collateral`, `bulk-borrow-duplicate-leg` | 1800 | as their class | as their class |

`pool-lifecycle` confs are host state at 300 s. Every conf sets its own
`-maxBlockCount` and `-maxCommandCount`.

- A rule that does not solve in 600 s will not solve in 2000 s. Change the rule
  shape, not the budget.
- `-dontStopAtFirstSplitTimeout true` is for satisfy confs and expected
  counterexamples only, never an assert conf.
- `precise_bitwise_ops` is on only in `fp-identities-bv`,
  `rate-accounting-hard`, `scaled-math` and the three `tolerance-math` confs.
  Each fixes an observed false counterexample. Do not remove one without a
  measurement.
- `multi_assert_check` is on only in the six multi-assert pool confs (see the
  pool guide). To find which assert fails elsewhere, turn it on for one run.

## Known prover limits

- `pool-lifecycle` and `lp-math-stable` run on the hosted prover only. Locally,
  `market_create_writes_zeroed_state` reports a violation,
  `accrue_is_noop_when_no_time_elapsed` gives a rule-encoding error, and every
  `lp-math-stable` rule gives an internal error.
- `isqrt_is_the_integer_floor_of_the_root` (`lp-math-isqrt.conf`) cannot be
  proved. The prover compares two U256 values as "equal digests give 0,
  otherwise any of 1 or -1", so every comparison in `isqrt_of_product` is
  nondeterministic. Its counterexample (a = 0x55555555555556, b = 3) does not
  reproduce in Rust. The conf is in `heavy` only because every conf must be in
  a profile.

## What a verdict does not prove

- Pool rules check the accounting before token transfers. They do not model
  token behavior, flash callbacks, allowances, reentrancy or rollback.
- Controller rules assume the pool, oracle and position NFT summaries. A
  verdict that reaches a summary is conditional on it. Token, swap aggregator,
  Blend and flash-position receiver calls have no summary, so a verdict says
  nothing about them.
- Controller valuation rules assume an accepted price unless the rule models a
  price failure.
- Long batch loops and multi-year accrual are out of scope where no induction
  invariant exists.

Before acting on a result, confirm the artifact fingerprint, then tell apart a
counterexample, a timeout, a loop-unwind failure and a transformation error.
A satisfy witness shows reachability only; it does not prove a universal
property.

## Adding a proof

1. State the invariant and the threat.
2. Put the rule in the right layer: common, pool, controller or
   price-aggregator.
3. Add a fixture that makes the state reachable, the rule, a conf with the
   correct `rule_sanity`, and a witness.
4. Run `./certora/compile_all.sh --wasm` and `check_orphans.py`, then the
   smallest profile or rule that covers it.
5. Update the layer guide when a proof boundary or assumption changes.

Prefer a small lemma to a large stateful rule. Do not fix a timeout by
weakening the property.

## Troubleshooting

| Symptom | Action |
|---|---|
| Provenance check fails | `make certora-wasm`, then `check_wasm_artifacts.py` |
| Transformation error | Rebuild with `make certora-wasm` (unoptimized) |
| Rule may be vacuous | Add or fix its satisfy witness before you trust the result |
| Expanded-command limit | Reduce the modeled surface; raise the limit only with a reason |
| Local run out of memory | One rule, split parallelism off, smaller heap |
| Counterexample looks bitwise-false | Re-run that rule with `precise_bitwise_ops` |
| `SANITY_FAILED` on a revert rule | Expected; move the rule to a `-reverts` conf |
| Arithmetic rule times out on every operand | Check that the WASM has a `name` section |
| Which assert fails? | Set `multi_assert_check: true` on that conf for one run |
| Job fails with `Usage limit reached` | This is a quota failure, not a proof result. Do not classify it as a violation or a timeout. Wait for the quota reset or run fewer jobs |

## References

- [Tuning and proof limits](../docs/explanation/certora-sunbeam-prover-tuning.md)
- [Protocol invariants](../docs/reference/invariants.md)
- [Threat model](../docs/explanation/threat-model.md)
- [Sunbeam documentation](https://docs.certora.com/en/latest/docs/sunbeam/index.html)
- [Sunbeam tutorials](https://certora-sunbeam-tutorials.readthedocs-hosted.com/en/latest/)
