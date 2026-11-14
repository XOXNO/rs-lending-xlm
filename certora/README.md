# Formal verification

Certora Sunbeam rules for the protocol's arithmetic, accounting, solvency,
liquidation, oracle and strategy properties.

A verdict holds only for the WASM it ran on; a compile, submission or older
report proves nothing about current code.

| Directory | Proves |
|---|---|
| `common/` | Fixed-point math, rate curve, indexes, LP pricing. See [common/spec/README.md](common/spec/README.md) |
| `pool/` | Shares, indexes, cash, revenue, bad debt, fees, flash loans. See [pool/spec/README.md](pool/spec/README.md) |
| `controller/` | Entrypoint gates, authorization, solvency, liquidation, strategies. See [controller/spec/README.md](controller/spec/README.md) |
| `price-aggregator/` | Source admission, freshness, tolerance, fail-closed pricing |
| `shared/summaries/` | Cross-contract summaries the controller rules assume |

Each directory has confs in `confs/` and rules in `spec/`. Conf that runs a
rule:

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

Hosted runs need `CERTORAKEY` and the Certora CLI's `certoraSorobanProver`.
Only `make certora` builds the WASM first. Arguments after `--` go to every
conf of the profile.

Locally (Java, Certora CLI dependencies, prover binary):
`run_profile.py <profile> --local`, or `./certora/scripts/run-rules-local.sh
<conf>` for one conf. It proves one rule at a time with an 8 GB heap,
`-splitParallel` and `multi_assert_check` off, logs to
`target/certora-local-logs/`, and refuses a stale artifact; override with
`CERTORA_JAVA_HEAP`, `-j <n>`, `CERTORA_LOCAL_SPLIT_PARALLEL`,
`CERTORA_LOCAL_MULTI_ASSERT`.

## Profiles

| Profile | Contents |
|---|---|
| `sanity` | Reachability witnesses, including every `-reverts-sanity` conf. Run this first |
| `fast` | Stable math, rate and light controller rules, plus the pure-layer `-reverts` confs |
| `core` | Main audit set: solvency, liquidation, strategies, pool accounting, oracle, host-state `-reverts` confs |
| `heavy` | The 1800 s confs outside `core`, plus `lp-math-isqrt` |
| `flash-position` | Flash-position strategy rules |
| `manual` | `core` + `heavy` |
| `all` | `sanity` + `fast` + `core` + `heavy` |

`certora/scripts/check_orphans.py` fails when:

- no conf runs a rule, or a conf names a missing rule;
- a rule runs in two non-satisfy confs of its layer (move rules, do not copy);
- a conf is in no profile;
- `rule_sanity` mismatches the rule shape, or a revert rule has no witness;
- a host-state conf has `loop_iter` below 28, or a conf lacks
  `-mediumTimeout` or `-maxCommandCount`.

## CI

**`certora-local.yml`** runs on pull requests that touch `certora/**`,
`common/src/**`, `contracts/**/src/**`, `Cargo.toml` or `Cargo.lock`, proving
six default common and price-aggregator confs on the self-hosted runner, 900 s
per rule. Controller and pool rules run hosted.

- Fails on: violation, loop-unwind failure, `SANITY_FAILED`, missing or empty
  rule log, log without verdict, missing conf, or a dispatch rule the conf
  does not list.
- Warns on: solver timeout, solver unknown, wrapper kill.
- **Stays green, with a warning, when the runner has no prover install.**
  Read the verdict summary in the job log.
- On failure it uploads `certora-local-prover-logs-<sha>`; the counterexample
  with `clog!` values is in `Reports/Report-<rule>-Assertions-example1.html`.

**`certora-fastRules.yml`** and **`certora-verification.yml`** submit hosted
jobs on manual dispatch only.

- `certora-fastRules.yml` proves a profile's confs in sequence, stopping at
  the first failure; `job_timeout_minutes` caps it (default 720).
- `certora-verification.yml` with empty `profile` runs `sanity`, one job per
  conf; with a profile, one job per (conf, rule), one summary row per rule.
  `all` is refused (a GitHub matrix holds at most 256 jobs).

## Build artifacts

| Artifact | Build |
|---|---|
| Deploy | Optimized, symbols stripped |
| Prover | `--optimize=false`, one rule module, `certora` feature on |

Prover-only code never ships (`certora` off in deploy builds). Optimized
bytecode can crash the prover's transformations.

`make certora-wasm` writes a manifest binding each artifact to its source
fingerprint and features. Rebuild after any contract, rule, fixture, summary
or dependency change, then check:

    python3 certora/scripts/check_wasm_artifacts.py

