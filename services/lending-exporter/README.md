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
view calls the price-aggregator `quotes` entrypoint, which returns a
`PriceStatus` per key and never reverts fail-closed. (`prices_status` is not an
entrypoint; `fetch_prices_status` is the controller-internal helper that wraps
`quotes`.) Provider probes are early-warning only.

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

Per listing (`network`, `spoke_id`, `spoke`, `hub_id`, `hub`, `asset`, `symbol`):

- `lending_spoke_paused`, `lending_spoke_frozen`,
  `lending_spoke_collateral_enabled`, `lending_spoke_borrow_enabled`,
  `lending_spoke_deprecated`
- `lending_spoke_ltv_bps`, `lending_spoke_liquidation_threshold_bps`,
  `lending_spoke_liquidation_bonus_bps`, `lending_spoke_liquidation_fees_bps`
- `lending_spoke_supply_cap`, `lending_spoke_borrow_cap`
- `lending_spoke_supply_usage`, `lending_spoke_supply_usage_usd`,
  `lending_spoke_borrow_usage`, `lending_spoke_borrow_usage_usd`
- `lending_spoke_supply_cap_utilization`,
  `lending_spoke_borrow_cap_utilization`

Per spoke (`network`, `spoke_id`, `spoke`): `lending_spoke_liquidation_target_hf`,
`lending_spoke_hf_for_max_bonus`, `lending_spoke_liquidation_bonus_factor_bps`.
`lending_spoke_deprecated` carries the spoke's deprecation flag on the
per-listing series.

A cap is **always** an enforced ceiling in asset units — there is no "unlimited"
sentinel. `0` means that side accepts nothing: the market is **closed**. Because
caps are orthogonal to the `is_collateralizable` / `is_borrowable` flags by
design, `cap = 0` on a side that is still flagged enabled is a legitimate soft
wind-down, and nothing else on the board distinguishes it from a live listing:

| Metric | Meaning |
|---|---|
| `lending_spoke_supply_closed` | 1 when `supply_cap = 0` (no new supply accepted) |
| `lending_spoke_borrow_closed` | 1 when `borrow_cap = 0` (no new borrows accepted) |

Cap utilization is 0/0 while closed and so is **not published** — read the gauge
above, not the gap, or a closed market is indistinguishable from a failed scrape.
`LendingSpokeSupplyClosedWhileCollateralEnabled` and
`LendingSpokeBorrowClosedWhileBorrowEnabled` in `ops/alerts.yml` fire on the
closed-but-enabled combination.

### Protocol + exporter health

- TVL / borrowed / liquidity / revenue aggregates, market/spoke counts, min borrow collateral
- `lending_exporter_scrape_duration_seconds`,
  `lending_exporter_last_success_timestamp`,
  `lending_exporter_ledger_skew_seconds`, `lending_ledger_timestamp_seconds`,
  `lending_ledger_sequence`, `lending_exporter_rpc_errors_total`,
  `lending_exporter_view_failures_total`
- `lending_exporter_build_info` — always `1`; labels `network` and `version`

Only aggregate / market / oracle / spoke-config data is exposed — **no per-user
account data** goes on the public dashboard.

## Run locally

```bash
cargo run -- --config config/testnet.yaml
# then:
curl -s localhost:9110/metrics | grep lending_
```

### Environment

`--config` is the only CLI flag. Environment values override the loaded YAML
before validation (`src/main.rs:22-29`, `src/config.rs:104-105` and
`src/config.rs:112-123`).

| Env var | Meaning | Default | Required |
| --- | --- | --- | --- |
| `EXPORTER_CONFIG` | Path to the YAML config; alternative to `--config` | `/etc/lending-exporter/testnet.yaml` | no |
| `EXPORTER_RPC_URL` | Overrides `rpc.url`. An empty value is ignored. | none | no |
| `EXPORTER_CONTROLLER` | Overrides `contracts.controller`. An empty value is ignored. | none | no |
| `EXPORTER_PRICE_AGGREGATOR` | Overrides `contracts.price_aggregator`. An empty value clears it back to the live controller lookup. | none | no |
| `EXPORTER_XOXNO_ORACLE_ADAPTER` | Overrides `contracts.xoxno_oracle_adapter`. An empty value clears it. | none | no |
| `RUST_LOG` | `tracing` filter. When set it replaces `log.level` from the YAML (`src/main.rs:99-107`). | unset; falls back to `log.level` | no |

`MAINNET_LENDING_CONTROLLER` is **not** read by the binary. It is a Compose-level
variable that the example Compose file pipes into `EXPORTER_CONTROLLER`
(`docker-compose.example.yaml:27`).

Like the sibling `keeper`, this service honours `RUST_LOG`: when set and
parseable it replaces `log.level` from the YAML (`../keeper/src/main.rs:168-178`).

## Config

One YAML per network (`config/testnet.yaml`, `config/mainnet.yaml`). Lists the
controller and `(hub_id, asset, symbol)` markets + `spokes` to scan.

- **Pool** and **price-aggregator** are resolved each scrape from the controller
  (`get_pool_address`, `price_aggregator`). YAML `price_aggregator` is a fallback.
- `configs/networks.json` is the source of truth for contract addresses;
  `config/*.yaml` must track it. `config/testnet.yaml` was re-synced to it on
  2026-08-26 (controller `CCXRWJ6SIU2W…`, price-aggregator `CAALOOTIDXCX…`,
  oracle adapter `CDYX4ZEO556Y…`). When an address changes, update
  `configs/networks.json` first, then mirror it here.
- Markets / hubs / spokes labels are meant to mirror
  `configs/{network}/{markets,hubs,spokes}.json`, but `config/mainnet.yaml`
  currently trails it — see below.
- `symbol`, `hubs`, `spoke_names` are display labels only.
- `scrape_interval_seconds` defaults to `30`. Values below `5` fail startup.
- `rpc.timeout_seconds` is parsed but **has no effect**: `RpcClient::new`
  (`src/stellar/client.rs:14-18`) never applies it.

`config/mainnet.yaml` lists hubs (`Core` / `RWA` / `Aquarius`) and eight of the
nine spokes (`Main` / `Etherfuse` / `Spiko` / `Centrifuge` / `Forex` /
`LP Tokens` / `Ondo` / `Commodities`) — spoke `9` (`Aquarius`) is missing. Its
market list covers 25 of the 30 markets in `configs/mainnet/markets.json`:
`xSolvBTC`, `AQUA`, `xSolvBTCSolvBTC_LP`, `XLMAQUA_LP` and `AQUAUSDC_LP` are
absent. Anything not listed is simply not scraped (`collector.rs:780`
iterates `cfg.spokes`), and `lending_protocol_spokes_count` reports
`cfg.spokes.len()` — 8, not 9. The controller is deliberately
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

- Dashboard: import `ops/grafana-dashboard.json` into the production Grafana.
  It pins that Grafana's Prometheus datasource UID, so an external/shared view
  never depends on an unresolved import variable. For another Grafana instance,
  replace the datasource UID before importing. Queries are static and pinned to
  `testnet` because public/shared dashboards reject template variables.
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
