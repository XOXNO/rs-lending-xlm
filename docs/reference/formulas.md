# Formulas — risk, health factor, liquidation math (code-matched)

Every formula below matches the code expression it cites. Anchors are
`path::symbol`. Notation (all primitives in `common/src/math/fp_core.rs`):

- `half_up(x*y/d)` = `(x*y + d/2) / d` in 256-bit (`mul_div_half_up`; requires
  `x >= 0`, `y >= 0`, `d > 0`; `MathOverflow` if the result exceeds `i128`);
- `floor(x*y/d)` = truncating 256-bit division (`mul_div_floor`);
- `ceil(x*y/d)` = quotient plus one on non-zero remainder (`mul_div_ceil`);
- `floor_sat` = `mul_div_floor` returning `i128::MAX` instead of panicking on
  overflow (`mul_div_floor_saturating`);
- decimal downscales use half-up / floor / ceil variants (`rescale_half_up`,
  `rescale_floor`, `rescale_ceil`); upscaling is an exact multiply.

## 1. Units

| Quantity | Scale | Anchor |
|---|---|---|
| Token amounts | native asset decimals, `i128`; `asset_decimals <= 18` enforced at market creation | `common/src/types/pool.rs::MarketParamsRaw::verify` |
| Interest indexes, rates, scaled shares, utilization | RAY = `1e27` | `common/src/constants/shared.rs::RAY` |
| USD values, prices, health factor | WAD = `1e18` | `common/src/constants/shared.rs::WAD` |
| Risk ratios (LTV, thresholds, bonuses, fees, reserve factor) | BPS, `10_000` = 100% | `common/src/constants/shared.rs::BPS` |
| Rates | RAY **per millisecond** | `common/src/rates/curve.rs::calculate_borrow_rate` |
| Year length | `MILLISECONDS_PER_YEAR = 31_556_926_000` | `common/src/constants/shared.rs::MILLISECONDS_PER_YEAR` |
| Pool clock | `now_ms = ledger.timestamp() * 1_000` (`MS_PER_SECOND`) | `contracts/pool/src/time.rs::now_ms` |

`Ray::from_asset(amount, decimals)` rescales asset base units to 27 decimals
(exact upscale for `decimals <= 27`), so one whole token is exactly `1 RAY`
regardless of token decimals (`common/src/math/fp.rs::Ray::from_asset`).
`Bps::to_wad` is exact — `WAD/BPS = 1e14` is an integer factor
(`common/src/math/fp.rs::Bps::to_wad`).

## 2. Scaled-balance model

Positions store scaled shares; the actual value in RAY terms is
`actual_ray = half_up(scaled * index / RAY)`
(`common/src/rates/scaling.rs::scaled_to_original`). Share mint/burn, all
starting from `amount_ray = Ray::from_asset(amount, decimals)`
(`common/src/rates/scaling.rs`):

| Operation | Formula | Rounds | Anchor |
|---|---|---|---|
| Supply mint (deposit) | `amount_ray * RAY / supply_index` | floor | `calculate_scaled_supply` |
| Supply burn (partial withdraw) | `amount_ray * RAY / supply_index` | ceil | `calculate_scaled_supply_ceil` |
| Debt mint (borrow) | `amount_ray * RAY / borrow_index` | ceil | `calculate_scaled_borrow` |
| Debt burn (partial repay) | `amount_ray * RAY / borrow_index` | floor | `calculate_scaled_borrow_floor` |

Every direction favors the pool. Readouts (`common/src/rates/scaling.rs`):
`unscale_supply` / `unscale_borrow` are half-up at both steps (share-to-RAY
multiply, then RAY-to-asset rescale); `unscale_supply_floor` is floor at both
steps; `unscale_borrow_ceil` is ceil at both steps; `unscale_borrow_ceil_ray`
stops at the RAY multiply (ceil).

Full-close semantics:

- `resolve_withdrawal`: `amount >= unscale_supply(pos_scaled)` (half-up) is a
  full close — burn all shares, pay `unscale_supply_floor(pos_scaled)` (at most
  one base unit below the half-up readout). Otherwise burn
  `calculate_scaled_supply_ceil(amount)` and pay exactly `amount`
  (`common/src/rates/scaling.rs::resolve_withdrawal`).
