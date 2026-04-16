# Invariants

For protocol engineers, auditors, and anyone who wants the algebra behind the Stellar lending
system rather than only the API surface.

The emphasis is:
- fixed-point domains
- exact state meanings
- why the formulas preserve solvency and accounting
- worked examples using the current implementation

## 1. Fixed-Point Domains

The protocol uses four number systems:

- asset-native units
  Raw token units using each asset's own decimals
- `BPS = 10^4`
  Basis points for percentages such as LTV, liquidation threshold, reserve factor, and fees
- `WAD = 10^18`
  USD values and health-factor arithmetic
- `RAY = 10^27`
  Index math, rates, and scaled balances

### Invariant

Every multiplication or division across domains must explicitly rescale into the target domain
before comparison or persistence.

### Why

Without explicit rescaling, caps, prices, and health-factor logic compare incompatible units.

### Example

For an XLM token with `7` decimals:

- `12_0000000` XLM in asset units
- rescaled to WAD:
  - `12_0000000 -> 12 * 10^18`

This is handled by `common::fp::Wad::from_token` (or `common::fp_core::rescale_half_up`).

## 2. Rounding Discipline

The protocol uses half-up rounding in fixed-point multiplication and division:

- multiply:
  - `(a * b + precision / 2) / precision`
- divide:
  - `(a * precision + b / 2) / b`

### Invariant

All fixed-point arithmetic uses the same half-up convention unless a function explicitly states a
different rule.

### Why

A single rounding policy avoids directional drift where one subsystem rounds down and another
rounds up. The code centralizes this in:
- `mul_half_up`
- `div_half_up`
- signed variants where needed

### Example

At WAD precision:

- `2 / 3`
- exact value: `0.666...`
- stored value:
  - `666_666_666_666_666_667`

That is the half-up rounded representation used by the protocol.

## 3. Scaled Balance Invariant

Positions are stored as scaled balances, never as actual balances.

Definitions:

- `scaled_supply = actual_supply / supply_index`
- `scaled_borrow = actual_borrow / borrow_index`

Reconstruction:

- `actual_supply = scaled_supply * supply_index / RAY`
- `actual_borrow = scaled_borrow * borrow_index / RAY`

### Invariant

For any position:

- `actual >= 0`
- if the index increases while the scaled amount stays fixed:
  - actual supply increases
  - actual debt increases

This is the entire mechanism by which interest accrues without rewriting every position each block.

### Why

Index updates stay O(1) at the market level instead of O(number of positions).

### Example

Suppose:

- user supplies `70 XLM`
- later supplies `50 XLM`
- `supply_index = 1.0 * RAY`

Stored scaled supply:

- first supply adds `70`
- second supply adds `50`
- stored scaled total = `120`

Later, if:

- `supply_index = 1.000000026916666650354166815 * RAY`

Then the actual supply reconstructed from the same scaled amount is slightly above `120 XLM`.

The recent testnet smoke showed exactly this:

- after full repay and before final withdraw, the remaining XLM position reconstructed above the
  originally supplied amount because the supply index had increased.

## 4. Pool State Identity

Each pool tracks:

- `supplied_ray`
- `borrowed_ray`
- `revenue_ray`
- `supply_index_ray`
- `borrow_index_ray`

Where:

- `supplied_ray` is total scaled supply
- `borrowed_ray` is total scaled debt
- `revenue_ray` is scaled protocol-owned supply

### Invariant

`revenue_ray` is always a subset of `supplied_ray`.

Formally:

- `0 <= revenue_ray <= supplied_ray`

### Why

Protocol revenue is modeled as a supply claim owned by the treasury path. It appreciates with the
same supply index as depositor balances.

`add_protocol_revenue` preserves the invariant by incrementing both:

- `revenue_ray`
- `supplied_ray`

Revenue claims burn scaled revenue from both in the same proportion.

## 5. Interest Split Invariant

When borrow interest accrues, the protocol splits it into:

- supplier rewards
- protocol fee

Definitions:

- `old_total_debt = borrowed_ray * old_borrow_index / RAY`
- `new_total_debt = borrowed_ray * new_borrow_index / RAY`
- `accrued_interest = new_total_debt - old_total_debt`
- `protocol_fee = accrued_interest * reserve_factor_bps / BPS`
- `supplier_rewards = accrued_interest - protocol_fee`

