# XOXNO Lending — Architecture Reference

This document describes the architecture implemented in this repository. It
summarizes contract responsibilities, storage layout, risk checks, oracle
validation, strategy and flash-loan flows, and operational boundaries without
duplicating function-level documentation. The Rust source is the
authoritative specification; module paths cited here are stable references
into that source.

Last reviewed against source: May 5, 2026.

## 1. Summary

The protocol is a multi-asset lending and borrowing system for Stellar
Soroban, implemented in Rust across four `no_std` crates:

- `controller`: single user-facing contract. Owns account state, market
  configuration, oracle resolution, access control, risk checks, liquidation,
  flash loans, and account-bound strategy flows.
- `pool`: one liquidity-pool contract per listed asset. Holds custody and
  asset-local accounting (supply, debt, indexes, reserves, protocol revenue,
  flash-loan settlement, rate-model updates).
- `pool-interface`: typed Soroban contract trait the controller uses to call
  pools.
- `common`: shared fixed-point math (`fp`, `fp_core`), rate model (`rates`),
  constants, errors, events, and contract types.

Pools are owner-gated: every mutating pool entrypoint enforces controller
authorization through `ownable::enforce_owner_auth`. Pools do not call
oracles, routers, or other pools.

## 2. Design Constraints

Properties enforced in the current implementation:

- Risk-increasing operations perform market, oracle, cap, e-mode, isolation,
  LTV, health-factor, and liquidity checks before final state persistence
  (`controller/src/positions/*.rs`, `controller/src/strategy.rs`,
  `controller/src/validation.rs`).
- Users interact only with the controller. The controller calls pools through
  `pool_interface::LiquidityPoolClient`.
- Pool mutating endpoints reject any caller other than the controller via
  `verify_admin` (`pool/src/lib.rs`).
- An account can hold supply and borrow positions in multiple assets within
  per-account limits stored in `ControllerKey::PositionLimits`. Isolated and
  e-mode accounts apply additional asset and category constraints.
- Strategy flows (`multiply`, `swap_collateral`, `swap_debt`,
  `repay_debt_with_collateral`) route through the same controller risk model
  as `supply`, `borrow`, `repay`, `withdraw`.
- Storage records are split per concern (account meta, per-side position
  maps, market config, isolated debt, e-mode category, pools list). Numeric
  domains are explicit per field (BPS, WAD, RAY, asset-native).

## 3. System Topology

```mermaid
flowchart TB
    User["User / integrator"]
    Owner["Owner"]
    Keeper["KEEPER role"]
    OracleRole["ORACLE role"]
    RevenueRole["REVENUE role"]

    subgraph Protocol["Protocol contracts"]
        Controller["Controller"]
        PoolA["Pool: asset A"]
        PoolB["Pool: asset B"]
        PoolN["Pool: ..."]
    end

    subgraph External["External contracts"]
        Token["SAC / SEP-41 token contracts"]
        Reflector["Reflector oracle contracts"]
        Router["Aggregator router"]
        Accumulator["Revenue accumulator"]
    end

    User -->|"supply, borrow, repay, withdraw, liquidate, flash_loan, strategies"| Controller
    Owner -->|"config, pause, upgrade, market listing"| Controller
    Keeper -->|"index sync, threshold updates, TTL keepalive, bad-debt cleanup"| Controller
    OracleRole -->|"oracle configuration"| Controller
    RevenueRole -->|"revenue claim, reward injection"| Controller

    Controller ==>|"owner-gated calls"| PoolA
    Controller ==>|"owner-gated calls"| PoolB
    Controller ==>|"owner-gated calls"| PoolN

    Controller --> Reflector
    Controller --> Router
    Controller --> Token
    PoolA --> Token
    PoolB --> Token
    PoolN --> Token
    Controller --> Accumulator
```

Boundaries enforced in code:

- The controller is the only user-facing protocol contract.
- Each pool is deployed by the controller and owned by it. Pool mutating
  endpoints call `verify_admin`.
- Aggregator-router output is validated by balance-delta checks: the
  controller snapshots its token balances, authorizes a single pull of the
  committed input amount, and verifies on return that the output delta meets
  `total_min_out` (`controller/src/strategy.rs`).
- Oracle prices are validated before use: market status, oracle configuration
  presence, freshness, future-timestamp guard, exchange-source policy, and
  deviation tolerance (`controller/src/oracle/mod.rs`).
- Token contracts must be owner-approved before market listing (single-use
  allow-list at `ApprovedToken(asset)` in `controller/src/storage/instance.rs`),
  and runtime token credits are measured via balance-delta accounting where
  user funds enter the protocol.

## 4. Contract Responsibilities

### 4.1 Controller

Implemented entrypoints (`controller/src/*`):

- Account creation, ownership matching, position lifecycle.
- `supply`, `borrow`, `repay`, `withdraw`, `liquidate`, `clean_bad_debt`.
- Strategies: `multiply`, `swap_collateral`, `swap_debt`,
  `repay_debt_with_collateral`.
- `flash_loan`.
- Market listing: `approve_token_wasm`, `revoke_token_wasm`,
  `set_liquidity_pool_template`, `create_liquidity_pool`,
  `configure_market_oracle`, `edit_oracle_tolerance`, `disable_token_oracle`.
- Asset, e-mode, isolation, caps, position-limit, aggregator, accumulator
  configuration.
- Pool parameter and pool WASM upgrades (`upgrade_pool_params`,
  `upgrade_pool`).
- `claim_revenue`, `add_rewards`.
- TTL keepalive: `keepalive_shared_state`, `keepalive_accounts`,
  `keepalive_pools`.