- `resolve_repay`: `amount >= unscale_borrow_ceil(pos_scaled)` is a full close —
  burn all debt shares, refund `amount - debt_ceil` as overpayment. Otherwise
  burn `calculate_scaled_borrow_floor(amount)` with zero overpayment
  (`common/src/rates/scaling.rs::resolve_repay`).
- The controller rewrites `withdraw` with `amount == 0` to
  `WITHDRAW_ALL_SENTINEL = i128::MAX`, which `resolve_withdrawal` turns into a
  full close (`contracts/controller/src/constants.rs::WITHDRAW_ALL_SENTINEL`,
  `contracts/controller/src/positions/withdraw.rs::resolve_withdraw_amount`).

Zero-share rejections — any value movement whose share delta rounds to zero
reverts rather than moving unbacked value:
supply asserts `amount == 0 || minted > 0` (`SupplyRoundsToZeroShares`,
`contracts/pool/src/ops/supply.rs::apply`); borrow asserts `minted > 0`
(`BorrowRoundsToZeroShares`, `contracts/pool/src/ops/borrow.rs::mint_debt`);
withdraw asserts `gross_amount == 0 || burned > 0`
(`WithdrawRoundsToZeroShares`, `contracts/pool/src/ops/withdraw.rs::resolve_close_or_partial`);
repay asserts `net_repay == 0 || burned > 0` (`RepayRoundsToZeroShares`,
`contracts/pool/src/ops/repay.rs::accounting`); net settle asserts
`gross == 0 || (burned_supply > 0 && burned_debt > 0)`
(`NetSettleRoundsToZeroShares`, `contracts/pool/src/ops/net_settle.rs::apply`).

The controller amount views use the half-up readouts, not the directional ones
(`contracts/controller/src/views/mod.rs::collateral_amount_for_hub_asset`,
`::borrow_amount_for_hub_asset`).

## 3. Interest-rate model

Utilization (RAY, on unscaled values):

    utilization = half_up(actual_borrowed * RAY / actual_supplied)   (0 when supplied == 0)

where each `actual_*` is `half_up(scaled * index / RAY)` at the side's own
index (`common/src/rates/curve.rs::utilization`,
`contracts/pool/src/cache/scale.rs::calculate_utilization`).

`calculate_borrow_rate` first clamps `u = min(utilization, 1 RAY)`, then picks
one of three regions (`common/src/rates/curve.rs::calculate_borrow_rate`; all
multiplies and divides half-up):

    u <  mid:               annual = base_borrow_rate + u * slope1 / mid
    mid <= u < optimal:     annual = base_borrow_rate + slope1 + (u - mid) * slope2 / (optimal - mid)
    u >= optimal:           annual = base_borrow_rate + slope1 + slope2 + (u - optimal) * slope3 / (RAY - optimal)

`slope1/slope2/slope3` are **cumulative region heights, not marginal slopes**:
each region starts from the full height of the regions below it and adds its own
slope scaled by progress through the region, so the curve is continuous at both
breakpoints. The annual rate is capped and converted to per-millisecond
(`div_by_int` half-up, same function):

    rate_per_ms = half_up(min(annual, max_borrow_rate) / MILLISECONDS_PER_YEAR)

Model validation (`common/src/types/pool.rs::InterestRateModel::verify`):
`base_borrow_rate >= 0`; `base <= slope1 <= slope2 <= slope3 <= max_borrow_rate`;
`max_borrow_rate > base`; `max_borrow_rate <= MAX_BORROW_RATE_RAY = 2 RAY`
(`common/src/constants/pool.rs::MAX_BORROW_RATE_RAY`); `0 < mid < optimal < RAY`;
`optimal <= max_utilization <= RAY`; `reserve_factor < BPS`;
`flashloan_fee <= MAX_FLASHLOAN_FEE_BPS = 500`.

Deposit-rate view (`common/src/rates/curve.rs::calculate_deposit_rate`):

    deposit_rate = half_up((BPS - reserve_factor) * half_up(u * borrow_rate / RAY) / BPS)

