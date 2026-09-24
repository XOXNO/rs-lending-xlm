# XOXNO Lending Keeper

Soroban contract storage has time-to-live (TTL). When TTL lapses, entries
archive and must be restored before contract calls can use them again. This
service keeps XOXNO Lending storage, instances, and WASM code entries alive. It
extends each discovered entry whose remaining TTL is below
`schedule.ttl_safety_margin_days`, and restores each discovered entry whose TTL
has lapsed.

`services/keeper` is a separate Rust workspace.

## Discovery Surface

Each TTL tick discovers:

- Controller instance entry. This covers instance-tier keys such as pool address,
  aggregators, accumulator, spoke/hub counters, and position limits.
- Price-aggregator persistent `Oracle(PriceKey)` rows when `contracts.price_aggregator`
  is set. The set comes from the aggregator's own `OracleKeys` instance index, so it
  covers `Ref` rows such as the `Ref("BTC")` reference, which no list of market
  addresses can produce. If the aggregator instance is unreadable or has no
  index, the scan falls back to the configured markets rather than renewing
  nothing. A malformed index fails the tick.
- Price-aggregator instance and WASM when `contracts.price_aggregator` is set.
- Controller persistent `Hub(id)` rows for `1..=LastHubId` and `Spoke(id)` rows
  for `1..=LastSpokeId`.
- Controller persistent `SpokeAsset(spoke, HubAssetKey)`,
  `SpokeUsage(spoke, HubAssetKey)` and `SpokeFlagsEpoch(spoke, HubAssetKey)` rows
  for every spoke in `1..=LastSpokeId` and every configured market. Pairs that
  are not listed read back `never_created` and plan nothing.
- Controller persistent `BlendPoolAllowed(pool)` rows for `contracts.blend_pools`
  and `PositionManager(address)` rows for `contracts.position_managers`. The
  contract keeps no index of either, so the lists are maintained by hand: add an
  address when governance runs `approve_blend_pool` or `set_position_manager`.
- Controller per-user persistent keys: `AccountMeta(id)`, `SupplyPositions(id)`,
  `BorrowPositions(id)`, `Delegates(id)`, plus the position-NFT `Owner(id)` key.
  Account ids are position-NFT token ids in `1..=max_account_id`, where
  `max_account_id` is one below the NFT's sequential counter. Each discovery
  pass (boot, TTL tick, index tick) scans a window of at most
  `schedule.max_accounts_scan` ids (default 50,000). Only a TTL tick moves the
  window forward, and it wraps, so a full cycle takes
  `ceil(max_account_id / max_accounts_scan)` TTL ticks. The window starts again
  at id 1 when the process restarts. Set `schedule.scan_users` to `false` to
  turn the scan off.
- Controller access-control persistent keys when present:
  `ExistingRoles`, `RoleAccountsCount`, `RoleAccounts`, `HasRole`, `RoleAdmin`.
- Governance instance and governance role-holder keys when `contracts.governance`
  is configured.
- Pool and position-NFT instances; the flash-loan receiver instance when
  configured.
- Controller WASM, configured `pool_wasm_hash`, live pool WASM, position-NFT
  WASM, and flash-loan receiver WASM.
- Pool persistent `Params(HubAssetKey)` and `State(HubAssetKey)` rows for
  configured markets.
- XOXNO oracle adapter instance, WASM, and persistent asset, feed and signer
  keys when `contracts.xoxno_oracle_adapter` is set.
- Each `contracts.extra_instances` instance and its WASM.

The controller defines no access-control roles. Governance role keys are
discovered from the governance `ExistingRoles` entry. Governance defines the
roles `PROPOSER`, `EXECUTOR`, `CANCELLER`, `ORACLE`, and `GUARDIAN`
(`contracts/governance/src/access.rs`). The controller/pool/governance boundary
and role model live in the contract rustdoc and
[`skills/xoxno-lending/SKILL.md`](../../skills/xoxno-lending/SKILL.md).

## Market Configuration

Use `contracts.markets` for current protocol storage keys:

```yaml
contracts:
  controller: C...
  pool_wasm_hash: "..."
  flash_loan_receiver: C...  # optional
  governance: C...           # optional
  price_aggregator: C...     # required when markets are configured
  markets:
    - hub_id: 1
      asset: C...
  blend_pools: [C...]        # optional; approved Blend pools
  position_managers: [G...]  # optional; G... or C... managers
```

`contracts.market_assets` is a legacy shorthand. Each entry maps to
`hub_id = 1`. Prefer `contracts.markets` because pool storage keys are encoded
as `HubAssetKey { hub_id, asset }`.

