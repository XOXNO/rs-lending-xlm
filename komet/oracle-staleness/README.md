# Oracle freshness proof

This SDK 28 harness calls the production `common::oracle::observation::is_stale`
through a local Cargo dependency. The price aggregator uses that helper to decide
whether a feed has exceeded its allowed age.

The property covers every `u64` timestamp and maximum age:

```rust
is_stale(now, now, max_age) == false
```

A feed timestamped at the current time is not stale, including when its maximum
age is zero. The two symbolic variables range independently from `0` through
`2^64 - 1`: `2^128` numeric input pairs. The feed timestamp is set to `now`.

`test_fresh_wrong` deliberately asserts the inverse. It fails because the
production function returns `false`; `(now, max_age) = (0, 0)` is a concrete
counterexample.

## Recorded result

The claim was rerun after removing the unsafe inherited KWasm optimizer and
repairing symbolic word bounds and wrapping. Both runs have fresh input
fingerprints, complete graphs and recorded CLI exits. See [toolchain setup](../README.md).

The current positive proof passed in 32 seconds (prover timer; 36.56 seconds
including setup). Its saved graph executes 411 steps
from node 1 to node 3, then covers target node 2 with exit code 0. It has no
pending, failing, vacuous or bounded paths, admissions, circularity, or subproofs.
The negative proof failed after 42.53 seconds including setup at `expectResult(true)` with a returned
`false` value, after completing Wasm execution.

The compiled Wasm retains the production helper's subtraction and comparisons;
the invariant was not replaced by a constant during compilation.

Verified Wasm SHA-256:
`5c9dff7f26e852382bea8e0b2262ae551ab41f7e4ad4d165fdebe191ca1dcede`.

## Reproduce

Use [XOXNO/komet](https://github.com/XOXNO/komet/tree/codex/soroban-sdk28-host-models)
with the local repairs and locked versions described in the toolchain guide,
and freshly built K definitions. The published base commit alone lacks these repairs.
The recorded toolchain uses K/pyk `7.1.337`, KWasm `0.1.155`, Stellar CLI `28.0.0`,
and Rust `1.95.0`.

From this directory:

```sh
stellar contract build --locked --out-dir target/wasm
komet prove run --always-allocate --wasm target/wasm/oracle_staleness_proof.wasm \
  --id test_fresh_never_stale --proof-dir target/proof-fresh

# Expected to fail: the deliberately incorrect inverse property.
komet prove run --always-allocate --wasm target/wasm/oracle_staleness_proof.wasm \
  --id test_fresh_wrong --proof-dir target/proof-negative
```

Use fresh proof directories when source or Wasm changes: Komet resumes saved
proofs by property name. A successful fuzz run is not a symbolic proof.

## Scope

The proof uses Komet's standard `--always-allocate` mode: numeric arguments are
represented by Soroban host objects. It covers the complete numeric `u64` ranges
in that mode, not the alternate packed-small argument decoding path.

This is one correctness lemma for the compiled production helper under Komet's
Wasm/Soroban semantics. It does not establish rejection of stale feeds, the
complete oracle, price authenticity, authorization, or ledger-budget feasibility.
The broader three-input staleness equivalence was attempted but did not complete;
it is not counted as a proof. No claim-local extra module or admitted claim is
used. The proof trusts the fork's reviewed simplification rules as part of its
recorded semantics.
The standalone workspace leaves production contract manifests and code unchanged.
