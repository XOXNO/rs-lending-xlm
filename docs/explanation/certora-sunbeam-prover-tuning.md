# Certora Sunbeam tuning and proof limits

This note combines the September 2026 suite review and prover research. Source reconciliation is against `d26b93ebb48d718b69571ec737f0097af3379916`. It distinguishes implemented harness/build changes from proof verdicts: no new prover run accompanied this documentation update; historical run outcomes below are not current-source verdicts.

Operational commands and profile definitions belong in [certora/README.md](../../certora/README.md) and [profiles.json](../../certora/profiles.json). [Audit records](../audit/README.md) retain the separate historical security-review conclusions.

## Evidence and provenance

| Evidence | Meaning |
|---|---|
| Source-verified | Inspected implementation or static configuration at the reconciliation revision |
| Historical observation | Reported by the pinned September research or earlier run; not rerun here |
| Inferred | Follows from source or arithmetic, without the corresponding prover execution |
| Proof-open | Requires a verdict with matching source, WASM fingerprint, configuration, and assumptions |

The original suite review began at `36a2ae181b47ab548c2c21e8de90835436ec89fd`; later sections record applied changes. The prover research used `certora-cli==8.17.1`, open-source CertoraProver `0436a658`, Documentation `c19ad019`, CVLR `f1e3e08b` and repository pin `b8bfb4c9`, and cvlr-soroban `70a9ddfc` and repository pin `5ff9d010`. These are historical research versions. The exact hosted backend build was not inspectable; open-source behavior is not attestation of the hosted service.