## Index Refresh

The optional index loop calls:

```text
controller.update_indexes(caller, Vec<HubAssetKey>)
```

The keeper signer is the transaction source and the `caller`. The controller
calls `caller.require_auth()` and checks no role. The operation carries one
`SorobanAuthorizationEntry` with source-account credentials whose root
invocation is this same call, which satisfies that check. When the loop is
enabled, boot simulates `update_indexes` with an empty asset list. If the
simulation fails, boot logs the error at `error` on target `keeper.boot`, turns
index refresh off and starts the TTL loop as usual. `--skip-role-check` skips
that simulation. The loop is disabled by default:

```yaml
schedule:
  enable_index_refresh: false
```

## Governance Notes

When `contracts.governance` is set, keeper also keeps governance alive.
Governance stores `Controller`, `PriceAggregator`, ownable `Owner`,
access-control `Admin`, and timelock `MinDelay` in instance storage, so the
governance instance bump covers them. `RoleAdmin` is persistent
(`stellar-access` `access_control/storage.rs:518-520`); the keeper renews it with
the other access-control keys.

Timelock `OperationLedger(BytesN<32>)` keys are persistent but not enumerable
from contract storage, so the keeper skips them. Event tracking would be needed
to renew them directly.

Cancel removes the entry. Execute keeps it: it rewrites the entry to the
`DONE_LEDGER` sentinel
(`stellar-governance` `timelock/storage.rs:341-342`, `:381-382`). An executed
operation therefore leaves a permanent persistent entry that nothing renews.
That entry is what `is_operation_done` reads, and `execute` rejects a chained
operation with `UnexecutedPredecessor` when its predecessor is not done
(`timelock/storage.rs:337-338`). If the done-marker of a predecessor has
archived, the chained operation is blocked until the marker is restored. Pending
operations also need TTL coverage through their tier-specific delay and execution
window; becoming ready neither executes nor renews them. The keeper does not
renew these entries.

## Coverage Table

| Class | Tier | Source | Renewed |
| --- | --- | --- | --- |
| Controller instance | instance | configured controller | yes |
| Price-aggregator `Oracle(PriceKey)`: token and `Ref` rows | persistent | the aggregator's own `OracleKeys` index, falling back to configured markets | yes |
| Controller `Hub(id)` / `Spoke(id)` | persistent | `LastHubId` / `LastSpokeId` | yes |
| Account state (`AccountMeta` / `SupplyPositions` / `BorrowPositions` / `Delegates`) | persistent | position-NFT counter scan | yes |
| Account ownership (`Owner(token_id)` on the position NFT) | persistent | position-NFT counter scan | yes, in the `per_user` metrics group. OpenZeppelin extends it to 30 days and the controller extends account keys to 120 days, so it archives first if unrenewed |
| Controller access-control keys | persistent | `ExistingRoles` | yes, when present |
| Controller `SpokeAsset` / `SpokeUsage` / `SpokeFlagsEpoch` | persistent | `1..=LastSpokeId` × configured markets | yes, in the `hub_spoke` metrics group |
| Controller `BlendPoolAllowed` / `PositionManager` | persistent | `contracts.blend_pools` / `contracts.position_managers` | yes, when listed, in the `hub_spoke` metrics group |
| Pool `Params/State(HubAssetKey)` | persistent | configured markets | yes |
| Governance instance | instance | configured governance | yes |
| Governance role keys | persistent | `ExistingRoles` | yes, when configured |
| Pool / position-NFT / receiver / price-aggregator instances and WASM code | instance / code | instance reads | yes |
| XOXNO oracle adapter instance, WASM and persistent keys | instance / code / persistent | configured `xoxno_oracle_adapter` | yes, when configured |
| Third-party instances the protocol reads through (`contracts.extra_instances`: RedStone adapter, swap router) and their WASM code | instance / code | configured list | yes, when configured — nothing in the protocol writes these, so nothing else renews them |
| Timelock `OperationLedger(BytesN<32>)` | persistent | event-only | no, documented gap |
| Temporary keys | temporary | n/a | no, expire by design |

## Metrics

The keeper serves Prometheus metrics and a liveness probe on `metrics.bind`
(`0.0.0.0:9090` on mainnet):

    GET /metrics      Prometheus text exposition
    GET /health       liveness

Every series carries a constant `network` label from the config. Storage state
is published per `(contract, key group)`, never per ledger key. Account ids are
never reused, so a series per key would add a permanent label value for every
account ever opened; grouping holds the series count flat.

