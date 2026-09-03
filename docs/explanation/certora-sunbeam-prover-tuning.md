# Certora Sunbeam prover tuning: research notes

Research date: 2026-09-03. Primary sources only. Every claim carries a
citation to the page or file that owns it. Where a fact was read from source
code but not executed, the label says so.

Evidence labels:

- **Observed** — read directly from the cited file or page.
- **Inferred** — follows from observed code paths; not reproduced by a run.
- **Not found** — no primary source states it.

Sources and the exact revisions read:

| Source | Revision | How it was read |
|---|---|---|
| `certora-cli==8.17.1` wheel (`certora_cli/...`) | PyPI wheel `certora_cli-8.17.1-py3-none-macosx_10_9_universal2.whl` | unpacked and read; the CLI pinned by [certora/requirements-cli.txt](../../certora/requirements-cli.txt) |
| github.com/Certora/CertoraProver (open-source prover) | `0436a658` (2026-08-28) | cloned; prover-side defaults come from `lib/Shared/src/main/kotlin/config/Config.kt` and the WASM front-end under `src/main/kotlin/wasm/` |
| github.com/Certora/Documentation (source of docs.certora.com) | `c19ad019` (2026-07-10) | cloned; page URLs cited alongside file paths |
| github.com/Certora/cvlr | `f1e3e08b` (0.6.1, 2026-08-27) and the repo pin `b8bfb4c9` | cloned; pinned rev read through the GitHub contents API |
| github.com/Certora/cvlr-soroban | `70a9ddfc` (0.4.0, 2026-08-27); repo pin `5ff9d010` | cloned; compared with `vendor/cvlr-soroban` |
| github.com/Certora/sunbeam-tutorials | `48c952a4` (2025-11-05) | cloned |
| Certora/aquarius-cantina-fv, code-423n4/2025-02-blend-fv, Certora/sunbeam-vs-other-tools, Certora/meridian2024-workshop, Certora/c-fv-sunbeam-demo, Certora/certora-run-action, Certora/reflector-subscription-contract, blend-capital/blend-contracts-v2 | HEAD on 2026-09-03 | cloned |

Caveat that applies to every prover-side statement: the hosted prover that
`certora-cli` 8.17.1 submits to is a build Certora operates. Its exact
revision is not inspectable from here. Prover defaults and modelling facts
below are **Observed in the open-source CertoraProver at `0436a658`** and may
differ from the deployed build.

## 1. Conf-file schema for certoraSorobanProver (certora-cli 8.17.1)

### How keys are accepted

- A conf key is the lower-cased attribute name (`get_conf_key()` returns
  `self.name.lower()`). Observed: wheel `certora_cli/Shared/certoraAttrUtil.py:108-109`.
- The Soroban entry point builds its context with the attribute class
  SorobanProverAttributes, which inherits exactly CommonAttributes,
  InternalUseAttributes, BackendAttributes, EvmRuleAttribute,
  RustAttributes, and adds its own `"files"`. Observed:
  `certora_cli/certoraSorobanProver.py:51` (`build_context(args, App.SorobanApp)`)
  and `certora_cli/CertoraProver/certoraContextAttributes.py:1942-1957`.
- Any conf key the class does not define is rejected: `"{option} appears in
  the conf file but is not a known attribute."` Observed:
  `certora_cli/CertoraProver/certoraConfigIO.py:135-147`. The conf file is read
  (`certoraContext.py:275`) before the CLI sets `context.process = None`
  (`certoraContext.py:276`), so that assignment does not rescue a `"process"`
  key.

### Keys accepted by certoraSorobanProver 8.17.1

Line numbers are in `certora_cli/CertoraProver/certoraContextAttributes.py`.

| Conf key | Prover flag it sets | Validation / default text in the wheel | Line |
|---|---|---|---|
| `"files"` | (input) | must end with `.wasm` (`SOROBAN_EXEC_EXTENSION = '.wasm'`, `certoraUtils.py:119`); "Rust projects must specify exactly one executable in 'files'."; cannot be combined with `"build_script"`; "The Soroban file ... cannot be accompanied with other files" | 1944; validator 186-190, 948-949 |
| `"build_script"` | (build) | `validate_exec_file()`; "script to build a rust project"; default text "Using default building command" | 1813 |
| `"cargo_features"` | (build) | list; "a list of strings that are extra features passed to the build_script" | 1824 |
| `"cargo_tools_version"`, `"cargo_build_verbose"`, `"solana_sbf_arch"` | (build) | part of RustAttributes, so accepted by the Soroban parser too; the first and third are Solana-oriented | 1835-1866 |
| `"rule"` | `-rule` | list; "Asterisks are interpreted as wildcards"; a name without `*` must be a valid identifier (`validate_evm_rule_name()`, `certoraValidateFuncs.py:903-907`) | 1869 |
| `"msg"` | (metadata) | `validate_msg()`; default "No message" | 64 |
| `"prover_version"` | (job) | regex `^[a-zA-Z][\w\-]*(/[\w\-]+)*$`; default text "Uses the latest public Prover version" | 124; validator 1017-1026 |
| `"server"` | (job) | regex `^[a-zA-Z_0-9\-]+$`; no default text | 135; validator 995-1001 |
| `"wait_for_results"` | (client) | values `ALL` or `NONE`; default text "Sends request and does not wait for results" | 144 |
| `"mutations"` | (certoraMutate only) | map; CLI use is NotAllowed; comment: "used by certoraMutate, ignored by certoraRun" | 159 |
| `"url_visibility"` | (report link) | `private` by default, `public` in CI | 219; validator 194-197 |
| `"override_base_config"` | (conf) | "Path to parent conf" | 208 |
| `"compilation_steps_only"`, `"build_only"`, `"build_dir"`, `"run_source"`, `"debug"`, `"show_debug_topics"`, `"debug_topics"`, `"version"`, `"commit_sha1"` | (client) | CommonAttributes | 62-236 |
| `"test"`, `"test_condition"`, `"expected_file"`, `"no_compare"` | (internal) | InternalUseAttributes | 1352-1392 |
| `"loop_iter"` | `-b` | non-negative integer; default text "A single iteration for variable iterations loops, all iterations for fixed iterations loops" | 1445 |
| `"smt_timeout"` | `-t` | positive integer; no default text in the wheel (prover default 300 s, see below) | 1457 |
| `"multi_assert_check"` | `-multiAssertCheck` | boolean; default text "Stops after a single violation of any assertion is found" | 1467 |
| `"independent_satisfy"` | `-independentSatisfies` | boolean; default text "For each `satisfy` statement, assumes that all previous `satisfy` statements were fulfilled" | 1479 |
| `"rule_sanity"` | `-ruleSanityChecks` | `none`, `basic`, `advanced` (RuleSanityValue, `certoraValidateFuncs.py:58-61`); default text "Basic sanity checks (Vacuity and trivial invariant check)" | 1513 |
| `"multi_example"` | `-multipleCEX` | "Shows a single example" by default | 1526 |
| `"optimistic_loop"` | `-assumeUnwindCond` | boolean; marked `unsound=True` in the job config data | 1552 |
| `"prover_args"` | (verbatim) | list of strings; rejected if a string equals a jar flag that has its own CLI attribute unless that attribute allows it (`validate_prover_args()`, lines 35-56); `-globalTimeout` is redirected to `"global_timeout"` | 1632 |
| `"global_timeout"` | `-userGlobalTimeout` | non-negative integer | 1655 |
| `"cloud_global_timeout"` | `-globalTimeout` | always rejected: "Cannot set the global timeout for the cloud. Use 'global_timeout' instead" | 1645; validator 225-229 |
| `"smt_use_bv"` | `-smt_useBV` | boolean | 1724 |
| `"precise_bitwise_ops"` | `-smt_preciseBitwiseOps` | boolean; also allowed inside `"prover_args"` (`temporary_jar_invocation_allowed=True`); help "Show precise bitwise operation counter examples. Models mathints as unit256 that may over/underflow"; default text "May report counterexamples caused by incorrect modeling of bitwise operations, but supports unbounded integers (mathints)" | 1735 |
| `"coverage_info"`, `"short_output"`, `"tool_output"`, `"protocol_name"`, `"protocol_author"`, `"group_id"` | (report) | BackendAttributes | 1749-1810 |
| `"java_args"`, `"jar"`, `"queue_wait_minutes"`, `"max_poll_minutes"`, `"log_query_frequency_seconds"`, `"max_attempts_to_fetch_output"`, `"delay_fetch_output_seconds"`, `"prover_resource_files"`, `"fe_version"`, `"job_definition"`, `"mutation_test_id"`, `"coinbase_mode"`, `"enforce_require_reason"`, `"save_verifier_results"`, `"include_empty_fallback"`, `"no_calltrace_storage_information"`, `"unused_summary_hard_fail"`, `"assert_autofinder_success"`, `"assert_source_finders_success"`, `"disable_source_finders"` | various | BackendAttributes; `"use_per_rule_cache"` is defined but its validator always fails (`validate_false()`) | 1393-1810 |

### Keys that certoraSorobanProver 8.17.1 does not accept

Observed from the class layout (EvmAttributes spans lines 290-1351,
DeprecatedAttributes 237-289, SolanaProverAttributes 2057-2124; none of
them is a base of SorobanProverAttributes):

- `"process"` — lives in DeprecatedAttributes (line 240) with the message
  "`process` is deprecated and will be removed in a future release." The
  Sunbeam user guide still shows `"process": "emv"` in its sample conf
  (docs.certora.com/en/latest/docs/sunbeam/usage.html, "Running Sunbeam";
  source `docs/sunbeam/usage.rst:86-97`). Inferred: with 8.17.1 that key would
  hit the unknown-attribute error for a Soroban run. Not executed here.
- `"exclude_rule"` (777), `"split_rules"` (791), `"cache"` (657),
  `"max_concurrent_rules"` (1248), `"method"` / `"exclude_method"`
  (1031/1045), `"verify"`, all `solc*` / `vyper*` keys, `"link"`,
  `"address"`, `"struct_link"`, `"optimistic_fallback"` (1173),
  `"optimistic_hashing"`, `"hashing_length_bound"`, `"nondet_difficult_funcs"`,
  `"auto_dispatcher"` — EVM only. The Solana class re-declares its own
  `"exclude_rule"` and `"split_rules"` (2014, 2110); the Soroban class does
  not.
- `"assert_on_panic"`, `"solana_inlining"`, `"solana_summaries"` — Solana
  only (2075-2109).
- `"verify_timeout"`, `"rule_timeout"` — **Not found**: no attribute with
  either name exists anywhere in the wheel (`grep -rn 'rule_timeout\|verify_timeout'`
  over `certora_cli/` returns nothing).

### Documented defaults

