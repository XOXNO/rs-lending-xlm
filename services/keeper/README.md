# XOXNO Lending Keeper

Soroban contract storage has time-to-live (TTL). When TTL lapses, entries
archive and must be restored before contract calls can use them again. This
service keeps XOXNO Lending storage, instances, and WASM code entries alive.

`services/keeper` is a separate Rust workspace.

## What it does

- **TTL loop.** Each TTL tick discovers the storage surface in
  [Coverage](#coverage). It extends each discovered entry whose remaining TTL
  is below `schedule.ttl_safety_margin_days`. It restores each discovered entry
  whose TTL has lapsed.
- **Index loop** (optional, off by default). It calls
  `controller.update_indexes(caller, Vec<HubAssetKey>)`. See
  [Index refresh](#index-refresh).
- **Metrics.** It serves Prometheus metrics and a liveness probe. See
  [Metrics](#metrics).

## Quick start

### Local build

```bash
cd services/keeper
cargo check
cargo test
cargo build --release
```

Local development with a dev mnemonic (never a funded key):

```bash
KEEPER_MNEMONIC="<dev mnemonic>" cargo run --release -- \
  --config config/testnet-fast.yaml \
  --dry-run \
  --skip-role-check
```

`testnet-fast.yaml` shortens tick cadence so a short run observes discovery and
planning. `inspect_ttls` prints the discovered surface and per-class counts for
read-only audit.

### Docker

Every dependency is public. The build needs no secrets. BuildKit is still
required, because the Dockerfile uses cargo cache mounts:

```bash
DOCKER_BUILDKIT=1 docker build -t keeper-bot:latest services/keeper
```

> **Caution: the default image targets mainnet.** The image sets
> `KEEPER_CONFIG=/etc/keeper/mainnet.yaml` and `RUST_LOG=info`. A container
> started without its own `KEEPER_CONFIG` runs against mainnet and spends
> fees from the mainnet signer. Set `KEEPER_CONFIG` for every testnet
> container, as the example Compose file does.

The example Compose file publishes testnet on host port `9091` and mainnet on
host port `9090`:

```bash
docker compose -f services/keeper/docker-compose.example.yaml up -d
```

## Configuration

### Markets

Use `contracts.markets` for current protocol storage keys:

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

`contracts.market_assets` is a shorthand that maps each entry to
`hub_id = 1`. Prefer `contracts.markets`, because pool storage keys are encoded
as `HubAssetKey { hub_id, asset }`.

### Index refresh

The loop is disabled by default:

```yaml
schedule:
  enable_index_refresh: false
```

- The keeper signer is the transaction source and the `caller`.
- The controller calls `caller.require_auth()` and checks no role.
- The operation carries one `SorobanAuthorizationEntry` with source-account
  credentials whose root invocation is this same call. That satisfies the
  check.
- When the loop is enabled, boot simulates `update_indexes` with an empty asset
  list.
- If the simulation fails, boot logs the error at `error` on target
  `keeper.boot`, turns index refresh off and starts the TTL loop as usual.
- `--skip-role-check` skips that simulation.

### RPC endpoints and failover

`rpc.url` takes one endpoint or a list, under either spelling:

```yaml
rpc:
  url: https://primary.example          # single-endpoint form
```

```yaml
rpc:
  urls:                                  # preference order: primary first
    - https://primary.example
    - https://fallback.example
```

- Reads and simulations (`get_latest_ledger`, `get_account`,
  `get_full_ledger_entries`, `get_contract_instance`,
  `simulate_transaction_envelope`) retry down the list until one endpoint
  answers.
- The endpoint that answers becomes the active one. The submission that
  follows a read and a simulation goes to the same node. Later requests start
  there rather than walking the dead primary again.
- Transaction submission does not fail over. A send that fails after the
  network accepted the transaction must not be replayed on a node that has not
  seen it yet. The job fails and the next tick rebuilds it.
- A failover logs `RPC failover` at `warn` on target `keeper.rpc`. A total
  outage fails the tick with `... failed on all N RPC endpoints`.

### CLI flags and environment

Every `keeper-bot` flag has an environment fallback.

| Flag | Env var | Default | Required |
| --- | --- | --- | --- |
| `--config`, `-c` | `KEEPER_CONFIG` | `/etc/keeper/testnet.yaml` | no |
| `--dry-run` | `KEEPER_DRY_RUN` | `false` | no |
| `--mnemonic` | `KEEPER_MNEMONIC` | none | no; without it the keeper loads the signer from Key Vault |
| `--skip-role-check` | `KEEPER_SKIP_ROLE_CHECK` | `false` | no |

The network name comes from `network` in the YAML config, which is selected by
`--config` or `KEEPER_CONFIG`. There is no separate network environment variable.

`RUST_LOG` overrides `log.level` from the YAML config when it is set and parses
as a filter directive. Otherwise `log.level` applies. A value that does not
parse on either side falls back to `info,keeper=debug` rather than failing
startup. The sibling `lending-exporter` also lets a `RUST_LOG` that parses
override `log.level`, but it has no fixed fallback filter.

`prepay_rent` reads one more variable:

| Env var | Meaning | Default | Required |
| --- | --- | --- | --- |
| `PREPAY_SECRET` | `S...` seed that funds and signs the prepay transactions. The variable name itself is `--secret-env`. | name defaults to `PREPAY_SECRET`; the value has no default | yes, also with `--dry-run` |

### Optional KMS backend: Azure Key Vault

When `--mnemonic` is not set, the keeper reads its mnemonic from an Azure Key
Vault secret. It derives the signer at `signer.derivation_path`. The `keyvault`
block is required in every config file, also when you pass `--mnemonic`:

```yaml
keyvault:
  url: https://<your-vault-name>.vault.azure.net
  secret_name: <your-secret-name>   # secret value: a BIP-39 mnemonic

signer:
  derivation_path: "m/44'/148'/<index>'"
```

Dry run against testnet with Key Vault credentials:

```bash
AZURE_TENANT_ID=<tenant-id> AZURE_CLIENT_ID=<client-id> AZURE_CLIENT_SECRET=<client-secret> \
  cargo run --release -- --config config/testnet-fast.yaml --dry-run
```

The `mx-keyvault` crate reads the credentials:

- When all three `AZURE_*` credential variables are set, it uses the
  client-secret credential.
- Otherwise it falls back to the Azure CLI login, then the Azure Developer CLI
  login.
- The distroless image contains neither CLI. A container needs all three
  variables.

| Env var | Meaning | Default | Required |
| --- | --- | --- | --- |
| `AZURE_TENANT_ID` | Azure tenant for the client-secret credential | none | yes, unless a CLI login is used |
| `AZURE_CLIENT_ID` | Azure client id | none | yes, unless a CLI login is used |
| `AZURE_CLIENT_SECRET` | Azure client secret | none | yes, unless a CLI login is used |
| `AZURE_IDENTITY_DISABLE_MANAGED_IDENTITY_CREDENTIAL` | `1` or `true` (any case) forbids the CLI fallback, so a missing credential variable fails the boot | unset (fallback allowed) | no |

## Coverage

Each TTL tick discovers and renews these classes. Notes in brackets point to
[Coverage notes](#coverage-notes).

| Class | Tier | Source | Renewed |
| --- | --- | --- | --- |
| Controller instance [1] | instance | configured controller | yes |
| Price-aggregator `Oracle(PriceKey)`: token and `Ref` rows [2] | persistent | the aggregator's own `OracleKeys` index, falling back to configured markets | yes, when `contracts.price_aggregator` is set |
| Controller `Hub(id)` / `Spoke(id)` | persistent | `1..=LastHubId` / `1..=LastSpokeId` | yes |
| Controller `SpokeAsset` / `SpokeUsage` / `SpokeFlagsEpoch` [3] | persistent | `1..=LastSpokeId` × configured markets | yes, in the `hub_spoke` metrics group |
| Controller `BlendPoolAllowed` / `PositionManager` [4] | persistent | `contracts.blend_pools` / `contracts.position_managers` | yes, when listed, in the `hub_spoke` metrics group |
| Account state (`AccountMeta` / `SupplyPositions` / `BorrowPositions` / `Delegates`) [5] | persistent | position-NFT counter scan | yes |
| Account ownership (`Owner(token_id)` on the position NFT) [5] [6] | persistent | position-NFT counter scan | yes, in the `per_user` metrics group |
| Controller access-control keys (`ExistingRoles`, `RoleAccountsCount`, `RoleAccounts`, `HasRole`, `RoleAdmin`) | persistent | `ExistingRoles` | yes, when present |
| Pool `Params/State(HubAssetKey)` | persistent | configured markets | yes |
| Governance instance [7] | instance | configured governance | yes, when configured |
| Governance role keys [7] | persistent | governance `ExistingRoles` | yes, when configured |
| Pool / position-NFT / flash-loan receiver / price-aggregator instances | instance | instance reads | yes; receiver and price-aggregator when configured |
| WASM code: controller, configured `pool_wasm_hash`, live pool, position NFT, flash-loan receiver, price aggregator | code | instance reads | yes; receiver and price aggregator when configured |
| XOXNO oracle adapter instance, WASM, and persistent asset, feed and signer keys | instance / code / persistent | configured `xoxno_oracle_adapter` | yes, when configured |
| Dependencies the protocol reads through, and their WASM code [8] | instance / code | `contracts.redstone_adapter`, `contracts.swap_router`, `contracts.extra_instances` | yes, when configured |
| Timelock `OperationLedger(BytesN<32>)` [7] | persistent | event-only | no, documented gap |
| Temporary keys | temporary | n/a | no, expire by design |

### Coverage notes

1. The controller instance entry covers instance-tier keys such as pool
   address, aggregators, accumulator, spoke/hub counters, and position limits.
2. The oracle row set comes from the aggregator's `OracleKeys` instance index.
   It covers `Ref` rows such as the `Ref("BTC")` reference, which no list of
   market addresses can produce.
   - If the aggregator instance is unreadable or has no index, the scan falls
     back to the configured markets rather than renewing nothing.
   - A malformed index fails the tick.
3. Spoke/market pairs that are not listed read back `never_created` and plan
   nothing.
4. The contract keeps no index of Blend pools or position managers, so you
   maintain both lists by hand. Add an address when governance runs
   `approve_blend_pool` or `set_position_manager`.
5. Per-user scan:
   - Account ids are position-NFT token ids in `1..=max_account_id`.
     `max_account_id` is one below the NFT's sequential counter.
   - Each discovery pass (boot, TTL tick, index tick) scans a window of at most
     `schedule.max_accounts_scan` ids (default 50,000).
   - Only a TTL tick moves the window forward, and it wraps. A full cycle takes
     `ceil(max_account_id / max_accounts_scan)` TTL ticks.
   - The window starts again at id 1 when the process restarts.
   - Set `schedule.scan_users` to `false` to turn the scan off.
6. OpenZeppelin extends `Owner` to 30 days and the controller extends account
   keys to 120 days. Unrenewed, `Owner` therefore archives first.
7. Governance:
   - The controller defines no access-control roles. Governance role keys are
     discovered from the governance `ExistingRoles` entry.
   - Governance defines the roles `PROPOSER`, `CANCELLER`, `ORACLE`, and
     `GUARDIAN` (`contracts/governance/src/access.rs`).
   - Governance stores `Controller`, `PriceAggregator`, ownable `Owner`,
     access-control `Admin`, and timelock `MinDelay` in instance storage. The
     governance instance bump covers them.
   - `RoleAdmin` is persistent storage in `stellar-access`. The keeper renews
     it with the other access-control keys.
   - See [Timelock operation entries](#timelock-operation-entries) for the gap.
8. Nothing in the protocol writes these dependencies, so nothing else renews
   them. The named dependencies appear as `redstone_adapter` and `swap_router`
   in Prometheus and Grafana. Unnamed extra instances keep their
   `other_<id prefix>` labels.

The contract rustdoc and
[`skills/xoxno-lending/SKILL.md`](../../skills/xoxno-lending/SKILL.md) describe
the controller/pool/governance boundary and the role model.

### Timelock operation entries

Timelock `OperationLedger(BytesN<32>)` keys are persistent but not enumerable
from contract storage, so the keeper skips them. Event tracking would be needed
to renew them directly.

- Cancel removes the entry.
- Execute keeps it: `stellar-governance` rewrites the entry to the
  `DONE_LEDGER` sentinel. An executed operation therefore leaves a permanent
  persistent entry that nothing renews.
- `is_operation_done` reads that entry. `execute` rejects a chained operation
  with `UnexecutedPredecessor` when its predecessor is not done.
- If the done-marker of a predecessor has archived, the chained operation is
  blocked until the marker is restored.
- Pending operations also need TTL coverage through their tier-specific delay
  and execution window. Becoming ready neither executes nor renews them.

The keeper does not renew these entries.

## Metrics

The keeper serves Prometheus metrics and a liveness probe on `metrics.bind`
(`0.0.0.0:9090` on mainnet):

    GET /metrics      Prometheus text exposition
    GET /health       liveness; returns ok after boot

Every series carries a constant `network` label from the config. Storage state
is published per `(contract, key group)`, never per ledger key. Account ids are
never reused, so a series per key would add a permanent label value for every
account ever opened. Grouping holds the series count flat.

| Metric | Type | Labels | Meaning |
| --- | --- | --- | --- |
| `keeper_entry_ttl_ledgers_min` | gauge | contract, group | Lowest remaining TTL in the group — the pacing item |
| `keeper_entries` | gauge | contract, group, state | Entry counts by `state`; see [Entry states](#entry-states) |
| `keeper_safety_margin_ledgers` | gauge | — | Headroom below which the keeper extends |
| `keeper_current_ledger` | gauge | — | Ledger the last tick observed |
| `keeper_last_tick_timestamp_seconds` | gauge | — | Unix time of the last completed discovery pass — how stale the storage gauges are |
| `keeper_sim_resource_fee_stroops` | gauge | kind | Simulated resource fee of the last job of this kind; see [Resource fee gauge](#resource-fee-gauge) |
| `keeper_entries_archived` | gauge | — | Entries the last TTL tick planned to restore: TTL lapsed, value still readable (the `expired` state) |
| `keeper_max_account_id` | gauge | — | Highest position-NFT token id minted, i.e. the largest account id that can exist. `0` when the NFT address or its counter cannot be read |
| `keeper_txs_total` | counter | kind, status | Keeper transactions by kind and outcome |
| `keeper_sim_failures_total` | counter | kind, reason | Simulation failures by kind and bucketed reason |
| `keeper_jobs_planned_total` | counter | loop | Jobs planned per loop tick |
| `keeper_tick_failed_total` | counter | loop | Tick failures per loop |

Divide a ledger count by `LEDGERS_PER_DAY` (17280) for days, or multiply by 5
for seconds.

### Entry states

- `live`: the entry exists and its TTL has not lapsed.
- `expired`: TTL lapsed, restorable. The RPC returns an archived or evicted
  entry with its value and live-until 0, so it counts as `expired` and the
  keeper restores it.
- `never_created`: the RPC returned no entry.

A group reading zero `live` and non-zero `never_created` means the keeper is
probing a key that does not exist. No other metric shows this: it looks like
"nothing to do". The "Key groups with nothing live" panel in
`ops/grafana-dashboard.json` and the `KeeperKeyGroupUnreachable` rule in
`ops/alerts.yml` watch for it.

### Resource fee gauge

`keeper_sim_resource_fee_stroops` is in stroops, before
`fees.resource_fee_multiplier`. A dry-run keeper sets it for every simulation
that passes. A submitting keeper sets it for every transaction that succeeds.

### Storage gauge freshness

The keeper publishes the storage gauges at boot and after the discovery pass of
every tick: each TTL tick (`schedule.ttl_tick_seconds`, 6h on mainnet) and, when
enabled, each index tick.

- The gauges can be up to one TTL interval old.
- If the boot pass fails, they stay empty until the first scheduled tick, one
  full interval after boot.

## Operations

- `--dry-run`: discovers and plans, then simulates planned extend/restore calls
  without submitting.
- `schedule.max_txs_per_tick`: caps transactions per tick.
- `rpc.timeout_seconds`: caps submission polling.
- SIGTERM/SIGINT: stops new ticks, then waits up to 30 seconds for the running
  tick and the metrics server to finish.

Alert on tick failures (`keeper_tick_failed_total`), tick age
(`keeper_last_tick_timestamp_seconds`) and `keeper_entries_archived`. A silent
keeper failure can become protocol downtime after TTL windows expire. Example
rules live in `ops/alerts.yml`.

### Extra binaries

- `inspect_ttls --config <path>` (env `KEEPER_CONFIG`, default
  `/etc/keeper/testnet.yaml`): read-only. Prints the discovered surface and
  per-class TTL counts. Submits nothing.
- `prepay_rent --config <path> [--secret-env PREPAY_SECRET] [--dry-run]`:
  spends funds.
  - It runs one discovery pass. It plans a restore for every lapsed entry and
    an extend for every live entry, whatever its TTL, with no per-tick cap. It
    submits them one by one.
  - The per-user scan covers at most `schedule.max_accounts_scan` ids.
  - The signer is built from the `S...` seed in `$PREPAY_SECRET` (or in the
    variable named by `--secret-env`), not from Key Vault.
  - `--config` (env `KEEPER_CONFIG`) has no default here.
  - Run `--dry-run` first: it prints the planned transaction counts and submits
    nothing.

```bash
PREPAY_SECRET=S... cargo run --release --bin prepay_rent -- \
  --config config/testnet-fast.yaml --dry-run
```

## Known limitations

- The per-user scan reads the position-NFT sequential counter, which counts ids
  ever minted, not live accounts. Burned ids are still scanned; their keys read
  back absent. Ids are never reused, so coverage is correct, but the scan cost
  grows with total accounts created rather than with accounts alive.
- The per-user scan window starts again at id 1 on every restart. Once
  `max_account_id` exceeds `schedule.max_accounts_scan`, a keeper that restarts
  more often than one full rotation keeps renewing the first windows only.
- A failed index preflight turns index refresh off for the life of the process,
  also when the cause was a transient RPC error. Only the `keeper.boot` error
  log shows it; restart the keeper to retry.
- Timelock `OperationLedger` entries are not renewed. See
  [Timelock operation entries](#timelock-operation-entries).

## Reference

### Layout

```text
services/keeper/
├── Cargo.toml
├── Dockerfile
├── docker-compose.example.yaml
├── config/
│   ├── testnet.yaml
│   ├── testnet-fast.yaml
│   └── mainnet.yaml
├── ops/
│   ├── alerts.yml
│   ├── grafana-dashboard.json
│   └── prometheus.example.yml
└── src/
    ├── main.rs
    ├── lib.rs
    ├── classify.rs
    ├── config.rs
    ├── discovery.rs
    ├── keys.rs
    ├── policy.rs
    ├── scheduler/
    ├── signer/
    ├── stellar/
    ├── metrics.rs
    └── bin/
```

### SDK stack

| Crate | Version |
| --- | --- |
| `stellar-rpc-client` | git rev `a44c2b6a` (resolves to 27.0.0) |
| `stellar-xdr` | `=28.0.0` |
| `stellar-strkey` | `^0.0.16` |
| `ed25519-dalek` | `^2` |
| `bip39` | `^2.2` |
| `mx-keyvault` | `0.1.0` (crates.io) |

The Stellar crates are pinned so passive dependency updates cannot change XDR or
RPC behavior silently.
