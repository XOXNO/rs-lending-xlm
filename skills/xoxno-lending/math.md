# XOXNO Lending math reference

Normative formulas, units, and rounding that the contract views, the `@xoxno/sdk-js` math helpers, and liquidation sizing must agree with. Pseudocode uses raw integers; `floor`, `ceil`, and `half_up` apply to the whole fraction `x * y / d` with exact intermediates (`I256` on chain, `BigInt` off chain). Never evaluate a formula in float `Number`.

Sources: [`docs/reference/formulas.md`](../../docs/reference/formulas.md), `common/src/{constants,math,rates}`, `contracts/{pool,controller}`; every worked number was recomputed from them with exact integer arithmetic.

## Units

| Quantity | Scale | Constant (`common/src/constants/shared.rs`) |
|---|---|---|
| Shares (`scaled_amount`), indexes, rate-engine asset values, rates, utilization | RAY = 10^27 | `RAY`, `RAY_DECIMALS = 27` |
| USD values, prices per whole token, health factor | WAD = 10^18 | `WAD`, `WAD_DECIMALS = 18` |
| LTV, liquidation threshold, bonus, fees, reserve factor | BPS = 10,000 | `BPS` |
| Token transfers, cash, caps, `get_collateral_amount`, `get_borrow_amount` | Token base units at the asset's decimals (3..=18) | `MIN_ASSET_DECIMALS = 3`, `MAX_ASSET_DECIMALS = 18` |
| Accrual time | Milliseconds | `MILLISECONDS_PER_YEAR = 31_556_926_000`, `MS_PER_SECOND = 1_000` |

Rescaling between units multiplies or divides by a power of ten:

| From | To | Factor |
|---|---|---|
| token base units (`d` decimals) | RAY | `× 10^(27 - d)` (exact) |
| RAY | token base units | `÷ 10^(27 - d)` (rounded) |
| RAY | WAD | `÷ 10^9` (rounded) |
| WAD | RAY | `× 10^9` (exact) |
| BPS | WAD ratio | `half_up(bps × WAD / BPS)` |

A RAY quantity is never divided by WAD (10^18) or by `10^d` directly; the divisor is `10^(27 - d)`.

## Rounding vocabulary

`common/src/math/fp_core.rs`:

```text
floor(x, y, d)   = ⌊x·y / d⌋
ceil(x, y, d)    = ⌈x·y / d⌉
half_up(x, y, d) = ⌊(x·y + ⌊d/2⌋) / d⌋          // x, y ≥ 0, d > 0
```

## Shares and token amounts