### Invariant

`accrued_interest = supplier_rewards + protocol_fee`

### Why

This identity keeps the pool balanced when indexes move.

### Example

If accrued interest is `100` units and reserve factor is `10%`:

- protocol fee = `10`
- supplier rewards = `90`

Then:

- borrow side grows by `100`
- supply side grows by `90`
- revenue side grows by `10`

No value disappears.

## 6. Borrow Index Monotonicity

Borrow index updates depend on:

- utilization
- rate model
- compound-interest factor

The implementation uses:

- piecewise annual borrow rate
- conversion to per-millisecond rate
- 5-term Taylor approximation of `e^(rate * time)`

### Invariant

If:
- utilization is non-negative
- borrow rate is non-negative
- elapsed time is non-negative

then:
- `interest_factor >= RAY`
- `new_borrow_index >= old_borrow_index`

### Why

Debt should not shrink over time absent repayment.

### Implementation note

The code caps the annual rate at `max_borrow_rate_ray` before converting to a per-time-step rate.

## 7. Supply Index Monotonicity And Its Single Exception

Normal updates:

- supplier rewards increase supply index
- external rewards increase supply index

Bad debt socialization:

- may decrease supply index

### Invariant

Outside bad debt socialization:

- `new_supply_index >= old_supply_index`

The only sanctioned decrease is:
- `apply_bad_debt_to_supply_index`

which scales the supply index down proportionally to socialize uncollectable debt.

### Why

Supplier claims should only move in two ways:
- up from earned interest or rewards
- down from explicitly socialized loss

No hidden third path is permitted.

### Safety floor

During bad debt application, the new supply index floors at `1`. So:

- `supply_index_ray >= 1`

always holds.

## 8. Utilization Invariant

Utilization is:

- `borrowed_actual / supplied_actual`

where both sides come from scaled values and current indexes.

### Invariant

If `supplied_actual = 0`, utilization is defined as `0`.

### Why

This avoids division by zero and keeps empty markets from producing undefined rates.

## 9. Health Factor Invariant

Health factor is:

- `HF = weighted_collateral / total_borrow`

where:

- `weighted_collateral = Σ(collateral_value * liquidation_threshold_bps / BPS)`
- `total_borrow = Σ(borrow_value)`

Both are computed in USD WAD.

### Invariant

For any account with debt:

- `HF >= 1e18` means solvent with respect to liquidation threshold
- `HF < 1e18` means liquidatable

For any account without debt:

- `HF = i128::MAX`

### Worked Example

From the recent live smoke:

- supplied: `1200 XLM`
- price: about `0.15014060408169 USD`
- total collateral value:
  - `1200 * 0.15014060408169 = 180.168724898028 USD`
  - for 7-decimal tokens, represented in WAD as
    `180168724898028000000`
- XLM liquidation threshold: `7000 bps = 70%`
- weighted collateral:
  - `180.168724898028 * 0.70 = 126.1181074286196 USD`
- borrowed: `600 XLM`
- total borrow:
  - `600 * 0.15014060408169 = 90.084362449014 USD`

So:

- `HF ≈ 126.1181 / 90.0843 ≈ 1.4`

Observed on-chain:

- `1399999996266666684`

which is `~1.4 WAD`, exactly consistent with the formula.

## 10. LTV Borrow Bound Invariant

Before a borrow batch, the controller computes:

- `ltv_collateral_wad = Σ(collateral_value * loan_to_value_bps / BPS)`

During borrow processing, the controller ensures the post-borrow debt does not exceed this bound.

### Invariant

New borrows succeed only if:

- `post_borrow_total_debt <= ltv_collateral_wad`

### Why

The liquidation threshold controls liquidation; LTV controls borrow allowance. Related, but not
identical risk surfaces.

## 11. Isolation Debt Invariant

For isolated accounts, the controller tracks a global isolated-debt counter on the isolated asset.

The counter is stored in USD WAD.

Borrow path:

- convert borrowed token amount to WAD
- multiply by current price WAD
- increment isolated debt

Repay and liquidation path:

- convert actual repaid amount to USD WAD
- decrement isolated debt
- clamp below zero to zero

