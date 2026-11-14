# Formulas and rounding

This reference defines the units, rounding and arithmetic limits used to value
positions and settle funds. [Runtime invariants](invariants.md) identify the
operations that enforce each constraint.

Expressions use raw integers. `half_up`, `floor` and `ceil` apply to the entire
fraction; the pseudocode does not imply unchecked Rust multiplication.

## Units and arithmetic domain

| Quantity | Scale |
|---|---|
| Shares, indexes, asset values used by the rate engine, rates, utilization | RAY = 10^27 |
| USD values, prices per whole token, health factor | WAD = 10^18 |
| Risk ratios and fees | BPS = 10,000 |
| Transfers and accounting cash | Native token base units |
| Accrual time | Milliseconds; year = 31,556,926,000 ms |

Protocol boundaries require non-negative amounts; `Ray`, `Wad` and `Bps`
constructors do not enforce that restriction themselves. Multiply-divide uses
an `i128` fast path or an exact `I256` intermediate. Unrepresentable results
raise `MathOverflow`, except at explicit saturating sites; zero divisors raise
`DivisionByZero`.

Half-up multiply-divide requires non-negative operands and a positive divisor.
Raw floor and ceiling multiply-divide also support signed quotients. Decimal
upscaling uses checked `i128` arithmetic. Downscaling and division by a positive
integer support the signed extremes; signed half-up downscaling rounds exact
halves away from zero. Floor decimal downscaling (`rescale_floor`,
`to_wad_floor`, `to_asset_floor`, `to_token_floor`) truncates toward zero.
That is a floor for the non-negative amounts at protocol boundaries.

## Shares and token amounts

For an asset with `d` decimals, first normalize token units into RAY. Multiplying
shares by an index returns a RAY asset value, requiring a second conversion
before a token transfer:

```rust
let amount_ray = token_units * 10_i128.pow(27 - d); // exact, checked
let value_ray = round(shares_ray * index_ray / RAY);
let token_units = round(value_ray / 10_i128.pow(27 - d));
let shares_ray = round(amount_ray * RAY / index_ray);
```

| Boundary | Direction |
|---|---|
| Supply mint | floor |
| Partial withdrawal burn | ceil |
| Debt mint | ceil |
| Partial repayment burn | floor |
| Supply claim paid in tokens | floor at both conversion steps |
| Debt full-close amount | ceil at both conversion steps |
| Displayed balance | half-up at both conversion steps |

A withdrawal request of at least the half-up displayed supply balance burns all
supply shares and pays their floor-valued balance. A repayment of at least the
ceiled debt balance burns all debt shares and refunds the excess.

Positive supply and borrow amounts must mint shares. Positive net repayment
and gross withdrawal amounts must burn shares. A zero-share result reverts at
these boundaries.

### Same-asset net settlement

Net settlement offsets supply against debt without moving cash:

```rust
let overlap = min(request, min(floor(supply), ceil(debt)));
```

The overlap burns all shares on a side when it exhausts that side's conservative
value. Partial settlement ceils the supply-share burn and floors the debt-share
burn, each capped at the held shares. Positive settlement must burn shares on
both sides.

## Cash, supply, debt, and revenue

Revenue is a supply-share claim included in total supplied shares. Revenue
minting increases both totals equally; claiming revenue burns both equally.
Reclassifying seized collateral as revenue leaves total supply unchanged.
Tracked cash is a separate reserve balance that incidental token donations do
not increase.

### Backing and cash constraints

The market's backing check uses native units:

```rust
let shortfall = max(0, floor(supply_value_ray - ceil_ray(debt_value)) - cash);
```

The RAY-precision gap rounds down to native units once; the cash subtraction
saturates. Supply entry rejects a positive shortfall. Recapitalization credits
at most that shortfall, refunds excess and mints no shares.

Borrow draws must retain a 200 BPS liquidation buffer, rounded up from the
floored supplied token value. Borrow, user withdrawal and revenue claims
enforce the configured utilization ceiling; liquidation withdrawal skips it.
The gate compares and divides nothing:

```rust
let allowed = floor(floor(supply_value_ray) * max_utilization_ray / RAY);
let passes = ceil(debt_value_ray) <= allowed;
```

Debt value rounds up and the allowed side rounds down. Rounding therefore
cannot admit utilization above the ceiling. Debt against a zero floored supply
value fails the gate. Zero supplied shares and zero debt value skip the gate. A debt mint
(borrow or strategy opening) is checked at every ceiling: at 100% it requires
debt value at most supply value. A withdrawal or revenue claim skips a 100%
ceiling, because cash already bounds it.
Withdrawal, net settlement and revenue claims reject zero total supply with
outstanding debt. These checks apply at their respective boundaries; they do
not establish full backing after every mutation.

