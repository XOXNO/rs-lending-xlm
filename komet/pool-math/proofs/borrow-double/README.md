# Borrow-index doubling: verified native Komet proof

`test_borrow_double` is **PASSED** across its original domain. The false
`test_borrow_double_wrong` control is **FAILED** at its Boolean assertion.

For every signed i128 input satisfying `10^27 <= old <= 10^36`:

```rust
common::rates::update_borrow_index(&env, Ray::from(old), Ray::from(2 * RAY)).raw()
    == (2 * old).min(MAX_BORROW_INDEX_RAY)
```

`RAY = 10^27` and `MAX_BORROW_INDEX_RAY = 10^36`. The harness calls the
production `common` crate, retaining an `#[inline(never)]` call boundary.
Inputs outside that valid range return true from the original implication
guard. Both uncapped and capped outcomes, including equality at the cap,
are covered. The negative witness is `old = cap / 2 + 1`: production returns
`cap`, while the intentionally wrong assertion expects `cap + 2`.

This establishes the fixed factor `2 * RAY` case. General growth factors,
storage transitions, time accrual, controller behavior, and pool endpoints
require separate proofs. The production contracts are unchanged.

## Result and evidence

| Check | Result |
|---|---|
| Native CLI | `komet prove run`, exit 0 |
| Saved APR | `PASSED`, 47 nodes, 5,871 rewrite steps |
| Open/failing/stuck/vacuous leaves | None |
| Admissions/bounds/circularity/refutations/subproofs | None |
| Cap outcomes | Capped, uncapped, and cap equality covered |
| Negative control | Exit 1, native APR `FAILED`, actual false versus expected true |
| Final continuation | 189 seconds, with 600-second cap and 15-second grace |
| Runtime integrity | All 6,522 recorded pins checked before and after |
| Independent review | Accepted native result, full domain and both cap outcomes |
| Source-to-Wasm check | Fresh build matches the proved Wasm byte for byte |

The 189 seconds cover the last continuation, which reuses earlier execution.
They are not the total time spent constructing the proof.

[result.json](result.json) records identifiers and hashes. The
[3 MB archive](native-proof.tar.gz) retains every positive and negative native
proof file, including `inputs.json`, the proved Wasm, original commands/results,
runtime manifests, independent reviews, and the exact selected Komet/pyk source
snapshots with licenses. The archive's own per-file manifest binds those bytes.
Original machine paths remain in historical metadata; they are provenance
records, not portable paths to execute.

[double-cli.k](double-cli.k) is the exact 17-rule extra module from the run.
The [Lean sources](lean/) check its arithmetic/word-normalization helpers and
strict partial-value reasoning. The readable `LowWord.lean` copy drops one
trailing blank line; its original bytes are retained inside the archive and
both hashes are recorded in `result.json`. Lean checks were compiled with `--trust=0`
using 4.22.0. They do not independently establish a refinement of K/Wasm hooks.

## Check the archived result

Use Python 3.10 or later with the dependencies of `kframework==7.1.337`
installed in the configured Komet environment:

```sh
cd komet/pool-math/proofs/borrow-double
shasum -a 256 -c SHA256SUMS
python3 verify.py
```

The checker validates archive hashes, loads both graphs through the exact
archived native pyk APR implementation, rejects shortcuts, and inspects the
negative leaf's actual/expected Boolean values. It makes no execution RPC.
Komet has no `prove verify` command; this is artifact validation, not independent
certificate replay. The result depends on the recorded Komet/KWasm/Kore/Booster
semantics, native binaries and reviewed simplification rules.

To recheck all Lean sources with Lean 4.22.0:

```sh
set -e
task_lean_out=$(mktemp -d)
for task_lemma in BorrowIndex FloorWordComparison FloorWordPartial SignedWordNegative \
  Fixed64Packing SignedDecoder DoubleRay FloorEquality LowWord \
  IndependentSignedModulo64 IndependentFixed64XorV2 CarryWord DoubleWord; do
  LEAN_PATH="$task_lean_out" lean --trust=0 -o "$task_lean_out/$task_lemma.olean" "lean/$task_lemma.lean"
done
```

## Rebuild or rerun

From the repository root, the selected harness build is:

```sh
stellar contract build --locked --manifest-path komet/pool-math/Cargo.toml \
  --no-default-features --features borrow-double --out-dir /tmp/borrow-double-wasm
```

Expected Wasm SHA-256:

```text
099e7e12701d31576386154e93ffe1c1b3afe9aa981e62a59f291bf442820073
```

A fresh symbolic run uses the configured fork and compiled semantics from the
recorded runtime, rather than assuming an arbitrary Komet installation matches:

```sh
komet prove run --always-allocate \
  --wasm /tmp/borrow-double-wasm/pool_math_proof.wasm \
  --id test_borrow_double \
  --extra-module komet/pool-math/proofs/borrow-double/double-cli.k:SIGNED-DISTRIBUTE-COMPLEMENT-NEGATIVE-PACK64-DOUBLE-RAY-SIGNED-DECODER-FLOOR-EQUALITY-LOW-WORD-SIGNED64-SIGN-FIXED64-XOR \
  --proof-dir /tmp/borrow-double-fresh-proof
```

The original backend uses a 5,000 ms initial SMT timeout, retry limit 3,
`--equation-max-local-steps 32`, `--no-fallback-simplify`, and
`--no-post-exec-simplify`. The archive records the exact wrapper and hashes.
A fresh run at a relocated module path must start a fresh graph: source metadata
participates in the native input fingerprint. Do not rewrite `inputs.json` or
disable its checks to reuse the archived graph on a different runtime.
