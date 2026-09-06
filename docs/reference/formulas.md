# Formulas and rounding

Expressions use raw integers: half_up/floor/ceil apply to the entire fraction.
Pseudocode does not imply unchecked Rust multiplication.

## Units and arithmetic domain

| Quantity | Scale |
|---|---|
| Shares, indexes, asset values used by the rate engine, rates, utilization | RAY = 10^27 |
| USD values, prices per whole token, health factor | WAD = 10^18 |
| Risk ratios and fees | BPS = 10,000 |
| Transfers and accounting cash | Native token base units |
| Accrual time | Milliseconds; year = 31,556,926,000 ms |

Amounts are non-negative at protocol boundaries. The `Ray`, `Wad`, and `Bps`
constructors themselves do not enforce that domain. Multiply-divide uses an
`i128` fast path or an exact `I256` intermediate; an unrepresentable result
raises `MathOverflow`, except at explicit saturating sites. A zero divisor
raises `DivisionByZero`. Half-up multiply-divide requires non-negative operands
and a positive divisor. Raw floor/ceiling multiply-divide also handle signed
quotients. Decimal upscaling stays in `i128` and rejects overflow; downscaling
and division by a positive integer support the signed extremes. Signed
half-up downscaling rounds exact halves away from zero.

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
shares and pays the floor-valued balance. A repayment at least the ceiled debt
balance burns all debt shares and refunds the excess. Positive supply/borrow mints and positive net-repay/gross-withdrawal burns
that would change zero shares revert.

Same-asset net settlement moves no cash. Its token overlap is
`min(request, floor(supply), ceil(debt))`. It burns all shares on a side only
when the overlap exhausts that side's conservative value; otherwise it uses a
ceiled supply burn and floored debt burn, each capped at the position.

## Cash, supply, debt, and revenue

Revenue shares form part of total supplied shares. Minting revenue adds equally
to both books; claiming revenue burns equally from both. Reclassifying seized
collateral as revenue leaves total supply unchanged. Cash is a separate reserve
book: incidental token donations do not increase lendable cash.

The market's backing check uses native units:

```rust
let shortfall = max(0, floor(supply_value) - (cash + ceil(debt_value)));
```

The implementation saturates the addition/subtraction. Supply entry rejects a
positive shortfall. Recapitalization credits at most that shortfall, refunds
excess, and mints no shares. Borrow draws must retain the 200 BPS liquidation
buffer, calculated half-up on the floored supplied token value. User withdrawal,
borrow, and revenue claims enforce configured utilization; liquidation
withdrawal skips that utilization gate. Withdrawal, net settlement, and revenue
claims reject zero total supply with outstanding debt. These are distinct checks;
they do not promise full backing after every mutation.

Revenue payout is `min(cash, floor(revenue_value))`. A full payout burns all
revenue shares; a cash-limited payout burns
`ceil(revenue_shares * payout / floor(revenue_value))`. Positive payout with
zero share burn reverts.

## Rates and accrual

Utilization divides half-up-valued debt by half-up-valued total supply:

```rust
let utilization_ray = half_up(debt_value_ray * RAY / supply_value_ray);
```

It is zero for zero supplied value. Rate selection caps utilization at one RAY.
The annual borrow curve has three joined linear regions: base plus slope1 up
to mid utilization, then slope2 up to optimal, then slope3 up to full utilization.
Each slope contribution uses half-up multiplication followed by half-up division.
The result is capped at the configured maximum, itself limited to 200% APR.
Pool borrow/deposit rate views return annual RAY fractions from stored indexes;
they do not accrue or project first.

```rust
let per_ms = half_up(annual_borrow_rate_ray / 31_556_926_000);
let rate_x_util = half_up(utilization_ray * annual_borrow_rate_ray / RAY);
let deposit_apr_ray = half_up(rate_x_util * (BPS - reserve_factor_bps) / BPS);
```

Deposit APR is a view with two rounding steps, not the exact realized supplier
return. It is zero at zero utilization or an out-of-range reserve factor.
Accrual processes elapsed time in chunks of at most one year. Each chunk reads
its starting utilization and rate, and approximates `exp(per_ms * delta_ms / RAY)`
through the eighth-order Taylor term, using fixed-point half-up arithmetic.
No elapsed time means no accrual. Index projections and mutations share `accrue_step`.

For each chunk, borrower interest is the difference between half-up-valued debt
at the new and old borrow indexes. The reserve factor allocates a half-up
protocol fee; the remainder becomes supplier rewards. The supply index update
floors the new total value divided by supplied shares, bounded by the old index
and the supply-index ceiling. Any reward not reflected by that index change is
added to the protocol reward. Revenue shares are then floor-converted at the
new supply index and capped at remaining total-supply share headroom. That final
floor/cap can leave value unrepresented by revenue shares; exact booked-value
conservation is not guaranteed.

Cadence can change borrower cost: each chunk freezes its starting rate while
later chunks recalculate utilization. Taylor truncation and integer rounding
also depend on partitioning. There is no general guarantee that every finer
partition produces the same result, a larger result, or an exact continuous
exponential bound.

## Valuation and health

A position uses one rounding direction at all three boundaries:

