# Certora Sunbeam tuning and proof limits

Use this guide to diagnose slow, vacuous or failing Sunbeam rules without weakening the property under review. A proof result applies to a specific source tree, WASM artifact, configuration and set of assumptions. Compilation, static inventory checks and successful submission do not establish a proof verdict.

[certora/README.md](../../certora/README.md) owns installation, profile selection and routine execution. This guide covers artifact checks, model boundaries and controlled tuning.

## Check the artifact before tuning

Run these checks from the repository root:

```sh
python3 certora/scripts/check_orphans.py
python3 certora/scripts/sync_wasm_conf.py --check
python3 certora/scripts/check_wasm_artifacts.py
```

| Check | What success establishes |
| --- | --- |
| `check_orphans.py` | Rule/configuration/profile consistency, including sanity policy, revert witnesses and loop-bound compatibility |
| `sync_wasm_conf.py --check` | Canonical focused artifact paths and features |
| `check_wasm_artifacts.py` | Artifact bytes and build provenance match the manifest and source inputs |

The rule count is an inventory, not a count of successful proofs. If artifact provenance fails, rebuild before running the prover:

```sh
make certora-wasm
python3 certora/scripts/check_wasm_artifacts.py
```

A valid manifest establishes artifact identity, not semantic correctness. Retain the report that identifies the artifact actually submitted.

Focused builds preserve function names with `CARGO_PROFILE_RELEASE_STRIP=none` and disable the Stellar post-link optimizer. The manifest checker requires that provenance and a WASM `name` section. Release overflow checks remain part of the arithmetic model.

The researched frontend matches compiler-rt and SDK summaries by function name. Stripped names can force it to process native `i128` limb arithmetic under approximate bitwise semantics, increasing cost and admitting spurious models. A name section enables matching; inspect the run to establish which summaries actually apply. Disabling the optimizer avoids observed transformation failures and does not by itself establish equivalence with a deploy artifact.

<a id="model-boundaries"></a>

## Establish the rule's scope

Read the rule, fixture, feature path and every summary it reaches before interpreting its verdict. A successful assertion may repeat a summary assumption without independently proving the underlying arithmetic.

| Surface | Boundary to check |
| --- | --- |
| Controller risk totals | Health and solvency features compile the real risk arithmetic. Other focused families use a nondeterministic summary unless they explicitly call the real body. |
| Controller position books | Health/solvency and selected fixtures constrain stored positions. Frame rules can deliberately retain arbitrary state. Read the actual pre-map and well-formedness assumptions. |
| Controller position limits | Fixture arrays derive from the limit and include a compile-time `POSITION_LIMIT_MAX == 5` assertion. Compilation does not establish fixture reachability. |
| Pool summaries | Rate fields are constrained to the validated domain. Controller ghost caches reconcile index reads per market. Faithful and consistent summaries remain proof premises. |
| Pool configuration | State-invariant and seize/settle fixtures vary decimals and reserve factor over governance's admitted domain; their rate curve remains fixed. Other families can use fixed fixtures. The symbolic 3–18 decimal domain excludes standalone pool metadata values 0–2. |
| Amount and price bounds | Families use different limits. A convenient bound below a representable ceiling is narrower than the production domain. Check intermediate arithmetic and trap-pruned paths too. |
| Rule placement | Follow transitive helpers and resolved symbols. A controller import through `crate::types` can re-export common arithmetic. Keep each non-satisfy rule in one configuration per layer. |
| Local CI | The default selection covers common and price-aggregator configurations. A successful local CI run does not establish controller or pool verification. |

[Controller fixtures](../../certora/controller/spec/fixture.rs), [shared summaries](../../certora/shared/summaries/mod.rs) and [pool fixtures](../../certora/pool/spec/fixture.rs) define these premises. A rule's presence in source, including a rejection rule, is not a verdict or evidence that its entire feature family is covered.

### Storage, context and external calls

