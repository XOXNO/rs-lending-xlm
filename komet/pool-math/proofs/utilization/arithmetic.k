module UTILIZATION-COMPLEMENT-LEMMAS
  imports KASMER
  rule [distribute-signed-word-comparison]:
    ((#if B:Bool #then W:Int
      #else W +Int -18446744073709551616 #fi) >=Int 0)
      => #if B
         #then notBool (W <Int 0)
         #else notBool (W <Int 18446744073709551616)
         #fi
    [simplification]
  rule [xor-zero-left-nonnegative-complement]:
    (0 <=Int u64Xor(X:Int, Y:Int))
      => notBool (u64Xor(X, Y) <Int 0)
    [simplification]
  rule [integer-self-subtraction]: I:Int -Int I => 0 [simplification]
  // Fixed nonzero divisor; both sides strictly retain X and N.
  rule [floor-word64-less-than]:
    (((X:Int -Int (X modInt 18446744073709551616)) /Int 18446744073709551616) <Int N:Int)
      => X <Int (N *Int 18446744073709551616)
    [simplification, preserves-definedness]
  rule [distribute-signed-negative-word-comparison]:
    ((#if B:Bool #then W:Int
      #else W +Int -18446744073709551616 #fi) <Int 0)
      => #if B
         #then W <Int 0
         #else W <Int 18446744073709551616
         #fi
    [simplification]
  rule [pack-fixed64-product-low-word]:
    ((H:Int *Int 18446744073709551616) |Int L:Int)
      => (H *Int 18446744073709551616 +Int L)
    requires (0 <=Int L) andBool (L <Int 18446744073709551616)
    [simplification]
  // Positive fixed divisor; X remains on both sides, including partial X.
  rule [double-ray-half-up-nonnegative]:
    ((X:Int *Int 2000000000000000000000000000
       +Int 500000000000000000000000000) /Int 1000000000000000000000000000)
      => X *Int 2
    requires 0 <=Int X
    [simplification, preserves-definedness]
  // Exact word reconstruction; fixed nonzero divisors and strict X on both sides.
  rule [signed-fixed64-decoder-roundtrip]:
    (((X:Int -Int (X modInt 18446744073709551616)) /Int 18446744073709551616)
        *Int 18446744073709551616
      +Int (X modInt 18446744073709551616))
      => X
    [simplification, preserves-definedness]
  // Exact Euclidean word interval; X and N remain strict on both sides.
  rule [floor-fixed64-word-equality]:
    (((X:Int -Int (X modInt 18446744073709551616)) /Int 18446744073709551616) ==Int N:Int)
      => ((N *Int 18446744073709551616) <=Int X)
         andBool (X <Int ((N +Int 1) *Int 18446744073709551616))
    [simplification, preserves-definedness]
  // Exact low word; adding 2^128 is a multiple of the fixed modulus.
  rule [signed-fixed64-low-word]:
    ((#if X:Int <Int 0
      #then X +Int 340282366920938463463374607431768211456
      #else X #fi) modInt 18446744073709551616)
      => X modInt 18446744073709551616
    [simplification, preserves-definedness]
  // Exact sign test of a masked word; strict X retained, modulus nonzero.
  rule [signed64-masked-nonnegative]:
    (#signed(i64, (X:Int modInt 18446744073709551616)) >=Int 0)
      => (X modInt 18446744073709551616) <Int 9223372036854775808
    [simplification, preserves-definedness]
  // Fixed nonzero modulus; both XOR operands remain strict on both sides.
  rule [fixed64-masked-xor-sign-bit]:
    (((X:Int xorInt Y:Int) modInt 18446744073709551616) <Int 9223372036854775808)
      => ((X modInt 18446744073709551616) <Int 9223372036854775808)
         ==Bool ((Y modInt 18446744073709551616) <Int 9223372036854775808)
    [simplification, preserves-definedness]
  // The existing projected XOR is exactly the primitive XOR modulo 2^64.
  rule [fixed64-projected-xor-masked-sign-bit]:
    ((u64Xor(X:Int, Y:Int) modInt 18446744073709551616) <Int 9223372036854775808)
      => ((X modInt 18446744073709551616) <Int 9223372036854775808)
         ==Bool ((Y modInt 18446744073709551616) <Int 9223372036854775808)
    [simplification, preserves-definedness]
  rule [fixed64-mod-idempotence]:
    ((X:Int modInt 18446744073709551616) modInt 18446744073709551616)
      => X modInt 18446744073709551616
    [simplification, preserves-definedness]
  // One-bit carry occupies the bit cleared by the projected left shift.
  // Every signed high word is supported; both operands remain strict.
  rule [fixed64-projected-shift-one-carry]:
    u64Or(((H:Int <<Int 1) modInt 18446744073709551616), L:Int)
      => (H modInt 9223372036854775808) *Int 2 +Int L
    requires (0 <=Int L) andBool (L <Int 2)
    [simplification, preserves-definedness]
  // Euclidean low-word projection commutes with the literal left shift by one.
  // All signed X; X remains strict and the modulus is fixed and nonzero.
  rule [fixed64-shift-one-low-projection]:
    (((X:Int modInt 18446744073709551616) <<Int 1) modInt 18446744073709551616)
      => (X *Int 2) modInt 18446744073709551616
    [simplification, preserves-definedness]
  // Exact quotient recomposition for doubling, including negative X.
  // /Int truncates; each numerator is exactly divisible by its fixed divisor.
  rule [fixed64-double-high-recomposition]:
    (((X:Int -Int (X modInt 18446744073709551616)) /Int 18446744073709551616) *Int 2
      +Int (((X modInt 18446744073709551616)
        -Int ((X modInt 18446744073709551616) modInt 9223372036854775808)) /Int 9223372036854775808))
      => ((X *Int 2 -Int ((X *Int 2) modInt 18446744073709551616)) /Int 18446744073709551616)
    [simplification, preserves-definedness]
  // A signed i128 high word has the same sign as its complete value.
  rule [utilization-signed128-highword-sign]:
    ((((X:Int -Int (X modInt 18446744073709551616)) /Int 18446744073709551616)
        modInt 18446744073709551616) <Int 9223372036854775808)
      => 0 <=Int X
    requires (-170141183460469231731687303715884105728 <=Int X)
      andBool (X <=Int 170141183460469231731687303715884105727)
    [simplification, preserves-definedness]
  // Both masked and signed forms denote the original signed high quotient.
  rule [utilization-signed128-native-highword-decoder]:
    #signed(i64, (((X:Int -Int (X modInt 18446744073709551616)) /Int 18446744073709551616)
        modInt 18446744073709551616))
      => (X -Int (X modInt 18446744073709551616)) /Int 18446744073709551616
    requires (-170141183460469231731687303715884105728 <=Int X)
      andBool (X <=Int 170141183460469231731687303715884105727)
    [simplification, preserves-definedness]
  // Under a nonnegative i128 premise the high quotient already fits u64.
  rule [utilization-nonnegative128-highword-projection]:
    (((X:Int -Int (X modInt 18446744073709551616)) /Int 18446744073709551616)
        modInt 18446744073709551616)
      => (X -Int (X modInt 18446744073709551616)) /Int 18446744073709551616
    requires (0 <=Int X)
      andBool (X <=Int 170141183460469231731687303715884105727)
    [simplification, preserves-definedness]
  // Sign of the exact signed high quotient, including all negative values.
  rule [utilization-highword-nonnegative]:
    (((X:Int -Int (X modInt 18446744073709551616)) /Int 18446744073709551616) >=Int 0)
      => 0 <=Int X
    [simplification, preserves-definedness]
  // Two nonnegative u64 words OR to zero exactly when both words are zero.
  rule [utilization-word-or-zero]:
    (((X:Int |Int Y:Int) modInt 18446744073709551616) ==Int 0)
      => (X ==Int 0) andBool (Y ==Int 0)
    requires (0 <=Int X) andBool (X <Int 18446744073709551616)
      andBool (0 <=Int Y) andBool (Y <Int 18446744073709551616)
    [simplification, preserves-definedness]
  rule [utilization-projected-word-or-zero]:
    (u64Or(X:Int, Y:Int) ==Int 0)
      => (X ==Int 0) andBool (Y ==Int 0)
    requires (0 <=Int X) andBool (X <Int 18446744073709551616)
      andBool (0 <=Int Y) andBool (Y <Int 18446744073709551616)
    [simplification, preserves-definedness]
  // Disjoint 32-bit packing used by LLVM's 64-bit product carry.
  rule [utilization-pack-fixed32]:
    ((H:Int *Int 4294967296) |Int L:Int)
      => H *Int 4294967296 +Int L
    requires (0 <=Int L) andBool (L <Int 4294967296)
    [simplification, preserves-definedness]
  rule [utilization-projected-pack-fixed32]:
    u64Or(H:Int *Int 4294967296, L:Int)
      => (H *Int 4294967296 +Int L) modInt 18446744073709551616
    requires (0 <=Int L) andBool (L <Int 4294967296)
    [simplification, preserves-definedness]
  // Composition of data.md's three exact integer-ordering cases.
  // The first comparison strictly retains both inputs, including partial terms.
  rule [utilization-native-integer-ordering]:
    Ordering2Int(compareInt(X:Int, Y:Int))
      => #if X <Int Y #then -1
         #else #if X >Int Y #then 1 #else 0 #fi #fi
    [simplification, preserves-definedness]
  // Literal count 1: arithmetic right shift is exact Euclidean halving.
  rule [utilization-right-one-low-word]:
    ((X:Int >>Int 1) modInt 18446744073709551616)
      => ((X -Int (X modInt 2)) /Int 2) modInt 18446744073709551616
    [simplification, preserves-definedness]
  // Literal count 63 retains only the lowest bit in the u64 projection.
  rule [utilization-left63-low-word]:
    ((X:Int <<Int 63) modInt 18446744073709551616)
      => (X modInt 2) *Int 9223372036854775808
    [simplification, preserves-definedness]
  rule [utilization-projected-pack-fixed63]:
    u64Or(H:Int *Int 9223372036854775808, L:Int)
      => (H *Int 9223372036854775808 +Int L) modInt 18446744073709551616
    requires (0 <=Int L) andBool (L <Int 9223372036854775808)
    [simplification, preserves-definedness]
  // Universal nonnegative integer quotient laws; all symbolic operands stay
  // strict on the RHS. These apply only after execution produces N /Int D.
  rule [utilization-quotient-lower]:
    (((N:Int /Int D:Int) *Int D) <=Int N)
      => (0 <=Int N) andBool (0 <Int D)
    requires (0 <=Int N) andBool (0 <Int D)
    [simplification, preserves-definedness]
  rule [utilization-quotient-upper]:
    (N:Int <Int (((N /Int D:Int) +Int 1) *Int D))
      => (0 <=Int N) andBool (0 <Int D)
    requires (0 <=Int N) andBool (0 <Int D)
    [simplification, preserves-definedness]
  rule [utilization-quotient-bound]:
    ((N:Int /Int D:Int) <=Int Q:Int)
      => N <Int ((Q +Int 1) *Int D)
    requires (0 <=Int N) andBool (0 <Int D)
    [simplification, preserves-definedness]
  // Fixed positive byte width: signed two-complement mask is Euclidean mod.
  rule [utilization-symbolic-byte-wrap]:
    #wrap(1, X:Int) => X modInt 256
    [simplification, preserves-definedness]
  // Literal 32-bit specializations of existing helpers. Both sides remain
  // strict in X; fixed nonzero divisors avoid arbitrary shift-count domains.
  rule [utilization-mask32]:
    (X:Int &Int 4294967295) => X modInt 4294967296
    [simplification, preserves-definedness]
  rule [utilization-right32]:
    (X:Int >>Int 32)
      => (X -Int (X modInt 4294967296)) /Int 4294967296
    [simplification, preserves-definedness]
  // Only representation bounds: both operand predicates remain strict.
  rule [utilization-and-nonnegative]:
    (0 <=Int (A:Int &Int B:Int))
      => (0 <=Int A) andBool (0 <=Int B)
    requires (0 <=Int A) andBool (0 <=Int B)
    [simplification, preserves-definedness]
  rule [utilization-and-word-upper]:
    ((A:Int &Int B:Int) <Int 18446744073709551616)
      => (A <Int 18446744073709551616) andBool (B <Int 18446744073709551616)
    requires (0 <=Int A) andBool (A <Int 18446744073709551616)
      andBool (0 <=Int B) andBool (B <Int 18446744073709551616)
    [simplification, preserves-definedness]
  // Fixed count 32 is valid in LLVM and Haskell; strict signed multiplication.
  rule [utilization-left32]:
    (X:Int <<Int 32) => X *Int 4294967296
    [simplification, preserves-definedness]
  // Complemented low64 bits cannot overlap X; X remains strict on both sides.
  rule [utilization-projected-complement-and]:
    u64Xor(X:Int, 18446744073709551615) &Int X => X -Int X
    [simplification, preserves-definedness]
endmodule
