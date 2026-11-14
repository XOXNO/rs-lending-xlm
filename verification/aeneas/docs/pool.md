# Pool accounting: Aeneas + Lean proofs

Machine-checked proofs about the production pool accounting code in
`contracts/pool/src`. They cover:

* the share-total mints and burns, and the revenue claim;
* the solvency and utilization guards;
* cash bookkeeping, protocol revenue booking and bad-debt socialization;
* the flash-loan fee arithmetic;
* interest accrual (`compound_interest`, `accrue_step`, `accrue_chunk`,
  `global_sync`);
* the operation legs of `contracts/pool/src/ops/` (supply, borrow, withdraw,
  repay, net settlement, seize, recapitalize, strategy, revenue claim), and
  the Certora pool summaries they discharge.

Charon + Aeneas (pinned in `../README.md`) translate the Rust to Lean. Every
theorem is stated about the generated `lean/PoolAeneas/Funs.lean`. The
extraction crate, trusted models and tool workarounds are in
[extraction.md](extraction.md#pool).

Every headline theorem depends only on `propext`, `Classical.choice` and
`Quot.sound` (checked with `#print axioms`, see [Axiom check](#axiom-check)). There is no
`sorry`, `admit` or `axiom` in `lean/PoolAeneas/`.

## Conventions

* `f ⦃ r => P r ⦄` is total correctness: `f` returns (does not abort) and
  `P` holds. `(∃ u, f = ok u) ↔ C` is an exact pass/fail condition: the
  guard returns iff `C`, and aborts otherwise. `f = ok c' → P c'` says what
  holds after every successful return, whatever the input.
* Specifications never divide. `IsFloor n d q` is `q*d ≤ n < q*d + d`,
  `IsCeil n d q` is `n ≤ q*d < n + d`, `IsHalfUp n d q` is
  `2qd ≤ 2n + d < 2qd + 2d` (all for `d > 0`).
* `RAY = 10^27`, `dec` is the asset's decimals, `k = 27 - dec`. Values written
  `x.val` are the integers inside `i128`/`Ray`.
* Error *codes* are not modelled: `panic_with_error!`/`assert_with_error!`
  abort, which Aeneas translates to failure.

## Extraction and trusted base

The extraction crate, its roots, stand-ins and shim, the full table of
hand-written models, the copied proof files, the rebuild commands and the
Aeneas workaround are in [extraction.md](extraction.md#pool).

The theorems trust three things
([details](extraction.md#pool-trusted-base)):

1. Charon and Aeneas translate faithfully (pinned in `../README.md`).
2. The hand-written models in `lean/PoolAeneas/TypesExternal.lean` and
   `FunsExternal.lean` are right:
   * `I256` is exact integer arithmetic with the host's 256-bit traps.
   * `i128::from(u64)`, `NonZero::<u64>::{new, get}` and the array iterator
     follow the Rust documentation.
   * Storage reads and ledger time (`storedParams`, `storedState`,
     `ledgerNowMs`) are uninterpreted. Theorems hold for every value.
   * Storage writes, TTL extensions and the two reached events return
     `ok ()`.
3. `pool/postextract.sh` renames one duplicated field in the generated Lean
   without changing its meaning
   ([Aeneas bug](extraction.md#aeneas-bug-duplicate-parent-clause-field)).

Token transfers, the flash-loan callback, `run_batch`, `market.rs` and the
contract entry points are not extracted
([roots](extraction.md#pool-extraction-roots)).

Storage and time are not state in the model: nothing here says that a read
returns what an earlier write stored. So the argument "the stored-state
invariant is inductive" is made explicitly, in three steps:

1. Every leg's committed cache satisfies `PoolInv` (the leg theorems).
2. `load_ok` says a load copies the stored fields.
3. `synced_market_spec` carries `PoolInv` through accrual.

## Theorems

Files are under `lean/PoolAeneas/Proofs/`.

### Share totals and revenue claims (`Shares.lean`, `contracts/pool/src/cache/shares.rs`)

`SharesInv c` is INV-ACCT-01 on one cache: `0 ≤ revenue ≤ supplied` and
`0 ≤ borrowed`.

| Lean name | Rust function | Claim | Hypotheses | Invariants |
|---|---|---|---|---|
| `mint_supply_preserves` | `Cache::mint_supply` | On success `supplied += s`, nothing else changes, `SharesInv` preserved | `SharesInv`, `s ≥ 0` | INV-ACCT-01 |
| `burn_supply_preserves` | `Cache::burn_supply` | On success `0 ≤ s ≤ supplied`, `supplied -= s`, nothing else changes, `SharesInv` preserved | `SharesInv` only | INV-ACCT-01 |
| `mint_debt_preserves` | `Cache::mint_debt` | On success `borrowed += s`, nothing else changes, `SharesInv` preserved | `SharesInv`, `s ≥ 0` | INV-ACCT-01 |
| `burn_debt_preserves` | `Cache::burn_debt` | On success `0 ≤ s ≤ borrowed`, `borrowed -= s`, `SharesInv` preserved | `SharesInv` only | INV-ACCT-01 |
| `accrue_revenue_preserves` | `Cache::accrue_revenue` | On success `revenue += s` and `supplied += s` (equally), nothing else, `SharesInv` preserved | `SharesInv`, `s ≥ 0` | INV-ACCT-01 |
| `absorb_supply_as_revenue_preserves` | `Cache::absorb_supply_as_revenue` | On success `revenue += s`, `supplied` unchanged, `SharesInv` preserved | `SharesInv`, `s ≥ 0` | INV-ACCT-01 |
| `burn_claimable_revenue_spec` | `Cache::burn_claimable_revenue` | Never aborts; pays `amt = min(cash, T)` or 0 and burns revenue shares worth at least `amt`; the `InternalError` assert is unreachable (below) | `supply_index > 0`, `0 ≤ revenue ≤ supplied`, `cash ≥ 0`, `dec ≤ 27`, `revenue·supply_index < 2^127·RAY` | INV-ACCT-06, INV-ACCT-01, ADR-0003 |
| `burn_claimable_revenue_preserves` | `Cache::burn_claimable_revenue` | Returns a cache satisfying `SharesInv` | `SharesInv` and the hypotheses above | INV-ACCT-01 |

Details:

* **`burn_claimable_revenue_spec`**: with
  `T = ⌊revenue·supply_index/(RAY·10^k)⌋`, the payout `amt` satisfies:
  * `0 ≤ amt ≤ cash` and `amt ≤ T`;
  * `amt = 0` iff `cash ≤ 0 ∨ T ≤ 0`, and then the cache is unchanged;
  * a positive `amt = min(cash, T)` burns `burned > 0` shares (at most all
    revenue) from `revenue` and `supplied` only;
  * a full claim burns all revenue; a cash-limited one burns
    `⌈revenue·amt/T⌉`;
  * `amt·RAY·10^k ≤ burned·supply_index`: the payout never exceeds the
    burned shares' value.

  **The `scaled_to_burn != Ray::ZERO` `InternalError` assert is
  unreachable.**

Why the `s ≥ 0` hypotheses hold. A mint receives one of three values:

* `calculate_scaled_supply` / `calculate_scaled_borrow` of a validated
  non-negative amount (floor/ceiling of a non-negative quotient);
* `protocol_fee_shares` (proved non-negative in `add_protocol_revenue_spec`);
* a seized position's validated non-negative scaled amount.

The burns need no hypothesis. The checked subtraction and the
`require_revenue_backed` assert reject every input that would break the
invariant.

### Guards (`Guards.lean`, `contracts/pool/src/guards.rs`)

`B = ⌈borrowed·borrow_index/RAY⌉` (ceiled debt value), `S =
⌊supplied·supply_index/RAY⌋` (floored supply value). All guards share two
hypotheses:

* share totals and indexes are non-negative (INV-ACCT-01; indexes start at
  RAY and are floored at `SUPPLY_INDEX_FLOOR_RAW`);
* the RAY values fit `i128` (otherwise `mul_floor`/`mul_ceil` abort, also a
  rejection).

| Lean name | Rust function | Claim | Extra hypotheses | Invariants |
|---|---|---|---|---|
| `require_supply_for_debt_spec` | `guards::require_supply_for_debt` | Passes iff `¬(supplied = 0 ∧ borrowed ≠ 0)` | none | INV-ACCT-09 |
| `require_utilization_below_max_spec` | `guards::require_utilization_below_max` (debt mints) | Passes iff `UtilPass` (`supplied = 0 ∨ B = 0 ∨ B·RAY ≤ max_utilization·S`), at every ceiling including 100% (below) | `B`, `S` fit; `0 ≤ max_utilization ≤ RAY` (`verify`) | INV-ACCT-08, audit S-03 |
| `require_exit_utilization_below_max_spec` | `guards::require_exit_utilization_below_max` (ordinary withdrawals, revenue claims) | Passes iff `max_utilization ≥ RAY ∨ UtilPass`: the same gate below a 100% ceiling, no check at 100% | as above | INV-ACCT-08 |
| `require_exit_utilization_ok` | `guards::require_exit_utilization_below_max` | Returned ⇒ `max_utilization ≥ RAY`, or the full gate returned on the same cache | none | INV-ACCT-08 |
| `max_market_value_spec`, `max_market_entry_spec`, `maxMarketEntry_eq` | `constants::MAX_MARKET_VALUE_RAY`, `MAX_MARKET_ENTRY_RAY` | Evaluate to `MaxMarketValue = 2^127 − 1 − 10^27` and `MaxMarketEntry = MaxMarketValue / 8` (rounded down) | none | INV-IDX-06, INV-ACCT-11 |
| `fits_entry_ceiling_spec` | `guards::fits_entry_ceiling` | Never aborts; `true` iff `EntryFits shares index`: the share count is at most `MaxMarketEntry` and so is its half-up value. A value that does not fit `i128` gives `false`, not an abort | `shares, index ≥ 0` | INV-ACCT-11 |
| `require_within_entry_ceiling_spec` | `guards::require_within_entry_ceiling` (supply, borrow, strategy and flash-loan entries) | Passes iff `EntryFits supplied supply_index ∧ EntryFits borrowed borrow_index` | share totals and indexes `≥ 0` | INV-ACCT-11 |
| `require_within_entry_ceiling_congr` | `guards::require_within_entry_ceiling` | Depends only on the share totals and the indexes: two caches that agree on them get the same answer | none | INV-ACCT-11 |
| `require_liquidation_buffer_spec` | `guards::require_liquidation_buffer` (reserve via `Bps::apply_to_ceil`) | Passes iff `cash − draw ≥ R`, iff `Sa·200 ≤ (cash − draw)·10000` (below) | `cash ≥ 0`, `draw ≥ 0`, `dec ≤ 27` | INV-ACCT-07 |
| `backing_shortfall_spec` | `guards::backing_shortfall` | Never aborts; returns the floored backing shortfall, never over-reported (below) | `cash ≥ 0`, `dec ≤ 27` | INV-ACCT-04 |
| `require_backed_market_spec` | `guards::require_backed_market` | Passes iff `C ≤ Dt ∨ C − Dt < (cash + 1)·10^k` | as above | INV-ACCT-04 |

Details:

* **`require_utilization_below_max_spec`**: at a 100% ceiling `UtilPass`
  reads `B ≤ S`. The code compares `B` with `⌊S·max_utilization/RAY⌋` and
  divides nothing:
  * debt is rounded up, supply and the allowed side down, so rounding cannot
    admit utilization above the ceiling;
  * no quotient can overflow;
  * debt against zero floored supply fails.
* **`require_liquidation_buffer_spec`**: here
  `Sa = ⌊supplied·supply_index/(RAY·10^k)⌋` and `R = ⌈Sa·200/10000⌉`.
* **`backing_shortfall_spec`**: with `C = S` (floored claims) and `Dt = B`
  (ceiled debt):
  * `C ≤ Dt` returns 0;
  * otherwise it returns `max(0, U − cash)` with `U = ⌊(C − Dt)/10^k⌋` (one
    floor to token units);
  * a positive shortfall satisfies `shortfall·10^k ≤ C − Dt − cash·10^k`
    (never over-reported).

### Typed fixed-point wrappers (`Wrappers.lean`, `FloorCeilScaling.lean`; `common/src/math/fp.rs`)

`fp_core` is private. The pool code reaches it only through these `fp.rs`
wrappers. Each spec is total correctness about the generated
wrapper.

| Lean name | Rust function | Claim | Hypotheses | Used by |
|---|---|---|---|---|
| `ray_div_floor_saturating_spec` (in `FloorCeilScaling.lean`) | `Ray::div_floor_saturating` | Never aborts; returns `⌊a·RAY/b⌋` clamped to `[i128::MIN, i128::MAX]` | `b ≠ 0` | `protocol_fee_shares` (`add_protocol_revenue_spec`, `book_fee_spec`), `calculate_scaled_cap_spec` |
| `bps_apply_to_ceil_spec` | `Bps::apply_to_ceil` | Returns exactly `⌈amount·bps/10000⌉` (every sign) | that ceiling fits `i128` | `require_liquidation_buffer_spec` |
| `ray_try_mul_spec` | `Ray::try_mul` | Never aborts; `None` for a negative operand; for `a, b ≥ 0`: `Some(half-up a·b/RAY)` when it fits, `None` exactly when it does not | none | `rates::simulate::market_value` (`accrue_step`), `guards::fits_entry_ceiling` |
| `ray_mul_int_spec` | `Ray::mul_int` | Returns exactly `a·m` | `a·m` fits `i128` | `compound_interest_spec` |
| `ray_mul_int_overflow` | `Ray::mul_int` | Never returns when `a·m` does not fit (the `MathOverflow` panic); with the above, the exact pass condition | `a·m` does not fit | |
| `i128_from_u64_spec` | `i128::from(u64)` (trusted model) | Never aborts; the same integer | none | `compound_interest` |
| `bps_from_same` | `Bps::from::<i128>` | Wraps the value unchanged | none | `require_liquidation_buffer_spec` |

`Ray::try_div` is not in the pool extraction. `utilization` divides with
`Ray::div`, and only when `borrowed < supplied`.
The other `fp.rs` wrappers (`Ray::mul_wad_ratio_floor`, `Wad::*`,
`Bps::{div, from_fraction_floor, from_fraction_ceil, try_from_fraction,
from_weighted_sum, apply_to_ray_ceil}`) are not reached from the pool
extraction roots, so they are not in the pool Lean code.

### Cash (`Cash.lean`, `contracts/pool/src/cache/cash.rs`)

| Lean name | Rust function | Claim | Hypotheses | Invariants |
|---|---|---|---|---|
| `require_nonneg_amount_iff` | `validation::require_nonneg_amount` | Passes iff `amount ≥ 0` | none | INV-ACCT-02 |
| `require_reserves_iff` | `Cache::require_reserves` | Passes iff `amount ≤ cash` | none | INV-ACCT-02 |
| `credit_cash_spec` | `Cache::credit_cash` | Returns iff `amount ≥ 0 ∧ cash + amount < 2^127`; then only `cash` changes, to `cash + amount` | none | INV-ACCT-02 |
| `debit_cash_spec` | `Cache::debit_cash` | Returns iff `0 ≤ amount ≤ cash`; then only `cash` changes, to `cash − amount ≥ 0` | none | INV-ACCT-02 |

### Interest booking (`Interest.lean`, `contracts/pool/src/interest.rs`)

| Lean name | Rust function | Claim | Hypotheses | Invariants |
|---|---|---|---|---|
| `add_protocol_revenue_spec` | `interest::add_protocol_revenue` | Never aborts; mints the fee's floored share count into `revenue` and `supplied`; `SharesInv` preserved (below) | `SharesInv`, `supply_index > 0`, `fee ≥ 0` | INV-ACCT-01, INV-IDX-05 (floor rounding of protocol shares) |
| `apply_bad_debt_spec` | `interest::apply_bad_debt_to_supply_index` | Never aborts; writes the supply index down at least pro rata, not below `SUPPLY_INDEX_FLOOR_RAW` (below) | `supplied ≥ 0`, `supply_index > 0`, `bad ≥ 0`, `V` fits | INV-IDX-03, INV-IDX-02 (lower clamp) |

Details:

* **`add_protocol_revenue_spec`**: a zero fee leaves the cache unchanged.
  Otherwise it mints
  `s = min(⌊fee·RAY/supply_index⌋ (saturating), i128::MAX − supplied) ≥ 0`
  into both `revenue` and `supplied`, and nothing else changes.
* **`apply_bad_debt_spec`**: only `supply_index` changes. Let `V` be the
  half-up `supplied·supply_index/RAY`.
  * `V = 0`: unchanged.
  * Otherwise, with `rem = V − min(bad, V)`, `f = ⌊rem·RAY/V⌋` and
    `n = ⌊supply_index·f/RAY⌋`, the new index is `max(n, RAY/1000)`.
  * So the new index is `≥ SUPPLY_INDEX_FLOOR_RAW` and `≤ max(old, floor)`.
  * `n·V ≤ old·rem`: at least the exact pro-rata write-down.

### Flash loans (`Flash.lean`, `contracts/pool/src/ops/flash.rs`)

| Lean name | Rust function | Claim | Hypotheses | Invariants |
|---|---|---|---|---|
| `flash_loan_fee_on_spec` | `Bps::flash_loan_fee_on` (`common/src/math/fp.rs`) | Never aborts; fee = half-up `amount·bps/10000`, or 1 when the rate is positive and that is 0 | `0 ≤ bps ≤ 10000`, `amount ≥ 0` | INV-FLASH-01 |
| `terms_spec` | `flash::terms` | Never aborts; `0 ≤ F ≤ P`, and the payout and repayment balances add up (below) | `P > 0`, `bps ≤ 10000`, `P ≤ B`, `B + P < 2^127` (below) | INV-FLASH-01 |
| `book_fee_spec` | `flash::book_fee` | Never aborts; `cash += F`, `revenue` and `supplied` both `+= s ≥ 0`, nothing else; `SharesInv` preserved | `SharesInv`, `supply_index > 0`, `F ≥ 0`, `dec < 27`, `F·10^k` and `cash + F` fit | INV-FLASH-01, INV-ACCT-01, INV-ACCT-02 |

Details:

* **`terms_spec`**:
  * `0 ≤ F ≤ P`, and `F > 0` when `bps > 0`;
  * `total_repayment = P + F`, `balance_after_payout = B − P`,
    `balance_after_repayment = B + F`, hence `(B − P) + (P + F) = B + F`.

  Hypotheses: `P > 0` (`require_positive_amount`); `bps ≤ 10000`
  (`flashloan_fee ≤ MAX_FLASHLOAN_FEE_BPS = 500`); `P ≤ B` (the payout comes
  out of `B`); `B + P < 2^127`.

### Compound interest (`Compound.lean`, `common/src/rates/compound.rs`)

| Lean name | Rust function | Claim | Hypotheses | Invariants |
|---|---|---|---|---|
| `compound_interest_spec` | `rates::compound_interest` | Never aborts; `RAY + rate·delta_ms ≤ f ≤ 21·RAY` (below) | `rate ≥ 0`, `rate·delta_ms ≤ 3·RAY` (below) | INV-IDX-01 (factor `≥ RAY`), INV-IDX-04 |
| `ci_step`, `ci_loop_cons`, `ci_loop_nil(_spec)`, `hu_pow_bound` | the Taylor loop | One iteration with bounds; loop unfolding | | |

Details:

* **`compound_interest_spec`**: returns `f` with
  `RAY + rate·delta_ms ≤ f ≤ 21·RAY`, exactly `RAY` for `delta_ms = 0`.
  * Proof: the k-th power stays `≤ 3^k·RAY`. Half-up rounding cannot exceed
    `3B` when the product is below `3B·RAY + RAY/2`.
  * Every term is non-negative, and `4 + Σ_{k=2..8} 3^k/k! < 21`.

  Hypotheses: `rate ≥ 0`, `rate·delta_ms ≤ 3·RAY` (production:
  `≤ 2·RAY + 1.6·10^10`, `compound_x_le`).

### Accrual step (`Accrual.lean`, `common/src/rates/simulate.rs`, `index.rs`)

| Lean name | Rust function | Claim | Hypotheses | Invariants |
|---|---|---|---|---|
| `compound_x_le` | (curve → compound) | The production exponent: `r = half_up(a/31_556_926_000)` with `0 ≤ a ≤ 2·RAY` and `delta ≤ 31_556_926_000` give `0 ≤ r` and `r·delta ≤ 3·RAY` | `a` from `calculate_borrow_rate_spec` | INV-IDX-04 |
| `protocol_fee_shares_spec` | `rates::protocol_fee_shares` | Never aborts; `0 ≤ sh`, `supplied + sh ≤ i128::MAX`, `sh·supply_index ≤ fee·RAY` | `fee ≥ 0`, `supply_index > 0`, `supplied ≥ 0` | INV-IDX-05, INV-ACCT-01 |
| `update_supply_index_strong` | `rates::update_supply_index` | Never aborts; `old ≤ n ≤ 10^36`, and `n = old` or `supplied·n ≤ (V + rewards)·RAY` (the index move distributes at most `rewards`) | `supplied, rewards ≥ 0`, `0 < old ≤ 10^36`, `V + rewards` fits | INV-IDX-02 |
| `supply_index_reward_shortfall_total` | `rates::supply_index_reward_shortfall` | Never aborts when `t ≤ t' ≤ t + rewards < 2^127` (old/new supply values); returns `rewards − (t' − t) ∈ [0, rewards]` | as stated | INV-IDX-05 |
| `market_value_spec` | `rates::simulate::market_value` | Never aborts on non-negative operands; `Some(hu(shares·index, RAY))` when that fits `i128`, `None` exactly when it does not | `shares, index ≥ 0` | INV-IDX-06 |
| `value_room_spec` | `rates::simulate::value_room` | Never aborts; `room = max(0, MAX_MARKET_VALUE_RAY − max(debt_value, supply_value))` | values `≥ 0` | INV-IDX-06 |
| `cap_borrow_index_spec` | `rates::simulate::cap_borrow_index` | Never aborts; `old ≤ n ≤ grown`, and the debt value grows by at most `room` (below) | `borrowed ≥ 0`, `0 ≤ old ≤ grown`, `room ≥ 0` | INV-IDX-06, INV-IDX-01 |
| `accrue_step_spec` | `rates::accrue_step` | **Never aborts, with no hypothesis on the size of the market values.** Indexes bounded; interest split and capped at the room (below) | `ValidRateModel`, totals and indexes in range, `delta ≤ MAX_COMPOUND_DELTA_MS` (below) | INV-IDX-06, INV-IDX-01, INV-IDX-02, INV-IDX-05, audit S-04 |
| `accrue_step_within_ceiling` | (arithmetic consequence of `accrue_step_spec`) | A step keeps both market values at most `MAX_MARKET_VALUE_RAY` (below) | the last two conclusions of `accrue_step_spec` | INV-IDX-06 |
| `hu_scale_le`, `hu_le_of_le`, `hu_lt_of_lt`, `hu_add_le`, `max_compound_delta_val` | | Half-up bounds (`hu_add_le`: `n' ≤ n + r·RAY → hu(n') ≤ hu(n) + r`); `MAX_COMPOUND_DELTA_MS = 31_556_926_000` | | |

Details:

* **`cap_borrow_index_spec`**: the result `n` satisfies `old ≤ n ≤ grown`
  and `borrowed·(n − old) ≤ room·RAY`. The debt value at `n` exceeds its
  value at `old` by at most `room`; the allowed index growth rounds down.
* **`accrue_step_spec`**: `od, nd` are the old/new debt values and
  `t ≤ t'` the old/new supply values.
  * `borrow_index ≤ b' ≤ 10^36` and `supply_index ≤ s' ≤ 10^36`.
  * `0 ≤ revenue_shares` and `supplied + revenue_shares ≤ i128::MAX`.
  * `revenue_shares·s' + (t' − t)·RAY ≤ (nd − od)·RAY`: revenue value plus
    value distributed to suppliers is at most the accrued interest.
  * `nd − od ≤ max(0, MAX_MARKET_VALUE_RAY − max(od, t))`: interest is at
    most the room below the value ceiling. It is zero when a total is at or
    above it, or does not fit `i128`.

  Hypotheses: `ValidRateModel` (incl. `max_borrow_rate ≤ 2·RAY`),
  `borrowed, supplied ≥ 0`, `0 ≤ borrow_index ≤ 10^36`,
  `0 < supply_index ≤ 10^36`, `delta ≤ MAX_COMPOUND_DELTA_MS`.
* **`accrue_step_within_ceiling`**: if the old debt value and the old
  supply value are at most `MAX_MARKET_VALUE_RAY`, so are the new debt value
  and the supply value after the step's revenue shares are added at the new
  index.

`accrue_step_spec` has no hypothesis that a market value fits `i128`
(audit S-04). The step values both totals with `Ray::try_mul`. It caps the
borrow index at the room below `MAX_MARKET_VALUE_RAY`. Every later product is
then bounded by an old value plus that room, and the proof uses this bound.
The only abort paths are the sign checks in `market_value`. INV-ACCT-01 and
the index bounds exclude them.

### Chunk and `global_sync` (`GlobalSync.lean`, `contracts/pool/src/interest.rs`)

`AccrualDomain c` is the hypothesis list of `accrue_step_spec` on the cache
fields plus `SharesInv c`: a validated rate model, INV-ACCT-01 and indexes in
range. It has no hypothesis on the size of the market values. `ChunkPost c c'`: `SharesInv c'`,
both indexes monotone and `≤ 10^36`, `revenue` and `supplied` grow by the same
`sh ≥ 0`, no other field changes. `ChunkPost` is reflexive on `AccrualDomain`
and transitive. A chunk keeps the cache in `AccrualDomain`
(`ChunkPost.refl`, `ChunkPost.trans`, `AccrualDomain.of_chunk`). So it is a
loop invariant for any number of chunks.

| Lean name | Rust function | Claim | Hypotheses | Invariants |
|---|---|---|---|---|
| `accrue_chunk_spec` | `interest::accrue_chunk` | Never aborts; `ChunkPost c c'` (the returned flag says whether the value ceiling held the chunk's interest) | `AccrualDomain c`, `delta_ms ≤ MAX_COMPOUND_DELTA_MS` | INV-IDX-01, INV-IDX-02, INV-IDX-06, INV-ACCT-01 |
| `global_sync_loop_spec` | `global_sync`'s loop | Induction principle over chunks of at most one year; the loop terminates (below) | the step hypothesis | INV-IDX-04 |
| `global_sync_noop` | `interest::global_sync` | `current ≤ last` (no elapsed time): returns the cache unchanged, no timestamp advance | none | INV-IDX-04 |
| `global_sync_spec_of_inv` | `interest::global_sync` | With `e = current − last`: given a chunk invariant with `Inv e c`, never aborts; `e = 0` → unchanged; else the result is `{x with last_timestamp := current}` for an `x` with `Inv 0 x` | as `global_sync_loop_spec` | INV-IDX-04 |
| `global_sync_spec` | `interest::global_sync` | **Never aborts, for any elapsed time.** Result `x` with `ChunkPost c x`, `AccrualDomain x` (below) | `AccrualDomain c` | INV-IDX-06, INV-IDX-01, INV-IDX-02, INV-IDX-04, INV-ACCT-01, audit S-04 |
| `min_u64_val`, `sat_sub_u64_val`, `elapsed_val` | `u64::min`, `u64::saturating_sub`, `Cache::elapsed_ms` | Value lemmas | | |

Details:

* **`global_sync_loop_spec`**: take any `Inv remaining cache` preserved by
  every chunk `k = min(r, MAX_COMPOUND_DELTA_MS)` with `r > 0` (from `Inv r`
  to `Inv (r − k)`). It holds at `remaining = 0`, and the loop terminates.
  So chunks are positive, at most one year, all but the last full, and sum
  to the elapsed time.
* **`global_sync_spec`**: `e = 0` leaves the cache unchanged. Otherwise:
  * the elapsed time is accrued in chunks of at most one year;
  * the result `x` satisfies `ChunkPost c x` and `AccrualDomain x`;
  * `last_timestamp = current_timestamp`;
  * the value-ceiling event, when published, changes nothing in the cache.

INV-IDX-04's "mutating accrual and read-only projection use the same
calculation" holds by construction (both call `accrue_step`); the projection
`simulate_update_indexes` is not extracted.

### Operation legs (`Legs.lean`, `Legs2.lean`, `contracts/pool/src/ops/`)

Shape. A leg loads and accrues its market (`ops::synced_market` =
`Cache::load` + `global_sync`), mutates the cache, and commits it. The leg
theorems name the synced cache by the hypothesis `ops.synced_market env key
= ok c` and assume `PoolInv c` on it. They then describe **every successful
return** of the leg (`leg = ok r → …`). That is the form a Certora summary
needs: a summary constrains the values a call returns, and an aborting call
reverts the transaction. The leg's own guards (`require_*`, checked
arithmetic, the `*RoundsToZeroShares` asserts) are facts the theorems
derive, not hypotheses. Fit hypotheses ("`amount·10^k < 2^127`", "the RAY
value of the position fits") are the domains on which the conversion
helpers are proved exact.

`PoolInv c` (stored-state invariant): `0 ≤ revenue ≤ supplied`,
`0 ≤ borrowed` (INV-ACCT-01), `cash ≥ 0` (INV-ACCT-02),
`10^24 ≤ supply_index ≤ 10^36`, `10^27 ≤ borrow_index ≤ 10^36`.
`ParamsOk c`: `asset_decimals ≤ 18`, `flashloan_fee ≤ 500` (established by
`MarketParamsRaw::verify` at creation / rate-model replacement, not
extracted; no leg changes params). `snapOf c` is the snapshot `commit`
returns, `indexOf c` the returned index pair. `k = 27 − decimals`.

| Lean name | Rust function | Claim (on every return) | Hypotheses | Invariants |
|---|---|---|---|---|
| `commit_eq` | `Cache::commit` | Never aborts; returns `snapOf c` | none | |
| `load_leg_ok` | `ops::load_leg` | Amount and position were `≥ 0`, the market synced to the returned cache, the returned position is the action's | none | INV-ACCT-05 (validation) |
| `renewed_market_ok` | `ops::renewed_market` | Returned `c` ⇒ `synced_market` returned `c` | none | |
| `market_index_eq`, `position_mutation_eq`, `strategy_mutation_eq` | `Cache::{market_index, position_mutation, strategy_mutation}` | The DTOs carry the given position/amounts, `indexOf c` and the cache's decimals | none | |
| `nonneg_of_ok`, `positive_of_ok`, `massert_ok`, `sub_unwrap_eq_ok`, `ceil_le_of_index_ge_ray` | validation helpers, `checked_sub().unwrap_or_else(panic)` | Returned ⇒ the checked condition; the checked difference is exact; `⌈A·RAY/bi⌉ ≤ A` for `bi ≥ RAY` | as stated | |
| `supply_apply_spec` | `ops::supply::apply` | Mints `m = ⌊amount·10^k·RAY/si⌋`; books `supplied += m`, `cash += amount`; `PoolInv`; entry ceiling passed (below) | `PoolInv c`, `dec ≤ 18`, `amount·10^k < 2^127`, floor fits | INV-ACCT-01/02/03/05/10/11, ADR-0003 |
| `mint_debt_ok` | `ops::borrow::mint_debt` | Mints `m = ⌈amount·10^k·RAY/bi⌉ > 0` after the cash, buffer, utilization and entry-ceiling guards (below) | `PoolInv c`, `dec ≤ 18`, `amount·10^k < 2^127` | INV-ACCT-05/07/08/11 |
| `borrow_accounting_spec` | `ops::borrow::accounting` | As `mint_debt_ok`, plus `actual_amount = amount`, committed cache = `c` with `borrowed += m`, `cash -= amount` only, `PoolInv`; the committed cache passes the entry-ceiling guard | as above | INV-ACCT-01/02/05/07/08/10/11 |
| `repay_accounting_spec` | `ops::repay::accounting` | Full or partial repay with exact `net`, `overpayment` and burn; `PoolInv` (below) | `PoolInv c`, `dec ≤ 18`, `⌈position·bi/RAY⌉` fits | INV-ACCT-01/02/03/05/10, ADR-0003 |
| `withhold_liquidation_fee_ok` | `withdraw::withhold_liquidation_fee` | `net = gross − fee`; the fee is booked as revenue shares worth at most the fee (below) | `SharesInv`, `si > 0`, `fee ≥ 0`, `dec < 27`, `gross·10^k < 2^127` | INV-ACCT-01 |
| `gate_and_debit_ok` | `withdraw::gate_and_debit` | `0 ≤ net ≤ cash`; the exit utilization guard passed unless skipped; not `supplied = 0 ∧ borrowed ≠ 0`; cache `cash -= net` only | none | INV-ACCT-02/08/09 |
| `withdraw_accounting_spec` | `ops::withdraw::accounting` | Full or partial withdrawal with exact gross, burn and fee shares; `PoolInv` (below) | `PoolInv c`, `dec ≤ 18`, `hu(position·si, RAY)` fits (the exact abort condition of `resolve_withdrawal`) | INV-ACCT-01/02/05/09/10, ADR-0003 |
| `net_settle_apply_spec` | `ops::net_settle::apply` | Burns both sides or neither, in the protocol's favour; no cash moves; `PoolInv` (below) | `PoolInv c`, `dec ≤ 18`, `⌊ss·si/RAY⌋`, `⌈ds·bi/RAY⌉` fit | INV-ACCT-01/02/05/09/10, ADR-0003 |
| `seize_apply_spec` | `ops::seize::apply` | Deposit side: `revenue += p`; borrow side: `borrowed -= p`, supply index written down; `PoolInv` (below) | `PoolInv c`; borrow side: `hu(supplied·si, RAY)` fits | INV-ACCT-01/10, INV-IDX-03 |
| `backing_shortfall_nonneg` | `guards::backing_shortfall` | Returned ⇒ `≥ 0` | none | INV-ACCT-04 |
| `recapitalize_accounting_spec` | `ops::recapitalize::accounting` | `applied = min(amount, sf)`, `refund = amount − applied`, only `cash += applied`; `PoolInv` (below) | `PoolInv c` | INV-ACCT-02/03/04 |
| `compute_fee_ok` | `strategy::compute_fee` | `0 ≤ fee ≤ amount`; 0 without `charge_fee`; otherwise the flash-loan fee | `amount ≥ 0`, `flashloan_fee ≤ 10000` | INV-FLASH-01 |
| `strategy_accounting_spec` | `ops::strategy::accounting` | Mints debt `m > 0`, books the fee as revenue shares, `cash -= amount − fee`; `PoolInv`; entry ceiling passed (below) | `PoolInv c`, `ParamsOk c`, `amount·10^k < 2^127` | INV-ACCT-01/02/05/07/10/11 |
| `revenue_accounting_spec` | `ops::revenue::accounting` | Pays `0 ≤ amt ≤ cash` after the exit utilization guard; `PoolInv` (below) | `PoolInv c`, `dec ≤ 27`, `revenue·si < 2^127·RAY` | INV-ACCT-01/02/06/08/09 |
| `load_ok` | `Cache::load` | The loaded share totals, indexes, cash and timestamp are the stored ones | `storedState k = ok st` | |
| `synced_market_spec` | `ops::synced_market` | **Never aborts, for any elapsed time and any size of the market values**; `PoolInv` preserved (below) | load returned `c0`, `PoolInv c0`, `ValidRateModel c0.params` | INV-IDX-06, INV-IDX-01/02/04, INV-ACCT-01/02, audit S-04 |
| `AccrualDomain.of_poolInv` | | `PoolInv c` with a validated rate model gives `AccrualDomain c` | as stated | |

Details:

* **`supply_apply_spec`**:
  * `amount, position ≥ 0`;
  * minted `m = ⌊amount·10^k·RAY/si⌋ ≥ 0` (floor); `m > 0` if
    `amount > 0`, `m = 0` if `amount = 0`;
  * new position `= position + m`; `actual_amount = amount`;
  * committed cache = `c` with `supplied += m`, `cash += amount` only;
  * `PoolInv`, and the committed cache passes the entry-ceiling guard.
* **`mint_debt_ok`**:
  * `amount > 0` and `amount ≤ cash`;
  * the liquidation-buffer guard passed on the gross amount;
  * minted `m = ⌈amount·10^k·RAY/bi⌉ > 0` and `m ≤ amount·10^k`;
  * position `+= m`; cache `borrowed += m` only;
  * the utilization guard (every ceiling, 100% included) and the
    entry-ceiling guard passed after.
* **`repay_accounting_spec`**: with `D = ⌈position·bi/(RAY·10^k)⌉`:
  * `0 ≤ net ≤ amount` and `overpayment = amount − net`;
  * full (`D ≤ amount`): new `= 0`, `net = D`;
  * partial: `net = amount`, burn `= ⌊amount·10^k·RAY/bi⌋`;
  * `amount > 0 ∧ position > 0 → new < position`, otherwise
    `new = position`;
  * cache `borrowed -= burn`, `cash += net` only; `PoolInv`.
* **`withhold_liquidation_fee_ok`**:
  * no fee-bearing liquidation: `net = gross`, cache unchanged;
  * otherwise `fee ≤ gross` and `net = gross − fee`. The fee is booked as
    `s ≥ 0` shares into `revenue` and `supplied`, worth at most the fee:
    `s·supply_index ≤ fee·10^k·RAY`.
* **`withdraw_accounting_spec`**: `H` is the half-up position value, `F` its
  floor value and `G` the gross amount.
  * `0 ≤ G ≤ F`, `G ≤ amount`, `0 ≤ new ≤ position`.
  * Full (`H ≤ amount`): `new = 0`, `G = F`.
  * Partial: `G = amount`, burn `= ⌈amount·10^k·RAY/si⌉`.
  * `amount = 0 ∧ H > 0 → new = position`; `G > 0 → new < position`;
    `amount > 0 ∧ position > 0 → new < position`; `position = 0 → new = 0`.
  * `net = G` or `G − fee` (fee-bearing liquidation), `0 ≤ net ≤ G`.
  * Cache: `supplied := supplied − burn + s`, `revenue += s`, `cash -= net`
    only. `s = 0` without fee, and always `s ≤ burn`: the fee shares never
    exceed the shares burned.
  * `PoolInv`, and not `supplied = 0 ∧ borrowed ≠ 0` after.
* **`net_settle_apply_spec`**:
  * `0 ≤ settled ≤ amount` (and `≤ F`, `≤ D` when positive);
  * `0 ≤ ns ≤ ss`, `0 ≤ nd ≤ ds`;
  * `settled > 0` burns both sides, `settled = 0` burns neither;
  * full close only when exhausted; partial burns favour the protocol;
  * cache `supplied -= ss − ns`, `borrowed -= ds − nd` only (no cash);
  * `PoolInv`, and not `supplied = 0 ∧ borrowed ≠ 0`.
* **`seize_apply_spec`**:
  * position `≥ 0`; the committed cache satisfies `PoolInv`; cash unchanged;
  * deposit side: `revenue += p` only;
  * borrow side: `borrowed -= p` and the supply index written down
    (`≥ 10^24`, `≤ old`), nothing else.
* **`recapitalize_accounting_spec`**:
  * `amount ≥ 0`;
  * `applied = min(amount, sf)` for the returned shortfall `sf ≥ 0`, so
    `0 ≤ applied ≤ amount`; `refund = amount − applied`;
  * cache `cash += applied` only (no shares, no index move); `PoolInv`.
* **`strategy_accounting_spec`**:
  * `amount > 0`; the buffer guard passed on the gross amount;
  * `0 ≤ fee ≤ amount`; minted `m > 0` (ceiling); position `+= m`;
  * `actual_amount = amount`, `amount_received = amount − fee`;
  * cache `borrowed += m`, `revenue` and `supplied += s ≥ 0` (`0` for zero
    fee), `cash -= amount − fee` only;
  * `PoolInv`; the committed cache passes the entry-ceiling guard, checked
    after the mint and again after the fee shares.
* **`revenue_accounting_spec`**:
  * `burn_claimable_revenue` gave `(amt, c1)` with `ClaimResult c T amt c1`;
  * `0 ≤ amt ≤ cash`; the exit utilization guard passed on `c1`;
  * committed cache = `c1` with `cash -= amt`;
  * `PoolInv`, and not `supplied = 0 ∧ borrowed ≠ 0`.
* **`synced_market_spec`**: also keeps cash and params unchanged, and the
  indexes do not decrease.

INV-ACCT-10 (pool side). The invariant is "Σ account supply shares =
supplied − revenue, Σ account debt shares = borrowed". The pool keeps no
per-account book. So the invariant holds if every leg moves the pool totals
by exactly the change of the account position it returns. The leg theorems
state exactly that:

* supply: `Δsupplied = Δposition`;
* borrow/strategy: `Δborrowed = Δposition` (fee shares go to `revenue` and
  `supplied` equally);
* repay: `Δborrowed = Δposition`;
* withdraw: `Δ(supplied − revenue) = Δposition`;
* net settle: the same on both sides;
* seize deposit: `Δ(supplied − revenue) = −p`; seize borrow:
  `Δborrowed = −p` (the controller then removes the seized position);
* accrual moves `revenue` and `supplied` equally (`ChunkPost`).

The controller half (it writes the returned position) is not covered here.

INV-ACCT-03 (pool side). The pool credits exactly the amount the
controller reports as received (supply: `cash += amount`; repay: `cash +=
net`, refund `= amount − net`; recapitalize: `cash += applied`, refund `=
amount − applied`). The measurement of the receipt (balance before/after)
is in the controller and the token, outside this model.

### The market value ceiling as a stored-state invariant (`ValueCeiling.lean`)

`ValueCeil c` (INV-IDX-06): the half-up value of total supply at the supply
index and of total debt at the borrow index are both at most
`MAX_MARKET_VALUE_RAY`. Every extracted leg keeps it, so no stored state
reached through these legs has a market total above the ceiling. Each exit
theorem takes the hypotheses of its leg theorem plus `ValueCeil c` on the
synced cache. The entry theorems and the flash-loan theorem need no ceiling
hypothesis.

| Lean name | Rust function | Claim | Why | Invariants |
|---|---|---|---|---|
| `accrue_chunk_keeps_ceiling` | `interest::accrue_chunk` | Never aborts; `ChunkPost` and `ValueCeil` of the result | `accrue_step_spec` caps interest at the room below the ceiling (`accrue_step_within_ceiling`) | INV-IDX-06 |
| `global_sync_keeps_ceiling` | `interest::global_sync` | Never aborts; `ValueCeil` of the result, **for any elapsed time** | chunk invariant | INV-IDX-06 |
| `synced_market_keeps_ceiling` | `ops::synced_market` | Never aborts; `ValueCeil` of the synced cache every leg starts from | load + `global_sync` | INV-IDX-06 |
| `supply_keeps_ceiling`, `borrow_keeps_ceiling`, `strategy_keeps_ceiling` | `ops::supply::apply`, `ops::borrow::accounting`, `ops::strategy::accounting` | `ValueCeil` of the committed cache, with no ceiling hypothesis | the committed cache passes the entry-ceiling guard (below) | INV-ACCT-11 |
| `repay_keeps_ceiling` | `ops::repay::accounting` | `ValueCeil` of the committed cache | debt shares only fall | INV-IDX-06 |
| `withdraw_keeps_ceiling` | `ops::withdraw::accounting` | `ValueCeil` of the committed cache, with or without a liquidation fee | fee shares minted `≤` supply shares burned | INV-IDX-06 |
| `net_settle_keeps_ceiling` | `ops::net_settle::apply` | `ValueCeil` of the committed cache | supply and debt shares only fall | INV-IDX-06 |
| `seize_keeps_ceiling` | `ops::seize::apply` | `ValueCeil` of the committed cache, both sides | deposit side reclassifies shares; borrow side burns debt and lowers the supply index | INV-IDX-06, INV-IDX-03 |
| `recapitalize_keeps_ceiling` | `ops::recapitalize::accounting` | `ValueCeil` of the committed cache | only cash changes | INV-IDX-06 |
| `revenue_keeps_ceiling` | `ops::revenue::accounting` | `ValueCeil` of the committed cache | revenue and supply shares only fall | INV-IDX-06 |
| `flash_finalize_keeps_ceiling` | `ops::flash::finalize` | `PoolInv` and `ValueCeil` of the returned (committed) cache, with no ceiling hypothesis; `cash += fee` | the fee is booked (`book_fee_spec`), then the cache passes the entry-ceiling guard | INV-ACCT-11, INV-FLASH-01 |
| `ValueWithin.mono`, `ValueWithin.of_entryFits`, `ValueCeil.of_le`, `ValueCeil.of_entry_guard` | | A smaller product has a smaller half-up value; the entry ceiling is below the value ceiling | | |

Details:

* **`supply_keeps_ceiling`, `borrow_keeps_ceiling`,
  `strategy_keeps_ceiling`**: they need no ceiling hypothesis, because
  `MAX_MARKET_ENTRY_RAY ≤ MAX_MARKET_VALUE_RAY` (`ValueCeil.of_entry_guard`).

`flash_finalize_keeps_ceiling` takes the hypotheses of `book_fee_spec`
(`fee ≥ 0`, `decimals < 27`, `fee·10^k` and `cash + fee` fit `i128`).
`finalize` runs after `flash::apply` has checked the token balances; that
part (callback, balance measurement, transfers) is not extracted.

Not covered by these: market creation, which writes zero totals.

**The ceiling discharges the fit hypotheses.** The leg theorems assume that
the values they convert fit `i128`. Those assumptions follow from
`ValueCeil`:

| Lean name | Claim | Discharges |
|---|---|---|
| `ValueCeil.supply_value_fits` | Within the ceiling, the half-up value of total supply is below `2^127` | the fit hypothesis of `seize_apply_spec`; `seize_keeps_ceiling` does not take it |
| `ValueCeil.revenue_fits` | Within the ceiling and with `revenue ≤ supplied`, `revenue·supply_index < 2^127·RAY` | the fit hypothesis of `revenue_accounting_spec`; `revenue_keeps_ceiling` does not take it |
| `ValueWithin.fits_half_up`, `ValueWithin.fits_floor`, `ValueWithin.fits_ceil` | For `p ≤ total` and `index ≥ 0`, the half-up, floored and ceiled values of `p` fit `i128` (below) | the position fit hypotheses, given "position ≤ market total" |
| `isFloor_le_halfUp`, `isCeil_le_halfUp_succ` | `⌊n/RAY⌋ ≤ hu(n/RAY)` and `⌈n/RAY⌉ ≤ hu(n/RAY) + 1` | |
| `withdraw_keeps_ceiling_of_position_le` | `withdraw_keeps_ceiling` with `position ≤ supplied` in place of the fit hypothesis | |
| `repay_keeps_ceiling_of_position_le` | `repay_keeps_ceiling` with `position ≤ borrowed` in place of the fit hypothesis | |
| `net_settle_keeps_ceiling_of_position_le` | `net_settle_keeps_ceiling` with `supply position ≤ supplied` and `debt position ≤ borrowed` in place of both fit hypotheses | |

Details:

* **`ValueWithin.fits_half_up`, `ValueWithin.fits_floor`,
  `ValueWithin.fits_ceil`**: the value of `p` at the index is below `2^127`.
  The ceiled value is at most the half-up value plus one, and the ceiling
  leaves one whole token below `i128::MAX`.

So the ceiling discharges the total-based fit assumptions (seize, revenue
claim). The position-based ones (withdraw, repay, net settlement) reduce to
one statement: **the caller's position is at most its market's total**. The
pool keeps no per-account book, so it cannot prove that statement. It is the
pool-side shape of INV-ACCT-10, which the controller maintains. The entry
legs keep their hypotheses on the *amount*: `amount·10^k` fits, and for
supply the minted share count fits. These are about the call's input, not
stored state.

### Certora summaries in their own terms (`Summaries.lean`)

`IndexInRange m`: `10^24 ≤ m.supply_index ≤ 10^36`, `10^27 ≤
m.borrow_index ≤ 10^36` (what `nondet_market_index_raw` assumes);
`indexOf_inRange`: `PoolInv c → IndexInRange (indexOf c)`.

| Lean name | Summary | Claim (every return of the leg) |
|---|---|---|
| `supply_summary_holds` | `supply_summary` | `amount > 0 → new > old`, else `new = old`; `actual_amount = amount`; `IndexInRange`; decimals `≤ 18` |
| `borrow_summary_holds` | `borrow_summary` | `amount > 0` and `new > old`; `actual_amount = amount`; `IndexInRange`; decimals `≤ 18` |
| `withdraw_summary_partial` | `withdraw_summary` | `new ≥ 0`; `amount > 0 ∧ position > 0 → new < position`; `position ≤ 0 → new = position`; `0 ≤ actual ≤ amount`; `IndexInRange`; decimals `≤ 18` |
| `withdraw_zero_amount_closes_dust`, `withdraw_dust_instance` | `withdraw_summary` (gap) | `amount = 0` with half-up value `H = 0` closes the position (`new = 0`, `actual = 0`); instance: decimals 0, `si = RAY`, position 1 |
| `repay_summary_holds` | `repay_summary` | Exactly the summary's position clause; `0 ≤ actual ≤ amount`; `IndexInRange`; decimals `≤ 18` |
| `net_settle_summary_holds` | `net_settle_summary` | Exactly the summary's clauses; `IndexInRange` |
| `recapitalize_summary_holds` | `recapitalize_summary` | `0 ≤ actual ≤ amount` |
| `create_strategy_summary_holds` | `create_strategy_summary` | `new ≥ old`, `amount ≥ 0`, fee `f ∈ [0, amount]` (`0` without `charge_fee`), `actual = amount`, `received = amount − f`; `IndexInRange`; decimals `≤ 18` |
| `claim_revenue_summary_holds` | `claim_revenue_summary` | `0 ≤ actual ≤ cash` |

### Supporting lemmas (`Basic.lean`)

* `Ray` comparisons (`ray_le_eq`, `ray_min_eq`, ...);
* the checked `Ray` addition/subtraction (`ray_checked_add_spec`,
  `ray_checked_sub_eq_ok`, ...);
* `sat_sub_val` (`i128::saturating_sub` is subtraction when it fits);
* `isFloor_ediv`/`isCeil_exists`/`floor_fits`;
* `mul_div_ceil_eq_ok`: whatever `fp_core::mul_div_ceil` returns is the
  exact ceiling. This is partial correctness with no fit hypothesis; an
  unrepresentable ceiling aborts. `seize_apply_spec` uses it.

### Copied math (from RatesAeneas)

The scaling, rounding, resolve and accrual specs the legs use
(`resolve_withdrawal_spec`, `calculate_borrow_rate_spec`, `utilization_spec`,
...) are copies of RatesAeneas proofs. The list of copies and their
pool-local differences is in
[extraction.md](extraction.md#pool-copied-proofs).

### Certora summaries (`certora/shared/summaries/pool.rs`): discharged / partial / not

The table shows what these theorems discharge of each of the 13 summaries,
plus the shared index generator.

"Discharged" means: every successful return of the production leg satisfies
every postcondition the summary assumes on the returned values. This holds
on a market whose synced state satisfies `PoolInv` (and `ParamsOk` where
decimals or the fee rate are used), within the stated fit domains.

In every row:

* the token transfers, events and storage I/O around the leg are **not**
  covered (shimmed);
* the conditions on the synced market are hypotheses. Their inductiveness is
  shown separately: `PoolInv` by every leg and `synced_market_spec`;
  `ParamsOk` only by the unextracted `MarketParamsRaw::verify`.

| Summary | Status | Theorems | What is not discharged |
|---|---|---|---|
| `nondet_market_index_raw` | Discharged | `indexOf_inRange`; every leg's `PoolInv` of its committed cache; `synced_market_spec` (any elapsed time), `apply_bad_debt_spec`, `accrue_chunk_spec` | |
| `supply_summary` | Discharged | `supply_summary_holds`, `supply_apply_spec` | Decimals `≥ 0` is by type (`u32`), `≤ 18` from `ParamsOk` |
| `borrow_summary` | Discharged | `borrow_summary_holds`, `borrow_accounting_spec` | The outbound transfer of `actual_amount` (`borrow::apply`) |
| `withdraw_summary` | **Partial** | `withdraw_summary_partial`, `withdraw_accounting_spec`, `withdraw_zero_amount_closes_dust` | **The summary's `amount = 0 → new = position` is false** for dust positions (Findings 1). All other clauses hold. Transfer of `net_transfer` not covered |
| `repay_summary` | Discharged | `repay_summary_holds`, `repay_accounting_spec` | Refund transfer of `overpayment` |
| `net_settle_summary` | Discharged | `net_settle_summary_holds`, `net_settle_apply_spec` | |
| `update_indexes_summary` (empty) | Discharged | `synced_market_spec`, `global_sync_spec` | `ops::market::accrue` (loop over keys, events) not extracted |
| `recapitalize_summary` | Discharged | `recapitalize_summary_holds`, `recapitalize_accounting_spec` | Refund transfer |
| `flash_loan_summary` | Discharged (fee arithmetic) | `terms_spec`, `book_fee_spec`, `flash_finalize_keeps_ceiling` | `flash::apply` (callback, balance measurement, transfers) not extracted |
| `create_strategy_summary` | Discharged | `create_strategy_summary_holds`, `strategy_accounting_spec` | Transfer of `amount_received`, strategy-fee event |
| `seize_positions_summary` (empty) | Discharged per entry | `seize_apply_spec` | The batch (`run_batch`, `unique_markets = false`) and events not extracted; that the summary has no return value means nothing more is assumed |
| `claim_revenue_summary` | Discharged | `claim_revenue_summary_holds`, `revenue_accounting_spec` | Owner lookup and payout transfer (`revenue::apply`) |
| `get_sync_data_summary` | **Partial** | `PoolInv` preserved by every leg + `load_ok` + `synced_market_spec` | The params clauses come from unextracted `verify` calls (below) |

Details:

* **`get_sync_data_summary`**:
  * The state clauses (`supplied, borrowed, revenue, cash ≥ 0`, index
    ranges) are the inductive `PoolInv`.
  * The rate-model clauses, `reserve_factor < BPS`, `flashloan_fee ≤ 500`
    and the decimals range come from `MarketParamsRaw::verify` /
    `InterestRateModel::verify`. These run at market creation and rate-model
    replacement (`ops::market`, not extracted).
  * No extracted leg writes params.
  * `get_sync_data` itself (a storage read in `lib.rs`) is not extracted.

## Axiom check

`PoolAeneas.lean` ends with `#assert_standard_axioms PoolAeneas`
(`lean/Audit.lean`). To rebuild, see
[extraction.md](extraction.md#pool-rebuild). `lake build PoolAeneas` fails unless every declaration
of the library depends only on `propext`, `Classical.choice` and
`Quot.sound` (so no `sorryAx`). The uninterpreted `storedParams`,
`storedState` and `ledgerNowMs` are Lean `opaque` definitions, not axioms,
and do not appear.

## Not covered

* **"Position ≤ market total" (INV-ACCT-10).** The withdraw, repay and
  net-settlement theorems assume that the value of the caller's position
  fits `i128`. With `ValueCeil` that follows from "the position is at most
  its market's total" (`*_keeps_ceiling_of_position_le`). That statement
  itself is not proved. The pool keeps no per-account book, and the
  controller, which does, is a separate model. It stays a hypothesis.
* **The flash loan before `finalize`.** `ops::flash::apply` (payout, the
  receiver callback, the two balance checks) is not extracted. So no proof
  here shows that `finalize` is reached only with the fee actually
  received.
* **Which accruals publish the ceiling event.** The `value_capped` flag is
  carried through the loop but no theorem relates it to the interest held.
* **`compound_interest` accuracy.** Only the bounds `RAY + x ≤ f ≤ 21·RAY`
  are proved, not the documented truncation error against `e^x`.
* **`simulate_update_indexes`** (the read-only projection) is not extracted;
  that it runs the same `accrue_step` is by construction.
* **Storage as state, events, token transfers and the flash-loan
  callback.** Storage reads and ledger time are uninterpreted: a read is not
  tied to an earlier write. So the leg theorems take the synced cache and
  `PoolInv` on it as hypotheses. Inductiveness is the chain "leg commits a
  `PoolInv` cache → `load_ok` → `synced_market_spec`". These are not
  extracted:
  * the `apply` wrappers (token transfers);
  * `run_batch` (the batch driver, its `unique_markets` check and batch
    events);
  * `ops::market` (create, rate-model replacement, `accrue`);
  * `ops::flash::apply` and the views.
* **Non-abort of the legs.** The leg theorems are about every successful
  return, which is what the summaries need. When a leg aborts (a guard
  fails) is not characterised beyond the guard specs (`Guards.lean`).
* **The controller half of INV-ACCT-10 and INV-ACCT-03** (writing the
  returned position; measuring the received balance).
* **`Cache::transfer_out`.** Excluded from extraction (token client).
* **Error codes.** Every abort is "failure"; which `CollateralError` /
  `GenericError` code is raised is not modelled.

## Audit findings S-03 and S-04

The Runtime Verification audit reported two pool findings. Both fixes are
in the extracted code, and the theorems below state what is proved about
them.

**Audit S-03 (utilization at a 100% ceiling).**

* `utilization_spec`: `utilization` never aborts on non-negative totals.
  Its result is in `[0, RAY]`. It returns `RAY` when
  `borrowed ≥ supplied > 0`, and divides only when `borrowed < supplied`.
* `require_utilization_below_max_spec`: the debt-mint guard passes exactly
  on `UtilPass` (`supplied = 0 ∨ B = 0 ∨ B·RAY ≤ max_utilization·S`). This
  holds at every ceiling, 100% included, where the condition reads `B ≤ S`.
  Hypotheses: share totals and indexes non-negative,
  `0 ≤ max_utilization ≤ RAY`, and `B`, `S` fit `i128`.
* `require_exit_utilization_below_max_spec`: the exit guard passes exactly
  on `max_utilization ≥ RAY ∨ UtilPass`, under the same hypotheses.
* `mint_debt_ok` and `borrow_accounting_spec` derive that the utilization
  guard passed after the mint.

**Audit S-04 (accrual overflow).**

* `accrue_step_spec`: one accrual step never aborts, with no hypothesis on
  the size of the market values. The accrued interest is at most the room
  below `MAX_MARKET_VALUE_RAY`. Hypotheses: `ValidRateModel`, non-negative
  share totals, indexes in range, and `delta ≤ MAX_COMPOUND_DELTA_MS`.
* `accrue_step_within_ceiling`: a step keeps both market totals at or below
  `MAX_MARKET_VALUE_RAY` when they start there.
* `global_sync_spec`: `global_sync` never aborts for any elapsed time on a
  cache in `AccrualDomain`. It keeps the cache in `AccrualDomain`.
* `synced_market_spec`: load plus accrual never aborts and keeps `PoolInv`,
  given a validated rate model.
* `ValueCeiling.lean`: every extracted leg keeps `ValueCeil`. The withdraw,
  repay and net-settlement theorems take "position ≤ market total" as a
  hypothesis (see "Not covered").

## Findings about the production code

1. **`withdraw_summary` is stricter than the code for zero-amount
   withdrawals of dust positions** (summary unsound in that corner, no
   economic impact). The summary assumes `amount = 0 → new_scaled =
   position`. The code compares the request with the position's half-up
   value `H = hu(hu(pos·si, RAY), 10^k)` and takes the full-close branch when
   `H ≤ amount`. For `amount = 0` and a position with `H = 0` it burns the
   whole position and pays `F = 0`. Concrete case (`withdraw_dust_instance`,
   `withdraw_zero_amount_closes_dust`): an asset with 0 decimals, supply
   index `RAY`, position of 1 scaled unit: `hu(1·RAY, RAY) = 1`,
   `hu(1, 10^27) = 0 = H ≤ 0`, so `burned = 1`, `gross = 0`, the
   `WithdrawRoundsToZeroShares` assert is skipped (gross is 0), and the
   returned position is 0, not 1. The position is worth `10^-27` units, so
   nothing of value moves. The user-facing `withdraw` maps `amount = 0` to
   `WITHDRAW_ALL_SENTINEL` (`contracts/controller/src/positions/supply.rs`).
   So the case is reachable only through controller paths that pass a
   literal 0 (`execute_withdrawal` callers, not checked here). Fix for the summary:
   in the else-branch allow `new_scaled = 0` when `amount == 0`
   (or `0 ≤ new_scaled ≤ position`).

No other counterexample was found. The proofs required the precisions
below:

* `burn_claimable_revenue` returns `0 > cash` if `cash` is negative. The claim
  `amount ≤ cash` therefore needs `cash ≥ 0` (INV-ACCT-02), which
  `credit_cash`/`debit_cash` maintain (`credit_cash_spec`, `debit_cash_spec`).
* `flash::terms` computes `amount + fee` with a checked add. `fee ≤ amount`
  makes it fit only when `2·amount` fits. So the theorem assumes
  `amount ≤ pre_balance` and `pre_balance + amount < 2^127`, which is always
  true for real token balances.
