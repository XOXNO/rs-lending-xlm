# Pool accounting: Aeneas + Lean proofs

Lean proofs, against the generated `lean/PoolAeneas/Funs.lean`, about the
pool accounting in `contracts/pool/src`: share mints and burns, revenue
claim, solvency and utilization guards, cash, protocol revenue, bad-debt
socialization, flash-loan fee, interest accrual (`compound_interest`,
`accrue_step`, `accrue_chunk`, `global_sync`), and the `contracts/pool/src/ops/`
legs with the Certora pool summaries they discharge.

## Conventions

* `f ⦃ r => P r ⦄`: total correctness (`f` returns, `P` holds).
  `(∃ u, f = ok u) ↔ C`: exact pass condition. `f = ok c' → P c'`: holds
  after every successful return.
* Specifications never divide. `IsFloor n d q` is `q*d ≤ n < q*d + d`,
  `IsCeil n d q` is `n ≤ q*d < n + d`, `IsHalfUp n d q` is
  `2qd ≤ 2n + d < 2qd + 2d` (all for `d > 0`).
* `RAY = 10^27`, `dec` is the asset's decimals, `k = 27 - dec`. `x.val` is
  the integer inside `i128`/`Ray`.
* `panic_with_error!`/`assert_with_error!` abort. Error codes
  (`CollateralError`, `GenericError`) are not modelled.

## Extraction and trusted base

