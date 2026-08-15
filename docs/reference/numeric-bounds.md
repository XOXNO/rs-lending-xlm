# Numeric bounds

Where the arithmetic stops. Every number below is derived from a constant or a
guard that exists in the repository today; the derivation is shown so a change
to any of those constants can be re-run against it. This is the analogue of
ChainSecurity's note 8.5 (type bounds) and note 8.4 (small-position liquidation
profitability) from the Aave V4 audit corpus — see
`docs/explanation/aave-v4-audit-comparison.md`, classes L and H.

Evidence labels: **Observed** = read from source. **Verified** = reproduced by a
named test. **Inferred** = follows from the above, not directly reproduced.

## 1. The domain

| Quantity | Type | Scale | Source |
|---|---|---|---|
| Interest indexes, scaled shares, rates | `i128` | RAY = 10^27 | [`common/src/math/fp.rs`](../../common/src/math/fp.rs) |
| USD values, health factor | `i128` | WAD = 10^18 | same |
| Risk ratios, fees | `i128` | BPS = 10^4 | same |
| Token amounts | `i128` | asset decimals, 3..=18 | `MIN_ASSET_DECIMALS` / `MAX_ASSET_DECIMALS` |

`i128::MAX = 170_141_183_460_469_231_731_687_303_715_884_105_727`.