The following frontend behavior is documented for the [pinned research versions](#research-provenance). Recheck it when changing the CLI or backend.

- Storage starts as arbitrary value and existence maps keyed by storage type and key digest. Reading an absent entry traps. An unwritten fixture book can contain arbitrary entries; it is not necessarily empty.
- Ledger timestamp, sequence and related context are symbolic values fixed at rule start. This supports reasoning about one call, not induction over arbitrary transaction histories.
- Cross-contract `call`/try_call, some argument-authorization functions and TTL extension functions are unimplemented in the researched model. An arbitrary return value does not model an external state transition. Explicit summaries define assumptions about pool, oracle, token and router behavior.
- Pool accounting rules exercise internal transitions. They do not independently model arbitrary token transfers, callbacks, allowances or whole-transaction rollback.

### Sanity and rejection witnesses

Traps normally remove paths as `assume(false)`. A rule shaped as `call(); cvlr_assert!(false)` establishes that no modeled execution reaches its end. An unrelated trap can satisfy that assertion, so a successful twin must demonstrate that the relevant fixture can complete when the rejection condition is absent.

| Rule shape | Repository `rule_sanity` policy | Required interpretation |
| --- | --- | --- |
| Assert | `advanced` | Inspect the non-vacuity result as well as the assertion verdict |
| Revert | `none` | Inspect the paired successful witness and its relation to the intended guard |
| Satisfy | `none` | Establishes one reachable execution, not a universal property |

In the researched WASM flow, `advanced` enables TAC vacuity checking and `basic` does not. The frontend does not handle the CVLR macro's appended `CVT_sanity`; macro presence alone is insufficient.

For `cvlr_satisfy!`, generated trap and unwind assertions become assumptions. An undersized loop bound can therefore restrict witness search silently. Keep witness and parent loop bounds compatible. A listed twin can still miss the exact rejection premise; static pairing checks cannot replace report review.

## Diagnose the stopping condition

Run one relevant rule per job. Record the source revision, artifact SHA and source fingerprint, configuration and overrides, CLI/backend identity, available solvers, verdict, sanity result and runtime. For resource failures, also retain command count, split count and loop bound. A wrapper kill supplies no prover verdict.

| Result | Next check |
| --- | --- |
| Counterexample | Inspect concrete operands and the violated assertion. Replay values against Rust before blaming the model or changing the property. |
| `SANITY_FAILED` | Inspect fixture reachability, assumptions and rule shape. A rejection-shaped rule requires its successful twin. |
| Loop-unwind failure | Compare the modeled loop with `loop_iter` and the witness bound. Keep `optimistic_loop` false for authoritative proofs. |
| Transformation, command or block limit | Find the expansion source and summary matching. A larger solver timeout does not repair preprocessing. |
| Per-query timeout | Measure the difficulty and number of split leaves before changing the solver budget. |
| Whole-job timeout | Include preprocessing and all split queries in the budget. |
| Wrapper kill, missing verdict or tooling failure | Preserve logs and resolve the runner failure; do not classify it as a proof or counterexample. |

Do not tune away an unexplained counterexample. Bound fixture state only to the domain the claim promises. Preserve stronger frame rules where needed, and state any narrower configuration domain explicitly.

Arithmetic splits must follow exact implementation predicates. The utilization rules partition whether `borrowed * RAY + supplied / 2` fits `i128`; both halves need reachable domains. A verification-only arithmetic rewrite needs explicit equivalence evidence and a stated proof boundary. The deployed native path still needs coverage.

## Change one setting at a time

Compare runs on identical artifacts and domains. Loop bounds and assertion splitting can change the formula as well as its cost. `multi_assert_check` splits assertions while assuming earlier assertions; `independent_satisfy` changes witness dependence. Compare the effective local and hosted settings explicitly.

| Setting | What to measure |
| --- | --- |
| `smt_timeout` | Per-query solve time; wrapper and job limits must allow the intended query |
| `global_timeout` | Entire job, including preprocessing; verify the service's allowed maximum |
| `-depth`, `-mediumTimeout` | Number and difficulty of split leaves; deeper splitting is not always faster |
| `-splitParallel` | Host memory/core capacity and wall time |
| `precise_bitwise_ops` | Whether a counterexample relies on approximate bitwise operations; bit-vector solving can greatly increase cost |
| `multi_assert_check` | More small queries versus one larger query, with the same artifact and domain |
| `-maxCommandCount`, `-maxBlockCount` | Actual expansion; historical caps are not measurements for another source tree |
| Solver portfolio | Installed solvers and their `--version` output; availability changes search behavior, not formula satisfiability |

Validate artifacts and fixtures first, then tune settings and rule structure. Increase resource limits only for the bottleneck shown by the run. Longer per-leaf timeouts do not fix transformation limits or underconstrained arithmetic.

### CLI compatibility

For the pinned historical CLI, Soroban accepts explicit WASM `files`, `rule`, `loop_iter`, `smt_timeout`, `global_timeout`, `rule_sanity`, `multi_assert_check`, `independent_satisfy`, `optimistic_loop` and `precise_bitwise_ops`. Its Soroban attribute class rejects EVM-only options such as split_rules, `exclude_rule`, `cache` and max_concurrent_rules. Neither verify_timeout nor `rule_timeout` is defined there. Recheck the installed CLI schema when upgrading.

The repository submits prebuilt `files`. For a build-script integration, the researched CLI invokes the script with `--json -l` and a single space-joined `--cargo_features` argument when configured. Standard output must contain one JSON document with `success`, project_directory, `sources` and `executables`; send diagnostics elsewhere. `files` and `build_script` are mutually exclusive.

## Native-i128 counterexample diagnosis

### 9.7 Historical utilization diagnosis

The section identifier is retained for the reference in [rates_rules.rs](../../certora/common/spec/rates_rules.rs).

A September 2, 2026 local run reported `utilization_bounded_when_borrowed_lte_supplied` as violated without concrete operand values. The recorded seven-configuration run had ten verified rules, 21 JVMs killed at 600 seconds and this one violated rule. These outcomes apply to that run only.

For nonnegative debt `B <= S`, with `S > 0`, the intended half-up expression satisfies:

```text
util = floor((B * RAY + floor(S / 2)) / S)
0 <= util <= RAY
```

The inspected artifact lacked function names. The researched frontend therefore could not match exact compiler-rt summaries and processed inlined limb arithmetic. Under its LIA/NIA bitwise approximations, variable `or`/`xor` received incomplete axioms and arithmetic right shift was underconstrained. An underestimated divisor-normalization expression is a plausible source of a spurious `util > RAY` model.

This diagnosis is an inference. The research record retains neither the concrete counterexample nor a rerun confirming that explanation. Named builds, native/widened rules and logging do not close the historical verdict by themselves. Treat the result as unresolved until matching reports and a concrete replay establish its cause.

To investigate on the source tree being reviewed:

1. Build and fingerprint the focused artifact using the preflight steps above.
2. Run both split rules from `certora/common/confs`:

   ```sh
   certoraSorobanProver rates.conf --rule utilization_bounded_when_borrowed_lte_supplied_native
   certoraSorobanProver rates.conf --rule utilization_bounded_when_borrowed_lte_supplied_widened
   ```

3. Inspect each assertion and sanity report. Retain report HTML and actual model operands; logging has model limitations.
4. Replay concrete values against the matching Rust source. Compare a controlled bit-precise run to test the approximation hypothesis.

Both split rules constrain `borrowed` to `0..=100 * RAY`, `supplied` to `1..=100 * RAY`, and `borrowed <= supplied`. Their complementary branch predicates cover that shared domain, not every representable balance. Neither the historical violation nor its proposed explanation alone establishes a protocol bug or a proved false positive.

## Research provenance

The frontend claims and historical case above come from the September 2026 research reconciled at source revision `d26b93ebb48d718b69571ec737f0097af3379916`. The research used:

| Component | Research version |
| --- | --- |
| Certora CLI | `8.17.1` |
| Open-source CertoraProver | `0436a658` |
| Certora Documentation | `c19ad019` |
| CVLR | `f1e3e08b`; repository pin `b8bfb4c9` |
| cvlr-soroban | `70a9ddfc`; repository pin `5ff9d010` |

The exact hosted backend build was not inspectable. Open-source behavior does not attest to the hosted service. The recorded cloud job cap was 7,200 seconds; verify service limits for each run.

Immutable records retain the detailed [suite review](https://github.com/XOXNO/rs-lending-xlm/blob/d26b93ebb48d718b69571ec737f0097af3379916/docs/explanation/certora-suite-review-2026-09-03.md) and [prover research](https://github.com/XOXNO/rs-lending-xlm/blob/d26b93ebb48d718b69571ec737f0097af3379916/docs/explanation/certora-sunbeam-prover-tuning.md). Their file names, counts, recommendations and review dispositions are historical evidence, not run instructions or verdicts for another artifact. In particular, an unseeded-storage claim of dead rules was withdrawn; the separate position-limit fixture mismatch is not the same issue.

## Primary references

- [Certora timeout guidance](https://docs.certora.com/en/latest/docs/user-guide/out-of-resources/timeout.html)
- [CLI option documentation](https://docs.certora.com/en/latest/docs/prover/cli/options.html)
- [Sanity checking](https://docs.certora.com/en/latest/docs/prover/checking/sanity.html)
- [Sunbeam usage](https://docs.certora.com/en/latest/docs/sunbeam/usage.html)
- [Researched open-source frontend](https://github.com/Certora/CertoraProver/tree/0436a658/src/main/kotlin/wasm)
- [Researched prover configuration](https://github.com/Certora/CertoraProver/blob/0436a658/lib/Shared/src/main/kotlin/config/Config.kt)
