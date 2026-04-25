# Glossary

Domain vocabulary for the `rs-lending-xlm` Stellar / Soroban lending
protocol. Terms appear in protocol code, architecture docs, audit
artefacts, and operator runbooks.

For the algebraic invariants that anchor each term, see
[`architecture/INVARIANTS.md`](./INVARIANTS.md).

## Fixed-point domains

The protocol uses four fixed-point domains. Every cross-domain conversion
goes through `common::fp_core::mul_div_half_up` with `I256` intermediates.

| Domain | Base | Decimals | Used for | Rust newtype |
|---|---|---|---|---|
| Asset-native | varies (per-token, read on-chain via `decimals()`) | varies | Token amounts on the SAC / SEP-41 boundary. | raw `i128` |
| **BPS** | `10_000` | 4 | Risk parameters: LTV, liquidation threshold, liquidation bonus, fees, reserve factor, oracle tolerance. `1 BPS = 0.01 %`. | `common::fp::Bps` |
| **WAD** | `10^18` | 18 | USD value, health factor, isolated-debt aggregate, prices after Reflector normalisation. | `common::fp::Wad` |
| **RAY** | `10^27` | 27 | Indexes (`supply_index_ray`, `borrow_index_ray`), scaled balances, interest rates per millisecond. Higher precision than WAD because indexes compound continuously. | `common::fp::Ray` |

Constants live in `common/src/constants.rs`:

```rust
pub const BPS: i128 = 10_000;
pub const WAD: i128 = 1_000_000_000_000_000_000;            // 10^18
pub const RAY: i128 = 1_000_000_000_000_000_000_000_000_000; // 10^27
```

## Account modes

A user's `AccountMeta.mode: PositionMode` determines which collateral /
borrow rules apply:

| Mode | Collateral | Borrow | Notes |
|---|---|---|---|
| **Normal** | Multiple assets, weighted by per-asset LTV / liquidation threshold. | Multiple assets, subject to global HF check. | Default mode. |
| **E-mode** | Restricted to assets in one pre-defined `EModeCategory`. | Restricted to the same category. The category overrides per-asset LTV / threshold with category-wide values. | Used for high-correlation asset baskets (stablecoins, ETH-LSTs). Higher LTV available because correlated assets liquidate together. |
| **Isolation** | A single isolated asset; the rest of the supply set is forbidden. | Restricted to assets flagged `isolation_borrow_enabled`; the aggregate USD-WAD debt against the isolated asset is capped by `isolation_debt_ceiling_usd_wad`. | Used for new / risky collateral assets. |

E-mode and isolation are mutually exclusive
(`EModeError::EModeWithIsolated`).

## Asset flags (per-asset, in `AssetConfig`)

| Flag | Meaning |
|---|---|
| `is_collateralizable` | Can be supplied as collateral. |
| `is_borrowable` | Can be borrowed. |
| `is_flashloanable` | Can be flash-loaned (`flash_loan` endpoint). |
| `e_mode_enabled` | Asset participates in at least one e-mode category. |
| `is_isolated_asset` | Asset can be the sole collateral in an isolation account. |
| `is_siloed_borrowing` | When borrowed, must be the **only** debt asset on the account. |
| `isolation_borrow_enabled` | Asset can be borrowed by an isolation-mode account. |

## Risk parameters (per-asset, in `AssetConfig`)

| Parameter | Domain | Meaning |
|---|---|---|
| `loan_to_value_bps` | BPS | Maximum borrow value as a fraction of supply value at borrow-time. Always strictly below `liquidation_threshold_bps`. |
| `liquidation_threshold_bps` | BPS | Health factor crosses 1 when `Σ debt > Σ collateral × LT`. |
| `liquidation_bonus_bps` | BPS | Extra collateral the liquidator receives, capped at `MAX_LIQUIDATION_BONUS = 1_500` (15 %). |
| `liquidation_fees_bps` | BPS | Protocol's cut of the liquidation bonus. |
| `flashloan_fee_bps` | BPS | Fee charged on flash-loan repayment, capped at `MAX_FLASHLOAN_FEE_BPS = 500` (5 %). |
| `borrow_cap` / `supply_cap` | asset-native | Maximum aggregate borrow / supply, in asset units. `0` means unlimited. |
| `isolation_debt_ceiling_usd_wad` | WAD | Aggregate USD ceiling for debt against an isolated asset. |

## Interest rate model

