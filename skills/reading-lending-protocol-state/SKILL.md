---
name: reading-lending-protocol-state
description: Use when reading XOXNO Lending on-chain state — account health factor, positions, collateral/debt values, market rates, utilisation, interest indexes, caps — via contract views from another contract or off-chain RPC simulation.
---

# Reading XOXNO Lending Protocol State

**REQUIRED BACKGROUND:** the `lending-protocol-fundamentals` skill (units,
HubAssetKey, HF semantics).

## Overview

All reads are contract views — free via RPC simulation off-chain, or typed
client calls from another contract. Account and risk views live on the
**controller**; market accounting and rate views live on the **pool**
(address from `get_pool_address()`). App backends wanting enriched REST data
should use the SDK read layer instead (`using-lending-sdk`).

## Controller views (per account)

```rust
fn get_health_factor(account_id: u64) -> i128;      // WAD; i128::MAX = debt-free/missing/saturated
fn is_liquidatable(account_id: u64) -> bool;        // HF < 1 WAD
fn get_total_collateral_usd(account_id: u64) -> i128;  // USD WAD
fn get_total_borrow_usd(account_id: u64) -> i128;      // USD WAD
fn get_ltv_collateral_usd(account_id: u64) -> i128;    // collateral counted toward LTV
fn get_collateral_amount(account_id: u64, hub_asset: HubAssetKey) -> i128;
fn get_borrow_amount(account_id: u64, hub_asset: HubAssetKey) -> i128;
fn get_account_positions(account_id: u64)
    -> (Map<HubAssetKey, AccountPositionRaw>, Map<HubAssetKey, DebtPositionRaw>);
fn get_account_attributes(account_id: u64) -> AccountAttributes; // mode + spoke
fn account_exists(account_id: u64) -> bool;
```

## Action sizing

There are **no** `max_supply` / `max_borrow` / `max_withdraw` views — they
were removed from the controller. Size actions yourself from the config and
usage views below.

```rust
fn get_spoke_usage(spoke_id: u32, hub_asset: HubAssetKey) -> SpokeUsageRaw;
```

`SpokeUsageRaw`: `supplied_scaled_ray`, `borrowed_scaled_ray` — RAY-scaled
shares, not underlying. The controller compares them against the cap rescaled
the same way, so compare in that domain:

```text
cap_scaled = floor(rescale(cap, asset_decimals -> 27) / index)  // supply or borrow index
headroom   = cap_scaled - usage_scaled                          // entry reverts if it would go negative
```

`supply_cap` / `borrow_cap` are **always-enforced** ceilings in asset units.
There is no unlimited sentinel: `0` means the market accepts nothing on that
side, and `i128::MAX` is rejected at config time (`InvalidBorrowParams`, #116)
— the only ceiling is the per-asset domain limit `i128::MAX / 10^(27 -
asset_decimals)`. Breaches trap with `SpokeSupplyCapReached` (#311) /
`SpokeBorrowCapReached` (#312).

Exits — withdraw, repay, bad-debt cleanup — are **uncapped**, so a closed
market still lets existing positions unwind. Caps are orthogonal to
`is_collateralizable` / `is_borrowable`: a `borrow_cap` of `0` on an otherwise
enabled side is a deliberate soft wind-down, not a misconfiguration.

## Market and config views (controller)

```rust
// Pool indexes + soft oracle status (does not trap on stale/deviation):
// price_wad / primary_price_wad / anchor_price_wad,
// price_timestamp, stale, deviation, valid.
fn get_market_indexes_detailed(hub_assets: Vec<HubAssetKey>) -> Vec<MarketIndexView>;
fn get_market_index(hub_asset: HubAssetKey) -> MarketIndexRaw; // accrued to now, reads NO oracle
fn get_spoke(spoke_id: u32) -> SpokeConfig;
fn get_spoke_asset(spoke_id: u32, hub_asset: HubAssetKey) -> SpokeAssetConfig; // panics AssetNotSupported if unlisted
fn get_pool_address() -> Address;
```

`valid` is true only when the price is fresh, in-band (if dual-source),
positive, and within sanity — usable for solvency-style decisions. `stale` /
`deviation` are diagnostic flags when `valid` is false.

`SpokeAssetConfig`: `loan_to_value`, `liquidation_threshold`,
`liquidation_bonus`, `liquidation_fees` (bps), `supply_cap`, `borrow_cap`
(asset units), `is_collateralizable`, `is_borrowable`, `paused`, `frozen`
(frozen = no new entries, exits still allowed).

## Pool views (per market)

```rust
fn get_utilisation(hub_asset: HubAssetKey) -> i128;
fn get_deposit_rate(hub_asset: HubAssetKey) -> i128;   // RAY, per MILLISECOND
fn get_borrow_rate(hub_asset: HubAssetKey) -> i128;    // RAY, per MILLISECOND
fn get_supplied_amount(hub_asset: HubAssetKey) -> i128;
fn get_borrowed_amount(hub_asset: HubAssetKey) -> i128;
fn get_reserves(hub_asset: HubAssetKey) -> i128; // accounted cash, donation-proof
fn get_revenue(hub_asset: HubAssetKey) -> i128;
fn get_bulk_indexes(hub_assets: Vec<HubAssetKey>) -> Vec<MarketIndexRaw>; // batch
fn get_sync_data(hub_asset: HubAssetKey) -> PoolSyncData; // raw params + accounting
```

`PoolSyncData` / market params include the rate curve plus flash-loan config:
`is_flashloanable` (bool) and `flashloan_fee` (bps, ≤ 500).

Rates are per-millisecond RAY values. Annualize with
`MILLISECONDS_PER_YEAR = 31_556_926_000` (simple APR = rate × ms-per-year;
compound for APY).

## Scaled positions → underlying

`get_account_positions` returns raw scaled shares. Underlying:

```text
underlying = rescale(scaled * index / RAY, 27 -> asset_decimals)  // half-up
```

(supply index for deposits, borrow index for debt). Prefer
`get_collateral_amount` / `get_borrow_amount`, which do this for you.

## Common mistakes

- **Treating `scaled * index / RAY` as final** — that value is in the
  27-decimal RAY domain; it still needs rescaling to asset decimals.
- **Treating rates as annual** — pool rates are per millisecond.
- **Reading rates from the controller** — they live on the pool.
- **Polling per-asset indexes N times** — use `get_bulk_indexes` /
  `get_market_indexes_detailed`.
- **Reading a cap of `0` as "unlimited"** — `0` is a literal ceiling: that
  side of the market accepts nothing. Headroom is `cap - usage`, never
  infinite.
- **Comparing a cap against underlying** — caps are asset units, usage is
  RAY-scaled shares; rescale and divide by the index before subtracting.