| metric | labels | meaning |
| --- | --- | --- |
| `keeper_entry_ttl_ledgers_min` | contract, group | lowest remaining TTL in the group — the pacing item |
| `keeper_entries` | contract, group, state | entry counts; `state` is `live`, `expired` (TTL lapsed, restorable) or `never_created` (the RPC returned no entry). The RPC returns an archived or evicted entry with its value and live-until 0, so it counts as `expired` and the keeper restores it |
| `keeper_safety_margin_ledgers` | — | headroom below which the keeper extends |
| `keeper_current_ledger` | — | ledger the last tick observed |
| `keeper_last_tick_timestamp_seconds` | — | unix time of the last completed discovery pass — how stale everything above is |
| `keeper_sim_resource_fee_stroops` | kind | resource fee the simulation returned for the last job of this kind, in stroops, before `fees.resource_fee_multiplier`. A dry-run keeper sets it for every simulation that passes; a submitting keeper sets it for every transaction that succeeds |

Divide a ledger count by `LEDGERS_PER_DAY` (17280) for days, or multiply by 5
for seconds.

A group reading zero `live` and non-zero `never_created` means the keeper is
probing a key that does not exist. No other metric shows this: it looks like
"nothing to do". The "Key groups with nothing live" panel in
`ops/grafana-dashboard.json` and the `KeeperKeyGroupUnreachable` rule in
`ops/alerts.yml` watch for it.

The keeper publishes the storage gauges at boot and after the discovery pass of
every tick: each TTL tick (`schedule.ttl_tick_seconds`, 6h on mainnet) and, when
enabled, each index tick. The gauges can be up to one TTL interval old. If the
boot pass fails, they stay empty until the first scheduled tick, one full
interval after boot.

## Layout

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

## SDK Stack

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

## Local Build

```bash
cd services/keeper
cargo check
cargo test
cargo build --release
```

Dry run against testnet with Azure Key Vault:

```bash
AZURE_TENANT_ID=... AZURE_CLIENT_ID=... AZURE_CLIENT_SECRET=... \
  cargo run --release -- --config config/testnet-fast.yaml --dry-run
```

Local development without Azure credentials (use a dev mnemonic only):

```bash
KEEPER_MNEMONIC="<dev mnemonic>" cargo run --release -- \
  --config config/testnet-fast.yaml \
  --dry-run \
  --skip-role-check
```

`testnet-fast.yaml` shortens tick cadence so a short run observes discovery and
planning. `inspect_ttls` prints the discovered surface and per-class counts for
read-only audit.

## CLI Flags and Environment

Every `keeper-bot` flag has an environment fallback (`src/main.rs:24-41`).

| Flag | Env var | Default | Required |
| --- | --- | --- | --- |
| `--config`, `-c` | `KEEPER_CONFIG` | `/etc/keeper/testnet.yaml` | no |
| `--dry-run` | `KEEPER_DRY_RUN` | `false` | no |
| `--mnemonic` | `KEEPER_MNEMONIC` | none | no; falls back to Key Vault |
| `--skip-role-check` | `KEEPER_SKIP_ROLE_CHECK` | `false` | no |

Key Vault credentials are read by `mx-keyvault` (`mx-keyvault-0.1.0/src/lib.rs:45-51`).
When all three `AZURE_*` credential variables are set, it uses the client-secret
credential. Otherwise it falls back to the Azure CLI login, then the Azure
Developer CLI login. The distroless image contains neither CLI, so a container
needs all three variables.

| Env var | Meaning | Default | Required |
| --- | --- | --- | --- |
| `AZURE_TENANT_ID` | Azure tenant for the client-secret credential | none | yes, unless a CLI login is used |
| `AZURE_CLIENT_ID` | Azure client id | none | yes, unless a CLI login is used |
| `AZURE_CLIENT_SECRET` | Azure client secret | none | yes, unless a CLI login is used |
| `AZURE_IDENTITY_DISABLE_MANAGED_IDENTITY_CREDENTIAL` | `1` or `true` (any case) forbids the CLI fallback, so a missing credential variable fails the boot | unset (fallback allowed) | no |

`prepay_rent` reads one more variable:

| Env var | Meaning | Default | Required |
| --- | --- | --- | --- |
| `PREPAY_SECRET` | `S...` seed that funds and signs the prepay transactions. The variable name itself is `--secret-env`. | name defaults to `PREPAY_SECRET`; the value has no default | yes, also with `--dry-run` |

`RUST_LOG` overrides `log.level` from the YAML config when it is set and parses
as a filter directive; otherwise `log.level` applies. A value that does not
parse on either side falls back to `info,keeper=debug` rather than failing
startup (`log_filter_directive` in `src/main.rs`). The sibling
`lending-exporter` also lets a `RUST_LOG` that parses override `log.level`, but
it has no fixed fallback filter.