- `pause`, `unpause`, `transfer_ownership`, `accept_ownership`,
  `grant_role`, `revoke_role`, `upgrade`.
- View surface: health, collateral, debt, positions, account attributes,
  market and e-mode configs, isolated-debt counter, batch market and index
  views, liquidation estimation.

### 4.2 Pool

Implemented in `pool/src/lib.rs`, `pool/src/cache.rs`, `pool/src/interest.rs`,
`pool/src/views.rs`. Each pool manages exactly one listed asset and:

- Holds the token balance for its asset.
- Tracks `supplied_ray`, `borrowed_ray`, `revenue_ray`, `supply_index_ray`,
  `borrow_index_ray`, `last_timestamp` in a single Instance record
  (`PoolKey::State`).
- Calls `interest::global_sync` before every mutation.
- Verifies reserve availability before outgoing transfers
  (`cache::has_reserves`).
- Records protocol revenue as a scaled supply claim and updates the supply
  index accordingly.
- Executes `flash_loan_begin`/`flash_loan_end` with a balance snapshot at
  `FLASH_LOAN_PRE_BALANCE` and verifies post-repay balance ≥ pre-balance + fee.
- Reduces the supply index on bad-debt socialization, floored at
  `SUPPLY_INDEX_FLOOR_RAW`.
- Updates rate-model parameters (`update_params`) after syncing accrued
  interest.
- Upgrades pool WASM through `upgrade` when called by its owner.

Pools store no account ownership, oracle configuration, e-mode state, or
isolation rules.

### 4.3 Pool Interface

`pool-interface/src/lib.rs` defines the controller-to-pool ABI as the
`LiquidityPoolInterface` trait. Mutating: `supply`, `borrow`, `withdraw`,
`repay`, `update_indexes`, `add_rewards`, `flash_loan_begin`,
`flash_loan_end`, `create_strategy`, `seize_position`, `claim_revenue`,
`update_params`, `upgrade`, `keepalive`. Read-only: `capital_utilisation`,
`reserves`, `deposit_rate`, `borrow_rate`, `protocol_revenue`,
`supplied_amount`, `borrowed_amount`, `delta_time`, `get_sync_data`.

## 5. Account and Storage Model

Account state is split into metadata plus two position maps:

- `ControllerKey::AccountMeta(u64)` → `AccountMeta { owner, is_isolated,
  e_mode_category_id, mode, isolated_asset }`.
- `ControllerKey::SupplyPositions(u64)` → `Map<Address, AccountPosition>`.
- `ControllerKey::BorrowPositions(u64)` → `Map<Address, AccountPosition>`.

`AccountPosition` does not store the asset, account id, or side. Asset is the
enclosing map key, side is the enclosing storage key, and account id is the
discriminant inside that key. Fields:
`scaled_amount_ray`, `liquidation_threshold_bps`, `liquidation_bonus_bps`,
`liquidation_fees_bps`, `loan_to_value_bps`. The four risk-parameter fields
are an open-time snapshot. Liquidation-threshold updates are keeper-gated by
`update_account_threshold` and require a 5% health-factor buffer for
risk-increasing changes.

Splitting positions per side allows:

- supply-only flows to read and write only the supply side
  (`process_supply` in `controller/src/positions/supply.rs`),
- repay-only flows to touch only the borrow side (`process_repay` in
  `controller/src/positions/repay.rs`),
- full health-factor checks to load both sides where required.

```mermaid
classDiagram
    direction LR

    class AccountMeta {
        +Address owner
        +bool is_isolated
        +u32 e_mode_category_id
        +PositionMode mode
        +Option~Address~ isolated_asset
    }

    class SupplyPositions {
        +Map~Address, AccountPosition~
    }

    class BorrowPositions {
        +Map~Address, AccountPosition~
    }

    class AccountPosition {
        +i128 scaled_amount_ray
        +u32 liquidation_threshold_bps
        +u32 liquidation_bonus_bps
        +u32 liquidation_fees_bps
        +u32 loan_to_value_bps
    }

    AccountMeta "1" --> "1" SupplyPositions : keyed by account id
    AccountMeta "1" --> "1" BorrowPositions : keyed by account id
    SupplyPositions ..> AccountPosition : map value
    BorrowPositions ..> AccountPosition : map value
```

## 6. Market Lifecycle

```mermaid
stateDiagram-v2
    [*] --> PendingOracle: create_liquidity_pool
    PendingOracle --> Active: configure_market_oracle
    Active --> Disabled: disable_token_oracle
    Disabled --> Active: configure_market_oracle
```

Listing path (`controller/src/router.rs::create_liquidity_pool`):

1. Owner sets the pool WASM template (`set_liquidity_pool_template`).
2. Owner approves the token contract address (`approve_token_wasm`).
3. Owner calls `create_liquidity_pool(asset, params, config)`.
4. The controller probes the token contract for `decimals` and `symbol`,
   rejects double-listing, and requires the token to be on the
   `ApprovedToken` allow-list. `validate_market_creation` runs
   `validate_asset_config` and `validate_interest_rate_model`.
5. The controller deploys a deterministic pool (salt derived from the asset
   address) with itself as owner and the asset `MarketParams` as
   constructor input.
6. The market is stored as `PendingOracle`. `e_mode_categories` is force-
   cleared at creation.
7. The `ApprovedToken` flag is consumed (single-use).
8. An `ORACLE` role calls `configure_market_oracle` to set Reflector wiring
   and transition the market to `Active`.

Constraints enforced at listing or oracle configuration:

- `MarketParams.asset_id` must equal the listed asset.
- In non-`testing` builds, `MarketParams.asset_decimals` must equal the
  token contract's reported decimals.
- `e_mode_categories` is controller-managed; membership is changed only
  through `add_asset_to_e_mode_category` /
  `edit_asset_in_e_mode_category` / `remove_asset_from_e_mode`.
- `ExchangeSource::SpotOnly` is rejected in non-`testing` builds at
  `configure_market_oracle`.
- Disabled markets reject normal risk operations. The repay path and view
  surface use the cache flag `allow_disabled_market_price = true` so they
  remain reachable.

## 7. Market Configuration and Risk Parameters

`ControllerKey::Market(asset)` stores `MarketConfig`:

- `status` (`MarketStatus`)
- `pool_address`
- `asset_config: AssetConfig`
- `oracle_config: OracleProviderConfig`
- Reflector wiring: `cex_oracle`, `cex_asset_kind`, `cex_symbol`,
  `cex_decimals`, `dex_oracle`, `dex_asset_kind`, `dex_symbol`,
  `dex_decimals`, `twap_records`.

`AssetConfig` fields: `loan_to_value_bps`, `liquidation_threshold_bps`,
`liquidation_bonus_bps`, `liquidation_fees_bps`, `is_collateralizable`,
`is_borrowable`, `is_isolated_asset`, `is_siloed_borrowing`,
`is_flashloanable`, `isolation_borrow_enabled`,
`isolation_debt_ceiling_usd_wad`, `flashloan_fee_bps`, `borrow_cap`,
`supply_cap`, `e_mode_categories`.

`validate_asset_config` (`controller/src/validation.rs`) rejects:

- `liquidation_threshold ≤ LTV` or `liquidation_threshold > BPS`.
- `liquidation_bonus > MAX_LIQUIDATION_BONUS` (1500 bps).
- `liquidation_fees > BPS` (10000 bps).
- Negative `supply_cap` or `borrow_cap` (zero is treated as uncapped per the
  cap-sentinel comment).
- Negative `isolation_debt_ceiling_usd_wad`.
- `flashloan_fee_bps > MAX_FLASHLOAN_FEE_BPS` (500 bps).

`validate_interest_rate_model` rejects:

- non-monotone slopes
  (`base ≤ slope1 ≤ slope2 ≤ slope3 ≤ max_borrow_rate`),
- `max_borrow_rate_ray > MAX_BORROW_RATE_RAY` (`2 * RAY`),
- `mid_utilization_ray ≤ 0`,
- `optimal_utilization_ray ≤ mid_utilization_ray`,
- `optimal_utilization_ray ≥ RAY`,
- `reserve_factor_bps ≥ BPS`.

```mermaid
classDiagram
    direction LR

    class MarketConfig {
        +MarketStatus status
        +AssetConfig asset_config
        +Address pool_address
        +OracleProviderConfig oracle_config
        +Option~Address~ cex_oracle
        +ReflectorAssetKind cex_asset_kind
        +Symbol cex_symbol
        +u32 cex_decimals
        +Option~Address~ dex_oracle
        +ReflectorAssetKind dex_asset_kind
        +Symbol dex_symbol
        +u32 dex_decimals
        +u32 twap_records
    }

    class AssetConfig {
        +u32 loan_to_value_bps
        +u32 liquidation_threshold_bps
        +u32 liquidation_bonus_bps
        +u32 liquidation_fees_bps
        +bool is_collateralizable
        +bool is_borrowable
        +bool is_isolated_asset
        +bool is_siloed_borrowing
        +bool is_flashloanable
        +bool isolation_borrow_enabled
        +i128 isolation_debt_ceiling_usd_wad
        +u32 flashloan_fee_bps
        +i128 borrow_cap
        +i128 supply_cap
        +Vec~u32~ e_mode_categories
    }

    class OracleProviderConfig {
        +Address base_asset
        +OracleType oracle_type
        +ExchangeSource exchange_source
        +u32 asset_decimals
        +OraclePriceFluctuation tolerance
        +u64 max_price_stale_seconds
    }

    class OraclePriceFluctuation {
        +u32 first_upper_ratio_bps
        +u32 first_lower_ratio_bps
        +u32 last_upper_ratio_bps
        +u32 last_lower_ratio_bps
    }

    class MarketParams {
        +i128 max_borrow_rate_ray
        +i128 base_borrow_rate_ray
        +i128 slope1_ray
        +i128 slope2_ray
        +i128 slope3_ray
        +i128 mid_utilization_ray
        +i128 optimal_utilization_ray
        +u32 reserve_factor_bps
        +Address asset_id
        +u32 asset_decimals
    }

    MarketConfig --> AssetConfig
    MarketConfig --> OracleProviderConfig
    OracleProviderConfig --> OraclePriceFluctuation
    MarketConfig ..> MarketParams : pool stores rate params
```

## 8. Fixed-Point Domains

Numeric domains (`common/src/constants.rs`, `common/src/fp.rs`):

- token-native units for token transfers,
- `BPS = 10_000` for percentages,
- `WAD = 10^18` for USD values and health factor,
- `RAY = 10^27` for indexes, rates, and scaled balances.

Positions store scaled balances. Actual amounts are reconstructed as:

- `supply_actual = scaled_supply * supply_index / RAY`
- `borrow_actual = scaled_debt * borrow_index / RAY`

Pool indexes are synced before each mutation, so accrual happens by updating
the indexes rather than rewriting positions.

Protocol revenue is held as a scaled supply claim in the pool: fees increase
`revenue_ray` and feed the supply index until `claim_revenue` burns the
realized scaled revenue and transfers tokens to the pool owner (the
controller), which forwards them to the configured accumulator.

