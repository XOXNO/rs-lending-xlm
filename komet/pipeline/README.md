# Komet proof pipeline (server and CI)

This directory rebuilds the Komet proofs of the [pool-math harness](../pool-math/README.md)
from pinned sources on a Linux x86-64 host with Docker, and checks the result
against a fixed evidence standard. It is what
[`.github/workflows/komet-proofs.yml`](../../.github/workflows/komet-proofs.yml)
runs on the self-hosted runner (manual dispatch only).

Nothing here changes production code. Harness crates under `komet/` call the
production `common` crate; the proofs are about the compiled harness Wasm,
identified by its SHA-256.

## What is pinned

[`pins.env`](pins.env) fixes every input by digest, commit or SHA-256:

| Input | Pin |
|---|---|
| K framework 7.1.337 | `runtimeverificationinc/kframework-k:ubuntu-jammy-7.1.337` image digest |
| Kore/Booster backend | `XOXNO/haskell-backend` commit, built with Stack 2.15.7 (GHC from Stack's verified setup-info, Hackage pinned by `stack.yaml.lock`) |
| Backend flags | `--equation-max-local-steps 32 --no-fallback-simplify --no-post-exec-simplify --smt-retry-limit 3` |
| Z3 | 4.13.4 official `x64-glibc-2.35` release, zip and binary SHA-256 |
| Komet (semantics, runner, pyk overlay) | `XOXNO/komet` commit and SHA-256 of `komet-lemmas.md` and `proof.py`; Python deps from its `uv.lock`; the two `vendor/pyk-overlay` files are hash-checked when applied |
| uv, Stellar CLI | release tarball SHA-256 (Stellar CLI pin matches `.github/scripts/install-stellar-cli.sh`) |
| Harness builder | `rust:1.95.0-slim-bookworm` image digest, `wasm32v1-none`, Stellar CLI 28.0.0 |
| Lean | 4.22.0 release tarball and `bin/lean` SHA-256 |

Image tags are hashes of the pins and Dockerfiles that define them, so a
changed pin produces a new image rather than silently reusing an old one.

## Layout

| Path | Purpose |
|---|---|
| `docker/backend.Dockerfile` | Builds `kore-rpc-booster` from the pinned backend commit |
| `docker/semantics.Dockerfile` | Komet fork, locked Python env, compiled `haskell`, `llvm`, `llvm-library` semantics (`komet-kdist build -j1`) |
| `docker/prover.Dockerfile` | Semantics + backend + Z3; records `/opt/toolchain.txt` and `/opt/semantics.sha256` |
| `docker/wasm.Dockerfile` | Harness Wasm builder |
| `scripts/build-images.sh` | Builds the images (`all`, `prover`, `backend`, `semantics`, `wasm`, `print-tags`) |
| `scripts/build-wasm.sh` | Builds one harness feature, prints the Wasm SHA-256 |
| `scripts/prove.sh` | Runs one `komet prove run` in a fresh directory, then `check-proof.py` |
| `scripts/check-proof.py` | Applies the evidence standard below to a saved proof graph |
| `scripts/check-lean.sh` | Checks every `komet/**/proofs/*/lean` directory with Lean 4.22.0 `--trust=0` |
| `scripts/run-proofs.sh` | Everything: images, Wasm hashes, proofs, controls, Lean, evidence bundle |
| `proofs.json` | Proof manifest: crate, feature, expected Wasm SHA-256, extra module, tests and expectations |
| `proofs/komet-lemmas/lean/` | Lean proofs of the Komet fork's own simplification rules |
| [`TRUSTED-BASE.md`](TRUSTED-BASE.md) | Everything a proof result depends on, and how each rule is justified |

## Evidence standard

`check-proof.py` accepts a positive proof only with native status `PASSED`
and no pending, failing, stuck, vacuous or bounded node, no admission, no
circularity, no refutation and no subproof; `komet` must exit 0. A false
control is accepted only with status `FAILED`, no pending node, `komet` exit 1,
and every failing leaf a completed Wasm call (empty instruction stack) whose
returned `SCVal:Bool(false)` meets `expectResult(SCVal:Bool(true))`. A crash,
trap, timeout or stuck host call is not an accepted negative control.

Each proof starts from a fresh directory; `inputs.json` inside it fingerprints
the claim, extra module, runner and compiled definitions.

## Run

```sh
komet/pipeline/scripts/build-images.sh all        # first build: several hours
komet/pipeline/scripts/run-proofs.sh /tmp/komet-evidence             # every proof
komet/pipeline/scripts/run-proofs.sh /tmp/komet-evidence borrow-double
```

The evidence directory receives `context.txt` (repository commit, image IDs),
`toolchain.txt`, `semantics.sha256`, `pins.env`, `proofs.json`, `lean.log`,
per-proof Wasm, build log, `wasm.sha256`, each test's proof graph, logs,
`run.json` and `check.json`, `summary.json` and a `SHA256SUMS` manifest.

Resource notes: the semantics build runs LLVM compilations one at a time
(two in parallel exceeded 30 GB RAM on a 12-core host). Positive proofs and
their controls run concurrently.