The network name comes from `network` in the YAML config, which is selected by
`--config` or `KEEPER_CONFIG`. There is no separate network environment variable.

## RPC Endpoints and Failover

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

Reads and simulations (`get_latest_ledger`, `get_account`,
`get_full_ledger_entries`, `get_contract_instance`,
`simulate_transaction_envelope`) retry down the list until one endpoint
answers. The endpoint that answers becomes the active one, so the submission
that follows a read and a simulation goes to the same node, and later requests
start there rather than walking the dead primary again.

Transaction submission does not fail over. A send that fails after the
network accepted the transaction must not be replayed on a node that has not
seen it yet; the job fails and the next tick rebuilds it. A failover logs
`RPC failover` at `warn` on target `keeper.rpc`; a total outage fails the tick
with `... failed on all N RPC endpoints`.

## Extra Binaries

- `inspect_ttls --config <path>` (env `KEEPER_CONFIG`, default
  `/etc/keeper/testnet.yaml`): read-only. Prints the discovered surface and
  per-class TTL counts. Submits nothing.
- `prove_permissionless --mnemonic <words> [--rpc …] [--passphrase …] [--controller …] [--derivation-path …]`:
  submits one `ExtendFootprintTtl` on the controller instance to show that TTL
  extension needs no contract role.
- `prepay_rent --config <path> [--secret-env PREPAY_SECRET] [--dry-run]`:
  spends funds. It runs one discovery pass, plans a restore for every lapsed
  entry and an extend for every live entry, whatever its TTL, with no per-tick
  cap, and submits them one by one. The per-user scan covers at most
  `schedule.max_accounts_scan` ids. The signer is built from the `S...` seed in
  `$PREPAY_SECRET` (or in the variable named by `--secret-env`), not from Key
  Vault. `--config` (env `KEEPER_CONFIG`) has no default here. Use `--dry-run`
  first: it prints the planned transaction counts and submits nothing.

```bash
PREPAY_SECRET=S... cargo run --release --bin prepay_rent -- \
  --config config/testnet-fast.yaml --dry-run
```

## Operations

- `GET :9090/health`: returns `ok` after boot.
- `GET :9090/metrics`: Prometheus metrics.
- `--dry-run`: discovers and plans, then simulates planned extend/restore calls
  without submitting.
- `schedule.max_txs_per_tick`: caps transactions per tick.
- `rpc.timeout_seconds`: caps submission polling.
- SIGTERM/SIGINT: stops new ticks, then waits up to 30 seconds for the running
  tick and the metrics server to finish.

Other registered metrics (`src/metrics.rs`):

| Metric | Type | Labels | Meaning |
| --- | --- | --- | --- |
| `keeper_txs_total` | counter | `kind`, `status` | Keeper transactions by kind and outcome. |
| `keeper_sim_failures_total` | counter | `kind`, `reason` | Simulation failures by kind and bucketed reason. |
| `keeper_jobs_planned_total` | counter | `loop` | Jobs planned per loop tick. |
| `keeper_tick_failed_total` | counter | `loop` | Tick failures per loop. |
| `keeper_entries_archived` | gauge | none | Entries the last TTL tick planned to restore: TTL lapsed, value still readable (the `expired` state above). |
| `keeper_max_account_id` | gauge | none | Highest position-NFT token id minted, i.e. the largest account id that can exist. `0` when the NFT address or its counter cannot be read. |

Alert on tick failures (`keeper_tick_failed_total`), tick age
(`keeper_last_tick_timestamp_seconds`) and `keeper_entries_archived`. A silent
keeper failure can become protocol downtime after TTL windows expire. Example
rules live in `ops/alerts.yml`.

## Docker

Every dependency is public. The build needs no secrets. BuildKit is still
required, because the Dockerfile uses cargo cache mounts:

```bash
DOCKER_BUILDKIT=1 docker build -t keeper-bot:latest services/keeper
```

The image sets `KEEPER_CONFIG=/etc/keeper/mainnet.yaml` and `RUST_LOG=info`.
Both take effect (see CLI Flags and Environment). A container started without
an explicit `KEEPER_CONFIG` therefore runs against mainnet and spends the
mainnet signer, so a testnet container must set the variable, as the example
Compose file does.
The example Compose file publishes testnet on host port `9091` and mainnet on
host port `9090`.

Compose example:

```bash
docker compose -f services/keeper/docker-compose.example.yaml up -d
```

## Open Items

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