## 9. Oracle Pricing

The controller resolves prices through `oracle::token_price`
(`controller/src/oracle/mod.rs`). Prices are normalized to WAD.

`ExchangeSource` modes:

- `SpotOnly`: development/testing path, rejected in non-`testing` builds at
  `configure_market_oracle`.
- `SpotVsTwap`: CEX spot vs CEX TWAP from the same Reflector contract.
- `DualOracle`: CEX TWAP vs DEX spot, where DEX unavailability falls back to
  CEX TWAP.

`configure_market_oracle` validates:

- token decimals and symbol via the token contract,
- `cex_decimals` from the CEX Reflector contract,
- `dex_decimals` when `dex_oracle` is set,
- CEX `lastprice` for the asset,
- DEX `lastprice` for the asset when `dex_oracle` is set,
- `twap_records ≤ 12`,
- `60 ≤ max_price_stale_seconds ≤ 86_400`,
- first tolerance in `[MIN_FIRST_TOLERANCE, MAX_FIRST_TOLERANCE]`,
- last tolerance in `[MIN_LAST_TOLERANCE, MAX_LAST_TOLERANCE]`,
- `first_tolerance < last_tolerance`.

Cache modes (`ControllerCache`, `controller/src/cache/mod.rs`) define how
oracle results are gated:

- **Strict** (`new(env, false)`): deviation outside the last band reverts;
  staleness reverts. Used by `borrow`, `liquidate`, `multiply`, `swap_*`,
  `repay_debt_with_collateral`, `update_account_threshold`.
- **Permissive** (`new(env, true)`): deviation outside the last band falls
  back to the safe price; staleness is bypassed. Used by `supply`,
  `flash_loan`, `update_indexes`, `claim_revenue`, debt-free `withdraw`,
  view paths, pool-parameter upgrade.
- **Disabled-market permissive**
  (`new_with_disabled_market_price(env, allow_unsafe)`): same as the prior
  two but additionally allows pricing for `MarketStatus::Disabled` markets.
  Used by `repay`. `allow_unsafe = !meta.is_isolated`: isolated accounts
  use the strict deviation/staleness gates because the global isolated-debt
  counter is updated in USD WAD.
- **View** (`new_view`): both flags `true` for read-only entrypoints.

The future-timestamp guard (`check_not_future`, ±60 seconds clock skew) is
unconditional and applies in every mode.

Price selection inside `calculate_final_price`:

1. Aggregator and safe inside the first tolerance band → safe price.
2. Inside the last tolerance band → midpoint.
3. Outside the last band → revert if the cache is strict; otherwise return
   the safe price.

```mermaid
flowchart TD
    A["token_price(asset)"] --> B{"ControllerCache hit?"}
    B -->|yes| Z["Return cached PriceFeed"]
    B -->|no| C{"Market status usable?"}
    C -->|no| R1["Revert PairNotActive"]
    C -->|yes| D{"oracle_type == Normal?"}
    D -->|no| R2["Revert PairNotActive"]
    D -->|yes| E{"ExchangeSource"}
    E -->|SpotOnly testing| S["CEX spot"]
    E -->|SpotVsTwap| ST["CEX spot + CEX TWAP"]
    E -->|DualOracle| DT["CEX TWAP + optional DEX spot"]
    S --> F["staleness + future-timestamp checks"]
    ST --> F
    DT --> F
    F --> G{"Within first band?"}
    G -->|yes| H["Use safe price"]
    G -->|no| I{"Within last band?"}
    I -->|yes| J["Use midpoint"]
    I -->|no| K{"allow_unsafe_price?"}
    K -->|yes| H
    K -->|no| R3["Revert UnsafePriceNotAllowed"]
    H --> Z
    J --> Z
```

## 10. Common Controller Flow

Every user operation enters through the controller and proceeds through the
same skeleton (`controller/src/positions/*.rs`,
`controller/src/strategy.rs`, `controller/src/flash_loan.rs`):

```mermaid
flowchart LR
    Entry["Controller entrypoint"] --> Auth["require_auth + pause check"]
    Auth --> Load["Load AccountMeta and required position maps"]
    Load --> Plan["Aggregate batch + market/risk validation"]
    Plan --> Price["Resolve prices via ControllerCache"]
    Price --> TokenMove{"Token movement?"}
    TokenMove -->|inbound| DeltaIn["Transfer + balance-delta measurement"]
    TokenMove -->|outbound| PoolCall["Pool mutation"]
    DeltaIn --> PoolCall
    PoolCall --> PoolSync["Pool global_sync + scaled accounting"]
    PoolSync --> PostRisk{"Risk-increasing?"}
    PostRisk -->|yes| HF["require_healthy_account"]
    PostRisk -->|no| Persist["Persist touched account side"]
    HF --> Persist
    Persist --> Events["Emit events + flush isolated-debt cache"]
```

The remaining subsections list only the per-flow specifics that diverge
from this skeleton.

### 10.1 Supply

`supply(caller, account_id, e_mode_category, assets)`
(`controller/src/positions/supply.rs`):

- `account_id == 0` creates a new account owned by `caller`. Existing-account
  deposits from third parties are accepted because they only add collateral.
- Duplicate payments are aggregated before token movement.
- Cache is permissive (`new(env, true)`).
- Token credit is measured by the pool's balance delta (fee-on-transfer
  tokens supported; over-crediting rejected).
- Validates active markets, supply caps, e-mode, isolation, and bulk
  position limits before transferring.
- Writes only the supply side.

