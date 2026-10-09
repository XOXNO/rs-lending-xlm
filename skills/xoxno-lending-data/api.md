# XOXNO Lending REST API

Use the public API for wallet screens and historical analytics.

| Network | Base URL |
| --- | --- |
| Mainnet | `https://api.xoxno.com` |
| Testnet | `https://testnet-api.xoxno.com` |

Select the API and RPC for the same network. These routes require no API key.
The SDK owns the public v1 types and schemas.

## Integrator v1 arrays

Use these routes for wallet, web, and React Native screens:

| Route | Body | HTTP cache |
| --- | --- | --- |
| `GET /stellar-lending/v1/assets` | `LendingAsset[]`; collateral listings | `public, s-maxage=10` |
| `GET /stellar-lending/v1/assets?usage=borrow` | `LendingAsset[]`; borrow listings | `public, s-maxage=10` |
| `GET /stellar-lending/v1/users/{owner}/positions` | `LendingPosition[]`; one item per indexed owned NFT | `no-store` |

These routes are deployed on both network roots above. They require no API key.
`owner` must be a valid Stellar `G...` or `C...` address. Invalid owner, usage,
page size, or cursor returns HTTP 400.

Positions contain `supplied` and `borrow` arrays, `healthFactor`,
`borrowLimitUsd`, `availableBorrowUsd`, `spokeName`, `netApy`, and `nftImage`.
Each token leg includes `sac`, `amountRaw`, `amount`, `decimals`, `apy`,
`valueUsd`, `priceUsd`, `logoUrl`, `name`, and `symbol`.
Raw amounts are integer token base-unit strings; do not apply RAY conversion.
APYs are fractions. Nullable values mean unavailable; `healthFactor` is also
null without debt, so inspect `hasDebt`.

Assets identify `(hubId, sac)` markets and contain matching `spokes`.
Each spoke includes LTV in BPS, supply/borrow `Capacity` objects, and action
flags. Each capacity has `amountRaw`, `amount`, and `usd`. Flags and capacities
are snapshot estimates, not complete contract admission checks.

Positions use `limit` 1–100, default 50, and an opaque `cursor`.
Follow `Link` with `rel="next"` until absent, including after an empty page.
Resolve relative links against the current URL; keep the same endpoint and
detect cycles. Several pages are not one atomic ledger snapshot.

Backend caches asset bodies for 5 seconds, position bodies for 3 seconds,
and token metadata for 600 seconds. Indexed NFT inventory is queried before
reusing a wallet body. These TTLs do not bound total age across indexing,
oracle state, metadata, and HTTP caches. Ownership is indexed, not live.

