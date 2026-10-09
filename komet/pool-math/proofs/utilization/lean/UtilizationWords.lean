import Lean.Elab.Tactic.Omega

/- Integer formula only. K /Int and Rust signed / use Int.tdiv.
   This file does not establish compiled-Wasm refinement. -/
namespace Utilization

def ray : Int := 1000000000000000000000000000
def numerator (b s : Int) : Int := b * ray + s.tdiv 2
def utilization (b s : Int) : Int :=
  if s = 0 then 0 else (numerator b s).tdiv s

theorem quotient_lower (n d : Int) (hn : 0 ≤ n) (hd : 0 < d) :
    n.tdiv d * d ≤ n := by
  rw [Int.tdiv_eq_ediv_of_nonneg hn]
  have := Int.ediv_add_emod' n d
  have := Int.emod_nonneg n (Int.ne_of_gt hd)
  omega

theorem quotient_upper (n d : Int) (hn : 0 ≤ n) (hd : 0 < d) :
    n < (n.tdiv d + 1) * d := by
  rw [Int.tdiv_eq_ediv_of_nonneg hn, Int.add_mul, Int.one_mul]
  have := Int.ediv_add_emod' n d
  have := Int.emod_lt_of_pos n hd
  omega

theorem quotient_le_iff (n d m : Int) (hn : 0 ≤ n) (hd : 0 < d) :
    n.tdiv d ≤ m ↔ n < (m + 1) * d := by
  rw [Int.tdiv_eq_ediv_of_nonneg hn]
  constructor
  · intro h
    exact (Int.ediv_lt_iff_lt_mul hd).mp (by omega)
  · intro h
    have := (Int.ediv_lt_iff_lt_mul hd).mpr h
    omega

theorem positive_domain (b s : Int) (hb : 0 ≤ b) (hs : 0 < s) (hbs : b ≤ s) :
    0 ≤ numerator b s ∧ numerator b s < (ray + 1) * s := by
  have hr : 0 ≤ ray := by decide
  have hp := Int.mul_le_mul_of_nonneg_right hbs hr
  have hm := Int.mul_nonneg hb hr
  have hh : 0 ≤ s.tdiv 2 ∧ s.tdiv 2 < s := by
    rw [Int.tdiv_eq_ediv_of_nonneg (by omega)]
    omega
  constructor
  · unfold numerator
    omega
  · unfold numerator
    rw [Int.add_mul, Int.one_mul, Int.mul_comm ray s]
    omega

theorem exact_and_bounded (b s : Int) (hb : 0 ≤ b) (hs : 0 < s) (hbs : b ≤ s) :
    0 ≤ utilization b s ∧ utilization b s ≤ ray ∧
    utilization b s * s ≤ numerator b s ∧
    numerator b s < (utilization b s + 1) * s := by
  have hdom := positive_domain b s hb hs hbs
  unfold utilization
  rw [if_neg (Int.ne_of_gt hs)]
  refine ⟨Int.tdiv_nonneg hdom.1 (by omega), ?_,
    quotient_lower _ _ hdom.1 hs, quotient_upper _ _ hdom.1 hs⟩
  rw [Int.tdiv_eq_ediv_of_nonneg hdom.1]
  have := (Int.ediv_lt_iff_lt_mul hs).mpr hdom.2
  omega

theorem zero_supply (b : Int) : utilization b 0 = 0 := by
  simp [utilization]

theorem tie_witness : utilization 1 (2 * ray) = 1 := by decide

def max128 : Int := 170141183460469231731687303715884105727
def max256 : Int :=
  57896044618658097711785492504343953926634992332820282019728792003956564819967

def word : Int := 18446744073709551616
def halfWord : Int := 9223372036854775808
def high (x : Int) : Int := (x - x % word).tdiv word

theorem high_as_floor (x : Int) : high x = x / word := by
  unfold high
  apply Int.tdiv_eq_of_eq_mul_right (show word ≠ 0 by decide)
  have := Int.ediv_add_emod x word
  omega

theorem high_sign (x : Int) (hlo : -max128 - 1 ≤ x) (hhi : x ≤ max128) :
    high x % word < halfWord ↔ 0 ≤ x := by
  rw [high_as_floor]
  unfold word halfWord max128 at *
  omega

theorem high_decoder (x : Int) (hlo : -max128 - 1 ≤ x) (hhi : x ≤ max128) :
    (if high x % word < halfWord then high x % word
      else high x % word - word) = high x := by
  rw [high_as_floor]
  unfold word halfWord max128 at *
  split <;> omega

theorem machine_bounds (b s : Int) (hb : 0 ≤ b) (hs : 0 < s)
    (hbs : b ≤ s) (hsmax : s ≤ max128) :
    numerator b s ≤ max256 ∧ utilization b s ≤ max128 := by
  have hbn : b ≤ max128 := by omega
  have hp := Int.mul_le_mul_of_nonneg_right hbn (show 0 ≤ ray by decide)
  have hh : s.tdiv 2 ≤ max128 / 2 := by
    rw [Int.tdiv_eq_ediv_of_nonneg (by omega)]
    omega
  have hq := (exact_and_bounded b s hb hs hbs).2.1
  unfold numerator ray max128 max256 at *
  omega

#print axioms quotient_lower
#print axioms quotient_upper
#print axioms quotient_le_iff
#print axioms positive_domain
#print axioms exact_and_bounded
#print axioms zero_supply
#print axioms tie_witness
#print axioms machine_bounds
#print axioms high_as_floor
#print axioms high_sign
#print axioms high_decoder

theorem nonnegative_high_projection (x : Int) (hlo : 0 ≤ x) (hhi : x ≤ max128) :
    high x % word = high x := by
  rw [high_as_floor]
  unfold word max128 at *
  omega

#print axioms nonnegative_high_projection

theorem high_nonnegative (x : Int) : 0 ≤ high x ↔ 0 ≤ x := by
  rw [high_as_floor]
  unfold word
  omega

#print axioms high_nonnegative
end Utilization
