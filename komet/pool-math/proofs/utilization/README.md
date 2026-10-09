# Exact utilization: proof in progress

The integer formula and its bounds are checked by Lean 4.22.0 with `--trust=0`.
The native Komet proof of the compiled production helper is **not complete**.
The first 17 simplification rules reuse the frozen borrow-double proof; its
original mathematics and runtime snapshots remain unchanged.
A timeout, interruption or open proof graph is not an established Wasm proof.

For nonnegative signed-i128 borrowed value `B` and supplied value `S`:

```text
RAY = 10^27
S = 0: U = 0, for every nonnegative B
S > 0 and B <= S: U = floor((B*RAY + floor(S/2))/S)
```

`test_utilization_exact` calls the unchanged production
`common::rates::utilization -> Ray::div -> fp_core::mul_div_half_up` path.
Its independent I256 specification checks `0 <= U <= RAY` and
`U*S <= N < (U+1)*S`, where `N = B*RAY + floor(S/2)`.
Inputs outside the stated domain return true from the implication guard.
This does not establish rejection behavior, utilization above 100%, pool state
transitions, or the alternate packed-small ABI representation.

The false `test_utilization_round_wrong` control expects zero at `B=1`,
`S=2*RAY`; production returns one. Both recovered 38-rule and current 39-rule
native runs rejected it at an actual false Boolean assertion, with no pending
or admitted nodes. The latest 39-rule control finished in 398.49 seconds
with 80 nodes and 1,926 rewrites. Independent native graph inspection confirms
completed Wasm execution returned `Bool(false)` against expected `Bool(true)`.

The backend now shares repeated SMT expressions through dependency-ordered
`let` bindings and memoizes successful translations within one fixed-context
translation call. A positive physical-identity check in `TermLike` equality
retains structural comparison for distinct nodes. Its equivalence assumes
finite, total internal terms with lawful payload equality; it does not change
K arithmetic rules. All 228 existing term, matcher and translation checks and
13 Z3 equivalence checks pass. A 64-layer shared-expression comparison takes
0.000159 seconds; translation takes 0.053882 seconds and emits 7,612 bytes.
These are backend regression results, not a financial theorem.

The combined runtime uses fresh positive and negative graphs. Backend patch,
source/build pins, checksums, commands, run caps and independent reviews are
retained under `utilization-20261007/smt-equality/` in the persistent artifact
workspace. Its patch passes `git apply --check` against the saved XOXNO Kore
fork. No Runtime fork source or published proof has been changed.

The exact positive proof remains open. The current module includes literal-32
mask and shift repairs, guarded symbolic AND bounds, and a projected complement
identity. Each addition has kernel-checked signed and strict partial-input
proofs. The complement keeps its operand as `X-X`; erasing a partial operand
would be unsound.

[model-review.md](model-review.md) is a historical applicability diagnosis;
its temporary graphs no longer exist. [left32-review.md](left32-review.md) and
[carry-review.md](carry-review.md) review the two latest arithmetic additions.
New native graphs and command logs are retained in persistent working storage.
Final native evidence will be packaged only after every obligation closes and
the false control completes on identical final inputs.

Check the mathematical lemmas with:

```sh
./check-math.sh /path/to/lean-4.22.0/bin/lean
```

Rebuild from the repository root:

```sh
stellar contract build --locked --manifest-path komet/pool-math/Cargo.toml \
  --no-default-features --features utilization --out-dir /tmp/utilization-wasm
```

A fresh build matches the saved Wasm byte for byte. Its SHA-256 is
`efd9339221e6e71e51b2d8ee209d1845dd2ed2947d619ce18ff752bdff49ff83`.

Use the pinned fork/runtime and start a fresh graph when source, Wasm, extra
module, runner or compiled semantics changes. Native runs use a 600-second
limit and 15-second interruption grace; actual wall times are recorded.
Unchanged inputs resume the existing graph. Lean helper checks establish
integer identities and partial-input models; they do not independently prove
K/Wasm refinement or validate a saved APR certificate.
