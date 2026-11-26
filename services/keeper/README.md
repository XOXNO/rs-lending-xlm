# XOXNO Lending Keeper

Soroban storage entries archive when their time-to-live (TTL) lapses and need a
restore before contract calls can use them. This service, a separate Rust
workspace, keeps XOXNO Lending storage, instances, and WASM code alive.

## What it does

- **TTL loop.** Each tick discovers the entries in [Coverage](#coverage), extends
  entries with TTL below `schedule.ttl_safety_margin_days`, and restores lapsed
  ones.
- **Index loop** (off by default). The loop calls
  `controller.update_indexes(caller, Vec<HubAssetKey>)`. See
  [Index refresh](#index-refresh).
- **[Metrics](#metrics).** The service serves metrics and a liveness probe.

## Quick start

### Local build

```bash
cd services/keeper
cargo check
cargo test
cargo build --release
```

For local development, use a dev mnemonic and never a funded key.
`testnet-fast.yaml` shortens ticks, so a short run shows discovery and planning:

```bash
KEEPER_MNEMONIC="<dev mnemonic>" cargo run --release -- \
  --config config/testnet-fast.yaml \
  --dry-run \
  --skip-role-check
```

### Docker

You need no secrets. BuildKit is required for the cargo cache mounts:

```bash
DOCKER_BUILDKIT=1 docker build -t keeper-bot:latest services/keeper
```

> **Caution.** The default image targets mainnet. It sets
> `KEEPER_CONFIG=/etc/keeper/mainnet.yaml` and `RUST_LOG=info`. A container
> without its own `KEEPER_CONFIG` spends mainnet signer fees. Set
> `KEEPER_CONFIG` for every testnet container, as the example Compose file does.

The example Compose file publishes testnet on host port `9091` and mainnet on `9090`:

```bash
docker compose -f services/keeper/docker-compose.example.yaml up -d
```

## Configuration

### Markets

```yaml
contracts:
  controller: C...
  pool_wasm_hash: "..."
  flash_loan_receiver: C...  # optional
  governance: C...           # optional
  price_aggregator: C...     # required when markets are configured
  redstone_adapter: C...     # optional; instance and WASM renewal
  swap_router: C...          # optional; instance and WASM renewal
  markets:
    - hub_id: 1
      asset: C...
  blend_pools: [C...]        # optional; approved Blend pools
  position_managers: [G...]  # optional; G... or C... managers
```

Use `contracts.markets`. Pool keys are `HubAssetKey { hub_id, asset }`. The
`contracts.market_assets` shorthand maps every entry to `hub_id = 1`.

### Index refresh

Index refresh is off by default (`schedule.enable_index_refresh: false`).

- The keeper signer is the transaction source and the `caller`. The controller
  calls `caller.require_auth()` and runs no role check. One
  `SorobanAuthorizationEntry` with source-account credentials rooted at the call
  satisfies `caller.require_auth()`.
- When index refresh is enabled, boot simulates `update_indexes` with an empty
  asset list. `--skip-role-check` skips this simulation. If the simulation
  fails, the keeper logs an `error` on target `keeper.boot`, turns index refresh
  off, and starts the TTL loop as usual.

### RPC endpoints and failover

Set `rpc.url` for one endpoint or `rpc.urls` for a list with the primary first:

```yaml
rpc:
  urls:
    - https://primary.example
    - https://fallback.example
```

- Reads and simulations (`get_latest_ledger`, `get_account`,
  `get_full_ledger_entries`, `get_contract_instance`,
  `simulate_transaction_envelope`) walk the list until one answers. That node
  becomes active, so the next submission and later requests start there.
- Submission never fails over. A send the network may have accepted must not
  replay on a node that has not seen it. The job fails and the next tick
  rebuilds it.
- Failover logs `RPC failover` at `warn` on `keeper.rpc`. A total outage fails
  the tick with `... failed on all N RPC endpoints`.

### CLI flags and environment

| Flag | Env var | Default | Required |
| --- | --- | --- | --- |
| `--config`, `-c` | `KEEPER_CONFIG` | `/etc/keeper/testnet.yaml` | no |
| `--dry-run` | `KEEPER_DRY_RUN` | `false` | no |
| `--mnemonic` | `KEEPER_MNEMONIC` | none | no; else signer from Key Vault |
| `--skip-role-check` | `KEEPER_SKIP_ROLE_CHECK` | `false` | no |

The network comes only from `network` in the YAML. No env var sets it. A
`RUST_LOG` that parses as a filter overrides `log.level`. If neither `RUST_LOG`
nor `log.level` parses, the filter is `info,keeper=debug` and startup does not
fail. `prepay_rent` also
reads `PREPAY_SECRET` ([Extra binaries](#extra-binaries)).

### Optional KMS backend: Azure Key Vault

Without `--mnemonic`, the keeper reads a BIP-39 mnemonic from a Key Vault
secret and derives the signer at `signer.derivation_path`. Every config needs
the `keyvault` block, even with `--mnemonic`:

```yaml
keyvault:
  url: https://<your-vault-name>.vault.azure.net
  secret_name: <your-secret-name>   # secret value: a BIP-39 mnemonic

signer:
  derivation_path: "m/44'/148'/<index>'"
```

```bash
AZURE_TENANT_ID=<tenant-id> AZURE_CLIENT_ID=<client-id> AZURE_CLIENT_SECRET=<client-secret> \
  cargo run --release -- --config config/testnet-fast.yaml --dry-run
```

`mx-keyvault` uses the client-secret credential when `AZURE_TENANT_ID`,
`AZURE_CLIENT_ID`, and `AZURE_CLIENT_SECRET` are all set (no defaults). If one is
missing, it falls back to the Azure CLI login, then the Azure Developer CLI
login. The distroless image has neither CLI, so containers need all three
variables. If you set `AZURE_IDENTITY_DISABLE_MANAGED_IDENTITY_CREDENTIAL` to
`1` or `true` (any case), the fallback is forbidden, so a missing variable fails
boot. If you leave it unset, the fallback is allowed.

## Coverage

Each TTL tick discovers and renews these classes. A `[n]` marker refers to
[Coverage notes](#coverage-notes). "If set" means renewed only when configured.

| Class | Tier | Source | Renewed |
| --- | --- | --- | --- |
| Controller instance [1] | instance | config | yes |
| Price-aggregator `Oracle(PriceKey)` token and `Ref` rows [2] | persistent | aggregator `OracleKeys` index, else configured markets | if `contracts.price_aggregator` set |
| Controller `Hub(id)` / `Spoke(id)` | persistent | `1..=LastHubId` / `1..=LastSpokeId` | yes |
| Controller `SpokeAsset` / `SpokeUsage` / `SpokeFlagsEpoch` [3] | persistent | `1..=LastSpokeId` × configured markets | yes (`hub_spoke` group) |
| Controller `BlendPoolAllowed` / `PositionManager` [4] | persistent | `contracts.blend_pools` / `contracts.position_managers` | if listed (`hub_spoke` group) |
| Account `AccountMeta` / `SupplyPositions` / `BorrowPositions` / `Delegates` [5] | persistent | position-NFT counter scan | yes |
| Position-NFT `Owner(token_id)` [5] [6] | persistent | position-NFT counter scan | yes (`per_user` group) |
| Controller access-control keys (`ExistingRoles`, `RoleAccountsCount`, `RoleAccounts`, `HasRole`, `RoleAdmin`) | persistent | `ExistingRoles` | if present |
| Pool `Params/State(HubAssetKey)` | persistent | configured markets | yes |
| Governance instance [7] | instance | config | if set |
| Governance role keys [7] | persistent | governance `ExistingRoles` | if set |
| Pool, position-NFT, flash-loan receiver, price-aggregator instances | instance | instance reads | yes; receiver and aggregator if set |
| WASM: controller, configured `pool_wasm_hash`, live pool, position NFT, flash-loan receiver, price aggregator | code | instance reads | yes; receiver and aggregator if set |
| XOXNO oracle adapter instance, WASM, persistent asset/feed/signer keys | instance / code / persistent | `xoxno_oracle_adapter` | if set |
| Read-through dependencies and their WASM [8] | instance / code | `contracts.redstone_adapter`, `contracts.swap_router`, `contracts.extra_instances` | if set |
| Timelock `OperationLedger(BytesN<32>)` [7] | persistent | event-only | no, documented gap |
| Temporary keys | temporary | n/a | no, expire by design |

### Coverage notes

1. The controller instance entry covers the instance-tier keys for the pool
   address, aggregators, accumulator, spoke and hub counters, and position
   limits.
2. The `OracleKeys` index yields `Ref` rows such as `Ref("BTC")` that no market
   list can produce. If the aggregator instance is unreadable or the index is
   missing, discovery falls back to configured markets. If the index is
   malformed, the tick fails.
3. Unlisted spoke and market pairs read `never_created` and plan nothing.
4. The contract keeps no index of these keys. Add the address by hand when
   governance runs `approve_blend_pool` or `set_position_manager`.
5. The per-user scan works as follows:
   - Account ids are position-NFT token ids `1..=max_account_id`, one below the
     NFT sequential counter.
   - Each discovery pass (boot, TTL tick, index tick) scans at most
     `schedule.max_accounts_scan` ids (default 50,000).
   - Only TTL ticks advance the window, and the window wraps. A cycle takes
     `ceil(max_account_id / max_accounts_scan)` TTL ticks. A restart resets the
     window to id 1 ([Known limitations](#known-limitations)).
   - `schedule.scan_users: false` disables the scan.
6. OpenZeppelin extends `Owner` to 30 days and the controller account keys to 120,
   so an unrenewed `Owner` archives first.
7. Governance notes:
   - The controller defines no access-control roles. Governance roles
     (`PROPOSER`, `CANCELLER`, `ORACLE`, `GUARDIAN`,
     defined in `contracts/governance/src/access.rs`) come from governance
     `ExistingRoles`.
   - The governance instance bump covers `Controller`, `PriceAggregator`,
     ownable `Owner`, access-control `Admin` and timelock `MinDelay`.
   - `RoleAdmin` is persistent in `stellar-access`. The keeper renews it with
     the other access-control keys.
   - The gap is described in
     [Timelock operation entries](#timelock-operation-entries).
8. Nothing in the protocol writes these, so nothing else renews them. The metric
   labels are `redstone_adapter`, `swap_router`, and `other_<id prefix>` for
   unnamed extras.

For the controller, pool, and governance boundary and the role model, see the
contract rustdoc and
[`skills/xoxno-lending/SKILL.md`](../../skills/xoxno-lending/SKILL.md).

### Timelock operation entries

`OperationLedger(BytesN<32>)` keys are persistent but not enumerable from
storage (renewal would need event tracking), so the keeper skips them.

- Cancel removes the entry. Execute rewrites it to the `stellar-governance`
  `DONE_LEDGER` sentinel. The sentinel is permanent and unrenewed.
- `is_operation_done` reads the entry. `execute` rejects a chained operation
  with `UnexecutedPredecessor` when the predecessor is not done. An archived
  predecessor marker blocks the chain until restored.
- Pending operations need TTL through their tier delay and execution window.
  Becoming ready does not execute or renew an operation.

## Metrics

The service serves metrics on `metrics.bind` (`0.0.0.0:9090` on mainnet):

    GET /metrics      Prometheus text exposition
    GET /health       liveness; returns ok after boot

Every series has a constant `network` label. Storage state is per
`(contract, key group)`, never per key. Account ids are never reused, so
per-key series would grow without bound.

| Metric | Type | Labels | Meaning |
| --- | --- | --- | --- |
| `keeper_entry_ttl_ledgers_min` | gauge | contract, group | Lowest remaining TTL in group (pacing item) |
| `keeper_entries` | gauge | contract, group, state | Entry counts by [state](#entry-states) |
| `keeper_safety_margin_ledgers` | gauge | — | Extend threshold |
| `keeper_current_ledger` | gauge | — | Ledger of last tick |
| `keeper_last_tick_timestamp_seconds` | gauge | — | Unix time of last completed discovery pass (storage gauge age) |
| `keeper_sim_resource_fee_stroops` | gauge | kind | Simulated resource fee of last job of kind ([details](#resource-fee-gauge)) |
| `keeper_entries_archived` | gauge | — | Entries the last TTL tick planned to restore (`expired`) |
| `keeper_max_account_id` | gauge | — | Highest position-NFT token id minted. `0` if NFT address or counter unreadable |
| `keeper_txs_total` | counter | kind, status | Transactions by kind and outcome |
| `keeper_sim_failures_total` | counter | kind, reason | Simulation failures by kind and bucketed reason |
| `keeper_jobs_planned_total` | counter | loop | Jobs planned per tick |
| `keeper_tick_failed_total` | counter | loop | Tick failures |

Divide ledgers by `LEDGERS_PER_DAY` (17280) to get days. Multiply ledgers by 5 to get seconds.

### Entry states

- `live`: the entry exists and its TTL has not lapsed.
- `expired`: the TTL has lapsed and the entry is restorable. The RPC returns the
  archived or evicted value with live-until 0, and the keeper restores it.
- `never_created`: the RPC returned no entry.

Zero `live` with non-zero `never_created` means the keeper probes a missing
key. That state otherwise looks like "nothing to do". The "Key groups with nothing
live" panel in `ops/grafana-dashboard.json` and the `KeeperKeyGroupUnreachable`
rule in `ops/alerts.yml` catch that state.

### Resource fee gauge

The gauge is in stroops, before `fees.resource_fee_multiplier` applies. The
keeper sets it after each passing simulation in dry-run mode and after each
successful transaction when it submits.

### Storage gauge freshness

The keeper publishes the storage gauges at boot and after each discovery pass.
A discovery pass runs on every TTL tick (`schedule.ttl_tick_seconds`, 6h on
mainnet) and, if enabled, on every index tick. The gauges are up to one TTL
interval old. If the boot pass fails, they stay empty until the first scheduled
tick.

## Operations

- `--dry-run` discovers entries, plans jobs, and simulates extend and restore.
  It submits nothing.
- `schedule.max_txs_per_tick` caps transactions per tick.
- `rpc.timeout_seconds` caps submission polling.
- On SIGTERM or SIGINT, the keeper starts no new ticks and waits up to 30 s for
  the running tick and the metrics server.

Alert on `keeper_tick_failed_total`, tick age
(`keeper_last_tick_timestamp_seconds`), and `keeper_entries_archived`. A silent
failure becomes protocol downtime once TTLs expire. Example rules are in
`ops/alerts.yml`.

### Extra binaries

- `inspect_ttls --config <path>` (env `KEEPER_CONFIG`, default
  `/etc/keeper/testnet.yaml`) is read-only. It prints the discovered entries and
  per-class TTL counts.
- `prepay_rent --config <path> [--secret-env PREPAY_SECRET] [--dry-run]`:
  spends funds.
  - The binary runs one discovery pass. It plans a restore for each lapsed
    entry and an extend for each live entry regardless of TTL, with no per-tick
    cap. It submits the jobs one by one. The per-user scan is capped at
    `schedule.max_accounts_scan` ids.
  - The signer is the `S...` seed in `PREPAY_SECRET` (or the variable that
    `--secret-env` names), not Key Vault. The seed is required even with
    `--dry-run`.
  - `--config` (env `KEEPER_CONFIG`) has no default.
  - Run `--dry-run` first. It prints planned transaction counts and submits
    nothing.

```bash
PREPAY_SECRET=S... cargo run --release --bin prepay_rent -- \
  --config config/testnet-fast.yaml --dry-run
```

## Known limitations

- The scan follows the counter of ids ever minted, so it scans burned ids
  (the keys read absent). The behavior is correct, since ids are never reused,
  but the cost grows with accounts created, not with accounts alive.
- Once `max_account_id` exceeds `schedule.max_accounts_scan`, a keeper that
  restarts more often than one rotation renews only the first windows.
- A failed index preflight disables index refresh for the process lifetime,
  even on a transient RPC error. Only the `keeper.boot` log shows the failure.
  Restart to retry.
- [Timelock operation entries](#timelock-operation-entries) are not renewed.

## Reference

### Layout

```text
services/keeper/
├── Cargo.toml, Dockerfile, docker-compose.example.yaml
├── config/  testnet.yaml, testnet-fast.yaml, mainnet.yaml
├── ops/     alerts.yml, grafana-dashboard.json, prometheus.example.yml
└── src/     main.rs, lib.rs, classify.rs, config.rs, discovery.rs, keys.rs,
             policy.rs, metrics.rs, scheduler/, signer/, stellar/, bin/
```

### SDK stack

Stellar crates are pinned so passive updates cannot silently change XDR or RPC
behavior.

| Crate | Version |
| --- | --- |
| `stellar-rpc-client` | git rev `a44c2b6a` (resolves to 27.0.0) |
| `stellar-xdr` | `=28.0.0` |
| `stellar-strkey` | `^0.0.16` |
| `ed25519-dalek` | `^2` |
| `bip39` | `^2.2` |
| `mx-keyvault` | `0.1.0` (crates.io) |