### 10.2 Borrow

`borrow(caller, account_id, borrows)`
(`controller/src/positions/borrow.rs`):

- Caller authorization and account-owner match.
- Cache is strict (`new(env, false)`).
- Validates borrowability, LTV, borrow caps, position limits, siloed
  borrowing, e-mode, and isolation debt ceilings.
- Pool checks reserve availability before transferring tokens.
- Isolated debt is tracked in `IsolatedDebt(asset)` USD WAD and flushed once
  per batch via `cache.flush_isolated_debts()`.
- Post-batch `require_healthy_account` gates the entire borrow batch.

### 10.3 Repay

`repay(caller, account_id, payments)`
(`controller/src/positions/repay.rs`):

- Any authenticated caller may repay any account.
- Cache is `new_with_disabled_market_price(env, !meta.is_isolated)`: isolated
  accounts use strict pricing because `IsolatedDebt(asset)` is decremented in
  USD WAD; non-isolated accounts use permissive pricing and remain reachable
  for `Disabled` markets.
- Tokens are pulled into the pool with balance-delta accounting; the pool
  burns scaled debt and refunds overpayment.
- Full repay does not delete the account; account deletion is reserved for
  owner-driven `withdraw` flows.

### 10.4 Withdraw

`withdraw(caller, account_id, withdrawals)`
(`controller/src/positions/withdraw.rs`):

- Caller authorization and account-owner match.
- `amount == 0` is the withdraw-all sentinel; pools clamp full withdrawals
  to the post-accrual balance and apply a dust-lock guard.
- Borrow side is loaded only if the account has debt.
- Cache permissiveness mirrors debt presence:
  `new(env, account.borrow_positions.is_empty())`.
- `require_healthy_account` is invoked in both branches and short-circuits
  for debt-free accounts.
- Account storage is removed when both sides are empty after the batch.

### 10.5 Liquidation and Bad Debt

`liquidate(liquidator, account_id, debt_payments)`
(`controller/src/positions/liquidation.rs`):

- Liquidator `require_auth`. Permissionless beyond authorization for the
  liquidator's debt spend.
- Cache is strict (`new(env, false)`).
- `execute_liquidation` derives target repayment, bonus, and protocol fee
  for an account with health factor `< 1.0 WAD`.
- Repaid debt is pulled from the liquidator into the affected pools;
  collateral is seized to the liquidator with bonus and protocol fee
  applied.
- After execution, `check_bad_debt_after_liquidation` may invoke
  `seize_position(Borrow)` on each remaining debt asset when collateral
  ≤ `BAD_DEBT_USD_THRESHOLD` (5 USD WAD) and debt > collateral; the pool
  reduces the supply index with floor `SUPPLY_INDEX_FLOOR_RAW`.
- `clean_bad_debt(account_id)` is a `KEEPER`-only standalone path.

```mermaid
flowchart TD
    A["Load supply + borrow maps"] --> B["Strict oracle prices"]
    B --> C{"HF < 1.0 WAD?"}
    C -->|no| R["Revert HealthFactorTooHigh"]
    C -->|yes| D["execute_liquidation: repayment, bonus, fee"]
    D --> E["Liquidator → pool: debt repayment"]
    E --> F["Pool burns scaled debt"]
    F --> G["Pool → liquidator: seized collateral"]
    G --> H["Protocol fee → revenue_ray"]
    H --> I{"collateral ≤ BAD_DEBT_USD_THRESHOLD<br/>and debt > collateral?"}
    I -->|yes| K["Pool seize_position(Borrow), supply index reduction"]
    I -->|no| L["Persist account sides"]
    K --> L
```

## 11. Strategy and Flash-Loan Flows

### 11.1 Strategies

`controller/src/strategy.rs` exposes:

- `multiply(caller, account_id, e_mode_category, collateral_token,
  debt_to_flash_loan, debt_token, mode, swap, initial_payment,
  convert_swap)`
- `swap_debt(caller, account_id, existing_debt_token, amount,
  new_debt_token, swap)`
- `swap_collateral(caller, account_id, current_collateral, amount,
  new_collateral, swap)`
- `repay_debt_with_collateral(caller, account_id, collateral_token,
  collateral_amount, debt_token, swap, close_position)`

All four require account-owner authorization and run market, oracle, e-mode,
isolation, cap, and health checks shared with the underlying flows.

`AggregatorSwap` shape (see `common/src/types.rs`):

- one or more `SwapPath`s,
- per-path `split_ppm` (parts per million),
- per-hop `token_in`, `token_out`, `pool`, `venue`, `fee_bps`,
- aggregate `total_min_out`.

`validate_aggregator_swap` (`controller/src/strategy.rs`) rejects:

- empty `paths`,
- `amount_in <= 0` or `total_min_out <= 0`,
- empty `hops` for any path,
- per-path `split_ppm == 0`,
- `sum_ppm != 1_000_000`,
- first hop `token_in != input token`,
- last hop `token_out != output token`.

The router is invoked through `aggregator::AggregatorClient::batch_execute`
with `BatchSwap { sender = current_contract_address, total_in, total_min_out,
referral_id = 0, paths }`. The controller snapshots its input and output
token balances around the call:

- If post-call input spend exceeds the committed `total_in`, the call
  reverts.
- If post-call output delta is below `total_min_out`, the call reverts.

The router call runs while the flash-loan single-flight flag is set, so
mutating controller endpoints reject re-entry during routing.

