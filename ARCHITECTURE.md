# Architecture

## Overview

The protocol is a two-tier Soroban system:

1. `controller`
   The single protocol entrypoint. Users and operators interact with it for every lending flow.
2. `pool`
   One child contract per listed asset. Pools own actual liquidity, interest accrual, reserves, and
   revenue accounting.

The controller deploys pools from a stored WASM template and owns them through standard
owner/admin control.

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

Pools make no protocol-level solvency decisions. They execute accounting the controller requests.

### Pool Interface

The controller depends on `pool-interface`, not on the `pool` crate itself.

This is intentional:
- it keeps the full pool contract out of controller runtime WASM
- it keeps controller exports and spec smaller
- it makes the controller/pool trust boundary explicit

The interface covers mutating calls such as:
- `supply`
- `borrow`
- `withdraw`
- `repay`
- `update_indexes`
- `add_rewards`
- `create_strategy`
- `seize_position`
- `claim_revenue`
- `update_params`
- `upgrade`

and read-side calls such as:
- `capital_utilisation`
- `reserves`
- `deposit_rate`
- `borrow_rate`
- `protocol_revenue`
- `supplied_amount`
- `borrowed_amount`
- `delta_time`
- `get_sync_data`

## Controller To Pool Communication

### Ownership Model

The controller deploys each pool as the pool's owner/admin.

As a result:
- user calls hit the controller
- controller validates policy and risk
- controller invokes the appropriate pool endpoint as admin
- pools trust controller-originated requests

### Supply Flow

1. User calls `controller.supply`.
2. Controller validates market support, account state, e-mode, isolation, and caps.
3. Controller transfers tokens from user to the target pool.
4. Controller calls `pool.supply(position, price_wad, amount)`.
5. Pool:
   - syncs indexes
   - converts actual amount into scaled supply
   - increases `supplied_ray`
   - returns the updated position plus the market index
6. Controller writes the updated account position and emits protocol events.

### Borrow Flow

1. User calls `controller.borrow`.
2. Controller validates LTV, health factor, borrowability, caps, siloing, e-mode, and isolation.
3. Controller calls `pool.borrow(caller, amount, position, price_wad)`.
4. Pool:
   - syncs indexes
   - checks reserves
   - increases `borrowed_ray`
   - transfers the borrowed tokens to the user
   - returns the updated debt position plus the market index
5. Controller persists the updated account and isolated-debt tracking.

### Repay Flow

1. Any address may call `controller.repay`.
2. Controller transfers repayment tokens from caller to pool.
3. Controller calls `pool.repay`.
4. Pool:
   - syncs indexes
   - applies repayment
   - refunds any overpayment
   - returns the updated position and the actual applied amount
5. Controller:
   - derives the actual applied repayment against the synced borrow index
   - removes or updates the borrow position
   - decrements isolated debt using the actual applied amount

### Withdraw Flow

1. User calls `controller.withdraw`.
2. Controller fetches safe prices and validates the position exists.
3. Controller calls `pool.withdraw`.
4. Pool:
   - syncs indexes
   - caps full withdraws when `amount = 0`
   - transfers tokens out
   - returns the updated position
5. Controller:
   - updates account storage
   - rechecks health factor after the batch if debt remains

### Revenue Flow

1. Operator calls `controller.claim_revenue`.
2. Controller resolves the pool and current safe price.
3. Controller calls `pool.claim_revenue(controller, price_wad)`.
4. Pool:
   - syncs indexes
   - computes claimable revenue from `revenue_ray`
   - caps transfers by actual reserves
   - burns the proportional scaled revenue share
5. Controller forwards claimed tokens to the configured accumulator.

## Storage Model

### Market Storage

One canonical per-market record:
- `ControllerKey::Market(asset) -> MarketConfig`

`MarketConfig` contains:
- `status`
- `asset_config`
- `pool_address`
- `oracle_config`
- flat oracle wiring fields:
  - `cex_oracle`
  - `cex_asset_kind`
  - `cex_symbol`
  - `cex_decimals`
  - `dex_oracle`
  - `dex_asset_kind`
  - `dex_decimals`
  - `twap_records`

No separate reflector storage key exists.

### Account Storage

Account storage is intentionally split:

- `AccountMeta(account_id)`
  - owner
  - isolation flags
  - e-mode category
  - position mode
  - isolated asset
  - lists of supply and borrow assets
- `SupplyPosition(account_id, asset)`
- `BorrowPosition(account_id, asset)`

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

## Market Lifecycle

1. `create_liquidity_pool`
   - deploys pool
   - creates `MarketConfig`
   - seeds `PendingOracle`
2. `configure_market_oracle`
   - validates oracle inputs
   - discovers decimals
   - activates the market
3. `edit_asset_config`
   - enables final collateral/borrow/flashloan flags
4. user operations become available

Markets may later move to:
- `Disabled` through `disable_token_oracle`

## Deployment Relationship

Deployment is template-driven:

- pool WASM uploads once per deployment round
- controller stores the pool template hash
- future `create_liquidity_pool` calls deploy child pools from that template

The deployment Make targets automatically update `configs/networks.json` with:
- controller contract id
- pool wasm hash

## Live Deployment Path

The live path covers:
- controller
- pool
- pool-interface
- common
- Makefile + `configs/script.sh`

## Read This Next

- [README.md](./README.md)
- [DEPLOYMENT.md](./DEPLOYMENT.md)
- [INVARIANTS.md](./INVARIANTS.md)
