# Certora Formal Verification

Formal verification is the strongest form of testing: instead of trying a handful
of example inputs, a prover mathematically checks that a property holds for
*every* possible input — or returns a concrete counterexample that breaks it.
Certora is the tool that does this for the lending protocol's most critical rules.

This directory contains the Certora Soroban verification surface for the
lending protocol. It is organized by proof boundary: fixed-point libraries,
pool accounting, controller safety, and shared summaries.

The suite separates low-level math, explicit summaries, fast and heavy
profiles, and targeted configs for expensive properties instead of raising
timeouts across the whole suite.

## Layout

```text
verification/certora/
├── common/          # RAY, WAD, BPS, rate, and index math rules
├── pool/            # pool accounting, summary-contract, and additivity rules
├── controller/      # account, risk, oracle, liquidation, and strategy rules
│   ├── harness/     # certora-only wrapper macros and storage adapters
│   ├── confs/
│   └── spec/
├── shared/          # reusable pool/SAC/oracle summaries and model helpers
├── profiles.json    # canonical profile manifest
├── run_profile.py   # profile runner
└── check_orphans.py # conf/spec/profile coverage check
```

Each layer (`common`, `pool`, `controller`) carries its own `certora_build.py`,
the per-crate build script the prover invokes via each conf's `build_script`.

Each `confs/*.conf` file is a prover entry point for one bounded proof domain.
Each `spec/*_rules.rs` file contains the corresponding Rust/Soroban rules
compiled only with the `certora` Cargo feature.

## Production Boundary

The production crates keep only the minimum Rust hooks needed for Soroban
Certora builds:

- `common/src/lib.rs`, `contracts/pool/src/lib.rs`, and `contracts/controller/src/lib.rs` include
  external `verification/certora/**/spec` modules behind `#[cfg(feature =
  "certora")]`.
- The controller summary macro implementation lives in
  `verification/certora/controller/harness/summarized.rs`.
- The controller storage compatibility adapter lives in
  `verification/certora/controller/harness/storage.rs` (augmentation pattern).
- The common WASM harness lives in `verification/certora/common/spec/harness.rs`.
- See controller/harness/ for the full set (helpers.rs is the most invasive
  full-module replacement; oracle_* and cross_contract/ + views/ are narrower
  cost-isolation replacements; oracle_reflector.rs is legacy/dead post-refactor).

No rule bodies, CVLR imports, harness structs, or storage adapters are kept
inside the production source tree. Summary call sites remain in production
functions because CVLR/Soroban summaries must wrap the Rust function being
summarized; the resolver and summary bodies stay in the verification tree.

