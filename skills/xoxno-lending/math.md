# XOXNO Lending math reference

This page gives formulas for raw contract state and liquidation sizing. The
pseudocode uses integers. Round the whole fraction `x * y / d` with `floor`,
`ceil`, or `half_up`. Keep intermediates exact, with `I256` on-chain and
`BigInt` off-chain. Never use floating-point `Number`.

The sources are [`docs/reference/formulas.md`](https://github.com/XOXNO/rs-lending-xlm/blob/main/docs/reference/formulas.md), `common/src/{constants,math,rates}`, and `contracts/{pool,controller}`. The worked numbers are recomputed with exact integers.

## Units

| Quantity | Scale | Constant (`common/src/constants/shared.rs`) |
|---|---|---|
| Shares (`scaled_amount`), indexes, rate-engine asset values, rates, utilization | RAY = 10^27 | `RAY`, `RAY_DECIMALS = 27` |
| USD values, prices per whole token, health factor | WAD = 10^18 | `WAD`, `WAD_DECIMALS = 18` |
| LTV, liquidation threshold, bonus, fees, reserve factor | BPS = 10,000 | `BPS` |
| Token transfers, cash, caps, `get_collateral_amount`, `get_borrow_amount` | Token base units at the asset's decimals (0..=18) | `MIN_ASSET_DECIMALS = 0`, `MAX_ASSET_DECIMALS = 18` |
| Accrual time | Milliseconds | `MILLISECONDS_PER_YEAR = 31_556_926_000`, `MS_PER_SECOND = 1_000` |

| From | To | Factor |
|---|---|---|
| token base units (`d` decimals) | RAY | `× 10^(27 - d)` (exact) |
| RAY | token base units | `÷ 10^(27 - d)` (rounded) |
| RAY | WAD | `÷ 10^9` (rounded) |
| WAD | RAY | `× 10^9` (exact) |
| BPS | WAD ratio | `half_up(bps × WAD / BPS)` |

## Rounding vocabulary

The rounding functions are in `common/src/math/fp_core.rs`:

```text
floor(x, y, d)   = ⌊x·y / d⌋
ceil(x, y, d)    = ⌈x·y / d⌉
half_up(x, y, d) = ⌊(x·y + ⌊d/2⌋) / d⌋          // x, y ≥ 0, d > 0
```

## Shares and token amounts

Positions store shares (`scaled_amount`, RAY). The value is shares × index, rounded twice before a token transfer ([formulas.md#shares-and-token-amounts](https://github.com/XOXNO/rs-lending-xlm/blob/main/docs/reference/formulas.md#shares-and-token-amounts)). The table shows the rounding per boundary (`common/src/rates/scaling.rs`, `contracts/pool/src/cache/scale.rs`):

| Boundary | Function | Direction |
|---|---|---|
| Supply mint | `calculate_scaled_supply` | floor |
| Partial withdrawal burn | `calculate_scaled_supply_ceil` | ceil |
| Debt mint | `calculate_scaled_borrow` | ceil |
| Partial repayment burn | `calculate_scaled_borrow_floor` | floor |
| Supply claim paid in tokens | `unscale_supply_floor` | floor at both steps |
| Debt full-close amount | `unscale_borrow_ceil` | ceil at both steps |
| Displayed balance (`get_collateral_amount`, `get_borrow_amount`, `get_supplied_amount`, `get_borrowed_amount`) | `unscale_supply`, `unscale_borrow` | half-up at both steps |

Full-close rules (`resolve_withdrawal`, `resolve_repay`):

- A withdrawal `≥` the half-up displayed balance burns **all** shares and pays the **floor** balance. A smaller withdrawal pays exactly the request and burns `ceil` shares. If floor < half-up, a request sized from the floor is partial and leaves dust.
- A repayment `≥` the ceiled debt burns **all** debt shares and refunds the excess. A smaller repayment burns `floor` shares.
- Positive supply, borrow, gross withdrawal, net repayment, and net settlement must move at least one share. Otherwise the call fails with `SupplyRoundsToZeroShares`, `BorrowRoundsToZeroShares`, `WithdrawRoundsToZeroShares`, `RepayRoundsToZeroShares`, or `NetSettleRoundsToZeroShares` (`contracts/pool/src/ops/*.rs`).
- A `withdraw` amount of `i128::MAX` means the whole position. Any amount at or above the position's half-up value withdraws all of it (`resolve_withdrawal`, `common/src/rates/scaling.rs`). Pass `i128::MAX` to close rather than computing a balance. An amount of `0` fails with `AmountMustBePositive` (`aggregate_positive_payments`, `payments.rs`). Two `i128::MAX` legs for the same hub asset overflow the aggregated sum and fail with `MathOverflow`.

### Worked example: USDC (7 decimals)

Supply 1,000 USDC (`10_000_000_000` base units) at supply index `1.05 RAY`:

```text
amount_ray = 10_000_000_000 × 10^20 = 1_000_000_000_000_000_000_000_000_000_000
shares     = floor(amount_ray × RAY / 1.05e27) = 952_380_952_380_952_380_952_380_952_380
```

Later the live supply index is `1.083 RAY`:

```text
value_ray (floor)   = 1_031_428_571_428_571_428_571_428_571_427
value_ray (half-up) = 1_031_428_571_428_571_428_571_428_571_428
tokens floor   = 10_314_285_714   // 1,031.4285714 USDC — what a full withdrawal pays
tokens half-up = 10_314_285_714   // what get_collateral_amount displays
tokens ceil    = 10_314_285_715
```

Withdrawing 500 USDC burns `ceil(5e9 × 10^20 × RAY / 1.083e27) = 461_680_517_082_179_132_040_627_885_504` shares (floor: `…503`). To close, request `≥ 10_314_285_714` (half-up) or `0`. The payout is `10_314_285_714`.

### Worked example: USST (18 decimals) debt

Borrow 250 USST (`250 × 10^18` base units) at borrow index `1.12 RAY`:

```text
amount_ray  = 250e18 × 10^9
debt shares = ceil(amount_ray × RAY / 1.12e27) = 223_214_285_714_285_714_285_714_285_715   // floor: …714
```

At live borrow index `1.1315 RAY`:

```text
debt_ray (ceil)  = 252_566_964_285_714_285_714_285_714_287
full-close tokens = ceil(debt_ray / 10^9) = 252_566_964_285_714_285_715   // repay ≥ this to burn all shares
display tokens    = half_up(...)          = 252_566_964_285_714_285_714   // get_borrow_amount
```

Repaying 100 USST burns `floor(100e27 × RAY / 1.1315e27) = 88_378_258_948_298_718_515_245_249_668` shares.

### Portfolio API amounts

Wallet display uses v1 `LendingPosition` arrays. `amountRaw` is a base-unit
string. `amount` is a decimal whole-token string. No RAY conversion applies,
because share formulas apply only to raw contract positions. For missing
values, prices, APYs, and pagination, see
[SDK reads](../xoxno-lending-sdk/reads.md#units-and-missing-data).

## Utilization

The utilization code is in `common/src/rates/curve.rs::utilization` and `contracts/pool/src/cache/scale.rs::calculate_utilization`:

```text
supply_value_ray = half_up(total_supplied_shares × supply_index / RAY)
debt_value_ray   = half_up(total_borrowed_shares × borrow_index / RAY)
utilization_ray  = supply_value_ray == 0 ? 0
                 : debt_value_ray ≥ supply_value_ray ? RAY
                 : half_up(debt_value_ray × RAY / supply_value_ray)      // always in [0, RAY]
```

`get_utilisation` returns the stored ratio. Reproduce it from `get_sync_data`
shares and indexes. Spokes on one hub-asset market share its rate.

Example: 10,000,000 USDC supplied value and 6,500,000 USDC borrowed value give `utilization_ray = 650_000_000_000_000_000_000_000_000` (0.65).

`max_utilization` (RAY) is a pool guard, not a curve input, checked after a
borrow, user withdrawal, or revenue claim:

```text
ceil(debt_value_ray) ≤ floor(floor(supply_value_ray) × max_utilization / RAY)
```

Zero debt or zero supplied shares pass. Positive debt against supply value
that floors to zero fails (`UtilizationAboveMax`). Borrows and strategy openings
are always checked, so at `max_utilization = RAY` debt cannot exceed supply.
User withdrawals and revenue claims skip it when `max_utilization ≥ RAY`.
Liquidation withdrawals always skip it. The source is `contracts/pool/src/guards.rs::require_utilization_below_max`
and `require_exit_utilization_below_max`.

## Borrow-rate curve

`calculate_annual_borrow_rate` (`common/src/rates/curve.rs`) takes `MarketParams { base_borrow_rate, slope1, slope2, slope3, mid_utilization, optimal_utilization, max_borrow_rate }`, all in RAY:

```text
u = min(utilization_ray, RAY)
if u < mid:            rate = base + half_up(u × slope1 / mid)
else if u < optimal:   rate = base + slope1 + half_up((u − mid) × slope2 / (optimal − mid))
else:                  rate = base + slope1 + slope2 + half_up((u − optimal) × slope3 / (RAY − optimal))
rate = min(rate, max_borrow_rate)
```

Each slope is its segment's total rise: `slope1` covers 0 to `mid`, `slope2` covers `mid` to `optimal`, and `slope3` covers `optimal` to 100%.

The rate model must satisfy these rules:

- `max_borrow_rate ≤ MAX_BORROW_RATE_RAY = 2 RAY` (200% APR).
- `base + slope1 + slope2 + slope3 ≤ max_borrow_rate` (the cap never flattens the curve before 100%).
- `slope1 / mid ≤ slope2 / (optimal − mid) ≤ slope3 / (1 − optimal)` (equality allowed). Steepness never falls at a kink.

The result is an **annual RAY fraction** (`get_borrow_rate`, `i128`).

Illustrative parameters follow. For live parameters, read pool `get_sync_data`. The values are base 0.005, slope1 0.03, slope2 0.095, slope3 1.0, mid 0.60, optimal 0.85, max 1.25, all × RAY:

| utilization | segment | annual borrow rate |
|---|---|---|
| 0.00 | 1 | 0.005 |
| 0.30 | 1 | 0.005 + 0.30/0.60 × 0.03 = 0.020 |
| 0.60 | 2 (kink) | 0.035 |
| 0.65 | 2 | 0.035 + 0.05/0.25 × 0.095 = **0.054** → `54_000_000_000_000_000_000_000_000` |
| 0.85 | 3 (kink) | 0.130 |
| 0.90 | 3 | 0.130 + 0.05/0.15 × 1.0 = 0.4633… |
| 1.00 | 3 | 1.130 (below the 1.25 cap) |

## Deposit rate

`calculate_deposit_rate(utilization, borrow_rate, reserve_factor)`:

```text
if utilization == 0 or reserve_factor ∉ [0, BPS): 0
rate_x_util      = half_up(utilization_ray × borrow_rate_ray / RAY)
deposit_rate_ray = half_up(rate_x_util × (BPS − reserve_factor_bps) / BPS)
```

The result has the same time unit as the input. `get_deposit_rate` returns annual RAY.

Example at utilization 0.65, borrow rate 0.054, `reserve_factor = 1500`:

```text
rate_x_util  = 35_100_000_000_000_000_000_000_000        // 0.0351
deposit_rate = 29_835_000_000_000_000_000_000_000        // 0.029835 = 2.9835% APR
```

## APR to APY

Views return simple annual rates. Label them APR (`get_deposit_rate / 1e27`
is an APR) and label compounded estimates APY:

```text
apr        = rate_ray / 1e27
apy_cont   = e^apr − 1                                   // idealized constant-rate approximation
apy_daily  = (1 + apr × 86_400_000 / 31_556_926_000)^365 − 1   // API APY estimate
```

The API computes the daily estimate from the curve at market utilization.
Supply APY compounds the supply APR, not borrow APY. The contract instead
runs the [Index accrual](#index-accrual) Taylor step, so realized growth
depends on update cadence, rate changes, and rounding. Use that algorithm for
contract-equivalent results.

| Rate | APR | `e^apr − 1` | daily compounding |
|---|---|---|---|
| borrow | 5.4000% | 5.5485% | 5.5443% |
| deposit | 2.9835% | 3.0285% | 3.0263% |

## Index accrual

Pool mutations accrue. Projection views (`get_bulk_indexes`,
`get_market_index`, `get_market_indexes_detailed`) and account balance and
risk views use indexes projected to now. Metadata and raw-position views do
not.

Each step has `delta_ms ≤ MILLISECONDS_PER_YEAR` (`MAX_COMPOUND_DELTA_MS`). The pool chunks longer gaps, and each chunk recomputes utilization with the revenue shares minted by the previous chunk. The formulas for one step are:

```text
per_ms     = half_up(annual_borrow_rate_ray / 31_556_926_000)          // calculate_borrow_rate
x          = per_ms × delta_ms                                           // RAY exponent, must fit i128
factor     = 1 + x + x²/2! + … + x⁸/8!   (each power half-up, each term half-up ÷ k!)   // eighth-order approximation
borrow_index' = min(half_up(borrow_index × factor / RAY), 10^36)         // update_borrow_index, monotone
room       = max(0, MAX_MARKET_VALUE_RAY − max(debt_value_ray, supply_value_ray))   // i128::MAX − RAY
borrow_index' = min(borrow_index', borrow_index + floor(room × RAY / borrowed))     // cap_borrow_index: interest ≤ room
interest   = half_up(borrowed × borrow_index' / RAY) − half_up(borrowed × borrow_index / RAY)
fee        = half_up(interest × reserve_factor / BPS);  rewards = interest − fee
supply_index' = clamp(floor((half_up(supplied × supply_index / RAY) + rewards) × RAY / supplied), supply_index, 10^36)
shortfall  = rewards − (half_up(supplied × supply_index' / RAY) − half_up(supplied × supply_index / RAY))   // booked to protocol
revenue_shares = min(floor((fee + shortfall) × RAY / supply_index'), i128::MAX − supplied)   // added to supplied and revenue
```

Monotonicity:

- Borrow index only grows, up to `MAX_BORROW_INDEX_RAY`, where interest stops.
- Accrual never reverts on market value. At `MAX_MARKET_VALUE_RAY` total debt or supply value, interest stops and the pool emits `MarketValueCeilingEvent`. Repay, withdraw, and liquidate still work.
- Only **bad-debt socialization** lowers the supply index (floor `SUPPLY_INDEX_FLOOR_RAW = 10^24`). A drop means a write-down, not an accrual bug.
- Revenue shares are in `supplied` and earn the supply index.

The example uses a borrow rate of 0.054 annual and 30 days (`2_592_000_000` ms):

```text
per_ms  = 1_711_193_289_232_291
x       = 4_435_413_005_690_098_272_000_000            // 0.0044354130
factor  = 1_004_445_264_008_993_483_975_660_215        // Taylor approximation; ideal e^x ≈ 1.0044452640089936
borrow_index 1.1315e27 → 1_136_529_816_226_176_127_118_459_533
```

## Valuation to USD WAD

The code in `common/src/rates/value.rs` uses one rounding direction throughout:

```text
asset_value_ray = round(shares × index / RAY)
asset_value_wad = round(asset_value_ray / 10^9)
usd_value_wad   = round(asset_value_wad × price_wad / WAD)
```

| Function | Rounding | Use |
|---|---|---|
| `position_value` | half-up | display, liquidation proportions |
| `position_value_floor` | floor | collateral in risk checks |
| `position_value_ceil` | ceil | debt in risk checks |

`price_wad` (USD per whole token, `MarketIndexView.price_wad`) is usable only
when `valid`. Valid prices are at most `MAX_REASONABLE_PRICE_WAD = 10^9 × WAD`.
Refused diagnostic values can exceed it. Risk numbers come from strict views
(`get_health_factor`, `get_total_collateral_usd`).

## Health factor and LTV weighting

Risk uses each position's stored `loan_to_value` and `liquidation_threshold`.
The health factor uses the threshold. Borrow, withdrawal, strategy risk checks, and
`get_ltv_collateral_usd` first refresh listed collateral LTVs plus any
liquidation tuple not favoring liquidators. Supply and non-liquidation
withdrawal also refresh eligible liquidation parameters.

A refresh never stores LTV at or above the stored threshold. When the stored
threshold is below the listed one, the position keeps the listing's gap,
floored at zero:

```text
refreshed_ltv_bps = stored_lt_bps >= listed_lt_bps
  ? listed_ltv_bps
  : max(listed_ltv_bps − (listed_lt_bps − stored_lt_bps), 0)
```

Consider a listing edit that raises the threshold but favors liquidators, with
a higher bonus or lower fee. The stored threshold stays. For example, the
stored values are `ltv 7500 / lt 8000`, and the listed values are
`ltv 8000 / lt 8500` with a higher bonus. A borrow then keeps
`ltv 7500 / lt 8000`. The same holds for `update_account_threshold` with
`has_risks = false`, which never changes the stored threshold.

`update_account_threshold` refreshes LTV. With `has_risks = true`, it also
refreshes threshold, bonus, and fees. A liquidator-favoring change needs a
hypothetical health factor `≥ 1.05 WAD`. With `has_risks = true`, the call
fails with `HealthFactorTooLow` only if the refresh drops the health factor
below both `1.05 WAD` and its prior value.

```text
for each supply position:
  value_hup   = position_value(shares, supply_index, price)          // half-up
  value_floor = position_value_floor(shares, supply_index, price)
  effective_ltv = min(loan_to_value_bps, liquidation_threshold_bps)
  total_collateral    += value_hup
  ltv_collateral      += floor(value_floor × half_up(effective_ltv × WAD / BPS) / WAD)
  weighted_collateral += floor(value_floor × half_up(liquidation_threshold × WAD / BPS) / WAD)
for each debt position:
  total_debt += position_value_ceil(shares, borrow_index, price)
health_factor_wad = total_debt == 0 ? i128::MAX : floor(weighted_collateral × WAD / total_debt)   // saturating at i128::MAX
```

| Convention | Formula | Liquidation boundary |
|---|---|---|
| Contract health factor | `weighted_collateral / total_debt` (WAD) | `< 1e18` |
| UI health percentage (`xoxno-ui/src/modules/lending/utils.ts::getHealthPercentage`) | `total_debt / weighted_collateral × 100` | `> 100` |

Borrow capacity (USD WAD) is `max(ltv_collateral − total_debt, 0)` after the
LTV refresh.

These parameters are illustrative and not live. XLM has `ltv 7500 / lt 7800`,
and USDC has `ltv 7600 / lt 8000`.

| Position | value (floor / half-up, WAD) | LTV-weighted | threshold-weighted |
|---|---|---|---|
| 4,080 XLM @ $0.25 | 1_020e18 / 1_020e18 | 765e18 | 795.6e18 |
| 1,031.4285714 USDC @ $1 (shares from the USDC example, index 1.083) | 1_031_428_571_428_571_428_571 / …571 | 783_885_714_285_714_285_713 | 825_142_857_142_857_142_856 |
| **totals** | total_collateral 2_051_428_571_428_571_428_571 | ltv_collateral 1_548_885_714_285_714_285_713 | weighted 1_620_742_857_142_857_142_856 |

The debt is 252.566964… USST (ceil) at $1.0897, which gives `total_debt = 275_222_220_982_142_857_144` (ceil). The half-up display is `…143`.

```text
HF = floor(1_620_742_857_142_857_142_856 × 1e18 / 275_222_220_982_142_857_144) = 5_888_851_748_086_195_444   // 5.8889
UI health percentage = 275.22 / 1620.74 × 100 = 16.98%
available borrow = 1_548_885_714_285_714_285_713 − 275_222_220_982_142_857_144 = 1_273_663_493_303_571_428_569   // $1,273.66
```

## Post-action risk gate

The gate runs for borrow, withdrawal, and strategy completion on an account
with debt. Listing, cap, halt, cash, and utilization checks also apply.

```text
ltv_collateral   ≥ total_debt                      // else InsufficientCollateral
health_factor    ≥ 1e18                            // else InsufficientCollateral
ltv_collateral   ≥ min_borrow_collateral_usd_wad   // when the floor is non-zero; else MinBorrowCollateralNotMet
low_decimal_collateral_base_units ≥ 2             // each supplied market below 3 decimals; else MinBorrowCollateralNotMet
```

`get_min_borrow_collateral_usd` returns the USD floor (default `5 × WAD`).
Two base units are 2 tokens at zero decimals, 0.2 tokens at one decimal, and
0.02 tokens at two decimals. To close an account, repay debt before you
withdraw collateral.

Example for the account above: the three checks are `1_548.89e18 ≥ 275.22e18`, `HF 5.89 ≥ 1`, and `1_548.89e18 ≥ 5e18`. All three hold, so the check passes. Withdrawing all USDC leaves `ltv_collateral = 765e18 ≥ 275.22e18`, still passing. The $5 floor binds on LTV-weighted collateral: XLM at 75% LTV needs at least $6.67 (`5 / 0.75`) while any debt remains.

## Liquidation: bonus, close amount, seizure, fees

The code is in `contracts/controller/src/positions/liquidation/{curve.rs, math.rs}`. The symbols are USD WAD unless stated. `D` is risk debt (ceil), `C` is unweighted collateral (half-up), `W` is threshold-weighted collateral (floor), `HF` is the health factor, `H` is the target health factor, and `b` is the bonus in BPS.

### Blended threshold and bonus bounds

```text
p       = C == 0 ? 0 : half_up(W × WAD / C)                      // proportion_seized
t       = clamp(ceil(p × BPS / WAD), 1, BPS)
max_bps = p ≤ 0 ? 0 : floor(BPS × (BPS − t) / t)                 // max_bonus_for_threshold
base_bps = min(half_up(Σ (value_i × bonus_i) / C), max_bps)   // USD-weighted stamped bonuses, one rounding; 0 when C == 0
```

### Bonus curve (`SpokeConfig`: `liquidation_target_hf_wad`, `hf_for_max_bonus_wad`, `liquidation_bonus_factor_bps`)

```text
if HF ≥ H:                       bonus = base
else:
  scale  = H ≤ hf_for_max_bonus ? WAD : min(WAD, half_up((H − HF) × WAD / (H − hf_for_max_bonus)))
  bonus  = base + half_up(half_up((max − base) × scale / WAD) × factor / BPS)
cap_bps  = (p > 0 and HF < WAD) ? floor(HF × BPS / p) − BPS : none         // HF-preserving ceiling
if cap_bps is not none and C < D:      quote = (min(D, floor(C × WAD / (WAD + base_wad))), base)  // insolvent: what C backs
elif cap_bps is not none and cap_bps < base: quote = (D, max(cap_bps, 0))                            // band D ≤ C < D × (1 + base)
else:                  b = cap_bps is none ? bonus : min(bonus, cap_bps)
```

### Close amount

```text
one_plus_b   = WAD + half_up(b × WAD / BPS)
d_max        = min(D, half_up(C × WAD / one_plus_b))
denom_term   = half_up(p × one_plus_b / WAD)
target_debt  = half_up(H × D / WAD)
ideal = (H ≤ denom_term or target_debt ≤ W) ? d_max
      : min(d_max, half_up((target_debt − W) × WAD / (H − denom_term)))
if 0 < D − ideal < 5 WAD: ideal = D                                          // dust-debt promotion
```

`get_liquidation_estimate` returns `max_payment_wad` (`ideal`, capped by the
offer, with each debt leg at most its ceiled balance) and `bonus_rate_bps`.

A partial offer in the full-close band uses the capped bonus. Rounding can
change the ratio, and low-decimal rules can reject it. Simulate the write.

For a full-debt quote, the controller pulls each merged offer and the pool
refunds the excess above each leg's ceiled debt. The controller pulls legs at
the rounded-up ceiling but credits the floor value cleared, so the closing
fraction buys no collateral and `max_payment_wad ≤ D`.

For other quotes, the planner trims from the last leg backward. On a solvent
account, it floors the refund, and a kept leg can round up one base unit. On an
insolvent account, it floors the kept amount. The planner also drops empty
legs. If no leg is left, the call reverts with `InvalidPayments`.

Worked example (spoke defaults `H = 1.1`, `hf_for_max_bonus = 0.8`, `factor = 10_000`, with a single XLM collateral with stamped bonus 900, threshold 7800):

```text
C = 1_000 WAD, W = 780 WAD, D = 800 WAD → HF = 0.975, p = 0.78
t = 7_800 → max_bps = floor(10_000 × 2_200 / 7_800) = 2_820;  base = 900
scale = (1.1 − 0.975) / (1.1 − 0.8) = 0.41667;  bonus = 900 + half_up(1_920 × 0.41667) = 1_700
cap   = floor(0.975 × 10_000 / 0.78) − 10_000 = 2_500 → b = 1_700
one_plus_b = 1.17;  d_max = min(800, 854.70) = 800;  denom_term = 0.9126;  target_debt = 880
ideal = (880 − 780) / (1.1 − 0.9126) = 533.617929562433297759 WAD;  remaining 266.38 ≥ 5 → no promotion
```

### Seizure and fees per collateral

```text
total_seizure_usd = half_up(repay_usd × one_plus_b / WAD)
share_i           = half_up(value_i × WAD / C)
seizure_usd_i     = half_up(total_seizure_usd × share_i / WAD)
seizure_ray_i     = half_up(seizure_usd_i × WAD / price_i) × 10^9
capped_ray        = min(seizure_ray_i, half_up(shares × supply_index / RAY));  full = capped_ray == held value
base_ray          = floor(seizure_ray_i × RAY / (one_plus_b × 10^9))    // principal from the UNCAPPED seizure; ×10^9 is the exact WAD→RAY upscale
bonus_ray         = max(0, capped_ray − base_ray)
fee_ray           = half_up(bonus_ray × liquidation_fees_bps / BPS)
```

| Seize mode | Liquidator receives | Protocol fee |
|---|---|---|
| `SeizeMode::Transfer` | tokens: rescaled `capped_ray` minus the fee | base units, withheld from the transfer |
| `SeizeMode::Credit(account_id)` | shares: `seized_scaled − fee_scaled` credited to the receiver account | `fee_scaled` shares |

`SeizeMode::Transfer`:

- `capped_ray` rescales **floor** on partial seizure, **half-up** on full (the pool still pays the floor claim).
- If `fee_ray > 0`, fee = `max(1, floor(fee_ray / 10^(27−d)))`, capped at `floor((paid_ray − base_ray) / 10^(27−d))`, or 0 when payout ≤ principal.

`SeizeMode::Credit(account_id)`:

```text
seized_scaled = floor(capped_ray × RAY / supply_index)                  // exact held shares on full close
bonus_scaled  = min(floor(bonus_ray × RAY / supply_index), seized_scaled)
fee_scaled    = ceil(bonus_scaled × fees_bps / BPS)
```

Under-delivery means that the measured repayment USD is below plan. It scales
every seizure field by `received / planned` (floored) and caps each transfer at
its scaled shares' floor value. Credit fees are recomputed. With a seized leg
below 3 decimals, the call reverts with `InvalidPayments`.

Planning drops legs that round to zero tokens or shares. Collateral below 3
decimals is the account's only supplied market. Full-debt repayment rounds
seizure up to base units. Partial repayment rounds down and trims to backed value. A
low-decimal plan with no seizure fails `InvalidPayments`. The 2-base-unit
[post-action minimum](#post-action-risk-gate) does not apply to repayment or
liquidation.

Example continued (repay 533.6179 USD, XLM at $0.25, supply index 1 RAY, fees 1200 bps):

```text
total_seizure_usd = 624_332_977_588_046_958_378 WAD ($624.33) → 2_497.331910352187833512 XLM (RAY 2_497_331_910_352_187_833_512_000_000_000)
base_ray  = 2_134_471_718_249_733_191_035_897_435_897;  bonus_ray = 362_860_192_102_454_642_476_102_564_103
fee_ray   = 43_543_223_052_294_557_097_132_307_692
Transfer: seized 24_973_319_103 stroops (2,497.3319103 XLM), fee 435_432_230 stroops (43.543223 XLM), liquidator receives the difference
Credit:   seized shares 2_497_331_910_352_187_833_512_000_000_000, fee shares 43_543_223_052_294_557_097_132_307_693, liquidator shares 2_453_788_687_299_893_276_414_867_692_307
```

### Low-decimal collateral quotes

Below 3 decimals, one quote unit is one base unit (0.01 token at two
decimals), worth `U` USD WAD. These rules need `C ≥ D`, a curve quote below `D`, and
at least one base unit in the sole supply position. The first matching rule
applies:

```text
R             = Σ over debt legs of the WAD USD value of one base unit of that debt token
unit_at_bonus = floor(U × WAD / one_plus_b)
margin        = max(floor(U / 1_000_000), 1)                           // raw WAD
raised        = ceil((U + margin) × WAD / one_plus_b)
if unit_at_bonus ≥ D + R:                                      ideal = D       // full close; the leg rounds up to one unit
elif half_up(ideal × one_plus_b / WAD) < U + margin and raised < D: ideal = raised  // seize one unit, refund the margin
else:                                                          ideal unchanged
```

The raised seizure takes one base unit and refunds the unseized margin in
debt base units, rounded down. It is a ceiling, and smaller offers are not
raised. It is never promoted to full debt, so remaining debt can be below
`5 WAD`. An offer below one unit's backed value fails `InvalidPayments`. An offer
between `U / one_plus_b` and `raised` still seizes one unit.

The cap uses the rounded health factor and proportion, so rounding can change
the resulting ratio. In the band `unit_at_bonus − R < D ≤ raised` no rule
applies. If the curve quote backs less than one unit, every offer fails until
accrual or a price change.

A rule-1 full close trades debt `D` for one unit worth `U`. The effective bonus is
`U / D − 1`, and the reported bonus is still `b`. For `k` held units and threshold `LT`,
the effective bonus is about `1 / (k × LT) − 1` when `HF < 1`. The borrower loses
`U − D × one_plus_b / WAD` beyond the normal bonus. Listings below 3 decimals
have no liquidation fee. Size with `get_liquidation_estimate`, not the curve
alone.

## Bad-debt socialization

`is_socializable_bad_debt` defines eligibility:

- Permissionless `clean_bad_debt` and the check after `liquidate` require `total_debt > total_collateral` and `total_collateral ≤ 5 WAD` (`BAD_DEBT_USD_THRESHOLD`).
- Owner-only `force_socialize_bad_debt` drops the collateral cap.

`contracts/pool/src/interest.rs::apply_bad_debt_to_supply_index` then lowers only the affected market's supply index ([formulas.md#bad-debt](https://github.com/XOXNO/rs-lending-xlm/blob/main/docs/reference/formulas.md#bad-debt)).

The example has 2,000,000 USDC of shares at index 1.083 (`total_supply_ray = 2_166_000e27`) and bad debt of 30,000 USDC:

```text
reduction = 986_149_584_487_534_626_038_781_163   // 0.98615
index 1.083e27 → 1_067_999_999_999_999_999_999_999_999   // ≈ 1.068
```

## Liquidation buffer

Every debt mint (`borrow`, strategy openings, gross of fee) keeps 200 bps of floored supplied token value in cash. Otherwise the call fails with `InsufficientLiquidity` (INV-ACCT-07, `contracts/pool/src/guards.rs::require_liquidation_buffer`). The backing-shortfall gate on supply entry is in [formulas.md#backing-and-cash-constraints](https://github.com/XOXNO/rs-lending-xlm/blob/main/docs/reference/formulas.md#backing-and-cash-constraints).

```text
reserved    = ceil(floor_supply_tokens × 200 / BPS)
cash − draw ≥ reserved
```

Example: supplied claim 9,999,999.9999999 USDC (floor), cash 3,500,000 USDC gives `reserved = 200_000 USDC`. The largest single borrow draw is `3_300_000 USDC` (`33_000_000_000_000` base units) before `max_utilization` is considered.

## Caps in the scaled domain

`SpokeAssetConfig.supply_cap` and `borrow_cap` are per-spoke base units,
enforced on regular entries. Liquidation credits bypass the supply cap. Zero
closes that side and is not unlimited. `i128::MAX` is rejected. Entries compare
shares:

```text
cap_scaled     = floor_saturating(cap × 10^(27−d) × RAY / index)      // calculate_scaled_cap, supply or borrow index
usage_scaled   = SpokeUsageRaw.supplied_scaled_ray | borrowed_scaled_ray   // get_spoke_usage
usage + new_shares ≤ cap_scaled   else SpokeSupplyCapReached | SpokeBorrowCapReached
headroom_tokens = floor(floor((cap_scaled − usage) × index / RAY) / 10^(27−d))
```

Exits skip caps. Conversion saturates at `i128::MAX`. The cap maximum is `MAX_MARKET_ENTRY_RAY / 10^(27−d)` (`max_cap_for_decimals`), about 21.27 billion whole tokens. The pool applies that ceiling to market total supply and debt after every supply, borrow, strategy opening and flash loan (`MarketCeilingReached`, 136).

Example: USDC `supply_cap = 50_000_000_000_000` (5,000,000 USDC), usage 4,000,000 USDC of shares, index 1.083:

```text
cap_scaled = 4_616_805_170_821_791_320_406_278_855_032_317
usage      = 3_693_444_136_657_433_056_325_023_084_025_854
headroom   = 923_361_034_164_358_264_081_255_771_006_463 shares → 9_999_999_999_999 base units (999,999.9999999 USDC)
```

## Flash-loan and strategy fees

`Bps::flash_loan_fee_on` (`common/src/math/fp.rs`) is used by `contracts/pool/src/ops/flash.rs` and `ops/strategy.rs`:

```text
fee = half_up(principal × fee_bps / BPS);  if fee_bps > 0 and fee == 0: fee = 1
```

`flashloan_fee ≤ MAX_FLASHLOAN_FEE_BPS = 500`. Strategies charge it only with `charge_fee` (`multiply`, `swap_debt`) and revert with `StrategyFeeExceeds` if `fee > principal`. `flash_position` and `migrate_from_blend` are fee-free. Example at 9 bps: 1,000 USDC gives a fee of `9_000_000` base units (0.9 USDC). 100 base units give a fee of 1 base unit.

## Numeric limits

The arithmetic bounds are in [formulas.md#numeric-limits](https://github.com/XOXNO/rs-lending-xlm/blob/main/docs/reference/formulas.md#numeric-limits). They cover decimals, index ceiling and floor, rate cap, token-to-RAY input maximum, and overflow behaviour. The controller limits are:

| Bound | Value | Consequence |
|---|---|---|
| View input batches | `MAX_VIEW_INPUTS = 256` | `get_market_indexes_detailed` and `get_liquidation_estimate` revert above it (`InvalidPayments`) |
| Positions per side | `POSITION_LIMIT_MAX = 5` upper bound on the configured limit | `PositionLimitExceeded` |

## Which view rounds how

| View | Contract | Rounding and unit |
|---|---|---|
| `get_collateral_amount`, `get_borrow_amount` | controller | half-up both steps, base units, projected index |
| `get_supplied_amount`, `get_borrowed_amount`, `get_revenue` | pool | half-up (revenue: floor), base units, stored index |
| `get_reserves` | pool | raw `cash`, no rounding |
| `get_health_factor`, `get_liquidation_collateral`, `get_ltv_collateral_usd` | controller | floor-valued collateral, ceil debt, WAD. `get_ltv_collateral_usd` refreshes LTV stamps first |
| `is_liquidatable` | controller | bool: debt present and HF < WAD |
| `get_total_collateral_usd`, `get_total_borrow_usd` | controller | half-up, WAD (display, not the ceiled risk debt) |
| `get_borrow_rate`, `get_deposit_rate` | pool | annual RAY fraction, stored index |
| `get_utilisation` | pool | RAY, stored index |
| `get_market_index`, `get_bulk_indexes`, `get_market_indexes_detailed` | controller and pool | RAY indexes projected to now |
| `get_liquidation_estimate` | controller | `max_payment_wad` WAD, `bonus_rate_bps` BPS, seizure in base units (`Transfer`) or RAY shares (`Credit`) |
| `get_min_borrow_collateral_usd` | controller | WAD |
| `get_spoke_usage` | controller | RAY shares |
