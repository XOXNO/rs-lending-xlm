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
   - the controller's per-user account keys — `AccountMeta`, `SupplyPositions`,
     `BorrowPositions` for every account `1..=AccountNonce`, and
     `IsolatedBasis(id, asset)` for accounts flagged isolated (the keeper
     decodes each `AccountMeta` to learn the isolated asset, so it builds at
     most one basis key per account);
   - the controller's access-control role keys (`ExistingRoles`, and per-role
     `RoleAccountsCount` / `RoleAccounts` / `HasRole` for `KEEPER` / `REVENUE` /
     `ORACLE`), which the contract self-extends only when a role-gated call
     reads them — so an idle protocol would otherwise let them archive;
   - when a `governance` contract is configured: its instance entry (which
     covers `Controller`, ownable `Owner`, access_control `Admin` + `RoleAdmin`,
     and the timelock `MinDelay` — all instance-tier) and its persistent
     access-control role keys for `PROPOSER` / `EXECUTOR` / `CANCELLER` /
     `ORACLE`;
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

### Per-user account keys

The keeper renews the per-user account keys (`AccountMeta`, `SupplyPositions`,
`BorrowPositions`, and `IsolatedBasis` for isolated accounts) for every account
`1..=AccountNonce`. A user auto-bumps their own keys when they interact, but an
inactive position would otherwise archive — losing it would block liquidation
and freeze the user's collateral, so the keeper keeps the whole account surface
alive. This is gated by `schedule.scan_users` (default `true`) and bounded by
`schedule.max_accounts_scan` (default `50_000`): if `AccountNonce` exceeds the
cap, the keeper logs a loud `warn!` naming the exact dropped id range and never
silently truncates.

### Governance

When `contracts.governance` is set, the keeper also keeps the governance
contract alive. Almost all of governance's state is instance-tier — `Controller`
(the owned controller), ownable `Owner`, access_control `Admin` + `RoleAdmin`,
and the timelock `MinDelay` — so a single instance bump covers it. Its
persistent access-control role-holder keys (`PROPOSER` / `EXECUTOR` /
`CANCELLER` / `ORACLE`) are discovered with the same code path as the
controller's role keys.

**`MinDelay` is instance-tier, not persistent** (verified against
stellar-governance 0.7.2 `src/timelock/storage.rs`: both `get_min_delay` and
`set_min_delay` use `e.storage().instance()`). The keeper therefore relies on
the governance instance bump and does **not** build a standalone `MinDelay`
persistent key — doing so would silently resolve to nothing.

The timelock per-operation keys `OperationLedger(BytesN<32>)` are persistent but
**not enumerable on-chain**: the operation id is a keccak256 hash known only
from the schedule event. They are transient — a scheduled op resolves within
`min_delay` ledgers, far inside any TTL window — so the keeper documents and
intentionally skips them. Closing this gap would require off-chain event
tracking (future work).

### Coverage table

| Class | Tier | Source | Renewed |
|-------|------|--------|---------|
| Controller instance (`Aggregator`, `Pool`, `Accumulator`, …) | instance | instance read | yes |
| `PoolsList`, per-asset `Market` / `IsolatedDebt` | persistent | `PoolsList` | yes |
| Pool `Params` / `State` per asset | persistent | `PoolsList` | yes |
| `EModeCategory(1..=LastEModeCategoryId)` | persistent | instance | yes |
| Controller role keys | persistent | `ExistingRoles` | yes |
| Per-user `AccountMeta` / `SupplyPositions` / `BorrowPositions` | persistent | `AccountNonce` | yes (`scan_users`) |
| Per-user `IsolatedBasis(id, asset)` | persistent | decoded `AccountMeta` | yes (isolated only) |
| Governance instance (`Controller`, `Owner`, `Admin`, `RoleAdmin`, `MinDelay`) | instance | instance read | yes (when configured) |
| Governance role keys (`PROPOSER` / `EXECUTOR` / `CANCELLER` / `ORACLE`) | persistent | `ExistingRoles` | yes (when configured) |
| Pool / flash-receiver instances + all wasm code | instance / code | instance read | yes |
| Timelock `OperationLedger(BytesN<32>)` | persistent | none (event only) | no — transient, documented gap |
| `FlashLoanOngoing`, `PendingOwner`, `PendingAdmin` | temporary | n/a | no — auto-expire by design |

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
    ├── keys.rs              # controller/per-user/access-control key → ScVal encoding
    ├── discovery.rs         # tick-time state read (per-asset, per-user, roles, governance) + self-check
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
full discovery + plan cycle. The `inspect_ttls` binary prints the full
discovered surface grouped by coverage class (per-asset, e-mode, per-user,
roles, governance, instances, wasm) with per-class counts, so coverage is
auditable read-only against a live network without submitting anything.

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
- **Alerting (required, not optional)**: a silently-dead keeper → un-extended
  TTLs → archived storage → frozen protocol is this service's highest-impact
  failure mode, so the metrics MUST be alerted on, not merely exposed. Ship the
  Prometheus rules in [`ops/alerts.yml`](ops/alerts.yml) (scrape example in
  [`ops/prometheus.example.yml`](ops/prometheus.example.yml)) and wire
  Alertmanager so `severity: critical` actually pages. Key signals: `KeeperDown`
  (`up==0` — early warning, fires long before any TTL can lapse),
  `KeeperEntriesArchived` (`keeper_entries_archived > 0` — active eviction
  incident), plus `KeeperNotProgressing` / `KeeperTickFailing` /
  `KeeperTxFailures` (alive-but-stuck).
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

- Mainnet contract IDs in `config/mainnet.yaml` are placeholders — populate
  before deploying to mainnet.
- The keeper assumes only source-account auth is required by `update_indexes`.
  If a future contract change introduces contract-side auth, the simulator's
  auth check rejects the job and logs a warning until the keeper learns to
  attach `SorobanAuthorizationEntry` payloads.
