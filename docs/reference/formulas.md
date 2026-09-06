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
halves away from zero.

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

A withdrawal request at least the half-up displayed supply balance burns all
supply shares and pays their floor-valued balance. A repayment at least the
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
let shortfall = max(0, floor(supply_value) - (cash + ceil(debt_value)));
```

Addition and subtraction saturate. Supply entry rejects a positive shortfall.
Recapitalization credits at most that shortfall, refunds excess and mints no
shares.

Borrow draws must retain a 200 BPS liquidation buffer, calculated half-up on
the floored supplied token value. Borrow, user withdrawal and revenue claims
enforce the configured utilization ceiling; liquidation withdrawal skips it.
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

Utilization divides half-up-valued debt by half-up-valued total supply:

```rust
let utilization_ray = half_up(debt_value_ray * RAY / supply_value_ray);
```

Zero supplied value gives zero utilization. Rate selection caps utilization at
one RAY. The annual borrow curve has three joined linear regions: base plus
slope1 up to mid utilization, slope2 up to optimal, then slope3 up to full
utilization. Each slope contribution uses half-up multiplication followed by
half-up division. The result cannot exceed the configured maximum, which is
limited to 200% APR.

Pool borrow and deposit rate views return annual RAY fractions using stored
indexes without first accruing or projecting them:

```rust
let per_ms = half_up(annual_borrow_rate_ray / 31_556_926_000);
let rate_x_util = half_up(utilization_ray * annual_borrow_rate_ray / RAY);
let deposit_apr_ray = half_up(rate_x_util * (BPS - reserve_factor_bps) / BPS);
```

Deposit APR has two rounding steps and is not an exact realized supplier return.
It is zero at zero utilization or an out-of-range reserve factor.

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

Risk collateral floors all three steps and its BPS weighting. LTV weighting
uses the position's stored `min(LTV, liquidation_threshold)`; health weighting
uses its stored liquidation threshold. Risk debt rounds upward, while the
separate debt display rounds half-up. The unweighted collateral total also
rounds half-up and is used for liquidation proportions and dust eligibility.

```rust
let health_factor_wad = floor(weighted_collateral_wad * WAD / debt_wad);
```

Health factor saturates at `i128::MAX`; debt-free accounts use that sentinel.
Liquidation requires debt and health below one WAD. Risk-increasing actions
with debt remaining require debt within LTV collateral, health at least one
WAD and any configured minimum LTV collateral. Listing, cap and pause gates
also apply; [risk invariants](invariants.md#inv-risk-01) define their scope.

## Liquidation sizing and fees

Liquidation calculates a bonus, sizes repayment toward a target health factor,
then distributes seizure across collateral. The quote can differ from final
settlement because of rounding and measured token receipt.

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
weight, using half-up division and multiplication. It is bounded by
`floor(BPS * (BPS - t) / t)`, where
`t = clamp(ceil(p * BPS / WAD), 1, BPS)`. Zero `p` gives a zero threshold bonus
bound; zero collateral also gives a zero base bonus.

The configured curve ramps the base-to-maximum increment as health falls, then
applies its BPS factor. The HF-preserving cap above limits the result. A cap
below base bypasses the target formula and quotes full debt at base bonus.
Only a nonnegative below-base cap rejects partial funding, with ceiling-USD
valuation tolerance for a rounding-only shortfall.

An ideal residual debt strictly between zero and $5 also promotes the quote to
full debt, without requiring full funding. Inputs are capped at actual debt and
trimmed before tokens are pulled. Neither a full-debt quote nor the target
health factor guarantees an executed full close after rounding or under-delivery.

### Seizure and fees

Seizure is proportional to collateral value and capped at held value. The bonus
is the capped seizure minus the floor-divided uncapped principal, bounded below
by zero. A collateral cap below principal leaves no bonus to charge.

Partial seizure floors the token amount and seized shares. Full seizure uses
the half-up token amount to request a full pool withdrawal and takes the exact
held shares for Credit mode; the pool payout still floors the supply claim.
Bonus shares floor at the supply index and cannot exceed seized shares.
Zero-share or zero-token planned seizure legs are omitted.

Transfer fees apply BPS half-up to bonus RAY, then floor to token units. A
positive subunit fee becomes one unit, capped at the pool's gross payout.
Credit fees use the ceiling of bonus shares:

```rust
let credit_fee_shares = ceil(bonus_shares * fee_bps / BPS);
let credited_shares = seized_shares - credit_fee_shares;
```

Under-delivery floor-scales seizure amounts, transfer fees, seized shares and
bonus shares by measured/planned repayment USD. Credit fees are recomputed
from the scaled bonus shares.

Bad-debt cleanup has separate eligibility: debt must exceed collateral, and
permissionless cleanup requires collateral at or below $5. Forced owner
cleanup omits the collateral cap. See [cleanup invariants](invariants.md#inv-liq-04)
for authorization and account deletion.

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

<a id="numeric-limits"></a>

## Caps, fees, and numeric limits

A cap is in native token units. Entry compares stored scaled usage plus the
new scaled amount with the cap floor-converted at the current index. Zero cap
allows no positive exposure. Exits subtract usage without checking caps;
missing usage rows and zero exit deltas are no-ops. Cap conversion saturates at
`i128::MAX`; position conversion still rejects overflow.

Flash-loan and charged strategy fees are half-up BPS of principal, with a
minimum of one base unit for a positive rate. Flash position has no origination
fee; see [its settlement invariant](invariants.md#inv-strat-04).

| Bound | Consequence |
|---|---|
| Asset decimals 3..=18 | Exact token-to-RAY upscaling |
| Both indexes initially RAY; ceiling 10^36 | 10^9 times initial index; protocol constants |
| Supply-index floor 10^24 | At most 1,000 times the shares minted at index one for the same deposit |
| Borrow APR maximum 2 RAY | 200% annual rate; not a bound on balance growth alone |
| Token-to-RAY input maximum `i128::MAX / 10^(27-d)` | About 170.14 billion whole tokens, before other limits |
| Deposit conversion at the supply-index floor | About 170.14 million whole tokens before scaled-share overflow |

The token-to-RAY maximum is also the admitted cap maximum. Accrued position
values and market totals must independently fit the RAY domain; valid caps and
bounded indexes do not guarantee that future accrual fits. Value overflow can
occur before the index ceiling and block repayment/withdrawal because those
operations accrue first. At the borrow-index ceiling, further accrual produces
no borrower interest. No dedicated ceiling alarm is emitted.

These are arithmetic limits, not recommended market sizes or deployment proofs.

### Liquidation fixture

These $5 repayment fixtures use ample collateral and selected USD prices.
They illustrate native arithmetic, excluding network fees and slippage. The
prices are neither live quotes nor maximum listing prices, and the results do
not guarantee profitability. The fixture models at most two token-unit rounding
costs per collateral leg; that is not a universal execution bound.

| Collateral | Fixture USD price | Seized token units | Fee units | Profit USD (rounded) |
|---|---:|---:|---:|---:|
| SolvBTC | 120,000 | 4,541 | 45 | 0.3952 |
| xSolvBTCSolvBTC_LP | 12,000 | 4,583 | 4 | 0.4948 |
| SPIKOUKTBL | 1.48035816 | 358,021 | 2,431 | 0.264006 |
| XAUM | 6,000 | 900,000 | 6,666 | 0.360004 |
| XLM | 1 | 54,500,000 | 540,000 | 0.3960 |
| USDC | 1.05 | 48,571,428 | 95,238 | 0.09000 |
| USST | 1.0897 | 4.818e18 | 2.294e16 | 0.2250 |

## Sources

- [Fixed-point arithmetic](../../common/src/math/fp_core.rs) and [share conversion](../../common/src/rates/scaling.rs).
- [Rate curve](../../common/src/rates/curve.rs), [compounding](../../common/src/rates/compound.rs), [index and reward calculations](../../common/src/rates/index.rs), and [accrual projection](../../common/src/rates/simulate.rs).
- [Position valuation](../../common/src/rates/value.rs) and [risk validation](../../common/src/validation.rs).
- [Liquidation planning and fees](../../contracts/controller/src/positions/liquidation/math.rs), [liquidation curve](../../contracts/controller/src/positions/liquidation/curve.rs), and [liquidation fixtures](../../contracts/controller/tests/positions/liquidation_math.rs).