```mermaid
sequenceDiagram
    autonumber
    actor U as Account owner
    participant C as Controller
    participant P as Pool
    participant R as Aggregator router
    participant T as Tokens
    participant O as Reflector

    U->>C: strategy call with AggregatorSwap
    C->>C: require_auth, owner match, validate mode + market rules
    C->>O: strict price for risk-bearing legs
    C->>P: initial borrow / withdraw leg
    P-->>C: actual amount, updated index
    C->>C: validate_aggregator_swap
    C->>T: snapshot controller balances
    C->>C: authorize router pull (total_in)
    C->>R: batch_execute(BatchSwap)
    R->>T: route via venues
    C->>T: read post-route balances
    C->>C: input spend ≤ total_in, output delta ≥ total_min_out
    C->>P: final supply / repay leg
    C->>C: post-flow health check + persist account sides
```

### 11.2 Flash Loans

`flash_loan(caller, asset, amount, receiver, data)`
(`controller/src/flash_loan.rs`):

- `caller.require_auth()`.
- `require_market_active(asset)`, `is_flashloanable`, `amount > 0`.
- Sets `FlashLoanOngoing = true`.
- Pool transfers `amount` to `receiver` and writes a balance snapshot at
  `FLASH_LOAN_PRE_BALANCE` (`pool/src/lib.rs::flash_loan_begin`).
- The controller invokes `execute_flash_loan(initiator, asset, amount, fee,
  data)` on `receiver`.
- Pool pulls `amount + fee` from `receiver` and verifies `balance_after ≥
  pre_balance + fee` (`flash_loan_end`).
- The fee is recorded as protocol revenue.
- Controller clears `FlashLoanOngoing` and emits `FlashLoanEvent`.

The receiver contract must pre-authorize the pool's repayment pull during
its callback.

```mermaid
sequenceDiagram
    autonumber
    actor I as Initiator
    participant C as Controller
    participant P as Pool
    participant RC as Receiver contract
    participant T as Token

    I->>C: flash_loan(caller, asset, amount, receiver, data)
    C->>C: require_auth, active market, flashloanable, set FlashLoanOngoing
    C->>P: flash_loan_begin(amount, receiver)
    P->>P: global_sync, snapshot FL_PREBAL
    P->>T: transfer pool → receiver
    C->>RC: execute_flash_loan(initiator, asset, amount, fee, data)
    RC->>RC: arbitrary logic + authorize pool repayment pull
    C->>P: flash_loan_end(amount, fee, receiver)
    P->>T: transfer receiver → pool, amount + fee
    P->>P: balance_after ≥ FL_PREBAL + fee
    P-->>C: ok
    C->>C: clear FlashLoanOngoing, emit FlashLoanEvent
```

## 12. E-Mode, Isolation, and Siloed Borrowing

E-mode is category-based. `ControllerKey::EModeCategory(u32)` stores
`EModeCategory { loan_to_value_bps, liquidation_threshold_bps,
liquidation_bonus_bps, is_deprecated, assets: Map<Address,
EModeAssetConfig> }`. Each market stores its reverse membership list in
`AssetConfig.e_mode_categories: Vec<u32>`.

`remove_e_mode_category` flags the category deprecated, clears its asset
map, and removes the category id from each member market's reverse
membership list. Deprecated categories remain readable; new activity is
blocked.

```mermaid
flowchart LR
    Cat["EModeCategory(id)<br/>ltv / threshold / bonus<br/>is_deprecated<br/>assets: Map&lt;Address, EModeAssetConfig&gt;"]
    AssetA["Market(asset A)<br/>AssetConfig.e_mode_categories"]
    AssetB["Market(asset B)<br/>AssetConfig.e_mode_categories"]
    Acct["AccountMeta<br/>e_mode_category_id"]

    Cat -->|"member config"| AssetA
    Cat -->|"member config"| AssetB
    AssetA -->|"reverse membership id"| Cat
    AssetB -->|"reverse membership id"| Cat
    Acct -->|"selected at account creation"| Cat
```

Isolation mode is account-level (`AccountMeta.is_isolated`,
`isolated_asset`):

- An isolated account uses one isolated collateral asset.
- Borrows are limited to assets with `isolation_borrow_enabled = true`.
- Total isolated debt is tracked in `ControllerKey::IsolatedDebt(asset)`
  in USD WAD.
- Borrowing increments the counter; repay and liquidation decrement it.

Siloed borrowing (`AssetConfig.is_siloed_borrowing`) is asset-level: if any
final debt asset is siloed, the account cannot hold multiple debt assets.

## 13. Access Control and Operations

Owner plus three roles (`controller/src/access.rs`):

- Owner (`#[only_owner]`): upgrades, pause/unpause, market listing,
  asset/e-mode/limits/aggregator/accumulator/template configuration, pool
  parameter and pool WASM upgrades, token-listing approval,
  `grant_role`/`revoke_role`.
- `KEEPER` (`#[only_role(caller, "KEEPER")]`): `update_indexes`,
  `update_account_threshold`, `clean_bad_debt`, `keepalive_shared_state`,
  `keepalive_accounts`, `keepalive_pools`.
- `ORACLE`: `configure_market_oracle`, `edit_oracle_tolerance`,
  `disable_token_oracle`.
- `REVENUE`: `claim_revenue`, `add_rewards`.

Constructor (`Controller::__constructor`):

- Sets the owner.
- Sets the access-control admin to the owner.
- Grants only `KEEPER` to the deployer (`REVENUE` and `ORACLE` require an
  explicit `grant_role` after deploy).
- Sets default position limits to 10 supply and 10 borrow positions; the
  validated cap on `set_position_limits` is 32 per side.
- Pauses the controller (`pausable::pause`).

