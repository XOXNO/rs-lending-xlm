# Keeper role-key coverage + self-healing restore

**Date:** 2026-06-03
**Component:** `services/keeper`
**Status:** Approved (design)

## Problem

The keeper keeps the XOXNO Lending protocol's Soroban storage alive by extending
TTL before entries fall inside a safety margin. Its discovery set
(`keys.rs::ControllerPersistentKey`) models only `PoolsList`, per-asset `Market` +
`IsolatedDebt`, and `EModeCategory`. It does **not** discover the controller's
access-control entries managed by the vendored OpenZeppelin `stellar_access`
crate: `HasRole(addr, role)`, `RoleAccounts({role, index})`,
`RoleAccountsCount(role)`, `ExistingRoles`.

Those entries are **persistent** and self-extend (+90d) **only when read** by a
role-gated call. On testnet nothing has exercised the ORACLE/KEEPER/REVENUE roles
since deploy (2026-05-08), so 6 of 7 role entries have **expired** (verified
on-chain 2026-06-03, ledger ~2,888,012, `live_until=0`). Once archived they can no
longer be extended — Soroban requires `RestoreFootprint` first — and any
role-gated controller op (oracle config, `update_indexes`, revenue claim) now
fails until the entry is restored. The flash-loan receiver instance + wasm are
archived for the same reason.

Two gaps to close:
1. **Coverage** — the keeper must keep the role entries alive while they are
   still live (prevents recurrence).
2. **Recovery** — the keeper must be able to *restore* entries that are already
   archived, and we must be able to verify that via a dry-run simulation before
   trusting it in production.

## Goals

- Discover and keep alive the controller's access-control role entries.
- Distinguish *live-but-in-margin* (extend) from *archived-but-restorable*
  (restore).
- Add a permissionless `RestoreFootprint` path.
- Make the self-healing loop always restore archived entries and extend in-margin
  entries each tick (operator chose always-on, no gate).
- Change `--dry-run` to actually **simulate** every planned job (extend + restore)
  and report pass/fail + resource fee, submitting nothing — with no funded signer
  required.

## Non-goals

- Reviving fully *evicted* entries (data gone). Restore sim will REJECT these; the
  dry-run is how we detect them. No recreation logic.
- Per-user triplet (`AccountMeta`/`SupplyPositions`/`BorrowPositions`) keepalive —
  still out of scope by design (users self-bump).
- Changing the `update_indexes` role-gated path.

## Design

### 1. Discovery — access-control role keys (`keys.rs`, `discovery.rs`)

Mirror `vendor/openzeppelin/stellar-access/src/access_control/storage.rs`:

- `AccessControlPersistentKey` enum with `ExistingRoles`, `RoleAccountsCount(Symbol)`,
  `RoleAccounts{role: Symbol, index: u32}`, `HasRole{account: [u8;32]/Address, role: Symbol}`,
  each `to_ledger_key(controller_id)` (persistent ContractData). Encoding mirrors
  soroban `#[contracttype]`: unit/tuple variants as `Vec[Symbol(name), args…]`;
  the `RoleAccounts` struct arg as `Map{index, role}` (fields sorted by symbol).
- Discovery procedure per tick:
  1. Read `ExistingRoles` → `Vec<Symbol>`. If archived/absent, fall back to the
     three known operational roles (`KEEPER`, `REVENUE`, `ORACLE`).
  2. For each role: read `RoleAccountsCount(role)` → `u32` count.
  3. For `index in 0..count`: read `RoleAccounts({role, index})`; its value is the
     holder `Address`. Build `HasRole(holder, role)`.
  4. Add to the keep-alive set: `ExistingRoles`, each `RoleAccountsCount`, each
     `RoleAccounts`, each `HasRole`. These fold into `snapshot.persistent_entries`
     so existing planning + metrics see them.
- `Admin`/owner are instance-tier → already covered by the controller-instance
  bump; no change.

### 2. Classification (`policy.rs`)

