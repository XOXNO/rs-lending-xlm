# keeper-bot

On Soroban, stored data is rented: every entry has a time-to-live (TTL), and if
its TTL runs out the entry is archived and the contract stops working until it is
restored. This keeper is the off-chain Rust service that quietly pays that rent —
it keeps the XOXNO Lending protocol's storage and wasm-code entries alive by
extending their TTL before they fall inside the configured safety margin, and
restores any that have already lapsed.

Per TTL tick the service:

1. **Discovers** the entries that keep the protocol functional:
   - the controller instance entry (which covers every instance-tier key,
     including the oracle `Aggregator`, pool template, and accumulators);
   - the controller's persistent keys — `PoolsList`, and per-asset `Market`
     (which embeds each asset's oracle config) and `IsolatedDebt`, plus each
     `EModeCategory`;
   - the controller's access-control role keys (`ExistingRoles`, and per-role
     `RoleAccountsCount` / `RoleAccounts` / `HasRole` for `KEEPER` / `REVENUE` /
     `ORACLE`), which the contract self-extends only when a role-gated call
     reads them — so an idle protocol would otherwise let them archive;
   - each pool's instance entry and the flash-loan receiver's instance entry;
   - the wasm-code entries for the controller, pool template, and flash-loan
     receiver.
2. **Decides**, per entry: live and inside the safety margin (default 14 days on
   testnet, 21 on mainnet) → extend; already lapsed but data still present →
   restore; healthy or never-written → skip.
3. **Submits** chunked `ExtendFootprintTtl` ops for the in-margin entries, and
   `RestoreFootprint` ops for the archived ones (freshly-restored entries are
   then extended to the cap the same tick). Both ops are permissionless — the
   signer needs no on-chain role, only XLM for fees and rent.

The keeper deliberately does **not** renew the per-user triplets
(`AccountMeta`, `SupplyPositions`, `BorrowPositions`): a user auto-bumps their
own three keys whenever they interact with the protocol. (Renewing keys for
inactive users approaching liquidation is a separate, future concern.)

Optionally, a second slower loop runs `update_indexes(assets)` so pool
interest accrual stays current. That call mutates pool state and is the **only**
keeper operation that requires the signer to hold the on-chain `KEEPER` role;
it is disabled by default (`schedule.enable_index_refresh: false`).

The signing key is a BIP-39 mnemonic fetched from Azure Key Vault through the
in-house `mx-keyvault` crate and derived per SEP-0005 (`m/44'/148'/0'`).

## Layout

```
services/keeper/
├── Cargo.toml
├── Dockerfile
├── docker-compose.example.yaml
├── README.md
├── config/
│   ├── testnet.yaml
│   ├── testnet-fast.yaml
│   └── mainnet.yaml
└── src/
    ├── main.rs              # entry point + signals + tracing init
    ├── config.rs            # YAML loader / validator
    ├── signer/              # KeyVault fetch + SLIP-0010 derivation
    ├── stellar/             # RPC, tx pipeline, op builders
    ├── keys.rs              # ControllerKey + access-control key → ScVal encoding
    ├── discovery.rs         # tick-time state read (incl. role keys) + self-check
    ├── scheduler/           # tick loops, extend/restore planner, per-tick budget
    ├── policy.rs            # per-entry extend / restore / skip decision
    ├── metrics.rs           # Prometheus surface + /health
    └── bin/
        ├── prove_permissionless.rs  # diagnostic: extend storage with no role
        └── inspect_ttls.rs          # diagnostic: per-key TTL / restore-vs-extend table
```

## SDK stack

| Crate | Version |
|-------|---------|
| `stellar-rpc-client` | =26.0.0 |
| `stellar-xdr` | =26.0.1 |
| `stellar-strkey` | ^0.0.16 |
| `ed25519-dalek` | ^2 |
| `bip39` | ^2.2 |
| `mx-keyvault` | git, XOXNO/mx-chain-rust@production |

The two `stellar-*` versions are hard-pinned so a passive dependency bump
cannot silently change keeper behavior.

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

For local dev without Azure creds, two flags bypass production gates
(both clearly log a DEV warning when used):

```bash
cargo run --release -- \
  --config config/testnet-fast.yaml \
  --dry-run \
  --skip-role-check \
  --mnemonic "$(your dev mnemonic; NEVER commit a real one)"
```

`testnet-fast.yaml` shortens the tick cadence to 20s so a short run observes a
full discovery + plan cycle. Verified against live testnet 2026-05-29: 5 listed
assets, encoding self-check passes, discovery reads no per-user keys, the
planner batches the in-margin instance + wasm-code entries into a single
`ExtendFootprintTtl` job, `/health` + `/metrics` serve, and SIGTERM shuts the
loop down cleanly.

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
- **Dry run**: `--dry-run` (or `KEEPER_DRY_RUN=1`) — runs discovery and planning,
  then **simulates** each planned extend / restore against the RPC and logs
  whether it would be accepted (`sim ok` with the resource fee, or
  `sim REJECTED`). Submits nothing, and needs no funded signer (simulation uses
  sequence `0`). Use it to confirm restores of currently-archived keys before
  trusting them.
- **Boot safety**: an encoding self-check reads `PoolsList` from the live
  controller and refuses to start if the `ControllerKey` encoding has drifted.
  When `enable_index_refresh` is on, the keeper additionally simulates
  `update_indexes(empty)` and refuses to start unless the signer holds the
  `KEEPER` role.
- **Budget cap**: `schedule.max_txs_per_tick` (default 50/80) bounds the
  worst-case fee burn per tick.
- **Submission timeout**: `rpc.timeout_seconds` caps each submission poll; a
  timed-out submit is retried on the next tick (TTL extends are idempotent).
- **Shutdown**: SIGTERM / SIGINT cancel in-flight ticks and wait up to 30 s
  for the active tx to reach a terminal status.

## Open items

- `mx-keyvault` is sourced from `XOXNO/mx-chain-rust@production`. Pin to a
  tagged release / commit once available.
- Mainnet contract IDs in `config/mainnet.yaml` are placeholders — populate
  before deploying to mainnet.
- The keeper assumes only source-account auth is required by `update_indexes`.
  If a future contract change introduces contract-side auth, the simulator's
  auth check rejects the job and logs a warning until the keeper learns to
  attach `SorobanAuthorizationEntry` payloads.
