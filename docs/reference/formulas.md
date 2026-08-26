# Formulas and rounding

This reference defines the arithmetic vocabulary used to review risk and
accounting. Rounding is part of the protocol policy, not an implementation
detail.

## Units

| Quantity | Unit |
|---|---|
| Interest indexes, scaled shares, utilization, rates | RAY = 10^27 |
| USD values and health factor | WAD = 10^18 |
| Risk ratios and fees | BPS = 10,000 |
| Token amounts | native token base units |
| Time | milliseconds |

Multiply-divide first attempts the whole computation in 128 bits and widens the
operands to 256 bits only when the intermediate product does not fit; both paths
return the same value, so the fast path is a cost optimisation, not a different
rounding. Decimal rescaling and divide-by-integer stay in 128 bits and revert on
overflow. Inputs are expected to be non-negative: the half-up path rejects
negatives, the floor and ceiling paths assume them. Results outside the supported
signed-integer domain revert unless a formula explicitly uses a saturating cap,
and a zero denominator reverts with `DivisionByZero`.

Implemented by: `common/src/math/fp.rs`, `common/src/math/fp_core.rs`.

## Rounding vocabulary

Implemented by: `common/src/math/fp_core.rs`.

- half_up(x × y ÷ d): nearest integer, with a half rounded upward.
- floor(x × y ÷ d): discard the fractional remainder.
- ceil(x × y ÷ d): add one when a non-zero remainder exists.

## Scaled balances

Implemented by: `common/src/rates/scaling.rs`.

Positions store scaled shares. Their current RAY value is:

```rust
let actual = half_up(scaled * index / RAY);
```

| Operation | Share conversion | Rounding |
|---|---|---|
| Supply | amount × RAY ÷ supply index | floor |
| Partial withdrawal | amount × RAY ÷ supply index | ceil |
| Borrow | amount × RAY ÷ borrow index | ceil |
| Partial repayment | amount × RAY ÷ borrow index | floor |
| Net settle size | min(request, floor(supply), ceil(debt)) | never settle unpayable supply |
| Net settle supply close | all shares iff overlap equals floor(supply) | no half-up promotion |
| Net settle debt close | all shares iff overlap equals ceil(debt) | no leftover dust when paid |

These directions favor the pool at the conversion boundary. A positive token
movement that would change zero shares is rejected.

A full withdrawal burns all supply shares and pays the floor-valued balance. A
full repayment burns all debt shares and refunds any excess payment.

## Interest rate

Implemented by: `common/src/rates/curve.rs`, `contracts/pool/src/cache/scale.rs`.

Utilization is:

```rust
let utilization = half_up(actual_borrowed * RAY / actual_supplied);
```

`actual_borrowed` and `actual_supplied` are the scaled totals multiplied by
their indexes, both in RAY, not token balances. Utilization is zero when
supplied value is zero and is capped at one RAY for rate selection.

The annual borrow curve has three continuous regions: below the mid point,
between mid and optimal, and above optimal. Each region begins at the
cumulative height of the prior regions. The selected annual rate is capped.
Pool view getters (`get_borrow_rate`, `get_deposit_rate`) return that annual
RAY value. Accrual converts it to a per-millisecond rate before compounding:

```rust
let rate_per_ms = half_up(annual_rate / milliseconds_per_year);
```

`milliseconds_per_year` is 31,556,926,000 — a 365.2422-day year.

The displayed deposit APR is approximately:

```rust
let rate_x_util  = half_up(utilization * annual_borrow_rate / RAY);
let deposit_apr  = half_up(rate_x_util * (BPS - reserve_factor) / BPS);
```

Two rounding steps, not one. A single composite-denominator division would
differ by one raw ray unit on a large share of inputs. The result is zero when
utilization is zero or when `reserve_factor` is outside `0..BPS`.

It is a view. Divide the returned RAY by `RAY` for a unit fraction
(0.05 = 5%). Realized supplier return also reflects the conservative
rounding remainder retained as protocol revenue.

## Accrual

Implemented by: `contracts/pool/src/interest.rs`, `common/src/rates/compound.rs`,
`common/src/rates/index.rs`.

When no time has elapsed, accrual changes nothing. Otherwise elapsed time is
processed in bounded chunks. For each chunk:

1. Calculate the compound factor as an eighth-order Taylor expansion of
   `e^(rate_per_ms x chunk_ms)`. The factor itself is not capped; the chunk is
   capped at one year, the rate at the market's configured maximum, and the
   resulting index at the borrow-index ceiling.
2. Update the borrow index by half-up multiplication with that factor.
3. Calculate accrued interest from the change in borrowed value.
4. Split accrued interest between supplier reward and protocol revenue.
5. Update the supply index conservatively and record any distribution
   shortfall as revenue.

The borrow index never decreases. The supply index may decrease when eligible
bad debt is socialized, but cannot fall below its configured floor.

## Valuation and health

Implemented by: `common/src/rates/value.rs`, `common/src/types/oracle.rs`,
`contracts/controller/src/risk/totals.rs`.

A position stores scaled shares, so its value takes three steps. A loose token
payment takes one:

```rust
// position (scaled shares)
let actual    = scaled_shares * index / RAY;   // RAY
let value_usd = wad(actual) * price / WAD;     // WAD

// loose token payment
let value_usd = token_amount * price / 10^asset_decimals;
```