Every `x * y / d` widens to `I256` before dividing
([`fp_core.rs`](../../common/src/math/fp_core.rs)), so an intermediate product
never overflows; only the *result* has to fit `i128`, and it panics with
`GenericError::MathOverflow` (#33) when it does not. Decimal rescaling
(`rescale_half_up` and friends) is the exception: it multiplies in `i128` and
panics with a plain message on overflow.

Unlike Aave, we do not pack these into narrower integers. There is no `uint120`
`drawnIndex`, no `int200` `premiumOffset`, no `uint40` cap. What we have instead
are three *configured clamps* and one *validation guard*, and those — not the
integer width — are where the domain actually ends.

## 2. Interest index ceiling

**Observed.** Both indexes are created at `RAY` (1.0) in
[`contracts/pool/src/ops/market.rs`](../../contracts/pool/src/ops/market.rs) and
clamped at `MAX_BORROW_INDEX_RAY = MAX_SUPPLY_INDEX_RAY = 10^36`
([`common/src/constants/pool.rs`](../../common/src/constants/pool.rs)). That is a
**1e9x growth budget**. The clamps live in `update_borrow_index` and
`update_supply_index`
([`common/src/rates/index.rs`](../../common/src/rates/index.rs)).

That is the whole of INV-IDX-01 and INV-IDX-02's "configured maximum": those
invariants read as if each market carried its own index ceiling, but the bound is
a **single protocol-wide compile-time constant**, identical for every market and
for both indexes, and not settable by governance. Nothing else in the codebase
bounds an index. The invariant text should be read as "the constant maximum".

The growth rate is bounded twice: `MarketParamsRaw::validate` rejects
`max_borrow_rate > MAX_BORROW_RATE_RAY = 2 * RAY` (200% APR)
([`common/src/types/pool.rs`](../../common/src/types/pool.rs)), and
`calculate_annual_borrow_rate` caps the curve output at that parameter.
`global_sync` splits elapsed time into chunks of at most
`MAX_COMPOUND_DELTA_MS = MILLISECONDS_PER_YEAR`
([`contracts/pool/src/interest.rs`](../../contracts/pool/src/interest.rs)).

Chunking barely moves the answer. `compound_interest` is a Taylor series
truncated at the eighth term, so a single one-year chunk at 200% APR yields
`7.387301587…` where continuous compounding would give `e^2 = 7.389056…` — the
truncation *under*-shoots by 0.024%, which means many small chunks (the
liquidator- or keeper-chosen partition) grow the index marginally faster than one
big chunk. Either way:

```
years_to_ceiling = ln(1e9) / annual_rate = 20.7233 / r
```

**Verified** by `test_borrow_index_ceiling_is_eleven_years_away_at_the_protocol_rate_cap`
and `test_borrow_index_ceiling_years_at_configured_and_realistic_rates`
([`common/tests/rates/index.rs`](../../common/tests/rates/index.rs)), which run
the real integer path with maximum-size chunks:

| Annual borrow rate | Where it comes from | 1-year chunk factor | Years to the 1e9x ceiling |
|---|---|---|---|
| 200% | `MAX_BORROW_RATE_RAY`, the protocol maximum | 7.387301587 | **11** |
| 175% | XLM / SolvBTC / AQUA / LP markets, `configs/mainnet` | 5.754090434 | 12 |
| 125% | USDC / EURC / PYUSD / RWA markets, `configs/mainnet` | 3.490319534 | 17 |
| 30% | ChainSecurity's own worked example for Aave | 1.349858808 | 70 |
| 10% | plausible steady state | 1.105170918 | 208 |

The 30% row is the direct comparison: ChainSecurity gave ~70 years before Aave's
`uint120` `drawnIndex` overflows. Our ceiling at the same rate is also 70 years.
The similarity is a coincidence of two unrelated constants, but the practical
conclusion is the same — the ceiling is not reachable by a market that is
functioning, and is only reachable at all by a market pinned at ~100%
utilization and the maximum rate for over a decade.

**The failure mode differs, and ours is quieter.** Aave's is an overflow; ours is
a clamp. At `MAX_BORROW_INDEX_RAY` the borrow index simply stops moving:
`update_borrow_index` returns the ceiling, `calculate_supplier_rewards` then sees
zero accrued interest, and **debt stops growing with no error and no event**.
Suppliers stop earning. Nothing reverts. If a market ever approaches this, it has
to be detected off-chain.

**Headroom before the clamp.** `update_borrow_index` multiplies *before* it
clamps, so the real overflow site is `MAX_BORROW_INDEX_RAY x factor`. At the
worst case (ceiling index, one-year chunk, 200% APR) that product is
`7.3873e36`, which is `i128::MAX / 23`. **Verified** by
`test_borrow_index_at_the_ceiling_multiplies_without_overflow`.

## 3. Largest representable balance

**Observed.** Every token amount enters the accounting through
`Ray::from_asset(amount, decimals)`, which is
`rescale_half_up(amount, decimals, 27)` — a plain `i128` multiplication by
`10^(27 - decimals)`. That single multiplication is the ceiling on any balance
the protocol can hold, in a position or in a market total:

```
max_units(d) = i128::MAX / 10^(27 - d)
```

| Decimals | Upscale factor | Max asset units | Max whole tokens |
|---|---|---|---|
| 3 | 10^24 | `170_141_183_460_469` | ~170.14 billion |
| 5 | 10^22 | `17_014_118_346_046_923` | ~170.14 billion |
| 6 | 10^21 | `170_141_183_460_469_231` | ~170.14 billion |
| 7 | 10^20 | `1_701_411_834_604_692_317` | ~170.14 billion |
| 8 | 10^19 | `17_014_118_346_046_923_173` | ~170.14 billion |
| 9 | 10^18 | `170_141_183_460_469_231_731` | ~170.14 billion |
| 18 | 10^9 | `170_141_183_460_469_231_731_687_303_715` | ~170.14 billion |

**Verified** by `test_ray_from_asset_ceiling_holds_across_the_listable_decimal_range`,
`test_balance_ceiling_is_the_same_whole_token_count_at_every_decimals`, and the
two `test_ray_from_asset_one_unit_above_the_ceiling_overflows_at_*` cases
([`common/tests/math/fp.rs`](../../common/tests/math/fp.rs)).

The interesting property is the last column: **expressed in whole tokens the
ceiling does not depend on decimals at all.** It is always
`i128::MAX / RAY = 170_141_183_460.469…` tokens, because ray normalization
divides out the decimal count. This is our analogue of ChainSecurity's note that
Aave's `int200` `premiumOffset` leaves ~106 bits for balances and therefore
constrains high-supply tokens: a token whose *supply* exceeds ~170.14 billion
whole units — memecoin-scale, 1e14 supply — cannot have its whole supply
represented here either, at any decimal count. No listed market is within six
orders of magnitude of this. The tightest configured headroom is XLM's supply cap
of `5e14` units (50 million XLM at 7 decimals), still ~3,400x below the d=7
ceiling of `1.7014e18` units.

The bound applies to three distinct things, all at the same number:

1. a single deposit or borrow (`calculate_scaled_supply` / `calculate_scaled_borrow`);
2. a position's current value (`unscale_supply` / `unscale_borrow_ceil`, whose
   `scaled * index` product is a ray value);
