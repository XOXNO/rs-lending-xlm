# XOXNO Lending Keeper

Soroban contract storage has time-to-live (TTL). When TTL lapses, entries
archive and must be restored before contract calls can use them again. This
service keeps XOXNO Lending storage, instances, and WASM code entries alive by
extending TTL before the configured safety margin and restoring archived entries
it discovers.

`services/keeper` is a separate Rust workspace.

## Discovery Surface

Each TTL tick discovers:

- Controller instance entry. This covers instance-tier keys such as pool address,
  pool template, accumulator, account nonce, spoke/hub counters, and position
  limits.
- Controller persistent `AssetOracle(asset)` rows for configured market assets.
- Controller persistent `Spoke(id)` rows for `1..=LastSpokeId`.
- Controller per-user persistent keys:
  `AccountMeta(id)`, `SupplyPositions(id)`, `BorrowPositions(id)`.
- Controller access-control persistent keys when present:
  `ExistingRoles`, `RoleAccountsCount`, `RoleAccounts`, `HasRole`, `RoleAdmin`.
- Governance instance and governance role-holder keys when `contracts.governance`
  is configured.
- Pool instance, flash-loan receiver instance, controller WASM, pool template
  WASM, live pool WASM, and flash-loan receiver WASM.
- Pool persistent `Params(HubAssetKey)` and `State(HubAssetKey)` rows for
  configured markets.

The current protocol does not have controller `KEEPER`, `REVENUE`, or `ORACLE`
roles. Governance role keys are discovered from `ExistingRoles`; expected
governance roles are `PROPOSER`, `EXECUTOR`, `CANCELLER`, and `ORACLE`.

## Market Configuration

Use `contracts.markets` for current protocol storage keys:

```yaml
contracts:
  controller: C...
  pool_wasm_hash: "..."
  flash_loan_receiver: C...
  governance: C...
  markets:
    - hub_id: 1
      asset: C...
```

`contracts.market_assets` remains as a legacy shorthand. Each entry maps to
`hub_id = 1`. Prefer `contracts.markets` because pool storage keys are encoded
as `HubAssetKey { hub_id, asset }`.

## Index Refresh

The optional index loop calls:

```text
controller.update_indexes(caller, Vec<HubAssetKey>)
```

The caller signs the transaction. The current controller does not require a
keeper role for this call. The loop is disabled by default:

```yaml
schedule:
  enable_index_refresh: false
```

## Governance Notes

When `contracts.governance` is set, keeper also keeps governance alive.
Governance stores `Controller`, ownable `Owner`, access-control `Admin` /
`RoleAdmin`, and timelock `MinDelay` in instance storage, so the governance
instance bump covers them.

Timelock `OperationLedger(BytesN<32>)` keys are persistent but not enumerable
from contract storage. They are intentionally skipped; operations resolve within
`min_delay`, far inside normal TTL windows. Event tracking would be needed to
renew them directly.

## Coverage Table

| Class | Tier | Source | Renewed |
| --- | --- | --- | --- |
| Controller instance | instance | configured controller | yes |
| Controller `AssetOracle(asset)` | persistent | `contracts.markets` / legacy `market_assets` | yes |
| Controller `Spoke(id)` | persistent | `LastSpokeId` | yes |
| Account state | persistent | `AccountNonce` scan | yes, bounded by `max_accounts_scan` |
| Controller access-control keys | persistent | `ExistingRoles` | yes, when present |
| Pool `Params/State(HubAssetKey)` | persistent | configured markets | yes |
| Governance instance | instance | configured governance | yes |
| Governance role keys | persistent | `ExistingRoles` | yes, when configured |
| Pool / receiver instances and WASM code | instance / code | instance reads | yes |
| Timelock `OperationLedger(BytesN<32>)` | persistent | event-only | no, documented gap |
| Temporary keys | temporary | n/a | no, expire by design |

## Layout

```text
services/keeper/
├── Cargo.toml
├── Dockerfile
├── config/
│   ├── testnet.yaml
│   ├── testnet-fast.yaml
│   └── mainnet.yaml
└── src/
    ├── main.rs
    ├── config.rs
    ├── discovery.rs
    ├── keys.rs
    ├── scheduler/
    ├── signer/
    ├── stellar/
    ├── metrics.rs
    └── bin/
```

## SDK Stack

| Crate | Version |
| --- | --- |
| `stellar-rpc-client` | `=26.0.0` |
| `stellar-xdr` | `=26.0.1` |
| `stellar-strkey` | `^0.0.16` |
| `ed25519-dalek` | `^2` |
| `bip39` | `^2.2` |
| `mx-keyvault` | `XOXNO/mx-chain-rust@production` |

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
  cargo run --release -- --config config/testnet.yaml --dry-run
```

Local development without Azure credentials:

```bash
cargo run --release -- \
  --config config/testnet-fast.yaml \
  --dry-run \
  --skip-role-check \
  --mnemonic "$(your dev mnemonic; never commit real one)"
```

`testnet-fast.yaml` shortens tick cadence so a short run observes discovery and
planning. `inspect_ttls` prints the discovered surface and per-class counts for
read-only audit.

## Operations

- `GET :9090/health`: returns `ok` after boot.
- `GET :9090/metrics`: Prometheus metrics.
- `--dry-run`: discovers and plans, then simulates planned extend/restore calls
  without submitting.
- `schedule.max_txs_per_tick`: caps transactions per tick.
- `rpc.timeout_seconds`: caps submission polling.
- SIGTERM/SIGINT: cancels in-flight ticks and waits up to 30 seconds for active
  submissions to finish.

Alert on keeper liveness and archived entries. A silent keeper failure can
become protocol downtime after TTL windows expire.

## Docker

`mx-keyvault` is a private dependency. Pass credentials with BuildKit secrets:

```bash
DOCKER_BUILDKIT=1 docker build \
  --secret id=git_credentials,src=$HOME/.git-credentials \
  -t keeper-bot:latest \
  services/keeper
```

Compose example:

```bash
docker compose -f services/keeper/docker-compose.example.yaml up -d
```

## Open Items

- Populate `config/mainnet.yaml` before mainnet deployment.
- If `update_indexes` gains contract-side auth in a future controller version,
  keeper must attach the required `SorobanAuthorizationEntry` payloads.