### Revenue payout

Payout is limited by cash and the floor-valued revenue claim:

```rust
let payout = min(cash, floor(revenue_value));
let burned = if payout == floor(revenue_value) { revenue_shares }
    else { ceil(revenue_shares * payout / floor(revenue_value)) };
```

The share-burn calculation applies to a positive payout. A positive payout
cannot burn zero shares.

## Rates and accrual

### Utilization and annual rates

Utilization is half-up-valued debt over half-up-valued total supply, capped
at one RAY:

```rust
let utilization_ray = if supply_value_ray == 0 { 0 }
    else if debt_value_ray >= supply_value_ray { RAY }
    else { half_up(debt_value_ray * RAY / supply_value_ray) };
```

Only a debt value below the supply value is divided. The quotient therefore
cannot overflow, and the result is always in `[0, RAY]`. Dust supply against
residual debt cannot stop accrual. Rate selection also caps utilization at one
RAY.

The annual borrow curve has three joined linear regions:

1. base plus slope1 up to mid utilization;
2. slope2 up to optimal;
3. slope3 up to full utilization.

Each slope contribution is one half-up multiply-divide,
`slope * excess / width`. Each region therefore ends exactly on its slope, and
the rate never falls as utilization rises. The result cannot exceed the
configured maximum, which is limited to 200% APR. Parameter validation requires
`base + slope1 + slope2 + slope3 <= max`, so the maximum never cuts the curve
before full utilization.

Validation also requires that the curve does not flatten at a kink. Each slope
is the rise across its whole segment. Validation therefore compares steepness
per unit of utilization:
`slope1 / mid <= slope2 / (optimal - mid) <= slope3 / (RAY - optimal)`. It
compares exactly and without division
(`slope1 * (optimal - mid) <= slope2 * mid`, and the same at optimal). Equal
steepness is allowed, so a straight line through a kink passes. Slopes must
also be non-negative, at most the max rate, and `slope1` at least the base rate.
Together these imply `base + slope1 + slope2 < max`. The segment above
optimal therefore always has room below the cap. A positive `slope3` leaves
that room. `slope3 = 0` forces `slope1 = slope2 = 0`, and then `max > base`
leaves it.

Accrual uses the per-millisecond rate `per_ms`. Pool borrow and deposit rate
views return annual RAY fractions from stored indexes, without first accruing
or projecting them:

```rust
let per_ms = half_up(annual_borrow_rate_ray / 31_556_926_000);
let rate_x_util = half_up(utilization_ray * annual_borrow_rate_ray / RAY);
let deposit_apr_ray = half_up(rate_x_util * (BPS - reserve_factor_bps) / BPS);
```

Deposit APR has two rounding steps and is not an exact realized supplier return.
It is zero at zero utilization or an out-of-range reserve factor, and never
above the borrow APR, since utilization is at most one RAY.

### Compounding and interest allocation

Accrual processes elapsed time in chunks of at most one year. Each chunk uses
its starting utilization and rate to approximate
`exp(per_ms * delta_ms / RAY)` through the eighth-order Taylor term, with
fixed-point half-up arithmetic. No elapsed time means no accrual. Index
projections and mutations use the same step calculation.

Borrower interest is the difference between half-up-valued debt at the new and
old borrow indexes. The reserve factor allocates a half-up protocol fee; the
remainder is supplier rewards.

The supply index floors the new total value divided by supplied shares, bounded
by the old index and the supply-index ceiling. Rewards not reflected in that
index change join the protocol reward. Revenue shares are floor-converted at
the new supply index and capped by remaining total-supply share headroom. This
last floor and cap can leave reward value unrepresented by revenue shares.
Exact conservation of booked supplier and revenue value is not guaranteed.

Accrual does not overflow on a market value. Each chunk takes the larger of
total debt value and total supply value, both half-up. It measures the room
that this value has below the value ceiling. It caps the borrow index so
borrower interest is at most that room:

```rust
let room = max(0, MAX_MARKET_VALUE_RAY - max(debt_value_ray, supply_value_ray));
let new_borrow_index = min(grown_borrow_index,
    old_borrow_index + floor(room * RAY / debt_shares));
```

`MAX_MARKET_VALUE_RAY` is `i128::MAX - RAY`. Interest raises both totals by
the same amount, so neither crosses the ceiling. With no room, or with a total
whose value does not fit `i128`, the chunk changes no index and books no
revenue; time still advances. Borrowers then accrue less than the rate
implies, and the pool emits `MarketValueCeilingEvent`.

### Accrual cadence