returning zero when `u == 0` or `reserve_factor` is outside `0..BPS`. This is
**view-only**: its sole caller is `contracts/pool/src/views.rs::deposit_rate`;
the accrual path never uses it. Realized supplier yield also differs from it by
the rounding shortfall booked to revenue (section 4).

## 4. Accrual

`global_sync` is a no-op when `elapsed_ms == 0`; otherwise it accrues in chunks
of `min(remaining, MAX_COMPOUND_DELTA_MS)` with
`MAX_COMPOUND_DELTA_MS == MILLISECONDS_PER_YEAR`, then stamps
`last_timestamp = now_ms` (`contracts/pool/src/interest.rs::global_sync`,
`common/src/rates/compound.rs::MAX_COMPOUND_DELTA_MS`). Each chunk recomputes
utilization from the updated state, so a long-stale market tracks rate drift
(`contracts/pool/src/interest.rs::accrue_chunk`).

Compounding factor (`common/src/rates/compound.rs::compound_interest`), with
`x = rate_raw * delta_ms` (256-bit product narrowed to `i128`, `MathOverflow`
on failure; `delta_ms == 0` returns `1 RAY`):

    factor = 1 + x + x^2/2 + x^3/6 + x^4/24 + x^5/120 + x^6/720 + x^7/5040 + x^8/40320

unrolled with no data-dependent branch; powers use half-up RAY multiply,
factorial divides are half-up. Every omitted tail term is positive, so the
cutoff error is one-directional: the factor under-estimates the true
exponential — the series never over-accrues interest.

Per chunk (`contracts/pool/src/interest.rs::accrue_chunk`):

1. Borrow index: `new_bi = half_up(old_bi * factor / RAY)`, clamped to
   `MAX_BORROW_INDEX_RAY = 1e36`; monotone non-decreasing
   (`common/src/rates/index.rs::update_borrow_index`,
   `common/src/constants/pool.rs::MAX_BORROW_INDEX_RAY`).
2. Supplier-reward split
   (`common/src/rates/index.rs::calculate_supplier_rewards`):

       accrued        = half_up(borrowed * new_bi / RAY) - half_up(borrowed * old_bi / RAY)
       protocol_fee   = half_up(accrued * reserve_factor / BPS)
       supplier_rewards = accrued - protocol_fee

3. Supply index (`common/src/rates/index.rs::update_supply_index`): unchanged
   when `supplied == 0`, `rewards == 0`, or `half_up(supplied*old_si/RAY) == 0`;
   otherwise

       grown  = floor_sat((half_up(supplied*old_si/RAY) + rewards) * RAY / supplied)
       new_si = max(min(grown, MAX_SUPPLY_INDEX_RAY), min(old_si, MAX_SUPPLY_INDEX_RAY))

   with `MAX_SUPPLY_INDEX_RAY = 1e36`.
4. Shortfall — rewards that index rounding kept from suppliers
   (`common/src/rates/index.rs::supply_index_reward_shortfall`):

       shortfall = rewards - (half_up(supplied*new_si/RAY) - half_up(supplied*old_si/RAY))

   computed with `checked_sub`, so the step reverts if the index ever
   distributes more than the reward pot.
5. `protocol_fee + shortfall` is booked as revenue shares:
   `shares = floor_sat(fee * RAY / supply_index)`, clamped to
   `i128::MAX - supplied` headroom
   (`common/src/rates/index.rs::protocol_fee_shares`), then `revenue += shares`
   **and** `supplied += shares`
   (`contracts/pool/src/cache/shares.rs::accrue_revenue`). No accrued value is
   destroyed: what the supply index cannot express becomes revenue.

A fresh market starts at `borrow_index = supply_index = RAY` with
`last_timestamp = now_ms` (`contracts/pool/src/ops/market.rs::create`).
`simulate_update_indexes` mirrors the same chunk loop off-storage for views and
controller prefetch, including re-supplying fee shares inside the loop, which
moves utilization between chunks (`common/src/rates/simulate.rs::simulate_update_indexes_body`).

## 5. Position valuation and health factor