`upgrade(new_wasm_hash)` auto-pauses before invoking
`upgradeable::upgrade`. `transfer_ownership` is two-step
(`stellar_access::role_transfer`); `accept_ownership` synchronizes the
access-control admin with the accepted owner.

```mermaid
flowchart TB
    subgraph UserBoundary["User boundary"]
        Users["Users, liquidators, flash-loan receivers"]
    end

    subgraph OperatorBoundary["Role-gated operator boundary"]
        Owner["Owner"]
        Keeper["KEEPER"]
        Oracle["ORACLE"]
        Revenue["REVENUE"]
    end

    subgraph ProtocolCore["Protocol core"]
        Controller["Controller"]
        Pools["Controller-owned pools"]
    end

    subgraph ExternalBoundary["External contracts"]
        Reflector["Reflector"]
        Router["Aggregator router"]
        Tokens["SAC / SEP-41 tokens"]
        Accumulator["Accumulator"]
    end

    Users -->|"require_auth where needed"| Controller
    Owner -->|"only_owner"| Controller
    Keeper -->|"only_role(KEEPER)"| Controller
    Oracle -->|"only_role(ORACLE)"| Controller
    Revenue -->|"only_role(REVENUE)"| Controller
    Controller ==>|"verify_admin"| Pools
    Controller -->|"validated prices"| Reflector
    Controller -->|"validated batch"| Router
    Controller -->|"balance-delta transfers"| Tokens
    Pools -->|"asset custody"| Tokens
    Controller -->|"claim forwarding"| Accumulator
```

## 14. Storage and TTL Strategy

Soroban storage is partitioned by entry kind:

- **Instance** (`ControllerKey::*` variants and `ApprovedToken(asset)` in
  `controller/src/storage/instance.rs`): `PoolTemplate`, `Aggregator`,
  `Accumulator`, `AccountNonce`, `PositionLimits`, `LastEModeCategoryId`,
  `FlashLoanOngoing`, plus the `ApprovedToken` allow-list.
- **Persistent shared**: `Market(asset)`, `PoolsList`, `EModeCategory(id)`,
  `IsolatedDebt(asset)`.
- **Persistent user**: `AccountMeta(id)`, `SupplyPositions(id)`,
  `BorrowPositions(id)`.
- **Pool Instance** (`PoolKey::Params`, `PoolKey::State`, plus the
  transient `FL_PREBAL` symbol during flash loans).

TTL is bumped explicitly:

- `keepalive_shared_state(assets)`,
- `keepalive_accounts(account_ids)`,
- `keepalive_pools(assets)`,
- ad-hoc instance bumps in market and pool configuration paths.

The split of account state per side lets each flow read/write only the side
it mutates.

```mermaid
flowchart LR
    subgraph ControllerStorage["Controller storage"]
        I["Instance<br/>PoolTemplate, Aggregator, Accumulator<br/>AccountNonce, PositionLimits<br/>LastEModeCategoryId, FlashLoanOngoing<br/>ApprovedToken(asset)"]
        M["Persistent shared<br/>Market(asset), PoolsList<br/>EModeCategory(id)<br/>IsolatedDebt(asset)"]
        U["Persistent user<br/>AccountMeta(id)<br/>SupplyPositions(id)<br/>BorrowPositions(id)"]
    end

    subgraph PoolStorage["Pool storage"]
        PS["Instance Params<br/>MarketParams"]
        ST["Instance State<br/>supplied_ray, borrowed_ray<br/>supply_index, borrow_index<br/>revenue_ray, last_timestamp"]
        FL["Instance FL_PREBAL<br/>flash-loan balance checkpoint"]
    end

    I -->|"deploys via PoolTemplate"| PoolStorage
    M -->|"Market.pool_address"| PoolStorage
```

## 15. Implemented Safety Checks and Access Controls

A non-exhaustive list of checks present in the code:

- Controller starts paused after construction; `upgrade` auto-pauses.
- `#[only_owner]` and `#[only_role]` macros gate operator endpoints.
- Pool mutating endpoints call `verify_admin`.
- Token-listing allow-list (`ApprovedToken(asset)`) is consumed at market
  creation.
- One deterministic pool per listed asset (salt = keccak256 of asset
  address).
- Cap on per-account positions: 32 per side at `set_position_limits`.
- Numeric domains separated into BPS, WAD, RAY with type wrappers in
  `common::fp`.
- Reserve availability is checked before pool transfers out
  (`pool/src/cache.rs::has_reserves`).
- Balance-delta accounting on user deposits, repayments, reward transfers,
  and strategy router output.
- Aggregator route-shape validation and post-route output verification.
- Single-flight `FlashLoanOngoing` guard during flash loans and router
  execution.
- Oracle deviation tolerance, staleness, and unconditional future-timestamp
  guard.
- Bad-debt socialization floor at `SUPPLY_INDEX_FLOOR_RAW`.
- Pool revenue routes only through pool ownership (no caller-supplied
  destination in `claim_revenue`).

## 16. Verification Surface

Repository-contained verification artifacts:

- Rust unit tests in source modules (`#[cfg(test)]`) and the
  `verification/test-harness/tests/` integration suite (`account_tests.rs`,
  `admin_config_tests.rs`, `bad_debt_index_tests.rs`,
  `bench_liquidate_max_positions.rs`, `borrow_tests.rs`,
  `chaos_simulation_tests.rs`, `decimal_diversity_tests.rs`,
  `emode_tests.rs`, `events_tests.rs`, `flash_loan_tests.rs`, …).
- Fuzz targets under `verification/fuzz/fuzz_targets/`: `flow_e2e`, `flow_strategy`,
  `fp_math`, `fp_ops`, `pool_native`, `rates_and_index`.
