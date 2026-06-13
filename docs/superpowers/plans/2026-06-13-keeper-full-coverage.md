# Keeper full-coverage upgrade (governance + per-user + key audit)

**Goal:** the keeper (`services/keeper`) scans and TTL-bumps **every** persistent storage entry across pool, controller, **and governance** — including **per-user** account keys — with a verified-complete key inventory. Currently it deliberately skips per-user keys and has no governance awareness.

**Authoritative key inventory** (verified against the contract enums):
- **Pool** persistent (per-asset, enum `PoolKey` in `common/src/types/pool.rs:386`): `Params(Address)`, `State(Address)`. Enumerate via controller `PoolsList`.
- **Controller** (`ControllerKey` in `interfaces/controller/src/types/controller.rs:650`):
  - instance (one ledger entry, covered by bumping the instance): PoolTemplate, Pool, Aggregator, Accumulator, AccountNonce, PositionLimits, LastEModeCategoryId, AppVersion.
  - persistent global/per-asset (covered today): `PoolsList`, `Market(Address)`, `IsolatedDebt(Address)`, `EModeCategory(u32)`.
  - persistent **per-user (NOT covered — the gap)**: `AccountMeta(u64)`, `SupplyPositions(u64)`, `BorrowPositions(u64)` for `1..=AccountNonce`; `IsolatedBasis(u64, Address)` for isolated accounts.
  - temporary: `SessionKey::FlashLoanOngoing` — auto-expires, **never bump**.
- **Governance** (`GovernanceKey` in `contracts/governance/src/storage.rs:11` + embedded libs):
  - instance (one entry): `GovernanceKey::Controller`, ownable `Owner`, access_control `Admin` + `RoleAdmin(role)`.
  - persistent: timelock `MinDelay`; access_control role-holder keys (`HasRole`, `RoleAccounts*`, `ExistingRoles`) for PROPOSER/EXECUTOR/CANCELLER/ORACLE.
  - persistent, **non-enumerable on-chain**: timelock `OperationLedger(BytesN<32>)` per scheduled op — needs event tracking; transient (resolved within `min_delay` ≪ TTL) → document-and-skip.
  - temporary: ownable `PendingOwner`, access_control `PendingAdmin` — auto-expire, never bump.

**Enumeration sources (all on-chain):** `PoolsList` → assets; `AccountNonce` → accounts `1..=N`; `LastEModeCategoryId` → e-mode `1..=N`; `ExistingRoles` → roles → per-role holder counts. Only timelock ops lack an on-chain source.

**Critical correctness rule:** every keeper-built `LedgerKey` must XDR-match exactly what the contract writes (Soroban `#[contracttype]` enum → `ScVal::Vec([Symbol(variant), args…])`, arg order per the enum). A mismatch silently bumps nothing. Verify each new key against the contract enum AND against a live entry via `inspect_ttls` on testnet (deployment exists: governance `CCGAETDF…`, controller `CCH62TUX…`, pool `CC4GOJKP…`; account `1` exists from an earlier supply).

---

### Task K-1: Per-user controller keys + enumeration
**Files:** `services/keeper/src/keys.rs`, `services/keeper/src/discovery.rs`
- [ ] Add a `ControllerUserKey` enum: `AccountMeta(u64)`, `SupplyPositions(u64)`, `BorrowPositions(u64)`, `IsolatedBasis(u64, [u8;32])` with `to_sc_val`/`to_ledger_key` (Persistent). Match the on-chain variant NAMES + arg order exactly (`ControllerKey::AccountMeta(u64)` → `Vec([Symbol("AccountMeta"), U64(id)])`; `IsolatedBasis(u64, Address)` → `Vec([Symbol("IsolatedBasis"), U64(id), Address(Contract(asset))])`).
- [ ] In `discovery.rs`, after role keys: read `AccountNonce` (already harvested from the controller instance), enumerate `1..=account_nonce`, build the 3 per-account keys (chunked fetch like e-mode). For `IsolatedBasis`: decode each fetched `AccountMeta` to learn `is_isolated`+`isolated_asset`; only build `IsolatedBasis(id, isolated_asset)` for isolated accounts (efficient — at most one per account). If decoding AccountMeta is heavy, fall back to building `IsolatedBasis(id, asset)` for assets in the account's `BorrowPositions` map.
- [ ] Add a config knob `schedule.scan_users: bool` (default true now per the goal) + `schedule.max_accounts_scan` cap with a loud `warn!` if `account_nonce` exceeds it (so we never silently truncate — log the dropped range). Append per-user entries to `persistent_entries`.
- [ ] `cargo build -p keeper` (its own workspace — use `--manifest-path services/keeper/Cargo.toml`); keeper unit tests green.