Three valuation variants, each directional at all three steps
(`contracts/controller/src/risk/totals.rs`):

    actual_ray           = half_up(scaled * index / RAY)
    position_value       = half_up(rescale_half_up(actual_ray, 27 -> 18) * price / WAD)
    position_value_floor = same chain, floor at every step
    position_value_ceil  = same chain, ceil at every step

Risk totals (`contracts/controller/src/risk/totals.rs::calculate_account_risk_totals_body`),
per supply leg computing both a half-up `value` and a floor `gate_value`:

    total_collateral    = sum of value_i                                    (half-up)
    ltv_collateral      = sum of floor(gate_value_i * ltv_i / BPS)          (floor)
    weighted_collateral = sum of floor(gate_value_i * threshold_i / BPS)    (floor)
    total_debt          = sum of position_value_ceil over borrow legs       (ceil)

The bps application is `Bps::apply_to_wad_floor`: exact bps-to-WAD conversion,
then floor multiply (`common/src/math/fp.rs::Bps::apply_to_wad_floor`,
`contracts/controller/src/risk/totals.rs::weighted_collateral`).

    health_factor = floor_sat(weighted_collateral * WAD / total_debt)
    health_factor = i128::MAX (as WAD)  when total_debt == 0

(`common/src/math/fp.rs::Wad::div_floor_saturating`). Saturation means
dust-debt accounts whose ratio overflows also read `i128::MAX`. The
`get_health_factor` view returns `i128::MAX` for missing and debt-free accounts
without computing totals; `can_be_liquidated` is exactly `health_factor < WAD`
(`contracts/controller/src/views/mod.rs::health_factor`, `::can_be_liquidated`).

The USD views are half-up, not directional: `sum_debt_usd` (behind
`get_total_borrow_in_usd`) uses `position_value`, so the view can read below
the ceil debt the risk gate enforces against; `sum_supply_usd` likewise
(`contracts/controller/src/risk/totals.rs::sum_debt_usd`, `::sum_supply_usd`).
Liquidation repay legs are valued
`usd = half_up(Wad::from_token(amount, decimals) * price / WAD)`
(`common/src/types/oracle.rs::PriceFeed::usd_value_wad`).

## 6. Risk gates

After any health-reducing verb (borrow = debt entry, withdraw = supply exit)
the controller restamps live LTV on listed supply legs and runs the post-pool
gate (`contracts/controller/src/positions/mod.rs::enforce_post_pool_solvency`).
The gate short-circuits for debt-free accounts; otherwise it checks, in order
(`contracts/controller/src/risk/validation.rs::require_post_pool_risk_gates`):

1. `ltv_collateral >= total_debt` else `InsufficientCollateral`;
2. `health_factor >= 1 WAD` else `InsufficientCollateral`;
3. if the floor is non-zero: `ltv_collateral >= min_borrow_collateral_usd_wad`
   else `MinBorrowCollateralNotMet`. The floor defaults to
   `DEFAULT_MIN_BORROW_COLLATERAL_USD_WAD = 5 WAD`; a stored `0` disables it
   (`contracts/controller/src/storage/protocol.rs::get_min_borrow_collateral_usd_wad`,
   `common/src/constants/shared.rs::DEFAULT_MIN_BORROW_COLLATERAL_USD_WAD`).

Risk-bound validation (`common/src/validation.rs::validate_risk_bounds`):

    threshold > ltv
    threshold <= BPS
    threshold * (BPS + bonus) <= BPS * BPS

The last inequality guarantees post-bonus seizure at the liquidation threshold
never exceeds 100% of collateral. Liquidation fees satisfy `fees <= BPS`
(`common/src/validation.rs::validate_liquidation_fees`). The per-spoke curve is
validated as `WAD < target_hf <= 10 WAD`, `0 < hf_for_max_bonus < target_hf`,
`bonus_factor <= BPS` (`common/src/validation.rs::validate_liquidation_curve`).

## 7. Liquidation

Preconditions: non-empty borrow map and `health_factor < 1 WAD`, else
`HealthFactorTooHigh` (`contracts/controller/src/positions/liquidation/plan.rs::build_liquidation_plan`).