3. the market total (`Cache::supplied().mul(supply_index)` in
   `update_supply_index` and `apply_bad_debt_to_supply_index`).

**Inferred:** (2) and (3) are where a *grown index* bites. Value is
`scaled * index`, and the scaled form shrinks as the index grows, so index growth
does not itself consume headroom — but the ray *value* is still bounded by
`i128::MAX`, so no market may hold more than ~170.14 billion whole tokens of
value however that value was accumulated.

## 4. Supply index floor

**Observed.** `SUPPLY_INDEX_FLOOR_RAW = RAY / 1_000` (0.001). It is applied in
exactly one place: the last line of `apply_bad_debt_to_supply_index`
([`contracts/pool/src/interest.rs`](../../contracts/pool/src/interest.rs)), which
is the only path that moves the supply index down.

It protects three conversions, all of which divide by the supply index:

- **Division by zero.** `calculate_scaled_supply` is
  `Ray::from_asset(amount, d).div_floor(supply_index)`. A fully socialized market
  would otherwise reach index 0 and every subsequent deposit would trap in the
  host's `I256` division rather than in a contract error.
- **Share inflation.** At the floor, a deposit mints 1,000x the shares it would
  at index 1.0. That is the *entire* inflation budget: no sequence of
  socializations can make a share cheaper than one thousandth of a token.
  **Verified** by `supply_index_floor_bounds_share_inflation_to_one_thousand_x`
  ([`common/tests/rates/scaling.rs`](../../common/tests/rates/scaling.rs)).
- **Deposit headroom.** The flip side of the same 1,000x: a market written down
  to the floor can only accept `max_units(d) / 1_000` in a single deposit before
  the scaled form overflows — ~170.14 *million* whole tokens instead of ~170.14
  billion. **Verified** by
  `supply_index_floor_costs_three_decades_of_deposit_headroom` and
  `supply_index_floor_makes_a_ceiling_deposit_overflow_its_scaled_form`.

Combined with §2, the supply index spans twelve decades: `[1e-3, 1e9]` in real
terms, `[1e24, 1e36]` raw. **Verified** by
`test_supply_index_shares_the_borrow_index_ceiling`.

Note that `calculate_scaled_cap` deliberately *saturates* rather than panicking
at this boundary, so a floored index makes cap checks fail open rather than
bricking an entry path — already covered by
`cap_at_domain_ceiling_saturates_under_a_floored_supply_index`.

## 5. Cap domain