`index` is the market's supply index for collateral and its borrow index for
debt. Each call rounds one way at every step: half up, floor, or ceiling.

Collateral is valued two ways. The gated sums that back the risk checks —
LTV-weighted collateral and liquidation-threshold-weighted collateral — floor at
every step. The plain collateral total, which sizes the liquidation seizure
share and the bad-debt dust test, rounds half up. Debt contributions to the risk
totals use ceiling rounding; the read-only debt view rounds half up and is not a
solvency input. The health factor is:

```rust
let health_factor = floor(weighted_collateral / total_debt);
```

`weighted_collateral` is the sum over collateral positions of the floor-valued
position multiplied by that position's liquidation threshold, floored. The
division truncates toward zero and saturates at the top of the signed domain
instead of reverting.

A debt-free account reads as maximally healthy. An account is liquidatable only
when it has debt and health factor is below one WAD.

## Risk gates

Implemented by: `contracts/controller/src/risk/validation.rs`,
`common/src/validation.rs`, `contracts/controller/src/spoke_usage.rs`.

A risk-increasing operation must leave:

- debt no greater than LTV-weighted collateral;
- health factor at least one WAD;
- LTV-weighted collateral at least the configured minimum-collateral floor, when
  that floor is non-zero; and
- all relevant position, cap, listing, and pause rules satisfied.

Risk parameters must keep the liquidation threshold strictly above the LTV, and
must satisfy `threshold x (1 + bonus) <= 100%`. The protocol's liquidation fee
comes out of the bonus, not on top of it, and must stay strictly below 100%.

## Liquidation

Implemented by: `contracts/controller/src/positions/liquidation/math.rs`,
`contracts/controller/src/positions/liquidation/curve.rs`,
`contracts/controller/src/positions/liquidation/apply.rs`.

The liquidator’s repay amount is bounded by the protocol’s close target. The
collateral base is derived from repaid debt and the price ratio. The bonus is
added within the configured bonus cap; the protocol fee applies only to that
bonus portion.

If received repayment is less than planned, every related seizure is reduced
proportionally with floor rounding. A seizure never exceeds the current
position. Repayment above the ideal close amount is never pulled: the plan trims
each `RepayEntry` (and drops whole legs) before transfer, so the liquidator
simply keeps the excess. The trimmed amounts are reported by the
`liquidation_estimations_detailed` view as `refunds`; no refund transfer occurs.

Tiny residual *debt* after the computed ideal can raise that ideal to a full
close, so a liquidator *may* finish a dust stub. The offer is still capped at
the ideal; leftover debt is socialized separately after the call, but only when
that debt exceeds the leftover collateral and that collateral is at or below the
dust threshold.

## Bad debt

Implemented by: `contracts/pool/src/interest.rs`,
`contracts/controller/src/positions/liquidation/mod.rs`,
`contracts/controller/src/positions/liquidation/curve.rs`.

After liquidation, socialization runs automatically only for eligible residual
debt: debt above the remaining collateral, with that collateral at or below the
dust threshold. A separate owner-only entry point (`force_socialize_bad_debt`)
socializes any account whose debt exceeds its collateral, without the dust cap.

The market supply index is reduced proportionally to the remaining value:

```rust
let reduction_factor  = floor(remaining_value * RAY / total_value);
let new_supply_index  = floor(old_supply_index * reduction_factor / RAY);
```

`remaining_value` is the total supplied value less the bad debt, capped at that
total, both in RAY. The two floors compound, so the written-down index is at
most the single-step value; the extra truncation falls on suppliers, never on
the protocol. The result is clamped to the non-zero supply-index floor. The loss
is confined to suppliers in that market.

## Caps and fees

Implemented by: `common/src/rates/scaling.rs`,
`contracts/controller/src/spoke_usage.rs`, `common/src/math/fp.rs`,
`contracts/pool/src/cache/shares.rs`.

Supply and borrow caps are native-asset limits. Before exposure grows, the
limit and usage are converted using the current index. Zero means no new
exposure. Exits reduce usage and do not consume a cap.

Flash and strategy fees are BPS of the relevant amount, rounded half-up and
raised to one token unit when a positive fee would otherwise round to zero.
Revenue claims burn enough revenue shares to cover their payout and cannot
pay more cash than the market holds.

## Rounding review table

| Boundary | Direction | Safety effect |
|---|---|---|
| Supply mint | floor | avoids over-crediting suppliers |
| Withdrawal share burn | ceil | avoids under-burning shares |
| Debt mint | ceil | avoids under-recording debt |
| Repayment share burn | floor | avoids erasing unpaid debt |
| Collateral valuation (gated sums) | floor | avoids overstating collateral |
| Collateral total (seizure share, dust test) | half-up | share of portfolio, not a solvency bound |
| Debt valuation (risk totals) | ceil | avoids understating debt |
| Health factor | floor | avoids overstating health |
| Bad-debt write-down | two floors with floor clamp | keeps loss explicit and domain safe |
| Net settle overlap | min(request, floor supply, ceil debt) | closes a side only when that side is exhausted |
| Cap conversion | floor | makes the cap slightly tighter |
| Partial revenue claim | ceil share burn | avoids overpaying treasury |