Proportion seized per unit of debt repaid
(`contracts/controller/src/positions/liquidation/math.rs::calculate_seizure_proportions`):

    proportion_seized = half_up(weighted_collateral * WAD / total_collateral)   (0 when total_collateral == 0)

Bonus ceiling (`contracts/controller/src/positions/liquidation/curve.rs::max_bonus_for_threshold`):

    eff_thr_bps = clamp(ceil(proportion_seized * BPS / WAD), 1, BPS)
    max_bonus   = floor(BPS * (BPS - eff_thr_bps) / eff_thr_bps)

— the largest bonus with `(1 + bonus) * effective_threshold <= 1`. The base
bonus is the collateral-value-weighted average of per-position bonuses, capped:
`base = min(sum of half_up(value_i/total_collateral) * bonus_i, max_bonus)`
(`contracts/controller/src/positions/liquidation/math.rs::get_account_bonus_params`).

Per-spoke curve (`contracts/controller/src/positions/liquidation/curve.rs::LiquidationCurve::from_config`;
defaults `target_hf = 1.10 WAD`, `hf_for_max_bonus = 0.80 WAD`,
`bonus_factor = 10_000` — `contracts/controller/src/constants.rs::DEFAULT_LIQUIDATION_TARGET_HF_WAD`):

    bonus_scale = 1                                                  when target_hf <= hf_for_max_bonus
                = min(half_up((target_hf - hf) / (target_hf - hf_for_max_bonus)), 1)   otherwise

    bonus = base                                                     when hf >= target_hf
          = base + half_up(bonus_factor * half_up((max - base) * bonus_scale / WAD) / BPS)   otherwise

with the `bonus_factor` multiply skipped when `bonus_factor == BPS`
(`::LiquidationCurve::bonus_scale`, `::calculate_linear_bonus_with_target`).

HF-preserving cap (`::max_hf_preserving_bonus_bps`), truncating integer math:

    cap = hf_raw * BPS / proportion_seized_raw - BPS      (None when proportion <= 0 or hf >= WAD)

Bonus selection (`::estimate_liquidation_amount`): no cap -> curve bonus;
curve bonus `<= cap` -> curve bonus; `cap >= base` -> cap; otherwise the plan
is a mandatory full close `(total_debt, base)`.

Ideal close amount (`::try_liquidation_at_target`; every multiply/divide
half-up):

    d_max      = half_up(total_collateral / (1 + bonus))
    denom_term = half_up(proportion_seized * (1 + bonus))
    None                       when target_hf <= denom_term
    min(d_max, total_debt)     when half_up(target_hf * total_debt) <= weighted_collateral
    d_ideal = half_up((target_hf * total_debt - weighted_collateral) / (target_hf - denom_term))
    result  = min(d_ideal, d_max, total_debt)

When `try_liquidation_at_target` returns `None`, the fallback is
`ideal = min(half_up(total_collateral / (1 + bonus)), total_debt)`. Dust
promotion: if `0 < total_debt - ideal < BAD_DEBT_USD_THRESHOLD = 5 WAD`, the
plan becomes a full close at the same bonus
(`::estimate_liquidation_amount`, `contracts/controller/src/constants.rs::BAD_DEBT_USD_THRESHOLD`).

Repayment normalization
(`contracts/controller/src/positions/liquidation/math.rs`):

- each repay leg is capped at `debt_close_amount = unscale_borrow_ceil(scaled)`;
  the excess goes to `refunds` (`::calculate_repayment_amounts`);
- `max_debt_to_repay_usd = min(total_payment_usd, ideal)`; surplus is trimmed
  off the last legs backwards — whole legs refunded until the residue fits,
  then the final leg splits by `ratio = floor(remaining_excess / leg_usd)` with
  floor token conversion (`::process_excess_payment`);
- under-payment reverts `FullCloseRequired` when an HF-preserving cap exists
  with `0 <= cap < base` and the ceil-valued payment
  (`ceil(from_token(amount) * price / WAD)`) is still below `ideal`
  (`::normalize_repayment_plan`, `::sum_repaid_usd_ceil`).