Piecewise linear in utilisation (`U = borrowed / supplied`), capped at
`max_borrow_rate_ray ≤ MAX_BORROW_RATE_RAY = 2 * RAY` (the Taylor
envelope for `compound_interest`).

```
        ^ borrow rate (per second, RAY)
        |
   max  +---------------------*
        |                    /
   slope3  ⎫              /
   slope2  ⎬-------*-----*
   slope1  ⎭ -*---*
   base    +*
        +-+----+----+-------+----> utilisation
        0 mid  optimal      RAY
```

| Field | Meaning |
|---|---|
| `base_borrow_rate_ray` | Rate at zero utilisation. |
| `slope1_ray` | Slope between 0 and `mid_utilization_ray`. |
| `slope2_ray` | Slope between `mid` and `optimal_utilization_ray`. |
| `slope3_ray` | Slope between `optimal` and 100 %. |
| `max_borrow_rate_ray` | Hard ceiling. Capped at `2 * RAY` (200 % per second annualised, well above any realistic operator setting). |
| `reserve_factor_bps` | Protocol's cut of borrow interest. Suppliers get `(BPS - reserve_factor_bps) / BPS`; protocol gets the remainder, accruing into `revenue_ray`. |

Annual rate converts to per-millisecond by dividing by
`MILLISECONDS_PER_YEAR = 31_556_926_000`.

## Indexes

The pool tracks two indexes per asset:

- `supply_index_ray` — monotonically non-decreasing. Increases each
  block by `(1 + supplier_rate * delta_t)`. Decreases **only** in
  bad-debt socialisation, clamped at `SUPPLY_INDEX_FLOOR_RAW = 10^18`
  raw.
- `borrow_index_ray` — monotonically non-decreasing. Increases each
  block by the compound factor over `delta_t`.

Account-level positions store **scaled** balances:

- `SupplyPosition.scaled_amount_ray = actual / supply_index` (RAY).
- `BorrowPosition.scaled_amount_ray = actual / borrow_index` (RAY).

To convert back to actual asset units:
`actual = scaled * index / RAY` (half-up rounding).

## Health factor (HF)

```
HF = (Σ supply_value_usd_wad × liquidation_threshold_bps / BPS) / Σ borrow_value_usd_wad
```

- `HF ≥ 1 WAD` — account is solvent.
- `HF < 1 WAD` — account is liquidatable.
- `HF = i128::MAX` — account has zero debt (well-known sentinel).

Liquidation cascade targets: `1.02 → 1.01 → fallback d_max =
total_coll / (1 + base_bonus)`. See
[`architecture/INVARIANTS.md §9`](./INVARIANTS.md).

## Bad debt

When an account's collateral (USD-WAD) drops at or below `5 * WAD`
(`BAD_DEBT_USD_THRESHOLD`) **and** debt exceeds collateral, the
liquidator path triggers `apply_bad_debt_to_supply_index`: the pool's
`supply_index_ray` decreases proportional to the deficit, socialising
the loss across all suppliers.

The KEEPER role can also call `clean_bad_debt(account_id)` for accounts
where in-liquidation cascade did not trigger; the math path is the
same (`liquidation.rs:463`).

The supply-index floor at `10^18` raw ensures the index never reaches
zero — division-by-near-zero panics (`MathOverflow`) are blocked.

## Reflector oracle

Stellar's primary on-chain oracle. The protocol wires CEX (and
optionally DEX) feeds via `configure_market_oracle`.

| Term | Meaning |
|---|---|
| **`PriceData`** | Tuple `(price, timestamp)` Reflector returns from `lastprice` / `prices`. Price is in 14-decimal USD; rescaled to WAD at the boundary. |
| **TWAP** | Time-weighted average price. Reflector returns the trailing `twap_records` samples (capped at 12, ≈ 1 hour at 300-s resolution). |
| **First tolerance / Last tolerance** | Two-tier deviation bands (`OraclePriceFluctuation`). Inside first: use safe; between first and last: average; beyond last: panic on risk-increasing ops. |
| **`allow_unsafe_price`** | Per-call flag. `true` for supply / repay (risk-decreasing); `false` for borrow / withdraw / liquidation (risk-increasing). Beyond-tolerance reads return safe-anchor or panic accordingly. |
| **Asset kind** (`Stellar` / `Other`) | Reflector dispatch hint. `Stellar(Address)` for native SAC contracts; `Other(Symbol)` for bridged tickers. |

## Storage tiers (Soroban)

