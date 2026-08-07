# 0016. Interest rates are per-millisecond RAY values, accrued in year-capped chunks of a truncated exponential series

Status: Accepted

## Context

Interest accrual must be exact enough that rounding never mints unbacked
value, cheap enough to run inside every pool operation, and honest across
arbitrary gaps — a market may go untouched for months and must not compound a
long-stale rate as if utilization never moved. The Soroban ledger clock ticks
in seconds, which is coarse for low rates: a per-second fixed-point rate loses
precision at the small end of the curve. True exponential compounding
(`e^(rate·t)`) has no exact integer form, so any implementation picks an
approximation and a rounding direction; that direction must favor the
protocol's solvency, and the approximation error must stay bounded no matter
how large the elapsed time is.

## Decision

The pool's clock is milliseconds: `contracts/pool/src/time.rs::now_ms`
multiplies `env.ledger().timestamp()` by `MS_PER_SECOND` with a checked
multiply. Rate parameters are stored as annual RAY values; the kinked
utilization curve computes the annual rate and converts it once per
evaluation — the final step of
`common/src/rates/curve.rs::calculate_borrow_rate` divides by
`common/src/constants/shared.rs::MILLISECONDS_PER_YEAR` (31_556_926_000),
yielding a per-millisecond RAY rate.

Accrual is chunked. `contracts/pool/src/interest.rs::global_sync` walks the
elapsed time in pieces of at most
`common/src/rates/compound.rs::MAX_COMPOUND_DELTA_MS` (one year); each
`contracts/pool/src/interest.rs::accrue_chunk` recomputes utilization and the
borrow rate for that chunk, so a long-stale market re-prices per simulated
year instead of freezing one rate across the whole gap. The per-chunk growth
factor is `common/src/rates/compound.rs::compound_interest`: an unrolled
8-term Taylor series of `e^x` for `x = rate · delta_ms`, with no
data-dependent branches beyond the `delta_ms == 0` fast path. Truncating the
series under-estimates `e^x` for positive rates — the error direction that
charges borrowers slightly less rather than crediting suppliers value that
was never collected.

Value conservation is explicit. The supplier reward that the floor-rounded
supply-index update fails to distribute is measured by
`common/src/rates/index.rs::supply_index_reward_shortfall` and booked to
protocol revenue together with the reserve-factor fee in `accrue_chunk`, so
no accrued interest is destroyed by rounding. Both indexes are clamped:
`common/src/rates/index.rs::update_borrow_index` and `::update_supply_index`
cap at `MAX_BORROW_INDEX_RAY` / `MAX_SUPPLY_INDEX_RAY` (1e36 raw), keeping
downstream share arithmetic inside the i128 domain.

## Alternatives

- **Per-second rates (the ledger's native unit).** No ×1000 clock synthesis
  and marginally simpler constants, but the per-unit rate for low APRs loses
  a decimal-order of precision in RAY fixed point, and every stored curve
  parameter's effective resolution shrinks with it. The millisecond domain
  costs one checked multiply per read and buys three orders of rate
  granularity.
- **Single-shot compounding over arbitrary gaps.** One `compound_interest`
  call for the whole elapsed time is cheaper for stale markets, but it
  freezes the rate observed at the start across the entire gap — a market
  that sat at high utilization for a year would accrue at whatever rate the
  last touch saw — and the Taylor truncation error grows with `x`, so an
  unbounded exponent has unbounded under-estimation. The year cap bounds both
  the rate-staleness and the per-chunk series error.
- **Linear (simple-interest) accrual between touches.** Cheapest of all and
  used by some lending pools, but it makes realized interest depend on how
  often the market is touched: frequent touchers compound, idle markets do
  not. Chunked exponential accrual makes the outcome a function of elapsed
  time alone.

## Consequences

Easy: accrual is deterministic in elapsed time, not in touch frequency;
stale markets catch up correctly inside one transaction with a loop bounded
by elapsed-years; the branch-free series body keeps execution cost flat and
predictable. Rounding losses are routed to revenue rather than leaked, which
is what the ACCT-domain conservation checks assume (see
../../reference/invariants.md §INV-ACCT).

Hard: every consumer must know the unit convention — stored curve parameters
are annual, the curve's output is per-millisecond, and off-chain APY math
must reproduce the same division and the same truncated series to match
on-chain indexes (`common/src/rates/simulate.rs` exists for exactly this).
Changing any piece — the year constant, the chunk cap, the term count —
re-scales observable interest and invalidates the curve and compounding
proofs together.

Must stay true: the series remains an under-estimate for positive rates and
the borrow index remains non-decreasing up to its clamp (see
../../reference/invariants.md §INV-IDX); the shortfall booking keeps
distributed rewards plus revenue equal to accrued interest; and the index
clamps stay above any economically reachable value, since hitting them stops
further accrual by construction.