Positions store shares (`scaled_amount`, RAY); value = shares × index, and two rounding steps sit between shares and a token transfer ([formulas.md#shares-and-token-amounts](../../docs/reference/formulas.md#shares-and-token-amounts)). Rounding per boundary (`common/src/rates/scaling.rs`, `contracts/pool/src/cache/scale.rs`):

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

- A withdrawal request `≥` the half-up displayed supply balance burns **all** supply shares and pays the **floor** balance. A request below it is partial: it pays exactly the request and burns `ceil` shares. When the floor balance is below the half-up balance, a request sized from the floor balance is partial and leaves dust shares.
- A repayment `≥` the ceiled debt balance burns **all** debt shares and refunds the excess. A smaller repayment burns `floor` shares.
- Positive supply, borrow, gross withdrawal, net repayment and net settlement amounts must move at least one share; a zero-share result reverts with `SupplyRoundsToZeroShares`, `BorrowRoundsToZeroShares`, `WithdrawRoundsToZeroShares`, `RepayRoundsToZeroShares`, or `NetSettleRoundsToZeroShares` (`contracts/pool/src/ops/*.rs`).
- `withdraw` with amount `0` means the whole position (`contracts/controller/src/positions/supply.rs` maps `0` to `WITHDRAW_ALL_SENTINEL = i128::MAX`). Any `0` leg for a hub asset makes that hub asset's aggregate a withdraw-all sentinel, regardless of positive legs before or after it (`payments.rs`, `ZeroLeg::MeansAll`). Pass `0` to close instead of computing a balance.

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

A partial withdrawal of 500 USDC burns `ceil(5e9 × 10^20 × RAY / 1.083e27) = 461_680_517_082_179_132_040_627_885_504` shares (floor would give `…503`). To close the position, request `≥ 10_314_285_714` (the half-up figure) or pass `0`; the payout is `10_314_285_714`.

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

### API position fields are RAY quantities

`AccountPositionDto` field semantics (`supplyScaledRay`, `supplyIndexRay`, `liveSupplyIndexRay`, `supplyAmount`, `entry*Bps`, and the borrow twins): [../xoxno-lending-sdk/reads.md#units-and-dto-semantics](../xoxno-lending-sdk/reads.md#units-and-dto-semantics). `supplyAmount` / `borrowAmount` are RAY token quantities: divide by `10^(27 - decimals)` for base units, or recompute from shares when a fresher index is available:

```ts
const RAY = 10n ** 27n;

type Rounding = 'floor' | 'halfUp' | 'ceil';

function mulDiv(x: bigint, y: bigint, d: bigint, r: Rounding): bigint {
  const p = x * y;
  if (r === 'floor') return p / d;
  if (r === 'ceil') return p % d === 0n ? p / d : p / d + 1n;
  return (p + d / 2n) / d;
}

/** shares (RAY) -> token base units, rounding both steps the same way. */
export function sharesToBaseUnits(
  scaledRay: string,
  indexRay: string,
  decimals: number,
  rounding: Rounding,
): bigint {
  const valueRay = mulDiv(BigInt(scaledRay), BigInt(indexRay), RAY, rounding);
  return mulDiv(valueRay, 1n, 10n ** BigInt(27 - decimals), rounding);
}

// position: AccountPositionDto; reserve.assetDecimals from ReserveDto
// const index = position.liveSupplyIndexRay ?? position.supplyIndexRay;
// display  = sharesToBaseUnits(position.supplyScaledRay, index, 7, 'halfUp');
// payout   = sharesToBaseUnits(position.supplyScaledRay, index, 7, 'floor');
// fullRepay= sharesToBaseUnits(position.borrowScaledRay, position.liveBorrowIndexRay ?? position.borrowIndexRay, 18, 'ceil');
```

## Utilization

`common/src/rates/curve.rs::utilization`, `contracts/pool/src/cache/scale.rs::calculate_utilization`:

```text
supply_value_ray = half_up(total_supplied_shares × supply_index / RAY)
debt_value_ray   = half_up(total_borrowed_shares × borrow_index / RAY)
utilization_ray  = supply_value_ray == 0 ? 0 : half_up(debt_value_ray × RAY / supply_value_ray)
```

Totals are the **hub pool's** totals (`get_supplied_amount`, `get_borrowed_amount`, `ReserveDto.hubPool`), not one spoke's slice; every spoke on a hub shares the rate. `ReserveDto.utilization` and `ReserveDto.suppliedShort/borrowedShort` differ in scope for that reason.

Example: 10,000,000 USDC supplied value, 6,500,000 USDC borrowed value → `utilization_ray = 650_000_000_000_000_000_000_000_000` (0.65).

`max_utilization` (`MarketParams`, RAY) is a guard, not a curve input: after borrow, user withdrawal, or revenue claim the pool requires `ceil(ceil(debt value) / floor(supply value)) ≤ max_utilization` (values in RAY) unless supply or debt is zero or `max_utilization ≥ 1 RAY`; debt against a supply value that floors to zero is rejected (`contracts/pool/src/guards.rs::require_utilization_below_max`, error `UtilizationAboveMax`). Liquidation withdrawals skip it.

## Borrow-rate curve

`calculate_annual_borrow_rate` (`common/src/rates/curve.rs`) with `MarketParams { base_borrow_rate, slope1, slope2, slope3, mid_utilization, optimal_utilization, max_borrow_rate }` (all RAY):

```text
u = min(utilization_ray, RAY)
if u < mid:            rate = base + half_up(half_up(u × slope1 / RAY) × RAY / mid)
else if u < optimal:   rate = base + slope1 + half_up(half_up((u − mid) × slope2 / RAY) × RAY / (optimal − mid))
else:                  rate = base + slope1 + slope2 + half_up(half_up((u − optimal) × slope3 / RAY) × RAY / (RAY − optimal))
rate = min(rate, max_borrow_rate)
```

Each segment adds its full slope at the kink: `slope1` is the total rise from 0 to `mid`, `slope2` from `mid` to `optimal`, `slope3` from `optimal` to 100%. `max_borrow_rate ≤ MAX_BORROW_RATE_RAY = 2 RAY` (200% APR). The result is an **annual RAY fraction**; `get_borrow_rate` returns it as `i128`.

`ReserveIrmCurveDto` (API) carries the same parameters as `baseRateRay`, `slope1Ray`, `slope2Ray`, `slope3Ray`, `midUtilizationRay`, `optimalUtilizationRay`, `maxUtilizationRay`, `maxBorrowRateRay`, `reserveFactorBps`. Map by field name; the swagger descriptions of which slope belongs to which segment are stale (Observed: they swap `optimal` and `mid`).

Worked example with illustrative parameters (not a live market; read live parameters with the pool's `get_sync_data`): base 0.005, slope1 0.03, slope2 0.095, slope3 1.0, mid 0.60, optimal 0.85, max 1.25 (all × RAY):

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

Same time unit as the input rate: annual in, annual out. `get_deposit_rate` returns the annual RAY fraction.

Example at utilization 0.65, borrow rate 0.054, `reserve_factor = 1500`:

```text
rate_x_util  = 35_100_000_000_000_000_000_000_000        // 0.0351
deposit_rate = 29_835_000_000_000_000_000_000_000        // 0.029835 = 2.9835% APR
```

## APR to APY

Views return simple annual rates. For an idealized constant rate with continuous
compounding, the closed-form APY is:

```text
apr        = rate_ray / 1e27
apy_cont   = e^apr − 1                                   // idealized constant-rate approximation
apy_daily  = (1 + apr × 86_400_000 / 31_556_926_000)^365 − 1   // what api.xoxno.com ReserveDto.supplyApy/borrowApy report
```

`xoxno-api-v2/src/endpoints/stellar-lending/stellar-lending.accrual.ts::aprRayToApy` computes the daily form from the curve at live hub utilization; `supplyApy` compounds the deposit **APR** (not the compounded borrow APY). Label view values APR and compounded values APY; never present `get_deposit_rate / 1e27` as APY.

The contract does not evaluate `e^apr − 1`. It converts the annual rate to a
per-millisecond RAY rate with integer half-up rounding, applies an eighth-order
Taylor approximation over each elapsed interval, and rounds every term. Gaps
longer than one year are chunked, with utilization and revenue shares updated
between chunks. Consequently, realized index growth depends on update cadence,
rate changes, and integer rounding; use the index-accrual algorithm below when
contract-equivalent results are required.

Example (rates above):

| Rate | APR | `e^apr − 1` | daily compounding |
|---|---|---|---|
| borrow | 5.4000% | 5.5485% | 5.5443% |
| deposit | 2.9835% | 3.0285% | 3.0263% |

## Index accrual

`common/src/rates/simulate.rs::accrue_step` runs on every mutation (`contracts/pool/src/interest.rs`) and in the projection views (`get_bulk_indexes`, controller `get_market_index`, `get_market_indexes_detailed`, and every controller account view, which fetch simulated indexes through `Context::cached_market_index`).

Per step of `delta_ms ≤ MILLISECONDS_PER_YEAR` (`MAX_COMPOUND_DELTA_MS`; longer gaps are chunked, each chunk recomputing utilization with the revenue shares minted by the previous chunk):

```text
per_ms     = half_up(annual_borrow_rate_ray / 31_556_926_000)          // calculate_borrow_rate
x          = per_ms × delta_ms                                           // RAY exponent, must fit i128
factor     = 1 + x + x²/2! + … + x⁸/8!   (each power half-up, each term half-up ÷ k!)   // eighth-order approximation
borrow_index' = min(half_up(borrow_index × factor / RAY), 10^36)         // update_borrow_index, monotone
interest   = half_up(borrowed × borrow_index' / RAY) − half_up(borrowed × borrow_index / RAY)
fee        = half_up(interest × reserve_factor / BPS);  rewards = interest − fee
supply_index' = clamp(floor((half_up(supplied × supply_index / RAY) + rewards) × RAY / supplied), supply_index, 10^36)
shortfall  = rewards − (half_up(supplied × supply_index' / RAY) − half_up(supplied × supply_index / RAY))   // booked to protocol
revenue_shares = min(floor((fee + shortfall) × RAY / supply_index'), i128::MAX − supplied)   // added to supplied and revenue
```

Monotonicity:

- Borrow index only grows (capped at `MAX_BORROW_INDEX_RAY`; at the cap no further interest accrues).
- Supply index never decreases from accrual, but **bad-debt socialization lowers it** (see below), floored at `SUPPLY_INDEX_FLOOR_RAW = 10^24`. A supply index below an earlier reading means a bad-debt write-down on that market, not an accrual bug.
- Revenue shares are part of `supplied`; the supply index applies to them like any supplier.

Example: borrow rate 0.054 annual, 30 days (`2_592_000_000` ms):

```text
per_ms  = 1_711_193_289_232_291
x       = 4_435_413_005_690_098_272_000_000            // 0.0044354130
factor  = 1_004_445_264_008_993_483_975_660_215        // Taylor approximation; ideal e^x ≈ 1.0044452640089936
borrow_index 1.1315e27 → 1_136_529_816_226_176_127_118_459_533
```

## Valuation to USD WAD

Three same-rounding steps, [formulas.md#valuation-and-health](../../docs/reference/formulas.md#valuation-and-health); `common/src/rates/value.rs` names them `position_value` (half-up: displayed totals, liquidation proportions), `position_value_floor` (collateral in risk gates), `position_value_ceil` (debt in risk gates). `price_wad` is USD per whole token from `MarketIndexView.price_wad`, at most `MAX_REASONABLE_PRICE_WAD = 10^9 × WAD`.

## Health factor and LTV weighting

`contracts/controller/src/risk/totals.rs::calculate_account_risk_totals`. Weights come from the **position's stored** parameters (`AccountPositionRaw.loan_to_value`, `.liquidation_threshold`; API `entryLtvBps`, `entryLiquidationThresholdBps`); HF always uses the stored `liquidation_threshold`. The borrow, withdraw and strategy risk gates and `get_ltv_collateral_usd` first restamp each listed supply leg's stored LTV to the current `SpokeAssetConfig.loan_to_value`. A supply, or a non-liquidation withdrawal that leaves shares in a listed leg, refreshes that leg's stored LTV and applies the gated threshold, bonus and fee refresh. `update_account_threshold` refreshes stored LTV. With `has_risks = true` it also refreshes threshold, bonus, and fees. It skips a change that favours liquidators unless HF stays ≥ 1.05 WAD, and reverts with `HealthFactorTooLow` if the final HF is below 1.05 WAD.

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

Two conventions for the same account:

| Convention | Formula | Liquidation boundary |
|---|---|---|
| Contract health factor | `weighted_collateral / total_debt` (WAD) | `< 1e18` |
| UI health percentage (`xoxno-ui/src/modules/lending/utils.ts::getHealthPercentage`) | `total_debt / weighted_collateral × 100` | `> 100` |

Borrow capacity in USD WAD: `available = ltv_collateral − total_debt` (zero when negative). LTV, not liquidation threshold, bounds borrowing.

Worked example (mainnet spoke 1 stamps: XLM `ltv 7500 / lt 7800`, USDC `ltv 7600 / lt 8000`):

| Position | value (floor / half-up, WAD) | LTV-weighted | threshold-weighted |
|---|---|---|---|
| 4,080 XLM @ $0.25 | 1_020e18 / 1_020e18 | 765e18 | 795.6e18 |
| 1,031.4285714 USDC @ $1 (shares from the USDC example, index 1.083) | 1_031_428_571_428_571_428_571 / …571 | 783_885_714_285_714_285_713 | 825_142_857_142_857_142_856 |
| **totals** | total_collateral 2_051_428_571_428_571_428_571 | ltv_collateral 1_548_885_714_285_714_285_713 | weighted 1_620_742_857_142_857_142_856 |

Debt: 252.566964… USST (ceil) at $1.0897 → `total_debt = 275_222_220_982_142_857_144` (ceil; half-up display `…143`).

```text
HF = floor(1_620_742_857_142_857_142_856 × 1e18 / 275_222_220_982_142_857_144) = 5_888_851_748_086_195_444   // 5.8889
UI health percentage = 275.22 / 1620.74 × 100 = 16.98%
available borrow = 1_548_885_714_285_714_285_713 − 275_222_220_982_142_857_144 = 1_273_663_493_303_571_428_569   // $1,273.66
```

## Post-action risk gate

`contracts/controller/src/risk/validation.rs::require_post_pool_risk_gates` runs after borrow, withdraw, and every strategy that leaves debt. Debt-free accounts skip it. With debt, all three must hold or the call reverts:

```text
ltv_collateral   ≥ total_debt                      // else InsufficientCollateral
health_factor    ≥ 1e18                            // else InsufficientCollateral
ltv_collateral   ≥ min_borrow_collateral_usd_wad   // when the floor is non-zero; else MinBorrowCollateralNotMet
```

`get_min_borrow_collateral_usd` returns the floor (default `5 × WAD`; API `StellarLendingLiveStateDto.minBorrowCollateralUsdWad`). The gate applies to `withdraw` while any debt remains, so close a position in the order repay → withdraw. Listing, cap, pause/freeze, and `max_utilization` gates apply in addition.

Example (account above): `1_548.89e18 ≥ 275.22e18`, `HF 5.89 ≥ 1`, `1_548.89e18 ≥ 5e18` → passes. Withdrawing all USDC first leaves `ltv_collateral = 765e18 ≥ 275.22e18` and still passes. The $5 floor binds on LTV-weighted collateral: for XLM stamped at 75% LTV, any debt at all requires at least $6.67 of XLM collateral to remain (`5 / 0.75`).

## Liquidation: bonus, close amount, seizure, fees

`contracts/controller/src/positions/liquidation/{curve.rs, math.rs}`. Symbols, all USD WAD unless stated: `D` risk debt (ceil), `C` unweighted collateral (half-up), `W` threshold-weighted collateral (floor), `HF`, `H` target health factor, `b` bonus in BPS.

### Blended threshold and bonus bounds

```text
p       = C == 0 ? 0 : half_up(W × WAD / C)                      // proportion_seized
t       = clamp(ceil(p × BPS / WAD), 1, BPS)
max_bps = p ≤ 0 ? 0 : floor(BPS × (BPS − t) / t)                 // max_bonus_for_threshold
base_bps = min(Σ half_up(half_up(value_i × WAD / C) × bonus_i / WAD), max_bps)   // USD-weighted stamped bonuses; 0 when C == 0
```

### Bonus curve (`SpokeConfig`: `liquidation_target_hf_wad`, `hf_for_max_bonus_wad`, `liquidation_bonus_factor_bps`)

```text
if HF ≥ H:                       bonus = base
else:
  scale  = H ≤ hf_for_max_bonus ? WAD : min(WAD, half_up((H − HF) × WAD / (H − hf_for_max_bonus)))
  bonus  = base + half_up(half_up((max − base) × scale / WAD) × factor / BPS)
cap_bps  = (p > 0 and HF < WAD) ? floor(HF × BPS / p) − BPS : none         // HF-preserving ceiling
if cap and C < D:      quote = (min(D, floor(C × WAD / (WAD + base_wad))), base)  // insolvent: what C backs
elif cap and cap < base: quote = (D, max(cap, 0))                            // band D ≤ C < D × (1 + base)
else:                  b = cap ? min(bonus, cap) : bonus
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

`get_liquidation_estimate(account_id, debt_payments, seize_mode)` returns `max_payment_wad` (= `ideal` capped by what was offered) and `bonus_rate_bps`. Offered payments are capped per asset at the ceiled debt balance; excess is listed in `refunds`. Any payment up to `ideal` is accepted: a partial in the band pays `bonus = cap`, which keeps `C / D` and HF from falling. When the quote is the full debt nothing is trimmed: `max_payment_wad` credits each leg up to its ceiled debt and can exceed `D` by unit rounding, `refunds` lists only the part of each offer above its leg's ceiled debt, and `liquidate` pulls each merged offered amount while the pool refunds exactly that excess. Otherwise the offer is trimmed to `ideal` from the last leg backward and `liquidate` pulls the trimmed amount. On a solvent account the trim floors the refund, so a kept leg can round up by one token unit; on an insolvent account (`C < D`) it floors the kept amount instead, drops a leg that keeps nothing, and a plan with no leg left makes `liquidate` revert with `InvalidPayments`. `FullCloseRequired` (#135) is reserved and never raised.

Worked example (spoke defaults `H = 1.1`, `hf_for_max_bonus = 0.8`, `factor = 10_000`; single XLM collateral with stamped bonus 900, threshold 7800):

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
| `SeizeMode::Transfer` | tokens: `capped_ray` rescaled **floor** (partial) or **half-up** (full, pool still pays the floor claim) minus the fee | `max(1, floor(fee_ray / 10^(27−d)))` when `fee_ray > 0`, capped at the whole units the pool pays above the principal (`floor(paid_ray − base_ray)`, 0 when the payout does not exceed it); withheld from the transfer |
| `SeizeMode::Credit(account_id)` | shares: `seized_scaled − fee_scaled` credited to the receiver account | `fee_scaled = ceil(bonus_scaled × fees_bps / BPS)` where `seized_scaled = floor(capped_ray × RAY / supply_index)` (exact held shares on full close), `bonus_scaled = min(floor(bonus_ray × RAY / supply_index), seized_scaled)` |

Under-delivery (measured repayment USD below plan) floors every seizure field by `received / planned`; credit fees are recomputed from the scaled bonus. Planning drops legs that round to zero tokens or zero shares.

Example continued (repay 533.6179 USD, XLM at $0.25, supply index 1 RAY, fees 1200 bps):

```text
total_seizure_usd = 624_332_977_588_046_958_378 WAD ($624.33) → 2_497.331910352187833512 XLM (RAY 2_497_331_910_352_187_833_512_000_000_000)
base_ray  = 2_134_471_718_249_733_191_035_897_435_897;  bonus_ray = 362_860_192_102_454_642_476_102_564_103
fee_ray   = 43_543_223_052_294_557_097_132_307_692
Transfer: seized 24_973_319_103 stroops (2,497.3319103 XLM), fee 435_432_230 stroops (43.543223 XLM), liquidator receives the difference
Credit:   seized shares 2_497_331_910_352_187_833_512_000_000_000, fee shares 43_543_223_052_294_557_097_132_307_693, liquidator shares 2_453_788_687_299_893_276_414_867_692_307
```

## Bad-debt socialization

Eligibility (`is_socializable_bad_debt`): `total_debt > total_collateral` and `total_collateral ≤ 5 WAD` for permissionless `clean_bad_debt` and the check after `liquidate`; owner-only `force_socialize_bad_debt` drops the collateral cap. `contracts/pool/src/interest.rs::apply_bad_debt_to_supply_index` then lowers only the affected market's supply index ([formulas.md#bad-debt](../../docs/reference/formulas.md#bad-debt)). Example: 2,000,000 USDC of shares at index 1.083 (`total_supply_ray = 2_166_000e27`), bad debt 30,000 USDC:

```text
reduction = 986_149_584_487_534_626_038_781_163   // 0.98615
index 1.083e27 → 1_067_999_999_999_999_999_999_999_999   // ≈ 1.068
```

## Liquidation buffer

Every debt mint (`borrow` and strategy openings, gross of any fee) keeps 200 bps of the floored supplied token value in cash (INV-ACCT-07, `contracts/pool/src/guards.rs::require_liquidation_buffer`, `InsufficientLiquidity`; the backing-shortfall gate on supply entry is [formulas.md#backing-and-cash-constraints](../../docs/reference/formulas.md#backing-and-cash-constraints)):

```text
reserved    = ceil(floor_supply_tokens × 200 / BPS)
cash − draw ≥ reserved
```

Example: supplied claim 9,999,999.9999999 USDC (floor), cash 3,500,000 USDC → `reserved = 200_000 USDC`; the largest single borrow draw is `3_300_000 USDC` (`33_000_000_000_000` base units) before `max_utilization` is considered.

## Caps in the scaled domain

`SpokeAssetConfig.supply_cap` / `borrow_cap` are token base units per spoke and always enforced (`0` = closed side; no unlimited sentinel; `i128::MAX` rejected at config time). Entry (`contracts/controller/src/spoke_usage.rs::enforce_spoke_cap`) compares shares:

```text
cap_scaled     = floor_saturating(cap × 10^(27−d) × RAY / index)      // calculate_scaled_cap, supply or borrow index
usage_scaled   = SpokeUsageRaw.supplied_scaled_ray | borrowed_scaled_ray   // get_spoke_usage
usage + new_shares ≤ cap_scaled   else SpokeSupplyCapReached | SpokeBorrowCapReached
headroom_tokens = floor(floor((cap_scaled − usage) × index / RAY) / 10^(27−d))
```

Exits subtract usage without checking caps. Cap conversion saturates at `i128::MAX`; the admitted cap maximum is `i128::MAX / 10^(27−d)` (`max_cap_for_decimals`).

Example: USDC `supply_cap = 50_000_000_000_000` (5,000,000 USDC), usage 4,000,000 USDC of shares, index 1.083:

```text
cap_scaled = 4_616_805_170_821_791_320_406_278_855_032_317
usage      = 3_693_444_136_657_433_056_325_023_084_025_854
headroom   = 923_361_034_164_358_264_081_255_771_006_463 shares → 9_999_999_999_999 base units (999,999.9999999 USDC)
```

## Flash-loan and strategy fees

`Bps::flash_loan_fee_on` (`common/src/math/fp.rs`), used by `contracts/pool/src/ops/flash.rs` and `ops/strategy.rs`:

```text
fee = half_up(principal × fee_bps / BPS);  if fee_bps > 0 and fee == 0: fee = 1
```

`flashloan_fee ≤ MAX_FLASHLOAN_FEE_BPS = 500`. Strategies charge it only when `charge_fee` is set (`multiply`, `swap_debt`) and revert with `StrategyFeeExceeds` if `fee > principal`; `flash_position` and `migrate_from_blend` borrow fee-free. Example at 9 bps: 1,000 USDC → `9_000_000` base units (0.9 USDC); 100 base units → 1 base unit.

## Numeric limits

Arithmetic bounds (decimals, index ceiling and floor, rate cap, token→RAY input maximum, overflow behaviour): [formulas.md#numeric-limits](../../docs/reference/formulas.md#numeric-limits). Controller limits:

| Bound | Value | Consequence |
|---|---|---|
| View input batches | `MAX_VIEW_INPUTS = 256` | `get_market_indexes_detailed` and `get_liquidation_estimate` revert above it (`InvalidPayments`) |
| Positions per side | `POSITION_LIMIT_MAX = 5` upper bound on the configured limit | `PositionLimitExceeded` |

## Which view rounds how

| View | Contract | Rounding / unit |
|---|---|---|
| `get_collateral_amount`, `get_borrow_amount` | controller | half-up both steps, base units, projected index |
| `get_supplied_amount`, `get_borrowed_amount`, `get_revenue` | pool | half-up (revenue: floor), base units, stored index |
| `get_reserves` | pool | raw `cash`, no rounding |
| `get_health_factor`, `get_liquidation_collateral`, `get_ltv_collateral_usd` | controller | floor-valued collateral, ceil debt, WAD; `get_ltv_collateral_usd` refreshes LTV stamps first |
| `is_liquidatable` | controller | bool: debt present and HF < WAD |
| `get_total_collateral_usd`, `get_total_borrow_usd` | controller | half-up, WAD (display; not the ceiled risk debt) |
| `get_borrow_rate`, `get_deposit_rate` | pool | annual RAY fraction, stored index |
| `get_utilisation` | pool | RAY, stored index |
| `get_market_index`, `get_bulk_indexes`, `get_market_indexes_detailed` | controller / pool | RAY indexes projected to now |
| `get_liquidation_estimate` | controller | `max_payment_wad` WAD, `bonus_rate_bps` BPS, seizure in base units (`Transfer`) or RAY shares (`Credit`) |
| `get_min_borrow_collateral_usd` | controller | WAD |
| `get_spoke_usage` | controller | RAY shares |
