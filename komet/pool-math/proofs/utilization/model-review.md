Verdict: model applicability gap; no demonstrated utilization counterexample. Helpers are present. Booster rejects their templates before matching. The pending carry check separately lacks symbolic AND range reasoning. Read-only diagnosis; no prover run.

Pinned evidence

- `wide-guard/byte-exact/test_utilization_exact/inputs.json` matches the supplied KDIST: Haskell `e26872853e8bdeb06963a8d85457649f088d5243aa32bf8871ec0f7ce876faa2`; LLVM `aff3d4319e8611cd4563925e28ad64f04f2696389cacf6887133b3db61f2684d`. The recorded runner hash `8423fa8a94f504588180464984914fe4a97d2453f013d0f94a25a868ea89285c` matches `runtime-batch25/toolchain/komet/proof.py`.
- The manifest embeds 33 extra rules, importing `KASMER`. Haskell `compiled.json` imports `KASMER -> KSOROBAN-LEMMAS -> INT-BITWISE-LEMMAS`. Both helpers occur as executable simplification axioms in Haskell `definition.kore`, not merely source comments. LLVM omits the symbolic module; that does not establish missing Haskell rules.
- Graph observed: 136 nodes, 2718 successful rewrites, four covers, no stuck/vacuous nodes, `admitted: false`. Nodes 130/131 are the two targets of the split at 125. The current pending frontier is 131–136; node130 has advanced. These are open proof branches, not findings or proof closure.

Why node124 retains the operators

The actual terms are `(ARG_0 modInt 18446744073709551616) &Int 4294967295` and `(ARG_0 modInt 18446744073709551616) >>Int 32`. Both structurally fit the imported helpers; their literal-mask/count restrictions are satisfied.

The helpers lack `preserves-definedness`. Booster `internaliseSimpleEquation` marks every partial symbol in either template side as a rejection reason, without using `requires` to discharge it. `applyEquation` rejects those reasons before matching or testing the guard:

- `full-mask-to-modulus` introduces `_modInt_`, a partial function.
- `constant-right-shift` contains partial `_>>Int_`, `_divInt_`, and `_^Int_`.

This explains the Booster applicability failure even for safe literal 32. Source evidence: `/nix/store/9kkn618zz84dgb9gn1959g607mgkh1gp-booster-source-v6/library/Booster/Syntax/ParsedKore/Internalise.hs:891`, `library/Booster/Pattern/ApplyEquations.hs:945`, `library/Booster/Pattern/Util.hs:200`. The indexed `/nix/store/vlhyk3av3v5rd7gcimrx57byknp2nppy-booster` source has the same rejection. The graph logs record rewrites, not attempted simplifications; they do not independently identify each rejection. The manifest also does not fingerprint the RPC executable, so the exact active binary/source linkage remains a provenance limit.

Relevant compiled axioms: Haskell `definition.kore:33946` and `:34558`. Relevant source: supplied KDIST `soroban-semantics/source/soroban-semantics/komet-lemmas.md:100`, `:138`. Production metadata in `compiled.json` confirms the partial declarations above; `_&Int_` itself is total.

Smallest sound local reuse

Use the already proved mask identity at n=32. Reuse the imported shift identity only within a safe literal-count range; this one guarded rule covers the observed counts 1 and 32:

```k
rule [utilization-low32-mask]:
  X:Int &Int 4294967295 => X modInt 4294967296
  [simplification, preserves-definedness]

rule [utilization-small-native-right-shift]:
  X:Int >>Int SHIFT:Int => X divInt (2 ^Int SHIFT)
  requires (0 <=Int SHIFT) andBool (SHIFT <Int 64)
  [simplification, concrete(SHIFT), preserves-definedness]
```

Both sides strictly retain every symbolic operand. Undefined X stays undefined. The fixed mask divisor is nonzero; accepted shift counts are representable by both backends, their powers are defined and positive, and `divInt` gives floor division. Do not substitute truncating `/Int` for arbitrary signed X. An equally safe literal-32 form is `X >>Int 32 => X divInt 4294967296`; exact-divisible `(X -Int (X modInt 4294967296)) /Int 4294967296` is also correct.

`lean-byte/ByteWrap.lean:38` already proves signed infinite-two's-complement AND with `2^n-1` equals Euclidean modulo for every integer. Its strict lifting at `:66`–`:81` supplies the partial-input model. Its byte specialization need not be reproved; instantiate the general theorem at 32. The existing projected right-one rule matches only `((X >>Int 1) modInt 2^64)`, so it does not replace a bare right shift after surrounding modulo disappears.

