# TTL Keeper Service — Design Spec

**Date:** 2026-05-28
**Author:** Mihai (with Claude)
**Status:** Draft — awaiting review

## Goal

Stand up an off-chain Rust service (`keeper-bot`) that keeps the protocol's
Soroban storage alive. The service signs and submits transactions from a single
admin account fetched at boot from Azure Key Vault via `mx-keyvault`. It runs
in Docker, one image, one binary, two containers (testnet, mainnet).

The service is responsible for:

1. **Storage TTL keepalive** — invoking the controller's existing keeper
   endpoints (`keepalive_shared_state`, `keepalive_pools`, `keepalive_accounts`)
   before any shared, instance, or per-account persistent entry expires.
2. **Wasm code TTL extension** — extending the on-chain WASM code entries
   (controller, pool template, flash-loan receiver) via raw
   `ExtendFootprintTTL` operations. Contracts cannot bump their own code
   entries from inside.
3. **Index refresh sweep** — calling `update_indexes(assets)` on a slower
   cadence so pool interest accrual stays current.

## Out of Scope (v1)

- Liquidations, bad-debt cleanup, threshold propagation
  (`update_account_threshold`). Roadmap for v2.
- Oracle configuration (ORACLE role) and revenue claims (REVENUE role) — both
  out, handled today by ops scripts.
- Multi-signer key sharding or HSM integration.

## Architecture

### Workspace placement

A new, **independent Cargo workspace** at `services/keeper/`. It does **not**
join the contracts workspace at the repo root.

Rationale: the contract workspace pins `panic = "abort"`,
`opt-level = "z"`, `lto = "fat"`, `wasm32v1-none` targets, and pulls only
`no_std` `soroban-sdk`. A tokio async service needs the opposite — `std`,
default panics, host target, dev/release split, and heavy crates (tokio,
hyper, prometheus). Mixing them adds friction with no benefit.

### Crate layout

```
services/keeper/
├── Cargo.toml                # workspace + bin package
├── Dockerfile
├── docker-compose.example.yaml
├── README.md
├── config/
│   ├── testnet.yaml
│   └── mainnet.yaml
└── src/
    ├── main.rs               # tokio runtime, signal handling, axum bind
    ├── config.rs             # serde_yaml load + validation
    ├── signer/
    │   ├── mod.rs            # Signer trait + Ed25519 backend
    │   ├── mnemonic.rs       # BIP-39 → SLIP-0010 m/44'/148'/0'
    │   └── vault.rs          # mx-keyvault fetch + boot-time decode
    ├── stellar/
    │   ├── mod.rs
    │   ├── client.rs         # thin wrapper over stellar-rpc-client::Client
    │   ├── tx.rs             # build → simulate → patch → sign → submit
    │   ├── ttl.rs            # ExtendFootprintTTL op builder
    │   └── invoke.rs         # InvokeHostFunction op builder
    ├── keys.rs               # ControllerKey ScVal encoding (see below)
    ├── discovery.rs          # scan controller state for live entries + TTLs
    ├── scheduler/
    │   ├── mod.rs            # tick loop + job dispatch
    │   ├── tasks.rs          # KeepalivePools, KeepaliveShared, …
    │   └── budget.rs         # per-tick caps (max txs, max accounts/batch)
    ├── policy.rs             # bump decision: live_until < now + safety_margin
    └── metrics.rs            # Prometheus collectors + axum /health + /metrics
```

Binary name: `keeper-bot`.

### Stellar SDK stack (pinned)

| Crate | Version | Role |
|-------|---------|------|
| `stellar-rpc-client` | `=26.0.0` | Official SDF RPC client — `get_ledger_entries`, `simulate_transaction_envelope`, `send_transaction_polling`, `get_account`. |
| `stellar-xdr` | `=26.0.1` | XDR types — `LedgerKey`, `LedgerEntryData`, `Operation`, `Transaction`, `SorobanTransactionData`, `ExtendFootprintTtlOp`, `InvokeHostFunctionOp`, `ScVal`. |
| `stellar-strkey` | `^0.0.13` | Encode/decode `G…` account IDs and `C…` contract IDs. |
| `ed25519-dalek` | `^2` | Ed25519 signatures over the transaction hash. |
| `bip39` | `^2.2` | Mnemonic → seed (matches mx-chain-rust workspace pin). |
| `slip10_ed25519` or hand-rolled HKDF chain | matched to `hkdf 0.12` | Derive `m/44'/148'/0'` per Stellar SEP-0005. |

