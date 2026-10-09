Carry model correct for the supplied expression. Smallest justified addition:

```k
rule [utilization-projected-complement-and]:
  u64Xor(X:Int, 18446744073709551615) &Int X => X -Int X
  [simplification, preserves-definedness]
```

No range guard needed: valid for every signed integer. X remains strict on both sides; an absent partial X stays absent. This rule has no partial template symbols; the annotation is sound but unnecessary for the template gate. It requires the second operand to become exactly X first. No K module or proof graph changed in this review.

For M=2^64 and 0 <= S <= 2^127-1, the supplied definitions give 0 <= L < M, 0 <= H0 < 2^63, 0 <= H < 2^62, and 0 <= Q < M. The numerators of every /Int are exactly divisible by their positive fixed denominators, so truncation agrees with Euclidean division, including in the intermediate identities. Q's integer comparison Q <Int 0 is false; its carry increment is zero. The AND then equals zero, and intRelOp(i64,intLt_s,0,0) returns i32 zero. Q may occupy the upper unsigned half; replacing the supplied integer test with a signed64 test would change the argument.

Existing normalization and applicability:

| Rule / source | What it does; limit |
|---|---|
| KDIST komet-lemmas.md:28-30, project-u64-xor | Primitive XOR modulo M becomes u64Xor. Its partial modInt template has no preserves-definedness; Booster's template gate can reject it. Presence does not prove execution. |
| :277-283, u64-xor-native | Total u64Xor has exact low64 bvxor SMT hook. Native expansion to primitive XOR modulo M is concrete-only; symbolic H does not expand. Compiled KORE declarations at :2799 and native axiom at :80618 confirm this. |
| :46-56, u64-xor-sign-bit | Sign-bit comparison only. The first rule introduces partial modInt and lacks preserves-definedness. It does not directly rewrite the full-word upper bound. |
| :73-81, masked-int-nonnegative / upper-bound | Require concrete MASK. Both operands of the supplied AND are symbolic, so these restrictions fail. |
| :127-135, masked-complement-and-left / right | Expect primitive projected XOR and an identical other operand. Supplied u64Xor and the uneliminated increment do not match. Even after matching, partial modInt in the template triggers Booster's gate; changing the annotation alone also erases X. Compiled axioms at :33974 and :34016 retain those restrictions. |
| Current arithmetic.k:209-219, utilization-and-nonnegative / word-upper | Both templates use total comparison, AND and Boolean symbols; both retain A/B and carry preserves-definedness. Their guarded value and partial-input semantics pass the existing Lean proofs. Template partialness does not explain their non-application. |

The generic AND rules still need guards proved for A=u64Xor(H,M-1) and B=ite(Q<0,1,0)+H. The lower rule needs nonnegative A/B; the upper rule also needs A/B<M. u64Xor's SMT hook supplies an unsigned word, but proving B's range still needs the supply bounds through fixed division/modulo and the conditional. Booster checks remaining guards with SMT and rejects unknown/invalid results. The exact current outcome requires fresh match/guard logs; no old-node evidence is available. Raw &Int itself is an uninterpreted SMT function: KORE :1021 supplies smtlib(andInt), and Booster Translate.hs:296-305 declares it as a function; ordinary simplifications are not automatically SMT lemmas (:181-184).

Definedness evidence: Booster source-v6 Internalise.hs:891-904 collects partial symbols from both template sides before evaluating guards. ApplyEquations.hs:945-957 rejects this list before matching. Its separate erased-substitution check at :1013-1025 is still required even for total templates. The proposed rule erases no variable. Existing integer-self-subtraction can subsequently reduce the supplied H-H because source-v6 :1039-1048 recognizes nonzero constant INT.emod / INT.tdiv denominators recursively. For arbitrary partial X that later erasure must remain blocked.

Kernel evidence: [ProjectedComplement.lean](carry-review/ProjectedComplement.lean) defines signed XOR independently by integer sign constructors and natural XOR bits, reuses UtilizationAnd.signedAnd, and proves projection equals signed XOR modulo M (:30). Every complement bit AND its original bit is false (:35-53); :55 proves the exact X-X result. Strict Option equality and both definedness directions are at :59-73. The unguarded theorem is stronger than any supplied word-range guard. A separate check proves erasure to some 0 is wrong for none. This is a Lean model with explicit K correspondence, not a mechanically imported K definition.

Exact correspondence: signedXor models total INT.xor, signedAnd models total INT.and, projectedXor models u64Xor's low64 primitive definition, and Lean subtraction models total INT.sub. K builtin domains.md:1243-1274 specifies infinite two's-complement bitwise operations; LLVM int.cpp:61-66 / :187-192 calls mpz_and / mpz_xor. Positive fixed modInt is Euclidean remainder (LLVM :23-40); /Int truncates (LLVM :89-98). The proposed K template contains u64Xor, &Int, -Int and literals only: all total, all symbolic inputs retained.

Recheck: `python3 carry-review/check.py`. Fresh Lean 4.22.0 --trust=0 compiles unchanged dependency copies and ProjectedComplement; all four exit zero. Printed dependencies contain only propext, Classical.choice and Quot.sound; no sorryAx or custom axioms. Dependency byte equality was checked against math-check/lean. No K prover run, graph mutation, runtime change, worktree edit, or frozen-borrow edit. Source-v6 was inspected directly; this review does not establish its linkage to a currently running RPC binary. The earlier model-review.md graph/node counts are historical and cannot establish current progress after graph loss.

Source roots: KDIST=/nix/store/zm2sn27mpb1xks6r2g68x22282lzr3ca-komet-ba813e1463a180ebcc4903658736ccb84755559a/kdist (source/soroban-semantics/komet-lemmas.md and haskell/definition.kore under soroban-semantics); Booster=/nix/store/9kkn618zz84dgb9gn1959g607mgkh1gp-booster-source-v6/library/Booster; LLVM=/nix/store/i3sfsz92jp90nylrzzb1mqsif8gl6iiy-source/runtime/arithmetic/int.cpp; K builtin=/nix/store/aqjgfc96ra0v1drpil3jlx23frcz5nrb-k-7.1.337-4a46d1231473b599c699160132fd6e76a5c46406/include/kframework/builtin/domains.md. Signed relation range comes from KDIST wasm-semantics/source/wasm-semantics/wasm-data.md:458 and numeric.md:399-400.