Seizure split (`::calculate_seized_collateral`; half-up unless stated):

    one_plus_bonus    = 1 WAD + bonus_to_wad
    total_seizure_usd = half_up(repay_usd * one_plus_bonus / WAD)
    per leg: share       = half_up(asset_value / total_collateral)
             seizure_usd = half_up(total_seizure_usd * share / WAD)
             seizure_ray = to_ray(half_up(seizure_usd / price))
             capped_ray  = min(seizure_ray, half_up(scaled * supply_index / RAY))
    base_ray         = floor(capped_ray * RAY / one_plus_bonus_ray)
    bonus_ray        = capped_ray - base_ray
    protocol_fee_ray = half_up(bonus_ray * liquidation_fees / BPS)

The protocol fee is charged **on the bonus portion only**. Token conversion of
`capped_ray` is half-up on a full-position close (`capped_ray == actual_ray`)
and floor otherwise; legs resolving to `<= 0` are skipped.

Fee chain (same function): `fee_asset = floor(protocol_fee_ray -> asset)`,
bumped to `1` when a positive RAY fee rounds to zero, then
`protocol_fee = min(bumped_fee, pool_gross)` where `pool_gross` is what
`resolve_withdrawal(capped_amount)` actually pays. The pool withholds the fee
from the liquidator's payout, requiring `gross >= protocol_fee`
(`WithdrawLessThanFee`); liquidation withdrawals skip the max-utilization guard
but keep the solvency and reserve guards
(`contracts/pool/src/ops/withdraw.rs::withhold_liquidation_fee`, `::gate_and_debit`).

Under-delivery (fee-on-transfer defence): a repay leg delivering less than sent
is valued `leg_usd = floor(entry.usd_wad * received / entry.amount)`
(`contracts/controller/src/positions/liquidation/apply.rs::apply_liquidation_repayments`);
when `received_usd < planned_usd` every seizure entry shrinks by
`floor(x * received_usd / planned_usd)` on both `amount` and `protocol_fee`, so
residue stays with the liquidated account
(`contracts/controller/src/positions/liquidation/math.rs::scale_seizures_to_received`).

## 8. Bad-debt socialization

Two gates (`contracts/controller/src/positions/liquidation/mod.rs::BadDebtGate::admits`):

- `DustCapped` (permissionless `clean_bad_debt`, and the post-liquidation
  re-check): `total_debt > total_collateral && total_collateral <=
  BAD_DEBT_USD_THRESHOLD (5 WAD)`
  (`contracts/controller/src/positions/liquidation/curve.rs::is_socializable_bad_debt`);
- `Insolvent` (`force_socialize_bad_debt`): only `total_debt > total_collateral`,
  no dust cap.

On the borrow side the pool converts the seized position to
`bad_debt = ceil(scaled * borrow_index / RAY)` (`unscale_borrow_ceil_ray`),
writes down the supply index, then burns the debt shares; on the deposit side
it calls `absorb_supply_as_revenue`, reassigning the shares to the protocol
with `supplied` unchanged (`contracts/pool/src/ops/seize.rs::apply`).

Write-down (`contracts/pool/src/interest.rs::apply_bad_debt_to_supply_index`):

    total_value      = half_up(supplied * supply_index / RAY)      (no-op when 0)
    capped           = min(bad_debt, total_value)
    remaining        = total_value - capped
    reduction_factor = floor(remaining * RAY / total_value)
    new_index        = floor(supply_index * reduction_factor / RAY)
    supply_index     = max(new_index, SUPPLY_INDEX_FLOOR_RAW)

with `SUPPLY_INDEX_FLOOR_RAW = RAY / 1_000 = 1e24`
(`common/src/constants/pool.rs::SUPPLY_INDEX_FLOOR_RAW`): the supply index is
never written below one-thousandth of its initial value, so a total wipeout
leaves a residual claim at the floor instead of exact proportionality.
Consequence: the supply index is **not monotone** — anything caching an index
must tolerate a decrease. The borrow index only ever grows
(`common/src/rates/index.rs::update_borrow_index`).

