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

## Action-sizing views

Use these instead of re-deriving limits; all return `0` while paused:

```rust
// pool cash, max-utilization, borrow cap, LTV/HF gates:
fn max_borrow(account_id: u64, hub_asset: HubAssetKey) -> i128;
// position, pool cash, max-utilization cap, LTV/HF gates, dust floor:
fn max_withdraw(account_id: u64, hub_asset: HubAssetKey) -> i128;
// supply-cap headroom ONLY; i128::MAX when uncapped:
fn max_supply(account_id: u64, hub_asset: HubAssetKey) -> i128;
```

## Market and config views (controller)

```rust
fn get_markets_detailed(hub_assets: Vec<HubAssetKey>) -> Vec<AssetExtendedConfigView>;
fn get_market_indexes_detailed(hub_assets: Vec<HubAssetKey>) -> Vec<MarketIndexView>;
fn get_market_index(hub_asset: HubAssetKey) -> MarketIndexRaw; // accrued to now, reads NO oracle
fn get_spoke(spoke_id: u32) -> SpokeConfig;
fn get_spoke_asset(spoke_id: u32, hub_asset: HubAssetKey) -> SpokeAssetConfig; // panics AssetNotSupported if unlisted
fn get_pool_address() -> Address;
```

`SpokeAssetConfig`: `loan_to_value`, `liquidation_threshold`,
`liquidation_bonus`, `liquidation_fees` (bps), `supply_cap`, `borrow_cap`
(asset units), `is_collateralizable`, `is_borrowable`, `paused`, `frozen`
(frozen = no new entries, exits still allowed), optional `oracle_override`.

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
- **Conflating `i128::MAX` meanings** — from `get_health_factor`: no
  effective debt; from `max_supply`: uncapped.