Call cadence can change borrower cost. Each chunk holds its starting rate
constant; subsequent chunks recalculate utilization. Taylor truncation and
integer rounding also depend on how time is partitioned. A finer partition
does not generally guarantee the same result, a larger result or an exact
continuous-exponential bound.

## Valuation and health

A position uses one rounding direction at all three boundaries:

```rust
let asset_ray = round(shares_ray * index_ray / RAY);
let asset_wad = round(asset_ray / 1_000_000_000);
let value_usd_wad = round(asset_wad * price_wad / WAD);
```

Risk collateral floors all three valuation steps. Risk BPS weights convert
BPS→WAD half-up, then multiply that ratio into the floored position value with
floor rounding (`Bps::apply_to_wad_floor`). LTV weighting uses the position's
stored `min(LTV, liquidation_threshold)`; health weighting uses its stored
liquidation threshold. Risk debt rounds upward, while the separate debt display
rounds half-up. The unweighted collateral total also rounds half-up and is used
for liquidation proportions and dust eligibility.

```rust
let health_factor_wad = floor(weighted_collateral_wad * WAD / debt_wad);
```

Health factor saturates at `i128::MAX`; debt-free accounts use that sentinel.
Liquidation requires debt and health below one WAD. Risk-increasing actions
with debt remaining require debt within LTV collateral, health at least one
WAD and any configured minimum LTV collateral. Listing, cap and pause gates
also apply; [risk invariants](invariants.md#inv-risk-01) define their scope.

`price_wad` has its own rounding. A Reflector `Twap` mean is floored at the
feed's precision. The two-source tolerance check rounds the leg ratio half-up,
so it accepts an exact ratio less than 0.5 BPS above `upper_ratio_bps`. The
two-source midpoint rounds down; a leg sum that overflows `i128` gives zero,
which fails as `InvalidPrice` (217). A stale leg (`PriceFeedStale`, 206) and
a tolerance breach (`UnsafePriceNotAllowed`, 205) come first when they also
apply; [price sources](#price-sources) gives the order.

## Price sources

The price aggregator gives one `price_wad` and one status for each asset. A
read reports the first failure in this order:

1. an error from the source read;
2. no configuration: `OracleNotConfigured` (216);
3. a stale leg: `PriceFeedStale` (206);
4. a tolerance breach: `UnsafePriceNotAllowed` (205);
5. a price that is not positive: `InvalidPrice` (217);
6. a price outside the sanity bounds: `SanityBoundViolated` (223).

Two sources blend into one price:

```rust
ratio_bps = half_up(high * BPS / low)
deviation = low <= 0 || ratio_bps > upper_ratio_bps
price_wad = floor((a + b) / 2)
timestamp = min(timestamp_a, timestamp_b)
stale     = stale_a || stale_b || spread_too_wide
```

`high` and `low` are the larger and the smaller leg. A ratio that overflows
also sets `deviation`. The last term of `stale` applies only when the two
legs are market sources. It is true when their timestamps are more than
`MAX_LEG_AGE_SPREAD_SECONDS` (3600) apart. A leg sum that overflows `i128`
gives a zero price.

Admission accepts a tolerance only in this form, else `BadLastTolerance`
(208):

```rust
BPS + MIN_TOLERANCE <= upper_ratio_bps <= BPS + MAX_TOLERANCE
BPS - MAX_TOLERANCE <= lower_ratio_bps <= BPS
lower_ratio_bps == half_up(BPS * BPS / upper_ratio_bps)
```

`MIN_TOLERANCE` is 150 and `MAX_TOLERANCE` is 2500.

Admission measures the sanity band as a ratio of the sum of its two bounds:

```rust
0 < min < max <= MAX_REASONABLE_PRICE_WAD
floor((max - min) * BPS / (max + min)) >= MIN_SANITY_BAND_BPS
ceil((max - min) * BPS / (max + min)) <= band_cap
```

The first two lines fail with `InvalidSanityBounds` (224);
`MIN_SANITY_BAND_BPS` is 50. The third line fails with
`SanityBandTooWideForSingleSource` (226). The cap is
`MAX_LP_SANITY_BAND_BPS` (8182) for an LP source and
`MAX_SINGLE_SOURCE_SANITY_BAND_BPS` (1000) for all other configurations,
unless a second source with a different trust set is present. That pair has
no cap.

A scaled source multiplies a factor feed by the price of a quote asset:

```rust
price_wad = half_up(factor_wad * quote_wad / WAD)
timestamp = min(factor_timestamp, quote_timestamp)
```

A factor outside `[min_factor_wad, max_factor_wad]` fails with
`FactorOutOfBounds` (230). An overflow fails with `InvalidPrice` (217). An
error from the quote read goes through unchanged. Only the factor leg sets
the stale flag.

A Reflector `Twap(records)` source has these admission limits:

```rust
MIN_ORACLE_RESOLUTION_SECONDS <= resolution <= max_stale
1 <= records <= MAX_TWAP_RECORDS
required_span = (records - 1) * resolution <= max_stale
```

`MIN_ORACLE_RESOLUTION_SECONDS` is 60 and `MAX_TWAP_RECORDS` is 12. The
resolution and span limits fail with `InvalidOracleResolution` (222). Zero
records fail with `TwapInsufficientObservations` (219) and too many with
`TwapRecordsOutOfRange` (228).

The read then requires a history that is not empty, else
`ReflectorHistoryEmpty` (212). It rejects more than `records + 1` samples,
adjacent samples closer than `resolution`, and a newest-to-oldest span below
the required span, each with `TwapInsufficientObservations` (219). A sample
from the future fails with `PriceFeedStale` (206). All samples must be
positive.

```rust
mean      = floor(sum(samples) / count(samples))
price_wad = mean * 10^(18 - feed_decimals)
timestamp = oldest sample
```

The upscale is exact; a failure gives `InvalidPrice` (217).

LP share prices use the prices of the pool's underlying tokens.

Constant-product pool, with `isqrt` as the floor of the square root in 256
bits:

```rust
value_i   = half_up(reserve_i * price_i_wad / 10^decimals_i)
price_wad = floor(2 * isqrt(value_a * value_b) * WAD / share_supply_wad)
```

Stable two-coin pool, with the reserves upscaled to WAD and `D` as the
stable-swap invariant:

```rust
price_wad = floor(D * min(price_a_wad, price_b_wad) / share_supply_wad)
```

`D` comes from Newton-Raphson with `Ann = amp * 2`. It starts at the reserve
sum and stops at a step of 1 or less, after at most 255 iterations.
`amp` must be in `[MIN_AMP, MAX_AMP]`, 1 to 1,000,000, and each upscaled
reserve at most `MAX_NORMALIZED_RESERVE_WAD`, 10^34. A reserve, price or
share supply that is not positive, or a limit breach, fails with
`InvalidPrice` (217).

An Aquarius pool also has a size floor:

```rust
pool_value_wad = half_up(price_wad * total_shares / 10^share_decimals)
```

A value below `min_pool_value_wad` fails with
`InsufficientAquariusLiquidity` (235). The timestamp is the older of the two
leg timestamps.

A Curve pool with two or three coins uses its `get_virtual_price` result:

```rust
price_wad      = floor(virtual_price * min(coin_price_wad) / WAD)
pool_value_wad = floor(price_wad * total_supply / WAD)
```

A value below `min_pool_value_wad` fails with `InsufficientCurveLiquidity`
(237). The timestamp is the oldest of the ledger time and the coin
timestamps.

## Liquidation sizing and fees

Liquidation calculates a bonus, sizes repayment toward a target health factor,
then distributes seizure across collateral. The quote can differ from final
settlement because of rounding and measured token receipt. An estimate read
at an earlier ledger also differs by the interest accrued before execution.

### Bonus and target repayment

| Symbol | Meaning, in raw WAD |
|---|---|
| `D` | Risk-valued debt |
| `C` | Unweighted collateral |
| `W` | Liquidation-threshold-weighted collateral |
| `HF` | Health factor |
| `H` | Target health factor |
| `b` | Selected bonus converted from BPS |

The ratio `p` is the account's blended liquidation threshold. This pseudocode
shows each WAD rescaling and half-up rounding step. For the selected bonus,
the collateral-backed repayment limit and target repayment are:

```rust
let p = if C == 0 { 0 } else { half_up(W * WAD / C) };
let hf_bonus_cap_bps = floor(HF * BPS / p) - BPS; // only p > 0 and HF < WAD
let weighted_seizure = half_up(p * (WAD + b) / WAD);
let target_debt = half_up(H * D / WAD);
let backed_max = min(D, half_up(C * WAD / (WAD + b)));
let ideal = if H <= weighted_seizure || target_debt <= W { backed_max }
    else { min(backed_max, half_up((target_debt - W) * WAD / (H - weighted_seizure))) };
```

The base bonus averages the stored collateral bonuses by half-up-valued USD
weight. It sums each leg's `value * bonus_bps` exactly. It then divides by
total collateral once, rounding half-up. It is bounded by
`floor(BPS * (BPS - t) / t)`, where
`t = clamp(ceil(p * BPS / WAD), 1, BPS)`. Zero `p` gives a zero threshold bonus
bound; zero collateral also gives a zero base bonus.

The configured curve ramps the base-to-maximum increment as health falls, then
applies its BPS factor `f`. `H` is the spoke's `liquidation_target_hf_wad` and
`K` is its `hf_for_max_bonus_wad`:

```rust
let s = if HF >= H { 0 } else { min(WAD, half_up((H - HF) * WAD / (H - K))) };
let curve = base + half_up(f * half_up((max - base) * s / WAD) / BPS);
```

The ramp starts at `H`, not at `HF = 1`. An account becomes liquidatable only
below `HF = 1`. Its first bonus is therefore already about
`base + f * (max - base) * (H - 1) / (H - K)`. The maximum comes only from the
blended threshold, not from the listed `liquidation_bonus`: `LT = 6000` gives
`max = 6666`. Take `H = 1.10`, `K = 0.80`, `f = 10000` and `base = 500`. The
bonus is then about 2555 BPS just below `HF = 1` and 4611 BPS at `HF = 0.90`.
Size `H`, `K` and `f` from the bonus at `HF = 1` and at `K`, not from the
listed bonus. For example, `LT = 5300` gives `max = 8867`. The curve
`H = 1.06`, `K = 0.90`, `f = 598` with `base = 500` then gives a bonus of
about 688 BPS just below `HF = 1` and 1000 BPS at `HF = 0.90` and below.

The HF-preserving cap above limits the result. With a
positive `p`, insolvency or a cap below base bypasses the target formula:

```rust
let quote = if p > 0 && C < D { (min(D, floor(C * WAD / (WAD + base))), base) } // insolvent
    else if p > 0 && cap < base { (D, max(cap, 0)) } // band: D <= C < D * (1 + base)
    else { /* target formula at min(curve, cap), or at curve when p == 0 */ };
```

In the band the quote is the full debt at the cap, and any smaller payment is
accepted. A partial repayment `x` seizes `x * (1 + cap)`, at most
`x * C / D`. `C / D` and the health factor therefore do not fall. The BPS
floor of the cap can raise them slightly.

When the curve bonus is above the cap and the cap is at least base, the
target formula runs at the cap. There `p * (1 + b)` is about `HF`, so the
formula gives about `D`. The quote is the whole debt, and the seizure is about
`C`. The bonus is about `C - D`, the borrower's whole equity
([borrower protection](../explanation/threat-model.md#price-integrity-and-availability)).
Take a spoke with the curve 1.02/0.80/10000 and an account with one collateral
leg. The account gets a whole-debt quote below about HF 0.93 to 0.97, down to
insolvency. The exact HF depends on the leg's threshold and base bonus.

Insolvency is the exact unweighted comparison `C < D`. With positive `p`, the
insolvency branch quotes the repayment that the collateral backs at the base
bonus, floored. An offer above it is trimmed, so the liquidator never pays
more than it seizes. On an insolvent account the trim rounds each kept leg
down to whole base units of its token. The kept value therefore never exceeds
the quote. A leg whose kept amount rounds to zero is dropped, and its whole
offer is refunded. If no leg remains, `liquidate` reverts with
`InvalidPayments` (16), and the estimate shows a zero payment. This insolvency
branch does not promote the quote to full debt; bad-debt cleanup takes the unbacked
residue. With `p == 0`, the target formula and dust promotion below apply instead.

For positive `p`, `HF / p` approximates `C / D`. `HF` floors and `p` rounds
half-up, however. An account at `C == D`, or a few raw WAD units above it, can
therefore compute a cap of `-1`. Such a
covered account takes the band quote with the cap clamped to zero. A full close
repays all of `D`, so no debt is left to socialize. Seizure floors to whole
token units, so it takes `C` less at most one token unit per collateral leg.
That unit stays with the account as collateral. Bad-debt cleanup does not
sweep it, because the account has no debt.

An ideal residual debt strictly between zero and $5 also promotes the quote to
full debt, without requiring full funding. A [raised whole-unit quote](#raised-partial-quote) is not
promoted to full debt, so it can leave debt below $5. Each input is capped at
its leg's ceiling-rounded debt, and the excess is listed as a refund. A partial quote
also trims the inputs above the quote from the last leg backward before tokens
are pulled, and execution pulls the trimmed amount. On a solvent account the
trim floors the refund, so the kept amount can round up by one token unit. A
full-debt quote trims nothing. The per-leg ceilings can exceed `D` by unit
rounding. A leg paid at its ceiling is credited only the floor value of the
debt it clears, so the rounded-up fraction buys no collateral. Execution then
pulls each merged offered amount. The pool refunds what exceeds each leg's
debt, which is exactly the listed refund. Neither a full-debt quote nor the target
health factor guarantees an executed full close after rounding or
under-delivery.

### Whole-unit collateral below 3 decimals

A collateral leg whose token has fewer than 3 decimals seizes whole units
only. One whole unit is one base unit of the token, `10^-d` of a token for `d`
decimals.

A partial seizure of such a leg rounds as follows:

- When the repayment covers the whole debt, the leg rounds up to the next
  whole unit, capped at the held balance. The debt closes.
- Otherwise the leg rounds down. The dropped fraction's USD value, divided by
  `1 + bonus` and floored, is trimmed from the repayment and refunded.
- Kept amounts round up, so the liquidator pays for the units it receives.
- A plan that then seizes nothing reverts with `InvalidPayments` (16).

Neither rounding rule applies when an insolvent account's repayment reaches
the collateral-backed quote. That call seizes every unit. Such a leg is its
account's only supply position, so it is the whole collateral and the seizure
stays proportional.

A borrow, withdrawal or account strategy that leaves debt needs at least 2
whole units in the leg. A liquidation can leave fewer, and a `Credit`
receiver's units are not checked at credit.

#### Raised partial quote

The controller can change a partial quote for such a leg. All of these
conditions must be true:

- the account is solvent (`C >= D`);
- the curve quote is below `D`;
- the leg is the account's only supply position;
- the leg holds at least one whole unit.

The rules use these values, in raw WAD:

- `U` is the USD value of one whole unit.
- `b` is the bonus.
- `m = max(floor(U / 1e6), 1)`.
- `R` is the sum, over the account's debt legs, of the USD value of one base
  unit of that debt token. `R` is the per-leg ceiling rounding that a full
  close can record.

The controller applies the first rule that matches:

1. If `floor(U / (1 + b)) >= D + R`, the quote becomes `D`. The plan closes in
   full and the leg rounds up to one whole unit. The liquidator repays all
   debt and receives one unit.
2. If the curve quote seizes less than `U + m`, and
   `ceil((U + m) / (1 + b)) < D`, the quote rises. The new quote is
   `ceil((U + m) / (1 + b))`. The seizure takes one unit. It refunds the
   margin that it does not seize, rounded down to whole debt-token units.
3. Otherwise the curve quote stays.

Size offers for such a leg from `get_liquidation_estimate`, not from the
curve formula:

- The raised quote is a ceiling, not a floor. A smaller offer is not raised.
- An offer that backs less than one whole unit seizes nothing and reverts
  with `InvalidPayments` (16).
- An offer between `U / (1 + b)` and the raised quote still takes one unit.

#### Liquidatability of a whole-unit leg

The HF-preserving bonus cap keeps `1 + b <= C / D`, so the sale of one unit
does not lower `C / D`. Take a solvent account whose only supply leg is below
3 decimals and holds a whole unit. It stays liquidatable below `HF = 1`, by a
one-unit sale or by a full close.

There is one exception. While
`floor(U / (1 + b)) - R < D <= ceil((U + m) / (1 + b))`, neither rule 1 nor
rule 2 applies. If the curve quote then backs less than one unit, every offer
reverts until accrual or a price move ends that state.

#### Effective bonus of a rule-1 full close

- A rule-1 full close pays the liquidator one unit worth `U` for `D`. Its
  effective bonus is `U / D - 1`, not `b`.
- Take `k` held units and liquidation threshold `LT`. `HF < 1` then gives
  `k * U * LT < D`. The effective bonus is therefore at most about
  `1 / (k * LT) - 1`.
- The borrower loses `U - D * (1 + b)` above the normal bonus.
- `bonus_rate_bps` in the estimate and `bonus_bps` in `LiquidationEvent` show
  only `b`.
- A listing below 3 decimals has no liquidation fee, so no protocol fee
  applies to that excess.

### Seizure and fees

Seizure is proportional to collateral value and capped at held value. The bonus
is the capped seizure minus the floor-divided uncapped principal, bounded below
by zero. A collateral cap below principal leaves no bonus to charge.

Partial seizure floors the token amount and seized shares. Full seizure uses
the half-up token amount to request a full pool withdrawal. For Credit mode it
takes the exact held shares. The pool payout still floors the supply claim.
In Transfer mode the pool treats any request at or above the half-up held
balance as a full close. Take a held fraction below half a unit. A partial
request equal to the floored balance then burns every share. It pays that
floored balance, the amount the plan priced. The fraction stays in
the pool as cash that no share claims.
Bonus shares floor at the supply index and cannot exceed seized shares.
Zero-share or zero-token planned seizure legs are omitted.

Transfer fees apply BPS half-up to bonus RAY, then floor to token units. A
positive subunit fee becomes one unit. The fee is then capped at the whole
token units the pool pays above the principal:

```rust
let realised_excess = if paid_ray > base_ray {
    floor(paid_ray - base_ray)
} else { 0 };
let transfer_fee = min(bumped_fee, realised_excess);
```

`paid_ray` is the pool's gross payout and `base_ray` the uncapped principal,
both in RAY; `floor` rescales to token units. The planned transfer fee
therefore never takes the liquidator's leg below its principal; the payout
floor still can. Credit fees use the ceiling of bonus shares:

```rust
let credit_fee_shares = ceil(bonus_shares * fee_bps / BPS);
let credited_shares = seized_shares - credit_fee_shares;
```

Under-delivery floor-scales seizure amounts, transfer fees, seized shares and
bonus shares by measured/planned repayment USD. The transfer amount is also
capped at the floor asset value of the scaled shares. The fee is capped at
that amount. A leg below 3 decimals reverts ([INV-LIQ-03](invariants.md#inv-liq-03)).
Credit fees are recomputed from the scaled bonus shares.

Bad-debt cleanup has separate eligibility: ceil risk debt must exceed half-up
unweighted collateral, and permissionless cleanup requires collateral at or
below $5. Forced owner cleanup omits the collateral cap. See
[cleanup invariants](invariants.md#inv-liq-04) for authorization and account
deletion.

## Bad debt

Debt socialization lowers only the affected market's supply index. The debt
being removed is valued with a ceiled RAY multiplication. The write-down uses
half-up total supplied value, two floors, and a non-zero floor clamp:

```rust
let remaining_ray = total_supply_ray - min(bad_debt_ray, total_supply_ray);
let reduction_ray = floor(remaining_ray * RAY / total_supply_ray);
let new_supply_index = max(10_i128.pow(24),
    floor(old_supply_index * reduction_ray / RAY));
```

Zero supplied value makes the write-down a no-op. The two floors can impose
extra truncation on supply claims, including revenue, before the index clamp.
The non-zero floor can leave residual claims without backing. Supply then
fails the backing gate until recapitalization fills the shortfall. Eligibility
and account deletion follow the [cleanup rules](invariants.md#inv-liq-04).

## Swap router fees and splits

The swap router takes fees only when a referral is active. It calculates each
fee separately on the vault balance of the fee token:

```rust
static_fee   = floor(balance * static_fee_bps / 10_000)
referral_fee = floor(balance * fee_bps / 10_000)
```

`fee_bps` is the referral rate. A sum of the two rates above `FEE_CAP`
(1000 BPS) fails with `FeeTooHigh` (21).

A route splits by parts per million of the balance available when the
instruction runs:

```rust
amount = floor(available_balance * ppm / PPM_DENOMINATOR)
```

`PPM_DENOMINATOR` is 1,000,000. A weight of zero fails with `ZeroSplitPpm`
(11) and a weight above the denominator with `SplitPpmMismatch` (12). The
weights of a route do not have to add up to the denominator, because each
split applies to the balance that remains.

The router accepts a residual of intermediate tokens up to this allowance, in
token units:

```rust
allowance = max(floor(credited / RESIDUAL_PPM), RESIDUAL_DUST_FLOOR)
```

`RESIDUAL_DUST_FLOOR` is 1,000. A larger residual fails with
`ExcessiveResidual` (29).

<a id="numeric-limits"></a>

## Caps, fees, and numeric limits

A cap is in native token units. Entry compares stored scaled usage plus the
new scaled amount with the cap floor-converted at the current index. Zero cap
allows no positive exposure. Exits subtract usage without checking caps;
missing usage rows and zero exit deltas are no-ops. Cap→scaled conversion
saturates at `i128::MAX` (`calculate_scaled_cap`), so the entry check fails
open instead of trapping. A saturated scaled cap does not enforce the
configured asset-unit limit. An admitted cap saturates only at an index below
an eighth of one RAY. Only a bad-debt write-down moves the supply index below
one RAY. At its floor (`RAY / 1000`), a supply cap above about 1/125 of the
admitted maximum saturates. The pool's entry ceiling still bounds such a
market's share count. The borrow index never falls below one RAY, so an
admitted borrow cap cannot saturate. Position conversion still rejects
overflow.

Flash-loan and charged strategy fees are half-up BPS of principal, with a
minimum of one base unit for a positive rate. Flash position has no origination
fee; see [its settlement invariant](invariants.md#inv-strat-04).

| Bound | Consequence |
|---|---|
| Asset decimals 0..=18 | Exact token-to-RAY upscaling. Below 3: collateral only, no flash loans, no liquidation fee, its account's only supply position, at least 2 whole units after a borrow, withdrawal or strategy that leaves debt |
| Both indexes initially RAY; ceiling 10^36 | 10^9 times initial index; protocol constants |
| Supply-index floor 10^24 | At most 1,000 times the shares minted at index one for the same deposit |
| Borrow APR maximum 2 RAY | 200% annual rate; not a bound on balance growth alone |
| Token-to-RAY input maximum `i128::MAX / 10^(27-d)` | About 170.14 billion whole tokens, before other limits |
| Market value ceiling `i128::MAX - RAY` | About 170.14 billion whole tokens of total supply value or total debt value. Accrual holds interest there |
| Market entry ceiling `(i128::MAX - RAY) / 8` | About 21.27 billion whole tokens of total supply or total debt, as a value and as a share count. Also the admitted cap maximum |
| Supply at the supply-index floor | About 21.27 million whole tokens before the share count passes the entry ceiling |

The admitted cap maximum is the market entry ceiling in asset units,
`MAX_MARKET_ENTRY_RAY / 10^(27-d)`, a factor 8 below the value ceiling. One
year at the 200% APR maximum multiplies debt by less than `e^2 < 8`. A market
filled to the admitted cap therefore accrues for at least a year before
interest is held. At 20% APR, it accrues for about ten years.

Supply, borrow, strategy openings and flash loans check the entry ceiling
after they raise one of the market's totals. The check covers total supply
and total debt, each as a share count and as a half-up value. Above the
ceiling these operations fail with `MarketCeilingReached` (136). The share count is checked because a supply
index at its floor mints 1,000 shares per unit of value. Exits never check
it. Interest can take a market above the entry ceiling. That market takes no
new entry until exits bring it back under. It keeps accruing, repaying,
withdrawing and liquidating.

Accrual does not revert on a market value. At the value ceiling, as at the
borrow-index ceiling, further accrual produces no borrower interest. The value
ceiling emits `MarketValueCeilingEvent`
([INV-IDX-06](invariants.md#inv-idx-06)).

These are arithmetic limits, not recommended market sizes.

### Liquidation fixture

This fixture shows the native arithmetic of a $5 repayment. Each row takes
the decimals, bonus and fee of one mainnet listing, with ample collateral.

The USD prices are illustrative values chosen for the fixture, not live
market prices. For example, SolvBTC at 120,000 and XAUM at 6,000 are test
inputs. Five rows sit at their listing's `max_sanity_price_wad`. The fixture
tests also run each row at its listing's `max_sanity_price_wad`.

The profit column excludes network fees and slippage. The fixture models at
most two token-unit rounding costs per collateral leg.

| Collateral | Illustrative USD price | Seized token units | Fee units | Profit USD (rounded) |
|---|---:|---:|---:|---:|
| SolvBTC | 120,000 | 4,541 | 45 | 0.3952 |
| xSolvBTCSolvBTC_LP | 12,000 | 4,583 | 41 | 0.4504 |
| SPIKOUKTBL | 1.48035816 | 358,021 | 2,431 | 0.264006 |
| XAUM | 6,000 | 900,000 | 6,666 | 0.360004 |
| XLM | 1 | 54,500,000 | 540,000 | 0.3960 |
| PYUSD | 1.05 | 49,047,619 | 142,857 | 0.1350 |
| USST | 1.0897 | 4.818e18 | 2.294e16 | 0.2250 |

## Sources

- [Fixed-point arithmetic](../../common/src/math/fp_core.rs) and [share conversion](../../common/src/rates/scaling.rs).
- [Rate curve](../../common/src/rates/curve.rs), [compounding](../../common/src/rates/compound.rs), [index and reward calculations](../../common/src/rates/index.rs), and [accrual projection](../../common/src/rates/simulate.rs).
- [Position valuation](../../common/src/rates/value.rs); [listing admission and shared checks](../../common/src/validation.rs); [post-pool risk gates](../../contracts/controller/src/risk/validation.rs).
- [Liquidation planning and fees](../../contracts/controller/src/positions/liquidation/math.rs), [liquidation curve](../../contracts/controller/src/positions/liquidation/curve.rs), and [liquidation fixtures](../../contracts/controller/tests/positions/liquidation_math.rs).
- [Price resolution](../../contracts/price-aggregator/src/engine.rs), [tolerance check](../../contracts/price-aggregator/src/tolerance.rs), [Reflector reads](../../contracts/price-aggregator/src/providers/reflector.rs), [constant-product LP price](../../common/src/oracle/lp.rs), [stable LP price](../../common/src/oracle/lp_stable.rs), [Aquarius provider](../../contracts/price-aggregator/src/providers/aquarius.rs), and [Curve provider](../../contracts/price-aggregator/src/providers/curve.rs).
- [Swap router fees](../../contracts/swap-aggregator/src/fees.rs) and [router constants](../../contracts/swap-aggregator/src/constants.rs).
