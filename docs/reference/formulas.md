# Formulas and rounding

[Invariants](invariants.md) name the operations enforcing each constraint.
Expressions use raw integers; `half_up`, `floor` and `ceil` apply to the whole
fraction. Pseudocode does not imply unchecked Rust multiplication.

## Units and arithmetic domain

| Quantity | Scale |
|---|---|
| Shares, indexes, rate-engine asset values, rates, utilization | RAY = 10^27 |
| USD values, whole-token prices, health factor | WAD = 10^18 |
| Risk ratios, fees | BPS = 10,000 |
| Transfers, accounting cash | Native token base units |
| Accrual time | ms; year = 31,556,926,000 ms |

Protocol boundaries require non-negative amounts; `Ray`, `Wad` and `Bps`
constructors do not enforce it. Multiply-divide uses an `i128` fast path or
exact `I256` intermediate. Unrepresentable results raise `MathOverflow`, except
at explicit saturating sites; zero divisors raise `DivisionByZero`.

Half-up multiply-divide needs non-negative operands and a positive divisor;
raw floor/ceil multiply-divide also support signed quotients. Decimal upscaling
is checked `i128`. Downscaling and division by a positive integer support the
signed extremes; signed half-up downscaling rounds exact halves away from zero.
Floor downscaling (`rescale_floor`, `to_wad_floor`, `to_asset_floor`,
`to_token_floor`) truncates toward zero, a floor for non-negative amounts.

## Shares and token amounts

For `d` decimals, shares times index is a RAY value; converting to token
units is a second step:

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

Withdrawing at least the half-up displayed balance burns all supply shares
and pays their floor value. Repaying at least the ceiled debt burns all debt
shares and refunds the excess. Positive supply, borrow, net repayment and gross
withdrawal revert if they mint or burn zero shares.

### Same-asset net settlement

Offsets supply against debt without moving cash:

```rust
let overlap = min(request, min(floor(supply), ceil(debt)));
```

Exhausting a side's conservative value burns all its shares. Otherwise the
supply-share burn ceils and the debt-share burn floors, each capped at held
shares. Positive settlement must burn shares on both sides.

## Cash, supply, debt, and revenue

Revenue is supply shares counted in total supplied shares: minting and
claiming move both totals equally. Reclassifying seized collateral as revenue
leaves total supply unchanged. Tracked cash is a separate reserve; donations do
not raise it.

### Backing and cash constraints

Backing check, native units:

```rust
let shortfall = max(0, floor(supply_value_ray - ceil_ray(debt_value)) - cash);
```

The RAY gap floors to native units once; the cash subtraction saturates.
Supply entry rejects a positive shortfall. Recapitalization credits at most the
shortfall, refunds excess and mints no shares.

Borrow draws keep a 200 BPS liquidation buffer, rounded up from the floored
supplied token value. Borrow, user withdrawal and revenue claims enforce the
utilization ceiling; liquidation withdrawal skips it:

```rust
let allowed = floor(floor(supply_value_ray) * max_utilization_ray / RAY);
let passes = ceil(debt_value_ray) <= allowed;
```

Rounding cannot admit utilization above the ceiling. Debt against zero
floored supply value fails; zero supplied shares and zero debt value skip the
gate. A debt mint (borrow or strategy opening) is checked at every ceiling, so
at 100% debt value must not exceed supply value. Withdrawals and revenue claims
skip a 100% ceiling, since cash bounds them. Debt mints, withdrawal, net
settlement and revenue claims reject zero total supply with outstanding debt.
These checks do not establish full backing after every mutation.

Directed rounding leaves cash that no share claims. Each full withdrawal or
full repayment adds less than one native unit. This residue counts as backing
in the shortfall check and as borrowable cash. No entrypoint pays it out; only
a pool upgrade can move it.

### Revenue payout

Payout is capped by cash and the floor-valued claim:

```rust
let payout = min(cash, floor(revenue_value));
let burned = if payout == floor(revenue_value) { revenue_shares }
    else { ceil(revenue_shares * payout / floor(revenue_value)) };
```

A positive payout cannot burn zero shares.

## Rates and accrual

### Utilization and annual rates

Half-up debt value over half-up supply value, capped at one RAY:

```rust
let utilization_ray = if supply_value_ray == 0 { 0 }
    else if debt_value_ray >= supply_value_ray { RAY }
    else { half_up(debt_value_ray * RAY / supply_value_ray) };
```

The quotient cannot overflow and stays in `[0, RAY]`; dust supply against
residual debt cannot stop accrual. Rate selection also caps utilization at one
RAY.

The annual borrow curve joins three linear regions: base plus slope1 up to
mid, slope2 up to optimal, slope3 up to full utilization. Each contribution is
one half-up `slope * excess / width`, so each region ends exactly on its slope
and the rate never falls as utilization rises. The rate cannot exceed the
configured maximum, itself at most 200% APR. Validation requires
`base + slope1 + slope2 + slope3 <= max`, so the maximum never cuts the curve.

Each slope is its whole segment's rise, so validation compares steepness per
unit of utilization, exactly and without division:
`slope1 / mid <= slope2 / (optimal - mid) <= slope3 / (RAY - optimal)`
(`slope1 * (optimal - mid) <= slope2 * mid`, likewise at optimal). Equal
steepness passes. Slopes must be non-negative and at most max. These imply
`base + slope1 + slope2 < max`, so the segment above
optimal always has room below the cap: a positive `slope3` leaves it, and
`slope3 = 0` forces `slope1 = slope2 = 0`, when `max > base` leaves it.

Pool rate views return annual RAY fractions from stored indexes, without
accruing or projecting; accrual uses `per_ms`:

```rust
let per_ms = half_up(annual_borrow_rate_ray / 31_556_926_000);
let rate_x_util = half_up(utilization_ray * annual_borrow_rate_ray / RAY);
let deposit_apr_ray = half_up(rate_x_util * (BPS - reserve_factor_bps) / BPS);
```

Deposit APR rounds twice and is not an exact realized return. It is zero at
zero utilization or an out-of-range reserve factor, and never above borrow APR.

### Compounding and interest allocation

Accrual splits elapsed time into chunks of at most one year. Each chunk
approximates `exp(per_ms * delta_ms / RAY)` at its starting utilization and
rate, to the eighth-order Taylor term, half-up fixed point. Zero elapsed time
accrues nothing. Projections and mutations share the step.

Borrower interest is half-up debt value at the new borrow index minus the old.
The reserve factor takes a half-up protocol fee; the rest goes to suppliers.

The supply index floors new total value over supplied shares, bounded by the
old index and the supply-index ceiling. Reward the index misses joins the
protocol reward. Revenue shares floor-convert at the new index, capped by
remaining total-supply share headroom, which can leave reward unrepresented.
Exact conservation of supplier and revenue value is not guaranteed.

Each chunk caps the borrow index so interest fits the room below the value
ceiling of the larger half-up total (debt or supply):

```rust
let room = max(0, MAX_MARKET_VALUE_RAY - max(debt_value_ray, supply_value_ray));
let new_borrow_index = min(grown_borrow_index,
    old_borrow_index + floor(room * RAY / debt_shares));
```

`MAX_MARKET_VALUE_RAY` is `i128::MAX - RAY`; interest raises both totals
equally, so neither crosses it. With no room, or a total exceeding `i128`, the
chunk changes no index and books no revenue, but time advances; borrowers
accrue less than the rate implies and the pool emits `MarketValueCeilingEvent`.

### Accrual cadence

Cadence changes borrower cost: each chunk holds its starting rate, and
truncation and rounding depend on the partition. A finer partition guarantees
neither the same result, a larger one, nor an exact exponential bound.

## Valuation and health

A position rounds one direction at all three steps:

