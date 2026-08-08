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

All multiplication and division uses widened non-negative arithmetic. Results
outside the supported signed-integer domain revert unless a formula explicitly
uses a saturating cap.

## Rounding vocabulary

- half_up(x × y ÷ d): nearest integer, with a half rounded upward.
- floor(x × y ÷ d): discard the fractional remainder.
- ceil(x × y ÷ d): add one when a non-zero remainder exists.

## Scaled balances

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

These directions favor the pool at the conversion boundary. A positive token
movement that would change zero shares is rejected.

A full withdrawal burns all supply shares and pays the floor-valued balance. A
full repayment burns all debt shares and refunds any excess payment.

## Interest rate

Utilization is:

```rust
let utilization = half_up(actual_borrowed * RAY / actual_supplied);
```

It is zero when supplied value is zero and is capped at one RAY for rate
selection.

The annual borrow curve has three continuous regions: below the mid point,
between mid and optimal, and above optimal. Each region begins at the
cumulative height of the prior regions. The selected annual rate is capped and
converted to a per-millisecond rate:

```rust
let rate_per_ms = half_up(annual_rate / milliseconds_per_year);
```

The displayed deposit rate is approximately:

```rust
let deposit_rate = half_up(
    (BPS - reserve_factor) * utilization * borrow_rate / RAY / BPS,
);
```

It is a view. Realized supplier return also reflects the conservative
rounding remainder retained as protocol revenue.

## Accrual

When no time has elapsed, accrual changes nothing. Otherwise elapsed time is
processed in bounded chunks. For each chunk:

1. Calculate a capped compound factor from the per-millisecond rate.
2. Update the borrow index by half-up multiplication with that factor.
3. Calculate accrued interest from the change in borrowed value.
4. Split accrued interest between supplier reward and protocol revenue.
5. Update the supply index conservatively and record any distribution
   shortfall as revenue.

The borrow index never decreases. The supply index may decrease when eligible
bad debt is socialized, but cannot fall below its configured floor.

## Valuation and health

For each position:

```rust
let value_usd = token_amount * price / token_scale;
```

Collateral contributions use floor rounding. Debt contributions use ceil
rounding. The health factor is:

```rust
let health_factor = floor(weighted_collateral / total_debt);
```

A debt-free account reads as maximally healthy. An account is liquidatable only
when it has debt and health factor is below one WAD.

## Risk gates

A risk-increasing operation must leave:

- debt no greater than LTV-weighted collateral;
- health factor at least one WAD;
- borrowed collateral above the minimum collateral floor; and
- all relevant position, cap, listing, and pause rules satisfied.

Risk parameters must maintain a strict separation between LTV and liquidation
threshold. Liquidation terms must leave room for the bonus and protocol fee.

## Liquidation

The liquidator’s repay amount is bounded by the protocol’s close target. The
collateral base is derived from repaid debt and the price ratio. The bonus is
added within the configured bonus cap; the protocol fee applies only to that
bonus portion.

If received repayment is less than planned, every related seizure is reduced
proportionally with floor rounding. A seizure never exceeds the current
position. Excess repayment is refunded.

Tiny residual positions can be promoted to a full close where partial
arithmetic would otherwise leave unusable dust.

## Bad debt

After liquidation, socialization is allowed only for eligible residual debt.
The market supply index is reduced proportionally to the remaining value:

```rust
let new_supply_index = floor(old_supply_index * remaining_value / total_value);
```

The result is clamped to the non-zero supply-index floor. The loss is confined
to suppliers in that market.

## Caps and fees

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
| Collateral valuation | floor | avoids overstating collateral |
| Debt valuation | ceil | avoids understating debt |
| Health factor | floor | avoids overstating health |
| Bad-debt write-down | floor with floor clamp | keeps loss explicit and domain safe |
| Cap conversion | floor | makes the cap slightly tighter |
| Partial revenue claim | ceil share burn | avoids overpaying treasury |