| Setting | Default | Source |
|---|---|---|
| `"smt_timeout"` | 300 s | docs.certora.com/en/latest/docs/prover/cli/options.html, `smt_timeout` ("The default time out for the solvers is 300 seconds"); prover `Config.kt:2475-2483` (`Config.SolverTimeout` = 300, flag `-t`, alias `-timeout`) |
| `"global_timeout"` | cloud job bound is 2 hours (7200 s); values above 7200 are ignored | options.html, `global_timeout`; prover `Config.kt:409-416` (`-userGlobalTimeout` default 0 = infinite at the jar level) |
| `"loop_iter"` | 1 | options.html, `loop_iter` ("The default number of loop iterations we unroll is one"); `Config.kt:1268-1276` (`Config.LoopUnrollConstant` = 1, flag `-b`, alias `-loopIter`) |
| `"optimistic_loop"` | off (unwind condition is asserted) | options.html, `optimistic_loop`; `Config.kt:1297-1304` |
| `"rule_sanity"` | `basic` when the flag is absent | docs.certora.com/en/latest/docs/prover/checking/sanity.html ("When not using the `rule_sanity` option at all, or when using it with the value `basic`, the vacuity and trivial invariant checks are performed"); `Config.kt:1053-1059` (`Config.DoSanityChecksForRules` default `SanityValues.BASIC`); changelog 8.x "Basic sanity checks now run by default" (`docs/prover/changelog/prover_changelog.md:90`) — but see section 7 for what this means on WASM |
| `"multi_assert_check"` | off | options.html, `multi_assert_check`; `Config.kt:1085-1092` |
| `"independent_satisfy"` | off | options.html, `independent_satisfy`; `Config.kt:1095-1102` |
| `"precise_bitwise_ops"` | off | options.html, `precise_bitwise_ops`; `Config.kt:2922-2926` |
| `"wait_for_results"` | does not wait, except in CI | options.html, `"wait_for_results"` |
| `"url_visibility"` | private (public in CI) | changelog `prover_changelog.md:94`; wheel validator 194-197 |
| `"prover_version"` | latest public prover | wheel line 127 |

## 2. The build_script protocol

Observed in the wheel, `certora_cli/CertoraProver/certoraBuildRust.py:74-96`
and `certora_cli/CertoraProver/certoraParseBuildScript.py:60-98`:

- Invocation: `[build_script, '--json', '-l']`. When `"cargo_features"` is
  set, the CLI appends `--cargo_features` followed by **one** argument, the
  features joined with spaces (`' '.join(context.cargo_features)`). The
  flags are not a convention: the CLI hard-codes them
  (`certoraBuildRust.py:77-91`).
- The script's stdout must be a single JSON document (`json.loads(result.stdout)`);
  an empty document is an error ("No JSON output from build script").
- Required keys: `"success"`, `"project_directory"`, `"sources"`,
  `"executables"`. Missing keys raise "Missing required keys in build script
  response: ...". `"success"` false raises "Compilation failed using build
  script". A non-zero exit status raises "Error running the script ..." with
  the script's stderr (`certoraParseBuildScript.py:70-85`).
- Optional keys the CLI reads: `"log"` with `"stdout"` and `"stderr"` file
  paths (copied into the build directory, `certoraBuildRust.py:151-154`);
  `"solana_inlining"` and `"solana_summaries"` (Solana only).
- `"return_code"` is emitted by the reference script but the CLI never reads
  it (it is absent from the required-key list and from every `json_obj.get`
  call in `certoraParseBuildScript.py`).
- The verified file becomes `project_directory/executables`
  (`context.files = [os.path.join(...)]`, line 97). `"sources"` globs are
  matched relative to `"project_directory"` and the script and conf file are
  added to the uploaded sources (`certoraBuildRust.py:120-142`).

Reference script (Observed: `Certora/sunbeam-tutorials` `projects/token/certora_build.py`):
argparse options `-o/--output`, `--json`, `-l/--log`, `-v/--verbose`;
globals `COMMAND = "just build"`, PROJECT_DIR, `SOURCES = ["src/**/*.rs", "Cargo.toml"]`,
`EXECUTABLES = "target/wasm32-unknown-unknown/release/certora_meridian24_token.wasm"`;
output object with the six keys `"project_directory"`, `sources`, `executables`,
`success`, `"return_code"`, `log` (lines 62-98). The user guide documents the
same globals (usage.html, "Running Sunbeam"; `docs/sunbeam/usage.rst:99-111`).

Inferred: the reference script does not declare `--cargo_features`, so a
conf that sets `"cargo_features"` together with that script would fail in
the script's argparse. This repository submits pre-built `"files"` (105 of
105 confs) and never triggers the build-script path.

## 3. Timeout mitigation options

Applicability column: "all" means the option is a generic prover setting
that the WASM pipeline also reaches (it is applied at TAC or SMT level);
"EVM" means the docs or code tie it to Solidity/EVM. Defaults are Observed
in `Config.kt` at `0436a658` unless a docs citation is given.

| Option | Applies to | What it does | Default | Documented guidance |
|---|---|---|---|---|
| `--smt_timeout` (`-t`) | all | per-SMT-query budget; "controls the timeout that is used to solve split leafs" | 300 s | options.html `smt_timeout`: "Usually, if the rule isn't solved in 600 seconds, it will not be solved in 2,000 either. It is better to concentrate your efforts on simplifying the rule, the source code, add more summaries, or use other time-saving options." |
| `--global_timeout` (`-userGlobalTimeout`) | all | whole-job budget; job is terminated at the limit | cloud 7200 s cap | options.html `global_timeout`: "Jobs that exceed the global timeout will simply be terminated, so the result reports may not be generated." |
| `-depth` | all | "Sets the maximum splitting depth" | 10 (`Config.kt:2045`; options.html `depth`) | "When the deepest splits are too heavy to solve, but not too high in number, increasing this will lead to smaller, but more numerous split leaves ... Conversely, if run time is too high because there are too many splits, decreasing this number means that more time is spent on fewer, but bigger split leaves." |
| `-mediumTimeout` | all | seconds a non-leaf split gets before being split again | 10 s (`Config.kt:2050`; docs give no number) | options.html `mediumTimeout`: "When a little more time can close some splitting subtrees early, this can save a lot of time, since the subtree's size is exponential in the remaining depth." |
| `-lowTimeout`, `-tinyTimeout` | all | timeouts for low / very low score splits | 5 s, 2 s (`Config.kt:2054-2059`) | Not documented on docs.certora.com |
| `-smt_initialSplitDepth` | all | "The first `<number>` split levels are not checked with the SMT solver, but rather split immediately"; generates 2^n splits; `-depth` takes precedence | 0 (`Config.kt:3344`) | options.html `smt_initialSplitDepth`: use "When there is a lot of overhead induced by processing and trying to solve splits that are very hard, and thus run into a timeout anyway"; "low numbers are advisable" |
| `-dontStopAtFirstSplitTimeout` | all | keep exploring other splits after a leaf times out | false (`Config.kt:2062`) | options.html: "only useful when there exists a counterexample for the rule under verification ... (In case of a rule using `satisfy` rather than `assert` ... this option is only useful if the rule is correct.)" |
| `-splitParallel` | all | "Enable parallelised control-flow splitting during TAC to SMT translation" | false (`Config.kt:2120`) | timeout.md: "It can also help to have splitting run in parallel (the splits are solved sequentially by default)."; changelog: "The `-splitParallel` option will now enable the new parallel splitter" (`prover_changelog.md:269`) |
| `-splitParallelTimelimit` | all | "Overall timelimit for the parallelised splitting approach" | 3600 s (`Config.kt:2141-2147`) | Not documented on docs.certora.com |
| `-splitParallelInitialDepth`, `-splitParallelStepSize` | all | immediate split depth for the parallel splitter; splits per step | 0, 2 (`Config.kt:2132-2160`) | Not documented on docs.certora.com |
| `-numOfParallelSplits` | all | number of splits solved concurrently by the parallel splitter | 5 (`Config.kt:2155`) | Not documented on docs.certora.com |
| `-smt_parallelLIASolvers`, `-smt_parallelNIASolvers` | all | solver lists for the parallel splitter | see `Config.kt:2161-2172` | Not documented |
| `-backendStrategy` | all | "chooses the backend solving strategy: adaptive, cegar or singlerace" | adaptive (`Config.kt:2932-2938`) | changelog: "`-adaptiveSolverConfig false` has been replaced by `-backendStrategy singlerace`" (`prover_changelog.md:97`) |
| `-adaptiveSolverConfig` | (deprecated) | old switch for a single solver race | removed: not present in `Config.kt` | changelog: "was mainly used in combination with `-smt_useNIA` true to run NIA solvers only. Instead, use: `--prover_args "-backendStrategy singleRace -smt_useLIA false -smt_useNIA true"`" (`prover_changelog.md:195`) |
| `-smt_useLIA` | all | include LIA solvers; values `false`/`none`, `without-verifier`, `true`/`with-verifier`, `unsat-only` (`UseLIAEnum.kt:37-45`) | `with-verifier`, forced to `none` when precise bitwise ops is on (`Config.kt:2952-2968`) | Only the changelog line above; no docs page |
| `-smt_useNIA` (aliases `-useNonLinearArithmetic`, `-smt_nonLinearArithmetic`) | all | "Include NIA solvers in the race" | true, "unless -preciseBitwiseOps is set" (`Config.kt:2970-2983`) | changelog line above |
| `-smt_useBV` (aliases `-useBitVectorTheory`, `-smt_bitVectorTheory`; CLI `"smt_use_bv"`) | all | "Use bit-vector encoding and bit-vector solvers in the race" | false; "[defaults to -preciseBitwiseOps]" | `Config.kt:2990-2994`: "Best used via -preciseBitwiseOps, otherwise make sure to disable LIA and NIA via -smt_useLIA false -smt_useNIA false" |
| `--precise_bitwise_ops` (`-smt_preciseBitwiseOps`) | all | "models bitwise operations exactly, instead of using the default overapproximation" and switches the solver race to BV (LIA/NIA off by default) | off | options.html `precise_bitwise_ops`: "enabling this option can significantly increase verification time"; limitations "The maximum supported integer value is 2^256 - 1, effectively restricting `mathint` to a `uint256`"; use "if a counterexample suggests that incorrect modeling of bitwise operations is affecting verification results"; changelog: "`--precise_bitwise_ops` to easily enable bit-vector theory solvers" (`prover_changelog.md:311`) |
| `-smt_LIASolvers`, `-smt_NIASolvers`, `-smt_BVSolvers`, `-smt_overrideSolvers` | all | per-theory solver lists; override instead of filter | empty = predefined lists; false (`Config.kt:3027-3063`) | Not documented |
| `-solvers` (aliases `-solver`, `-s`) | all | solver programs/configurations, e.g. `-solvers=[z3,cvc5]` or `-solvers=[z3:def,cvc5:nl,z3:lia1]` | "All configurations from solvers from this set { z3, cvc4, cvc5, yices, bitwuzla } that are available on this system" (`Config.kt:2505-2520`) | Not documented on docs.certora.com |
| `-smt_easy_LIA` | all | "Inhibits the generation of LIA axioms for pairs of var by var multiplications" | true (`Config.kt:3159-3166`; the description string still says "default : false") | changelog: "The `-prover_args` option `-smt_easy_LIA` is now set to `true` by default." (`prover_changelog.md:241`) |
| `-smt_noLIAAxioms` | all | "Inhibits the generation of all LIA axioms ... experimental flag for expert users" | false (`Config.kt:3145-3157`) | Not documented |
| `-smt_hashingScheme` | all | hashing axiomatisation: `Legacy`, PlainInjectivity, `Datatypes` | PlainInjectivity (`HashingScheme.kt:33`, `Config.kt:3066-3078`) | only the CVL2 migration note (`docs/cvl/cvl2/changes.md:862-875`) |
| `-recursionEntryLimit` | all (used by the WASM inliner) | "Number of unfolding for a recursive function" | 3 (`Config.kt:588-594`); `Inliner.kt:78-86` inserts an internal-unreachable block above the limit | Not documented for Soroban |
| `-recursionErrorAsAssert` | all | "Determine if to always set recursion errors as assert failures and not as plain throws" | true (`Config.kt:596-603`) | Not documented for Soroban |
| `-maxCommandCount` (alias `-maxDecompiledCommandCount`), `-maxBlockCount` | all | "Maximum number of TAC commands per method", "Maximum number of TAC blocks per method" | 1,000,000 and 100,000 (`Config.kt:544-556`) | Not documented on docs.certora.com; this repository raises both in every conf |
| `-destructiveOptimizations` (alias `-calltraceFreeOpt`) | all | "Allow more aggressive optimizations, but disable the generation of call traces."; values `disable`, `twostage`, `"twostage_checked"`, `"twostage_interpreted"`, `enable` (`DestructiveOptimizationsModeEnum.kt:24-35`) | disable | timeout.md "Command line options": `-destructiveOptimizations enable` "enables some aggressive simplifications that speed up the Prover in many cases, but breaks call trace generation in case a rule is violated" |
| `-trapAsAssert` | WASM/Move | "Treat traps as asserts." | false (`Config.kt:3904-3911`) | Not documented; used in `Certora/c-fv-sunbeam-demo/sunbeam.conf` |
| `-prettifyCEX` | all | "Attempt to make counterexamples prettier"; values `none`, `basic`, `joint`, `extensive` | basic (`Config.kt:2609-2617`; `PrettifyCEXEnum.kt`) | Not documented |
| `-optimisticFallback` / `--optimistic_fallback` | EVM | "optimistically assume unresolved fallback functions do not havoc state" | false (`Config.kt:3612-3620`) | options.html `"optimistic_fallback"` (Solidity fallback semantics) |
| `-enableSolidityBasedInlining` | EVM | "use the Solidity source code to inline for _all_ methods" | false (`Config.kt:1456-1463`) | changelog `prover_changelog.md:357` |
| `-calltraceFreeVars` | — | **Not found** in `Config.kt`, the prover sources, or the docs | — | — |
| `--multi_assert_check` | all; Soroban since 8.13.0 | each assert becomes a sub-rule that assumes the preceding asserts | off | options.html: "As a timeout mitigation strategy: checking each assertion separately may, in some cases, perform better"; changelog 8.13.0 (`prover_changelog.md:21-22`) |
| `--rule` | all | run a subset of rules | all rules | timeout.md "Running rules individually": "Even if no rule is very expensive on its own, working on all of them in parallel can add up quickly and thereby exceed the timeout." |

