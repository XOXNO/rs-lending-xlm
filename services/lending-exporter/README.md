# lending-exporter

A read-only Prometheus exporter for the XOXNO Lending protocol on Soroban.

On a timer it reads the controller, pool and price-aggregator contracts over
Soroban RPC, then serves the results at `/metrics` for a public Grafana
dashboard.

It holds no signer and writes nothing. It only simulates read-only calls and
reads ledger entries. It is a standalone Cargo workspace, shipped as its own
container, with one instance per network.

## Quick start

```bash
cargo run -- --config config/testnet.yaml
curl -s localhost:9110/metrics | grep lending_
```

## How it reads the protocol

| Source | How it is found | What it gives |
|---|---|---|
| Controller | from the config file | market indexes, spokes, minimum borrow collateral |
| Pool | asked from the controller each scrape | liquidity, rates, IRM parameters, last accrual |
| Price-aggregator | asked from the controller each scrape | oracle config and feed freshness |

Only the pool and price-aggregator addresses are looked up live. If the
price-aggregator lookup fails, the address in the config file is used instead.

The controller reports a **soft oracle status** for every asset. This status is
the authority for solvency monitoring, because it always answers instead of
failing closed. Direct provider probes are an early warning only. Trust the soft
status first.

## What it publishes

All metric names start with the lending_ prefix. No per-user account data is
published.

**Oracle**, per asset — blended price, primary price, anchor price, deviation in
basis points, blend timestamp, and three flags: stale, deviation and healthy.

**Oracle config**, per asset — maximum staleness, tolerance band, sanity bounds,
and how many seconds remain before a feed goes stale.

**Market**, per hub and asset — supplied, borrowed, available liquidity and
revenue, in tokens and in USD. Also utilization, supply and borrow APY, the
interest-rate parameters, and the time since the last accrual.

**Spoke**, per listing — the paused, frozen, collateral, borrow and deprecated
flags. LTV, liquidation threshold, liquidation bonus and fees. Supply and borrow
caps, how much of each is used, and how full each is.

**Protocol** — total value locked, total borrowed, liquidity, revenue, market
and spoke counts, and the minimum borrow collateral.

**Exporter health** — scrape duration, last success time, ledger skew, RPC
errors, view failures and build info.

### Caps and closed markets

A cap is always an enforced ceiling, in asset units. There is no value that
means unlimited. A cap of `0` means that side accepts nothing, so the market is
closed.

Caps are independent of the collateral and borrow flags. A cap of `0` on a side
that is still flagged as enabled is a normal, deliberate wind-down.

Two metrics make this visible:

| Metric | Meaning |
|---|---|
| `lending_spoke_supply_closed` | `1` when the supply cap is `0` |
| `lending_spoke_borrow_closed` | `1` when the borrow cap is `0` |

Read these two gauges, not the gap in the data. While a market is closed its cap
utilization is `0/0`, so it is **not published**. A closed market and a failed
scrape look the same on a graph. The two gauges are the only way to tell them
apart, and two alerts fire on the closed-but-enabled combination.

## Configuration

One YAML file per network: `config/testnet.yaml` and `config/mainnet.yaml`.
Each file lists the controller address, the markets to read as
`(hub_id, asset, symbol)`, and the spoke ids to scan.

Rules to follow:

- **Addresses come from `configs/networks.json`.** That file is the source of
  truth. Change it first, then copy the address here.
- **A market must be listed here or it is never read.** When an asset is listed
  in the protocol config, add it here in the same commit.
- **Spoke ids are the on-chain ids, not the ids in the protocol config.** The
  two differ, because one deferred spoke shifted every later id down by one.
  `configs/networks.json` holds the map between them.
- **Give every hub and spoke a name.** A missing name shows on the dashboard as
  a bare `Spoke 7`. A test fails if a name is missing.
- `symbol`, `hubs` and `spoke_names` are display labels only.
- `scrape_interval_seconds` defaults to `30`. A value below `5` stops startup.
- `rpc.timeout_seconds` is read but does nothing. It is not applied yet.

Mainnet currently reads 25 of the 31 markets in the protocol config. The six it
skips are the `SPIKO*` markets, which are disabled and not deployed.

### Environment variables

`--config` is the only command-line flag. These variables override the YAML file
before it is checked. An empty value is ignored, so the committed address wins.

| Variable | Overrides |
|---|---|
| `EXPORTER_CONFIG` | the config file path |
| `EXPORTER_RPC_URL` | the RPC URL |
| `EXPORTER_CONTROLLER` | the controller address |
| `EXPORTER_PRICE_AGGREGATOR` | the price-aggregator address |
| `EXPORTER_XOXNO_ORACLE_ADAPTER` | the XOXNO oracle adapter address |
| `RUST_LOG` | the log level from the YAML |

`MAINNET_LENDING_CONTROLLER` is **not** read by the binary. It is a Compose
variable that feeds `EXPORTER_CONTROLLER`.

## Deploy

Both networks ship their addresses in their config file, so no override is
needed.

```bash
docker compose up -d lending-exporter-testnet
docker compose --profile mainnet up -d lending-exporter-mainnet
```

Mainnet sits behind a profile so a plain `docker compose up` does not start it
by accident. Add both scrape jobs from `ops/prometheus.example.yml` to
Prometheus. Every series carries a `network` label.

### Dashboard

Import `ops/grafana-dashboard.json` into Grafana. Its sections are Health,
Protocol, Markets, Oracles, Spokes, Exporter health, and Alerting.

The dashboard uses no template variables, because public dashboards reject
them. Every query is therefore pinned to `network="mainnet"`, and every panel is
pinned to the production datasource UID `cfgw0aa7mups0d`, because public
dashboards also reject panels without a fixed datasource.

To reuse it elsewhere, replace that UID. For a testnet copy, also replace
`network="mainnet"` with `network="testnet"` and change the `uid` and `title`.

No panel names an asset, so a newly listed market appears on its own once the
exporter reads it.

### Alerts

Rebuild the expressions from `ops/alerts.yml` as Grafana-managed alert rules, so
they stay internal and off the public panels. Prefer the soft-status flags over
hard error codes.

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
