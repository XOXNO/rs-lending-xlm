# Stellar / Soroban Platform Notes

Items this team understands **less firmly** after migrating from MultiversX. Each item states our current understanding and the question we want auditors to confirm.

## §1. Soroban Authorization Model

### Our understanding
- `caller.require_auth()` records an auth requirement that the host validates against the tx's signature tree.
- Cross-contract invocations propagate auth: when controller calls `pool.supply(...)` and wraps the call in `require_auth_for_args`, the host requires the controller's auth on that sub-invocation.
- In pool, `verify_admin(&env)` reads `Admin` storage and requires `admin.require_auth()`. Since admin is the controller, the controller must auto-authorize itself in the sub-call.

### What's unclear
- **Q1**: When controller calls `pool.supply(...)`, must the controller call `env.current_contract_address().require_auth()` explicitly first, or does Soroban auto-authorize sub-invocations from a contract to its known callee? Pool's `verify_admin` calls `admin.require_auth()`, and the controller does not wrap the pool call. **Confirm this works** — the host treats contract-to-contract auth specially.
- **Q2**: For user-facing endpoints like `supply(caller, ...)` → `pool.supply(...)`, does the user's `caller.require_auth()` propagate to the pool call, or only to the controller call? Pool's `supply` takes no `caller` arg — the *controller* is the verified caller. So user auth applies to controller, controller auth applies to pool. Confirm.

## §2. Soroban Storage Tiers

### Our understanding
- Three storage tiers carry TTL semantics: `instance`, `persistent`, and `temporary`.
- Per `common/src/constants.rs`:
  - `TTL_BUMP_INSTANCE = 180 days` (Soroban max)
  - `TTL_BUMP_SHARED = 120 days`
  - `TTL_BUMP_USER = 120 days`
- `PoolState` and `MarketParams` live in pool Instance storage (`PoolKey::State`, `PoolKey::Params`).
- `MarketConfig` and per-account positions live in persistent storage.
- `FlashLoanOngoing` should live in Instance — it survives only within a tx anyway, but the TTL provides a safety net.

### What's unclear
- **Q3**: RESOLVED. `FlashLoanOngoing` lives in Instance storage — confirmed at `controller/src/storage/mod.rs:175-186` (`env.storage().instance().get/set`). Every contract operation bumps Instance entry TTLs, so the cosmic-ray expiry concern does not apply.
- **Q4**: Tiered keepalive (`keepalive_shared_state`, `keepalive_accounts`, `keepalive_pools`) — does the "writes touch which tier" matrix hold? For example, does a `supply` op on an account auto-bump the account's persistent TTL, or does it require explicit keepalive?
- **Q5**: Account creation — what initial TTL governs `AccountMeta` and the per-asset position keys? A 30-day shared TTL would let an inactive account lose state and lock up.

## §3. Reflector Oracle

### Our understanding
- Reflector contracts return `(price, timestamp)` for `lastprice(asset)` and a vector of `(price, timestamp)` records for `prices(asset, records)`.
- `cex_asset_kind` (`Stellar` / `Other`) tells Reflector how to interpret the asset key — a Stellar SAC contract address versus an external symbol.
- We wired `ReflectorClient` minimally (`controller/src/oracle/reflector.rs`, 40 LOC).
- `configure_market_oracle` reads decimals on-chain.

### What's unclear (HIGH PRIORITY — different from MultiversX)
- **Q6**: When Reflector has no published price (fresh asset), does `lastprice(...)` return `None`, revert, or return `(0, 0)`? Our `OracleError::NoLastPrice = 210` expects something `None`-like.
- **Q7**: TWAP records — if the asset has fewer than `twap_records` published, does Reflector return a shorter vector or revert? Our `OracleError::TwapInsufficientObservations = 219` handles the short-vector case, but confirm Reflector does not panic.
- **Q8**: Reflector publish cadence — what's typical? This bounds the meaningful range for `max_price_stale_seconds`. Tests default to 900s.
- **Q9**: Reflector contract upgrades — if the operator upgrades Reflector to change decimals, our cached `cex_decimals` and `dex_decimals` go stale. Does any on-chain signal flag this and force re-config? Currently none.
- **Q10**: `Stellar` versus `Other` asset kind — what does Reflector do internally? Does each kind handle every asset, or does dispatch route to different price sources?

## §4. Token (SAC) Semantics

### Our understanding
- Stellar Asset Contract (SAC) implements an ERC-20-like ABI: `transfer`, `transfer_from`, `approve`, `balance`, `decimals`.
- `transfer` panics on insufficient balance and returns nothing.
- `transfer_from` requires a prior `approve`.
- Fee repayment in `flash_loan_end` uses plain `tok.transfer(receiver→pool, amount+fee)` (verified pool/lib.rs:353), NOT ERC-20 `transfer_from`/`approve`. Soroban SAC `transfer(from, to, amount)` requires `from.require_auth()` internally — so the receiver must pre-authorize via `env.authorize_as_current_contract(...)` inside `execute_flash_loan`. See Q13b below and `ACTORS.md` flash-loan section.

