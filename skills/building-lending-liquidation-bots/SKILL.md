---
name: building-lending-liquidation-bots
description: Use when building a liquidation bot, keeper, or risk monitor for XOXNO Lending — detecting underwater accounts, estimating seize/repay amounts, executing liquidations, or understanding the liquidation bonus curve.
---

# Building XOXNO Lending Liquidation Bots

**REQUIRED BACKGROUND:** the `lending-protocol-fundamentals` skill.

## Overview

An account is liquidatable when its health factor drops below 1 WAD. Any
address **except the account owner** may liquidate (self-liquidation reverts
`SelfLiquidationNotAllowed`; registered delegates and active position managers
are deliberately allowed per governance). Liquidations survive global pause and
frozen; a paused debt listing blocks only the repay leg (tainted-debt). All
through the controller (see ADR 0011 addendum, INVARIANTS §3.3/4.4).

```rust
fn is_liquidatable(account_id: u64) -> bool;         // HF < 1 WAD
fn get_health_factor(account_id: u64) -> i128;       // WAD

fn liquidate(liquidator: Address, account_id: u64,
             debt_payments: Vec<(HubAssetKey, i128)>);
```

`debt_payments` pairs each debt market with the amount you offer, in native
asset decimals. The controller caps what it accepts (close amount) and pulls
**only the accepted amounts** from your balance — offering more than the cap
is safe because the excess is never transferred.

## Estimate before executing

```rust
fn get_liquidation_estimate(account_id: u64,
    debt_payments: Vec<(HubAssetKey, i128)>) -> LiquidationEstimate;

pub struct LiquidationEstimate {
    pub seized_collaterals: Vec<PaymentTuple>, // asset-native units
    pub protocol_fees: Vec<PaymentTuple>,      // fee cut from seized collateral
    pub refunds: Vec<PaymentTuple>,            // informational: the part of your offer that will NOT be taken
    pub max_payment_wad: i128,                 // max accepted debt payment, USD WAD
    pub bonus_rate_bps: i128,                  // bonus used for this estimate
}

fn get_liquidation_collateral(account_id: u64) -> i128; // seizable value, USD WAD
```

Simulate the estimate, check profitability
(`seized - protocol_fees` vs accepted debt paid + fees/gas), then submit
`liquidate` with the same payments.

## Bonus and close-amount model

The bonus is a curve, not a flat rate:

- **Base/floor:** each position's per-asset `liquidation_bonus` (bps,
  snapshotted at position entry), collateral-value-weighted across the
  account.
- **Growth:** the realized bonus rises as HF falls, shaped by the account's
  spoke (`SpokeConfig { liquidation_target_hf_wad, hf_for_max_bonus_wad,
  liquidation_bonus_factor_bps, .. }`).
- **Ceiling:** an account-level solvency bound derived from the seizure
  proportion — not a per-asset cap.
- **Close amount:** bounded by what restores the account toward the spoke's
  target HF (`max_payment_wad`).

Always read the effective bonus from `get_liquidation_estimate`; never assume
a constant.

## Bot loop shape

1. Discover candidates from `position:batch_update` events (see
   `indexing-lending-events`) — do not scan all account ids.
2. Recompute `get_health_factor` for candidates on price/index movement
   (HF views read oracles; batch and budget them).
3. For HF < 1 WAD: pick debt markets, run `get_liquidation_estimate`, verify
   profit after `protocol_fees`.
4. Hold (or flash-source) the debt tokens and submit `liquidate` — build the
   transaction with `buildStellarLiquidateTx` (see `using-lending-sdk`) or
   raw RPC.
5. Confirm via the `position:liquidation` event plus the
   `position:batch_update` legs whose action discriminant is `LiqRepay` (4) /
   `LiqSeize` (5).

## Common mistakes

- **Assuming a fixed bonus** — it is HF-, position-, and spoke-dependent;
  quote it per liquidation.
- **Treating `refunds` as a token transfer** — nothing is pulled beyond the
  accepted amounts; `refunds` only reports the untaken part of your offer.
- **Trying to repay 100% of debt** — the close amount is capped to restore
  the target HF.
- **Ignoring `protocol_fees`** — a slice of seized collateral goes to the
  protocol; profit math without it overstates margin.
- **Trusting `get_health_factor == i128::MAX` as "healthy"** — it also means
  missing account or saturated dust-debt; combine with `account_exists` and
  debt views.
- **Missing funding** — the transaction reverts if the liquidator lacks
  balances/trustlines for the accepted payments.