Immutable detail: [suite review](https://github.com/XOXNO/rs-lending-xlm/blob/d26b93ebb48d718b69571ec737f0097af3379916/docs/explanation/certora-suite-review-2026-09-03.md) and [prover research](https://github.com/XOXNO/rs-lending-xlm/blob/d26b93ebb48d718b69571ec737f0097af3379916/docs/explanation/certora-sunbeam-prover-tuning.md). Their evolving conf/rule counts, scratch paths, old file names, and early recommendations are historical snapshots, not the current runbook.

## Current static status

The documentation reconciliation ran:

```sh
python3 certora/scripts/check_orphans.py
python3 certora/scripts/sync_wasm_conf.py --check
python3 certora/scripts/check_wasm_artifacts.py
```

The first two passed: 119 confs, 386 source rules, seven profiles, no orphan/dead rules, and canonical focused artifact paths. These are inventory checks, **not 386 proofs**. Artifact checking failed with stale source input hashes and controller source-file-count drift. No artifacts were rebuilt or submitted; prior artifact results do not establish current-source proofs.

Before another proof run, build the artifacts and require the provenance check to pass:

```sh
make certora-wasm
python3 certora/scripts/check_wasm_artifacts.py
```

A valid manifest is necessary but insufficient: it establishes artifact identity, not semantic verification.

## Historical findings reconciled with source

| Review item | Source state at reconciliation | Remaining evidence limit |
|---|---|---|
| F1: missing WASM vacuity policy | Assert confs require `advanced`; revert and satisfy confs use `none`. The gate checks revert witnesses and compatible loop bounds. | Read each witness and report: a listed twin can still miss the exact rejection premise. Hosted sanity behavior remains backend-dependent. |
| F2: arbitrary unbounded position books | Health/solvency and selected other fixtures bound their state; frame proofs retain deliberate arbitrary-state premises. | Read the actual pre-map and well-formedness assumptions per rule. An unwritten map is arbitrary in the researched model, not empty. |
| F3: risk totals always summarized | Real risk arithmetic compiles for health and solvency features; other focused families still use a nondeterministic summary. | Controller proofs outside those features remain conditional on the summary. |
| F4: stripped function names | Focused builds set `CARGO_PROFILE_RELEASE_STRIP=none`; the manifest/checker require this provenance and a WASM `name` section. | Current artifacts are stale. A name section enables matching; only a matching prover run demonstrates actual summary use and verdicts. |
| F5/F7: dead helpers, duplicates, wrong layer | Dead harness files and duplicate rules were removed; shared arithmetic families moved to common. The static gate rejects duplicate non-satisfy coverage within a layer. | Follow transitive helpers and resolved symbols before moving rules. `crate::types` may only re-export common types. |
| F9: verdict classification and execution scope | Local runner distinguishes top-level rule lines, `SANITY_FAILED`, unwind failures, killed runs, and missing verdicts. Default local selection still covers common and price-aggregator confs. | PR-local success is not controller/pool verification; timeout and killed outcomes are warnings. Actual runner solver availability was not inspected here. |
| F10: position-limit fixtures seeded ten after limit became five | Fixture arrays derive from the limit and carry a compile-time `POSITION_LIMIT_MAX == 5` assertion. | Compilation prevents that particular drift; it does not prove fixture reachability. |
| F11: direction assertions repeated summary assumptions | Persistence rules cover the controller seam; repeated premise-only direction claims must not be credited as independent arithmetic proofs. | Inspect each actual rule/summary pair; a successful assertion can add little beyond its assumptions. |
| F12: rate-model fields unconstrained; index reads independent | Pool summary now constrains rate fields to the validated domain; controller ghost caches reconcile index values per market, not merely their ranges. | Summary faithfulness and consistency remain proof premises. |
| F13: pool configuration fixed to one instance | State-invariant and seize/settle fixtures now vary decimals and reserve factor over the governance admission domain; other families retain fixed fixtures. | The rate curve remains fixed. Symbolic 3–18 decimals do not cover standalone pool metadata values 0–2. |
| F14: narrow arithmetic domains | Selected price and index bounds were widened; source still contains family-specific amount/value limits. | Check each domain rather than treating the historical table as either wholly open or wholly fixed. A convenient bound below a representable ceiling is still narrower than production. |
| Gap-hunt follow-up rules | Some requested rules now exist, including `flash_position_rejects_normal_mode` and `flash_position_rejects_non_flashloanable_market`. | Do not retain the old blanket “all unwritten” claim or infer that every requested family is complete. Static presence is not a verdict. |

The early claim that fifteen assertion rules and four witnesses were dead because unseeded storage was empty was withdrawn. The later ten-versus-five position-limit mismatch was a separate, real fixture defect; do not merge those dispositions.

## Model boundaries

These details were read from CertoraProver `0436a658`; the hosted build was not revalidated.

- Storage starts as havoced value/existence maps keyed by storage type and key digest. A read of an absent entry traps; an unwritten fixture book can contain arbitrary entries.
- Ledger timestamp/sequence and related context are symbolic values fixed at rule start. This supports one-call reasoning, not induction across arbitrary transaction histories.
- Traps normally prune paths as `assume(false)`. `call(); cvlr_assert!(false)` establishes that no modeled execution reaches its end, but an unrelated trap can satisfy it too. A targeted successful twin distinguishes the intended guard from an impossible fixture.
- `cvlr_satisfy!` establishes one reachable execution. Generated trap/unwind assertions are converted to assumptions on this shape, so an undersized loop bound can silently restrict witness search.
- `advanced` enables TAC vacuity checking in the researched WASM flow; `basic` did not. The CVLR macro's appended `CVT_sanity` was not handled in that frontend. Macro presence is not a substitute for the conf policy and report.
- Cross-contract `call`/try_call, some argument-authorization functions, and TTL extension functions were unimplemented in that model. A havoced return is not a faithful external state transition. Explicit summaries state what controller proofs assume about pool, oracle, token, and router behavior.
- Pool accounting rules call internal transitions. They do not independently model arbitrary token transfers, callback behavior, allowances, or whole-transaction rollback.
- `multi_assert_check` splits assertions while assuming earlier assertions; `independent_satisfy` changes witness dependence. Their presence changes the query, so local and hosted settings must be compared explicitly.

## Build and configuration constraints

Focused WASM keeps function names and disables the Stellar post-link optimizer. The optimizer choice follows repository transformation failures; it is not evidence that production behavior is different. Release overflow checks remain part of the arithmetic model.

The researched frontend matches compiler-rt and SDK summaries by function name. With names stripped it inlines native i128 limb arithmetic; under imprecise bitwise modeling this can be slower and admit spurious models. Keep names before experimenting with arithmetic flags.

For the pinned historical CLI, Soroban accepts explicit WASM `files`, `rule`, `loop_iter`, `smt_timeout`, `global_timeout`, `rule_sanity`, `multi_assert_check`, `independent_satisfy`, `optimistic_loop`, and `precise_bitwise_ops`. EVM-only keys such as split_rules, `exclude_rule`, `cache`, and max_concurrent_rules were not accepted by its Soroban attribute class. Neither verify_timeout nor `rule_timeout` was defined. Recheck the installed CLI schema when upgrading; do not copy a generic EVM sample into these confs.

This repository submits prebuilt `files`. If adopting a build script, the historical CLI invokes it with `--json -l` and a single space-joined `--cargo_features` argument when configured. Stdout must be one JSON document containing `success`, project_directory, `sources`, and `executables`; diagnostics belong elsewhere. `files` and `build_script` cannot be combined.

## Tune from the failure

1. Verify exact source/artifact identity and the rule's feature path. Resolve stale provenance before measuring.
2. Check assumptions, fixture initialization, summary fidelity, revert twins, and sanity outcomes. Never optimize away an unexplained counterexample.
3. Run one relevant rule per job. Record artifact SHA/fingerprint, conf and overrides, CLI/backend identity, verdict and sanity result, runtime, command count, split count, loop bound, and available solvers.
4. Classify the stop: transformation/command limit, loop unwind, per-query timeout, whole-job timeout, wrapper kill, or actual violation. A wrapper kill supplies no prover verdict.
5. Bound fixture state only to the domain the claim promises. Preserve explicit stronger frame rules when needed; do not silently turn full-domain claims into sampled configurations.
6. Split arithmetic at exact implementation predicates. The native/widened utilization rules partition `borrowed * RAY + supplied / 2` fitting i128; both halves need reachable domains.
7. Measure loop and assertion splitting costs per conf. The static host-state floor and witness/parent bounds must stay consistent; do not enable optimistic loops to hide unwind failures.
8. Adjust splitting and budgets for the observed bottleneck. Increasing a per-leaf timeout does not repair a transformation limit or an underconstrained arithmetic model.

| Setting | What to measure |
|---|---|
| `smt_timeout` | Per-query solve time; wrapper and job limits must permit the intended query |
| `global_timeout` | Entire job, including preprocessing; historical cloud cap was 7,200 seconds |
| `-depth`, `-mediumTimeout` | Number versus difficulty of split leaves; deeper is not universally faster |
| `-splitParallel` | Host memory/core capacity as well as wall time |
| `precise_bitwise_ops` | Whether a concrete counterexample relies on approximate bitwise operations; it switches toward bit-vector solving and may greatly increase cost |
| `multi_assert_check` | More small queries versus one larger query, with identical artifact and domain |
| `-maxCommandCount`, `-maxBlockCount` | Measured expansion; copied historical caps are not measurements for current source |
| Solver portfolio | Actual `--version` availability on PATH; changing solver availability changes search, not formula satisfiability |

The documentation recommends settings, then specs, then source changes. In this suite, fixture correctness and artifact identity precede speed tuning because a fast vacuous result is no assurance. Avoid a Certora-only arithmetic rewrite unless its equivalence and changed proof boundary are explicit; the deployed native path still needs coverage.

## 9. Historical native-i128 investigation

### 9.7 Historical utilization diagnosis

This identifier is retained because `certora/common/spec/rates_rules.rs` cites §9.7.

A September 2 local run reported `utilization_bounded_when_borrowed_lte_supplied` violated without concrete operand values. The recorded seven-conf run had ten verified rules, 21 JVMs killed at 600 seconds, and this one violated rule. Those are historical outcomes, not a fresh verification result.

For nonnegative debt `B <= S` with `S > 0`, the intended half-up expression is:

```text
util = floor((B * RAY + floor(S / 2)) / S)
0 <= util <= RAY
```

The source investigation found that the historical artifact lacked names. Consequently the researched frontend could not match its exact compiler-rt summaries and instead processed inlined limb arithmetic. Under its LIA/NIA bitwise approximations, variable `or`/`xor` received incomplete axioms and arithmetic right shift was underconstrained. An underestimated divisor-normalization expression was a plausible route to a spurious `util > RAY` model.

That explanation was **inferred, not confirmed by rerunning the offending artifact/rule**. The exact counterexample was not retained in the research note. Named builds, the native/widened split, and logging were implemented later; those changes do not themselves close the original verdict. Current source also documents logging/model limitations, so retained report HTML and actual model operands are the useful evidence.

To settle it, rebuild and fingerprint the current focused artifact, run both split rules, inspect their sanity and counterexample reports, and replay any concrete values against current Rust. A controlled bit-precise comparison can test the approximation hypothesis. Do not label the historical violation either a confirmed protocol bug or a proved false positive solely from this note.

## Primary references

- [Certora timeout guidance](https://docs.certora.com/en/latest/docs/user-guide/out-of-resources/timeout.html)
- [CLI option documentation](https://docs.certora.com/en/latest/docs/prover/cli/options.html)
- [Sanity checking](https://docs.certora.com/en/latest/docs/prover/checking/sanity.html)
- [Sunbeam usage](https://docs.certora.com/en/latest/docs/sunbeam/usage.html)
- [Researched open-source frontend](https://github.com/Certora/CertoraProver/tree/0436a658/src/main/kotlin/wasm)
- [Researched prover configuration](https://github.com/Certora/CertoraProver/blob/0436a658/lib/Shared/src/main/kotlin/config/Config.kt)
