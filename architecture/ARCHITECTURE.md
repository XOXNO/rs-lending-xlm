# Architecture

## Overview

The protocol is a two-tier Soroban system:

1. `controller` — the single protocol entrypoint. Users and operators
   interact with it for every lending flow.
2. `pool` — one child contract per listed asset. Pools own actual
   liquidity, interest accrual, reserves, and revenue accounting.

The controller deploys pools from a stored WASM template and owns them
through standard owner/admin control.

### System topology

```mermaid
flowchart TB
    subgraph users[" "]
        U[User]
        Op[Operator]
    end

    subgraph core["protocol core"]
        Controller
        PoolXLM[Pool XLM]
        PoolUSDC[Pool USDC]
        PoolN[Pool ...]
    end

    subgraph oracles["price surface"]
        CEX[Reflector CEX oracle]
        DEX[Reflector DEX oracle]
    end

    subgraph external["external liquidity"]
        Agg[Swap aggregator]
        AMM[DEX AMM]
    end

    subgraph revenue["revenue sink"]
        Acc[Accumulator]
    end

    U -->|supply / borrow / repay / withdraw / liquidate / flash_loan / multiply / swap_*| Controller
    Op -->|configure / claim_revenue / admin| Controller

    Controller -->|as admin| PoolXLM
    Controller -->|as admin| PoolUSDC
    Controller -.-> PoolN

    Controller -->|price / TWAP| CEX
    Controller -->|price / TWAP| DEX

    Controller -->|route swap| Agg
    Agg --> AMM

    Controller -->|forward claimed revenue| Acc

    PoolXLM -->|token custody| XLMSAC[XLM SAC]
    PoolUSDC -->|token custody| USDCSAC[USDC SAC]
```

Trust boundaries:

- Controller trusts pools only for asset-local accounting.
- Pools trust the controller as admin for every mutation.
- Controller validates every oracle response before using it.
- Aggregator calls execute inside the controller; the controller verifies
  input/output balances around the call (`strategy.rs::swap_tokens`).

## Component Boundaries

### Controller

The controller owns:

- user-facing endpoints
- protocol risk checks
- account lifecycle and storage
- market registry
- oracle and price safety logic
- e-mode and isolation mode
- liquidation orchestration
- strategy orchestration
- flash-loan orchestration
- pool deployment and upgrades
- routing claimed revenue to the accumulator

It stores protocol-wide shared state such as:

- `MarketConfig`
- `EModeCategory`
- `EModeAsset`
- `IsolatedDebt`
- `PoolsList`
- `PositionLimits`

It also stores per-account state as split storage:

- `AccountMeta(account_id)`
- `SupplyPosition(account_id, asset)`
- `BorrowPosition(account_id, asset)`

### Pool

Each pool is asset-local and owns:

- token custody
- aggregate scaled supply and debt
- supply and borrow indexes
- interest-rate model execution
- protocol revenue accrual
- reserve availability checks
- socialization of bad debt into the supply index

Pools make no protocol-level solvency decisions. They execute accounting
the controller requests.

### Pool interface

The controller depends on `pool-interface`, not on the `pool` crate
itself. This is intentional:

- it keeps the full pool contract out of controller runtime WASM
- it keeps controller exports and spec smaller
- it makes the controller/pool trust boundary explicit

The interface covers mutating calls such as:

- `supply`, `borrow`, `withdraw`, `repay`
- `update_indexes`, `add_rewards`
- `create_strategy`, `seize_position`, `claim_revenue`
- `update_params`, `upgrade`
- `flash_loan_begin`, `flash_loan_end`

and read-side calls such as:

- `capital_utilisation`, `reserves`
- `deposit_rate`, `borrow_rate`
- `protocol_revenue`, `supplied_amount`, `borrowed_amount`
- `delta_time`, `get_sync_data`

## Controller-to-Pool Communication

The controller deploys each pool as its owner/admin. Every pool
mutation is therefore gated by `verify_admin`.

### Supply flow

