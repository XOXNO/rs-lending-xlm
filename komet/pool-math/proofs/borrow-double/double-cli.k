module SIGNED-DISTRIBUTE-COMPLEMENT-NEGATIVE-PACK64-DOUBLE-RAY-SIGNED-DECODER-FLOOR-EQUALITY-LOW-WORD-SIGNED64-SIGN-FIXED64-XOR
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
endmodule