Higher-level wrappers (`soroban-rs`, `soroban-client`) are **not** adopted in
v1. The two operations we issue (`InvokeHostFunction`, `ExtendFootprintTTL`)
are short and well-documented in raw XDR. Adding a wrapper introduces an
opinion to keep in lockstep with the SDF stack — easier to drop in later if
we outgrow the raw approach.

### Common type sharing (`ControllerKey` encoding)

**Plan A (preferred):** `services/keeper/src/keys.rs` constructs `ScVal::Vec`
values directly via `stellar-xdr`, mirroring how `soroban-sdk`'s
`#[contracttype]` macro serializes the on-chain `ControllerKey` enum
(`ScVal::Vec([ScVal::Symbol("AccountMeta"), ScVal::U64(id)])`, etc.).

**Why not path-dep `common`:** `common` is `#![no_std]` and pulls
`soroban-sdk` v26. soroban-sdk technically compiles for the host target but
brings substantial transitives into an async service binary and forces the
keeper to track every contract-side refactor of `ControllerKey`. The
encoding contract is small (~9 variants) and stable.

**Risk mitigation:** a single Rust test loads a known ledger entry from
testnet by name (e.g. `PoolsList`) and asserts our encoding produces a
match. Boot-time discovery also verifies non-empty `PoolsList`, so a wrong
encoding fails loudly within seconds of startup.

If keys encoding becomes a churn point, swap to path-dep `common` later.

### mx-keyvault sourcing

`mx-keyvault` lives at
`/home/truststaking/actions-runner/_work/mx-chain-rust/mx-chain-rust/crates/mx-keyvault`.
A `path = "…"` Cargo dep won't survive a Docker build — the build context
can't reach it.

**Decision:** depend via **git**, e.g.

```toml
mx-keyvault = { git = "https://github.com/multiversx/mx-chain-rust.git", tag = "mx-keyvault-v0.1.0", default-features = false, features = ["keyvault"] }
```

If the repo is private / not yet tagged, fallback: vendor under
`services/keeper/vendor/mx-keyvault` and ship the source in tree (open
decision below).

**Caveat to surface:** `mx-keyvault`'s README claims Managed Identity
support, but `lib.rs` only wires `ClientSecretCredential` and
`DeveloperToolsCredential`. Production deployment must inject
`AZURE_TENANT_ID`, `AZURE_CLIENT_ID`, `AZURE_CLIENT_SECRET`. There is no
true MI path today.

## Discovery flow (per TTL tick)

1. `Client::get_latest_ledger()` → `current_ledger`.
2. Batch read controller seed entries:
   `get_ledger_entries([PoolsList, AccountNonce, Aggregator, PoolTemplate])`.
3. For each asset in `PoolsList`, batch read:
   `[Market(asset), IsolatedDebt(asset)]` plus the pool's instance entry
   (resolved via `get_contract_instance(pool_addr)`).
4. For account ids `1..=nonce`, chunk reads (~100 keys per RPC call):
   `[AccountMeta(id), SupplyPositions(id), BorrowPositions(id)]`.
5. For each returned `LedgerEntry`, read `live_until_ledger_seq`. The bump
   decision is in `policy.rs`:
   ```
   bump_if  live_until - current_ledger < safety_margin_ledgers
   ```
   `safety_margin_days` (default 14) × `ONE_DAY_LEDGERS` (17 280) =
   ~241 920 ledgers.
6. Group results into work items by execution mechanism (next section).

## Execution flow

The keeper translates the discovery output into four kinds of jobs:

| Job | On-chain call | Footprint |
|-----|--------------|-----------|
| Bump per-asset shared state | `controller.keepalive_shared_state(assets[≤K])` | Read/write resolved by simulation |
| Bump pool instance / state via inner `keepalive()` | `controller.keepalive_pools(assets[≤K])` | Resolved by simulation |
| Bump per-account triplet | `controller.keepalive_accounts(ids[≤N])` | Resolved by simulation |
| Bump wasm code entries | raw `ExtendFootprintTTL` (no invoke) | `[ContractCode{controller_hash}, ContractCode{pool_hash}, ContractCode{receiver_hash}]` |

Chunk sizes K, N start at K=20, N=50 and are configurable. They are sized
to leave headroom under the Soroban budget; verified with the existing
`fuzz_budget_metering.rs` test as a reference.

**Per-tx pipeline (`stellar/tx.rs`):**

```
1. account = client.get_account(signer_address)            // seq
2. envelope = build_envelope(op, fee=base_fee, soroban_data=empty)
3. sim     = client.simulate_transaction_envelope(envelope, AuthMode::Enforce)
4. final_envelope = patch(envelope, sim.transaction_data, sim.min_resource_fee)
5. sign(final_envelope, signer, network_passphrase)
6. resp   = client.send_transaction_polling(final_envelope) (timeout 60s)
7. record metrics, on retriable error → see retry policy
```

### Sequence number + retry policy

Single mutating worker holds the source account's sequence. It refreshes
the sequence from RPC at boot and whenever a tx returns a sequence error.

| Failure | Action |
|---------|--------|
| `TX_BAD_SEQ` | Refresh sequence from RPC, retry once. If still bad, log and back off 60 s. |
| `TX_TOO_LATE` | Treat as `MAYBE_LANDED`. Wait `min_ledger_seq + 2` ledgers, then `get_transaction(hash)`. If `NOT_FOUND`, refresh sequence and retry. If `SUCCESS`, advance. |
| `send_transaction_polling` timeout (no terminal status) | Same as `TX_TOO_LATE` — never advance sequence locally, always reconcile via `get_transaction`. |
| Simulation `error` | Skip the job, log, increment `keeper_sim_failures_total{kind, reason}`. Do not consume sequence. |
| HTTP 5xx | Exponential back-off (1, 2, 4, 8 s, cap 30 s). |
| Other terminal failure | Mark tick failed, emit metric, continue to next tick (do not crash). |

The submitter never enqueues more than one in-flight tx per source. This
sidesteps the multi-pending-tx complexity that the SDF SDK does not handle
cleanly.

## Signer / key vault flow

1. **Boot** (synchronous, before scheduler starts):
   - Read config; require `keyvault.url` and `keyvault.secret_name`.
   - `KeyVaultClient::new(url)?.fetch_secret(name).await?` → mnemonic string.
   - `bip39::Mnemonic::parse_normalized(&mnemonic)?.to_seed("")` → 64-byte seed.
   - SLIP-0010 ed25519 derive `m/44'/148'/0'` (SEP-0005).
   - Wrap secret key in an `Ed25519Signer { secret, public }`.
2. **Boot safety check** (synchronous):
   - Derive `G…` address from `public`.
   - Simulate `controller.keepalive_pools(empty Vec<Address>)` from this
     source. If simulation rejects with `not_authorized` /
     `unauthorized_keeper`, **panic at boot** with a clear message
     pointing the operator to grant the KEEPER role.
   - On success, log the signer address and the KEEPER auth check passed.
3. **Per-tx**:
   - The signer's only operation is signing the 32-byte tx hash over
     `network_passphrase` (Stellar standard). Secret key never leaves
     process memory.

## Operational safety

- **`--dry-run` flag**: simulate every planned tx, log decisions, never
  submit. Useful for testnet validation and post-deploy smoke checks.
- **Per-tick budget cap**: `max_txs_per_tick` (default 50). When hit, log
  remaining work and skip — the next tick picks up.
- **Health endpoint (`/health`)**: returns 200 once boot safety check passed
  and at least one discovery tick completed without error.
