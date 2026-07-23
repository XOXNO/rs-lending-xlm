# lending-exporter

Read-only Prometheus exporter for the XOXNO Lending Soroban protocol. On a timer
it reads pool / controller / price-aggregator views over Soroban RPC and serves
them at `/metrics` for a **public** Grafana dashboard. No signer, no writes — it
only simulates read-only contract calls and ledger entry reads.

It is a standalone Cargo workspace (like `../keeper`), shipped as its own
container, one instance per network.

## Data sources

| Source | How resolved | What we publish |
|---|---|---|
| **Controller** | `contracts.controller` | `get_market_indexes_detailed` (`MarketIndexView`), spokes, min borrow collateral |
| **Pool** | live `get_pool_address` each scrape | liquidity, rates, IRM params, last accrual, delta time |
| **Price-aggregator** | live `price_aggregator` view, else YAML fallback | `AssetOracle` config + provider feed freshness probes |

Soft oracle status is the authority for solvency monitoring: the controller bulk
view calls price-aggregator `prices_status` (no fail-closed revert). Provider
probes are early-warning only.

## What it publishes

### MarketIndexView (controller soft status)

Per asset (oracle labels: `network`, `asset`, `symbol`):

| Metric | On-chain field |
|---|---|
| `lending_oracle_price_usd` | `price_wad` (final blend) |
| `lending_oracle_primary_price_usd` | `safe_price_wad` (primary leg) |
| `lending_oracle_anchor_price_usd` | `aggregator_price_wad` (secondary/anchor leg — historical ABI name) |
| `lending_oracle_deviation_bps` | derived \|primary−anchor\| |
| `lending_oracle_status_timestamp_seconds` | `price_timestamp` (blend freshness) |
| `lending_oracle_stale` | `stale` (0/1) |
| `lending_oracle_deviation_flag` | `deviation` (0/1) |
| `lending_oracle_healthy` | `valid` (1 = usable for solvency) |

Per hub-asset (market labels include `hub_id` / `hub`):

- `lending_market_supply_index_ray` / `lending_market_borrow_index_ray`

### Oracle config + provider freshness (price-aggregator)

- max stale / effective max stale (worst leg), tolerance bands, sanity min/max, strategy
- provider-probe timestamp + seconds until stale

### Pool hub-asset

- supplied / borrowed / available liquidity / revenue (tokens + USD)
- utilization, supply/borrow APY
- IRM params (`lending_market_param{param=…}`)
- `lending_market_last_accrual_timestamp`, `lending_market_delta_time_seconds`

### Spokes (controller)

- per listing: paused/frozen/collateral/borrow, LTV/threshold/bonus/fees, caps, usage, cap util
- per spoke: deprecation (on asset series), liquidation target HF, HF for max bonus, bonus factor bps

### Protocol + exporter health

- TVL / borrowed / liquidity / revenue aggregates, market/spoke counts, min borrow collateral
- scrape duration, last success, ledger time/sequence/skew, RPC errors, view failures

Only aggregate / market / oracle / spoke-config data is exposed — **no per-user
account data** goes on the public dashboard.

## Run locally

```bash
cargo run -- --config config/testnet.yaml
# then:
curl -s localhost:9110/metrics | grep lending_
```

`EXPORTER_CONFIG` env var is an alternative to `--config`.

## Config

One YAML per network (`config/testnet.yaml`, `config/mainnet.yaml`). Lists the
controller and `(hub_id, asset, symbol)` markets + `spokes` to scan.

- **Pool** and **price-aggregator** are resolved each scrape from the controller
  (`get_pool_address`, `price_aggregator`). YAML `price_aggregator` is a fallback.
- Addresses in `config/testnet.yaml` mirror `configs/networks.json`.
- `symbol`, `hubs`, `spoke_names` are display labels only.

Mainnet contracts are not deployed yet; `config/mainnet.yaml` is a stub and the
exporter refuses to boot until `contracts.controller` is a valid `C…` address.

## Deploy (two networks)

Build the image and run one container per network (see
`docker-compose.example.yaml`), then add the scrape jobs from
`ops/prometheus.example.yml` to `/data/coolify/prometheus/prometheus.yml`. Each
series already carries a `network` label, so one Grafana renders both.

- Dashboard: import `ops/grafana-dashboard.json`. It is variable-free (queries
  are static, network pinned to `testnet`) so it can be **externally shared** —
  Grafana's public/shared dashboards reject template variables.
  - **MarketIndexView** — snapshot of indexes + soft prices/flags.
  - **Oracles** — price/deviation/freshness trends + **oracle config table**
    (strategy, max/effective stale, tolerance, sanity, probe vs blend timestamps).
    Soft-flag timeline lives only under MarketIndexView (not duplicated).
  - **Spokes** — liq curve, listing table with pause/freeze/collateral/borrow,
    LTV/threshold/bonus/fees, usage & cap util.
- Alerts: recreate the exprs in `ops/alerts.yml` as Grafana-managed alert rules
  (they stay internal, off the public panels). Prefer soft-status flags over
  hard-path error codes.

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
