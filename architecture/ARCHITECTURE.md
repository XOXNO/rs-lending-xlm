# Architecture

Two-tier Soroban system:

1. `controller` — single protocol entrypoint for every lending flow.
2. `pool` — one child contract per listed asset. Owns liquidity, interest
   accrual, reserves, and revenue accounting.

The controller deploys pools from a stored WASM template and retains
owner/admin control.

## System topology

```mermaid
flowchart TB
    U["User"]
    Op["Operator"]

    subgraph core["Protocol core"]
        direction TB
        Controller["Controller"]
        subgraph pools["Pools (one per asset)"]
            direction LR
            PoolXLM["Pool XLM"]
            PoolUSDC["Pool USDC"]
            PoolN["Pool ..."]
        end
    end

    subgraph oracles["Price surface"]
        direction LR
        CEX["Reflector<br/>CEX oracle"]
        DEX["Reflector<br/>DEX oracle"]
    end

    subgraph external["External liquidity"]
        direction LR
        Agg["Swap<br/>aggregator"]
        AMM["DEX AMM"]
    end

    Acc["Accumulator<br/>(revenue sink)"]

    U -->|"supply • borrow • repay<br/>withdraw • liquidate<br/>flash_loan • multiply • swap_*"| Controller
    Op -->|"configure<br/>claim_revenue<br/>admin"| Controller

    Controller ==>|"as admin"| PoolXLM
    Controller ==>|"as admin"| PoolUSDC
    Controller -.->|"as admin"| PoolN

    Controller -->|"price / TWAP"| CEX
    Controller -->|"price / TWAP"| DEX

    Controller -->|"route swap"| Agg
    Agg --> AMM

    Controller -->|"forward<br/>revenue"| Acc
```

Trust boundaries:

- Controller trusts pools only for asset-local accounting.
- Pools trust the controller as admin for every mutation.
- Controller validates every oracle response before use.
- Aggregator calls run inside the controller, which verifies input/output
  balances around the call (`strategy.rs::swap_tokens`).

## Component Boundaries

### Controller

Owns: user-facing endpoints, risk checks, account lifecycle and storage,
market registry, oracle/price-safety logic, e-mode, isolation mode,
liquidation orchestration, strategy orchestration, flash-loan
orchestration, pool deployment and upgrades, and revenue routing to the
accumulator.

Protocol-wide storage: `MarketConfig`, `EModeCategory`, `EModeAsset`,
`IsolatedDebt`, `PoolsList`, `PositionLimits`.

Per-account storage (split):

- `AccountMeta(account_id)`
- `SupplyPosition(account_id, asset)`
- `BorrowPosition(account_id, asset)`

### Pool

Asset-local. Owns token custody, aggregate scaled supply and debt,
supply/borrow indexes, interest-rate model execution, protocol revenue
accrual, reserve availability checks, and socialization of bad debt into
the supply index.

Pools make no protocol-level solvency decisions; they execute the
accounting the controller requests.

### Pool interface

The controller depends on `pool-interface`, not on the `pool` crate.
This keeps the pool contract out of controller runtime WASM, shrinks
controller exports and spec, and makes the trust boundary explicit.

Mutating calls: `supply`, `borrow`, `withdraw`, `repay`, `update_indexes`,
`add_rewards`, `create_strategy`, `seize_position`, `claim_revenue`,
`update_params`, `upgrade`, `flash_loan_begin`, `flash_loan_end`.

Read calls: `capital_utilisation`, `reserves`, `deposit_rate`,
`borrow_rate`, `protocol_revenue`, `supplied_amount`, `borrowed_amount`,
`delta_time`, `get_sync_data`.

## Controller-to-Pool Communication