Replace `needs_bump` with an explicit decision over `(live_until, value_present,
current, safety)`:

- `Extend` — value present, `live_until` present, `live_until > current`, and
  `live_until - current < safety`. (live, inside margin)
- `Restore` — value present, `live_until` present, `live_until <= current`.
  (archived, restorable)
- `Skip` — value absent (never written / evicted), or healthy (`>= safety`
  headroom), or no TTL.

### 3. Op builders + planning (`stellar/restore.rs`, `scheduler/tasks.rs`)

- New `restore_footprint(read_write_keys)` builder:
  `OperationBody::RestoreFootprint(RestoreFootprintOp{ext: V0})` with the keys in
  the seed `SorobanTransactionData.resources.footprint.read_write`. New `TxKind::RestoreFootprint`.
- `plan_extends` (existing) → `Extend` entries → `ExtendFootprintTtl` (read-only),
  chunked.
- `plan_restores` (new) → `Restore` entries → `RestoreFootprint` (read-write),
  chunked.
- Soroban permits one Soroban op per tx, so restore and extend are separate txs.

### 4. Self-healing loop — always on (`scheduler/mod.rs`)

Each TTL tick:
1. Drive restore jobs (revives archived entries → live at network-min TTL).
2. Drive extend jobs for in-margin entries **and** for the keys just restored, so
   they reach the ~31-day cap the same tick (avoids a re-archival race on the
   network-min TTL). Restores first, then extends. Bounded by `max_txs_per_tick`.

The follow-up extend of restored keys is built in the driver from the set of keys
whose restore succeeded; in dry-run it is reported but not simulated (an extend
over a still-archived entry would fail simulation pre-restore).

### 5. Dry-run actually simulates (`tx.rs`, `scheduler/mod.rs`)

Refactor `submit_with_sim` into:
- `simulate(ctx, job, dry_run) -> SimReport` — build envelope (source = signer
  pubkey; seq = real sequence when submitting, `0` in dry-run so **no funded
  signer is needed**), call `simulate_transaction_envelope`, return
  `Rejected(reason)` or `Ok{resource_fee, ro, rw}`.
- `submit_with_sim` = `simulate` + (on `Ok`) the existing patch/sign/submit tail.

Dry-run path in `drive_jobs`: for every planned job call `simulate` and log
`sim ok (kind=…, resource_fee=…, rw=N)` or `sim REJECTED: <reason>`; submit
nothing. This turns `--dry-run` from "log intentions" into "simulate intentions"
— strictly more useful; note it now calls the RPC simulate endpoint each tick.

### 6. Metrics

- `keeper_jobs_planned{loop, kind}` already exists; add `kind` label values
  `restore`.
- New gauge `keeper_entries_archived` (count of `Restore`-classified entries per
  tick) so the archived backlog is observable.

## Testing

- **Unit:** role-key ScVal encoding round-trips (mirror `keys.rs` tests);
  classify matrix (live-healthy / live-in-margin / archived / absent / no-ttl);
  `plan_restores` chunking; `restore_footprint` op + read-write footprint shape.
- **Live dry-run (testnet):** restore sims **PASS** for the 6 expired role entries
  + flash-receiver instance + wasm, with reported rent fees; extend sims pass for
  in-margin entries; any fully-evicted entry surfaces as `sim REJECTED`.
  `inspect_ttls` provides the before/after per-key view.

## Verification bar

`cargo check`, `cargo clippy -- -D warnings`, `cargo test` (keeper workspace), plus
a live `--dry-run` against `config/testnet.yaml` showing restore simulations
passing for the currently-expired keys.

## Risks / call-outs

- `--dry-run` semantics change (now simulates). Documented in README + here.
- Always-on restore pays rent whenever something is archived; bounded by
  `max_txs_per_tick`. Acceptable per operator choice (self-healing).
- Restore cannot revive fully-evicted entries; detected (not silently skipped) via
  `sim REJECTED`.