Public types, schemas, and the optional pagination client are exported from
`@xoxno/sdk-js/stellar-lending/read` in SDK `1.0.228`. The published
[API reference](https://xoxno.com/docs/stellar-lending/dev/integrator-api)
defines every field and missing-data rule. Build supply synchronously in the
SDK, then prepare with RPC; there is no supply API endpoint.

## Historical analytics

Use the following routes for charts and accounting. Use the v1 arrays above
for current wallet positions and asset selection.

| Parameter | Format |
| --- | --- |
| `from`, `to` | UTC calendar date, `yyyy-MM-dd` |
| `bin` | `Nd`, `Nh`, or `Nm`; for example, `1d`, `4h`, or `15m` |
| `spokeId`, `hubId` | On-chain integer ID, at least 1 |
| `asset`, `token` | Token contract address, `C...`; URL-encode path values |
| `scope` | `asset`, `hub`, or `protocol` |

The cache column gives `s-maxage` in seconds. These routes also permit stale
responses during revalidation. Cache expiry does not prove that indexed data
matches the current ledger. Respect the response cache headers.

APYs and utilization are fractions: `0.05` means 5%. USD values are display
numbers. Keep integer strings when exact arithmetic is required.

### History graphs

| Route | Params | DTO | Meaning | Cache |
|---|---|---|---|---|
| `GET /stellar-lending/reserves/{spokeId}/{hubId}/{asset}/graph` | `?from&to&bin` | `MarketGraphDto` | per-market history: `points[]` with `supplyApy`, `borrowApy`, `utilization`, `totalDepositsUsd`, `totalBorrowsUsd`, `availableLiquidityUsd`, `usdPrice`; optional `fees[]` | 120 |
| `GET /stellar-lending/assets/{asset}/graph` | `?from&to&bin` | `MarketGraphDto` | asset across hubs | 120 |
| `GET /stellar-lending/hubs/{hubId}/graph` | `?from&to&bin` | `MarketGraphDto` | hub | 120 |
| `GET /stellar-lending/spokes/{spokeId}/graph` | `?from&to&bin` | `SpokeGraphDto` | spoke-attributed `suppliedUsd`, `borrowedUsd`, `*Native`, APYs | 120 |
| `GET /stellar-lending/stats/history` | `?from&to&bin` | `StellarStatsHistoryDto` | protocol-wide parallel arrays `timestamps[]`, `supplied[]`, `borrowed[]`, `revenue[]` (USD) | 120 |

### Analytics series

| Route | Params | DTO | Meaning | Cache |
|---|---|---|---|---|
| `GET /stellar-lending/revenue` | `?from&to&bin&scope=protocol` | `RevenueSeriesDto` | reserve (interest) revenue: `cumulativeUsd`, `deltaUsd`, `cumulativeNative` | 120 |
| `GET /stellar-lending/revenue/fees` | `?from&to&bin&hubId?` | `FeeRevenueSeriesDto` | flash-loan + strategy fees per `action`: `feeNative`, `feeUsd` | 120 |
| `GET /stellar-lending/volume` | `?from&to&bin&hubId?&token?` | `VolumeSeriesDto` | supply/borrow/withdraw/repay volume, `netFlowUsd` | 120 |
| `GET /stellar-lending/liquidations` | `?from&to&bin&hubId?` | `LiquidationsSeriesDto` | `repaidUsd`, `seizedUsd` (gross), `creditedNetUsd` (net, Credit mode only), `liquidations` count | 120 |
| `GET /stellar-lending/liquidations/leaderboard` | `?top=25&hubId?&token?` | `LiquidationsLeaderboardDto` | per-liquidator counts, `seizedUsd`, `creditedNetUsd` | 120 |
| `GET /stellar-lending/participants` | `?hubId?&spokeId?&token?` | `ParticipantCountsDto` | distinct `suppliers`, `borrowers`, `total` | 120 |
| `GET /stellar-lending/active-users` | `?from&to&bin` | `ActiveUsersSeriesDto` | `activeAccounts`, `activeOwners`, `newUsers` per bin | 120 |
| `GET /stellar-lending/distribution` | `?side=deposits&hubId?&spokeId?&token?` | `HolderDistributionDto` | holder count, avg/median/p90, top-1/top-10 share | 120 |
| `GET /stellar-lending/rate-spread` | `?from&to&bin&hubId?&token?` | `RateSpreadSeriesDto` | `borrowApy - supplyApy` per (hub, token) | 120 |
| `GET /stellar-lending/defillama` | `?from&to&bin` | `DefiLlamaDimensionsDto` | per bin: `tvl`, `borrowed`, `fees` (borrow interest accrued), `revenue` (reserve revenue accrued), USD | 120 |

Reserve graph `points[]` describe the shared `(hubId, asset)` pool across
spokes. `spokeId` filters optional `fees[]`, not pool balances. Use
`/stellar-lending/spokes/{spokeId}/graph` for spoke-attributed balances.

The graph, stats/history, revenue, and defillama routes can extend to the
current bin when `to` reaches today. Other series stop at `to`. A current bin
can change as the indexer receives new data.

Historical REST data is derived. It cannot prove raw event order or recover
omitted event payloads. Use an archival event source for those tasks.

## Example: market history in Python

```python
import json
import urllib.parse
import urllib.request

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
graph = get(
    f"/stellar-lending/reserves/{spoke_id}/{hub_id}/{urllib.parse.quote(asset, safe='')}/graph",
    **{"from": "2026-01-01", "to": "2026-10-01", "bin": "1d"},
)
borrow_apy = [(point["timestamp"], point["borrowApy"]) for point in graph["points"]
dimensions = get(
    "/stellar-lending/defillama",
    **{"from": "2026-01-01", "to": "2026-10-01", "bin": "1d"},
)["points"]
```

Source: `xoxno-api-v2` commit `8d891f562b0322d659c6ea0ccc3a1f168afb4d03`.
Public types: `@xoxno/sdk-js` version `1.0.228`.
