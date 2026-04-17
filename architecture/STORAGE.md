# Storage

How the controller and pool contracts lay out Soroban storage — durability
tiers, keys, TTL, and the in-memory cache layer. See `common/src/types.rs`
for the authoritative key enums.

## Durability model

Soroban offers three durability tiers. The protocol uses two:

| Tier | Purpose | Expiry policy |
|---|---|---|
| `Instance` | Globals the contract needs on almost every call (templates, addresses, nonces, flash-loan guard). Shares the contract instance's lifetime; extending the contract TTL extends these. | `INSTANCE_THRESHOLD = 120 days`, `INSTANCE_BUMP = 180 days` (Soroban's ceiling). |
| `Persistent` | Per-market, per-account, per-e-mode records that outlive any single transaction. Keeper routines must extend them. | Shared (market/emode): threshold 30 d, bump 120 d. User (accounts, positions): threshold 100 d, bump 120 d. |
| `Temporary` | Single-transaction scratch (flash-loan pre-balance); auto-GCs after the lifetime elapses. | Effectively one transaction. |

Threshold constants live in `common/src/constants.rs:45-55`.

## Controller storage

### `ControllerKey` (Persistent)

The persistent tree lives at `common/src/types.rs:499-520`. Every variant holds
per-market or per-account data. All writes route through
`controller/src/storage/mod.rs` helpers, which enforce the durability contract
at the API boundary.

| Variant | Value | Purpose |
|---|---|---|
| `Market(Address)` | `MarketConfig` | Consolidated per-asset record: status, `AssetConfig`, `pool_address`, `OracleProviderConfig`, `ReflectorConfig`. One key replaces what older designs split across four. |
| `AccountMeta(u64)` | `AccountMeta` | Account header: owner, isolation flag, e-mode category, borrow/supply asset lists. |
| `SupplyPosition(u64, Address)` | `AccountPosition` | Scaled (RAY) deposit balance for `(account_id, asset)`. |
| `BorrowPosition(u64, Address)` | `AccountPosition` | Scaled (RAY) debt balance for `(account_id, asset)`. |
| `EModeCategory(u32)` | `EModeCategory` | Category definition (LTV, liquidation threshold, deprecated flag). |
| `EModeAsset(u32, Address)` | `EModeAssetConfig` | Per-asset flags inside a category (collateralizable/borrowable). |
| `AssetEModes(Address)` | `Vec<u32>` | Reverse index: every category this asset belongs to. |
| `IsolatedDebt(Address)` | `i128` | Aggregate USD-WAD debt against an isolated-mode asset; compared to its ceiling on each borrow. |
| `PoolsList(u32)` | `(Address, Address)` | Indexed list of `(asset, pool_address)` tuples for enumeration. |

### `LocalKey` (Instance)

Contract-lifetime singletons at `controller/src/lib.rs` and
`controller/src/storage/mod.rs`.

| Variant | Value | Purpose |
|---|---|---|
| `PoolTemplate` | `BytesN<32>` | WASM hash of the pool contract; pools are deployed from this template at market-create time. Set once at controller `__constructor`. |
| `Aggregator` | `Address` | Swap aggregator used by the strategy ops (`multiply`, `swap_*`). |
| `Accumulator` | `Address` | Revenue destination. Pools read this via controller and forward protocol fees. |
| `AccountNonce` | `u64` | Monotonic counter for new account IDs. |
| `PositionLimits` | `PositionLimits` | Global cap on positions per account (`max_supply`, `max_borrow`). |
| `LastEModeCategoryId` | `u32` | High-water mark, auto-incremented on `add_e_mode_category`. |
| `FlashLoanOngoing` | `bool` | Reentrancy guard; see flash-loan section below. |
| `PoolsCount` | `u32` | Length of the `PoolsList` registry. |
| `ApprovedToken(Address)` | `bool` | Allow-list of token contracts eligible to back a new market. |

### Access-control storage

Administrative identity lives in the `stellar-access` crate's own storage
tree, wired in at `controller/src/lib.rs:37-95`. Roles relevant to the protocol:

| Role | Grant path | Scope |
|---|---|---|
| Owner | Two-step transfer via `set_owner` → `accept_owner` (`stellar-access::ownable`). Pending admin held in temporary storage, confirmed admin in instance. | Config mutations, role grants. |
| `KEEPER` | Granted at `__constructor`; additional keepers via `grant_role`. | `update_indexes`, `clean_bad_debt`, keepalive routines, account-threshold propagation. |
| `REVENUE` | Explicit `grant_role` post-deploy. | `claim_revenue`, `add_rewards`. |
| `ORACLE` | Explicit `grant_role` post-deploy. | `configure_market_oracle`, `edit_oracle_tolerance`, `disable_token_oracle`. |

See `architecture/ENTRYPOINT_AUTH_MATRIX.md` for the full role × entrypoint table.

### Flash-loan reentrancy guard

`FlashLoanOngoing` (instance, `bool`) is the single-flight guard. Lifecycle
in `controller/src/flash_loan.rs`:

1. `flash_loan_begin` — asserts the guard is unset, then sets it.
2. Receiver callback runs; any nested entrypoint that reads the guard
   short-circuits with `FlashLoanInProgress`.
3. `flash_loan_end` — clears the guard after verifying repayment.