Crate, models and Aeneas workaround: [extraction.md](extraction.md#pool)
([trusted base](extraction.md#pool-trusted-base);
[not extracted](extraction.md#pool-extraction-roots): token transfers,
flash-loan callback, `run_batch`, `ops::market`, entry points, views,
`Cache::transfer_out`).

Storage reads and ledger time (`storedParams`, `storedState`, `ledgerNowMs`)
are uninterpreted, so a read is not tied to an earlier write. The
stored-state invariant is inductive in three steps:

1. Every leg's committed cache satisfies `PoolInv` (the leg theorems).
2. `load_ok` says a load copies the stored fields.
3. `synced_market_spec` carries `PoolInv` through accrual.

## Theorems

Files are under `lean/PoolAeneas/Proofs/`.

### Share totals and revenue claims (`Shares.lean`, `contracts/pool/src/cache/shares.rs`)

`SharesInv c` is INV-ACCT-01 on one cache: `0 ≤ revenue ≤ supplied`,
`0 ≤ borrowed`. Each `*_preserves` theorem (INV-ACCT-01) assumes `SharesInv`
and says on success only the named totals change and `SharesInv` holds; mints
also assume `s ≥ 0`.

| Lean name | Rust function | Claim |
|---|---|---|
| `mint_supply_preserves` | `Cache::mint_supply` | `supplied += s` |
| `burn_supply_preserves` | `Cache::burn_supply` | `0 ≤ s ≤ supplied`, `supplied -= s` |
| `mint_debt_preserves` | `Cache::mint_debt` | `borrowed += s` |
| `burn_debt_preserves` | `Cache::burn_debt` | `0 ≤ s ≤ borrowed`, `borrowed -= s` |
| `accrue_revenue_preserves` | `Cache::accrue_revenue` | `revenue += s`, `supplied += s` |
| `absorb_supply_as_revenue_preserves` | `Cache::absorb_supply_as_revenue` | `revenue += s`, `supplied` unchanged |
| `burn_claimable_revenue_preserves` | `Cache::burn_claimable_revenue` | `SharesInv` (with the hypotheses below) |

**`burn_claimable_revenue_spec`** (`Cache::burn_claimable_revenue`;
INV-ACCT-06, INV-ACCT-01, ADR-0003) never aborts. Hypotheses:
`supply_index > 0`, `0 ≤ revenue ≤ supplied`, `cash ≥ 0`, `dec ≤ 27`,
`revenue·supply_index < 2^127·RAY`. With
`T = ⌊revenue·supply_index/(RAY·10^k)⌋`, the payout `amt` satisfies:
* `0 ≤ amt ≤ cash`, `amt ≤ T`;
* `amt = 0` iff `cash ≤ 0 ∨ T ≤ 0`, cache then unchanged;
* positive `amt = min(cash, T)` burns `burned > 0` shares (at most all
  revenue) from `revenue` and `supplied` only;
* full claim burns all revenue; cash-limited burns `⌈revenue·amt/T⌉`;
* `amt·RAY·10^k ≤ burned·supply_index` (payout ≤ burned value);
* the `scaled_to_burn != Ray::ZERO` `InternalError` assert is unreachable.

With negative `cash` it returns `0 > cash`, hence `cash ≥ 0` (INV-ACCT-02,
kept by `credit_cash_spec`, `debit_cash_spec`).

Mint inputs are non-negative in production: `calculate_scaled_supply` /
`calculate_scaled_borrow` of a validated amount, `protocol_fee_shares`
(`add_protocol_revenue_spec`), or a seized position's validated scaled
amount. Burns need no hypothesis: the checked subtraction and the
`require_revenue_backed` assert reject every invariant-breaking input.

### Guards (`Guards.lean`, `contracts/pool/src/guards.rs`)

`B = ⌈borrowed·borrow_index/RAY⌉` (ceiled debt), `S =
⌊supplied·supply_index/RAY⌋` (floored supply). Every guard assumes
non-negative share totals and indexes (INV-ACCT-01; indexes start at RAY,
floored at `SUPPLY_INDEX_FLOOR_RAW`) and RAY values fitting `i128` (else
`mul_floor`/`mul_ceil` abort, also a rejection).

| Lean name | Rust function | Claim | Extra hypotheses | Invariants |
|---|---|---|---|---|
| `require_supply_for_debt_spec` | `guards::require_supply_for_debt` | Passes iff `¬(supplied = 0 ∧ borrowed ≠ 0)` | none | INV-ACCT-09 |
| `require_utilization_below_max_spec` | `guards::require_utilization_below_max` (debt mints) | Passes iff `UtilPass` (`supplied = 0 ∨ B = 0 ∨ B·RAY ≤ max_utilization·S`), every ceiling incl. 100% | `B`, `S` fit; `0 ≤ max_utilization ≤ RAY` (`verify`) | INV-ACCT-08, audit S-03 |
| `require_exit_utilization_below_max_spec` | `guards::require_exit_utilization_below_max` (ordinary withdrawals, revenue claims) | Passes iff `max_utilization ≥ RAY ∨ UtilPass` (no check at 100%) | as above | INV-ACCT-08, audit S-03 |
| `require_exit_utilization_ok` | same | Returned ⇒ `max_utilization ≥ RAY` or the full gate returned | none | INV-ACCT-08 |
| `max_market_value_spec`, `max_market_entry_spec`, `maxMarketEntry_eq` | `constants::MAX_MARKET_VALUE_RAY`, `MAX_MARKET_ENTRY_RAY` | `MaxMarketValue = 2^127 − 1 − 10^27`, `MaxMarketEntry = ⌊MaxMarketValue / 8⌋` | none | INV-IDX-06, INV-ACCT-11 |
| `fits_entry_ceiling_spec` | `guards::fits_entry_ceiling` | Never aborts; `true` iff `EntryFits shares index` (shares and their half-up value `≤ MaxMarketEntry`); unrepresentable value gives `false` | `shares, index ≥ 0` | INV-ACCT-11 |
| `require_within_entry_ceiling_spec` | `guards::require_within_entry_ceiling` (supply, borrow, strategy and flash-loan entries) | Passes iff `EntryFits supplied supply_index ∧ EntryFits borrowed borrow_index` | share totals and indexes `≥ 0` | INV-ACCT-11 |
| `require_within_entry_ceiling_congr` | same | Depends only on share totals and indexes | none | INV-ACCT-11 |
| `require_liquidation_buffer_spec` | `guards::require_liquidation_buffer` (reserve via `Bps::apply_to_ceil`) | Passes iff `cash − draw ≥ R`, iff `Sa·200 ≤ (cash − draw)·10000`, with `Sa = ⌊supplied·supply_index/(RAY·10^k)⌋`, `R = ⌈Sa·200/10000⌉` | `cash ≥ 0`, `draw ≥ 0`, `dec ≤ 27` | INV-ACCT-07 |
| `backing_shortfall_spec` | `guards::backing_shortfall` | Never aborts; floored backing shortfall (below) | `cash ≥ 0`, `dec ≤ 27` | INV-ACCT-04 |
| `require_backed_market_spec` | `guards::require_backed_market` | Passes iff `C ≤ Dt ∨ C − Dt < (cash + 1)·10^k` | as above | INV-ACCT-04 |

* **`require_utilization_below_max_spec`**: at 100% `UtilPass` reads
  `B ≤ S`. The code compares `B` with `⌊S·max_utilization/RAY⌋`, dividing
  nothing: debt rounds up, supply down, so rounding cannot admit excess
  utilization, no quotient overflows, and debt against zero floored supply
  fails.
* **`backing_shortfall_spec`**: `C = S` (floored claims), `Dt = B` (ceiled
  debt). `C ≤ Dt` → 0; else `max(0, U − cash)`, `U = ⌊(C − Dt)/10^k⌋`. A
  positive shortfall has `shortfall·10^k ≤ C − Dt − cash·10^k` (never
  over-reported).

### Typed fixed-point wrappers (`Wrappers.lean`, `FloorCeilScaling.lean`; `common/src/math/fp.rs`)

`fp_core` is private; pool code reaches it only through these.

| Lean name | Rust function | Claim | Hypotheses | Used by |
|---|---|---|---|---|
| `ray_div_floor_saturating_spec` (in `FloorCeilScaling.lean`) | `Ray::div_floor_saturating` | Never aborts; `⌊a·RAY/b⌋` clamped to `i128` | `b ≠ 0` | `protocol_fee_shares` (`add_protocol_revenue_spec`, `book_fee_spec`), `calculate_scaled_cap_spec` |
| `bps_apply_to_ceil_spec` | `Bps::apply_to_ceil` | Exactly `⌈amount·bps/10000⌉` (every sign) | that ceiling fits `i128` | `require_liquidation_buffer_spec` |
| `ray_try_mul_spec` | `Ray::try_mul` | Never aborts; `None` for a negative operand; else `Some(half-up a·b/RAY)` if it fits, else `None` | none | `rates::simulate::market_value` (`accrue_step`), `guards::fits_entry_ceiling` |
| `ray_mul_int_spec` | `Ray::mul_int` | Exactly `a·m` | `a·m` fits `i128` | `compound_interest_spec` |
| `ray_mul_int_overflow` | `Ray::mul_int` | Aborts (`MathOverflow`) | `a·m` does not fit | |
| `i128_from_u64_spec` | `i128::from(u64)` (trusted model) | Never aborts; same integer | none | `compound_interest` |
| `bps_from_same` | `Bps::from::<i128>` | Wraps the value | none | `require_liquidation_buffer_spec` |

`utilization` divides with `Ray::div` (`Ray::try_div` is not extracted),
only when `borrowed < supplied`. Unreached from the pool roots:
`Ray::mul_wad_ratio_floor`, `Wad::*`, `Bps::{div, from_fraction_floor,
from_fraction_ceil, try_from_fraction, from_weighted_sum, apply_to_ray_ceil}`.

### Cash (`Cash.lean`, `contracts/pool/src/cache/cash.rs`)

All INV-ACCT-02, no hypotheses.

| Lean name | Rust function | Claim |
|---|---|---|
| `require_nonneg_amount_iff` | `validation::require_nonneg_amount` | Passes iff `amount ≥ 0` |
| `require_reserves_iff` | `Cache::require_reserves` | Passes iff `amount ≤ cash` |
| `credit_cash_spec` | `Cache::credit_cash` | Returns iff `amount ≥ 0 ∧ cash + amount < 2^127`; then only `cash += amount` |
| `debit_cash_spec` | `Cache::debit_cash` | Returns iff `0 ≤ amount ≤ cash`; then only `cash -= amount`, `≥ 0` |

### Interest booking (`Interest.lean`, `contracts/pool/src/interest.rs`)

| Lean name | Rust function | Claim | Hypotheses | Invariants |
|---|---|---|---|---|
| `add_protocol_revenue_spec` | `interest::add_protocol_revenue` | Never aborts; zero fee: unchanged; else mints only `s = min(⌊fee·RAY/supply_index⌋ (saturating), i128::MAX − supplied) ≥ 0` into `revenue` and `supplied`; keeps `SharesInv` | `SharesInv`, `supply_index > 0`, `fee ≥ 0` | INV-ACCT-01, INV-IDX-05 (floor rounding of protocol shares) |
| `apply_bad_debt_spec` | `interest::apply_bad_debt_to_supply_index` | Never aborts; only the supply index moves, down at least pro rata, floored at `SUPPLY_INDEX_FLOOR_RAW` (below) | `supplied ≥ 0`, `supply_index > 0`, `bad ≥ 0`, `V` fits | INV-IDX-03, INV-IDX-02 (lower clamp) |

* **`apply_bad_debt_spec`**: `V` = half-up `supplied·supply_index/RAY`.
  `V = 0`: unchanged. Else, with `rem = V − min(bad, V)`, `f = ⌊rem·RAY/V⌋`,
  `n = ⌊supply_index·f/RAY⌋`: new index `max(n, RAY/1000)`, in
  `[SUPPLY_INDEX_FLOOR_RAW, max(old, floor)]`, and `n·V ≤ old·rem` (at least
  the exact pro-rata write-down).

### Flash loans (`Flash.lean`, `contracts/pool/src/ops/flash.rs`)

| Lean name | Rust function | Claim | Hypotheses | Invariants |
|---|---|---|---|---|
| `flash_loan_fee_on_spec` | `Bps::flash_loan_fee_on` (`common/src/math/fp.rs`) | Never aborts; fee = half-up `amount·bps/10000`, or 1 if that is 0 at a positive rate | `0 ≤ bps ≤ 10000`, `amount ≥ 0` | INV-FLASH-01 |
| `terms_spec` | `flash::terms` | Never aborts; `0 ≤ F ≤ P`, `F > 0` when `bps > 0`; `total_repayment = P + F`, `balance_after_payout = B − P`, `balance_after_repayment = B + F` | `P > 0`, `bps ≤ 10000`, `P ≤ B`, `B + P < 2^127` (below) | INV-FLASH-01 |
| `book_fee_spec` | `flash::book_fee` | Never aborts; only `cash += F`, `revenue` and `supplied` `+= s ≥ 0`; keeps `SharesInv` | `SharesInv`, `supply_index > 0`, `F ≥ 0`, `dec < 27`, `F·10^k` and `cash + F` fit | INV-FLASH-01, INV-ACCT-01, INV-ACCT-02 |

`terms_spec` hypotheses: `P > 0` (`require_positive_amount`); `bps ≤ 10000`
(`flashloan_fee ≤ MAX_FLASHLOAN_FEE_BPS = 500`); `P ≤ B` (payout comes out of
`B`); `B + P < 2^127`, since the checked `amount + fee` fits only when
`2·amount` does (true for real balances).

### Compound interest (`Compound.lean`, `common/src/rates/compound.rs`)

| Lean name | Rust function | Claim | Hypotheses | Invariants |
|---|---|---|---|---|
| `compound_interest_spec` | `rates::compound_interest` | Never aborts; `RAY + rate·delta_ms ≤ f ≤ 21·RAY`, exactly `RAY` for `delta_ms = 0` | `rate ≥ 0`, `rate·delta_ms ≤ 3·RAY` (production: `≤ 2·RAY + 1.6·10^10`, `compound_x_le`) | INV-IDX-01 (factor `≥ RAY`), INV-IDX-04 |
| `ci_step`, `ci_loop_cons`, `ci_loop_nil(_spec)`, `hu_pow_bound` | the Taylor loop | Iteration bounds; loop unfolding | | |

Proof: the k-th power stays `≤ 3^k·RAY` (half-up cannot exceed `3B` when the
product is below `3B·RAY + RAY/2`); terms are non-negative and
`4 + Σ_{k=2..8} 3^k/k! < 21`.

### Accrual step (`Accrual.lean`, `common/src/rates/simulate.rs`, `index.rs`)

| Lean name | Rust function | Claim | Hypotheses | Invariants |
|---|---|---|---|---|
| `compound_x_le` | (curve → compound) | `r = half_up(a/31_556_926_000)`, `0 ≤ a ≤ 2·RAY`, `delta ≤ 31_556_926_000` ⇒ `0 ≤ r`, `r·delta ≤ 3·RAY` | `a` from `calculate_borrow_rate_spec` | INV-IDX-04 |
| `protocol_fee_shares_spec` | `rates::protocol_fee_shares` | Never aborts; `0 ≤ sh`, `supplied + sh ≤ i128::MAX`, `sh·supply_index ≤ fee·RAY` | `fee ≥ 0`, `supply_index > 0`, `supplied ≥ 0` | INV-IDX-05, INV-ACCT-01 |
| `update_supply_index_strong` | `rates::update_supply_index` | Never aborts; `old ≤ n ≤ 10^36`, and `n = old` or `supplied·n ≤ (V + rewards)·RAY` (distributes at most `rewards`) | `supplied, rewards ≥ 0`, `0 < old ≤ 10^36`, `V + rewards` fits | INV-IDX-02 |
| `supply_index_reward_shortfall_total` | `rates::supply_index_reward_shortfall` | Never aborts; returns `rewards − (t' − t) ∈ [0, rewards]` | `t ≤ t' ≤ t + rewards < 2^127` (old/new supply values) | INV-IDX-05 |
| `market_value_spec` | `rates::simulate::market_value` | Never aborts; `Some(hu(shares·index, RAY))` if it fits `i128`, else `None` | `shares, index ≥ 0` | INV-IDX-06 |
| `value_room_spec` | `rates::simulate::value_room` | Never aborts; `room = max(0, MAX_MARKET_VALUE_RAY − max(debt_value, supply_value))` | values `≥ 0` | INV-IDX-06 |
| `cap_borrow_index_spec` | `rates::simulate::cap_borrow_index` | Never aborts; `old ≤ n ≤ grown`, `borrowed·(n − old) ≤ room·RAY` (debt value grows ≤ `room`) | `borrowed ≥ 0`, `0 ≤ old ≤ grown`, `room ≥ 0` | INV-IDX-06, INV-IDX-01 |
| `accrue_step_spec` | `rates::accrue_step` | **Never aborts, whatever the market size.** Indexes bounded; interest split and capped at the room (below) | below | INV-IDX-06, INV-IDX-01, INV-IDX-02, INV-IDX-05, audit S-04 |
| `accrue_step_within_ceiling` | (consequence of `accrue_step_spec`) | Old debt and supply values `≤ MAX_MARKET_VALUE_RAY` ⇒ new debt value and post-revenue-share supply value (new index) too | the last two conclusions of `accrue_step_spec` | INV-IDX-06 |
| `hu_scale_le`, `hu_le_of_le`, `hu_lt_of_lt`, `hu_add_le`, `max_compound_delta_val` | | Half-up bounds (`hu_add_le`: `n' ≤ n + r·RAY → hu(n') ≤ hu(n) + r`); `MAX_COMPOUND_DELTA_MS = 31_556_926_000` | | |

**`accrue_step_spec`** (`od, nd`: old/new debt values; `t ≤ t'`: old/new
supply values):

* `borrow_index ≤ b' ≤ 10^36`, `supply_index ≤ s' ≤ 10^36`;
* `0 ≤ revenue_shares`, `supplied + revenue_shares ≤ i128::MAX`;
* `revenue_shares·s' + (t' − t)·RAY ≤ (nd − od)·RAY` (revenue plus supplier
  value ≤ accrued interest);
* `nd − od ≤ max(0, MAX_MARKET_VALUE_RAY − max(od, t))` (interest ≤ room
  below the ceiling; zero when a total is at or above it or does not fit
  `i128`).

Hypotheses: `ValidRateModel` (incl. `max_borrow_rate ≤ 2·RAY`),
`borrowed, supplied ≥ 0`, `0 ≤ borrow_index ≤ 10^36`,
`0 < supply_index ≤ 10^36`, `delta ≤ MAX_COMPOUND_DELTA_MS`.

No hypothesis says a market value fits `i128` (audit S-04): the step values
both totals with `Ray::try_mul` and caps the borrow index at the room below
`MAX_MARKET_VALUE_RAY`, bounding every later product by an old value plus
that room. The only abort paths, `market_value`'s sign checks, are excluded
by INV-ACCT-01 and the index bounds.

### Chunk and `global_sync` (`GlobalSync.lean`, `contracts/pool/src/interest.rs`)

`AccrualDomain c`: `accrue_step_spec`'s hypotheses on the cache plus
`SharesInv c` (nothing on market-value size). `ChunkPost c c'`:
`SharesInv c'`, both indexes monotone and `≤ 10^36`, `revenue` and
`supplied` grow by the same `sh ≥ 0`, nothing else changes. `ChunkPost` is
reflexive and transitive and chunks keep `AccrualDomain` (`ChunkPost.refl`,
`ChunkPost.trans`, `AccrualDomain.of_chunk`): a loop invariant for any
number of chunks.

| Lean name | Rust function | Claim | Hypotheses | Invariants |
|---|---|---|---|---|
| `accrue_chunk_spec` | `interest::accrue_chunk` | Never aborts; `ChunkPost c c'`; returned flag: the ceiling held the chunk's interest | `AccrualDomain c`, `delta_ms ≤ MAX_COMPOUND_DELTA_MS` | INV-IDX-01, INV-IDX-02, INV-IDX-06, INV-ACCT-01 |
| `global_sync_loop_spec` | `global_sync`'s loop | Induction over chunks; the loop terminates (below) | the step hypothesis | INV-IDX-04 |
| `global_sync_noop` | `interest::global_sync` | `current ≤ last`: cache unchanged, timestamp not advanced | none | INV-IDX-04 |
| `global_sync_spec_of_inv` | `interest::global_sync` | `e = current − last`, chunk invariant with `Inv e c`: never aborts; `e = 0` → unchanged; else `{x with last_timestamp := current}` with `Inv 0 x` | as `global_sync_loop_spec` | INV-IDX-04 |
| `global_sync_spec` | `interest::global_sync` | **Never aborts, any elapsed time.** `e = 0` → unchanged; else accrues in chunks of at most a year, result `x` has `ChunkPost c x`, `AccrualDomain x`, `last_timestamp = current_timestamp`; the ceiling event leaves the cache unchanged | `AccrualDomain c` | INV-IDX-06, INV-IDX-01, INV-IDX-02, INV-IDX-04, INV-ACCT-01, audit S-04 |
| `min_u64_val`, `sat_sub_u64_val`, `elapsed_val` | `u64::min`, `u64::saturating_sub`, `Cache::elapsed_ms` | Value lemmas | | |

**`global_sync_loop_spec`**: any `Inv remaining cache` preserved by each
chunk `k = min(r, MAX_COMPOUND_DELTA_MS)`, `r > 0` (`Inv r` → `Inv (r − k)`)
holds at `remaining = 0`, and the loop terminates: chunks are positive, at
most one year, all but the last full, summing to the elapsed time.

INV-IDX-04's "accrual and read-only projection use the same calculation"
holds by construction (both call `accrue_step`); `simulate_update_indexes`
is not extracted.

### Operation legs (`Legs.lean`, `Legs2.lean`, `contracts/pool/src/ops/`)

A leg loads and accrues its market (`ops::synced_market` = `Cache::load` +
`global_sync`), mutates the cache, and commits. Leg theorems name the synced
cache by `ops.synced_market env key = ok c`, assume `PoolInv c`, and
describe **every successful return** (`leg = ok r → …`), the form a Certora
summary needs (an aborting call reverts). The leg's guards (`require_*`,
checked arithmetic, `*RoundsToZeroShares` asserts) are derived, not assumed;
abort conditions are not characterised beyond the guard specs. Fit
hypotheses ("`amount·10^k < 2^127`", "the position's RAY value fits") are
the conversion helpers' exactness domains.

`PoolInv c` (stored-state invariant): `0 ≤ revenue ≤ supplied`,
`0 ≤ borrowed` (INV-ACCT-01), `cash ≥ 0` (INV-ACCT-02),
`10^24 ≤ supply_index ≤ 10^36`, `10^27 ≤ borrow_index ≤ 10^36`.
`ParamsOk c`: `asset_decimals ≤ 18`, `flashloan_fee ≤ 500` (set by the
unextracted `MarketParamsRaw::verify` at creation / rate-model replacement;
no leg changes params). `snapOf c`: snapshot `commit` returns; `indexOf c`:
returned index pair.

| Lean name | Rust function | Claim (on every return) | Hypotheses | Invariants |
|---|---|---|---|---|
| `commit_eq` | `Cache::commit` | Never aborts; returns `snapOf c` | none | |
| `load_leg_ok` | `ops::load_leg` | Amount, position `≥ 0`; market synced to the returned cache; returned position is the action's | none | INV-ACCT-05 (validation) |
| `renewed_market_ok` | `ops::renewed_market` | Returned `c` ⇒ `synced_market` returned `c` | none | |
| `market_index_eq`, `position_mutation_eq`, `strategy_mutation_eq` | `Cache::{market_index, position_mutation, strategy_mutation}` | DTOs carry the given position/amounts, `indexOf c`, decimals | none | |
| `nonneg_of_ok`, `positive_of_ok`, `massert_ok`, `sub_unwrap_eq_ok`, `ceil_le_of_index_ge_ray` | validation helpers, `checked_sub().unwrap_or_else(panic)` | Returned ⇒ the checked condition; the checked difference is exact; `⌈A·RAY/bi⌉ ≤ A` for `bi ≥ RAY` | as stated | |
| `supply_apply_spec` | `ops::supply::apply` | (below) | `PoolInv c`, `dec ≤ 18`, `amount·10^k < 2^127`, floor fits | INV-ACCT-01/02/03/05/10/11, ADR-0003 |
| `mint_debt_ok` | `ops::borrow::mint_debt` | (below) | `PoolInv c`, `dec ≤ 18`, `amount·10^k < 2^127` | INV-ACCT-05/07/08/11 |
| `borrow_accounting_spec` | `ops::borrow::accounting` | As `mint_debt_ok`; `actual_amount = amount`; commits `c` with only `borrowed += m`, `cash -= amount`; `PoolInv`; entry-ceiling guard passes | as above | INV-ACCT-01/02/05/07/08/10/11 |
| `repay_accounting_spec` | `ops::repay::accounting` | (below) | `PoolInv c`, `dec ≤ 18`, `⌈position·bi/RAY⌉` fits | INV-ACCT-01/02/03/05/10, ADR-0003 |
| `withhold_liquidation_fee_ok` | `withdraw::withhold_liquidation_fee` | No fee: `net = gross`, cache unchanged; else `fee ≤ gross`, `net = gross − fee`, fee booked as `s ≥ 0` shares into `revenue` and `supplied`, `s·supply_index ≤ fee·10^k·RAY` | `SharesInv`, `si > 0`, `fee ≥ 0`, `dec < 27`, `gross·10^k < 2^127` | INV-ACCT-01 |
| `gate_and_debit_ok` | `withdraw::gate_and_debit` | `0 ≤ net ≤ cash`; exit utilization guard passed unless skipped; not `supplied = 0 ∧ borrowed ≠ 0`; only `cash -= net` | none | INV-ACCT-02/08/09 |
| `withdraw_accounting_spec` | `ops::withdraw::accounting` | (below) | `PoolInv c`, `dec ≤ 18`, `hu(position·si, RAY)` fits (the exact abort condition of `resolve_withdrawal`) | INV-ACCT-01/02/05/09/10, ADR-0003 |
| `net_settle_apply_spec` | `ops::net_settle::apply` | (below) | `PoolInv c`, `dec ≤ 18`, `⌊ss·si/RAY⌋`, `⌈ds·bi/RAY⌉` fit | INV-ACCT-01/02/05/09/10, ADR-0003 |
| `seize_apply_spec` | `ops::seize::apply` | Position `≥ 0`, cash unchanged, `PoolInv`; deposit side: only `revenue += p`; borrow side: only `borrowed -= p` and supply index written down (`≥ 10^24`, `≤ old`) | `PoolInv c`; borrow side: `hu(supplied·si, RAY)` fits | INV-ACCT-01/10, INV-IDX-03 |
| `backing_shortfall_nonneg` | `guards::backing_shortfall` | Returned ⇒ `≥ 0` | none | INV-ACCT-04 |
| `recapitalize_accounting_spec` | `ops::recapitalize::accounting` | `amount ≥ 0`; `applied = min(amount, sf)`, shortfall `sf ≥ 0`; `refund = amount − applied`; only `cash += applied`; `PoolInv` | `PoolInv c` | INV-ACCT-02/03/04 |
| `compute_fee_ok` | `strategy::compute_fee` | `0 ≤ fee ≤ amount`; 0 without `charge_fee`; otherwise the flash-loan fee | `amount ≥ 0`, `flashloan_fee ≤ 10000` | INV-FLASH-01 |
| `strategy_accounting_spec` | `ops::strategy::accounting` | (below) | `PoolInv c`, `ParamsOk c`, `amount·10^k < 2^127` | INV-ACCT-01/02/05/07/10/11 |
| `revenue_accounting_spec` | `ops::revenue::accounting` | `burn_claimable_revenue` gave `(amt, c1)`, `ClaimResult c T amt c1`; `0 ≤ amt ≤ cash`; exit utilization guard passed on `c1`; commits `c1` with `cash -= amt`; `PoolInv`; not `supplied = 0 ∧ borrowed ≠ 0` | `PoolInv c`, `dec ≤ 27`, `revenue·si < 2^127·RAY` | INV-ACCT-01/02/06/08/09 |
| `load_ok` | `Cache::load` | Loaded share totals, indexes, cash, timestamp are the stored ones | `storedState k = ok st` | |
| `synced_market_spec` | `ops::synced_market` | **Never aborts, for any elapsed time and market size**; `PoolInv` preserved; cash, params unchanged; indexes non-decreasing | load returned `c0`, `PoolInv c0`, `ValidRateModel c0.params` | INV-IDX-06, INV-IDX-01/02/04, INV-ACCT-01/02, audit S-04 |
| `AccrualDomain.of_poolInv` | | `PoolInv c` with a validated rate model gives `AccrualDomain c` | as stated | |

* **`supply_apply_spec`**: `amount, position ≥ 0`; mints
  `m = ⌊amount·10^k·RAY/si⌋ ≥ 0`, `m > 0` iff `amount > 0`; position `+= m`;
  `actual_amount = amount`; commits `c` with only `supplied += m`,
  `cash += amount`; `PoolInv`; entry-ceiling guard passes.
* **`mint_debt_ok`**: `0 < amount ≤ cash`; liquidation-buffer guard passed on
  the gross amount; mints `m = ⌈amount·10^k·RAY/bi⌉ > 0`, `m ≤ amount·10^k`;
  position `+= m`; only `borrowed += m`; utilization guard (every ceiling,
  100% included) and entry-ceiling guard passed after.
* **`repay_accounting_spec`** (`D = ⌈position·bi/(RAY·10^k)⌉`):
  `0 ≤ net ≤ amount`, `overpayment = amount − net`; full (`D ≤ amount`):
  new `= 0`, `net = D`; partial: `net = amount`, burn
  `= ⌊amount·10^k·RAY/bi⌋`; `amount > 0 ∧ position > 0 → new < position`,
  else `new = position`; only `borrowed -= burn`, `cash += net`; `PoolInv`.
* **`withdraw_accounting_spec`** (`H`/`F`: half-up/floor position value, `G`:
  gross):
  * `0 ≤ G ≤ F`, `G ≤ amount`, `0 ≤ new ≤ position`.
  * Full (`H ≤ amount`): `new = 0`, `G = F`. Partial: `G = amount`, burn
    `= ⌈amount·10^k·RAY/si⌉`.
  * `amount = 0 ∧ H > 0 → new = position`; `G > 0 → new < position`;
    `amount > 0 ∧ position > 0 → new < position`; `position = 0 → new = 0`.
  * `net = G`, or `G − fee` with a liquidation fee; `0 ≤ net ≤ G`.
  * Only `supplied := supplied − burn + s`, `revenue += s`, `cash -= net`;
    `s = 0` without fee, always `s ≤ burn`.
  * `PoolInv`, and not `supplied = 0 ∧ borrowed ≠ 0` after.
* **`net_settle_apply_spec`**: `0 ≤ settled ≤ amount` (and `≤ F`, `≤ D` when
  positive); `0 ≤ ns ≤ ss`, `0 ≤ nd ≤ ds`; `settled > 0` burns both sides,
  `= 0` neither; full close only when exhausted; partial burns favour the
  protocol; only `supplied -= ss − ns`, `borrowed -= ds − nd` (no cash);
  `PoolInv`; not `supplied = 0 ∧ borrowed ≠ 0`.
* **`strategy_accounting_spec`**: `amount > 0`; buffer guard passed on the
  gross amount; `0 ≤ fee ≤ amount`; mints `m > 0` (ceiling); position `+= m`;
  `actual_amount = amount`, `amount_received = amount − fee`; only
  `borrowed += m`, `revenue` and `supplied += s ≥ 0` (`0` for zero fee),
  `cash -= amount − fee`; `PoolInv`; entry-ceiling guard passes after the
  mint and after the fee shares.

**INV-ACCT-10 (pool side).** "Σ account supply shares = supplied − revenue,
Σ account debt shares = borrowed". With no per-account book, it holds if
each leg moves the totals by exactly its returned position's change, which
the leg theorems state:

* supply: `Δsupplied = Δposition`;
* borrow/strategy: `Δborrowed = Δposition` (fee shares go to `revenue` and
  `supplied` equally);
* repay: `Δborrowed = Δposition`;
* withdraw: `Δ(supplied − revenue) = Δposition`;
* net settle: the same on both sides;
* seize deposit: `Δ(supplied − revenue) = −p`; seize borrow:
  `Δborrowed = −p` (the controller then removes the seized position);
* accrual moves `revenue` and `supplied` equally (`ChunkPost`).

**INV-ACCT-03 (pool side).** The pool credits exactly what the controller
reports received (supply: `cash += amount`; repay: `cash += net`, refund
`= amount − net`; recapitalize: `cash += applied`, refund
`= amount − applied`). The controller halves of both (writing the position,
measuring the receipt) are outside this model.

### The market value ceiling as a stored-state invariant (`ValueCeiling.lean`)

`ValueCeil c` (INV-IDX-06): half-up total supply value (supply index) and
total debt value (borrow index) are both `≤ MAX_MARKET_VALUE_RAY`. Every
extracted leg keeps it. Exit theorems take their leg theorem's hypotheses
plus `ValueCeil c` on the synced cache; entry and flash-loan theorems need no
ceiling hypothesis (`MAX_MARKET_ENTRY_RAY ≤ MAX_MARKET_VALUE_RAY`,
`ValueCeil.of_entry_guard`). Market creation (zero totals) is not covered.
Each `<leg>_keeps_ceiling` is about its leg theorem's function and gives
`ValueCeil` of the committed cache; rows are INV-IDX-06 unless noted.

| Lean name | Claim | Why |
|---|---|---|
| `accrue_chunk_keeps_ceiling` | Never aborts; `ChunkPost`, `ValueCeil` | `accrue_step_within_ceiling` |
| `global_sync_keeps_ceiling` | Never aborts; `ValueCeil`, **any elapsed time** | chunk invariant |
| `synced_market_keeps_ceiling` | Never aborts; `ValueCeil` of the synced cache | load + `global_sync` |
| `supply_keeps_ceiling`, `borrow_keeps_ceiling`, `strategy_keeps_ceiling` | No ceiling hypothesis (INV-ACCT-11) | entry-ceiling guard passes |
| `repay_keeps_ceiling` | | debt shares only fall |
| `withdraw_keeps_ceiling` | With or without a liquidation fee | fee shares minted `≤` supply shares burned |
| `net_settle_keeps_ceiling` | | supply and debt shares only fall |
| `seize_keeps_ceiling` | Both sides (also INV-IDX-03) | deposit side reclassifies shares; borrow side burns debt, lowers supply index |
| `recapitalize_keeps_ceiling` | | only cash changes |
| `revenue_keeps_ceiling` | | revenue and supply shares only fall |
| `flash_finalize_keeps_ceiling` | `ops::flash::finalize`: `PoolInv` too, no ceiling hypothesis, `cash += fee`; takes `book_fee_spec`'s hypotheses (INV-ACCT-11, INV-FLASH-01) | `book_fee_spec`, then entry-ceiling guard |
| `ValueWithin.mono`, `ValueWithin.of_entryFits`, `ValueCeil.of_le`, `ValueCeil.of_entry_guard` | Smaller product, smaller half-up value; entry ceiling below value ceiling | |

**The ceiling discharges the fit hypotheses** the leg theorems take:

| Lean name | Claim | Discharges |
|---|---|---|
| `ValueCeil.supply_value_fits` | Half-up total supply value `< 2^127` | `seize_apply_spec`'s fit hypothesis (`seize_keeps_ceiling` drops it) |
| `ValueCeil.revenue_fits` | With `revenue ≤ supplied`: `revenue·supply_index < 2^127·RAY` | `revenue_accounting_spec`'s fit hypothesis (`revenue_keeps_ceiling` drops it) |
| `ValueWithin.fits_half_up`, `ValueWithin.fits_floor`, `ValueWithin.fits_ceil` | `p ≤ total`, `index ≥ 0`: half-up, floored, ceiled values of `p` fit `i128` (ceiled ≤ half-up + 1; the ceiling leaves one token below `i128::MAX`) | position fit hypotheses, given "position ≤ market total" |
| `isFloor_le_halfUp`, `isCeil_le_halfUp_succ` | `⌊n/RAY⌋ ≤ hu(n/RAY)` and `⌈n/RAY⌉ ≤ hu(n/RAY) + 1` | |
| `withdraw_keeps_ceiling_of_position_le` | `withdraw_keeps_ceiling` with `position ≤ supplied` replacing the fit hypothesis | |
| `repay_keeps_ceiling_of_position_le` | `repay_keeps_ceiling` with `position ≤ borrowed` replacing it | |
| `net_settle_keeps_ceiling_of_position_le` | `net_settle_keeps_ceiling` with `supply position ≤ supplied`, `debt position ≤ borrowed` replacing both | |

So the position fit hypotheses (withdraw, repay, net settlement) reduce to
**the caller's position is at most its market's total**. With no
per-account book the pool cannot prove it; it is the pool-side shape of
INV-ACCT-10, which the controller maintains. Entry legs keep hypotheses on
the input *amount* (`amount·10^k` fits; for supply, the minted share count
fits), not stored state.

### Certora summaries in their own terms (`Summaries.lean`)

`IndexInRange m`: `10^24 ≤ m.supply_index ≤ 10^36`, `10^27 ≤
m.borrow_index ≤ 10^36` (what `nondet_market_index_raw` assumes);
`indexOf_inRange`: `PoolInv c → IndexInRange (indexOf c)`.

| Lean name | Claim (every return of the leg) |
|---|---|
| `supply_summary_holds` | `amount > 0 → new > old`, else `new = old`; `actual_amount = amount`; `IndexInRange`; decimals `≤ 18` |
| `borrow_summary_holds` | `amount > 0` and `new > old`; `actual_amount = amount`; `IndexInRange`; decimals `≤ 18` |
| `withdraw_summary_partial` | `new ≥ 0`; `amount > 0 ∧ position > 0 → new < position`; `position ≤ 0 → new = position`; `0 ≤ actual ≤ amount`; `IndexInRange`; decimals `≤ 18` |
| `withdraw_zero_amount_closes_dust`, `withdraw_dust_instance` | The `withdraw_summary` gap: `amount = 0` with half-up value `H = 0` closes the position (`new = 0`, `actual = 0`); instance: decimals 0, `si = RAY`, position 1 |
| `repay_summary_holds` | Exactly the summary's position clause; `0 ≤ actual ≤ amount`; `IndexInRange`; decimals `≤ 18` |
| `net_settle_summary_holds` | Exactly the summary's clauses; `IndexInRange` |
| `recapitalize_summary_holds` | `0 ≤ actual ≤ amount` |
| `create_strategy_summary_holds` | `new ≥ old`, `amount ≥ 0`, fee `f ∈ [0, amount]` (`0` without `charge_fee`), `actual = amount`, `received = amount − f`; `IndexInRange`; decimals `≤ 18` |
| `claim_revenue_summary_holds` | `0 ≤ actual ≤ cash` |

### Supporting lemmas (`Basic.lean`)

`Ray` comparisons (`ray_le_eq`, `ray_min_eq`, ...); checked `Ray`
addition/subtraction (`ray_checked_add_spec`, `ray_checked_sub_eq_ok`, ...);
`sat_sub_val` (`i128::saturating_sub` is subtraction when it fits);
`isFloor_ediv`/`isCeil_exists`/`floor_fits`; `mul_div_ceil_eq_ok`: whatever
`fp_core::mul_div_ceil` returns is the exact ceiling (partial correctness,
no fit hypothesis; an unrepresentable ceiling aborts), used by
`seize_apply_spec`.

### Copied math (from RatesAeneas)

The scaling, rounding, resolve and accrual specs the legs use
(`resolve_withdrawal_spec`, `calculate_borrow_rate_spec`, `utilization_spec`,
...) are copies of RatesAeneas proofs; see
[extraction.md](extraction.md#pool-copied-proofs).

### Certora summaries (`certora/shared/summaries/pool.rs`): discharged / partial / not

Coverage of the 13 summaries and the shared index generator. "Discharged":
every successful return of the production leg meets every postcondition the
summary assumes on returned values, given synced state in `PoolInv` (and
`ParamsOk` where decimals or the fee rate matter) and the fit domains. No
row covers the token transfers (`borrow::apply` payout, refunds,
`amount_received`, `net_transfer`), events or storage I/O around the leg.
`PoolInv` is inductive (every leg, `synced_market_spec`); `ParamsOk` rests on
the unextracted `MarketParamsRaw::verify`.

| Summary | Status | Theorems | What is not discharged |
|---|---|---|---|
| `nondet_market_index_raw` | Discharged | `indexOf_inRange`; every leg's `PoolInv` of its committed cache; `synced_market_spec` (any elapsed time), `apply_bad_debt_spec`, `accrue_chunk_spec` | |
| `supply_summary` | Discharged | `supply_summary_holds`, `supply_apply_spec` | Decimals `≥ 0` is by type (`u32`), `≤ 18` from `ParamsOk` |
| `borrow_summary` | Discharged | `borrow_summary_holds`, `borrow_accounting_spec` | |
| `withdraw_summary` | **Partial** | `withdraw_summary_partial`, `withdraw_accounting_spec`, `withdraw_zero_amount_closes_dust` | **The summary's `amount = 0 → new = position` is false** for dust positions ([finding 1](#findings-about-the-production-code)). All other clauses hold |
| `repay_summary` | Discharged | `repay_summary_holds`, `repay_accounting_spec` | |
| `net_settle_summary` | Discharged | `net_settle_summary_holds`, `net_settle_apply_spec` | |
| `update_indexes_summary` (empty) | Discharged | `synced_market_spec`, `global_sync_spec` | `ops::market::accrue` (loop over keys, events) not extracted |
| `recapitalize_summary` | Discharged | `recapitalize_summary_holds`, `recapitalize_accounting_spec` | |
| `flash_loan_summary` | Discharged (fee arithmetic) | `terms_spec`, `book_fee_spec`, `flash_finalize_keeps_ceiling` | `flash::apply` (callback, balance measurement, transfers) not extracted |
| `create_strategy_summary` | Discharged | `create_strategy_summary_holds`, `strategy_accounting_spec` | |
| `seize_positions_summary` (empty) | Discharged per entry | `seize_apply_spec` | The batch (`run_batch`, `unique_markets = false`) and events not extracted; the summary has no return value, so nothing more is assumed |
| `claim_revenue_summary` | Discharged | `claim_revenue_summary_holds`, `revenue_accounting_spec` | Owner lookup (`revenue::apply`) |
| `get_sync_data_summary` | **Partial** | `PoolInv` preserved by every leg + `load_ok` + `synced_market_spec` | The params clauses (below) |

**`get_sync_data_summary`**: state clauses (non-negative totals and cash,
index ranges) are the inductive `PoolInv`. Rate-model clauses,
`reserve_factor < BPS`, `flashloan_fee ≤ 500` and the decimals range come
from `MarketParamsRaw::verify` / `InterestRateModel::verify` at market
creation and rate-model replacement (`ops::market`, unextracted); no
extracted leg writes params. `get_sync_data` (a storage read in `lib.rs`) is
unextracted.

## Axiom check

`PoolAeneas.lean` ends with `#assert_standard_axioms PoolAeneas`
([what it enforces](../README.md#trusted-base); rebuild:
[extraction.md](extraction.md#pool-rebuild)). The uninterpreted
`storedParams`, `storedState` and `ledgerNowMs` are Lean `opaque`
definitions, not axioms.

## Not covered

* **"Position ≤ market total" (INV-ACCT-10).** The withdraw, repay and
  net-settlement theorems assume it
  ([above](#the-market-value-ceiling-as-a-stored-state-invariant-valueceilinglean)).
* **The flash loan before `finalize`.** `ops::flash::apply` (payout,
  receiver callback, two balance checks) is unextracted, so nothing shows
  `finalize` runs only with the fee received.
* **Which accruals publish the ceiling event.** No theorem relates the
  carried `value_capped` flag to the interest held.
* **`compound_interest` accuracy.** Only `RAY + x ≤ f ≤ 21·RAY`, not the
  documented truncation error against `e^x`.
* **Storage as state and the unextracted code** ([trusted base](#extraction-and-trusted-base)).
* **Exact abort conditions of the legs** and **the controller halves of
  INV-ACCT-10 and INV-ACCT-03** ([above](#operation-legs-legslean-legs2lean-contractspoolsrcops)).

## Audit findings S-03 and S-04

Both Runtime Verification pool findings are fixed in the extracted code; see
the rows tagged "audit S-03" / "audit S-04". S-03 also uses
`utilization_spec` (in `[0, RAY]`, divides only when `borrowed < supplied`;
[rates](rates-half-up-curve-index.md)) and `mint_debt_ok` /
`borrow_accounting_spec` (guard passed after the mint); S-04 also uses
`accrue_step_within_ceiling` and `ValueCeiling.lean`.

## Findings about the production code

1. **`withdraw_summary` is stricter than the code for zero-amount
   withdrawals of dust** (summary unsound there, no economic impact). It
   assumes `amount = 0 → new_scaled = position`; the code fully closes when
   the half-up value `H = hu(hu(pos·si, RAY), 10^k) ≤ amount`, so `amount = 0`
   with `H = 0` burns the whole position and pays `F = 0`. Case
   (`withdraw_dust_instance`, `withdraw_zero_amount_closes_dust`): 0
   decimals, index `RAY`, position 1: `hu(1·RAY, RAY) = 1`,
   `hu(1, 10^27) = 0 = H ≤ 0`, so `burned = 1`, `gross = 0`, the
   `WithdrawRoundsToZeroShares` assert is skipped, and the position returns
   0, not 1 (worth `10^-27` units). User-facing `withdraw` maps `amount = 0`
   to `WITHDRAW_ALL_SENTINEL` (`contracts/controller/src/positions/supply.rs`),
   so only controller paths passing a literal 0 reach it
   (`execute_withdrawal` callers, unchecked). Summary fix: in the
   else-branch allow `new_scaled = 0` when `amount == 0` (or
   `0 ≤ new_scaled ≤ position`).

No other counterexample was found.