Sources for the whole table: docs.certora.com/en/latest/docs/user-guide/out-of-resources/timeout.html
(source `docs/user-guide/out-of-resources/timeout.md`),
docs.certora.com/en/latest/docs/prover/cli/options.html
(`docs/prover/cli/options.md:1219-1275, 1639-1731, 2564-2605, 2770-2935`),
docs.certora.com/en/latest/docs/prover/techniques/index.html ("Control flow
splitting"), and `Config.kt` at the cited lines.

## 4. The cvlr and cvlr-soroban API, and rule discovery

All statements Observed in the cloned crates unless labelled.

### Assertions

- `cvlr_assert!(cond)`, `cvlr_assume!(cond)`, `cvlr_satisfy!(cond)` accept an
  optional string literal that is ignored, and call `CVT_assert`,
  `CVT_assume`, `CVT_satisfy` — `extern "C"` imports from WASM module `env`
  (`cvlr-asserts/src/core.rs:1-92`). `cvlr_assert!` and `cvlr_satisfy!` also
  record a source location (`add_loc!`).
- The prover recognises exactly these imports (plus the legacy
  `CERTORA_assert_c` family) and lowers them to TAC assume / assert with
  `TACMeta.ASSERT_ID` / assert with `TACMeta.SATISFY_ID`
  (`src/main/kotlin/wasm/summarization/WasmBuiltinCallSummarizer.kt:104-107,149-151,258-264,880-930`).
- A rule is classified as a **satisfy rule** if its user-level checks are only
  `CVT_satisfy` calls. Mixing assert and satisfy in one rule throws
  `MixedAssertAndSatisfy("Cannot mix assert and satisfy commands")`. A rule
  with no user assert/satisfy left after compilation throws
  `TrivialRule("Rule contains no assertions after compilation. Assertions may
  have been trivially unreachable and removed by the compiler.")` unless
  `-trapAsAssert true` (`src/main/kotlin/wasm/WasmEntryPoint.kt:448-472`).
- In a satisfy rule every generated assert (overflow traps, storage-miss
  traps and similar) is rewritten to an assume (`rewriteAsserts`,
  `WasmEntryPoint.kt:474-500`). The satisfy statement is encoded as an assert
  of the negated condition (`argToCond`, `WasmBuiltinCallSummarizer.kt:933-960`);
  a SAT result is reported as VERIFIED and UNSAT as VIOLATED
  (`src/main/kotlin/report/TreeViewReporter.kt:66-74`). So **a rule that
  contains only `cvlr_satisfy!` passes if and only if some execution reaches
  the statement with a true condition** — reachability, not universality.
  CVL semantics say the same (docs.certora.com/en/latest/docs/cvl/statements.html,
  `satisfy` statements: "A success only guarantees that there is some
  satisfying execution starting in some arbitrary state.").

### Nondeterminism

- `cvlr::nondet()` is `Nondet::nondet()`; `nondet_with(pred)` draws a value
  and assumes `pred` (`cvlr-nondet/src/core.rs:1-25`). Primitive impls map to
  `CVT_nondet_u8` … `CVT_nondet_i128` and `CVT_nondet_usize` imports
  (`cvlr-nondet/src/scalars.rs:3-97`).
- `cvlr_soroban::nondet_address()` builds an `Address` from a nondet `u64`
  payload `(v << 8) | 77` through `Address::try_from_val`; `nondet_vec()` is
  bounded to at most 5 elements (`if l <= 5`); `nondet_i128()` and
  `nondet_u128()` call `cvlr_nondet_small_i128()` / `cvlr_nondet_small_u128()`;
  `nondet_bytesn()` uses the import `CVT_nondet_bytes_n_32`
  (`cvlr-soroban/src/nondet.rs:10-119`).
- Inferred from the builtin table: the WASM front-end at `0436a658` knows
  `CVT_nondet_i128` / `CVT_nondet_u128` (`WasmBuiltinCallSummarizer.kt:121-122`)
  but lists no `small` variant and no `CVT_nativeint_u64_*` import; an import
  the front-end does not know is lowered as an "unresolved call" annotation
  with a havoced return value (`src/main/kotlin/wasm/impCfg/WasmImpInstr.kt:782-806`).
  Not executed; verify on a live run before relying on `nondet_i128()` or on
  `cvlr::mathint::NativeInt` in Soroban rules.

### Logging and math helpers

- `clog!` is `cvlr::log::cvlr_log` (`cvlr/src/lib.rs`, prelude). It accepts
  `value`, `value => "tag"`, lists, and a `; logger` form
  (`cvlr-log/src/log.rs:49-90`); values are printed through the
  `CVT_calltrace_print_*` imports the prover summarises
  (`WasmBuiltinCallSummarizer.kt:136-143`).
- `cvlr::mathint::NativeInt` is NativeIntU64, a symbolic 256-bit integer
  implemented by the `CVT_nativeint_u64_*` imports (`cvlr-mathint/src/nativeint_u64.rs:1-46`);
  public API includes `new`, `div_ceil`, `muldiv`, `muldiv_ceil()`,
  `from_u128`, `into_u128()`, `from_u256()`, `is_u128()`, `nondet`, `checked_sub`,
  `sext`, `slt`/`sle`/`sgt`/`sge`, `mask`, arithmetic operator impls and
  `From` conversions (`nativeint_u64.rs:200-594`). The crate doc says "Typically,
  this is a 256 bit integer" (`cvlr-mathint/src/lib.rs`). See the Inferred
  note above about WASM support.
- `cvlr::u128_arith` exposes `cvlr_u128_leq()`, `cvlr_u128_gt0()`,
  `cvlr_u128_ceil_div()` over `CVT_u128_*` imports (`cvlr/src/u128_arith.rs`).
- `cvlr_soroban::is_auth(addr)` calls the import `CERTORA_SOROBAN_is_auth`
  (`cvlr-soroban/src/auth.rs`); the prover reads the auth map for the address
  digest (`src/main/kotlin/wasm/host/soroban/types/AddressType.kt:49-54`).

### The two `#[rule]` macros and how the prover finds rules

- `cvlr::macros::rule` (crate `cvlr-macros`): takes no attribute arguments
  (the attribute argument is ignored), adds `#[no_mangle]`, inserts
  `cvlr::log::cvlr_rule_location!();` as the first statement and appends
  `cvlr::cvlr_vacuity_check!();` as the last (`cvlr-macros/src/lib.rs:20-36`,
  identical at the repo's pinned rev `b8bfb4c9`). `cvlr_vacuity_check!()`
  expands to `cvlr_sanity_checked(true)` → import `CVT_sanity`, or to
  `cvlr_assert!(false)` when the `vacuity` cargo feature is on
  (`cvlr-asserts/src/core.rs:94-108`). The function keeps its own name; no
  'rule_' prefix is added.
- `cvlr_soroban_derive::rule`: takes no attribute arguments, emits
  `declare_rule!(name)` and `#[no_mangle]`. `declare_rule!` writes the
  NUL-separated rule names into a static placed in the custom link section
  `"certora_rules"` (`cvlr-soroban-derive/src/rule.rs:24-53`).
- Discovery in the prover (`WasmEntryPoint.kt:136-180`, `WasmLoader.kt:355-374`):
  the loader reads the `"certora_rules"` custom section if present. Each
  listed name must be a WASM export ("Binary lists X as a rule but it is not
  listed as an export"). Names given with `--rule` must be exports ("Invalid
  user-selected entry point"); when the section exists they must also be
  listed in it ("Selected rules are unknown"). With no `--rule` and no
  section the prover logs "No rules selected" and runs nothing. Rule
  arguments are havoced ("havoc the input arguments", `WasmEntryPoint.kt:266`).
- This repository uses `cvlr::macros::rule` (Observed: `use cvlr::macros::rule;`
  in `certora/**/spec/*.rs`), so its artifacts carry no `"certora_rules"`
  section and do import `CVT_sanity` (Observed with `strings` on
  `artifacts/wasm/certora/controller-solvency-rules.wasm`: `CVT_sanity` = 1,
  `"certora_rules"` = 0). Inferred: at `0436a658` the WASM front-end has no
  handler for `CVT_sanity` (absent from `WasmBuiltinCallSummarizer.kt` and
  from the CVT `env` module, which only exports `CERTORA_SOROBAN_is_auth`,
  `CvtEnvModuleImpl.kt:31-46`), so the appended call is lowered as an
  unresolved-call annotation with no effect. The Solana front-end, by
  contrast, rewrites `CVT_sanity` into its own vacuity sub-rule
  (`src/main/kotlin/sbf/cfg/RemoveSanityCalls.kt:23-26`).

## 5. Public Soroban/Sunbeam conventions in first-party repositories

| Repository (revision) | Conf settings | Build command | Cargo release profile |
|---|---|---|---|
| Certora/sunbeam-tutorials (`48c952a4`), `projects/token/confs/*.conf` | `"build_script"`, `"rule"`, `"precise_bitwise_ops": true`; nothing else | justfile: `RUSTFLAGS="-C strip=none"`, `cargo build --release --target=wasm32-unknown-unknown --features certora`; soroban-sdk 22.0.7; cvlr from git with `default-features=false` | opt-level "z", overflow-checks true, debug 0, debug-assertions false, panic "abort", codegen-units 1, lto true; `release-with-logs` inherits with debug-assertions true |
| Certora/meridian2024-workshop (`97177b78`), `confs/*.conf` | adds `"process": "emv"`, `"prover_version": "master"`, `"server": "production"` to the tutorial set | same justfile; its `certora_build.py` sets `RUSTFLAGS="-C strip=none --emit=llvm-ir -C debuginfo=2"` and `COMMAND = "cargo build --target=wasm32-unknown-unknown --release --features certora"` | same as tutorial |
| Certora/aquarius-cantina-fv (`03a865a2`), `fees_collector/confs/starter_verified.conf` | `"build_script"`, `"optimistic_loop": true`, `"process": "emv"`, `"rule"` | justfile as tutorial; soroban-sdk 22.0.6; cvlr 0.4.0 | opt-level "z", overflow-checks true, debug 0, strip "symbols", debug-assertions false, panic "abort", codegen-units 1, lto true |
| code-423n4/2025-02-blend-fv (`fc843bcc`), `blend-contracts-v2/backstop/confs/*.conf` (Certora-run FV contest; the upstream blend-capital/blend-contracts-v2 at `ba22b487` has no Certora files) | `"build_script"`, `"optimistic_loop": true`, `"process": "emv"`, `"rule_sanity": "basic"`; math-heavy confs (`pool.conf`, `withdraw.conf`) add `"precise_bitwise_ops": true`; `withdraw.conf` sets `"server": "production"` | justfile builds dependencies first with `cargo rustc --crate-type=cdylib --target=wasm32-unknown-unknown --release --features certora`; soroban-sdk =22.0.4 | opt-level "z", overflow-checks true ("DEV: Do not remove this check"), strip "symbols", panic "abort", codegen-units 1, lto true |
| Certora/c-fv-sunbeam-demo (`60ff85ee`), `sunbeam.conf` | `"files"`, `"msg"`, `"rule"`, `"prover_args": ["-trapAsAssert true", "-dontStopAtFirstSplitTimeout true", "-mediumTimeout 20", "-depth 5"]`, `"loop_iter": "5"`, `"global_timeout": "3600"` | pre-built wasm | — |
| Certora/sunbeam-vs-other-tools (`8a115995`), `*/sunbeam/confs/comm.conf` | `"build_script"`, `"precise_bitwise_ops": true`, `"rule"` | justfile: `RUSTFLAGS="-C strip=none"`, `cargo build --release --target=wasm32v1-none --no-default-features --features certora`; soroban-sdk 26.0.1 | opt-level "z", overflow-checks true, panic "abort", codegen-units 1, lto true |
| Certora/CertoraProver `Public/TestSoroban/*` (`0436a658`) | `multi_assert/*.conf`: `"multi_assert_check": true`; `symbol_eq/Default.conf`: `"loop_iter": 3`, `"optimistic_loop": true`; `i128FromVal`, `u64FromVal`, `overflow`: `"precise_bitwise_ops": true`; `Meridian2024-workshop/Default.conf`: a `"mutations"` block | `multi_assert/justfile`: `RUSTFLAGS="-C strip=none" cargo build --target=wasm32-unknown-unknown --release --features certora`; `overflow/README.md`: `RUSTFLAGS="-C opt-level=<level>" cargo build --target=wasm32-unknown-unknown --release` for levels 0-3 | test crates use opt-level 2, strip "none", overflow-checks true, panic "abort", lto true; the workshop crate uses opt-level "z" |
| Certora/certora-run-action (`b7140d91`), `tests/soroban/simple.conf` | `"files"`, `"rule"` only | `scripts/run-certora.sh` selects `certoraSorobanProver` for ecosystem `soroban` | — |
| Certora/reflector-subscription-contract (`773ea7be`) | no conf files in the repository; the user guide links its `src/lib.rs#L44` as the ghost-variable example | — | opt-level "z", overflow-checks true, strip "symbols", panic "abort", lto true |
| Soroswap, stellar/soroban-examples | **Not found**: GitHub code search for `cvlr` in `org:soroswap` and in `repo:stellar/soroban-examples` returns 0 results (2026-09-03) | — | — |

Findings on compiler settings:

- Every first-party build uses `RUSTFLAGS="-C strip=none"` (names kept). The
  internal WASM README goes further: `RUSTFLAGS="-C strip=none -C debuginfo=2"`
  (`CertoraProver/src/main/kotlin/wasm/README.md:46-52`). Names matter because
  rules are found by export name (section 4) and the SDK summariser matches
  demangled function names (`src/main/kotlin/wasm/summarization/soroban/SorobanSDKSummarizer.kt:57-116`).
- Every first-party profile keeps `overflow-checks = true`,
  `debug-assertions = false`, `panic = "abort"`, `codegen-units = 1`,
  `lto = true`, with `opt-level = "z"` in projects and `opt-level = 2` in the
  prover's own test crates. The only comparison across optimisation levels is
  `Public/TestSoroban/overflow`: the same crate built at `-C opt-level=0`
  through `3` is expected to give identical verdicts (`expectedOpt0.json` …
  `expectedOpt3.json`, all four rules `SUCCESS`).
- **Not found**: no primary source states that `opt-level = "z"`/`"s"` versus
  `3`, `lto`, or `debug-assertions` makes the prover faster or slower. The
  only related statement is the prover's own error text: "Assertions may have
  been trivially unreachable and removed by the compiler."
  (`WasmEntryPoint.kt:463`).
- The Sunbeam troubleshooting page's build check is
  `cargo build --release --target wasm32-unknown-unknown`
  (docs.certora.com/en/latest/docs/sunbeam/troubleshooting.html, "Build step of
  certoraSorobanProver is failing"; `docs/sunbeam/troubleshooting.rst:31-38`).
- This repository builds prover artifacts with
  `stellar contract build ... --optimize=false` under a profile with
  `opt-level = "z"`, `overflow-checks = true`, `lto = "fat"`
  ([Makefile](../../Makefile) target `certora-wasm`; [Cargo.toml](../../Cargo.toml)
  `[profile.release]`). The `--optimize` switch controls the post-link
  optimiser, not `opt-level`; the local reason for disabling it is recorded in
  [certora/README.md](../../certora/README.md) ("Optimized bytecode can trigger
  internal prover transformation failures"). That is repository experience,
  not a Certora statement.

## 6. Sunbeam modelling facts

Observed in `CertoraProver/src/main/kotlin/wasm/host/soroban/` at `0436a658`.

Host-function resolution:

- Imports are resolved against `src/main/resources/soroban/env.json`
  (modules `context/x`, `int/i`, `map/m`, `vec/v`, `ledger/l`, `call/d`,
  `buf/b`, `crypto/c`, `address/a`, `test/t`, `prng/p`) plus a CVT-only
  module `env` (`SorobanEnv.kt:38-69`). A function present in `env.json`
  whose module implementation returns no body is emitted as a label
  "`<module>/<function> not implemented`" with a **havoced return value**
  (`SorobanHost.kt:146-150`). An import that is not in `env.json` at all
  becomes an unresolved-call annotation with a havoced return
  (`WasmImpInstr.kt:782-806`).
- Implemented (`modules/*.kt`): ledger `"put_contract_data"`,
  `"has_contract_data"`, `"get_contract_data"`, `"del_contract_data"`; address
  `require_auth`, `"address_to_strkey"`; context `"get_ledger_version"`,
  `"get_ledger_sequence"`, `"get_ledger_timestamp"`, `"get_ledger_network_id"`,
  `"get_max_live_until_ledger"`, `"get_current_contract_address"`, `"obj_cmp"`,
  `"fail_with_error"` (trap), `"contract_event"` and `"log_from_linear_memory"`
  (no visible effect); the full `int` module for `obj_from/to_*` pieces and
  u256/i256 add/sub/mul/div/rem/shl/shr (with traps on overflow and division
  by zero); the `vec`, `map`, `buf`, `prng` modules; crypto
  `"compute_hash_keccak256"`.
- **Not** implemented (returns null → havoc): `call` and `"try_call"`
  (`CallModuleImpl.kt:24-25`, "TODO CERT-6437"), `require_auth_for_args`,
  `"strkey_to_address"`, `"authorize_as_curr_contract"`
  (`AddressModuleImpl.kt:29-33`), `"u256_pow"`, `"i256_pow"`, and every ledger
  TTL/extension function (`LedgerModuleImpl.kt:30`, "TODO CERT-6459 CERT-6457").
  Consequence, Inferred: a cross-contract client call in the verified WASM
  returns an unconstrained value and changes no modelled state; a
  `require_auth_for_args` call checks nothing.

Storage:

- Contract data is two maps keyed by `(key digest, storage type)`:
  values and an existence flag, both havoced at rule start
  (`Contract.kt:29-45`). `"get_contract_data"` traps with "Contract data not
  found" when the flag is false (`Contract.kt:69-79`); `"del_contract_data"`
  clears the flag. Storage types are 0 temporary, 1 persistent, 2 instance.

Ledger context:

- Timestamp, sequence, version, max-live-until and network id are havoced
  globals set once at rule start and returned by the context functions
  (`ContextModuleImpl.kt:30-70`).

Authorisation:

- `require_auth` is `Trap.assert("not authorized")` on the auth map entry
  for the address digest; `CERTORA_SOROBAN_is_auth` reads the same entry
  (`AddressType.kt:49-61`). Because a trap is an assume-false by default (see
  below), an unauthorised path is pruned rather than reported.

Traps and panics:

- "Trap/abort are similar to the EVM 'revert' operation ... CVLR/CVLM rules
  have no way to handle this event, and so the default behavior is to
  implicitly assume no abort/trap paths will be taken. We implement this
  behavior by inserting RevertCmd, which the general TAC pipeline treats as
  an `assume(false)`." `-trapAsAssert true` makes rules assert that no trap
  path is taken (`src/main/kotlin/tac/generation/Trap.kt:31-44,95-105`).
  Overflow checks, `unwrap`, storage misses and `"fail_with_error"` all become
  traps.
- `PropagateRevertConditions.kt` inserts `assume !p(x)` ahead of branches that
  must revert, typically Rust enum discriminant checks
  (`src/main/kotlin/wasm/transform/PropagateRevertConditions.kt:22-33`).

Integers:

- Rust `i128`/`u128` multiply and divide compile to compiler-rt calls
  `"__multi3"`, `"__muloti4"`, `"__udivti3"`, `"__divti3"`, `"__modti3"`,
  which the prover summarises as 256-bit TAC multiplication and division with
  sign extension, an overflow flag for `muloti4`, and a "Denominator is 0"
  trap (`WasmBuiltinCallSummarizer.kt:79-90, 413-512`). Soroban host `obj_*`
  pieces are recombined into 256-bit values (`IntType.kt:88-105`). Each such
  operation is a nonlinear SMT term.
- **Not found**: no documented numeric limit on `i128`/`u128` nonlinear
  operations. The generic guidance is timeout.md, "Dealing with nonlinear
  arithmetic": "The main techniques for reducing these numbers are
  modularization and underapproximation." and its note that "There are
  formulas with 10 nonlinear operations that are out of reach of current SMT
  solvers, while in other cases formulas with 120 operations are solved."

Loops and `loop_iter`:

- The WASM pipeline runs LoopHoistingOptimization, summarises and unrolls
  constant array-initialisation loops, then applies the generic
  `convertToLoopFreeCode` (report stage `ReportTypes.UNROLL`), which unrolls with
  `Config.LoopUnrollConstant` (`-b` / `"loop_iter"`) (`WasmEntryPoint.kt:296-315`,
  `src/main/kotlin/vc/data/TACProgram.kt:1818-1824`). The unwind condition
  is asserted unless `"optimistic_loop"` (`-assumeUnwindCond`) is set
  (options.html, `optimistic_loop`; `Config.kt:1297-1304`).
- Host-level loop bound: `"vec_first_index_of"` is under-approximated with
  `Config.LoopUnrollConstant` iterations and traps with "vec_first_index_of
  exceeds loop_iter" when the vector is longer (`VecType.kt:147-172`).
  `nondet_vec()` in cvlr-soroban caps length at 5 (`nondet.rs:43-44`).

Cross-contract calls and summaries in Rust:

- The docs recommend conditional compilation: "For Solana and Soroban, we
  recommend summarizing hotspots by enabling munging with conditional
  compilation." (timeout.md, "Detect candidates for summarization"). The
  Sunbeam guide describes `nondet` summaries and user `Nondet` impls
  (usage.html, "Nondet"; `docs/sunbeam/usage.rst:43-67`).
- `cvlr_soroban_macros::apply_summary!` swaps a function body for a summary
  when `feature = "certora"` is on and keeps the original otherwise
  (`cvlr-soroban-macros/src/apply_summary.rs:1-52`);
  `#[cvlr_mock_client]` generates a mock contract client
  (`cvlr-soroban-derive/src/mock_client.rs`).
- SDK summaries: `-useSorobanSDKSummaries` (default true, `Config.kt:3926-3932`)
  replaces `soroban_sdk::symbol::Symbol::new`, the `i128`/`u64` `TryFromVal`
  conversions and symbol-from-`&str` helpers by name
  (`SorobanSDKSummarizer.kt:57-116`).

## 7. rule_sanity and multi_assert_check on Sunbeam

`"rule_sanity"`:

- CLI values `none`, `basic`, `advanced`; prover default `SanityValues.BASIC`
  (section 1). The CVL-level checks (trivial invariant, assert tautology,
  assertion structure, redundant require) are CVL constructs
  (docs.certora.com/en/latest/docs/prover/checking/sanity.html); the
  WASM flow has only a TAC-level vacuity check.
- Observed at `0436a658`: `WasmVerificationFlow.buildContinuationRules`
  constructs `TACSanityChecks(vacuityCheckLevel = SanityValues.ADVANCED)` and
  `generateRules` only emits a check when
  `Config.DoSanityChecksForRules.get() >= it.sanityLevel`
  (`src/main/kotlin/wasm/WasmVerificationFlow.kt:60-68`,
  `src/main/kotlin/rules/sanity/TACSanityChecks.kt:45-56`; enum order
  `NONE < BASIC < ADVANCED`, `lib/GeneralUtils/src/main/kotlin/cli/Converter.kt:354-360`).
  Inferred: on WASM the vacuity sub-rule `"rule_not_vacuous_tac"` is generated
  only with `"rule_sanity": "advanced"`; `basic` (the default) and `none`
  generate nothing. The Move flow passes `SanityValues.BASIC` and the Solana flow has its
  own `"rule_not_vacuous_cvlr"` sub-rule at `basic`
  (`MoveVerificationFlow.kt:86`, `docs/solana/sanity.md:12-19,56-59`). No
  docs page states this for Soroban. This repository sets `basic` in 66
  confs and `advanced` in 2 (Observed in [certora/profiles.json](../../certora/profiles.json)
  members); re-check on a live report before trusting `basic` for vacuity.
- What the TAC vacuity check does (`TACSanityChecks.kt:142-193`): it runs only
  for rules whose base result is UNSAT (proved), removes every user assert
  and satisfy (`TACMeta.ASSERT_ID` / `TACMeta.SATISFY_ID`), appends `assert false` at every
  sink, and re-solves. SAT means a sink is reachable → status VERIFIED; UNSAT
  means every path is cut by assumes or traps → status SANITY_FAILED
  (`TreeViewReporter.kt:76-84`). So yes: a rule whose `cvlr_assume!` set is
  unsatisfiable (or whose paths all trap) is reported vacuous — when the
  check runs at all.
- `cvlr_vacuity_check!()` appended by `cvlr::macros::rule` is a separate,
  crate-level mechanism: with feature `vacuity` it turns every rule into
  `cvlr_assert!(false)`, so a rule that passes is vacuous and one that fails
  is reachable; without the feature it emits `CVT_sanity(true)`, which the
  WASM front-end at `0436a658` does not handle (section 4).

`"multi_assert_check"`:

- "This mode checks each assertion statement that occurs in a rule,
  separately. The check is done by decomposing each rule into multiple
  sub-rules, each of which checks one assertion, while it assumes all
  preceding assertions." (options.html, `multi_assert_check`). Supported for
  Soroban since prover 8.13.0: "Added multi-assert for Soroban smart contracts
  ... the behavior is the same as with Solidity." (`prover_changelog.md:21-22`).
  First-party test confs: `Public/TestSoroban/multi_assert/BothPass.conf` and
  `SecondFails.conf`.
- `"independent_satisfy"`: "checks each satisfy statement independently from
  all other satisfy statements" (options.html). The splitter honours
  `Config.IndependentSatisfies` for every ecosystem
  (`src/main/kotlin/rules/RuleSplitter.kt:384`). **Not found**: no docs or
  changelog statement about it for Soroban specifically.

## 8. Budgets, hard stops, and the order of remedies

- Timeout classes (timeout.md, "Classification of Timeouts"): (1) before SMT
  starts, (2) SMT queries that sum to the global timeout, (3) a single SMT
  query. "Types 1. and 2. are signified by a hard stop of the Prover. That
  means the Prover ran into the timeout of the cloud job, which is set at 2
  hours ... A message like 'hard stop reached' appears in the 'Global problems'
  pane"; type 3 "is signified by a soft stop", the orange clock next to the
  rule. "Non-SMT Timeouts (Type 1.) should be reported to Certora."
- Per-split budgets (techniques index, "Control flow splitting"; options.html
  `mediumTimeout`, `smt_initialSplitDepth`): non-leaf splits get
  `-mediumTimeout` (prover default 10 s; lower/tiny score splits 5 s / 2 s),
  leaves get `--smt_timeout` (default 300 s), depth is bounded by `-depth`
  (default 10), and the run stops at the first leaf timeout unless
  `-dontStopAtFirstSplitTimeout true`. The parallel splitter has its own
  `-splitParallelTimelimit` (3600 s). `-solvers` selects programs; the
  per-theory lists are `-smt_LIASolvers`, `-smt_NIASolvers`, `-smt_BVSolvers`
  (section 3).
- Job budget: `--global_timeout` "constrains the processing of the entire
  job, including static analysis and other preprocessing" and cannot exceed
  7200 s (options.html, `global_timeout`).
- Documented order of remedies (timeout.md, "Timeout prevention"): "1.
  Changing prover settings 2. Changing specs 3. Changing source code.
  Changing Prover settings is the least invasive and easiest to do, so it is
  usually preferable to the other options." Then, by cause:
  - many rules: run them individually with `--rule` ("Running rules
    individually");
  - high path count: summaries, and splitting settings — eager
    `-smt_initialSplitDepth 5 -depth 15` "When the relevant source code is
    very large"; lazier `-mediumTimeout 30 -depth 5` "When there are very
    many subproblems medium difficulty"; `-splitParallel true`; and, when a
    violation (or a satisfy witness) is expected,
    `-dontStopAtFirstSplitTimeout true -depth 15 -mediumTimeout 5 --smt_timeout 10`
    ("Dealing with a high path count");
  - nonlinear arithmetic: modularisation (summaries at the hot spots shown
    in Live Statistics) and under-approximation (fix a frequently multiplied
    value, or bound its range with a require) ("Dealing with nonlinear
    arithmetic");
  - per-assert splitting: `multi_assert_check` "As a timeout mitigation
    strategy" (options.html);
  - aggressive optimisation: `-destructiveOptimizations enable` at the cost
    of call traces ("Command line options").
- **Not found**: no primary source prescribes the specific sequence
  "first `-depth`/`-splitParallel`, then `-smt_useBV`/`-smt_useLIA`, then
  summaries". The theory-of-BV/LIA choice is only in `Config.kt`
  descriptions: BV "Best used via -preciseBitwiseOps", and the deprecation
  note that NIA-only solving is `-backendStrategy singleRace -smt_useLIA false -smt_useNIA true`.
- Memory: `--max_concurrent_rules` is the documented out-of-memory lever
  (options.html), but it is an EVM-only attribute in 8.17.1 (section 1).

## Open questions / not found

- `"verify_timeout"`, `"rule_timeout"`: no such conf keys or CLI flags exist
  in certora-cli 8.17.1.
- `-calltraceFreeVars`: not present in `Config.kt`, the prover sources, or
  the docs.
- `-adaptiveSolverConfig`: removed; replaced by `-backendStrategy` (changelog
  only).
- `"server"` values: the wheel validates the character set only; no page
  lists valid names. First-party confs use `"production"`.
- `"cache"`: an EVM-only attribute with no docs page; not accepted by
  certoraSorobanProver 8.17.1.
- Whether `"rule_sanity": "basic"` performs any vacuity check on the hosted
  Soroban prover: source at `0436a658` says no (only `advanced` does); no
  documentation covers Soroban sanity. Needs one live run to confirm.
- Whether `CVT_sanity`, `CVT_nondet_small_i128`, `CVT_nondet_small_u128` and
  `CVT_nativeint_u64_*` imports are handled by the hosted WASM front-end:
  absent from the open-source builtin table at `0436a658`; behaviour on the
  hosted build unverified.
- Any first-party guidance on `opt-level`, `lto`, `debug-assertions` or
  `overflow-checks` for prover speed: none found; only the equal-verdict
  opt-level test in `Public/TestSoroban/overflow`.
- Documented `i128`/`u128` nonlinear-arithmetic limits or Soroban-specific
  lemma/`mul_div` summary guidance: none found beyond the generic timeout
  page and the "munging with conditional compilation" recommendation.
- Soroswap, Reflector (beyond the ghost-variable example) and
  stellar/soroban-examples: no Certora conf files found in public
  repositories.
- The Sunbeam user guide's sample conf still contains `"process": "emv"`,
  which the 8.17.1 Soroban attribute set does not define; not executed here.

## 9. Native i128 lowering in the WASM front-end

Research date: 2026-09-03. Question: the rule
`utilization_bounded_when_borrowed_lte_supplied`
(`certora/common/spec/rates_rules.rs:46-55`) reports VIOLATED on a local run
(only `z3` on PATH, `precise_bitwise_ops` false, `-t 900`; conf
`certora/common/confs/rates.conf`) after `common/src/math/fp_core.rs` gained
the native `i128` fast path (`bb4b1832`, 2026-08-26). Can the prover's model
of native i128 arithmetic produce that counterexample?

Additional sources for this section, all read on 2026-09-03:

| Source | How it was read |
|---|---|
| CertoraProver `0436a658`: `src/main/kotlin/wasm/**`, `src/main/kotlin/smt/**`, `src/main/kotlin/verifier/**`, `lib/GeneralUtils/src/main/kotlin/solver/**`, `lib/Shared/src/main/kotlin/config/Config.kt` | cloned; every line number below is at this revision |
| `artifacts/wasm/certora/common-rates-rules.wasm` (mtime 2026-08-26 08:21) | `wasm-objdump -h`, `wasm-objdump -x -j Function` / `-j Import`, `wasm2wat`; function numbers below are WASM function indices (imports are 0-17, local functions 18-62) |
| compiler-builtins 0.1.160, `library/compiler-builtins/compiler-builtins/src/int/{mul.rs,traits.rs}` in the sysroot of the pinned `1.95` toolchain (`rust-toolchain.toml`) | read |
| docs.certora.com `docs/prover/cli/options.html` (`precise_bitwise_ops`), `docs/sunbeam/usage.html`, `docs/sunbeam/troubleshooting.html` | fetched |

No prover run was made for this section. Every "Inferred" statement is a
consequence of the cited code that was not reproduced by a run.

### 9.1 The repository's artifacts carry no function names, so no compiler-rt summary applies

- Observed: `Cargo.toml:58-66` (`[profile.release]`) sets `strip = "symbols"`,
  unchanged since the initial commit `84963027` (2026-06-01). `make certora-wasm`
  builds with `stellar contract build ... --optimize=false` under that profile
  (`Makefile:215-229`); nothing in `Makefile` or `certora/scripts/` overrides
  `strip` or sets `RUSTFLAGS`.
- Observed: `wasm-objdump -h` on `common-rates-rules.wasm` lists Type, Import,
  Function, Memory, Global, Export, Code, Data and the custom sections
  `contractspecv0`, `contractenvmetav0`, `contractmetav0` (twice). There is no
  `name` section. `strings` finds no `__muloti4`, `__multi3`, `__divti3`,
  `__udivti3`.
- Observed: the loader names a local function from the export section, else
  from the `name` custom section, else `FunctionIndex_<idx>`
  (`src/main/kotlin/wasm/WasmLoader.kt:142-166`; `toString` at `:105-109`).
  The compiler-rt table is keyed by name: `MULTI3("$__multi3")`,
  `MULOTI4("$__muloti4")`, `UDIVTI3`, `DIVTI3`, `MODTI3`
  (`src/main/kotlin/wasm/summarization/WasmBuiltinCallSummarizer.kt:85-89`),
  matched with `it.id == f && params && ret` (`:202-205`). `canSummarize` is
  that lookup (`:209`); the inliner inlines every callee the summarizer cannot
  summarize (`src/main/kotlin/wasm/WasmEntryPoint.kt:248-250`).
- Inferred: on this repository's artifacts none of the five compiler-rt
  summaries fires, and neither does the name-matched SDK summarizer
  (section 5). The prover inlines the compiler-builtins bodies and models
  their limb arithmetic instruction by instruction. The hosted prover uses
  the same loader, so this is not a local-versus-cloud difference (the
  deployed build itself is not inspectable).
- Observed in the artifact (`wasm2wat`): the export
  `utilization_bounded_when_borrowed_lte_supplied` is function 54; it calls
  26, which calls 22, of type `(param i32 i64 i64 i64 i64 i64 i64)` = sret
  pointer plus three i128s: `mul_div_half_up` with `try_mul_div_half_up`
  inlined (its first call is function 29 -> 24 -> import 17 `x.5`
  = `fail_with_error`, the `require_nonzero_divisor` panic; the trailing
  `i64.const 141733920771; call 24; unreachable` is the `MathOverflow`
  panic). On the fast path function 22 stores `0` to the flag slot
  (`i32.store offset=44`), calls function 62 of type
  `(param i32 i64 i64 i64 i64 i32)` (the `__muloti4` ABI: sret, a.lo, a.hi,
  b.lo, b.hi, `int *overflow`), reads the flag with `i32.load offset=44`,
  forms `half = d >> 1` with `i64.shr_u 1`, `i64.shl 63`, `i64.or`,
  `i64.shr_u 1`, adds the limbs (`i64.add`, carry by `i64.lt_u` +
  `i64.extend_i32_u` + `i64.add`), tests overflow with `i64.xor`,
  `i64.xor -1`, `i64.xor`, `i64.and`, `i64.lt_s 0`, and calls function 58 of
  type `(param i32 i64 i64 i64 i64)` (`__divti3`). On the widened path it
  calls imports `i.x`, `i.v`, `i.y` (`i256_mul`, `i256_add`, `i256_div` per
  `src/main/resources/soroban/env.json`) through functions 31/33/34
  (`I256::from_i128` via `i.g` = `obj_from_i256_pieces`, `to_i128` via
  `i.j`..`i.m`).
- Observed: function 62 calls function 59 six times; 59 is
  `(param i32 i64 i64 i64 i64)` with 6 `i64.mul`, 2 `i64.and 0xFFFFFFFF`,
  3 `i64.shr_u 32`, 2 `i64.shl 32`, 1 `i64.or` and `i64.lt_u` carries: the
  shape of compiler-builtins `Mul::mul` (`mul.rs:7-29`, four 32x32 partial
  products plus two 64-bit cross products). Function 62 has 2 `i64.or` (zero
  tests on limb pairs), 2 `i64.xor` (sign combination), 6 `i64.sub` and
  `i64.lt_u` (the `wrapping_neg` and `overflowing_add`), matching
  `impl_signed_mulo` (`mul.rs:60-95`) over `UMulo::mulo` (`mul.rs:33-54`),
  where `widen_mul` is `wrapping_mul` on the widened type (`traits.rs:75-80`)
  and so a `__multi3` call. `__muloti4` itself is `mul.rs:125-129`. Function
  58 calls function 57 (`u128_div_rem`: 6 `i64.clz`, 8 `i64.div_u`,
  5 `i64.mul`, 4 `i64.or`, 2 `i64.and`, 26 `i64.sub`), which calls 59 (x4),
  60 (x5) and 61 (x2); 60 and 61 are 128-bit shift-by-variable helpers made
  of `i64.shl`, `i64.shr_u` and `i64.or`.
- Observed: the parent of `bb4b1832` had no `checked_mul` in
  `mul_div_half_up` (`git show bb4b1832^:common/src/math/fp_core.rs`), and the
  artifact's function 22 has the `__muloti4`-shaped call, the flag read and
  the limb add. Inferred: the artifact (built 08:21, commit 18:52) contains
  the fast path. The July verification ran with the same stripped profile
  (Observed: `strip` unchanged since `84963027`) and the same conf
  (`precise_bitwise_ops: false` since `a1530a74`, 2026-06-16), so July and
  August differ only in the code path: host `i256_*` calls, which
  `IntModuleImpl` models exactly with 256-bit TAC arithmetic and overflow
  traps (`src/main/kotlin/wasm/host/soroban/modules/IntModuleImpl.kt:68-71,
  121-140`), versus inlined limb code.

### 9.2 Q1: how `__muloti4`, `__multi3`, `__divti3`, `__udivti3`, `__modti3`, `__umodti3` are summarised

All in `src/main/kotlin/wasm/summarization/WasmBuiltinCallSummarizer.kt`.
Every summary rebuilds each i128 from its two i64 arguments as
`high * 2^64 + low` (256-bit TAC expressions) and writes the result as two
8-byte words at `loc` and `loc + 8`.

| Builtin | Expression | Stored words | Trap | Lines |
|---|---|---|---|---|
| `__multi3` | `(x * y) mod 2^128`, unsigned recombinations | `res mod 2^64`, `res div 2^64`; both stores carry `WASM_MEMORY_OP_WIDTH = 8` | none | 381-403 |
| `__muloti4` | `x`, `y` sign-extended from bit 127 to 256 bits (`signExt128` = `SignExtend(15, .)`, `:480`); `res = x * y`, no `mod 2^128` | low `res mod 2^64`; high `(res sDiv 2^64) mod 2^64`; flag `ite(MIN_I128 <=s res <=s MAX_I128, 0, 1)` stored to the sixth argument; none of the three stores has a width meta | none | 413-443 (flag 431-437, stores 439-441) |
| `__udivti3` | `(x div y) mod 2^128`, unsigned | `res mod 2^64`, `res div 2^64` | `Trap.assert("Denominator is 0")` (`:471`) | 455-478 |
| `__divti3` | sign-extend both, `(x sDiv y) mod 2^128` | same | same (`:508`) | 490-514 |
| `__modti3` | sign-extend both, `(x sMod y) mod 2^128` | same | same (`:537`) | 519-541 |
| `__umodti3`, `__ashlti3`, `__lshrti3`, `__ashrti3` | **not summarised**: absent from the table; `ASHLTI3` is commented out, "May need at some point" (`:90`). Even in a named binary their compiler-builtins bodies are inlined | | | 85-90 |

Findings on the summaries:

- Overflow flag (Observed): the value written is `0` when the exact product
  lies in `[-2^127, 2^127 - 1]` and `1` otherwise, through the pointer in the
  last argument, the same contract as compiler-rt's `int *overflow`. The
  stored product is the truncation of the exact product to 128 bits, whether
  or not it overflows.
- Truncation gap (Observed code, Inferred arithmetic): `sDiv` is EVM signed
  division and truncates toward zero
  (`lib/GeneralUtils/src/main/kotlin/utils/ModZm.kt:147-153`,
  `(a.from2s() / b.from2s()).to2s()`). For a negative product whose low 64
  bits are not zero, `trunc(res / 2^64)` is one above `floor(res / 2^64)`, so
  the stored high word is one greater than the two's-complement high word:
  `res = -(2^64 + 1)` is stored as high `0xFFFF..FFFF`, low `0xFFFF..FFFF`,
  which reads back as `-1`. Non-negative products are exact. Not reachable
  for this rule (`0 <= borrowed`, `RAY > 0`, `supplied > 0`).
- Width meta (Observed): `memStore`
  (`src/main/kotlin/tac/generation/TACGenerationUtils.kt:63-68`) emits a
  `ByteStore` with an empty meta unless one is passed; `__multi3`
  (`:399-400`) and the i128 nondet summary (`:727-728`) pass
  `WASM_MEMORY_OP_WIDTH to 8`, `__muloti4` passes nothing for the two words
  or for the `i32` flag. `MemoryPartitionAnalysis.kt:118-121` treats a
  missing width as 8 bytes when it computes written ranges. The caller
  narrows every load to its own width (`src/main/kotlin/wasm/impCfg/WasmImpInstr.kt:522-531`,
  `mod 2^(8 * width)`), so a 4-byte flag read gives `0`/`1` if the store is
  visible to it. How the memory model resolves overlapping widths: Not traced.
- Signed division with a negative quotient (Inferred from the code): the
  operands are sign-extended to 256 bits, `sDiv`/`sMod` follow EVM semantics
  (truncating; the remainder takes the dividend's sign, `ModZm.kt`), then
  `mod 2^128` keeps the low 128 bits, which is the correct two's-complement
  i128 for any quotient of magnitude below 2^127. `i128::MIN / -1` yields
  `2^127 mod 2^128`, the `MIN` bit pattern; rustc rejects that case before
  the call. Division by zero is a trap, an assume-false unless
  `-trapAsAssert` (section 6).
- `__multi3` sign handling (Observed): an unsigned product modulo 2^128 is
  the correct two's-complement product for signed inputs too.

### 9.3 Q2: bitwise instructions when `-smt_preciseBitwiseOps` is false

TAC lowering (`src/main/kotlin/wasm/impCfg/NumericExpr.kt`; every WASM value
is a `Bit256` TAC variable, `:50`):

| WASM | TAC | Line |
|---|---|---|
| `i64.add`, `i64.sub`, `i64.mul` | `Add`/`Sub`/`Mul`, then `mod 2^64` | 227-231 |
| `i64.and`, `i64.or`, `i64.xor` | `BWAnd`/`BWOr`/`BWXOr` on Bit256, **no `mod 2^64`** (`:223`: "these should all be mod-ed by the bitwidth to be sound") | 241-245 |
| `i64.shl` | `ShiftLeft(a, b mod 64) mod 2^64` | 247 |
| `i64.shr_u` | `ShiftRightLogical(a, b mod 64)` | 249 |
| `i64.shr_s` | `ShiftRightArithmetical(SignExtend(7, a), b mod 64) mod 2^64` | 251 |
| `i64.div_s`, `i64.rem_s` | `SDiv`/`SMod` of sign-extended operands, `mod 2^64` | 235, 239 |
| `i64.lt_u` and the other compares | `ite(Lt(a, b), 1, 0)`; signed compares on `SignExtend(7, .)` | 257-278 |
| `i64.clz`, `i64.ctz` | `BitCounts`: havoc `r`, assume `r <= 64` and `highBit >>l r <= x < highBit >>l (r - 1)` (ctz: `x mod 2^r = 0`, `x mod 2^(r+1) != 0`) | 189-191; `src/main/kotlin/analysis/BitCounts.kt:31-63, 67-99` |
| `i64.rotl`, `i64.rotr`, `i64.popcnt` (and the i32 forms) | `WasmInstruction.NondetStub`, an unconstrained result | `WasmLoader.kt:591-597` |

TAC-to-TAC rewrites that remove bitwise operators before SMT
(`WasmEntryPoint.kt:288-296`):

- `BitopsRewriter.rewriteXorEquality`
  (`src/main/kotlin/wasm/transform/BitopsRewriter.kt:32-58`):
  `((X xor Y) | (X' xor Y')) == 0` becomes `X == Y && X' == Y'`.
- `BitopsRewriter.rewriteSignedOverflowCheck` (`:76-152`): the pattern
  `X = HI_0 xor HI_1 xor (2^64 - 1)`,
  `Y = HI_0 xor (((HI_0 + HI_1) mod 2^64 + C) mod 2^64)`,
  `B = (X & Y) sle -1` (or `slt 0`), with `C` an `ite(v, 1, 0)` (`:199-203`),
  becomes `sign(HI_0) == sign(HI_1) && sign(HI_0 + HI_1 + C) != sign(HI_0)`;
  "The rewritten form is much friendlier to LIA solvers" (`:97`). This is
  rustc's i128 `sadd.with.overflow` limb pattern; function 22 of the artifact
  contains it in the `lt_s 0` form (a `shr_s 63` form would not match the
  comparison pattern, `:212-245`, and `shr_s` has no axiom, see below).
  Whether the matcher fires on this artifact is not verified: it requires
  the carry's `ite` condition to be a variable (`:199-203`).
- `MaskNormalizer.normalizeMasks` (`MaskNormalizer.kt:32-72`): drops a
  `mod 2^k` whose only use is a smaller `& (2^j - 1)`.
- `IntervalBasedExprSimplifier::analyze` (`:296`) simplifies with interval
  facts; interval transfer functions for the shifts are in
  `analysis/opt/intervals/ForwardCalculator.kt:98-100`.

SMT encoding under LIA/NIA (`precise_bitwise_ops` false):

- Observed: `ExpNormalizerIA` replaces `BWAnd`/`BWOr`/`BWXOr` and the three
  shifts by the uninterpreted functions `uninterp_bwand`, `uninterp_bwor`,
  `uninterp_bwxor`, `uninterp_bwshl`, `uninterp_bwlshr`, `uninterp_bwashr`
  (`src/main/kotlin/smt/axiomgenerators/fullinstantiation/expnormalizer/ExpNormalizerIA.kt:43-48,
  173-180`; symbols in
  `src/main/kotlin/smt/solverscript/functionsymbols/AxiomatizedFunctionSymbol.kt:304-347`).
  There is no havoc and no bounded nondeterminism: each operator is a
  function symbol constrained only by the axioms below, so it is consistent
  across equal arguments but otherwise free.
- Axioms added per occurrence
  (`src/main/kotlin/smt/axiomgenerators/fullinstantiation/BitwiseAxiomGenerator.kt:69-192`;
  definitions in `src/main/kotlin/smt/axiomgenerators/BitwiseAxiomsDefs.kt`;
  `-smt_bitwisePrecision` default 4, `Config.kt:3167-3173`; precision 0 means
  no axioms):
  - `and`: `a & MAX = a`, `a & 0 = 0`, `a & a = a`, commutativity,
    `0 <= a & b <= min(a, b)` for non-negative operands
    (`BitwiseAxiomsDefs.kt:44-92`). With a constant mask (`:189-268`): exact
    as `a mod 2^k` for masks of the form `0..0 1..1` (precision >= 2,
    `:198-203`), exact for `1..1 0..0` (`:206-214`), for one interval of ones
    (>= 3, `:217-227`) and one interval of zeros (>= 4, `:229-238`); other
    masks get implication-only axioms (`:241-268`). The
    `i64.and 0xFFFFFFFF` in `__multi3` is therefore exact.
  - `or`: `a | MAX = MAX`, `a | 0 = a`, `a | a = a`, commutativity, and
    **only bounds for two variables**: `a | b >= a`, `a | b >= b`,
    `a | b <= a + b` (`:96-135`). Constant-mask cases at `:279-321`.
  - `xor`: `a ^ MAX = MAX - a`, `a ^ 0 = a`, `a ^ a = 0`, commutativity,
    bounds `0 <= a ^ b <= a + b` (`:139-181`); no constant-mask axioms
    ("`//TODO: Xor`", `:324`); constant-constant is folded
    (`BitwiseAxiomGenerator.kt:173-176`).
  - `signextend`: exact, through `a & lowOnes` and an ite on the sign bit
    (`BitwiseAxiomsDefs.kt:331-343`).
  - shifts (`src/main/kotlin/smt/axiomgenerators/fullinstantiation/IntMathAxiomGenerator.kt:40-46,
    104-141`): `a >>l b = ite(b < 256, a / 2^b, 0)`;
    `a <<l b = ite(b < bitwidth, a * 2^b, 0) mod 2^bitwidth`; `2^b` is a
    literal for a constant `b`, otherwise `uninterp_exp(2, b)` with the table
    `2^0 .. 2^256` (`src/main/kotlin/smt/axiomgenerators/BasicMathAxiomsDefs.kt:273-288`).
    Logical shifts are therefore exact, also for variable amounts up to 256.
    **`ShiftRightArithmetical` has no axiom anywhere**: outside the symbol
    declaration and the normaliser lines above, the only handling is the
    trivial constant cases in
    `src/main/kotlin/analysis/opt/ConstantPropagatorAndSimplifier.kt:574-578`.
    Under LIA/NIA `i64.shr_s` is an unconstrained function of its operands
    (Observed absence; the BV encoding maps it to `bvashr`,
    `ExpNormalizerBV.kt:120-125`).
  - `mul` of two non-constants: `IntMul` in NIA, `UninterpMul` under LIA
    linearization, both followed by `applyModulo`
    (`ExpNormalizerIA.kt:59-67, 272-283`); `div` by a non-constant becomes
    `UninterpDiv` under linearization (`:303-306`).
- Summary per operator: `shl`, `shr_u`, `clz`, `ctz`, `and` with a
  power-of-two mask, `signextend`: exact. `and`, `or`, `xor` between two
  variables: bounds only. `shr_s`, `rotl`, `rotr`, `popcnt`: unconstrained.
- Consequence for the LIA counterexample verifier (Inferred): `UseLIA`
  defaults to `WITH_VERIFIER` (`Config.kt:2948-2963`), so a LIA SAT is
  re-checked by `CEXVerifier` with `SolverConfig.z3.default`
  (`src/main/kotlin/verifier/LExpVcChecker.kt:415-431, 650-671`;
  `src/main/kotlin/verifier/Executable.kt:44-71`). The verifier's query uses
  the same `BitwiseAxiomGenerator` (the theory only selects the precision
  knob, `BitwiseAxiomGenerator.kt:44-52`), so a model that exists only
  because `or`, `xor` or `shr_s` are under-constrained passes verification
  and is reported as VIOLATED.
- `precise_bitwise_ops` true switches to `UseBV` with LIA and NIA off
  (`Config.kt:2922-2926, 2966-3003`). The docs call the default "the default
  overapproximation": "This option models bitwise operations exactly, instead
  of using the default overapproximation." and "Use this option if a
  counterexample suggests that incorrect modeling of bitwise operations is
  affecting verification results." (options.html, `precise_bitwise_ops`,
  fetched). The Sunbeam usage and troubleshooting pages contain no statement
  about bitwise modelling, `strip`, function names or i128 (fetched; Not
  found).

### 9.4 Q3: reconstruction of 128-bit values

- Observed: every compiler-rt summary rebuilds an i128 as
  `high * 2^64 + low` from the two i64 call arguments
  (`WasmBuiltinCallSummarizer.kt:389-390, 422-423, 463-464, 497-499,
  526-528`); `__muloti4`, `__divti3` and `__modti3` sign-extend that sum from
  bit 127 (`:480`). The recombination is exact when both limbs lie in
  `[0, 2^64)`.
- Observed sources of the limb range: rule parameters are havoced with
  `assume v < 2^64` for `i64` (`WasmCfgToWasmImpCfg.kt:515-522`,
  `WasmImpInstr.kt:90-99, 831-839`); loads are narrowed to their width
  (`WasmImpInstr.kt:522-531`); `CVT_nondet_i128`/`u128` write two
  range-assumed words (`WasmBuiltinCallSummarizer.kt:706-729`); arithmetic
  results are reduced `mod 2^64` (`NumericExpr.kt:227-231, 247, 251`).
  `BWAnd`/`BWOr`/`BWXOr` results are not reduced (`:241-245`); with only the
  bound axioms `a | b <= a + b`, `a ^ b <= a + b`, an `or`/`xor` result used
  as a limb may exceed 2^64 under LIA/NIA (Inferred).
- Observed: Soroban host integers are recombined as
  `sum(piece_i * 2^(64 * (n - 1 - i)))` and split with `div`/`mod` by powers
  of two (`src/main/kotlin/wasm/host/soroban/types/IntType.kt:77-105`), with
  `assume v <= allBitsSet` on every read (`:66-75`): exact.
- Not found: any place where a summary rebuilds an i128 from memory words;
  the summaries take limbs from call arguments only.
  `summarize128BitCalltrace` (`:810`) also takes two i64 arguments.

### 9.5 Q4: what `Public/TestSoroban/overflow` covers

- Observed: `src/lib.rs` has four rules on i128 **addition** only:
  `test_overflow_add` (`assume x >= 0 && y >= 0; assert x <= x + y`),
  `test_underflow_add` (both `<= 0`; `x >= x + y`) and their two
  `cvlr_satisfy!` twins. No multiplication and no `checked_*` call.
- Observed: `Opt0.conf`..`Opt3.conf` each set `precise_bitwise_ops: true`
  and list the four rules; `expectedOpt0.json`..`expectedOpt3.json` are
  identical: all four `SUCCESS`, the satisfy rules "Property satisfied".
  `README.md`: the wasms are the same crate built with
  `RUSTFLAGS="-C opt-level=<level>" cargo build --target=wasm32-unknown-unknown --release`;
  `Cargo.toml` keeps `strip = "none"`, `overflow-checks = true`, `lto = true`.
- Observed: `strings test_opt_0.wasm` contains `__muloti4`, `__multi3`,
  `__udivti3`, `__umodti3`, `__ashlti3`; `test_opt_1/2/3.wasm` contain none
  of them. Inferred: at opt-level >= 1 the unused builtins are dropped, and
  no rule calls them at any level; the test exercises the limb
  `sadd.with.overflow` pattern of 9.3 under the **BV** encoding only.
- Not found: any Soroban/WASM test of `checked_mul` on i128, with or without
  a real overflow, and any test of the compiler-rt summaries under LIA/NIA.
  `grep -rl 'muloti\|checked_mul\|overflowing_mul\|__multi3' Public src/test`
  hits only Solana tests (`Public/TestSolana/MathEquivTest/src/tests.rs:59`,
  `for_all!(bin_equiv, checked_mul)`, a different front-end) and two debug
  WAT fixtures under `src/test/resources/wasm/wasm_wat/` whose assertions
  were not examined. `i128FromVal` and `u64FromVal` (section 5) also run with
  `precise_bitwise_ops: true`.

### 9.6 Q5: solver discovery in the jar

- Observed: a solver is "available" when its default command answers
  `--version`: `SolverInfo.isAvailable()` / `isDefaultCommandAvailable`
  (`lib/GeneralUtils/src/main/kotlin/solver/SolverInfo.kt:46-55`) call
  `RuntimeEnvInfo.getSolverVersionIfAvailable`, which is
  `Runtime.getRuntime().exec(cmd)`
  (`lib/GeneralUtils/src/main/kotlin/utils/RuntimeEnvInfo.kt:50-55, 96-98`),
  a PATH lookup of `z3`, `cvc5`, `yices-smt2`, `bitwuzla`
  (`Z3SolverInfo.kt:49`, `CVC5SolverInfo.kt:50`, `YicesSolverInfo.kt:23`,
  `BitwuzlaSolverInfo.kt:31`).
- Observed: `-solvers` (`-s`, `-solver`) defaults to
  `AllCommonAvailableSolversWithClOptions`, "All configurations from solvers
  from this set { z3, cvc4, cvc5, yices, bitwuzla } that are available on
  this system" (`Config.kt:2506-2520`); that list is the prioritised
  configuration list filtered by `isDefaultCommandAvailable`
  (`SolverInfo.kt:161-171, 184-206`). `LExpVcCheckerConfig` then takes, per
  theory and timeout, a predefined list and keeps the members present in the
  (user or default) choice, appending user-chosen configurations that
  qualify (`src/main/kotlin/verifier/LExpVcCheckerConfig.kt:111-140,
  158-200, 262-264`; override instead of filter with `-smt_overrideSolvers`,
  `Config.kt:3054-3062`). The race for a split uses `config.niaSolvers` or
  `config.liaSolvers` by the query's arithmetic
  (`LExpVcChecker.kt:373-393`).
- Observed predefined NIA lists (`SolverInfo.kt:270-326`):
  `NIASolversLargeTimeout` (timeout above 10 s, so the `-t 900` case) =
  `yices:def, z3:def, cvc5:nonlinNoDio, cvc5:nonlin, cvc4:nonlin, z3:arith1,
  z3:eq2, z3:eq1, z3:arith2, cvc5:def, cvc4:def, cvc5:q`; the medium and
  small lists have the same members in a different order. LIA lists are at
  `:207-268`, the BV list at `:328`.
- Inferred: with only `z3` on PATH the NIA race is
  `z3:def, z3:arith1, z3:eq2, z3:eq1, z3:arith2` and the LIA race is
  `z3:lia2, z3:lia1, z3:arith2, z3:eq2, z3:eq1, z3:arith1, z3:def`. Missing:
  `yices:def` (`yices-smt2`), `cvc5:nonlin`, `cvc5:nonlinNoDio`, `cvc5:def`,
  `cvc5:q`, `cvc4:nonlin`, `cvc4:def` for NIA, and `cvc5:lin`,
  `cvc5:linNoDio`, `cvc4:lin`, `yices:def` for LIA. The LIA CEX verifier
  (`z3:def`) is present. Fewer solvers change run time and which
  configuration wins; they cannot turn an UNSAT encoding into SAT, so a
  VIOLATED verdict is a model of the same formula the cloud would solve.
- Not found: the solver binaries on the hosted prover's PATH; no docs page
  lists them. The discovery code is the same jar in both places.

### 9.7 Verdict

(a) Most plausibly a bitwise over-approximation artifact, with the specific
root cause that the exact compiler-rt summaries are inert on this
repository's artifacts. Chain:

1. Observed: the artifact has no `name` section (`strip = "symbols"`), so
   `__muloti4`, `__multi3` and `__divti3` are inlined compiler-builtins limb
   code (9.1), not the exact 256-bit summaries (9.2).
2. Observed: that limb code recombines partial results with `i64.or` on two
   non-constant operands (`__multi3`, function 59; the shift helpers 60/61
   and `u128_div_rem`, function 57) and tests signs with `i64.xor`/`i64.and`
   (function 22). Under LIA/NIA these are function symbols with bound axioms
   only (`max(a, b) <= a | b <= a + b`, `0 <= a ^ b <= a + b`); `i64.shr_s`
   has no axiom (9.3).
3. Inferred: an under-constrained `or` in the divisor normalisation of the
   inlined `__udivti3` (`(hi << n) | (lo >> (64 - n))` may be modelled below
   its true value, the true value being the sum of the two disjoint terms)
   lets a 64-bit `div_u` step return a quotient above the truth, which is
   enough for `util > RAY`. The multiplication side can at most lose carries
   and cannot push the quotient above `RAY` while `borrowed <= supplied`.
   The July verdict used the same encoding on a path whose only bitwise
   operators were the sign words of `I256::from_i128`/`to_i128` around exact
   host `i256_*` calls, which cannot inflate a quotient.
4. The LIA counterexample verifier shares the bitwise axioms, so it does not
   filter such a model (9.3).

(b) is not the cause: the `__muloti4` summary writes a correct 0/1 flag
through the pointer, its only gaps (`sDiv` high word for negative products;
no width meta) are unreachable for non-negative operands, and the summary
never ran (9.1, 9.2). (c) is not the cause: the solver set changes speed and
the winner, not satisfiability (9.6). (d) does not apply: the open-source
model admits a spurious SAT.

What would confirm it (not executed here): rebuild the certora artifacts
with names kept (for example `CARGO_PROFILE_RELEASE_STRIP=none` on the
`certora-wasm` target, or a dedicated profile; Unverified that
`stellar contract build` forwards the variable), check with
`wasm-objdump -h` for a `name` section and
`wasm-objdump -x -j Function | grep muloti`, then re-run the rule. With
names present the five summaries fire, `__muloti4` becomes one exact 256-bit
product with an exact flag, `__divti3` one exact `sDiv`, and the bitwise
limb code leaves the TAC. A second run with `precise_bitwise_ops: true` on
the stripped artifact (BV encoding, exact `bvor`/`bvashr`) separates
"bitwise axioms" from "solver". `-smt_bitwisePrecision` cannot help: its
levels only refine constant masks, not `or`/`xor` between two variables.