Every pool mutation is gated by `verify_admin` (the controller is the
pool's admin).

### Supply flow

```mermaid
sequenceDiagram
    autonumber
    actor U as User
    participant C as Controller
    participant O as Oracle
    participant P as Pool

    U->>C: supply(asset, amount)
    Note over C: validate market,<br/>e-mode, isolation, caps
    C->>O: token_price(asset)
    O-->>C: safe price (WAD)
    Note over C: transfer amount<br/>user → pool
    C->>P: supply(position,<br/>price_wad, amount)
    Note over P: global_sync<br/>(accrue interest)
    Note over P: scaled = amount<br/>* RAY / supply_index
    Note over P: supplied_ray += scaled
    P-->>C: updated position<br/>+ MarketIndex
    Note over C: write SupplyPosition<br/>+ AccountMeta
    C-->>U: emit Supply event
```

### Borrow flow

```mermaid
sequenceDiagram
    autonumber
    actor U as User
    participant C as Controller
    participant O as Oracle
    participant P as Pool

    U->>C: borrow(asset, amount)
    Note over C: validate LTV, HF,<br/>borrowability, caps,<br/>silo, e-mode, isolation
    C->>O: token_price(asset)
    O-->>C: safe price (WAD)
    C->>P: borrow(caller, amount,<br/>position, price_wad)
    Note over P: global_sync
    Note over P: has_reserves(amount)<br/>or revert
    Note over P: scaled_debt = amount<br/>* RAY / borrow_index
    Note over P: borrowed_ray<br/>+= scaled_debt
    P->>U: transfer amount
    P-->>C: updated position<br/>+ MarketIndex
    Note over C: write BorrowPosition;<br/>bump IsolatedDebt if isolated
```

### Repay flow

```mermaid
sequenceDiagram
    autonumber
    actor R as Caller
    participant C as Controller
    participant P as Pool

    R->>C: repay(account, asset, amount)
    Note over C: transfer amount<br/>caller → pool
    C->>P: repay(caller, amount,<br/>position, price_wad)
    Note over P: global_sync
    Note over P: current_debt =<br/>scaled * borrow_index<br/>→ asset
    alt amount ≥ current_debt
        P->>R: refund<br/>(amount - current_debt)
        Note over P: full scaled burn
    else partial
        Note over P: scaled_repay = amount<br/>* RAY / borrow_index
    end
    P-->>C: updated position<br/>+ actual_applied
    Note over C: decrement isolated_debt<br/>by actual_applied
```

### Withdraw flow

```mermaid
sequenceDiagram
    autonumber
    actor U as User
    participant C as Controller
    participant O as Oracle
    participant P as Pool

    U->>C: withdraw(asset, amount)
    C->>O: safe price (WAD)
    O-->>C: price_wad
    C->>P: withdraw(caller, amount,<br/>position, price_wad)
    Note over P: global_sync
    Note over P: full = amount ≥<br/>current_supply_actual ?
    alt full
        Note over P: scaled_withdrawal<br/>= position.scaled
    else partial + dust-lock guard
        Note over P: scaled = amount<br/>* RAY / supply_index
        Note over P: escalate to full if<br/>residual → 0 asset tokens
    end
    Note over P: has_reserves(net_transfer)
    P->>U: transfer(net_transfer)
    P-->>C: updated position
    Note over C: recheck HF if borrows<br/>remain (≥ 1.0 WAD)
```

"Withdraw all" uses two sentinels. The controller maps `amount == 0` to
`i128::MAX` (`controller/src/positions/withdraw.rs:84`). The pool takes
the full-withdraw branch via `amount ≥ current_supply_actual`
(`pool/src/lib.rs:181-183`). Passing `0`, or any value ≥ current actual
supply, triggers a full close.

### Revenue flow

```mermaid
sequenceDiagram
    autonumber
    actor Op as Operator
    participant C as Controller
    participant P as Pool
    participant Acc as Accumulator

    Op->>C: claim_revenue([assets])
    loop per asset
        C->>P: claim_revenue(controller,<br/>price_wad)
        Note over P: global_sync
        Note over P: treasury_actual =<br/>revenue_ray<br/>* supply_index<br/>→ asset
        Note over P: transfer = min(<br/>reserves,<br/>treasury_actual)
        Note over P: burn revenue_ray<br/>and supplied_ray<br/>proportionally
        P->>C: transfer claimed tokens
    end
    C->>Acc: forward claimed tokens
```

## Storage Model

### Market storage

One canonical per-market record: `ControllerKey::Market(asset) -> MarketConfig`.

```mermaid
classDiagram
    direction LR

    class MarketConfig {
        +MarketStatus status
        +AssetConfig asset_config
        +Address pool_address
        +OracleConfig oracle_config
    }

    class CexOracle {
        +Address cex_oracle
        +AssetKind cex_asset_kind
        +Symbol cex_symbol
        +u32 cex_decimals
    }

    class DexOracle {
        +Option~Address~ dex_oracle
        +AssetKind dex_asset_kind
        +u32 dex_decimals
        +u32 twap_records
    }

    class AssetConfig {
        +i128 borrow_cap
        +i128 supply_cap
        +u32 loan_to_value_bps
        +u32 liquidation_threshold_bps
        +u32 liquidation_bonus_bps
        +u32 liquidation_fees_bps
        +u32 flashloan_fee_bps
        +i128 isolation_debt_ceiling_usd_wad
    }

    class AssetFlags {
        +bool is_collateralizable
        +bool is_borrowable
        +bool e_mode_enabled
        +bool is_isolated_asset
        +bool is_siloed_borrowing
        +bool is_flashloanable
        +bool isolation_borrow_enabled
    }

    MarketConfig --> AssetConfig : risk params
    MarketConfig --> CexOracle : CEX feed
    MarketConfig --> DexOracle : DEX feed (optional)
    AssetConfig ..> AssetFlags : flags (stored inline)
```

Oracle wiring is flat on `MarketConfig`; no separate reflector key.

### Account storage

Split into three key families so hot paths touch only what they need.

```mermaid
classDiagram
    direction LR

    class AccountMeta {
        +Address owner
        +bool is_isolated
        +u32 e_mode_category_id
        +PositionMode mode
        +Option~Asset~ isolated_asset
        +Vec~Asset~ supply_assets
        +Vec~Asset~ borrow_assets
    }

    class SupplyPosition {
        +i128 scaled_amount_ray
        +u32 loan_to_value_bps
        +u32 liquidation_threshold_bps
        +u32 liquidation_bonus_bps
    }

    class BorrowPosition {
        +i128 scaled_amount_ray
    }

    AccountMeta "1" --> "0..*" SupplyPosition : one per supply asset
    AccountMeta "1" --> "0..*" BorrowPosition : one per borrow asset
```

Avoids rewriting nested account maps on every change, lets views touch
only relevant positions, and supports targeted TTL bumps per account and
per position.

## Oracle Architecture

Oracle state lives in `MarketConfig`. Operator endpoint:
`configure_market_oracle(caller, asset, cfg)`.

- Operators pass neither token decimals nor oracle-feed decimals.
- Controller reads token decimals from the asset contract, CEX oracle
  decimals from the CEX oracle, and DEX oracle decimals from the DEX
  oracle when configured.
- Unreadable required decimals revert the transaction.

Price resolution tiers and the `allow_unsafe_price` rule:
[INVARIANTS.md §14](./INVARIANTS.md#14-market-oracle-invariants).

## Market Lifecycle

```mermaid
stateDiagram-v2
    direction LR

    [*] --> PendingOracle : create_liquidity_pool<br/>(deploy pool<br/>+ seed MarketConfig)

    PendingOracle --> Active : configure_market_oracle

    Active --> Active : edit_asset_config<br/>(caps • flags • thresholds)

    Active --> Disabled : disable_token_oracle
    Disabled --> Active : configure_market_oracle

    Active --> [*]
    Disabled --> [*]
```

User operations unlock only after `configure_market_oracle` and the final
`edit_asset_config` land. See [DEPLOYMENT.md](./DEPLOYMENT.md).

## Deployment

Template-driven:

- Pool WASM uploads once per deployment round.
- Controller stores the pool template hash.
- Subsequent `create_liquidity_pool` calls deploy child pools from that
  template.

Deployment Make targets update `configs/networks.json` with the
controller contract id and pool WASM hash.

Live path: `controller`, `pool`, `pool-interface`, `common`, `Makefile`,
`configs/script.sh`.

## Read This Next

- [README.md](./README.md)
- [DEPLOYMENT.md](./DEPLOYMENT.md)
- [INVARIANTS.md](./INVARIANTS.md)
- [MATH_REVIEW.md](./MATH_REVIEW.md) — rule-coverage audit of the math flows above.
