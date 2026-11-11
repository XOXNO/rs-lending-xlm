# Trusted base of the Komet proofs

A Komet proof here states that the compiled harness Wasm, executed by the
K/KWasm/Komet semantics from the given symbolic input domain, always returns
`true`. This file lists everything that statement relies on, and how each
custom simplification rule is justified. Pins are in [`pins.env`](pins.env).

## 1. Tools (trusted, not re-verified)

| Component | Version / pin | Role |
|---|---|---|
| K frontend (`kompile`, `kprove` parsing) | 7.1.337 (K image digest) | Compiles the semantics and parses the extra module |
| Kore/Booster backend | `XOXNO/haskell-backend` `818a5e4c` (branch `proof/booster-v6-compact-rpc`, base `v0.1.155` `38afc81f`) | Symbolic execution, simplification, implication checks |
| LLVM backend | 7.1.337 | Concrete deployment step and Booster's simplification library |
| Z3 | 4.13.4 official release | Decides path conditions and side conditions |
| pyk | `kframework` 7.1.337 with the two files of Komet's `vendor/pyk-overlay` (SHA-256 `fdd855b4…`, `057fcc12…`) | Proof driver (APR prover, KCFG) |
| Komet runner | `XOXNO/komet` `7e74d11` `src/komet/proof.py` (SHA-256 `ac43e6ca…`) | Builds the claim; prunes a branch only on an explicit `UNSAT` |
| KWasm semantics | `pykwasm` 0.1.155 (from Komet's `uv.lock`) | Wasm execution model; the optional `KWASM-LEMMAS` module is excluded by the fork |
| Komet host model | `XOXNO/komet` `7e74d11` (SDK 28 host functions, see its `docs/sdk28-support.md`) | Soroban host functions (`I256`, `i128` objects, …) |

Not trusted for the Wasm-level claim: `rustc` and the Stellar CLI. They link
the claim to Rust source only through the reproducible build: the pipeline
rebuilds each harness and compares its SHA-256 with the proved Wasm.

### Backend changes over `v0.1.155`

`818a5e4c` is the source snapshot of the original proof's Booster
(`hs-backend-booster-0.1.155-v6-compact-rpc`). Its `booster/`, `kore/` and
`kore-rpc-types/` trees are identical to `130d6edb` (branch
`proof/borrow-double-repro-2026-11`); the two differ only in README and Nix
recipe files. Reviewed for soundness; each change only refuses or exactly
evaluates.

* `ApplyEquations.hs`: after a simplification rule matches, if it would erase a
  matched operand that is not syntactically defined (a variable, a domain
  value, a total symbol, or `modInt`/`/Int` by a nonzero literal over defined
  operands), the rule is not applied and any facts recorded while checking its
  `requires` are rolled back. This closes the unsound case of an erasing rule
  (e.g. `I -Int I => 0`) applied to an undefined operand. Stock Booster lacks it.
* `Match.hs`: a substituted map-key pattern `c1 -Int c2` with two concrete
  integers is evaluated by the existing `INT.sub` builtin before map matching.
* `Translate.hs` (kore): the reverse SMT symbol map is built lazily; decoding an
  ambiguous name still fails instead of picking a symbol.
* `Server.hs`: compact single-line JSON-RPC responses (framing only).

Decision on the stock K 7.1.337 Booster: not used for accepted results, since
it lacks the erased-operand guard that the `integer-self-subtraction` rule
needs. Comparison runs are recorded in the proof READMEs when available.

### pyk overlay

The semantics image replaces two files of the installed `kframework` 7.1.337
package with Komet's `vendor/pyk-overlay` copies, checking the stock hashes
(`ddc9aa9a…`, `dfc1f86e…`) before and the selected hashes after. Changes: emit
`preserves-definedness` to Kore (stock pyk rejects the attribute in an extra
module); do not reject an empty-substitution match in the fast subsumption
pre-check; treat the SMT `retry limit is 0` error during a cover check as no
cover (other errors still propagate). None of these can close an obligation.

## 2. Modelling assumptions used by the Lean proofs

The Lean files prove each rule over a model of K's builtins. These mappings
are the assumptions that connect them to K:

| K | Lean model |
|---|---|
| `Int`, `+Int`, `-Int`, `*Int`, comparisons | `Int` |
| `A /Int B` (truncating), `A %Int B` | `Int.tdiv`, `Int.tmod` |
| `A divInt B`, `A modInt B` (Euclidean) | `A / B`, `A % B` (`Int.ediv`, `Int.emod`) |
| `&Int`, `|Int`, `xorInt` (GMP, infinite two's complement) | `KBits.land`, `KBits.lor`, `KBits.xor`, characterised bit by bit |
| `X <<Int N`, `X >>Int N`, `N >= 0` | `X * 2^N`, `X >>> N` (floor) |
| `#u64Or` / `#u64Xor` SMT hook `bv2int(bvor/bvxor(int2bv64 x, int2bv64 y))` | OR/XOR of the residues mod 2^64 (proved equal to the K rule) |
| `SparseBytes`, `#getRange`, `#setRange` | byte function `Nat → Nat`, unmapped bytes zero, little-endian (`HostObjectLemmas`) |
| `#minWidth` on `N >= 0` | `0` or `log2 N + 1`; proved to satisfy KWasm's defining equations |

Lean 4.22.0, every file checked with `--trust=0`; the checker rejects `sorry`,
`admit`, `axiom`, `native_decide`, `implemented_by`, `extern`, and any
`#print axioms` report beyond `propext`, `Classical.choice`, `Quot.sound`.

## 3. Custom simplification rules and their justification

Every rule below is a `[simplification]` rule outside upstream KWasm/Komet.
All are proved in Lean; the column gives file and theorem.

### Komet fork `komet-lemmas.md` (commit `7e74d11`, SHA-256 `2ddebe51…`)

Files under [`proofs/komet-lemmas/lean/`](proofs/komet-lemmas/lean/).

| Rule | Lean theorem |
|---|---|
| `C \|Int S => S \|Int C` | `IntBitwiseLemmas.or_comm` |
| `X \|Int 0 => X` | `IntBitwiseLemmas.or_zero` |
| `xor-zero-right`, `xor-zero-left` | `IntBitwiseLemmas.xor_zero_right`, `xor_zero_left` |
| `_X &Int 0 => 0` (erases X; default definedness kept) | `IntBitwiseLemmas.and_zero` |
| `masked-int-nonnegative` | `IntBitwiseLemmas.masked_int_nonnegative` |
| `masked-int-upper-bound` | `IntBitwiseLemmas.masked_int_upper_bound` |
| `&Int` commutativity and both associativity rules | `IntBitwiseLemmas.and_comm`, `and_assoc` |
| `isPowerOf2`, `isFullMask`, `fullMask` side-condition definitions | `IntBitwiseLemmas.isPowerOf2_iff`, `isFullMask_iff` |
| `full-mask-to-modulus` | `IntBitwiseLemmas.full_mask_to_modulus` |
| `masked-or-zero-right`, `masked-or-zero-left` | `IntBitwiseLemmas.masked_or_zero_right`, `masked_or_zero_left` |
| `masked-xor-sign-bit` | `IntBitwiseLemmas.masked_xor_sign_bit` |
| `masked-complement-and-right`, `-left` | `IntBitwiseLemmas.masked_complement_and_right`, `masked_complement_and_left` |
| `constant-right-shift` | `IntBitwiseLemmas.constant_right_shift` |
| `pack-bounded-low-word` | `IntBitwiseLemmas.pack_bounded_low_word` |
| `pack-four-u64-limbs` | `IntBitwiseLemmas.pack_four_u64_limbs` |
| `project-u64-or`, `project-u64-xor`, `u64-or-native`, `u64-xor-native` | `U64Lemmas.project_u64_or`, `project_u64_xor` (definitional) |
| `#u64Or` / `#u64Xor` SMT hooks | `U64Lemmas.smt_hook_or`, `smt_hook_xor` |
| `u64-or-zero-right`, `u64-or-zero-left` | `U64Lemmas.u64_or_zero_right`, `u64_or_zero_left` |
| `u64-xor-sign-bit`, `u64-xor-sign-bit-complement` | `U64Lemmas.u64_xor_sign_bit`, `u64_xor_sign_bit_complement` |
| `u64-xor-nonnegative`, `u64-xor-upper-bound` | `U64Lemmas.u64_xor_nonnegative`, `u64_xor_upper_bound` |
| `project-u64-shift32` | `U64Lemmas.project_u64_shift32` |
| `u64-pack-bounded-low32` | `U64Lemmas.u64_pack_bounded_low32` |
| `reverse-min-width-comparison` | `HostObjectLemmas.reverse_min_width_comparison` |
| `bounded-min-width-comparison-nonpositive`, `-above`, `-threshold` | `HostObjectLemmas.bounded_min_width_nonpositive`, `bounded_min_width_above`, `bounded_min_width_threshold` |
| `boolean-word-value` | `HostObjectLemmas.boolean_word_value` |
| `bitwise-mk-hostval-then-mask` | `HostObjectLemmas.bitwise_mk_hostval_then_mask` |
| `#getRange(SB, ADDR, 8) &Int (2^64-1) => #getRange(SB, ADDR, 8)` | `HostObjectLemmas.getRange8_mask` |
| `wrap-in-range` | `HostObjectLemmas.wrap_in_range` |
| `signed-total-in-range`, `signed-in-range` | `HostObjectLemmas.signed_in_range` (with KWasm's `#signedTotal`/`#signed` rules) |
| `unsigned-in-range` | `HostObjectLemmas.unsigned_in_range` |
| `#getRange(#setRange(…, ADDR, VAL, WIDTH), ADDR, WIDTH') => #wrap(WIDTH', VAL)` | `HostObjectLemmas.getRange_setRange_same` |
| `getRange-before-setRange`, `getRange-after-setRange` | `HostObjectLemmas.getRange_before_setRange`, `_after_setRange` |

`runLemma`/`doneLemma` (KSOROBAN-LEMMAS) is a test-harness rewrite, not a
simplification.

### Proof-specific extra modules

| Module | Rules | Lean |
|---|---|---|
| [`double-cli.k`](../pool-math/proofs/borrow-double/double-cli.k) (borrow-double) | 17 | [`DoubleCliRules.lean`](../pool-math/proofs/borrow-double/lean/DoubleCliRules.lean): one theorem per rule, stated in the rule's shape, from the 13 original lemma files |
| [`arithmetic.k`](../pool-math/proofs/utilization/arithmetic.k) (utilization) | 39 = the 17 above (byte-identical) + 22 `utilization-*` | [`UtilizationRules.lean`](../pool-math/proofs/utilization/lean/UtilizationRules.lean): one theorem per `utilization-*` rule, from the 11 utilization lemma files |
| [`pool-math-lemmas.k`](proofs/pool-math-lemmas.k) (every other pool-math proof run by this pipeline) | 52 = the 39 of `arithmetic.k` (byte-identical) + `mul-div-cancel`, `u64-xor-zero-right/left`, `u64-or-sign-bit`, `ray-identity-half-up`, `bool-word-and/or/xor/mod`, `u64-xor-and-sign(-ge)`, `i256-shr1-limbs`, `i256-quotient-object-tag` | the files above + [`PoolMathRules.lean`](proofs/komet-lemmas/lean/PoolMathRules.lean); `i256-quotient-object-tag` is an instance of a Komet semantics rule (see Definedness) |

Definedness: a rule marked `preserves-definedness` keeps every variable of its
left side on its right side, and every division or modulus in it has a literal
nonzero divisor; so its right side is defined whenever its left side is. The
only erasing rules are `integer-self-subtraction` (`I -Int I => 0`),
`_X &Int 0 => 0` and the U64 range facts (`u64-xor-nonnegative`,
`u64-xor-upper-bound`); none carries `preserves-definedness`, and the backend
applies them only when the erased operand is syntactically defined.
`i256-quotient-object-tag` (`getTagWithFlag(true, I256(N /Int D)) => 71`
requires `D =/=Int 0`) is the Komet `data.md` rule
`getTagWithFlag(true, I256(_)) => 71` restricted to a quotient operand. It
erases `N` and `D`, which must each be syntactically defined, and its side
condition makes the quotient itself defined; it carries no
`preserves-definedness`.

## 4. What a passing proof does not cover

* The host model is a model: authorization, events, resource budgets and the
  packed-small `i128` argument encoding (`--always-allocate` is used) are
  outside it. See the Komet fork's `docs/sdk28-support.md`.
* No proof certificate is produced; the result is the backend's verdict on the
  saved graph, re-checked structurally by `scripts/check-proof.py`.
* Each claim covers only its stated input domain; inputs outside it return
  `true` from the harness guard.