### Invariant

For an isolated asset:

- isolated debt is never negative
- isolated debt is bounded by the configured debt ceiling for new borrows

### Dust rule

If remaining isolated debt is:

- `0 < debt < 1 USD WAD`

the tracker is zeroed.

### Why

This keeps stale sub-dollar residue from permanently blocking isolated-asset accounts.

## 12. Claim Revenue Invariant

When claiming revenue:

- pool computes actual claimable revenue from `revenue_ray`
- transfer is capped by current token reserves
- scaled revenue is burned proportionally

### Invariant

Claimed revenue can never exceed current reserves.

Formally:

- `claimed_amount <= current_reserves`

And after a full claim:

- corresponding `revenue_ray` share is removed
- corresponding `supplied_ray` share is removed

### Why

This keeps treasury extraction from creating synthetic liquidity.

### Example

From the recent testnet smoke:

- XLM pool `protocol_revenue` before claim: `29`
- first claim returned `43` after a later index sync because the revenue claim had appreciated with
  the supply index
- `protocol_revenue` then returned `0`

Scaled revenue ownership behaves exactly that way.

## 13. Reserve Availability Invariant

Pool withdrawals, borrows, and flash-loan starts each check reserves against the pool's actual
token balance.

### Invariant

Any outgoing token transfer requiring liquidity must satisfy:

- `current_reserves >= requested_amount`

### Why

Scaled accounting alone does not guarantee inventory. The contract must hold the actual tokens.

## 14. Market Oracle Invariants

The market config stores:
- generic oracle config
- flat CEX/DEX oracle wiring
- cached oracle decimals

### Invariants

1. token decimals are read from the token contract during configuration
2. CEX oracle decimals are read from the CEX oracle during configuration
3. DEX oracle decimals are read from the DEX oracle during configuration if DEX is configured
4. unreadable required decimals revert configuration
5. oracle-feed decimals are never inferred from token decimals

### Why

Token precision and price-feed precision are different domains. Conflating them creates pricing
bugs.

## 15. Controller And Pool Separation Invariant

The controller depends on `pool-interface`, not the full pool contract crate at runtime.

### Invariant

Controller runtime code may only assume the pool ABI, not pool implementation internals.

### Why

This keeps:
- deploy size smaller
- trust boundaries explicit
- upgrades cleaner

## 16. Account Storage Invariant

Account storage is split:

- meta key
- per-supply keys
- per-borrow keys

### Invariant

`AccountMeta` is the canonical index of which position keys must exist for the account.

This means:
- reading an account starts from meta
- account removal removes all listed positions and then meta
- TTL bumps iterate through the asset lists stored in meta

### Why

This prevents hidden orphan positions and keeps account assembly deterministic.

## 17. Design Decisions That Are Intentional

### Half-up rounding instead of truncation

Decision:
- use half-up rounding across fixed-point math

Reason:
- less systematic bias than truncation
- consistent across multiply/divide flows

### Revenue as scaled supply

Decision:
- protocol revenue is represented as scaled supply, not as a separate non-interest-bearing bucket

Reason:
- protocol revenue should appreciate like supplier balances until claimed

### Flat oracle fields in `MarketConfig`

Decision:
- market config holds flat oracle wiring fields
- no separate reflector storage key

Reason:
- one market record reads, caches, and audits more easily

### On-chain decimal discovery

Decision:
- read decimals from contracts during setup

Reason:
- operator-supplied decimals are too error-prone

## 18. What To Re-Verify After Any Math Change

If you touch:
- rate model
- index updates
- liquidation math
- isolation debt
- revenue claim
- precision/rescaling logic

re-verify:

1. `scaled -> actual -> scaled` consistency
2. `accrued_interest = supplier_rewards + protocol_fee`
3. `revenue_ray <= supplied_ray`
4. `HF` transitions around the `1.0 WAD` boundary
5. reserve caps on borrow, withdraw, and claim revenue
6. isolated debt clamping and dust erasure
7. bad debt socialization cannot drive supply index below `1`

## Related Documents

- [README.md](./README.md)
- [ARCHITECTURE.md](./ARCHITECTURE.md)
- [DEPLOYMENT.md](./DEPLOYMENT.md)