After every liquidation, `check_bad_debt_after_liquidation` re-checks the
dust-capped gate on the post-liquidation totals and removes the account when it
is debt-free (`contracts/controller/src/positions/liquidation/apply.rs::check_bad_debt_after_liquidation`).

## 9. Caps

`supply_cap` / `borrow_cap` are stored per spoke-asset listing in **asset base
units** (`common/src/types/controller.rs::SpokeAssetConfig`). Enforcement
converts to scaled-share space at the live index
(`contracts/controller/src/spoke/caps.rs::cap_to_scaled`):

    cap_scaled = floor_sat(Ray::from_asset(cap, decimals) * RAY / index)

and an entry asserts `usage_scaled + delta_scaled <= cap_scaled`, raising
`SpokeSupplyCapReached` / `SpokeBorrowCapReached`
(`contracts/controller/src/spoke/caps.rs::enforce_spoke_cap`). Because the cap
is converted at the current index, it bounds the current asset-unit balance
including accrued interest; the floor makes the effective cap marginally
conservative.

- **Zero means zero.** There is no unlimited sentinel: `cap == 0` gives
  `cap_scaled == 0`, so any positive entry delta reverts.
- **Exits are uncapped.** `apply_exit` subtracts usage with no cap comparison
  and is a no-op when no usage row exists
  (`contracts/controller/src/spoke/caps.rs::SpokeUsageContext::apply_exit`).
  A zero cap is therefore a soft wind-down, not a freeze.
- Only entry legs enforce caps; the sign convention is
  `entry_delta = new_scaled - old_scaled`, `exit_delta = old_scaled - new_scaled`
  (`contracts/controller/src/positions/mod.rs::apply_leg_usage`).

Cap domain validation at listing time: `cap >= 0`
(`contracts/controller/src/config/asset.rs::validate_spoke_asset_args`) and
`cap <= max_cap_for_decimals(decimals) = i128::MAX / 10^(27 - decimals)`, with
`decimals > 27` rejected as `AssetDecimalsTooHigh`
(`common/src/validation.rs::require_cap_within_asset_domain`,
`::max_cap_for_decimals`) — so the RAY form of any accepted cap fits `i128`.

## 10. Fees

Flash-loan fee (`common/src/math/fp.rs::Bps::flash_loan_fee_on`):

    fee = 0                                    when bps == 0
    fee = max(1, half_up(amount * bps / BPS))  when bps > 0

— a positive fee rate never rounds to a free loan. Terms
(`contracts/pool/src/ops/flash.rs::terms`): `total_repayment = amount + fee`,
with exact pool-balance assertions after payout (`pre - amount`) and after
repayment (`pre + fee`). The fee is credited to cash and booked as revenue
shares (`contracts/pool/src/ops/flash.rs::book_fee`); rate bound
`flashloan_fee <= MAX_FLASHLOAN_FEE_BPS = 500`.

Strategy fee: the same `flash_loan_fee_on` on the same `flashloan_fee` bps
(min-1-unit rule included), asserted `fee <= amount` (`StrategyFeeExceeds`);
the receiver gets `amount - fee` and the fee is booked as revenue shares
(`contracts/pool/src/ops/strategy.rs::compute_fee`, `::accounting`).

Liquidation protocol fee: section 7 — `half_up(bonus_ray * fees / BPS)` on the
bonus portion only, floored to asset units, bumped to a 1-unit minimum, capped
at the pool's gross payout. Reserve factor: per accrual chunk,
`half_up(accrued * reserve_factor / BPS)` plus the supply-index shortfall
(section 4); validated `reserve_factor < BPS`.

Revenue shares: `accrue_revenue` **mints** (`revenue += s` and `supplied += s`;
interest fees, flash/strategy fees, withheld liquidation fees) while
`absorb_supply_as_revenue` only **reassigns** existing shares (`revenue += s`,
`supplied` unchanged; seized deposits); `revenue <= supplied` is asserted after
every burn or absorb (`contracts/pool/src/cache/shares.rs`). Claiming pays
`min(cash, unscale_supply_floor(revenue))`; a partial claim burns
`ceil(revenue * amount / treasury_actual)` shares — more than proportional
(`contracts/pool/src/cache/shares.rs::burn_claimable_revenue`).