Do not add `preserves-definedness` to the unrestricted imported shift rule with only `SHIFT >=Int 0`. Pinned LLVM `hook_INT_shr` accepts a positive count above ULONG_MAX and returns 0/-1; `hook_INT_pow` rejects that exponent. Haskell also converts shift counts through machine `Int`. Thus the broad rule has a backend-definedness limit absent from its guard. Evidence: `/nix/store/i3sfsz92jp90nylrzzb1mqsif8gl6iiy-source/runtime/arithmetic/int.cpp:136`, `:159`; `/nix/store/44j0m84525anm69vksijaih7lb2w371m-kore-source/src/Kore/Builtin/Int.hs:283`. The two source helpers can instead be repaired and recompiled, but the shift repair must retain a justified count bound. No blanket annotation of other lemmas is justified.

Nodes130/131: supply-half carry

Let M=2^64, S=ARG_1, L=S mod M, H0=(S-L)/M, H=(H0-(H0 mod 2))/2. Their normalized split compares this expression with `undefined`:

```text
intRelOp(i64, intLt_s,
  u64Xor(H, M-1) &Int (ite(Q <Int 0, 1, 0) +Int H),
  0)
Q = (H0 mod 2)*2^63 + (L-(L mod 2))/2
```

Node130 asserts equality with `undefined`; node131 asserts its negation. Node130 is an internal branch node in the inspected graph, while node131 is pending. The active instruction retains outer modulo/OR/shift forms, while the normalized split predicate uses the displayed form.

The harness supplies `0 <= S <= 2^127-1`, hence `0 <= H0 < 2^63`, `0 <= H < 2^62`, and `0 <= Q < M`. Therefore the carry predicate is false, its increment is zero, and the AND is the low64 complement of H AND H, mathematically zero. This is a value/range argument, not evidence that either pending branch has already been closed by K.

`intRelOp(i64, intLt_s, A, 0)` needs `definedSigned(i64,A)`, meaning `0 <= A < M`, before the regular signed-comparison rule applies. Existing `masked-int-nonnegative` and `masked-int-upper-bound` require a concrete MASK. Here both AND operands are symbolic; they cannot match those restrictions. Existing complement rules also expect primitive projected XOR and an identical second operand; the pending predicate instead has `u64Xor` and an uneliminated increment.

Small generic AND repair: prove the two unsigned-range obligations, retaining both inputs on the RHS rather than erasing them to `true`:

```k
rule [utilization-u64-and-lower]:
  (0 <=Int (A:Int &Int B:Int))
    => (0 <=Int A) andBool (A <Int 18446744073709551616)
       andBool (0 <=Int B) andBool (B <Int 18446744073709551616)
  requires (0 <=Int A) andBool (A <Int 18446744073709551616)
    andBool (0 <=Int B) andBool (B <Int 18446744073709551616)
  [simplification, preserves-definedness]

rule [utilization-u64-and-upper]:
  ((A:Int &Int B:Int) <Int 18446744073709551616)
    => (0 <=Int A) andBool (A <Int 18446744073709551616)
       andBool (0 <=Int B) andBool (B <Int 18446744073709551616)
  requires (0 <=Int A) andBool (A <Int 18446744073709551616)
    andBool (0 <=Int B) andBool (B <Int 18446744073709551616)
  [simplification, preserves-definedness]
```

Sound reason: AND of two unsigned 64-bit integers contains no sign-extension/high bits. Guarded identities support every such pair. Their RHS comparisons retain A and B strictly even when either is a partial expression. These are proposed patterns, not compiled, kernel-checked additions or measured proof progress. A bare `definedSigned` simplification may be bypassed by its unconditional function expansion; matching the two expanded bounds avoids relying on that evaluation order.

Do not replace the signed comparison or complement AND with a constant while merely asserting `preserves-definedness`: erased partial operands require separate definedness evidence. Source-v6 additionally checks erased substitutions at `ApplyEquations.hs:1013`–`:1025`. Do not relax the backend's global definedness gate or mark primitive shifts/modulo/power total.

Model or module changes require a fresh proof-input identity and fresh graph. The recorded runner explicitly rejects changed inputs when resuming. This review changed only this report; production, module, graph, and fingerprints were left untouched. `final-module-review.md` covers the existing byte addition, not these proposed new rules.
