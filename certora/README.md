# Certora Formal Verification

Formal verification for the lending protocol's critical invariants, using
[Certora Sunbeam](https://docs.certora.com/en/latest/docs/sunbeam/index.html)
(Rust + CVLR `#[rule]` specs compiled to WASM).

## Layout

```text
certora/
├── common/          # fixed-point and rate math
├── pool/            # pool accounting and summary-contract proofs
├── controller/      # solvency, liquidation, oracle, strategy rules
│   ├── harness/
│   ├── confs/
│   └── spec/        # README.txt — domain invariant + conf map
├── shared/          # cross-contract summaries
├── scripts/         # Python entrypoints, wasm helpers, run-all.sh wrapper
├── profiles.json    # sanity | fast | core | critical | heavy | manual | all
└── compile_all.sh
```

Partitioning is by **crate/WASM boundary** (`common` / `pool` / `controller`),
not by proof theme. Domain docs live in each layer's `spec/README.txt`.
Cross-reference: [`architecture/INVARIANTS.md`](../architecture/INVARIANTS.md) (the enforceable properties these rules protect, including numeric model, pool accounting, account solvency, oracle, storage/boundaries, pause/freeze, and bad-debt socialization) and the verification surface in `SCF_BUILD_ARCHITECTURE.md §14`.

See also the central implementation facts: controller owns risk/oracle/strategies and is sole caller of the pool; governance owns controller; new deployments start paused; 3-layer pause/freeze; GUARDIAN for immediate per-listing actions; keeper self-authorizes.

## WASM artifacts (deploy + prover)

Production and Certora WASM share one tree under `artifacts/wasm/`:

| Path | Built by | Used for |
| --- | --- | --- |
| `deploy/pool.wasm`, `deploy/controller.wasm` | `make deploy-artifacts` | Mainnet deploy / upgrades |
| `certora/common.wasm`, `certora/pool.wasm`, `certora/controller.wasm` | `make certora-wasm` | All Certora conf `files` entries |

```bash
make wasm-artifacts          # both deploy + certora
make certora-wasm            # prover only (rebuild after contract/spec changes)
```

Conf files reference prebuilt `certora/*.wasm` so Certora cloud skips
`stellar contract build`. Rebuild locally, then submit jobs.

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
cd certora/common/confs
<venv>/bin/python ~/certora-install/certoraSorobanProver.py math.conf \
    --jar ~/certora-install/emv.jar                      # whole conf
<venv>/bin/python ~/certora-install/certoraSorobanProver.py math.conf \
    --jar ~/certora-install/emv.jar --rule ray_mul_identity
```

Passing `--jar ~/certora-install/emv.jar` forces local execution — our confs
keep `"server": "prover"` so plain `certoraSorobanProver` still submits to the
cloud unchanged. Reports land next to the conf in `emv-*-certora-*/Reports`.
Rebuild the prover after upstream updates with
`cd ~/certora-work/CertoraProver && ./gradlew assemble`.
Local runs still need `make certora-wasm` first, same as cloud.

## Hosted prover

**CI:** `.github/workflows/certora-verification.yml` runs a reachability matrix
when dispatched; the full `sanity` profile (17 reachability rules) is the local
equivalent. Requires `CERTORAKEY` repository secret.

**Manual profiles:**

```bash
./certora/scripts/run_profile.py --list
./certora/scripts/run-all.sh sanity
./certora/scripts/run_profile.py fast
./certora/scripts/run_profile.py core
./certora/scripts/run_profile.py critical
./certora/scripts/run_profile.py heavy
```

| Profile | Purpose |
| --- | --- |
| `sanity` | Reachability / non-vacuity smoke (CI) |
| `fast` | Stable subset: math, rates, integrity, light controller safety |
| `core` | Audit: summaries, solvency, liquidation, strategy |
| `critical` | Highest-signal accounting and safety proofs |
| `heavy` | Expensive targeted configs (parallel-friendly) |

Forward extra prover flags after `--`:

```bash
./certora/scripts/run_profile.py fast -- --rule borrow_rate_capped
```

## Lemma-before-main

Follow lemma-before-main ordering when adding proofs:

1. `pool/confs/summary-contract.conf` before controller solvency that summarizes pool calls
2. `tolerance-math.conf` before full oracle-dependent liquidation
3. Light configs (`rule_sanity: basic`) before paired `*-heavy.conf`

## Production boundary

Production crates expose only `#[cfg(feature = "certora")]` hooks; rule bodies,
harnesses, and summary implementations live under `certora/`.
Summary call sites use `apply_summary!` in production because CVLR must wrap
the summarized function at its definition site.

Controller proofs that summarize pool calls are accounting evidence only after
`summary-contract.conf` passes.

## Cloud readiness (Certora hosted prover)

Not all confs are equally reliable in Certora cloud. Config syntax is valid
(orphan/coverage checks pass), but runtime behavior splits into three tiers:

| Tier | Confs | Expectation |
| --- | --- | --- |
| **A — reliable** | `common/math`, `flash_loan`, `health`, `indexes`, `positions`, `oracle`, `tolerance-math`, `liquidation-light` | Usually complete within 30–60 min |
| **B — may timeout** | `common/rates`, `math`, `interest`, `spoke`, `liquidation`, `strategy`, `market-guard`, `controller-pool-consistency-light`, `pool/integrity`, `summary-contract`, `additivity` | Run individually; 1–2 h jobs |
| **C — heavy / often stuck** | `solvency-*` (split bundles), `boundary-*` (split bundles), the `heavy` profile confs (`math-hard`, `math-bv`, `summary-contract-critical`, `additivity-heavy`, `no-collateral-no-debt`, `controller-pool-consistency`, `global-solvency-heavy`, `liquidation-integrity-heavy`) | Use `--rule <one>` per invocation; expect multi-hour runs |

**Build requirement:** run `make certora-wasm` locally (or in CI) before
submitting jobs. Confs use the `files` field pointing at
`artifacts/wasm/certora/*.wasm`, so the hosted prover does not rebuild contracts.
You still need `stellar-cli` ≥ 25.2 on the machine that produces those WASM
artifacts (`experimental_spec_shaking_v2` in soroban-sdk 26).

**Why jobs look "stuck":** Tier C confs bundle many rules with
`global_timeout: 7200` and `rule_sanity: basic`, which multiplies SMT work.
`splitParallel` heavy configs can sit at 100% for hours before reporting. That
is timeout pressure, not a hung portal.

**Recommended cloud usage:**

```bash
# One rule per submission for Tier B/C
certoraSorobanProver solvency-borrow.conf --rule ltv_borrow_bound_enforced
certoraSorobanProver boundary-math.conf --rule mul_at_max_i128
```

Run `sanity` profile rules first (17 reachability checks) before `fast`/`core`.

## Config policy

- `rule_sanity: "basic"` by default; heavy configs may use `"none"` when paired
  with a basic-sanity config for the same rule family
- `independent_satisfy: true` on all configs
- `loop_iter`: `1` (light math), `2` (boundary), `3` (state rules)
- `precise_bitwise_ops` is escalation-only: the default LIA encoding
  overapproximates bitwise ops, which is sound for Verified verdicts and an
  order of magnitude faster (common/math: 8/8 in 6 min locally vs 4/8 with
  bit-blasting). Enable it per-rule only when a counterexample is
  bitwise-spurious. Boundary confs that assert exact overflow behavior may
  still need it — validate locally before removing. Dedicated escalation
  confs: `common/confs/math-hard.conf` (NIA-hard bps→wad floor chain) and
  `controller/confs/math-bv.conf` (bit-precise sign/rounding semantics);
  both run in the `heavy` profile
- EVM-only options (`multi_assert_check`, `solc`, Gambit) are not used
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
- **`-splitParallel true`** is on every conf with `global_timeout: 7200` (the
  heavy tier) — parallel splitting is pure upside.
- **Eager splitting** (`-smt_initialSplitDepth 5 -depth 15`) is reserved for the
  confs observed to hard-stop or run long (`health`, `market-guard`, `strategy`,
  `solvency-roundtrip`, `spoke`, `interest`, `liquidation`,
  `liquidation-light`, `health-gated`).

The escape hatch — the same lever the Certora/Blend pool confs use on their
hardest status rules:

```json
"prover_args": [
    "-maxBlockCount 500000",
    "-maxCommandCount 2000000",
    "-splitParallel true",
    "-smt_initialSplitDepth 5",
    "-depth 15"
]
```

`-splitParallel true` solves control-flow splits across workers instead of
sequentially; eager splitting (`-smt_initialSplitDepth 5 -depth 15`) carves the
large rule body into solvable sub-problems early. If a conf still hard-stops,
run it one rule at a time (`--rule <name>`) and/or summarise the nonlinear
hotspot (the kinked interest-rate model) rather than only raising the timeout.

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
certoraSorobanProver certora/pool/confs/summary-contract-critical.conf
certoraSorobanProver certora/controller/confs/no-collateral-no-debt.conf
certoraSorobanProver certora/controller/confs/controller-pool-consistency.conf
certoraSorobanProver certora/controller/confs/global-solvency-heavy.conf
certoraSorobanProver certora/controller/confs/liquidation-integrity-heavy.conf
```
