# keeper-bot

Off-chain Rust service that keeps the XOXNO Lending Soroban protocol's
storage and wasm-code entries alive.

Per tick the service:

1. Discovers the controller's persistent state (pool list, e-mode entries,
   per-account triplets) and the wasm-code entries for the controller, pool
   template, and flash-loan receiver.
2. Decides which entries are inside the configured safety margin (default 14
   days for testnet, 21 days for mainnet).
3. Submits the minimum number of transactions to keep them alive:
   - `keepalive_shared_state`, `keepalive_pools`, `keepalive_accounts`
     (chunked) on the controller.
   - A single `ExtendFootprintTTL` op for the wasm-code entries (contracts
     cannot extend their own `ContractCode` entry — that's the only place
     this op is used).
   - On a separate slower cadence, `update_indexes(assets)` so pool
     interest accrual stays current.

The admin signing key is a BIP-39 mnemonic fetched from Azure Key Vault
through the in-house `mx-keyvault` crate and derived per SEP-0005
(`m/44'/148'/0'`).

## Layout

```
services/keeper/
├── Cargo.toml
├── Dockerfile
├── docker-compose.example.yaml
├── README.md
├── config/
│   ├── testnet.yaml
│   └── mainnet.yaml
└── src/
    ├── main.rs              # entry point + signals + tracing init
    ├── config.rs            # YAML loader / validator
    ├── signer/              # KeyVault fetch + SLIP-0010 derivation
    ├── stellar/             # RPC, tx pipeline, op builders
    ├── keys.rs              # ControllerKey → ScVal encoding
    ├── discovery.rs         # tick-time state read + self-check + role gate
    ├── scheduler/           # tick loops, planner, per-tick budget
    ├── policy.rs            # bump-or-not decision
    └── metrics.rs           # Prometheus surface + /health
```

## SDK stack

| Crate | Version |
|-------|---------|
| `stellar-rpc-client` | =26.0.0 |
| `stellar-xdr` | =26.0.1 |
| `stellar-strkey` | ^0.0.16 |
| `ed25519-dalek` | ^2 |
| `bip39` | ^2.2 |
| `mx-keyvault` | git, XOXNO/mx-chain-rust@feat/node |

## Local build

```bash
cd services/keeper
cargo check
cargo build --release
```

Run dry against testnet:

```bash
AZURE_TENANT_ID=... AZURE_CLIENT_ID=... AZURE_CLIENT_SECRET=... \
  cargo run --release -- --config config/testnet.yaml --dry-run
```

## Docker build

`mx-keyvault` is a private dep. Pass `~/.git-credentials` (or an SSH agent
forwarder) at build time via BuildKit secrets:

```bash
DOCKER_BUILDKIT=1 docker build \
  --secret id=git_credentials,src=$HOME/.git-credentials \
  -t keeper-bot:latest \
  services/keeper
```

Compose example bundles a testnet + mainnet pair from the same image:

```bash
docker compose -f services/keeper/docker-compose.example.yaml up -d
```

## Operations

- **Health**: `GET :9090/health` returns `ok` once boot completed.
- **Metrics**: `GET :9090/metrics` (Prometheus text format).
- **Dry run**: `--dry-run` (or `KEEPER_DRY_RUN=1`) — simulates everything,
  never submits.
- **Boot safety**: refuses to start unless `keepalive_pools(empty)`
  simulates successfully (proves the signer has the `KEEPER` role).
- **Budget cap**: `schedule.max_txs_per_tick` (default 50/80) bounds the
  worst-case fee burn per tick.
- **Shutdown**: SIGTERM / SIGINT cancel in-flight ticks and wait up to 30 s
  for the active tx to reach a terminal status.

## Open items

- `mx-keyvault` is sourced from `XOXNO/mx-chain-rust@feat/node`. Pin to a
  tagged release once available.
- Mainnet contract IDs in `config/mainnet.yaml` are placeholders — populate
  before deploying to mainnet.
- The keeper assumes only source-account auth is required by the
  controller's keeper endpoints. If a future contract change introduces
  contract-side auth, the simulator's auth check will reject the job and
  log a warning until the keeper learns to attach `SorobanAuthorizationEntry`
  payloads.