### Function names must survive the build

The prover build keeps symbols (`CARGO_PROFILE_RELEASE_STRIP=none`): the
prover applies its compiler-rt summaries (`__muloti4`, `__multi3`, `__divti3`,
`__udivti3`, `__modti3`) and soroban-sdk summaries by function name. Stripped,
every function is `FunctionIndex_<n>`, no summary applies, and each `i128`
multiply or divide becomes slow bit-level code that can give a false
counterexample. `make certora-wasm` refuses a module without a `name` custom
section; `check_wasm_artifacts.py` also requires `"strip": "none"`
provenance.

## Sanity checking on WASM

On Sunbeam, `rule_sanity: basic` equals `none`: no vacuity check. Only
`advanced` runs one, at one extra solve per proved rule.

| Rule shape | `rule_sanity` |
|---|---|
| assert | `advanced` |
| satisfy | `none` (the witness proves reachability) |
| revert | `none` |

A revert rule, `call(...); cvlr_assert!(false);`, proves a gate rejects the
call. The vacuity check reports `SANITY_FAILED` on it even when correct, so
revert rules live in `<name>-reverts.conf` with their origin conf's budgets,
and each needs a satisfy witness completing the same fixture.
`check_orphans.py` accepts:

- a `<rule>_fixture_completes` twin in the sibling `<name>-reverts-sanity.conf`;
- an existing module witness listed in `EXISTING_WITNESS` in
  `check_orphans.py`, added only if it drives the same verb or is the
  module's only witness.

A satisfy conf's `loop_iter` must not be below its assert twin's: the prover
turns satisfy asserts, loop-unwind included, into assumes, so a low bound
silently cuts the search.

## Budgets

| Class | `smt_timeout` | `prover_args` | `loop_iter` |
|---|---|---|---|
| Pure arithmetic | 600 | `-depth 5 -mediumTimeout 20` | the exact loop count: 1, or 8 where `compound_interest` is reached, or 6 where `isqrt_of_product` or another fixed loop is |
| Host state (pool, controller, aggregator) | 900 | `-depth 10 -splitParallel true -mediumTimeout 20` | measured, at least 28 |
| `math-hard`, `rate-accounting-hard`, `fp-identities-bv`, `strategy-repay-collateral`, `bulk-borrow-duplicate-leg` | 1800 | as their class | as their class |

`pool-lifecycle` confs are host state at 300 s. Every conf sets its own
`-maxBlockCount` and `-maxCommandCount`.

- A rule unsolved in 600 s will not solve in 2000 s: change its shape, not
  the budget.
- `-dontStopAtFirstSplitTimeout true` only for satisfy confs and expected
  counterexamples, never assert confs.
- `precise_bitwise_ops` is on only in `fp-identities-bv`,
  `rate-accounting-hard`, `scaled-math` and the three `tolerance-math` confs,
  each fixing an observed false counterexample. Remove none without a
  measurement.
- `multi_assert_check` is on only in the six multi-assert pool confs (see the
  [pool guide](pool/spec/README.md)). To find which assert fails elsewhere,
  turn it on for one run.

## Known prover limits

- `pool-lifecycle` and `lp-math-stable` run hosted only. Locally,
  `market_create_writes_zeroed_state` reports a violation,
  `accrue_is_noop_when_no_time_elapsed` a rule-encoding error, and every
  `lp-math-stable` rule an internal error.
- `isqrt_is_the_integer_floor_of_the_root` (`lp-math-isqrt.conf`) is
  unprovable: the prover compares U256 values as "equal digests give 0, else
  any of 1 or -1", so every comparison in `isqrt_of_product` is
  nondeterministic. Its counterexample (a = 0x55555555555556, b = 3) does not
  reproduce in Rust. The conf is in `heavy` only because every conf needs a
  profile.