```mermaid
sequenceDiagram
    actor U as User
    participant C as Controller
    participant O as Oracle
    participant T as Token SAC
    participant P as Pool

    U->>C: supply(asset, amount)
    C->>C: validate market / e-mode / isolation / caps
    C->>O: token_price(asset)
    O-->>C: safe price (WAD)
    C->>T: transfer(user → pool, amount)
    C->>P: supply(position, price_wad, amount)
    P->>P: global_sync (accrue interest)
    P->>P: scaled = amount * RAY / supply_index
    P->>P: supplied_ray += scaled
    P-->>C: updated position + MarketIndex
    C->>C: write SupplyPosition + AccountMeta
    C-->>U: emit Supply event
```

### Borrow flow

```mermaid
sequenceDiagram
    actor U as User
    participant C as Controller
    participant O as Oracle
    participant P as Pool

    U->>C: borrow(asset, amount)
    C->>C: validate LTV / HF / borrowability / caps / silo / e-mode / isolation
    C->>O: token_price(asset)
    O-->>C: safe price (WAD)
    C->>P: borrow(caller, amount, position, price_wad)
    P->>P: global_sync
    P->>P: has_reserves(amount) or revert
    P->>P: scaled_debt = amount * RAY / borrow_index
    P->>P: borrowed_ray += scaled_debt
    P->>U: transfer amount
    P-->>C: updated position + MarketIndex
    C->>C: write BorrowPosition, bump IsolatedDebt if isolated
```

### Repay flow

```mermaid
sequenceDiagram
    actor R as Caller
    participant C as Controller
    participant T as Token SAC
    participant P as Pool

    R->>C: repay(account, asset, amount)
    C->>T: transfer(caller → pool, amount)
    C->>P: repay(caller, amount, position, price_wad)
    P->>P: global_sync
    P->>P: current_debt = scaled * borrow_index → asset
    alt amount ≥ current_debt
        P->>R: refund (amount - current_debt)
        P->>P: full scaled burn
    else partial
        P->>P: scaled_repay = amount * RAY / borrow_index
    end
    P-->>C: updated position + actual_applied
    C->>C: decrement isolated_debt by actual_applied
```

### Withdraw flow

```mermaid
sequenceDiagram
    actor U as User
    participant C as Controller
    participant O as Oracle
    participant P as Pool

    U->>C: withdraw(asset, amount)
    C->>O: safe price (WAD)
    C->>P: withdraw(caller, amount, position, price_wad)
    P->>P: global_sync
    P->>P: full = amount ≥ current_supply_actual?
    alt full
        P->>P: scaled_withdrawal = position scaled
    else partial + dust-lock guard
        P->>P: scaled = amount * RAY / supply_index
        Note over P: escalate to full if residual → 0 asset tokens
    end
    P->>P: has_reserves(net_transfer)
    P->>U: transfer(net_transfer)
    P-->>C: updated position
    C->>C: recheck HF if borrows remain (≥ 1.0 WAD)
```

Note: "withdraw all" composes two sentinels. The controller maps
`amount == 0` to `i128::MAX` (`controller/src/positions/withdraw.rs:84`,
comment `// 0 = withdraw all`). The pool then takes the full-withdraw
branch via `amount ≥ current_supply_actual`
(`pool/src/lib.rs:181-183`). Either passing `0` from the caller or
passing any value ≥ the position's current actual supply triggers a
full close.

### Revenue flow

```mermaid
sequenceDiagram
    actor Op as Operator
    participant C as Controller
    participant P as Pool
    participant Acc as Accumulator

    Op->>C: claim_revenue([assets])
    loop per asset
        C->>P: claim_revenue(controller, price_wad)
        P->>P: global_sync
        P->>P: treasury_actual = revenue_ray * supply_index → asset
        P->>P: transfer = min(reserves, treasury_actual)
        P->>P: burn revenue_ray and supplied_ray proportionally
        P->>C: transfer claimed tokens
    end
    C->>Acc: forward claimed tokens
```

## Storage Model

### Market storage

One canonical per-market record:

- `ControllerKey::Market(asset) -> MarketConfig`