## 11. Rounding direction table

| Operation | Rounds | Favors | Anchor |
|---|---|---|---|
| Supply mint shares | floor | pool over depositor | `common/src/rates/scaling.rs::calculate_scaled_supply` |
| Supply burn shares (partial withdraw) | ceil | pool over withdrawer | `common/src/rates/scaling.rs::calculate_scaled_supply_ceil` |
| Debt mint shares (borrow) | ceil | pool over borrower | `common/src/rates/scaling.rs::calculate_scaled_borrow` |
| Debt burn shares (partial repay) | floor | pool over repayer | `common/src/rates/scaling.rs::calculate_scaled_borrow_floor` |
| Full-close withdrawal payout | floor | pool over withdrawer | `common/src/rates/scaling.rs::resolve_withdrawal` |
| Full-close debt (repay target) | ceil | pool over repayer | `common/src/rates/scaling.rs::resolve_repay` |
| Compounding series cutoff | under-estimate of exp | borrowers (never over-accrues) | `common/src/rates/compound.rs::compound_interest` |
| Supply-index growth | floor (+ shortfall to revenue) | protocol over suppliers | `common/src/rates/index.rs::update_supply_index` |
| Protocol fee shares | floor | suppliers over treasury | `common/src/rates/index.rs::protocol_fee_shares` |
| Backing shortfall: supplied claim floor, debt ceil | asymmetric | declares solvency conservatively | `contracts/pool/src/guards.rs::backing_shortfall` |
| Collateral `gate_value` (LTV/threshold sums) | floor | protocol over account | `contracts/controller/src/risk/totals.rs::calculate_account_risk_totals_body` |
| LTV / threshold bps application | floor | protocol over account | `contracts/controller/src/risk/totals.rs::weighted_collateral` |
| Risk-gate debt total | ceil | protocol over account | `contracts/controller/src/risk/totals.rs::calculate_account_risk_totals_body` |
| Health factor division | floor (saturating) | protocol (HF reads lower) | `contracts/controller/src/risk/totals.rs::calculate_account_risk_totals_body` |
| USD views (`sum_debt_usd`, `sum_supply_usd`) | half-up | neutral (display) | `contracts/controller/src/risk/totals.rs::sum_debt_usd` |
| Bonus-ceiling effective threshold | ceil, then floor on bonus | account over liquidator | `contracts/controller/src/positions/liquidation/curve.rs::max_bonus_for_threshold` |
| HF-preserving bonus cap | truncating | account over liquidator | `contracts/controller/src/positions/liquidation/curve.rs::max_hf_preserving_bonus_bps` |
| Seizure base/bonus split | floor on base | protocol fee base (bonus larger) | `contracts/controller/src/positions/liquidation/math.rs::calculate_seized_collateral` |
| Partial-seizure token conversion | floor | account over liquidator | `contracts/controller/src/positions/liquidation/math.rs::calculate_seized_collateral` |
| Liquidation fee to asset units | floor, then min 1, then cap at gross | protocol (dust) / liquidator (cap) | `contracts/controller/src/positions/liquidation/math.rs::calculate_seized_collateral` |
| Excess-payment refund split | floor on refund | liquidated account | `contracts/controller/src/positions/liquidation/math.rs::process_excess_payment` |
| Under-delivery leg valuation and seizure scaling | floor | liquidated account | `contracts/controller/src/positions/liquidation/apply.rs::apply_liquidation_repayments`, `math.rs::scale_seizures_to_received` |
| Bad-debt write-down factor and index | floor, clamped to `1e24` floor | suppliers (index falls no further) | `contracts/pool/src/interest.rs::apply_bad_debt_to_supply_index` |
| Cap conversion to shares | floor (saturating) | protocol (cap slightly tighter) | `contracts/controller/src/spoke/caps.rs::cap_to_scaled` |
| Flash/strategy fee | half-up, min 1 unit | protocol over borrower | `common/src/math/fp.rs::Bps::flash_loan_fee_on` |
| Partial revenue claim share burn | ceil | pool over treasury | `contracts/pool/src/cache/shares.rs::burn_claimable_revenue` |