- The utilization bound is proved as two rules over
  `0 <= borrowed <= supplied <= 100 * RAY`, `supplied >= 1`, each on one
  arithmetic branch: `utilization_bounded_when_borrowed_lte_supplied_native`
  where `borrowed * RAY + supplied / 2` fits `i128`, and
  `utilization_bounded_when_borrowed_lte_supplied_widened` for the rest. On a
  stripped artifact the unsplit rule gave a counterexample without operand
  values, likely from the bit-level limb arithmetic described in
  [Function names must survive the build](#function-names-must-survive-the-build).

## Reading a Sunbeam verdict

Before you use a verdict, read the rule, its fixture, its feature path and
every summary it reaches. An assertion that repeats a summary assumption does
not prove the arithmetic behind it.

### Model boundaries

| Surface | What to check |
|---|---|
| Controller risk totals | Real risk arithmetic in health and solvency features; elsewhere a nondeterministic summary unless the rule calls the real function. |
| Controller position books | Health, solvency and some fixtures constrain stored positions; frame rules may keep arbitrary state. Read the pre-state and well-formedness assumptions. |
| Controller position limits | Fixture arrays derive from the limit; a compile-time assertion pins `POSITION_LIMIT_MAX == 5`. |
| Pool summaries | Proof premises. Rate fields stay in the validated domain; controller ghost caches reconcile index reads per market. |
| Pool configuration | Decimals (3–18) and reserve factor are symbolic in some families, the rate curve is fixed; see the [pool fixture domain](pool/spec/README.md#fixture-domain). |
| Amount and price bounds | Vary by family; a bound below the representable ceiling narrows the production domain. |

The [controller fixtures](controller/spec/fixture.rs),
[shared summaries](shared/summaries/mod.rs) and
[pool fixtures](pool/spec/fixture.rs) define these premises.

### Storage, context and external calls

- Storage starts as arbitrary maps keyed by storage type and key; reading an
  absent entry traps; an unwritten fixture book can hold arbitrary entries.
- Ledger timestamp and sequence are symbolic, fixed at rule start: one call,
  not a transaction sequence.
- The model does not implement cross-contract calls, `require_auth_for_args`
  or TTL extension. Only the summaries in
  [What a verdict does not prove](#what-a-verdict-does-not-prove) stand in for calls.

### Stopping conditions

Run one rule per job. Record the source revision, artifact fingerprint, conf,
CLI version, verdict, sanity result and runtime.

| Result | Next step |
|---|---|
| Counterexample | Read operands and the failed assertion; replay the values in Rust before changing the rule. |
| `SANITY_FAILED` | Check fixture reachability and rule shape; a revert rule needs its satisfy twin. |
| Loop-unwind failure | Compare the loop with `loop_iter` and the witness bound. Keep `optimistic_loop` false. |
| Transformation, command or block limit | Find the expansion source; a larger solver timeout does not help. |
| Per-query timeout | Measure the split leaves before changing `smt_timeout`. |
| Wrapper kill or missing verdict | Keep the logs. Neither proof nor counterexample. |

Change one setting at a time, same artifact and domain. `multi_assert_check`
and `independent_satisfy` change the formula, not just its cost.
`SOROBAN_CONF_KEYS` in `certora/scripts/check_orphans.py` lists the conf keys
the pinned CLI accepts.

## What a verdict does not prove

- Pool rules check accounting before token transfers; no token behavior,
  flash callbacks, allowances, reentrancy or rollback.
- Controller rules assume the pool, oracle and position NFT summaries; a
  verdict reaching one is conditional on it. Token, swap aggregator, Blend and
  flash-position receiver calls have no summary, so verdicts say nothing
  about them.
- Controller valuation rules assume an accepted price unless modelling a
  price failure.
- Long batch loops and multi-year accrual, where no induction invariant
  exists.

A satisfy witness shows reachability only, not a universal property.

## Adding a proof

1. State the invariant and the threat.
2. Put the rule in the right layer: common, pool, controller or
   price-aggregator.
3. Add a fixture reaching the state, the rule, a conf with the correct
   `rule_sanity`, and a witness.
4. Run `./certora/compile_all.sh --wasm` and `check_orphans.py`, then the
   smallest covering profile or rule.
5. Update the layer guide when a proof boundary or assumption changes.

Prefer a small lemma to a large stateful rule. Never fix a timeout by
weakening the property.

## Troubleshooting

| Symptom | Action |
|---|---|
| Transformation error | Rebuild with `make certora-wasm` (unoptimized) |
| Rule may be vacuous | Fix its satisfy witness before trusting the result |
| Expanded-command limit | Shrink the modeled surface; raise the limit only with a reason |
| Local run out of memory | One rule, split parallelism off, smaller heap |
| Counterexample looks bitwise-false | Re-run that rule with `precise_bitwise_ops` |
| Arithmetic rule times out on every operand | Check the WASM has a `name` section |

## References

- [Protocol invariants](../docs/reference/invariants.md)
- [Threat model](../docs/explanation/threat-model.md)
- [Sunbeam documentation](https://docs.certora.com/en/latest/docs/sunbeam/index.html)
- [Sunbeam tutorials](https://certora-sunbeam-tutorials.readthedocs-hosted.com/en/latest/)