```mermaid
classDiagram
    class MarketConfig {
        MarketStatus status
        AssetConfig asset_config
        Address pool_address
        OracleConfig oracle_config
        Address cex_oracle
        AssetKind cex_asset_kind
        Symbol cex_symbol
        u32 cex_decimals
        Option~Address~ dex_oracle
        AssetKind dex_asset_kind
        u32 dex_decimals
        u32 twap_records
    }
    class AssetConfig {
        u32 loan_to_value_bps
        u32 liquidation_threshold_bps
        u32 liquidation_bonus_bps
        u32 liquidation_fees_bps
        bool is_collateralizable
        bool is_borrowable
        bool e_mode_enabled
        bool is_isolated_asset
        bool is_siloed_borrowing
        bool is_flashloanable
        bool isolation_borrow_enabled
        i128 isolation_debt_ceiling_usd_wad
        u32 flashloan_fee_bps
        i128 borrow_cap
        i128 supply_cap
    }
    MarketConfig --> AssetConfig
```

No separate reflector storage key exists. Oracle wiring is flat on
`MarketConfig`.

### Account storage

Account storage is intentionally split into three key families so the hot
paths touch only what they need.

```mermaid
classDiagram
    class AccountMeta {
        Address owner
        bool is_isolated
        u32 e_mode_category_id
        PositionMode mode
        Option~Asset~ isolated_asset
        Vec~Asset~ supply_assets
        Vec~Asset~ borrow_assets
    }
    class SupplyPosition {
        i128 scaled_amount_ray
        u32 loan_to_value_bps
        u32 liquidation_threshold_bps
        u32 liquidation_bonus_bps
    }
    class BorrowPosition {
        i128 scaled_amount_ray
    }
    AccountMeta "1" --> "many" SupplyPosition : one per supply asset
    AccountMeta "1" --> "many" BorrowPosition : one per borrow asset
```

Benefits:

- avoids rewriting large nested account maps on every change
- lets views touch only relevant positions
- supports targeted TTL bumps per account and per position

## Oracle Architecture

Oracle state lives inside `MarketConfig`.

The operator endpoint is:

- `configure_market_oracle(caller, asset, cfg)`

Key design points:

- operators pass neither token decimals nor oracle-feed decimals
- controller reads:
  - token decimals from the asset contract
  - CEX oracle decimals from the CEX oracle
  - DEX oracle decimals from the DEX oracle when configured
- unreadable required decimals revert the transaction

This closes a whole class of operator misconfiguration risk.

Detailed price-resolution logic, including the first/last tolerance tiers
and the `allow_unsafe_price` rule, is documented in
[INVARIANTS.md §14](./INVARIANTS.md#14-market-oracle-invariants).

## Market Lifecycle

```mermaid
stateDiagram-v2
    [*] --> PendingOracle: create_liquidity_pool<br/>(deploys pool + seeds MarketConfig)
    PendingOracle --> Active: configure_market_oracle
    Active --> Active: edit_asset_config<br/>(caps + flags + thresholds)
    Active --> Disabled: disable_token_oracle
    Disabled --> Active: configure_market_oracle
    Active --> [*]
    Disabled --> [*]
```

User operations become available only after both
`configure_market_oracle` and the final `edit_asset_config` land. The
deployment runbook calls these in sequence — see
[DEPLOYMENT.md](./DEPLOYMENT.md).

## Deployment Relationship

Deployment is template-driven:

- pool WASM uploads once per deployment round
- controller stores the pool template hash
- future `create_liquidity_pool` calls deploy child pools from that
  template

The deployment Make targets automatically update `configs/networks.json`
with:

- controller contract id
- pool wasm hash

## Live Deployment Path

The live path covers:

- controller
- pool
- pool-interface
- common
- `Makefile` + `configs/script.sh`

## Read This Next

- [README.md](./README.md)
- [DEPLOYMENT.md](./DEPLOYMENT.md)
- [INVARIANTS.md](./INVARIANTS.md)
- [MATH_REVIEW.md](./MATH_REVIEW.md) — rule-coverage audit of the math
  flows above.