A panic between begin and end rolls back the entire transaction, so the guard's
`true` value never reaches storage.

## Pool storage

Pools are per-asset child contracts deployed from the controller's
`PoolTemplate`. Each pool has its own instance storage.

### `PoolKey` (Instance)

From `common/src/types.rs:522-544`. Three keys, all instance-scoped.

| Variant | Value | Mutability |
|---|---|---|
| `Params` | `MarketParams` | Immutable after initialization (rate curve, reserve factor, asset decimals, borrow caps). |
| `State` | `PoolState` | Mutable: `supplied_ray`, `borrowed_ray`, `revenue_ray`, `borrow_index_ray`, `supply_index_ray`, `last_timestamp`. |
| `Accumulator` | `Address` | Revenue destination inherited from controller at init. |

### Temporary scratch keys

- `FL_PREBAL` (`symbol_short!`, in `pool/src/cache.rs:26`) — pre-flash-loan
  token balance snapshot, written in `flash_loan_begin` and cleared in
  `flash_loan_end`. Temporary durability because it MUST NOT survive the
  transaction; any post-transaction residue would be a reentrancy vector.

## Cache layer

Two caches. Each eliminates redundant storage reads inside a single
transaction.

### Pool cache (`pool/src/cache.rs`)

- `Cache::load()` reads `Params` and `State` once at the entrypoint.
- The cache holds in-memory `Ray`-typed mirrors of the raw i128 fields.
- **No write-back on `Drop`.** Mutating paths call `cache.save()` explicitly.
  A panic before save must roll back cleanly; `Drop` firing during unwinding
  would smear mid-computation values onto storage.

### Controller cache (`controller/src/cache/mod.rs`)

Transient maps built at the start of each controller entrypoint:

- `prices_cache` — asset → (safe_price_wad, flags). Oracle reads cost host
  budget; the cache amortises them across one transaction.
- `market_configs` — asset → `MarketConfig`. Saves repeated `Market(asset)`
  reads when one call dispatches through several entrypoints.
- `market_indexes` — asset → `MarketIndex` synced from the pool; optionally
  bumps the pool's TTL as a keep-alive side effect.
- `emode_assets` — `(category, asset) → Option<EModeAssetConfig>`.
- `isolated_debts` — asset → i128 accumulator for debt-ceiling adjustments.

The isolated-debt map is the only cache with write-back semantics.
`flush_isolated_debts` iterates the accumulator, writes each delta to
`IsolatedDebt(asset)`, and emits `UpdateDebtCeilingEvent` for each touched
asset. Callers must flush before transaction end; no `Drop` guard runs on
unwind.

## TTL strategy

Persistent keys need active extension. Three keeper-gated routines handle
the fan-out. All live in `controller/src/router.rs:257-302` and dispatch
through `controller/src/storage/mod.rs`.

| Routine | Extends |
|---|---|
| `keepalive_shared_state(assets)` | Instance singletons (`PoolTemplate`, `Aggregator`, `Accumulator`, `PositionLimits`, `PoolsCount`, all `PoolsList(i)`); per-asset: `Market(asset)`, `IsolatedDebt(asset)`, `AssetEModes(asset)`; per e-mode: `EModeCategory(id)` and every `EModeAsset(id, asset)`. |
| `keepalive_accounts(ids)` | `AccountMeta(id)` plus every `SupplyPosition(id, *)` and `BorrowPosition(id, *)` referenced by that account's asset lists. Implemented via `storage::bump_account`. |
| `keepalive_pools(assets)` | Delegates to each pool's own `keepalive` ABI, which bumps the pool's instance keys (`Params`, `State`, `Accumulator`). |

Keeping the three routines separate lets an operator bump a narrow slice —
one account being liquidated, say — without paying the protocol-wide cost.

Regression coverage: `test-harness/tests/fuzz_ttl_keepalive.rs` property-tests
that no orphan `SupplyPosition` / `BorrowPosition` key survives a full
withdraw, and that each keepalive routine extends the keys it claims.

## Cross-contract wiring

Asset ↔ pool lookup goes through the controller's `MarketConfig`:

```
controller.Market(USDC) → MarketConfig { pool_address: Pool_USDC, ... }
                                          │
                                          ▼
                         Pool_USDC.instance: { Params, State, Accumulator }
```

There is no separate `AssetPools` map — the pool address is a field on
`MarketConfig`, so a single `ControllerKey::Market(asset)` read resolves
both the asset's config and its pool address. `PoolsList` exists only for
enumeration (e.g., iterating every market during `keepalive_shared_state`).

The pool reads `Accumulator` from its own instance storage (written once at
pool init from the controller's value). Revenue forwarding never crosses
contracts on the hot path.

## What's not here

- **No migration / versioning framework.** `ControllerKey` and `PoolKey`
  variants carry no version byte; a breaking storage change needs a
  hand-rolled migration entrypoint and a coordinated keeper sweep.
  Redeploying the controller swaps the `PoolTemplate` hash, but existing
  pools keep their original WASM until an operator replaces them.
- **No inter-transaction cache.** Every entrypoint rebuilds the controller
  cache from scratch. Soroban instance reads are cheap once the contract is
  paged in, so the cost stays acceptable.
- **No per-user TTL sharding.** `keepalive_accounts` batches up to the budget
  limit in one call; beyond that the keeper splits the work across calls.