| Tier | Lifetime | Bumped by |
|---|---|---|
| **Instance** | Bound to contract WASM lifetime. Bumped on every contract operation. Threshold 120 d, bump 180 d (Soroban max). | Auto-bumped per call. |
| **Persistent (shared)** | Per-market / per-emode records. Threshold 30 d, bump 120 d. | `keepalive_shared_state` (KEEPER). |
| **Persistent (user)** | Per-account / per-position records. Threshold 100 d, bump 120 d. | `keepalive_accounts` (KEEPER). |
| **Temporary** | Single-tx scratch. Auto-GC's after the lifetime elapses. | Not extended. |

Cross-contract example: `FlashLoanOngoing` is in **Instance** storage so
it never expires mid-loan; `FL_PREBAL` (pool's pre-flash-loan balance
snapshot) is **Temporary** so it cannot survive a tx.

## Roles

| Role | Granted by | Scope |
|---|---|---|
| **Owner** | Two-step transfer (`transfer_ownership` + `accept_ownership`). | Lifecycle (`upgrade`, `pause`/`unpause`), config (`edit_asset_config`, `set_position_limits`, `approve_token_wasm`), e-mode mgmt, role grants. |
| **KEEPER** | Owner via `grant_role`. Multiple addresses possible. | `update_indexes`, `keepalive_*`, `clean_bad_debt`, `update_account_threshold`. |
| **REVENUE** | Owner via `grant_role`. | `claim_revenue`, `add_rewards`. |
| **ORACLE** | Owner via `grant_role`. | `configure_market_oracle`, `edit_oracle_tolerance`, `disable_token_oracle`. |

See [`architecture/ACTORS.md`](./ACTORS.md) for trust boundaries and
threat-surface notes per role.

## Strategy primitives

User-facing endpoints that bracket multiple pool operations into a
single atomic call, routed through the operator-set aggregator:

| Endpoint | Purpose |
|---|---|
| `multiply` | Open / increase a leveraged position. Modes: `Multiply` (pure leverage on one asset pair), `Long` / `Short` (asymmetric leverage with optional convert steps). |
| `swap_debt` | Swap one debt asset for another in a single tx. |
| `swap_collateral` | Swap one collateral asset for another in a single tx. Forbidden in isolation mode (`SwapCollateralNoIso`). |
| `repay_debt_with_collateral` | Close (or partially close) a debt position by selling collateral via the aggregator. |

All four bracket the aggregator call with
`set_flash_loan_ongoing(true/false)` and re-verify
`received >= amount_out_min` controller-side after the swap.

## Reserves and revenue

| Term | Meaning |
|---|---|
| **Reserves** | Asset-native balance of a pool contract held in token storage (`tok.balance(pool_addr)`). Equals `supplied_actual - borrowed_actual + revenue_actual` in the no-bad-debt steady state. |
| **`revenue_ray`** | Scaled protocol fee accrued into the supply index. Burned in proportion to a `claim_revenue` transfer; suppliers' implicit share grows as a side-effect of the same scaling. |
| **`supplied_ray`** | Scaled aggregate of all supply positions for the pool. |
| **`borrowed_ray`** | Scaled aggregate of all borrow positions for the pool. |
| **Accumulator** | The external address that `claim_revenue` forwards revenue to. Owner-set, written into the pool at construction. |

## Bulk endpoints

`supply` / `borrow` / `withdraw` / `repay` accept `Vec<(Address, i128)>`
so a user can touch multiple assets in one tx. `liquidate` accepts a
`Vec<(Address, i128)>` of debt payments and seizes proportionally
across **all** of the target's collateral assets.

`PositionLimits = (max_supply, max_borrow)` clamps the per-account
counts to `[1, 32]`. The controller default is `10/10` (set at
`__constructor`); operator can raise after empirical-budget
benchmarking.

## Frequently abbreviated symbols

| Symbol | Expansion |
|---|---|
| `WAD` | "wei add 18 decimals" — `10^18` fixed-point base. |
| `RAY` | "ray, 27 decimals" — `10^27` fixed-point base. |
| `BPS` | "basis points" — `10_000` fixed-point base. |
| `LT` | Liquidation threshold (BPS). |
| `LTV` | Loan-to-value ratio (BPS). |
| `HF` | Health factor (WAD). |
| `SAC` | Soroban Asset Contract — Stellar's native token contract. |
| `SEP-41` | Stellar Ecosystem Proposal 41 — the SAC ABI for fungible tokens. |
| `SEP-40` | Stellar Ecosystem Proposal 40 — the oracle ABI Reflector implements. |
