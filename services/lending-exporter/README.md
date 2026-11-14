# lending-exporter

Read-only Prometheus exporter for XOXNO Lending. On a timer it reads the
controller, pool, price-aggregator and oracle provider contracts over Soroban
RPC and serves `/metrics` for a public Grafana dashboard. No signer: only
read-only simulations and ledger reads. Standalone Cargo workspace and
container, one instance per network.

## Quick start

```bash
cargo run -- --config config/testnet.yaml
curl -s localhost:9110/metrics | grep lending_
```

## How it reads the protocol

| Source | Address from | Gives |
|---|---|---|
| Controller | config file | market indexes, soft oracle status, spokes, minimum borrow collateral |
| Pool | controller, each scrape | liquidity, rates, IRM parameters, last accrual |
| Price-aggregator | controller, each scrape (config file on failure) | oracle config |
| Oracle providers and LP pools | oracle config, each scrape | feed timestamps, LP share supply |

The controller's per-asset **soft oracle status** is the solvency-monitoring
authority: it always answers instead of failing closed. Direct provider probes
are early warning only.

## What it publishes

All metric names start with the lending_ prefix. No per-user data.

- **Oracle**, per asset: blended, primary and anchor price; deviation (bps);
  blend timestamp; stale, deviation and healthy flags; error code behind an
  invalid price.
- **Oracle config**, per asset: max staleness, tolerance band, sanity bounds,
  source count, seconds until a feed goes stale. Single-LP-source assets add
  pool value floor and LP share supply.
- **Market**, per hub and asset: supplied, borrowed, available liquidity and
  revenue (tokens and USD); utilization; supply and borrow APY; interest-rate
  parameters; time since last accrual.
- **Spoke**, per listing: paused, frozen, collateral, borrow and deprecated
  flags; LTV, liquidation threshold, bonus and fees; supply and borrow caps,
  usage and fill. Per spoke: liquidation target HF, HF at max bonus, bonus
  factor. A listing removed on chain or undecodable loses every series.
- **Protocol**: TVL, total borrowed, liquidity, revenue, market and spoke
  counts, minimum borrow collateral.
- **Exporter health**: scrape duration, last success time, ledger sequence,
  time and skew, RPC errors, view failures, simulation count, build info.

### Caps and closed markets

A cap is always an enforced ceiling in asset units; no value means unlimited.
A cap of `0` closes that side to new supply or borrows. Caps are independent of
the collateral and borrow flags: a `0` cap on a side still flagged enabled is a
deliberate wind-down.

`lending_spoke_supply_closed` and `lending_spoke_borrow_closed` read `1` when
the supply or borrow cap is `0`. Read these gauges, not the data gap: a closed
side's cap utilization divides by zero and is not published, so on a graph it
looks like a failed scrape. Two alerts fire on closed-but-enabled when the
spoke is not deprecated.

## Configuration

`config/testnet.yaml` and `config/mainnet.yaml` list the controller address,
markets as `(hub_id, asset, symbol)`, and spoke ids to scan.

- **Contract addresses come from `configs/networks.json`**, the source of
  truth. Change it first, then copy the address here.
- **An unlisted market is never read.** When an asset is listed in
  `configs/<network>/markets.json`, add it here in the same commit. Do not list
  a market with `enabled: false` there: it is not deployed, so every read
  fails.
- **Spoke ids are on-chain ids.** On mainnet they differ from the ids in
  `configs/<network>/spokes.json`; `spoke_ids` in `configs/networks.json` maps
  them.
- **Name every hub and spoke.** A missing name shows as a bare `Spoke 7`, and a
  test fails.
- `symbol`, `hubs` and `spoke_names` are display labels only.
- `scrape_interval_seconds` defaults to `30`; below `5` stops startup.
- `rpc.timeout_seconds` is parsed but not applied.
- `contracts.xoxno_oracle_adapter` is validated at startup, but no scrape reads
  it.

### Environment variables

`--config` is the only flag; `EXPORTER_CONFIG` is its fallback. One is
required (no default in binary or image): without it the container exits and
`LendingExporterDown` fires, rather than reporting another network's data. The
other variables replace YAML values before validation. Empty
`EXPORTER_RPC_URL` or `EXPORTER_CONTROLLER` is ignored; empty
`EXPORTER_PRICE_AGGREGATOR` or `EXPORTER_XOXNO_ORACLE_ADAPTER` clears that
address.

| Variable | Overrides |
|---|---|
| `EXPORTER_CONFIG` | config path (required; image ships `/etc/lending-exporter/testnet.yaml` and `/etc/lending-exporter/mainnet.yaml`) |
| `EXPORTER_RPC_URL` | RPC URL |
| `EXPORTER_CONTROLLER` | controller address |
| `EXPORTER_PRICE_AGGREGATOR` | price-aggregator address |
| `EXPORTER_XOXNO_ORACLE_ADAPTER` | XOXNO oracle adapter address |
| `RUST_LOG` | `log.level`, when it parses as a filter; no fixed fallback |

`MAINNET_LENDING_CONTROLLER` is a Compose variable that feeds
`EXPORTER_CONTROLLER`; the binary does not read it.

## Deploy

Config files carry their network's addresses; no override needed.

```bash
docker compose -f docker-compose.example.yaml up -d lending-exporter-testnet
docker compose -f docker-compose.example.yaml --profile mainnet up -d lending-exporter-mainnet
```

Mainnet sits behind a profile so a plain `docker compose up` skips it. Add both
scrape jobs from `ops/prometheus.example.yml`. Every series except
`lending_exporter_simulations_total` has a `network` label.

Order for a Chainlink oracle source:

1. Deploy an exporter release that decodes Chainlink sources.
2. Upgrade governance and the price aggregator with `make <network> upgradeAll`.
3. Configure the Chainlink source.

Remove stored Chainlink config before downgrading any of these to a release without Chainlink support.

### Dashboard

Import `ops/grafana-dashboard.json`. Sections: Health status, Protocol,
Markets, Oracles, Spokes, Exporter health, Alerting.

Public dashboards reject template variables and panels without a fixed
datasource, so every query panel names one Prometheus datasource UID, and every
query outside Alerting filters on `network="mainnet"` (Alerting shows every
network). To import it:

1. Replace the committed datasource UID with `<your-prometheus-datasource-uid>`.
2. For a testnet copy, replace `network="mainnet"` with `network="testnet"`.
3. For a testnet copy, change the dashboard `uid` and `title`.

No panel names an asset, so a new market appears once the exporter reads it.

### Alerts

Rebuild `ops/alerts.yml` expressions as Grafana-managed alert rules, keeping
them off the public panels. Prefer soft-status flags over hard error codes.

## Code layout

| Path | Role |
|---|---|
| `src/main.rs` | startup, scrape loop, graceful shutdown |
| `src/config.rs` | YAML and environment configuration |
| `src/collector.rs` | one scrape cycle, with error isolation |
| `src/contract/` | typed decoders for pool, controller and oracle |
| `src/stellar/` | RPC client and read-only simulation |
| `src/model.rs` | RAY, WAD and BPS scaling, APY, deviation, staleness |
| `src/metrics.rs` | Prometheus families, `/metrics` and `/health` |
| `src/keys.rs`, `src/scval.rs` | XDR key builders and value readers |
