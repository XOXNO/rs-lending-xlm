import ByteWrap
import HalfWord
import Left32
import ProjectedComplement
import UtilizationAnd
import UtilizationOrdering
import UtilizationPacking
import UtilizationQuotient
import UtilizationWords
import WordOr

/-
One theorem per `utilization-*` rule of ../arithmetic.k (its first 17 rules
are byte-identical to ../../borrow-double/double-cli.k and are covered by
DoubleCliRules.lean there). Operator model as in the imported files:
`/Int` = Int.tdiv, `modInt` = Euclidean %, `|Int` = Fixed64Packing's Int.lor,
`&Int` = UtilizationAnd.signedAnd, `xorInt` = ProjectedComplement.signedXor,
`u64Xor(A, B)` = UtilizationCarry.projectedXor A B (proved equal to
`signedXor A B % 2^64`), `u64Or(A, B)` = HalfWord.u64Or, `#wrap(1, X)` =
ByteWrap.wrapOne, shifts by literal counts as in HalfWord/Left32.
-/
namespace UtilizationRules

abbrev W : Int := 18446744073709551616
abbrev H : Int := 9223372036854775808

def hi (X : Int) : Int := (X - X % W).tdiv W
def signed64 (n : Int) : Int := if n < H then n else n - W

-- [utilization-signed128-highword-sign]
theorem signed128_highword_sign (X : Int) (h0 : -170141183460469231731687303715884105728 ≤ X)
    (h1 : X ≤ 170141183460469231731687303715884105727) :
    decide (hi X % W < H) = decide (0 ≤ X) :=
  decide_eq_decide.mpr (Utilization.high_sign X (by unfold Utilization.max128; omega)
    (by unfold Utilization.max128; omega))

-- [utilization-signed128-native-highword-decoder]
theorem signed128_native_highword_decoder (X : Int) (h0 : -170141183460469231731687303715884105728 ≤ X)
    (h1 : X ≤ 170141183460469231731687303715884105727) :
    (0 ≤ hi X % W ∧ hi X % W < W) ∧ signed64 (hi X % W) = hi X := by
  refine ⟨⟨Int.emod_nonneg _ (by decide), Int.emod_lt_of_pos _ (by decide)⟩, ?_⟩
  exact Utilization.high_decoder X (by unfold Utilization.max128; omega)
    (by unfold Utilization.max128; omega)

-- [utilization-nonnegative128-highword-projection]
theorem nonnegative128_highword_projection (X : Int) (h0 : 0 ≤ X)
    (h1 : X ≤ 170141183460469231731687303715884105727) :
    hi X % W = hi X :=
  Utilization.nonnegative_high_projection X h0 (by unfold Utilization.max128; omega)

-- [utilization-highword-nonnegative]
theorem highword_nonnegative (X : Int) : decide (hi X ≥ 0) = decide (0 ≤ X) :=
  decide_eq_decide.mpr (Utilization.high_nonnegative X)

theorem lor_nonneg_eq (X Y : Int) (hx : 0 ≤ X) (hy : 0 ≤ Y) :
    Int.lor X Y = WordOr.nonnegativeOr X Y := by
  obtain ⟨a, rfl⟩ := Int.eq_ofNat_of_zero_le hx
  obtain ⟨b, rfl⟩ := Int.eq_ofNat_of_zero_le hy
  rfl

-- [utilization-word-or-zero]   requires 0 <= X < 2^64 and 0 <= Y < 2^64
theorem word_or_zero (X Y : Int) (hx : 0 ≤ X) (hxw : X < W) (hy : 0 ≤ Y) (hyw : Y < W) :
    decide (Int.lor X Y % W = 0) = (decide (X = 0) && decide (Y = 0)) := by
  rw [lor_nonneg_eq X Y hx hy, ← Bool.decide_and]
  exact decide_eq_decide.mpr (WordOr.integer_projected_or_zero_iff X Y hx hxw hy hyw)

-- [utilization-projected-word-or-zero]
theorem projected_word_or_zero (X Y : Int) (hx : 0 ≤ X) (hxw : X < W) (hy : 0 ≤ Y)
    (hyw : Y < W) :
    decide (HalfWord.u64Or X Y = 0) = (decide (X = 0) && decide (Y = 0)) :=
  word_or_zero X Y hx hxw hy hyw

-- [utilization-pack-fixed32]   requires 0 <= L < 2^32
theorem pack_fixed32 (Hi L : Int) (h0 : 0 ≤ L) (h1 : L < 4294967296) :
    Int.lor (Hi * 4294967296) L = Hi * 4294967296 + L :=
  UtilizationPacking.packing32 Hi L h0 h1

-- [utilization-projected-pack-fixed32]
theorem projected_pack_fixed32 (Hi L : Int) (h0 : 0 ≤ L) (h1 : L < 4294967296) :
    HalfWord.u64Or (Hi * 4294967296) L = (Hi * 4294967296 + L) % W :=
  UtilizationPacking.projected_packing32 Hi L h0 h1

