# lending-exporter

Read-only Prometheus exporter for the XOXNO Lending Soroban protocol. On a timer
it reads the pool/controller/oracle view functions over Soroban RPC and serves
them at `/metrics` for a **public** Grafana dashboard. No signer, no writes — it
only simulates read-only contract calls.

It is a standalone Cargo workspace (like `../keeper`), shipped as its own
container, one instance per network.

## What it publishes

- **Per market** (`hub_id`, `asset`, `symbol`): supplied / borrowed / available
  liquidity / revenue (tokens + USD), utilization, supply/borrow APY, live
  supply/borrow indexes, last accrual timestamp, and the full IRM curve params
  (`lending_market_param{param=…}`).
- **Per oracle asset**: final / primary / anchor USD price, primary-vs-anchor
  deviation (bps), health, configured max-stale / tolerance band / sanity
  bounds / strategy, price timestamp, and **seconds until stale** (vs the ledger
  clock).
- **Per spoke-asset** (`spoke_id`): paused/frozen/collateral/borrow/deprecated
  flags, LTV / threshold / bonus / fees (bps), supply/borrow caps, usage, and
  cap utilization.
- **Protocol aggregate**: TVL, total borrowed, total liquidity, revenue (USD),
  market/spoke counts, min borrow collateral.
- **Exporter health**: scrape duration, last-success timestamp, ledger
  timestamp/sequence/skew, RPC errors, and contract view failures (bucketed by
  error code).

Only aggregate / market / oracle / spoke-config data is exposed — **no per-user
account data** goes on the public dashboard. Everything published is already
public on-chain.

## Run locally

```bash
cargo run -- --config config/testnet.yaml
# then:
curl -s localhost:9110/metrics | grep lending_
```

`EXPORTER_CONFIG` env var is an alternative to `--config`.

## Config

One YAML per network (`config/testnet.yaml`, `config/mainnet.yaml`). It lists the
controller address and the `(hub_id, asset, symbol)` markets + `spokes` to scan.
The central pool address and each asset's oracle sources are resolved on-chain,
so they are not configured.

Two caveats to resolve before production use:

1. **Verify the addresses.** `config/testnet.yaml` mirrors the running
   `../keeper/config/testnet.yaml`. `configs/networks.json` currently lists
   *different* testnet controller/oracle addresses — confirm which deployment is
   live and keep this file in sync. The exporter is per-market resilient: an
   address that no longer resolves surfaces as a `view_failures` counter, not a
   crash.
2. **Fill in real `symbol`s** — the testnet markets ship with `TKN1…TKN5`
   placeholders.

Mainnet contracts are not deployed yet; `config/mainnet.yaml` is a stub and the
exporter refuses to boot until `contracts.controller` is a valid `C…` address.

## Deploy (two networks)

Build the image and run one container per network (see
`docker-compose.example.yaml`), then add the scrape jobs from
`ops/prometheus.example.yml` to `/data/coolify/prometheus/prometheus.yml`. Each
series already carries a `network` label, so one Grafana renders both.

- Dashboard: import `ops/grafana-dashboard.json`.
- Alerts: recreate the exprs in `ops/alerts.yml` as Grafana-managed alert rules
  (they stay internal, off the public panels).

## Layout

| File | Role |
|---|---|
| `src/stellar/view.rs` | read-only `simulateTransaction` → decode return `ScVal` |
| `src/stellar/client.rs` | RPC wrapper + ledger close-time |
| `src/keys.rs`, `src/scval.rs` | XDR arg/key builders + `ScVal` field readers |
| `src/contract/{pool,controller,oracle}.rs` | typed view/ledger decoders |
| `src/model.rs` | RAY/WAD/BPS scaling, APY, deviation, staleness math |
| `src/metrics.rs` | Prometheus families + `/metrics` + `/health` |
| `src/collector.rs` | one scrape cycle (batch-trap fallback, error isolation) |
| `src/main.rs` | runtime, interval loop, graceful shutdown |
