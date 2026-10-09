Verdict: accept the literal-32 arithmetic simplification, with no operand range guard:

```k
rule X:Int <<Int 32 => X *Int 4294967296
  [simplification, preserves-definedness]
```

Lean 4.22.0, compiler commit ba2cbbf09d4978f416e0ebd1fceeebc2c4138c05,
checked all three local modules with `--trust=0`; every command exited 0.
`Left32.lean` reuses `Int.shiftLeft_eq_mul` from an unchanged copy of
`HalfWord.lean`. The theorem has no sign, magnitude, utilization, or path
assumptions. `factor32` proves the exact constant. The strict Option model
proves equal results for every `Option Int`, plus both directions of
preserved definedness and absence. `none` is preserved; every `some x`
returns `some (x * 4294967296)` on both sides.

Axiom audit: `factor32` uses none. Signed identity and the three strict
claims use only standard Lean `propext`. No `sorryAx`, custom axioms,
`native_decide`, or external solver results occur in these claims.

Pinned source correspondence:

- `/nix/store/i3sfsz92jp90nylrzzb1mqsif8gl6iiy-source/runtime/arithmetic/int.cpp:115-126`:
  `hook_INT_shl` accepts `mpz_fits_ulong_p(32)`, converts that exact count
  with `mpz_get_ui`, then calls `mpz_mul_2exp(result, a, 32)`. GMP multiplies
  the entire signed arbitrary-precision operand by `2^32`; no operand
  bound or sign test occurs. The same file at `68-73` maps multiplication
  to signed arbitrary-precision `mpz_mul`.
- `/nix/store/44j0m84525anm69vksijaih7lb2w371m-kore-source/src/Kore/Builtin/Int.hs:269`:
  `mulKey` uses Integer multiplication. At `281`, `shlKey` uses
  `shift a . fromInteger`: only the count is converted to machine `Int`.
  Positive literal `32` converts exactly, so Integer left shift is signed
  multiplication by `2^32`. Operand `a` remains arbitrary precision.
  At `295-300`, both operators use the same binary-operator evaluator.
- `/nix/store/44j0m84525anm69vksijaih7lb2w371m-kore-source/src/Kore/Builtin/Builtin.hs:221-235`:
  concrete evaluation needs values for both arguments. Neither operation
  bypasses the first operand; their second operands are defined literals.

Limits: this is a kernel-checked arithmetic and strict-denotation model,
with source review of its correspondence to the pinned hooks. It is not
an end-to-end refinement proof of Haskell/GMP, compiled K semantics,
K rule matching, or solver integration. The Haskell evaluator's `empty`
branch means NotApplicable for non-domain terms; it must not be identified
with the model's denotational `none`. Literal count 32 is essential to this
review: no general signed or unbounded count lemma is approved. Runtime
resource exhaustion is outside mathematical definedness. No K prover ran.

Recheck: `python3 check.py` from any directory. Fresh imported oleans are
built from the two copied sources, entirely within this review directory.
The source hashes recorded in `source-sha256.json` matched again after the
check. Existing modules, worktree files, and graphs were not changed.