### Task K-2: Governance contract coverage
**Files:** `services/keeper/src/config.rs`, `services/keeper/src/discovery.rs`, `services/keeper/src/keys.rs`, keeper YAML configs
- [ ] `ContractsConfig`: add `governance: Option<String>` (validate `C…` strkey). `ContractIds`: add `governance: Option<[u8;32]>`. Add `governance` to the testnet/mainnet YAML with the deployed address.
- [ ] Add a `GovernancePersistentKey` enum for `MinDelay` (verify the exact stellar-governance `TimelockStorageKey::MinDelay` ScVal encoding from the published crate source in `~/.cargo/registry/.../stellar-governance-0.7.2/src/timelock/storage.rs`). 
- [ ] In `discovery.rs`: if `governance` configured — fetch the governance **instance** entry; build the `MinDelay` persistent key; **reuse `discover_role_keys(client, governance_id, …)`** for the governance role-holder keys (PROPOSER/EXECUTOR/CANCELLER/ORACLE). On any governance read error, `warn!` and continue (don't fail the whole tick).
- [ ] Document the timelock `OperationLedger` non-enumerability in code + README (transient, event-tracked future work).
- [ ] Build + tests green.

### Task K-3: Coverage audit + inspect_ttls + docs
**Files:** `services/keeper/src/bin/inspect_ttls.rs`, `services/keeper/README.md`, key-encoding tests
- [ ] Make `inspect_ttls` report the FULL new surface grouped by class (per-asset, per-user, e-mode, roles, governance) with counts, so coverage is auditable. 
- [ ] Add key-encoding unit tests for every new key variant (assert the `ScVal`/`LedgerKey` bytes against a golden value derived from the contract enum — the way existing keeper key tests do).
- [ ] README: replace the "deliberately does not renew per-user" section with the new behavior; add the governance section; document the timelock-op limitation. Update the durability/coverage table.

### Task K-4: Live validation on testnet (read-only)
- [ ] Run `inspect_ttls` (read-only, no submission) against testnet with the new code + the deployed addresses. Confirm it discovers: account `1`'s three per-user keys (exists from the earlier supply), the per-asset keys for all 5 markets, e-mode 1, the governance instance + `MinDelay` + role keys. Confirm key encodings resolve to REAL entries (value_present=true), proving the XDR matches. Capture the output as evidence.
- [ ] If any new key resolves to `value_present=false` where an entry should exist → the XDR encoding is wrong; fix before declaring done.

## Verify (each task)
- `cargo build --manifest-path services/keeper/Cargo.toml` (+ `clippy -- -D warnings`, `test`) — keeper is its OWN workspace; do NOT use the root workspace. No `cd` (gvm hook) — use `--manifest-path` / `cargo --manifest-path`.
- Final proof is K-4: `inspect_ttls` on testnet shows every key class resolving to live entries.

## Notes / boundaries
- Bumping is permissionless (ExtendFootprintTtl/RestoreFootprint need no role) — per-user scanning is safe to add.
- Cost: per-user scanning is `O(accounts)` RPC reads + bumps per tick; the `max_accounts_scan` cap + chunking bound it; log truncation loudly.
- Timelock per-op keys are the one un-closeable gap on-chain — documented, low-risk (transient).
- Commits per task: `feat(keeper): …`.