**Observed.** `require_cap_within_asset_domain`
([`common/src/validation.rs`](../../common/src/validation.rs)) rejects
`asset_decimals > RAY_DECIMALS` with `CollateralError::AssetDecimalsTooHigh`
(#132) and `cap > max_cap_for_decimals(asset_decimals)` with
`CollateralError::InvalidBorrowParams` (#116), where

```rust
max_cap_for_decimals(d) = i128::MAX / 10^(27 - d)
```

This is **numerically identical to the balance ceiling in §3**, and that is the
point: a stored cap is converted with `Ray::from_asset` inside
`calculate_scaled_cap`, so admitting a cap the balance domain cannot represent
would panic on an entry path instead of rejecting a bad configuration.
**Verified** by `cap_ceiling_is_exactly_the_largest_representable_balance` and
`cap_ceiling_is_a_constant_whole_token_count`
([`common/tests/validation.rs`](../../common/tests/validation.rs)).

Two properties worth stating explicitly:

- The decimals check is load-bearing and not redundant with the comparison:
  `max_cap_for_decimals` returns `0` above `RAY_DECIMALS`, so without the
  explicit check every positive cap would be rejected with the wrong error.
  **Verified** by `cap_ceiling_collapses_to_zero_above_ray_decimals`.
- Compared with Aave's `uint40` caps (~1.1e12 whole units), ours are ~170 billion
  whole tokens — two orders of magnitude larger, and uniform in decimals where
  Aave's is uniform in raw units.

## 6. Small-position liquidation profitability

ChainSecurity Mar-2026 note 8.4 gives the closed form for the position value
below which a liquidation loses money to rounding:

```
V < L_round / (b * (1 - f))
```

`L_round` is the summed rounding loss across both legs, `b` the liquidation bonus
and `f` the protocol's cut of it. They worked it to ~4.4 cents for a
WBTC-debt/ETH-collateral pair on Aave, ~6.7 cents after a later rounding change,
from a count of **2 debt-leg sites plus 2 collateral-leg sites**.

### 6.1 Our rounding-site count is 0 + 2, not 2 + 2

Counted directly from
[`contracts/controller/src/positions/liquidation/math.rs`](../../contracts/controller/src/positions/liquidation/math.rs).
Scope: the seizure path that pays the liquidator in transferred asset units, via
`apply_liquidation_seizures` → `PoolWithdrawEntry`. A seizure settled in some
other representation would need its own count.

**Debt leg — no asset-unit loss to the liquidator.** `calculate_repayment_amounts`
does ceil the closeable debt (`unscale_borrow_ceil` = `mul_ceil` in ray, then
`to_asset_ceil` to asset units), so a full close pays up to one unit more than
the exact debt. But the very next line prices that ceiling back:

```rust
let payment_usd = feed.usd_value_wad(env, payment_amount);
```

`payment_usd` is the value of what was actually transferred, and it becomes
`RepayEntry::usd_wad` → `NormalizedRepaymentPlan::repay_usd` →
`total_seizure_usd = repay_usd * (1 + bonus)`. **The liquidator is credited, with
the bonus on top, for every unit it ceils.** **Verified** by
`the_debt_legs_asset_unit_ceiling_is_priced_into_the_repayment_credit`.

**Inferred:** the excess-refund path does not undo this. `process_excess_payment`
floors the refund (`mul_floor` then `to_token_floor`), so a sub-unit excess
refunds exactly zero tokens and the entry's `usd_wad` is left intact; and where it
does trim a leg it recomputes `new_usd` from the trimmed amount, so credit tracks
payment either way. Not separately reproduced.

Everything else on this leg is half-up at WAD granularity (5e-19 USD) and does
not accumulate in either direction.

**Collateral leg — two sites, both against the liquidator, per seized position.**

1. `capped_amount = capped_ray.to_asset_floor(feed.asset_decimals)` on a partial
   seizure. Loses **< 1 collateral unit**. (A leg whose whole seizure floors below
   one unit is skipped outright — `capped_amount <= 0 => continue` — which is the
   same bound, taken all at once.)
2. The dust fee bump:
   ```rust
   let bumped_fee = if protocol_fee_ray > Ray::ZERO && fee_asset == 0 { 1 } else { fee_asset };
   ```
   When the fair fee is a fraction of a unit the protocol charges a whole one, and
   that unit comes out of the liquidator's proceeds
   (`withhold_liquidation_fee` in
   [`contracts/pool/src/ops/withdraw.rs`](../../contracts/pool/src/ops/withdraw.rs)
   subtracts it from the transfer). Costs **< 1 collateral unit**, and it fires
   precisely in the small-position regime this section is about. Already recorded
   as a known defect by `the_dust_fee_bump_charges_more_than_the_realised_excess`.

So:

```
L_round = 2 * unit_value(collateral)     per seized collateral position
```

and because our seizure is **pro-rata across every collateral the account holds**
rather than a single liquidator-chosen pair, `L_round` scales with the leg count
`N <= POSITION_LIMIT_MAX = 5`. That is a cost of the same design choice that buys
us immunity from Aave's dust-collateral griefing (ToB-AAVE-1 / Blackthorn L-3).

```
V* = 2 * N * unit_value(collateral) / (b * (1 - f))
```

`unit_value = price / 10^decimals`, i.e. the USD value of one base unit.

### 6.2 Instantiated for the listed set

Each asset is priced at its oracle's `max_sanity_price_wad` — the highest price
the feed will accept, and therefore the coarsest its unit can get — with the
`liquidation_bonus` and `liquidation_fees` `configs/mainnet/spokes.json`
configures. Floor for comparison:
`DEFAULT_MIN_BORROW_COLLATERAL_USD_WAD = BAD_DEBT_USD_THRESHOLD = $5`.

| Collateral | Dec | Max price | Unit value | `L_round` (N=1) | V* (N=1) | V* (N=5) | Margin at N=5 |
|---|---|---|---|---|---|---|---|
| SolvBTC / xSolvBTC | 8 | $120,000 | $1.2e-3 | $2.4e-3 | **$0.0303** | **$0.1515** | 33x |
| xSolvBTCSolvBTC_LP | 7 | $12,000 | $1.2e-3 | $2.4e-3 | $0.0242 | $0.1212 | 41x |
| SPIKOUKTBL (worst SPIKO) | 5 | $1.4804 | $1.48e-5 | $2.96e-5 | $5.6e-4 | $2.8e-3 | 1,783x |
| XAUM | 9 | $6,000 | $6.0e-6 | $1.2e-5 | $1.67e-4 | $8.3e-4 | 6,000x |
| USDC (200 bps spoke) | 7 | $1.05 | $1.05e-7 | $2.1e-7 | $1.17e-5 | $5.8e-5 | 85,714x |
| XLM | 7 | $1.00 | $1.0e-7 | $2.0e-7 | $2.53e-6 | $1.26e-5 | 396,000x |
| USST / DEJTRSY / DEJAAA | 18 | ~$1.09 | $1e-18 (1 wad) | $2e-18 | $4.5e-17 | $2.2e-16 | ~2e16x |

**Verified** by
`the_min_borrow_collateral_floor_clears_the_unprofitability_threshold_for_every_listed_pair`,
which asserts `V*(N=5) * 30 < floor` for every row
([`contracts/controller/tests/positions/liquidation_math.rs`](../../contracts/controller/tests/positions/liquidation_math.rs)).

**The min-collateral floor clears the unprofitability threshold for every listed
pair, with at least 33x to spare.** The binding pair is SolvBTC — high price
against only 8 decimals — and even an account holding five SolvBTC-like
collaterals needs only 15 cents of repayment to break even against a $5 floor.

Run through `calculate_seized_collateral` rather than the closed form, a
floor-sized ($5) full close realises:

| Collateral | Seized units | Fee units | Liquidator profit | Ideal `5·b·(1−f)` | Rounding cost |
|---|---|---|---|---|---|
| SolvBTC | 4,541 | 45 | $0.3952 | $0.3960 | $8.0e-4 |
| xSolvBTCSolvBTC_LP | 4,583 | 4 | $0.4948 | $0.4950 | $2.0e-4 |
| SPIKOUKTBL | 358,021 | 2,431 | $0.264006 | $0.2640 | −$5.6e-6 |
| XAUM | 900,000 | 6,666 | $0.360004 | $0.3600 | −$4.0e-6 |
| XLM | 54,500,000 | 540,000 | $0.3960 | $0.3960 | $0 |
| USDC (200 bps) | 48,571,428 | 95,238 | $0.09000 | $0.0900 | $5.0e-8 |
| USST | 4.818e18 | 2.294e16 | $0.2250 | $0.2250 | $0 |

**Verified** by `a_floor_sized_liquidation_pays_the_liquidator_for_every_listed_collateral`
(asserts `profit > 0` and `ideal - profit <= L_round`) and
`floor_sized_liquidation_profits_match_the_documented_table` (pins these exact
numbers). The two negative "costs" are half-up rounding landing in the
liquidator's favour, which the bound permits.

### 6.3 Why the floor is the right thing to compare against

- `require_post_pool_risk_gates`
  ([`contracts/controller/src/risk/validation.rs`](../../contracts/controller/src/risk/validation.rs))
  rejects any risk-increasing action leaving `ltv_collateral < floor`. Since
  `liquidation_threshold >= min(ltv, lt)` over the same floored `gate_value`,
  `weighted_collateral >= ltv_collateral >= floor` at that moment, and `HF < 1`
  means `total_debt > weighted_collateral`. **Inferred:** an account that becomes
  liquidatable without an intervening price move owes more than the floor.
- `estimate_liquidation_amount` promotes to a **full close** whenever the leftover
  debt after a partial would fall below `BAD_DEBT_USD_THRESHOLD`, so small
  positions are never left as unprofitable stubs.
- Below the floor, `is_socializable_bad_debt` opens the permissionless
  `clean_bad_debt` path, which needs no profitable liquidator at all.

The three constants are the same $5 and are meant to be read together.

### 6.4 Finding: nothing bounds an asset's *unit* value

**The floor clears the threshold for every asset listed today. It is not
guaranteed to for every asset the validation layer would admit.** `V*` is
proportional to `unit_value = price / 10^decimals`, and nothing checks that
product:

- `MIN_ASSET_DECIMALS = 3` (`validate_market_creation` in
  [`contracts/governance/src/validate/asset.rs`](../../contracts/governance/src/validate/asset.rs));
- `validate_sanity_bounds` accepts `max_wad` up to
  `MAX_REASONABLE_PRICE_WAD = $1e9` per whole token.

At the corner — 3 decimals, $1e9 — one base unit is worth **$1,000,000**, two
hundred thousand times the entire borrow floor. A $5 full close then seizes
*nothing*: the whole seizure floors to zero units, the leg is dropped, and the
repayment settles and burns the debt anyway. **Verified** by
`an_expensive_low_decimal_collateral_makes_a_floor_sized_liquidation_seize_nothing`.

Where the boundary actually sits, at 3 decimals with a 900 bps bonus and 1,200
bps fee: the closed form breaks at a **$198** token price
(`2 x 0.198 = 0.396 = 5 x 0.09 x 0.88`), and the realised net stays positive a
little past it — turning negative at **$237** — because the seizure floor
typically costs well under a full unit. **Verified** by
`the_profitability_boundary_at_three_decimals_sits_between_198_and_237_dollars`.

This is a **listing-admission constraint, not a code defect**: no configured
asset is within four orders of magnitude of it, and reaching it requires
governance to list a 3-decimal asset worth hundreds of dollars a token. The
condition to check before listing a collateral is

```
price / 10^decimals  <=  MinBorrowCollateralUsd * b * (1 - f) / (2 * N)
```

with `N = POSITION_LIMIT_MAX`. At the current floor and the least generous
listed curve (b = 400 bps, f = 1,200 bps, N = 5) that is `unit_value <= $0.0176`,
i.e. a maximum sanity price of:

| Decimals | Max admissible price per whole token |
|---|---|
| 3 | $17.60 |
| 5 | $1,760 |
| 7 | $176,000 |
| 8 | $1,760,000 |
| 18 | $1.76e16 |

Recommended follow-ups, in order of cost:

1. Add this check to the listing checklist / `_preflight-validate-configs`.
   Cheapest, and it is the actual control.
2. Consider making `calculate_seized_collateral` revert instead of returning an
   empty seizure for a non-zero repayment. Today a liquidator can repay debt and
   receive nothing — reachable on live config too, by choosing a repayment
   smaller than one unit of seizure (for SolvBTC, under ~$0.0011). The loss is
   self-inflicted and dust-sized, but it is a silent no-op where a revert would
   be clearer.
3. Consider removing the dust fee bump. It is half of `L_round` and it charges
   the protocol's fee on a bonus the liquidator did not realise.

## 7. Summary table

| Bound | Value | Enforced at | Reached when |
|---|---|---|---|
| Borrow / supply index ceiling | 1e9x initial (`1e36` raw) | `update_borrow_index`, `update_supply_index` | 11 years at 200% APR; 70 at 30% |
| Supply index floor | 1e-3 (`1e24` raw) | `apply_bad_debt_to_supply_index` | total socialization of a market |
| Balance / cap ceiling | ~170.14e9 whole tokens | `Ray::from_asset`, `require_cap_within_asset_domain` | token supply above ~1.7e11 |
| Max borrow rate | 200% APR (`2 * RAY`) | `MarketParamsRaw::validate` | configuration |
| Max oracle price | $1e9 / whole token | `validate_sanity_bounds` | configuration |
| Asset decimals | 3..=18 | `validate_market_creation` | configuration |
| Liquidation profitability floor | $0.15 worst listed pair vs a $5 floor | not enforced; see §6.4 | governance listing an expensive low-decimal asset |
