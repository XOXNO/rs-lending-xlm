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
| `lending_oracle_primary_price_usd` | `primary_price_wad` (primary leg) |
| `lending_oracle_anchor_price_usd` | `anchor_price_wad` (second independent oracle leg) |
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

A cap is **always** an enforced ceiling in asset units — there is no "unlimited"
sentinel. `0` means that side accepts nothing: the market is **closed**. Because
caps are orthogonal to the `can_be_collateral` / `can_be_borrowed` flags by
design, `cap = 0` on a side that is still flagged enabled is a legitimate soft
wind-down, and nothing else on the board distinguishes it from a live listing:

| Metric | Meaning |
|---|---|
| `lending_spoke_supply_closed` | 1 when `supply_cap = 0` (no new supply accepted) |
| `lending_spoke_borrow_closed` | 1 when `borrow_cap = 0` (no new borrows accepted) |

Cap utilization is 0/0 while closed and so is **not published** — read the gauge
above, not the gap, or a closed market is indistinguishable from a failed scrape.
`LendingSpoke{Supply,Borrow}Closed*` in `ops/alerts.yml` fires on the closed-but-
enabled combination.

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
- Addresses in `config/*.yaml` mirror `configs/networks.json` when present.
- Markets / hubs / spokes labels mirror `configs/{network}/{markets,hubs,spokes}.json`.
- `symbol`, `hubs`, `spoke_names` are display labels only.

`config/mainnet.yaml` lists the canonical mainnet market set (including the LP
and XAUM listings), hubs (`Core` / `RWA` / `Aquarius`), and spokes (`Main` /
`Etherfuse` / `Spiko` / `Centrifuge` / `Forex`). The controller is deliberately
empty in `configs/networks.json` because it is not deployed yet. At deployment,
set `EXPORTER_CONTROLLER` to the deployed `C…` address; it overrides only the
container's configuration and is validated before the exporter starts.

## Deploy (two networks)

Build the image and run testnet with `docker compose up -d
lending-exporter-testnet`. After the mainnet controller is deployed, set the
required `MAINNET_LENDING_CONTROLLER=C…` in the Compose environment and run
`docker compose --profile mainnet up -d lending-exporter-mainnet`. The mainnet
container fails closed at startup if that value is absent. Add both scrape jobs
from `ops/prometheus.example.yml` to Prometheus. Each series already carries a
`network` label.

- Dashboard: import `ops/grafana-dashboard.json` and select Prometheus at the
  import prompt. It is variable-free (queries are static, network pinned to
  `testnet`) so it can be **externally shared** — Grafana's public/shared
  dashboards reject template variables.
  - **MarketIndexView** — snapshot of indexes + soft prices/flags.
  - **Oracles** — price/deviation/freshness trends + **oracle config table**
    (strategy, max/effective stale, tolerance, sanity, probe vs blend timestamps).
    Soft-flag timeline lives only under MarketIndexView (not duplicated).
  - **Spokes** — per-spoke liq curve and listing tables, plus an all-spokes
    operational table that includes new listings and cap-closed state.
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