-- [utilization-native-integer-ordering]
theorem native_integer_ordering (X Y : Int) :
    UtilizationOrdering.decode (UtilizationOrdering.compareInt X Y) =
      (if X < Y then -1 else if X > Y then 1 else 0) :=
  UtilizationOrdering.comparison X Y

-- [utilization-right-one-low-word]
theorem right_one_low_word (X : Int) :
    (X >>> (1 : Nat)) % W = ((X - X % 2).tdiv 2) % W :=
  HalfWord.right_one_low X

-- [utilization-left63-low-word]
theorem left63_low_word (X : Int) : (X <<< (63 : Nat)) % W = (X % 2) * H :=
  HalfWord.left_63_low X

-- [utilization-projected-pack-fixed63]   requires 0 <= L < 2^63
theorem projected_pack_fixed63 (Hi L : Int) (h0 : 0 ≤ L) (h1 : L < H) :
    HalfWord.u64Or (Hi * H) L = (Hi * H + L) % W :=
  HalfWord.pack_63_low Hi L h0 h1

-- [utilization-quotient-lower]   requires 0 <= N, 0 < D
theorem quotient_lower (N D : Int) (hn : 0 ≤ N) (hd : 0 < D) :
    decide (N.tdiv D * D ≤ N) = (decide (0 ≤ N) && decide (0 < D)) :=
  UtilizationQuotient.lower N D hn hd

-- [utilization-quotient-upper]
theorem quotient_upper (N D : Int) (hn : 0 ≤ N) (hd : 0 < D) :
    decide (N < (N.tdiv D + 1) * D) = (decide (0 ≤ N) && decide (0 < D)) :=
  UtilizationQuotient.upper N D hn hd

-- [utilization-quotient-bound]
theorem quotient_bound (N D Q : Int) (hn : 0 ≤ N) (hd : 0 < D) :
    decide (N.tdiv D ≤ Q) = decide (N < (Q + 1) * D) :=
  UtilizationQuotient.bound N D Q hn hd

-- [utilization-symbolic-byte-wrap]   #wrap(1, X) => X modInt 256
theorem symbolic_byte_wrap (X : Int) : ByteWrap.wrapOne X = X % 256 :=
  ByteWrap.wrapOne_eq_mod X

-- [utilization-mask32]
theorem mask32 (X : Int) : UtilizationAnd.signedAnd X 4294967295 = X % 4294967296 :=
  UtilizationAnd.mask32_literal X

-- [utilization-right32]
theorem right32 (X : Int) : X >>> (32 : Nat) = (X - X % 4294967296).tdiv 4294967296 :=
  UtilizationAnd.right32_literal X

-- [utilization-and-nonnegative]   requires 0 <= A, 0 <= B
theorem and_nonnegative (A B : Int) (ha : 0 ≤ A) (hb : 0 ≤ B) :
    decide (0 ≤ UtilizationAnd.signedAnd A B) = (decide (0 ≤ A) && decide (0 ≤ B)) :=
  UtilizationAnd.lower_range A B ha hb

-- [utilization-and-word-upper]   requires 0 <= A, B < 2^64
theorem and_word_upper (A B : Int) (ha : 0 ≤ A) (haw : A < W) (hb : 0 ≤ B) (hbw : B < W) :
    decide (UtilizationAnd.signedAnd A B < W) = (decide (A < W) && decide (B < W)) :=
  UtilizationAnd.upper_range A B ha haw hb hbw

-- [utilization-left32]
theorem left32 (X : Int) : X <<< (32 : Nat) = X * 4294967296 :=
  Left32Review.signed_left32 X

-- [utilization-projected-complement-and]   u64Xor(X, 2^64-1) &Int X => X -Int X
theorem projected_complement_and (X : Int) :
    UtilizationAnd.signedAnd (UtilizationCarry.projectedXor X (W - 1)) X = X - X :=
  UtilizationCarry.complement_and_sub X

#print axioms signed128_highword_sign
#print axioms signed128_native_highword_decoder
#print axioms nonnegative128_highword_projection
#print axioms highword_nonnegative
#print axioms word_or_zero
#print axioms projected_word_or_zero
#print axioms pack_fixed32
#print axioms projected_pack_fixed32
#print axioms native_integer_ordering
#print axioms right_one_low_word
#print axioms left63_low_word
#print axioms projected_pack_fixed63
#print axioms quotient_lower
#print axioms quotient_upper
#print axioms quotient_bound
#print axioms symbolic_byte_wrap
#print axioms mask32
#print axioms right32
#print axioms and_nonnegative
#print axioms and_word_upper
#print axioms left32
#print axioms projected_complement_and

end UtilizationRules