- Certora verification under `verification/certora/`: `common/spec/` covers
  fixed-point and rate math, `pool/spec/` covers pool accounting and
  summary-contract proofs, `controller/spec/` covers account/risk/oracle
  rules, `controller/harness/` holds verification-only wrappers, and
  `shared/summaries/` holds external-call summaries reachable only behind the
  `certora` Cargo feature.
- Fixed-point and protocol invariants: `architecture/INVARIANTS.md`.
- Vulnerability reporting policy: `SECURITY.md`.

Areas with high implementation complexity (entry points to trace when
extending tests, fuzzing targets, or rules):

- Liquidation math and bad-debt socialization
  (`controller/src/positions/liquidation.rs`,
  `pool/src/interest.rs::apply_bad_debt_to_supply_index`).
- Oracle fallback selection and disabled-market repayment behavior
  (`controller/src/oracle/mod.rs`,
  `controller/src/cache/mod.rs`).
- Strategy authorization, aggregator validation, and balance-delta gates
  (`controller/src/strategy.rs`).
- Isolation debt accounting under partial repay, liquidation, and
  permissive pricing (`controller/src/positions/repay.rs`,
  `controller/src/utils.rs`).
- E-mode category deprecation and keeper-driven threshold updates
  (`controller/src/config.rs`, `controller/src/positions/supply.rs`).
- Pool revenue claim accounting under low-liquidity conditions
  (`pool/src/lib.rs`, `pool/src/interest.rs`).
- Flash-loan callback authorization and re-entry guard coverage
  (`controller/src/flash_loan.rs`, `pool/src/lib.rs`).
- Storage TTL behavior for accounts, shared state, and pools
  (`controller/src/storage/ttl.rs`, `controller/src/router.rs`).

## 17. Deployment and Operations

Deployment is template-driven: the pool WASM is uploaded once, the
controller stores its hash, and each listed asset gets a deterministic pool
deployed by the controller.

```mermaid
flowchart LR
    A["Build and optimize contracts"] --> B["Upload pool WASM"]
    B --> C["Deploy controller<br/>constructor pauses"]
    C --> D["set_liquidity_pool_template"]
    D --> E["set_aggregator"]
    E --> F["set_accumulator"]
    F --> G["grant_role(ORACLE), grant_role(REVENUE)"]
    G --> H["approve_token_wasm(asset)"]
    H --> I["create_liquidity_pool(asset, params, config)"]
    I --> J["configure_market_oracle<br/>PendingOracle → Active"]
    J --> K["edit_asset_config (optional)"]
    K --> L["e-mode setup (optional)"]
    L --> M["Smoke tests / monitoring"]
    M --> N["unpause"]
```

Operational maintenance:

```mermaid
flowchart TD
    Monitor["Off-chain monitoring"] --> A{"Action"}
    A -->|index freshness| K1["KEEPER: update_indexes"]
    A -->|TTL window| K2["KEEPER: keepalive_*"]
    A -->|threshold migration| K3["KEEPER: update_account_threshold"]
    A -->|bad debt| K4["KEEPER: clean_bad_debt"]
    A -->|oracle outage| O1["ORACLE: disable_token_oracle / reconfigure"]
    A -->|revenue| R1["REVENUE: claim_revenue"]
    A -->|incident| P1["Owner: pause"]
    P1 --> Fix["Patch config or upgrade"]
    Fix --> U["Owner: unpause"]
```

## 18. Source Map

- `controller/src/access.rs`: ownership, roles, pause, upgrade, ownership
  transfer.
- `controller/src/router.rs`: market listing, pool deployment, pool
  parameter and WASM upgrades, revenue claim, reward injection, keepalive.
- `controller/src/config.rs`: asset config, e-mode, oracle config,
  aggregator, accumulator, token approval, position limits.
- `controller/src/positions/*.rs`: supply, borrow, repay, withdraw,
  liquidation, account lifecycle, e-mode application, threshold updates.
- `controller/src/strategy.rs`: multiply, debt swap, collateral swap,
  collateral-funded repay, aggregator route validation.
- `controller/src/flash_loan.rs`: flash-loan entrypoint and callback.
- `controller/src/oracle/mod.rs`, `controller/src/oracle/reflector.rs`:
  Reflector integration and price selection.
- `controller/src/cache/mod.rs`: transaction-local cache, isolated-debt
  flush.
- `controller/src/storage/*.rs`: controller storage layout, TTL helpers.
- `controller/src/validation.rs`: input and config validation.
- `controller/src/views.rs`: controller read surface.
- `pool/src/lib.rs`: pool ABI and accounting mutations.
- `pool/src/interest.rs`: interest accrual, revenue accrual, bad-debt
  socialization.
- `pool/src/cache.rs`: pool transient state and reserve checks.
- `pool/src/views.rs`: pool read surface.
- `pool-interface/src/lib.rs`: controller-to-pool ABI trait.
- `common/src/types.rs`: shared ABI types (storage keys, configs,
  positions, oracle, swap).
- `common/src/constants.rs`: fixed-point constants and protocol bounds.
- `common/src/rates.rs`: rate model and index math.
- `common/src/fp.rs`, `common/src/fp_core.rs`: typed fixed-point
  arithmetic.
- `architecture/INVARIANTS.md`: invariant inventory keyed to module paths.

## 19. Status

The repository is pre-audit. Production deployment should be gated on
external audit completion, formal verification review against the target
branch, deployment runbook validation, role-holder policy finalization,
oracle and asset-listing procedures, and incident-response procedures.