- **Metrics endpoint (`/metrics`)**: Prometheus exposition. Key series:
  - `keeper_txs_total{kind, status}`
  - `keeper_bumps_planned_total{tier}`
  - `keeper_bumps_submitted_total{tier}`
  - `keeper_sim_failures_total{kind, reason}`
  - `keeper_rpc_latency_seconds_bucket{method}`
  - `keeper_signer_balance_stroops`
  - `keeper_account_nonce` (gauge)
  - `keeper_pools_listed` (gauge)
- **Graceful shutdown**: SIGTERM / SIGINT cancels in-flight ticks and
  waits up to 30 s for the active tx (if any) to reach a terminal state.

## Config (YAML)

`services/keeper/config/testnet.yaml`:

```yaml
network: testnet
rpc:
  url: https://soroban-testnet.stellar.org
  passphrase: "Test SDF Network ; September 2015"
  timeout_seconds: 30
contracts:
  controller: CBSCWXCIAASFR2F2332D2I7C6VWUJZKUW4ONOZR2LZ32KOZ5UZVNJ3LA
  pool_wasm_hash: a1e7db9b32626c8d4c57343c50407956ea1b642054bf6aee0a613da06359a6fa
  flash_loan_receiver: CCYDZ6SLHGZKBJF3MNKRK2QPITSVTHL5NYWKWWPMNSOTW4HHCK32JNLZ
keyvault:
  url: https://my-vault.vault.azure.net
  secret_name: keeper-mnemonic-testnet
signer:
  derivation_path: "m/44'/148'/0'"
fees:
  base_fee_stroops: 100
  resource_fee_multiplier: 1.20
schedule:
  ttl_tick_seconds: 21600          # 6 h
  index_tick_seconds: 3600         # 1 h
  ttl_safety_margin_days: 14
  account_chunk: 50
  asset_chunk: 20
  max_txs_per_tick: 50
metrics:
  bind: 0.0.0.0:9090
log:
  level: info
  format: json
```

The mainnet copy differs only in contract IDs, RPC URL, passphrase, and
`secret_name`.

## Docker

Multi-stage Dockerfile:

```dockerfile
FROM rust:1.84-bookworm AS builder
WORKDIR /build
COPY services/keeper /build
RUN --mount=type=cache,target=/root/.cargo/registry \
    --mount=type=cache,target=/build/target \
    cargo build --release --bin keeper-bot && \
    cp target/release/keeper-bot /keeper-bot

FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=builder /keeper-bot /usr/local/bin/keeper-bot
COPY services/keeper/config /etc/keeper
USER nonroot
ENV KEEPER_NETWORK=testnet
ENTRYPOINT ["/usr/local/bin/keeper-bot"]
CMD ["--config", "/etc/keeper/${KEEPER_NETWORK}.yaml"]
```

`docker-compose.example.yaml` shows side-by-side testnet+mainnet containers
binding `:9091`/`:9090` and injecting Azure env vars + per-network log
levels.

## Open decisions (resolve on review)

1. **mx-keyvault sourcing**: git tag preferred. If
   `multiversx/mx-chain-rust` doesn't yet publish a usable tag for
   `mx-keyvault`, fall back to `services/keeper/vendor/mx-keyvault/`?
2. **Crate name**: `keeper-bot` vs `ttl-keeper` vs `xoxno-lending-keeper`.
   Default in this spec: `keeper-bot`.
3. **Scheduler**: hand-rolled `tokio::time::interval` ticks (zero deps) vs
   `tokio-cron-scheduler` (cron strings, more flexible). Default: hand-rolled
   — simpler, the cadences are fixed intervals.
4. **Telemetry export**: Prometheus pull only, or also Azure Monitor /
   OpenTelemetry push? Default: Prometheus pull only in v1.
5. **Confirm `ContractCode` is the only entry type needing raw
   `ExtendFootprintTTL`** (advisor read of Soroban semantics — to be
   verified empirically against testnet during v1 dev with
   `get_ledger_entries` on one entry before/after a keepalive call).
