# lending-exporter

A read-only Prometheus exporter for the XOXNO Lending protocol on Soroban.

On a timer it reads the controller, pool, price-aggregator and oracle provider
contracts over Soroban RPC, then serves the results at `/metrics` for a public
Grafana dashboard.

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
| Controller | from the config file | market indexes, soft oracle status, spokes, minimum borrow collateral |
| Pool | asked from the controller each scrape | liquidity, rates, IRM parameters, last accrual |
| Price-aggregator | asked from the controller each scrape | oracle config |
| Oracle providers and LP pools | read from the oracle config each scrape | feed timestamps, LP share supply |

The controller and market asset addresses come from the config file. Every
other address is looked up on each scrape. If the price-aggregator lookup fails,
the address in the config file is used instead.

The controller reports a **soft oracle status** for every asset. This status is
the authority for solvency monitoring, because it always answers instead of
failing closed. Direct provider probes are an early warning only.

## What it publishes

All metric names start with the lending_ prefix. No per-user account data is
published.

**Oracle**, per asset — blended price, primary price, anchor price, deviation in
basis points, blend timestamp, three flags (stale, deviation and healthy), and
the error code behind an invalid price.

**Oracle config**, per asset — maximum staleness, tolerance band, sanity bounds,
source count, and how many seconds remain before a feed goes stale. An asset
priced by a single LP source also reports its pool value floor and LP share
supply.

**Market**, per hub and asset — supplied, borrowed, available liquidity and
revenue, in tokens and in USD. Also utilization, supply and borrow APY, the
interest-rate parameters, and the time since the last accrual.

**Spoke**, per listing — the paused, frozen, collateral, borrow and deprecated
flags. LTV, liquidation threshold, liquidation bonus and fees. Supply and borrow
caps, how much of each is used, and how full each is. Per spoke: the
liquidation target health factor, the health factor at the maximum bonus, and
the bonus factor. When a listing is removed on chain, or the exporter cannot
decode it, every series of that listing is removed.

**Protocol** — total value locked, total borrowed, liquidity, revenue, market
and spoke counts, and the minimum borrow collateral.

**Exporter health** — scrape duration, last success time, ledger sequence, time
and skew, RPC errors, view failures, simulation count and build info.

### Caps and closed markets

A cap is always an enforced ceiling, in asset units. There is no value that
means unlimited. A cap of `0` closes that side to new supply or new borrows, so
the market is closed.

Caps are independent of the collateral and borrow flags. A cap of `0` on a side
that is still flagged as enabled is a normal, deliberate wind-down.

Two metrics make this visible:

| Metric | Meaning |
|---|---|
| `lending_spoke_supply_closed` | `1` when the supply cap is `0` |
| `lending_spoke_borrow_closed` | `1` when the borrow cap is `0` |

Read these two gauges, not the gap in the data. While a market is closed its cap
utilization divides by zero and is not published. A closed market and a failed
scrape look the same on a graph. The two gauges are the only way to tell them
apart. Two alerts fire on the closed-but-enabled combination when the spoke is
not deprecated.

## Configuration

One YAML file per network: `config/testnet.yaml` and `config/mainnet.yaml`.
Each file lists the controller address, the markets to read as
`(hub_id, asset, symbol)`, and the spoke ids to scan.

Rules to follow:

- **Contract addresses come from `configs/networks.json`.** That file is the
  source of truth. Change it first, then copy the address here.
- **A market must be listed here or it is never read.** When an asset is listed
  in `configs/<network>/markets.json`, add it here in the same commit.
- **Spoke ids are the on-chain ids, not the config ids.** The ids in
  `configs/<network>/spokes.json` differ on mainnet, because one deferred spoke
  shifted every later id down by one. `configs/networks.json` holds the map
  between them.
- **Give every hub and spoke a name.** A missing name shows on the dashboard as
  a bare `Spoke 7`. A test fails if a name is missing.
- `symbol`, `hubs` and `spoke_names` are display labels only.
- `scrape_interval_seconds` defaults to `30`. A value below `5` stops startup.
- `rpc.timeout_seconds` is parsed but not applied.
- `contracts.xoxno_oracle_adapter` is validated at startup, but no scrape reads
  it.

Mainnet reads 25 of the 31 markets in `configs/mainnet/markets.json`. The six it
skips are the `SPIKO*` markets, which are disabled and not deployed.

### Environment variables

`--config` is the only command-line flag. `EXPORTER_CONFIG` sets the same path
when the flag is absent. The RPC and address variables replace YAML values
before validation. An empty `EXPORTER_RPC_URL` or `EXPORTER_CONTROLLER` is
ignored, so the committed value wins. An empty `EXPORTER_PRICE_AGGREGATOR` or
`EXPORTER_XOXNO_ORACLE_ADAPTER` clears that address.

| Variable | Overrides |
|---|---|
| `EXPORTER_CONFIG` | the config file path (binary default `/etc/lending-exporter/testnet.yaml`; the image sets `/etc/lending-exporter/mainnet.yaml`) |
| `EXPORTER_RPC_URL` | the RPC URL |
| `EXPORTER_CONTROLLER` | the controller address |
| `EXPORTER_PRICE_AGGREGATOR` | the price-aggregator address |
| `EXPORTER_XOXNO_ORACLE_ADAPTER` | the XOXNO oracle adapter address |
| `RUST_LOG` | `log.level` from the YAML, when it parses as a filter |

`MAINNET_LENDING_CONTROLLER` is not read by the binary. It is a Compose
variable that feeds `EXPORTER_CONTROLLER`.

## Deploy

Both networks ship their addresses in their config file, so no override is
needed.

```bash
docker compose -f docker-compose.example.yaml up -d lending-exporter-testnet
docker compose -f docker-compose.example.yaml --profile mainnet up -d lending-exporter-mainnet
```

Mainnet sits behind a profile so a plain `docker compose up` does not start it
by accident. Add both scrape jobs from `ops/prometheus.example.yml` to
Prometheus. Every series except `lending_exporter_simulations_total` carries a
`network` label.

### Dashboard

Import `ops/grafana-dashboard.json` into Grafana. Its sections are Health
status, Protocol, Markets, Oracles, Spokes, Exporter health, and Alerting.

The dashboard uses no template variables, because public dashboards reject
them. Every query outside the Alerting section is therefore pinned to
`network="mainnet"`; five Alerting panels show every network. Every query panel
is pinned to the production datasource UID `cfgw0aa7mups0d`, because public
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
