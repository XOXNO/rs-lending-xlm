# Local Komet toolchain

These harnesses call production Rust through local Cargo dependencies.
[Pool math](pool-math/README.md), [pool state](pool-state/README.md) and
[oracle freshness](oracle-staleness/README.md) record individual claims and their limits. The full pool is not proved.
Earlier arithmetic-bearing symbolic results remain provisional. The unsafe
inherited KWasm optimization module is removed; fresh proofs must use rebuilt
semantics.

## Source and versions

The complete reviewed baseline is published on
[`XOXNO/komet`, branch `proof/sdk28-host-models-2026-09-29`](https://github.com/XOXNO/komet/tree/proof/sdk28-host-models-2026-09-29),
pinned at [`157b6f3`](https://github.com/XOXNO/komet/commit/157b6f3e3b56ac43aadf4a3af3a24777f18fa92e).
Historical `/tmp/komet-sdk28` paths in the evidence are not persistent checkouts.
The pinned commit includes the earlier
`87586bd` repairs to forced-allocation tags, integer host ABI/arithmetic,
macOS Booster worker stacks and proof bookkeeping, plus guarded symbolic word
bounds and memory wrapping. It also normalizes signed casts, Boolean words and
full masks, prunes only solver-confirmed infeasible branches, and rejects
empty-domain initial states. Guarded right shifts, unsigned conversions and
four-word packing and XOR with zero now expose their arithmetic to the solver. Test discovery
filters unrelated endpoints before parsing unsupported struct arguments; selected
unsupported signatures still fail. Concrete fuzzing uses the installed KRun
interpreter command and separates completed Boolean failures from crashes or
unknown stopped execution. The frontend revisions passed 131 unit checks; the
actual pool witness passes through the CLI and the false half-two control fails.
The stateful proof target now permits changes to four persistent-world cells
while keeping initial state, assertions and termination fixed; all 148 unit
checks pass. Guarded CLZ comparisons and masked OR-zero rules passed 53
compiled regressions with independent review. The runner now limits each
execution RPC to 1,000 rewrites and checkpoints its pending continuation;
this is not a total proof bound. Its resume regression and all 149 unit tests
pass. A real 1,000-rewrite checkpoint was reloaded independently; that
time-limited diagnostic did not complete a proof. Reversed CLZ comparisons and
masked XOR sign-bit normalization now pass 74 combined backend checks, with
independent source, definedness, fixture and artifact review. All 149 unit checks
also pass after that integration. Guarded masked-complement rules bring the
combined backend checks to 90, with independent review. General bounded
two-word packing brings this to 111 checks, including all earlier four-limb
regressions. Guarded signed-word normalization records 133 distinct passing
backend cases across focused, prior and retry runs; see the precise breakdown below. The arithmetic and full-i128 storage proofs remain incomplete.
Booster now runs in a unique temporary directory to protect caller files from
its fixed-name crash reports. This operational repair passes 153 unit checks
and an actual typed RPC startup check. The shared runtime now starts SMT
queries at 5000 ms, with the native retry policy unchanged. The previously
timed-out range query returns typed UNSAT in 0.585 seconds in the separate
OR-model diagnostic; it was not rerun on the current signed cache. The integrated
runtime passes 153 unit checks in 2.68 seconds; no new contract proof follows.
These repairs are included in the published `157b6f3` baseline. Preserve
its complete source, including
`src/komet/native_stack.c`, its Python launcher and regression tests, with the
run evidence. The repair commit includes these files; a source archive must
retain them too.

The local review artifacts are collected in
`/Users/mihaieremia/.codex/visualizations/2026/09/26/01a0ded9-99d4-7d02-bbb7-d1cb3fcfd966/komet-pool-math-proof`.
The complete current source is archived under `repaired-toolchain/komet-source.tar.gz`,
with a per-file SHA-256 manifest and the base Git commit. Keep this source and
evidence together when moving machines.

Recorded environment: Rust `1.95.0`, Stellar CLI `28.0.0`, K and Python
`kframework`/pyk `7.1.337`, KWasm `0.1.155`, Python `3.10`, macOS arm64.
The fork now pins K and pyk to `7.1.337` in its Python and Nix locks; KWasm
remains `0.1.155`. Lock consistency was checked offline. The fork's older
CONTRIBUTING guide mentions Stellar CLI 23; these harnesses use CLI 28.

## Build and run from the fork

With K `7.1.337`, its backend tools, Rust and Stellar CLI on `PATH`, run from the
preserved fork source:

```sh
uv sync --locked --python 3.10
export XDG_CACHE_HOME="$PWD/.komet-cache"
if [ "$(uname -sm)" = 'Darwin arm64' ]; then export APPLE_SILICON=true; fi
uv run --locked komet-kdist -v build -j2 'soroban-semantics.*'
uv run --locked pytest src/tests/unit
```

Use the same cache and locked environment for every runtime command. Rebuild the
definitions after changing any semantics source. The Apple Silicon setting
above follows the fork's Nix build.
The macOS launcher compiles `native_stack.c` with `xcrun clang` on first use;
Xcode command-line tools are therefore required. It enlarges default worker
stacks to 64 MiB while preserving explicit thread attributes. This finite-stack
workaround changes execution resources, not arithmetic rules.
The launcher prints each retained working directory, including after successful
runs. Explicit reports keep their requested path and request IDs. Pinned pyk's
auxiliary `server_version.txt` identifies Python when using the wrapper; record
the real Booster executable's version and hash separately.

From the lending repository root, build one harness group. The default Cargo
features include the initial four groups; `--all-features` includes all claims.
Selecting one feature reduces Wasm allocation overhead.

```sh
RUN_DIR=$(mktemp -d "${TMPDIR:-/tmp}/pool-math-run.XXXXXX")
stellar contract build --locked --manifest-path komet/pool-math/Cargo.toml \
  --no-default-features --features controls --out-dir "$RUN_DIR/wasm"
KOMET_SOURCE=/tmp/komet-sdk28
uv run --project "$KOMET_SOURCE" --locked komet test \
  --wasm "$RUN_DIR/wasm/pool_math_proof.wasm" \
  --id test_full_utilization --max-examples 1
uv run --project "$KOMET_SOURCE" --locked komet prove run \
  --always-allocate --wasm "$RUN_DIR/wasm/pool_math_proof.wasm" \
  --id test_full_utilization --proof-dir "$RUN_DIR/proof"
```

`komet test` is concrete execution/fuzzing; `komet prove run` is symbolic.
`--always-allocate` belongs to `prove`, not `test`. The false control is
`test_full_utilization_wrong`; run it into a separate fresh proof directory.
Its assertion must fail after Wasm execution. A backend crash or timeout is not
a successful negative control. See each harness guide for symbolic input
domains and the other Cargo feature groups.

These portable setup commands are a reproduction procedure, not a claim that
a fresh installation or every listed proof has completed.

## Existing local runtime

The current local runs use a Nix-backed Python launcher and rebuilt semantics
without `KWASM-LEMMAS`. The launcher's default points to an older cache;
always select the intended cache explicitly.

```sh
export KOMET_SRC=/tmp/komet-sdk28
export KOMET_CACHE=/tmp/komet-signed-in-range-cache
/tmp/komet-python -m komet prove run --always-allocate \
  --wasm "$RUN_DIR/wasm/pool_math_proof.wasm" \
  --id test_full_utilization --proof-dir "$RUN_DIR/local-proof"
```

`/tmp/komet-python` and its Nix store paths are machine-specific. Preserve that
launcher with the evidence; use the fork/uv procedure on another machine.
The current Haskell, LLVM-library and concrete LLVM targets resolve under
`/private/tmp/komet-signed-in-range-cache/kdist-0d7b9c5/soroban-semantics/`.
All 27 cached model files match this committed model revision. The fresh
concrete LLVM target built successfully; the symbolic artifacts retained
their recorded hashes.

| Compiled input | SHA-256 |
|---|---|
| `haskell/definition.kore` | `2b614851e1c34f525ffc8c7b3477fcfb01da0be015d9b102aeb7691db326b42f` |
| `llvm-library/definition.kore` | `4016b60c9bc27ba435cdcda4f5ed24d118d88a491b9f03be47d1ff294484e7c6` |
| `llvm-library/interpreter.dylib` | `648ecdc85d142fdae4579b8edeb50855a8afce7cb8dfcb858b111d716bbdbe6a` |
| `src/komet/proof.py` | `a0018d69561fcfc0a74505d569c23ff49c50f2877d35af7b0bb0f4b1decdad64` |

Native-library bytes may differ on another platform; record that rebuild's
hashes rather than attributing the old proof to it.

Operational isolation evidence is under `booster-working-directory/`, including
failure regressions that preserve pre-existing caller files, relative-path replay
checks, an explicit RPC report and 4,840 unchanged input hashes. Actual report
replay was not executed. Earlier arithmetic runs retain their original frozen
runner; changing this runner requires a fresh proof directory.

The current signed-word repair exposes both sign branches at every actual width,
retaining the original unsigned-word domain and default definedness. Its 22 new
checks passed; the prior suite recorded 110 passes and one startup failure, then
the exact missing case passed with unchanged inputs. Thus 133 distinct cases
passed across runs, not a clean 133-case run. The original failure is retained;
socket-discovery versus process-exit cause remains unresolved. All 116 concrete
modules match the preceding model after path/order normalization. Evidence is
under `signed-in-range/`.

The SMT budget evidence is under `smt-budget/`. The 5000 ms value is the initial
per-query timeout; default retries can increase it. Unknown remains unproved.
Neither model controls nor solver-budget checks complete a contract theorem.

The preceding two-word packing repair passes 21 new cases plus 90 earlier
regressions. It preserves arbitrary signed high words, requires a bounded low
word and concrete nonnegative width, and retains default definedness. All 116
parsed concrete modules match the preceding model modulo source paths and
declaration ordering. Sources and compiled/runtime inputs stayed unchanged.
Evidence is under `booster-native-stack/word-pack-general/`. The original
full-i128 storage proof advanced but remains incomplete; its interrupted
graph is under `booster-native-stack/storage-roundtrip-packed/`.

The preceding complement repair passes 16 new cases plus 74 earlier regressions.
The complete impossible half-up paths close; feasible siblings and witnesses
remain feasible. Sources, compiled artifacts and runtime inputs were unchanged
throughout both successful runs. Evidence is under `masked-complement/`; the
documented modulus-one matching limit does not narrow the half-up claim.

The preceding reverse-comparison and XOR-sign repair passes 14 new cases plus
60 earlier regressions. The 116 concrete modules are unchanged after metadata
and ordering normalization. Evidence, complete candidate source and a replay
command are under `xor-sign/`. A runtime stack library was generated during the
first test startup; its source was monitored throughout and its binary was
captured unchanged in subsequent inventories. The archive records that boundary.

The actual pool fixed-supply proof remains incomplete: original and smaller
Wasm attempts were interrupted after 23.3 and 26.1 minutes with only initial
and target nodes. No speedup or formal verdict was established; see
`pool-state/rooted-proof/`. These resource observations do not change any claim.

The CLZ and masked-OR revision passed 12 new backend cases, 35 prior
focused checks and six existing symbolic regressions. Exact impossible
branches close, while feasible siblings retain independently checked
witnesses. All five new rules retain definedness checks and are excluded
from concrete semantics; the 116 concrete parsed modules match the prior
model after normalizing path metadata and ordering. No universal pool
claim follows from these regression checks.

The XOR revision passed four new backend cases, 31 prior focused checks and
six existing symbolic regressions. Its exact infeasible signed-half path closes;
a feasible sibling and its concrete witness remain feasible. The concrete
model excludes these symbolic rules; all 116 parsed concrete modules match the
preceding word-conversion model after normalizing source paths and ordering.
No fresh concrete replay is claimed for XOR. Evidence is under `xor-zero/`.

The preceding word-conversion revision passed 31 focused, six symbolic and
49 concrete checks. Its exact impossible constructor path closes with all 12
original constraints retained, and its false half-two control fails at the
returned Boolean after normal Wasm execution. Evidence is under `word-concat/`
and `booster-native-stack/word-concat-regressions/`. These checks validate repairs,
not the full pool. Earlier proof models retain their own source snapshots under
`repaired-toolchain/previous-<commit>/`.

## Evidence to retain

Keep the exact harness and production source revision/diff, complete fork
source, tool versions, selected features, Wasm SHA-256, compiled-definition and
native-library hashes, command line, exit status, logs and complete proof
directory, including `inputs.json`. The manifest also binds `proof.py`, which
implements infeasible-path pruning. Use a fresh proof directory after any
artifact, model or runner change. Historical graphs without the current input manifest
remain historical evidence; do not silently resume them as current results.

A completed APR proof must report `PASSED`, no pending/failing/stuck or bounded
nodes, and no admissions, circularity or unreviewed subproofs. The initial input
domain must be feasible. An infeasible child branch may close only with
reproducible `UNSAT` evidence for its unchanged constraints; review every such
vacuous node. Unknown solver results do not justify pruning. Inspect the
executed graph and actual property body; an early-return-only result does not
establish the intended arithmetic domain. A green fuzz test, compiled harness
or checkpoint alone establishes no symbolic theorem.

The fork still has explicit host-model limits, including authorization, events,
guest object isolation and native resource budgets. Read its
`docs/sdk28-support.md` with the proof result. Source packaging also needs to
retain the C launcher input: importing the Python package alone does not check
that runtime compilation can find it.