```rust
let asset_ray = round(shares_ray * index_ray / RAY);
let asset_wad = round(asset_ray / 1_000_000_000);
let value_usd_wad = round(asset_wad * price_wad / WAD);
```

Risk collateral floors all three; risk debt ceils; displayed debt and
unweighted collateral (liquidation proportions, dust eligibility) round
half-up. Weights convert BPS→WAD half-up, then floor-multiply
(`Bps::apply_to_wad_floor`): LTV uses the stored
`min(LTV, liquidation_threshold)`, health the stored liquidation threshold.

```rust
let health_factor_wad = floor(weighted_collateral_wad * WAD / debt_wad);
```

HF saturates at `i128::MAX`, the debt-free sentinel. Liquidation requires
debt and HF below one WAD. Risk-increasing actions leaving debt require debt
within LTV collateral, HF at least one WAD and any minimum LTV collateral,
plus listing, cap and pause gates ([risk invariants](invariants.md#inv-risk-01)).

## Price sources

The aggregator returns one `price_wad` and status per asset, reporting the
first failure in this order:

1. source-read error;
2. no configuration: `OracleNotConfigured` (216);
3. stale leg: `PriceFeedStale` (206);
4. tolerance breach: `UnsafePriceNotAllowed` (205);
5. nonpositive price: `InvalidPrice` (217);
6. outside sanity bounds: `SanityBoundViolated` (223).

Two sources blend:

```rust
ratio_bps = half_up(high * BPS / low)
deviation = low <= 0 || ratio_bps > upper_ratio_bps
price_wad = floor((a + b) / 2)
timestamp = min(timestamp_a, timestamp_b)
newest    = min(newest_a, newest_b)
stale     = stale_a || stale_b || spread_too_wide
```

`high`/`low` are the larger/smaller leg. Half-up rounding accepts an exact
ratio under 0.5 BPS above `upper_ratio_bps`; ratio overflow sets `deviation`.
The spread term of `stale` applies only to two market legs. It is true when
the `newest` values of the two legs differ by more than
`MAX_LEG_AGE_SPREAD_SECONDS` (3600 seconds). A leg-sum overflow gives price zero, failing as `InvalidPrice`
(217) after any stale or tolerance failure.

Each leg carries two timestamps in seconds. `timestamp` is its oldest sample;
the staleness checks and the reported price use it. `newest` is its newest
sample; only the spread term uses it:

| Leg | `timestamp` | `newest` |
|---|---|---|
| One-sample feed: Reflector spot, Chainlink, RedStone, XOXNO | the sample | the sample |
| Reflector `Twap(records)` | oldest used round | newest used round |
| Scaled, LP, or a nested key | minimum over its inputs | minimum over its inputs |

The TWAP window does not enter the spread term. Admission therefore has no
rule that relates the window to `MAX_LEG_AGE_SPREAD_SECONDS`.

Admission accepts only this tolerance form, else `BadLastTolerance` (208):

```rust
BPS + MIN_TOLERANCE <= upper_ratio_bps <= BPS + MAX_TOLERANCE
BPS - MAX_TOLERANCE <= lower_ratio_bps <= BPS
lower_ratio_bps == half_up(BPS * BPS / upper_ratio_bps)
```

`MIN_TOLERANCE` is 150 and `MAX_TOLERANCE` is 2500.

Sanity-band admission:

```rust
0 < min < max <= MAX_REASONABLE_PRICE_WAD
floor((max - min) * BPS / (max + min)) >= MIN_SANITY_BAND_BPS
ceil((max - min) * BPS / (max + min)) <= band_cap
```

Lines 1–2 fail with `InvalidSanityBounds` (224); `MIN_SANITY_BAND_BPS` is 50.
Line 3 fails with `SanityBandTooWideForSingleSource` (226); the cap is
`MAX_LP_SANITY_BAND_BPS` (8182) for LP, else
`MAX_SINGLE_SOURCE_SANITY_BAND_BPS` (1000), and none for a pair with differing
trust sets. In such a pair, each scaled leg whose factor provider also serves
the other leg must keep `[min_factor_wad, max_factor_wad]` within the 1000-BPS
cap; else it fails with error 226.

A scaled source multiplies a factor feed by a quote-asset price:

```rust
price_wad = half_up(factor_wad * quote_wad / WAD)
timestamp = min(factor_timestamp, quote_timestamp)
newest    = min(factor_newest, quote_newest)
```

A factor outside `[min_factor_wad, max_factor_wad]` fails with
`FactorOutOfBounds` (230), overflow with `InvalidPrice` (217). Quote errors
pass through; only the factor sets stale.

Reflector `Twap(records)` admission:

```rust
MIN_ORACLE_RESOLUTION_SECONDS <= resolution <= max_stale
1 <= records <= MAX_TWAP_RECORDS
oldest_sample_age = records * resolution <= max_stale
```

`MIN_ORACLE_RESOLUTION_SECONDS` is 60, `MAX_TWAP_RECORDS` 12. Resolution and
age breaches fail with `InvalidOracleResolution` (222), zero records with
`TwapInsufficientObservations` (219), too many with `TwapRecordsOutOfRange`
(228).

A read uses the first `records` samples of the answer, newest first, and
ignores the rest. It carries the oldest and the newest timestamp it uses. The
Reflector contract answers with `records + 1` samples on a contiguous history.
It fills a missing round with the round before it, so a skipped round gives
fewer samples over a longer span.

The oldest used sample is `records - 1` rounds behind the newest. The newest
sample is one `resolution` old when the next sample is due. With `max_stale`
at the admission bound, a late or skipped sample makes the read stale.

Reads fail on an empty answer (`ReflectorHistoryEmpty`, 212) and on a future
used sample (`PriceFeedStale`, 206). Used samples closer than `resolution`,
or spanning less than `(records - 1) * resolution`, fail with
`TwapInsufficientObservations` (219). Used samples must be positive. The read
does not check the samples that it ignores.

```rust
mean      = floor(sum(samples) / count(samples))
price_wad = mean * 10^(18 - feed_decimals)
timestamp = oldest sample
newest    = newest sample
```

`samples` are the samples the read uses.

The upscale is exact; failure gives `InvalidPrice` (217).

Constant-product LP, `isqrt` the 256-bit floor square root:

```rust
value_i   = half_up(reserve_i * price_i_wad / 10^decimals_i)
price_wad = floor(2 * isqrt(value_a * value_b) * WAD / share_supply_wad)
```

Stable two-coin LP, reserves upscaled to WAD, `D` the stable-swap invariant:

```rust
price_wad = floor(D * min(price_a_wad, price_b_wad) / share_supply_wad)
```

`D` is Newton-Raphson with `Ann = amp * 2`, from the reserve sum until a step
<= 1, at most 255 iterations. `amp` in `[MIN_AMP, MAX_AMP]` (1 to 1,000,000);
each upscaled reserve <= `MAX_NORMALIZED_RESERVE_WAD` (10^34). A nonpositive
reserve, price or share supply, or a limit breach, fails with `InvalidPrice`
(217).

Aquarius size floor:

```rust
pool_value_wad = half_up(price_wad * total_shares / 10^share_decimals)
```

Below `min_pool_value_wad`: `InsufficientAquariusLiquidity` (235). Timestamp:
older leg. Newest: older of the two leg `newest` values.

Curve pool (two or three coins), from `get_virtual_price`:

```rust
price_wad      = floor(virtual_price * min(coin_price_wad) / WAD)
pool_value_wad = floor(price_wad * total_supply / WAD)
```

Below `min_pool_value_wad`: `InsufficientCurveLiquidity` (237). Timestamp:
oldest of ledger time and coin timestamps. Newest: oldest of ledger time and
coin `newest` values.

## Liquidation sizing and fees

Liquidation picks a bonus, sizes repayment toward a target HF, and spreads
seizure across collateral. Rounding, measured receipt and interest accrued
after an estimate's ledger can move settlement off the quote.

### Bonus and target repayment

| Symbol | Meaning, in raw WAD |
|---|---|
| `D` | Risk-valued debt |
| `C` | Unweighted collateral |
| `W` | Liquidation-threshold-weighted collateral |
| `HF` | Health factor |
| `H` | Target health factor |
| `b` | Selected bonus converted from BPS |

`p` is the blended liquidation threshold:

```rust
let p = if C == 0 { 0 } else { half_up(W * WAD / C) };
let hf_bonus_cap_bps = floor(HF * BPS / p) - BPS; // only p > 0 and HF < WAD
let weighted_seizure = half_up(p * (WAD + b) / WAD);
let target_debt = half_up(H * D / WAD);
let backed_max = min(D, half_up(C * WAD / (WAD + b)));
let ideal = if H <= weighted_seizure || target_debt <= W { backed_max }
    else { min(backed_max, half_up((target_debt - W) * WAD / (H - weighted_seizure))) };
```

The base bonus averages stored collateral bonuses by half-up USD weight: exact
sum of `value * bonus_bps`, one half-up division by total collateral. It is
bounded by `floor(BPS * (BPS - t) / t)`, `t = clamp(ceil(p * BPS / WAD), 1, BPS)`.
Zero `p` gives a zero bound; zero collateral a zero base bonus.

The curve ramps the base-to-maximum increment as HF falls, then applies BPS
factor `f`. `H` is the spoke's `liquidation_target_hf_wad`, `K` its
`hf_for_max_bonus_wad`:

```rust
let s = if HF >= H { 0 } else { min(WAD, half_up((H - HF) * WAD / (H - K))) };
let curve = base + half_up(f * half_up((max - base) * s / WAD) / BPS);
```

The ramp starts at `H` but liquidation below `HF = 1`, so the first bonus is
already ≈ `base + f * (max - base) * (H - 1) / (H - K)`. `max` comes from the
blended threshold, not the listed `liquidation_bonus`: `LT = 6000` gives
`max = 6666`, and `H = 1.10`, `K = 0.80`, `f = 10000`, `base = 500` give ≈ 2555
BPS just below `HF = 1` and 4611 at `HF = 0.90`. Size `H`, `K`, `f` from the
bonus at `HF = 1` and at `K`: `LT = 5300` (`max = 8867`) with `H = 1.06`,
`K = 0.90`, `f = 598`, `base = 500` gives ≈ 688 BPS just below `HF = 1` and
1000 at `HF <= 0.90`.

The HF-preserving cap limits the result. With positive `p`, insolvency or a cap
below base bypasses the target formula:

```rust
let quote = if p > 0 && C < D { (min(D, floor(C * WAD / (WAD + base))), base) } // insolvent
    else if p > 0 && cap < base { (D, max(cap, 0)) } // band: D <= C < D * (1 + base)
    else { /* target formula at min(curve, cap), or at curve when p == 0 */ };
```

In the band the quote is full debt at the cap; smaller payments are accepted.
Partial repayment `x` seizes `x * (1 + cap)`, at most `x * C / D`, so `C / D`
and HF do not fall (the cap's BPS floor can raise them slightly).

When the curve bonus exceeds a cap at or above base, the target formula runs at
the cap. There `p * (1 + b)` ≈ `HF`, so the quote is ≈ `D`, seizure ≈ `C`, and
the bonus ≈ `C - D`, the borrower's whole equity
([borrower protection](../explanation/threat-model.md#price-integrity-and-availability)).
With curve 1.02/0.80/10000, a one-leg account gets a whole-debt quote from
about HF 0.93–0.97 (depending on its threshold and base bonus) down to
insolvency.

Insolvency is exactly `C < D` (unweighted). With positive `p`, the quote is
the floored repayment collateral backs at the base bonus. Larger offers are
trimmed, each kept leg down to whole base units, so the liquidator never pays
more than it seizes. A leg rounding to zero is dropped and refunded; if none
remains, `liquidate` reverts with `InvalidPayments` (16) and the estimate shows
zero. This branch never promotes to full debt; cleanup takes the unbacked
residue. With `p == 0`, the target formula and dust promotion apply.

`HF / p` approximates `C / D`, but `HF` floors and `p` rounds half-up, so an
account at or a few raw WAD units above `C == D` can compute cap `-1`. It takes
the band quote with cap clamped to zero. A full close repays all of `D`,
leaving nothing to socialize. Seizure floors to whole token units, at most one
unit per collateral leg below `C`; that unit stays as collateral, unswept by
cleanup because no debt remains.

Ideal residual debt strictly between zero and $5 promotes the quote to full
debt, funded or not; a [raised whole-unit quote](#raised-partial-quote) is not
promoted and can leave debt below $5. Inputs are capped at each leg's
ceiling-rounded debt, excess listed as refund. A partial quote trims inputs
above it from the last leg backward before pulling; on a solvent account the
refund floors, so the kept amount can round up one unit. Full-debt quotes trim
nothing. Per-leg ceilings can exceed `D` by unit rounding, but a leg paid at
its ceiling is credited only the floor value cleared, so the rounded fraction
buys no collateral. The pool refunds each leg's excess, exactly the listed
refund. Neither a full-debt quote nor the target HF guarantees an executed full
close after rounding or under-delivery.

### Whole-unit collateral below 3 decimals

A collateral token with fewer than 3 decimals seizes whole units only (one base
unit, `10^-d` token). A partial seizure of such a leg:

- rounds up to the next unit, capped at the balance, when repayment covers the
  whole debt; the debt closes;
- otherwise rounds down, trimming and refunding the dropped fraction's USD
  value divided by `1 + bonus`, floored;
- rounds kept amounts up, so the liquidator pays for units received;
- reverts with `InvalidPayments` (16) if nothing is then seized.

Neither rule applies when an insolvent account's repayment reaches the
collateral-backed quote: every unit is seized. The leg is the account's only
supply position, so seizure stays proportional.

A borrow, withdrawal or account strategy leaving debt needs at least 2 whole
units in the leg. Liquidation can leave fewer; a `Credit` receiver's units are
not checked.

#### Raised partial quote

The controller can change such a leg's partial quote when the account is
solvent (`C >= D`), the curve quote is below `D`, and the leg is the only
supply position holding at least one whole unit. In raw WAD: `U` is one unit's
USD value, `b` the bonus, `m = max(floor(U / 1e6), 1)`, and `R` the sum over
debt legs of one base unit's USD value (the per-leg ceiling rounding a full
close can record). The first matching rule applies:

1. If `floor(U / (1 + b)) >= D + R`, the quote becomes `D`: full close, the leg
   rounds up to one unit, and the liquidator repays all debt for one unit.
2. If the curve quote seizes less than `U + m` and
   `ceil((U + m) / (1 + b)) < D`, the quote rises to
   `ceil((U + m) / (1 + b))`. Seizure takes one unit and refunds the unseized
   margin, floored to whole debt-token units.
3. Otherwise the curve quote stays.

Size offers from `get_liquidation_estimate`, not the curve formula:

- The raised quote is a ceiling; smaller offers are not raised.
- An offer backing under one unit seizes nothing. The controller refunds each
  offer leg with a positive USD value and reverts with `InvalidPayments` (16)
  when no leg remains. A leg whose USD value rounds to zero stays paid.
- An offer between `U / (1 + b)` and the raised quote still takes one unit.

#### Liquidatability of a whole-unit leg

The HF-preserving cap keeps `1 + b <= C / D`, so selling one unit does not
lower `C / D`. A solvent account whose only supply leg is below 3 decimals and
holds a whole unit stays liquidatable below `HF = 1`, by one-unit sale or full
close.

Exception: while `floor(U / (1 + b)) - R < D <= ceil((U + m) / (1 + b))`,
neither rule 1 nor 2 applies. If the curve quote then backs under one unit,
every offer reverts until accrual or a price move ends that state.

#### Effective bonus of a rule-1 full close

- Rule 1 pays one unit worth `U` for `D`: effective bonus `U / D - 1`, not `b`.
- With `k` held units and threshold `LT`, `HF < 1` gives `k * U * LT < D`, so
  the effective bonus is at most about `1 / (k * LT) - 1`.
- The borrower loses `U - D * (1 + b)` beyond the normal bonus.
- `bonus_rate_bps` (estimate) and `bonus_bps` (`LiquidationEvent`) show only
  `b`.
- Listings below 3 decimals have no liquidation fee, so no protocol fee applies
  to the excess.

### Seizure and fees

Seizure is proportional to collateral value, capped at held value. The bonus is
capped seizure minus floor-divided uncapped principal, at least zero, so a cap
below principal leaves no bonus to charge.

Partial seizure floors token amount and shares. Full seizure requests the
half-up token amount (exact held shares in Credit mode); the payout still
floors the claim. Transfer mode treats any request at or above the half-up
balance as full: with a held fraction under half a unit, a request for the
floored balance burns every share, pays the planned floored amount, and leaves
the fraction as unclaimed cash. Bonus shares floor at the supply index, capped
at seized shares. Zero-share or zero-token legs are omitted.

Transfer fees apply BPS half-up to bonus RAY, floor to token units (a positive
subunit fee becomes one unit), then cap at whole units paid above principal:

```rust
let realised_excess = if paid_ray > base_ray {
    floor(paid_ray - base_ray)
} else { 0 };
let transfer_fee = min(bumped_fee, realised_excess);
```

`paid_ray` is the gross pool payout, `base_ray` the uncapped principal; `floor`
rescales to token units. The planned fee never takes the liquidator below
principal; the payout floor still can. Credit fees ceil bonus shares:

```rust
let credit_fee_shares = ceil(bonus_shares * fee_bps / BPS);
let credited_shares = seized_shares - credit_fee_shares;
```

Under-delivery floor-scales seizure amounts, transfer fees, seized and bonus
shares by measured/planned repayment USD. The transfer amount is also capped at
the scaled shares' floor asset value, and the fee at that amount. A leg below 3
decimals reverts ([INV-LIQ-03](invariants.md#inv-liq-03)). Credit fees are
recomputed from scaled bonus shares.

Bad-debt cleanup requires ceil risk debt above half-up unweighted collateral;
permissionless cleanup also requires collateral <= $5, which forced owner
cleanup omits ([cleanup invariants](invariants.md#inv-liq-04)).

## Bad debt

Socialization lowers only the affected market's supply index. Removed debt is
ceil-valued in RAY; the write-down uses half-up total supply value, two floors
and a non-zero clamp:

```rust
let remaining_ray = total_supply_ray - min(bad_debt_ray, total_supply_ray);
let reduction_ray = floor(remaining_ray * RAY / total_supply_ray);
let new_supply_index = max(10_i128.pow(24),
    floor(old_supply_index * reduction_ray / RAY));
```

Zero supply value makes it a no-op. The floors can further truncate supply
claims, revenue included. The clamp can leave claims unbacked; supply then fails
the backing gate until recapitalization fills the shortfall. Eligibility and
deletion: [cleanup rules](invariants.md#inv-liq-04).

## Swap router fees and splits

Fees apply only with an active referral, each on the fee token's vault balance:

```rust
static_fee   = floor(balance * static_fee_bps / 10_000)
referral_fee = floor(balance * fee_bps / 10_000)
```

`fee_bps` is the referral rate. Rates summing above `FEE_CAP` (1000 BPS) fail
with `FeeTooHigh` (21). Splits take parts per million of the balance available
when the instruction runs:

```rust
amount = floor(available_balance * ppm / PPM_DENOMINATOR)
```

`PPM_DENOMINATOR` is 1,000,000. Zero weight fails with `ZeroSplitPpm` (11),
above the denominator with `SplitPpmMismatch` (12). Weights need not sum to it,
since each split applies to the remaining balance. Intermediate-token residual
allowance, token units:

```rust
allowance = max(floor(credited / RESIDUAL_PPM), RESIDUAL_DUST_FLOOR)
```

`RESIDUAL_DUST_FLOOR` is 1,000; a larger residual fails with `ExcessiveResidual` (29).

<a id="numeric-limits"></a>

## Caps, fees, and numeric limits

Caps are native token units. Entry compares stored scaled usage plus the new
scaled amount with the cap floor-converted at the current index; a zero cap
allows no exposure. Exits subtract usage unchecked; missing rows and zero
deltas are no-ops. Cap→scaled conversion saturates at `i128::MAX`
(`calculate_scaled_cap`): the check fails open instead of trapping and no
longer enforces the asset-unit limit. An admitted cap saturates only at an
index below RAY / 8. Only bad-debt write-downs push the supply index below
RAY; at its floor (`RAY / 1000`), supply caps above about 1/125 of the admitted
maximum saturate, though the entry ceiling still bounds share count. The
borrow index never falls below RAY, so admitted borrow caps cannot saturate.
Position conversion still rejects overflow.

Flash-loan and charged strategy fees are half-up BPS of principal, minimum one
base unit at a positive rate. Flash position has no origination fee
([settlement invariant](invariants.md#inv-strat-04)).

| Bound | Consequence |
|---|---|
| Asset decimals 0..=18 | Exact token-to-RAY upscaling. Below 3: collateral only, no flash loans or liquidation fee, sole supply position, >= 2 whole units after a borrow, withdrawal or strategy leaving debt |
| Both indexes initially RAY; ceiling 10^36 | 10^9 × initial index; protocol constants |
| Supply-index floor 10^24 | At most 1,000× the shares minted at index one |
| Borrow APR maximum 2 RAY | 200% annual; not a bound on balance growth alone |
| Token-to-RAY input maximum `i128::MAX / 10^(27-d)` | ~170.14 billion whole tokens, before other limits |
| Market value ceiling `i128::MAX - RAY` | ~170.14 billion whole tokens of supply or debt value; accrual holds interest there |
| Market entry ceiling `(i128::MAX - RAY) / 8` | ~21.27 billion whole tokens of supply or debt, as value and share count; also the admitted cap maximum |
| Supply at the supply-index floor | ~21.27 million whole tokens before share count passes the entry ceiling |

The admitted cap maximum, `MAX_MARKET_ENTRY_RAY / 10^(27-d)`, sits 8× below
the value ceiling. A year at 200% APR multiplies debt by under `e^2 < 8`, so a
market at cap accrues at least a year (about ten at 20% APR) before interest is
held.

Supply, borrow, strategy openings and flash loans check the entry ceiling after
raising a total: supply and debt, each as share count (a floored supply index
mints 1,000 shares per unit of value) and half-up value. Above it they fail
with `MarketCeilingReached` (136); exits never check it. A market that interest
lifts above the ceiling takes no new entry until exits bring it under, but keeps
accruing, repaying, withdrawing and liquidating.

Accrual never reverts on market value. At the value ceiling, as at the
borrow-index ceiling, it produces no borrower interest; the value ceiling emits
`MarketValueCeilingEvent` ([INV-IDX-06](invariants.md#inv-idx-06)).

These are arithmetic limits, not recommended market sizes.

### Liquidation fixture

A $5 repayment per row, using one mainnet listing's decimals, bonus and fee,
with ample collateral. Prices are illustrative test inputs; five sit at their
listing's `max_sanity_price_wad`, and tests run every row there too. Profit
excludes network fees and slippage; at most two token-unit rounding costs per
leg are modeled.

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