### What's unclear
- **Q11**: SAC `decimals()` — does every Stellar asset expose it? Do wrapped XLM and Circle-issued USDC both? (config.rs:311-318 panics with `GenericError::InvalidAsset` when `try_decimals()` fails.)
- **Q12**: Custom token contracts (non-SAC) that implement the SEP-41 token interface — does the pool accept arbitrary token contracts that implement the interface? (`approve_token_wasm` allowlist suggests yes, but confirm.)
- **Q13**: Transferring tokens to a contract address credits `balance` like an EOA (no callback). CONFIRMED by code structure: pool reads `tok.balance(env.current_contract_address())` after a transfer to itself, and Soroban SAC handles this as expected.
- **Q13b** (NEW): Soroban SAC `transfer(from, to, amount)` requires `from.require_auth()` internally. For contract-to-contract transfers (e.g., flash_loan_end at pool/lib.rs:353), the `from` contract must pre-authorize via `env.authorize_as_current_contract(...)`. Confirm the receiver-side flash-loan integration spec calls this out.

## §5. Soroban Tx Limits (for §3 Threat Model)

Per current Stellar protocol params (April 2026):

| Limit | Value | Relevance |
|---|---|---|
| `tx_max_instructions` | 400_000_000 | **Bulk liquidation** at max positions — see THREAT_MODEL §3.3 |
| `tx_max_disk_read_entries` | 200 | 32 supply + 32 borrow + 32 pool states + 32 oracle = 128 reads — within limit |
| `tx_max_write_ledger_entries` | 200 | Same calc; within limit |
| `tx_max_size_bytes` | 132_096 | A `Vec<(Address, i128)>` with 32 elements runs ~32 × ~40 = ~1.3 KB — within limit |
| `tx_memory_limit` | 41_943_040 | 41 MB — comfortable |
| `tx_max_contract_events_size_bytes` | 16_384 | We emit per-asset events; a 32-asset op emits ~32 events × ~200B = ~6 KB — within limit |
| `tx_max_footprint_entries` | 400 | Same as r/w combined; within limit |

### What's unclear
- **Q14**: Empirical instruction count for a single `pool.supply` call — multiplied by 32, does it approach the 400M limit?
- **Q15**: `update_indexes([32 assets])` — do all 32 fit the instruction budget?

## §6. Inner Contract Calls

### Our understanding
- `env.invoke_contract::<T>(addr, fn, args)` invokes a contract function. Errors panic up through the host.
- Soroban storage permissioning does not pre-approve cross-contract calls — any contract may call any other contract.
- The host's auth tree handles auth propagation through sub-calls.

### What's unclear
- **Q16**: When `process_flash_loan` calls `env.invoke_contract::<()>(receiver, "execute_flash_loan", ...)` (flash_loan.rs:51-55), the receiver is a user-controlled contract. If the receiver calls `controller.borrow(...)` from inside the callback, `require_not_flash_loaning` (borrow.rs:100) panics. **Resolved**: the receiver cannot re-enter mutating controller endpoints during the callback. This verification downgraded THREAT_MODEL §1 from P0 to P2.
- **Q17**: Aggregator interaction (`strategy::swap_tokens`) — the aggregator may do arbitrary work, including calling back into controller. Does any re-entry guard cover that path? Confirm.

## §7. Event Semantics

### Our understanding
- The host syscall emits events; the tx's event log persists them.
- A panic in a sub-call rolls back state — but **does it roll back events**?

### What's unclear
- **Q18**: When a sub-call emits events and then panics, does the host roll those events back atomically with state? This matters for off-chain consumers that subscribe to events.

## §8. Upgrade Semantics

### Our understanding
- `upgrade(new_wasm_hash)` on controller swaps the contract's WASM. Storage layout survives as long as new code reads the same keys.
- Pool upgrades run per pool via `upgrade_pool(asset, new_wasm_hash)`.
- Pre-upgrade calls `pause()` — confirm it blocks user ops until `unpause`.

### What's unclear
- **Q19**: If an upgrade ships a pool with a different `MarketParams` layout, does deserialization panic on the first read? (Storage layout breakage.)
- **Q20**: Upgrade during flash loan — some endpoints check `FlashLoanOngoing`, but `upgrade` does not. Owner could upgrade while a flash loan is open. Probably not exploitable, but document.

## Summary Of Confirmation Asks

Auditors should produce authoritative answers to **at minimum** these questions:

| ID | Topic | Why critical |
|---|---|---|
| Q1, Q2 | Auth propagation through controller→pool | Ground truth for the entire authz model |
| Q6, Q7, Q9, Q10 | Reflector behavior under edge cases | The single largest unfamiliar dependency |
| Q14, Q15 | Empirical bulk-op cost vs the 400M limit | Decides whether max position counts stay reachable |
| Q16 | Auth context inside the flash-loan receiver callback | Decides THREAT_MODEL §1 exploitability |
| Q11, Q13 | SAC decimals plus transfer-to-contract semantics | Foundation for all token math |
| Q3 | `FlashLoanOngoing` storage tier | Could brick the system on TTL expiry |
