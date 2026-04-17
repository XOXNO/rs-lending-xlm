# Stellar / Soroban Platform Notes

Reference notes on Soroban platform behaviors the protocol depends on. Each item states the observed behavior and, where relevant, the open question.

## Authorization

- `caller.require_auth()` records an auth requirement the host validates against the transaction's signature tree.
- Cross-contract invocations propagate auth: when controller calls `pool.supply(...)` and wraps it in `require_auth_for_args`, the host requires the controller's auth on the sub-invocation.
- In pool, `verify_admin(&env)` reads `Admin` storage and calls `admin.require_auth()`. Since admin is the controller, the controller must auto-authorize itself in the sub-call.
- Open: when controller calls `pool.supply(...)`, whether the controller must call `env.current_contract_address().require_auth()` explicitly or whether the host auto-authorizes contract-to-contract sub-invocations.
- Open: for `supply(caller, ...)` on controller, user's `caller.require_auth()` applies to the controller call; the controller's auth applies to the pool call. Pool's `supply` takes no `caller` arg.

## Storage Tiers

- Three tiers carry TTL semantics: `instance`, `persistent`, `temporary`.
- Constants in `common/src/constants.rs`:
  - `TTL_BUMP_INSTANCE = 180 days` (Soroban max)
  - `TTL_BUMP_SHARED = 120 days`
  - `TTL_BUMP_USER = 120 days`
- `PoolState` and `MarketParams` live in pool Instance storage (`PoolKey::State`, `PoolKey::Params`).
- `MarketConfig` and per-account positions live in persistent storage.
- `FlashLoanOngoing` lives in Instance storage (`controller/src/storage/mod.rs:175-186`, `env.storage().instance().get/set`). Every contract operation bumps Instance entry TTLs.
- Open: whether a write to a persistent account entry auto-bumps that entry's TTL, or whether `keepalive_shared_state` / `keepalive_accounts` / `keepalive_pools` must touch it explicitly.
- Open: initial TTL for newly-created `AccountMeta` and per-asset position keys. A short initial TTL on an inactive account could lose state.

## Reflector Oracle

- `lastprice(asset)` returns `(price, timestamp)`. `prices(asset, records)` returns a vector of `(price, timestamp)` records.
- `cex_asset_kind` (`Stellar` / `Other`) tells Reflector how to interpret the asset key: a Stellar SAC contract address vs. an external symbol.
- `ReflectorClient` is wired minimally in `controller/src/oracle/reflector.rs` (~40 LOC).
- `configure_market_oracle` reads decimals on-chain.
- Open: `lastprice(...)` behavior for a fresh asset with no published price (returns `None`, reverts, or `(0, 0)`). `OracleError::NoLastPrice = 210` expects `None`-like.
- Open: whether `prices(asset, twap_records)` returns a shorter vector or reverts when fewer records are available. `OracleError::TwapInsufficientObservations = 219` handles short-vector.
- Open: Reflector publish cadence, which bounds a meaningful range for `max_price_stale_seconds`. Tests default to 900s.
- Open: Reflector operator upgrades. Cached `cex_decimals` / `dex_decimals` go stale if decimals change. No on-chain signal currently forces re-config.
- Open: internal dispatch difference between `Stellar` and `Other` asset kinds.

## Tokens (SAC)

- Stellar Asset Contract (SAC) exposes `transfer`, `transfer_from`, `approve`, `balance`, `decimals`.
- `transfer` panics on insufficient balance and returns nothing.
- `transfer_from` requires a prior `approve`.
- `flash_loan_end` uses plain `tok.transfer(receiver -> pool, amount + fee)` (`pool/lib.rs:353`), not `transfer_from` / `approve`.
- SAC `transfer(from, to, amount)` calls `from.require_auth()` internally. For contract-to-contract transfers, the `from` contract must pre-authorize via `env.authorize_as_current_contract(...)` inside the callback (e.g., `execute_flash_loan`).
- Transferring to a contract address credits `balance` like an EOA (no callback). Pool reads `tok.balance(env.current_contract_address())` after self-transfers.
- Open: whether every Stellar asset (wrapped XLM, issued USDC, etc.) exposes `decimals()`. `config.rs:311-318` panics with `GenericError::InvalidAsset` when `try_decimals()` fails.
- Open: whether non-SAC token contracts that implement SEP-41 are accepted. `approve_token_wasm` allowlist suggests yes.

## Transaction Limits

Stellar protocol parameters (April 2026) and how bulk ops compare:

| Limit | Value | Relevance |
|---|---|---|
| `tx_max_instructions` | 400_000_000 | Bulk liquidation at max positions |
| `tx_max_disk_read_entries` | 200 | 32 supply + 32 borrow + 32 pool states + 32 oracle = 128 reads |
| `tx_max_write_ledger_entries` | 200 | Same calc |
| `tx_max_size_bytes` | 132_096 | `Vec<(Address, i128)>` of 32 elems ~= 1.3 KB |
| `tx_memory_limit` | 41_943_040 | 41 MB |
| `tx_max_contract_events_size_bytes` | 16_384 | 32-asset op emits ~32 events x ~200B = ~6 KB |
| `tx_max_footprint_entries` | 400 | Combined r/w |

- Open: empirical instruction count for a single `pool.supply` call multiplied by 32.
- Open: whether `update_indexes([32 assets])` fits the instruction budget.

## Inner Contract Calls

- `env.invoke_contract::<T>(addr, fn, args)` invokes a contract function. Errors panic up through the host.
- The host does not pre-approve cross-contract calls; any contract may call any other contract. Auth is handled by the host auth tree.
- `process_flash_loan` invokes `env.invoke_contract::<()>(receiver, "execute_flash_loan", ...)` (`flash_loan.rs:51-55`). If the receiver re-enters mutating controller endpoints, `require_not_flash_loaning` (`borrow.rs:100`) panics.
- Open: re-entry guard coverage for the aggregator path in `strategy::swap_tokens`. The aggregator may call back into controller.

## Events

- The host syscall emits events; the transaction event log persists them.
- Open: whether events emitted by a sub-call are rolled back atomically with state when that sub-call panics. Matters for off-chain consumers.

## Upgrades

- `upgrade(new_wasm_hash)` on controller swaps the contract's WASM. Storage layout survives as long as new code reads the same keys.
- Pool upgrades run per pool via `upgrade_pool(asset, new_wasm_hash)`.
- Pre-upgrade flow calls `pause()`; confirm it blocks user ops until `unpause`.
- Open: deserialization behavior when a new pool ships a changed `MarketParams` layout (storage layout breakage).
- Open: `upgrade` does not check `FlashLoanOngoing`. Owner could upgrade mid-flash-loan. Probably not exploitable; document.