Controller harness philosophy (2026): follow the providers/*/client.rs +
shared/summaries + apply_summary! pattern wherever possible (see oracle
reflector client and cross-contract thin wrappers). Full #[path] module
replacement is reserved for expensive leaf subsystems (price resolution,
tolerance math, aggregates) or unavoidable shared buckets (helpers). The
latter remains a maintainability hotspot; future work should carve thin
wrappers for the heavy fns (calculate_account_totals) so the replacement
surface shrinks.

## Proof Domains

- `common`: fixed-point arithmetic, unit conversion, utilization, rates,
  compounding, and index movement.
- `pool`: supply, withdraw, borrow, repay, liquidation seizure, revenue,
  flash-loan accounting, summary-contract proofs, and additivity/no-profit
  properties.
- `controller`: position accounting, health factor gates, oracle freshness,
  e-mode, isolation mode, paused/status behavior, liquidation, strategies, and
  controller-pool consistency.
- `shared`: summaries for external calls and expensive protocol boundaries.

Controller proofs may use summaries for tractability. Critical pool summaries
must be backed by `pool/confs/summary-contract.conf` or the targeted
`pool/confs/summary-contract-critical.conf` before controller proof results are
treated as accounting evidence.

## Config Policy

Committed configs use Soroban-supported prover settings only.

- `msg` is set on every config so hosted runs are identifiable.
- `rule_sanity: "basic"` is the default for non-vacuity coverage.
- Heavy targeted configs may use `rule_sanity: "none"` only when the same rule
  family is also covered by a paired basic-sanity config.
- `independent_satisfy: true` is set so reachability checks are evaluated
  independently instead of being masked by another satisfy statement.
- `optimistic_loop: true` is kept for bounded symbolic execution.
- `loop_iter` is deliberate: `1` for light math/additivity, `2` for boundary
  math, and `3` for normal state rules.
- `precise_bitwise_ops: true` is used only for math and boundary configs.
- `smt_timeout` and `global_timeout` are profile-sized instead of globally
  inflated.
- `server`, `build_script`, and `cargo_features` are set in every config.

EVM-specific Aave/Solidity options are not copied into this Soroban suite.

## Profiles

Profiles are defined once in `profiles.json` and executed through
`run_profile.py`.

```bash
./verification/certora/run_profile.py --list
./verification/certora/run_profile.py sanity
./verification/certora/run_profile.py fast
./verification/certora/run_profile.py core
./verification/certora/run_profile.py heavy
```

Profile intent:

| Profile | Purpose |
| --- | --- |
| `sanity` | Targeted reachability and non-vacuity smoke checks. |
| `fast` | Stable CI profile: common math/rates, pool integrity, controller light safety. |
| `core` | Manual audit profile: pool summaries, solvency, liquidation, isolation, strategy, boundary, account isolation, and market guards. |
| `critical` | Small set of the highest-signal accounting and safety proofs. |
| `heavy` | Split-parallel targeted configs for expensive properties. |
| `manual` | `core` plus `heavy`. |
| `all` | `fast` plus `core` plus `heavy`. |

Forward extra prover flags after `--`:

```bash
./verification/certora/run_profile.py fast -- --rule borrow_respects_reserves
```

Preview commands without dispatching:

```bash
./verification/certora/run_profile.py heavy --dry-run
```

## Targeted Runs

Full assurance sequence (local compile checks, then hosted profiles):

```bash
cargo check -p common --features certora
cargo check -p pool --features certora --no-default-features
cargo check -p controller --features certora --no-default-features
python3 verification/certora/check_orphans.py
./verification/certora/run_profile.py sanity
./verification/certora/run_profile.py fast
./verification/certora/run_profile.py critical
./verification/certora/run_profile.py heavy
```

Single high-signal runs:

```bash
certoraSorobanProver verification/certora/pool/confs/summary-contract-critical.conf
certoraSorobanProver verification/certora/controller/confs/no-collateral-no-debt.conf
certoraSorobanProver verification/certora/controller/confs/controller-pool-consistency.conf
certoraSorobanProver verification/certora/controller/confs/global-solvency-heavy.conf
certoraSorobanProver verification/certora/controller/confs/liquidation-integrity-heavy.conf
```

Use the paired basic configs first when investigating vacuity:

```bash
certoraSorobanProver verification/certora/pool/confs/summary-contract.conf
certoraSorobanProver verification/certora/controller/confs/account-isolation.conf
certoraSorobanProver verification/certora/controller/confs/market-guard.conf
certoraSorobanProver verification/certora/controller/confs/controller-pool-consistency-light.conf
certoraSorobanProver verification/certora/controller/confs/solvency.conf
certoraSorobanProver verification/certora/controller/confs/liquidation.conf
```

## Local Checks

Compile all Certora feature paths and verify config/profile-to-rule coverage:

```bash
cargo check -p common --features certora
cargo check -p pool --features certora --no-default-features
cargo check -p controller --features certora --no-default-features
python3 verification/certora/check_orphans.py
```

`check_orphans.py` confirms every conf and profile rule resolves to a `#[rule]`
function in the matching `spec/` tree, and prints the current inventory:

```text
OK: 28 confs, 217 source rules, 7 profiles, zero orphans
```