```rust
let asset_ray = round(shares_ray * index_ray / RAY);
let asset_wad = round(asset_ray / 1_000_000_000);
let value_usd_wad = round(asset_wad * price_wad / WAD);
```

Risk collateral floors each step and its BPS weighting. LTV weighting uses the
position's stored `min(LTV, liquidation_threshold)`; health weighting uses its
stored liquidation threshold. The unweighted collateral total rounds half-up
and sizes liquidation shares and dust eligibility. Risk debt rounds upward;
the separate debt display rounds half-up.

```rust
let health_factor_wad = floor(weighted_collateral_wad * WAD / debt_wad);
```

Health factor saturates at `i128::MAX`; debt-free accounts use that sentinel.
Liquidation eligibility requires debt and health below one WAD. Risk-increasing
actions with debt remaining require debt within LTV collateral, health at least one WAD, and any
configured minimum LTV collateral, alongside listing, cap, and pause gates.

## Liquidation sizing and fees

Let `D`, `C`, `W`, `HF`, `H` be raw WAD debt, unweighted/weighted collateral, health and target health.
mul_wad/div_wad round half-up; `b` is the chosen bonus converted from BPS to raw WAD.

```rust
let p = if C == 0 { 0 } else { div_wad(W, C) };
let hf_bonus_cap_bps = floor(HF * BPS / p) - BPS; // only p > 0 and HF < WAD
let backed_max = min(D, div_wad(C, WAD + b));
let ideal = if H <= mul_wad(p, WAD + b) || mul_wad(H, D) <= W { backed_max }
    else { min(backed_max, div_wad(mul_wad(H, D) - W, H - mul_wad(p, WAD + b))) };
```

The base bonus is collateral-value-weighted from stored bonuses, bounded by
`BPS * (BPS - t) / t`, where `t = clamp(ceil(p * BPS / WAD), 1, BPS)`.
Zero `p` gives a zero threshold bonus bound. The configured curve ramps the
base-to-maximum increment as health falls, then applies its BPS factor; the
HF-preserving cap above limits it. A cap below base quotes full debt at base.
Only a nonnegative below-base cap rejects partial funding, with ceil-USD rounding
tolerance. An ideal residual debt strictly between zero and $5 also quotes full debt.
Inputs are capped at actual debt and trimmed before pulling; neither a quote
nor its target health guarantees a full executed close after rounding/underdelivery.
Seizure is pro-rata to collateral value and capped at held value. Bonus equals
capped seizure minus floor-divided uncapped principal, floored at zero. Transfer fee
applies fee BPS half-up to bonus RAY, then floors to token units; positive subunit fees become one unit, capped
at gross payout. Credit fee is `ceil(bonus_shares * fee_bps / BPS)`; credit is the
exact seized-share remainder. Under-delivery floor-scales seizure and bonus
representations by measured/planned USD; credit fees are recomputed. Cleanup is
separate: debt must exceed collateral, with collateral <= $5 permissionlessly;
forced owner cleanup omits that collateral cap. See [liquidation source](../../contracts/controller/src/positions/liquidation/math.rs) and [curve](../../contracts/controller/src/positions/liquidation/curve.rs).

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

Zero supplied value makes the write-down a no-op. Before the clamp, the two
floors can impose extra truncation on supplied claims, including revenue.
The floor prevents a zero index; it can leave residual claims without backing.
Supply then fails the backing gate until recapitalization fills the shortfall.
Controller eligibility and account cleanup are described in the liquidation
lifecycle, rather than guaranteed by this arithmetic helper.

<a id="numeric-limits"></a>
## Caps, fees, and numeric limits

A cap is in native token units. Entry compares stored scaled usage plus the
new scaled amount with the cap floor-converted at the current index. Zero cap
allows no positive exposure. Exits subtract usage without checking caps;
missing usage rows and zero exit deltas are no-ops. Cap conversion saturates at
`i128::MAX`; position conversion still rejects overflow.

Flash/charged strategy fees are half-up BPS of principal, minimum one base unit for a positive rate.

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

Selected $5 repayment seizure fixtures with ample collateral; native arithmetic examples, not live prices,
maximum listing prices, network fees, slippage, or guaranteed profitability. The fixture models
at most two token-unit rounding costs per collateral leg; it is not a universal execution bound.

| Collateral | Fixture USD price | Seized token units | Fee units | Profit USD (rounded) |
|---|---:|---:|---:|---:|
| SolvBTC | 120,000 | 4,541 | 45 | 0.3952 |
| xSolvBTCSolvBTC_LP | 12,000 | 4,583 | 4 | 0.4948 |
| SPIKOUKTBL | 1.48035816 | 358,021 | 2,431 | 0.264006 |
| XAUM | 6,000 | 900,000 | 6,666 | 0.360004 |
| XLM | 1 | 54,500,000 | 540,000 | 0.3960 |
| USDC | 1.05 | 48,571,428 | 95,238 | 0.09000 |
| USST | 1.0897 | 4.818e18 | 2.294e16 | 0.2250 |

Sources: [shared rates](../../common/src/rates/mod.rs), [fixed-point arithmetic](../../common/src/math/fp_core.rs),
[liquidation fixtures](../../contracts/controller/tests/positions/liquidation_math.rs).