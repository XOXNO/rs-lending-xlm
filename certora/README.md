# Certora Formal Verification

Formal verification for the lending protocol's critical invariants, using
[Certora Sunbeam](https://docs.certora.com/en/latest/docs/sunbeam/index.html)
(Rust + CVLR `#[rule]` specs compiled to WASM).

## Layout

```text
certora/
├── common/          # fixed-point and rate math
├── pool/            # minimal pool-core rate/share/cash/fee accounting suite
├── controller/      # solvency, liquidation, oracle, strategy rules
│   ├── harness/
│   ├── confs/
│   └── spec/        # README.txt — domain invariant + conf map
├── shared/          # cross-contract summaries
├── scripts/         # Python entrypoints, wasm helpers, run-all.sh wrapper
├── profiles.json    # sanity | fast | core | heavy | manual | all
└── compile_all.sh
```

Partitioning starts at the crate boundary (`common` / `pool` / `controller` /
`price-aggregator`) and then narrows each prover artifact to one rule-source
module. Domain docs live in each layer's `spec/README.txt`.
Cross-reference: [`architecture/INVARIANTS.md`](../architecture/INVARIANTS.md) (the enforceable properties these rules protect, including numeric model, pool accounting, account solvency, oracle, storage/boundaries, pause/freeze, and bad-debt socialization) and the verification surface in `SCF_BUILD_ARCHITECTURE.md §14`.

See also the central implementation facts: controller owns risk/oracle/strategies and is sole caller of the pool; governance owns controller; new deployments start paused; 3-layer pause/freeze; GUARDIAN for immediate per-listing actions; keeper self-authorizes.

## WASM artifacts (deploy + prover)

Production and Certora WASM share one tree under `artifacts/wasm/`:

| Path | Built by | Used for |
| --- | --- | --- |
| `deploy/pool.wasm`, `deploy/controller.wasm` | `make deploy-artifacts` | Mainnet deploy / upgrades |
| `certora/<layer>-<rule-module>.wasm` | `make certora-wasm` | Certora conf `files` entries, one rule module per artifact |

```bash
make wasm-artifacts          # both deploy + certora
make certora-wasm            # prover only (rebuild after contract/spec changes)
```

Conf files reference prebuilt focused `certora/*.wasm` files so Certora cloud
skips `stellar contract build`. Each config also declares the exact three
Cargo features recorded in the artifact manifest: `certora`,
`certora-focused`, and its rule-module feature. Rebuild locally, then submit
jobs. `check_wasm_artifacts.py` rejects artifact hashes, source fingerprints,
paths, or feature declarations that do not match.

The focused build removes unrelated `#[rule]` exports before the Prover's
initial WASM transformation. The feature is used only by rule-module gates;
production behavior has no `certora-focused` branch. This is intended as a
transformation RAM/time optimization, not a separate equivalence proof. The
full non-focused `certora` feature path is still compiled by `compile_all.sh`
as a compatibility check.

**Important:** `make certora-wasm` uses `stellar contract build --optimize=false`.
Stellar's WASM optimizer can emit bytecode that passes `wasm-validate` but triggers
Certora internal errors on large controller builds, e.g.:

```text
Inconsistent ref stack sizes in preds ... FunctionIndex_294
```

Mainnet deploy still uses optimized WASM from `make deploy-artifacts`.

## Local checks

```bash
./certora/compile_all.sh
./certora/compile_all.sh --wasm   # also builds + checks certora WASM
```

Runs `cargo check` for all `certora` feature paths, then `check_orphans.py`
(conf ↔ `#[rule]` alignment) and `check_invariant_coverage.py` (INVARIANTS.md
↔ spec modules).

## Local prover (no cloud)

The open-source [CertoraProver](https://github.com/Certora/CertoraProver) is
built on this machine and runs our Soroban confs fully locally:

| Piece | Location |
| --- | --- |
| Prover source | `~/certora-work/CertoraProver` |
| Built artifacts (`emv.jar`, `certora_jars`, CLI scripts) | `~/certora-install` |
| Python CLI deps | any venv with `~/certora-work/CertoraProver/scripts/certora_cli_requirements.txt` installed |
| JDK 21 (temurin) | `/Library/Java/JavaVirtualMachines/temurin-21.jdk` |

```bash
# Reproducible CLI environment (run once from the repository root).
python3.12 -m venv ~/certora-install/.venv
~/certora-install/.venv/bin/pip install --require-hashes \
    -r certora/requirements-cli.txt

(cd certora/common/confs && \
  JAVA_HOME=/Library/Java/JavaVirtualMachines/temurin-21.jdk/Contents/Home \
  PATH="$HOME/certora-install:$PATH" \
  ~/certora-install/.venv/bin/python ~/certora-install/certoraSorobanProver.py math.conf \
      --jar ~/certora-install/emv.jar)                   # whole conf
(cd certora/common/confs && \
  JAVA_HOME=/Library/Java/JavaVirtualMachines/temurin-21.jdk/Contents/Home \
  PATH="$HOME/certora-install:$PATH" \
  ~/certora-install/.venv/bin/python ~/certora-install/certoraSorobanProver.py math.conf \
      --jar ~/certora-install/emv.jar --rule ray_mul_identity)

# Runs each rule separately, keeps full logs, propagates failures, and prefers JDK 21.
CERTORA_PYTHON="$HOME/certora-install/.venv/bin/python" \
./certora/scripts/run-rules-local.sh certora/common/confs/math.conf

# The same local runner across every config in a profile.
CERTORA_PYTHON="$HOME/certora-install/.venv/bin/python" \
./certora/scripts/run_profile.py sanity --local
```

Run the block above from the repository root. Passing
`--jar ~/certora-install/emv.jar` forces local execution — our confs
keep `"server": "prover"` so plain `certoraSorobanProver` still submits to the
cloud unchanged. The runner writes durable text logs to
`target/certora-local-logs/`; its isolated build/report directories are
temporary so concurrent rules cannot reset each other's `.certora_internal`.
Rebuild the prover after upstream updates with
`cd ~/certora-work/CertoraProver && ./gradlew assemble`.
Local runs still need `make certora-wasm` first, same as cloud. The local
optimizer helper (`tac_optimizer`) is installed beside the Prover, so that
directory must be on `PATH`. The runner does this automatically. It also
defaults each JVM to `-Xmx8g`; override with `CERTORA_JAVA_HEAP` for a solo
heavy rule. The heap cap does not include external Z3 workers. To prevent local
solver fan-out from exhausting RAM, the runner removes `-splitParallel true`
from a temporary conf copy by default; set `CERTORA_LOCAL_SPLIT_PARALLEL=true`
only when the host has measured headroom. Do not combine local split-parallel
with `-j N`. On a 48 GiB host, use `-Xmx12g` only for one rule after closing
other memory-heavy processes; giving Java all physical RAM starves Z3.

## Hosted prover

**CI:** `.github/workflows/certora-verification.yml` derives its dispatched
reachability matrix from every focused config in the `sanity` profile. It
builds the focused WASMs once, transfers them with the manifest, and rechecks
their source fingerprints before submission. Requires the `CERTORAKEY` secret.

**Manual profiles:**

```bash
./certora/scripts/run_profile.py --list
./certora/scripts/run-all.sh sanity
./certora/scripts/run_profile.py fast
./certora/scripts/run_profile.py core
./certora/scripts/run_profile.py heavy
```

| Profile | Purpose |
| --- | --- |
| `sanity` | Reachability / non-vacuity smoke (CI) |
| `fast` | Stable subset: math, rates, integrity, light controller safety |
| `core` | Audit: summaries, solvency, liquidation, strategy |
| `heavy` | Expensive targeted configs (parallel-friendly) |

Forward extra prover flags after `--`:

```bash
./certora/scripts/run_profile.py fast -- --rule borrow_rate_capped
```

## Proof ordering

Follow lemma-before-main ordering when adding proofs:

1. `pool/confs/pool-core-sanity.conf` for explicit fixture reachability
2. `pool/confs/rate-index-accounting.conf` for pure accrual lemmas
3. Pool position, seizure/settlement, fee/strategy, and flash accounting jobs
4. `tolerance-math.conf` before full oracle-dependent liquidation

## Production boundary

Production crates expose only `#[cfg(feature = "certora")]` hooks; rule bodies,
harnesses, and summary implementations live under `certora/`.
Summary call sites use `apply_summary!` in production because CVLR must wrap
the summarized function at its definition site.

Controller jobs use trusted cross-contract summaries for tractability. The
pool-core jobs call the production accounting functions used immediately
before token transfers. They do not prove arbitrary SAC/callback behavior,
unbounded batches, or controller persistence of returned account positions. A
controller verdict that reaches a summarized pool call remains conditional on
that summary; keep pool accounting and controller summary proofs separate.

## Cloud readiness (Certora hosted prover)

All confs pass local syntax, rule-coverage, profile-coverage, compilation, and
artifact-provenance gates. Those checks are not proof verdicts. Runtime status
must come from the report for the exact artifact hash; do not infer success
from a submitted job or from an older run.

**Build requirement:** run `make certora-wasm` locally (or in CI) before
submitting jobs. Confs use the `files` field pointing at
`artifacts/wasm/certora/*.wasm`, so the hosted prover does not rebuild contracts.
You still need `stellar-cli` ≥ 25.2 on the machine that produces those WASM
artifacts (`experimental_spec_shaking_v2` in soroban-sdk 26).

Controller state rules can spend minutes in initial WASM transformation before
SMT starts. This is distinct from an SMT timeout. Run focused configs one rule
at a time locally to distinguish transformation cost, loop-unwind failure,
counterexample, and solver timeout.

**Recommended cloud usage:**

```bash
# One rule per submission for expensive state jobs
(cd certora/controller/confs && \
  certoraSorobanProver solvency-borrow.conf --rule ltv_borrow_bound_enforced)
(cd certora/controller/confs && \
  certoraSorobanProver boundary-math.conf --rule mul_at_max_i128)
```

Run the `sanity` profile before `fast`/`core`.

## Config policy

- `rule_sanity: "basic"` by default; heavy configs may use `"none"` when paired
  with a basic-sanity config for the same rule family
- `independent_satisfy: true` on all configs
- `optimistic_loop: false` everywhere; unwind failures remain visible
- `loop_iter`: `1`, `6`, or `8` for bounded pure math; `32` for Soroban
  host-state jobs. A real pool fixture needs at least 28 iterations because
  host-value/storage encoding contains fixed loops longer than ten. The static
  checker rejects undersized state configs.
- `multi_assert_check: true` for universal jobs and `false` for standalone
  witness jobs. `dontStopAtFirstSplitTimeout` is reserved for witness search.
- `precise_bitwise_ops` is escalation-only: the default LIA encoding
  overapproximates bitwise ops, which is sound for Verified verdicts and an
  order of magnitude faster (common/math: 8/8 in 6 min locally vs 4/8 with
  bit-blasting). Enable it per-rule only when a counterexample is
  bitwise-spurious. Boundary confs that assert exact overflow behavior may
  still need it — validate locally before removing. Dedicated escalation
  confs: `common/confs/math-hard.conf` (NIA-hard bps→wad floor chain) and
  `controller/confs/math-bv.conf` (bit-precise sign/rounding semantics);
  both run in the `heavy` profile
- EVM-only options (`solc`, `solc_via_ir`, hashing bounds,
  `havocAllByDefault`) are not used. `multi_assert_check` is supported by the
  Soroban Prover and is intentionally enabled for universal jobs.
- `-maxCommandCount` must exceed the rule's expanded command count, or the job
  errors (`expanded to too many commands: N > limit`). Controller state confs
  set `2000000`; raise it (not lower it) when a sanity rule trips the cap.

### Difficulty timeouts (hard stop at `global_timeout`)

Confs whose rules run the full position/strategy/solvency paths (high path
count, kinked-rate nonlinearity, multi-loop portfolios) can hit the
`global_timeout` hard stop rather than the SMT `smt_timeout`. Provisioning
policy across all confs:

- **`-maxCommandCount`** is set on every state/oracle conf (≥ `2000000`); the
  prover default (`1000000`) is below what a single position-mutation sanity
  rule expands to. Pure fixed-point math confs (`common/math`, `controller/math`,
  `tolerance-math`) stay lower — they never approach the cap.
- **`-splitParallel true`** is used selectively on control-flow-heavy LIA/NIA
  jobs. It is omitted from `math-bv.conf` because the parallel splitter does
  not support bit-vector theory.
- **Destructive optimization** is reserved for the nine heavy configs. Routine
  configs keep smaller depth/command limits so easy rules do not inherit
  heavy-job cost.
- Heavy jobs use the hosted maximum `global_timeout` of 7200 seconds and an
  1800-second per-query SMT timeout. Ten-seed solver portfolios are not the
  default: add one only after a reproducible solver-instability result, because
  every seed increases solver and credit load.

The escape hatch — the same lever the Certora/Blend pool confs use on their
hardest status rules:

```json
"prover_args": [
    "-maxBlockCount 500000",
    "-maxCommandCount 2000000",
    "-splitParallel true",
    "-depth 15"
]
```

`-splitParallel true` solves supported control-flow splits across workers
instead of sequentially. If a conf still hard-stops, run it one rule at a time
(`--rule <name>`) and reduce the modeled surface with a separately proved
summary rather than only raising the timeout.

**`Inconsistent ref stack sizes … FunctionIndex_294`** is the Stellar-optimizer
internal error: re-run `make certora-wasm` (it builds `--optimize=false`) and
submit the freshly-built `artifacts/wasm/certora/*.wasm`. A stale or optimized
artifact reproduces it and cascades into spurious `Violated` sanity rules.

## Learning resources

- [Sunbeam docs](https://docs.certora.com/en/latest/docs/sunbeam/index.html) and [tutorials](https://certora-sunbeam-tutorials.readthedocs-hosted.com/en/latest/)
- [Certora user guide](https://docs.certora.com/en/latest/docs/user-guide/index.html) — sanity, CI, timeout strategy (translate to Sunbeam)
- Large Certora projects for examples of solvency README patterns and lemma→main splits (see Certora user guide)
- [AIComposer](https://github.com/Certora/AIComposer) — Solidity/CVL only; use its spec-first workflow manually with `*_rules.rs`

## Targeted high-signal runs

```bash
(cd certora/pool/confs && certoraSorobanProver position-accounting.conf)
(cd certora/pool/confs && certoraSorobanProver seize-settle-accounting.conf)
(cd certora/controller/confs && certoraSorobanProver no-collateral-no-debt.conf)
(cd certora/controller/confs && certoraSorobanProver controller-pool-consistency.conf)
(cd certora/controller/confs && certoraSorobanProver global-solvency-heavy.conf)
(cd certora/controller/confs && certoraSorobanProver liquidation-integrity-heavy.conf)
```
