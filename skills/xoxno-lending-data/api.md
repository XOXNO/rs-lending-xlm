# XOXNO Lending REST API

Public API for wallet screens and historical analytics.

| Network | Base URL |
| --- | --- |
| Mainnet | `https://api.xoxno.com` |
| Testnet | `https://testnet-api.xoxno.com` |

Use API and RPC of the same network. No route needs an API key.

## Integrator v1 arrays

For wallet, web, and React Native screens:

| Route | Body | HTTP cache |
| --- | --- | --- |
| `GET /stellar-lending/v1/assets` | `LendingAsset[]`; collateral listings | `public, s-maxage=10` |
| `GET /stellar-lending/v1/assets?usage=borrow` | `LendingAsset[]`; borrow listings | `public, s-maxage=10` |
| `GET /stellar-lending/v1/users/{owner}/positions` | `LendingPosition[]`; one item per indexed owned NFT | `no-store` |

Both roots serve them. `owner` must be a valid Stellar `G...` or `C...`
address; invalid owner, usage, page size, or cursor returns HTTP 400.

Positions carry `supplied` and `borrow` arrays, `healthFactor`,
`borrowLimitUsd`, `availableBorrowUsd`, `spokeName`, `netApy`, and `nftImage`.
Token legs carry `sac`, `amountRaw`, `amount`, `decimals`, `apy`, `valueUsd`,
`priceUsd`, `logoUrl`, `name`, and `symbol`. Raw amounts are integer base-unit
strings (no RAY conversion); APYs are fractions; `null` means unavailable, and
`healthFactor` is also null without debt, so check `hasDebt`
([full rules](../xoxno-lending-sdk/reads.md#units-and-missing-data)).

Assets identify `(hubId, sac)` markets with matching `spokes`. Each spoke has
LTV in BPS, supply/borrow `Capacity` objects (`amountRaw`, `amount`, `usd`),
and action flags: snapshot estimates, not full admission checks.

Positions take `limit` 1–100 (default 50) and an opaque `cursor`. Follow
`Link` `rel="next"` until absent, even after an empty page. Resolve relative
links against the current URL; keep the same endpoint and detect cycles.
Pages are not one atomic ledger snapshot.

Backend caches: asset bodies 5 s, position bodies 3 s, token metadata 600 s.
Indexed NFT inventory is queried on every wallet request before a position
body is reused. These TTLs do not bound total age across indexing, oracle
state, metadata, and HTTP caches. Ownership is indexed, not live.

`@xoxno/sdk-js/stellar-lending/read` (pinned SDK) exports the public types,
schemas, and optional pagination client. The
[API reference](https://xoxno.com/docs/stellar-lending/dev/integrator-api)
defines every field and missing-data rule. No supply endpoint exists: build
supply synchronously in the SDK, then prepare with RPC.

## Historical analytics

For charts and accounting; current positions and asset selection use v1.

| Parameter | Format |
| --- | --- |
| `from`, `to` | UTC calendar date, `yyyy-MM-dd` |
| `bin` | `Nd`, `Nh`, or `Nm`, e.g. `1d`, `4h`, `15m` |
| `spokeId`, `hubId` | On-chain integer ID, at least 1 |
| `asset`, `token` | Token contract address, `C...`; URL-encode path values |
| `scope` | `asset`, `hub`, or `protocol` |

Every route below sends `s-maxage=120` and permits stale responses during
revalidation. Respect the cache headers; expiry does not prove indexed data
matches the ledger.

APYs and utilization are fractions (`0.05` = 5%). USD values are display
numbers; keep integer strings for exact arithmetic.

### History graphs

| Route | Params | DTO | Meaning |
|---|---|---|---|
| `GET /stellar-lending/reserves/{spokeId}/{hubId}/{asset}/graph` | `?from&to&bin` | `MarketGraphDto` | per-market history: `points[]` with `supplyApy`, `borrowApy`, `utilization`, `totalDepositsUsd`, `totalBorrowsUsd`, `availableLiquidityUsd`, `usdPrice`; optional `fees[]` |
| `GET /stellar-lending/assets/{asset}/graph` | `?from&to&bin` | `MarketGraphDto` | asset across hubs |
| `GET /stellar-lending/hubs/{hubId}/graph` | `?from&to&bin` | `MarketGraphDto` | hub |
| `GET /stellar-lending/spokes/{spokeId}/graph` | `?from&to&bin` | `SpokeGraphDto` | spoke-attributed `suppliedUsd`, `borrowedUsd`, `*Native`, APYs |
| `GET /stellar-lending/stats/history` | `?from&to&bin` | `StellarStatsHistoryDto` | protocol-wide parallel arrays `timestamps[]`, `supplied[]`, `borrowed[]`, `revenue[]` (USD) |

### Analytics series

| Route | Params | DTO | Meaning |
|---|---|---|---|
| `GET /stellar-lending/revenue` | `?from&to&bin&scope=protocol` | `RevenueSeriesDto` | reserve (interest) revenue: `cumulativeUsd`, `deltaUsd`, `cumulativeNative` |
| `GET /stellar-lending/revenue/fees` | `?from&to&bin&hubId?` | `FeeRevenueSeriesDto` | flash-loan + strategy fees per `action`: `feeNative`, `feeUsd` |
| `GET /stellar-lending/volume` | `?from&to&bin&hubId?&token?` | `VolumeSeriesDto` | supply/borrow/withdraw/repay volume, `netFlowUsd` |
| `GET /stellar-lending/liquidations` | `?from&to&bin&hubId?` | `LiquidationsSeriesDto` | `repaidUsd`, `seizedUsd` (gross), `creditedNetUsd` (net, Credit mode only), `liquidations` count |
| `GET /stellar-lending/liquidations/leaderboard` | `?top=25&hubId?&token?` | `LiquidationsLeaderboardDto` | per-liquidator counts, `seizedUsd`, `creditedNetUsd` |
| `GET /stellar-lending/participants` | `?hubId?&spokeId?&token?` | `ParticipantCountsDto` | distinct `suppliers`, `borrowers`, `total` |
| `GET /stellar-lending/active-users` | `?from&to&bin` | `ActiveUsersSeriesDto` | `activeAccounts`, `activeOwners`, `newUsers` per bin |
| `GET /stellar-lending/distribution` | `?side=deposits&hubId?&spokeId?&token?` | `HolderDistributionDto` | holder count, avg/median/p90, top-1/top-10 share |
| `GET /stellar-lending/rate-spread` | `?from&to&bin&hubId?&token?` | `RateSpreadSeriesDto` | `borrowApy - supplyApy` per (hub, token) |
| `GET /stellar-lending/defillama` | `?from&to&bin` | `DefiLlamaDimensionsDto` | per bin: `tvl`, `borrowed`, `fees` (borrow interest accrued), `revenue` (reserve revenue accrued), USD |

Reserve graph `points[]` describe the shared `(hubId, asset)` pool across
spokes; `spokeId` filters only optional `fees[]`. Spoke-attributed balances
come from `/stellar-lending/spokes/{spokeId}/graph`.

The graph, stats/history, revenue, and defillama routes can extend to the
current bin when `to` reaches today; other series stop at `to`. A current bin
can change as the indexer receives new data.

Historical REST data is derived: it cannot prove raw event order or recover
omitted event payloads. Use an archival event source for those.

## Example: market history in Python

```python
import json
import urllib.parse
import urllib.request
from datetime import datetime, timedelta, timezone

BASE = "https://api.xoxno.com"


def get(path: str, **params: str) -> dict | list:
    url = f"{BASE}{path}"
    if params:
        url += "?" + urllib.parse.urlencode(params)
    request = urllib.request.Request(url, headers={"Accept": "application/json"})
    with urllib.request.urlopen(request, timeout=30) as response:
        return json.load(response)


def market_for(sac: str, spoke_id: int, hub_id: int) -> dict:
    assets = get("/stellar-lending/v1/assets")
    matches = [
        asset for asset in assets
        if asset["sac"] == sac and asset["hubId"] == hub_id
        and any(spoke["spokeId"] == spoke_id for spoke in asset["spokes"])
    ]
    if len(matches) != 1:
        raise LookupError((spoke_id, hub_id, sac))
    return matches[0]


# Get these coordinates from trusted configuration or explicit user selection.
asset = "C..."
spoke_id = 1
hub_id = 1
market = market_for(asset, spoke_id, hub_id)

# The last 90 days, as UTC calendar dates (yyyy-MM-dd).
today = datetime.now(timezone.utc).date()
window = {"from": (today - timedelta(days=90)).isoformat(), "to": today.isoformat(), "bin": "1d"}

graph = get(
    f"/stellar-lending/reserves/{spoke_id}/{hub_id}/{urllib.parse.quote(asset, safe='')}/graph",
    **window,
)
borrow_apy = [(point["timestamp"], point["borrowApy"]) for point in graph["points"]]
dimensions = get("/stellar-lending/defillama", **window)["points"]
```

SDK pin: [Verified against](../xoxno-lending/SKILL.md#verified-against).
