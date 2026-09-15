# XOXNO Lending REST API (`/stellar-lending/*`)

Endpoint reference for consumers in any language. The Rust `lending-api` crate in
`rs-lending-xlm` is **not** the production API; do not target it.

## Base URLs, auth, limits, caching

| Item | Value | Source |
|---|---|---|
| Mainnet | `https://api.xoxno.com` | `xoxno-ui/src/lib/blockchain/network.ts` `apiHosts.mainnet.public` |
| Testnet | `https://testnet-api.xoxno.com` | `apiHosts.testnet.public` |
| Path prefix | none; routes are absolute `/stellar-lending/...` (`@Controller()` with no prefix, no global prefix) | controller |
| Auth | none — no header, no API key on any route; only a global `ThrottlerGuard` (30 requests / 3 s per client, `app-core.module.ts`) | `app-core.module.ts` |
| Caching | every route sets `Cache-Control: public, s-maxage=N, stale-while-revalidate=N`; `N` is listed per route below. Poll no faster than `s-maxage` — the edge serves the same body until then. `/assets/{asset}/page` sets it only when `owner` is absent | controller `@Header` |
| Content | JSON; bigint-precision fields are decimal **strings** (`*Ray`, `*Wad`, caps, `amount`); display fields are JS numbers (`*Short`, `*Usd`, `*Apy`, `utilization`) | DTOs |

Only one deployment serves both chains per environment: any app pinned to Stellar testnet
uses `testnet-api.xoxno.com` for everything (`network.ts` `apiEnvironment`).

## Parameter conventions

| Parameter | Format | Validation (`src/utils/pipes/common.pipe.ts`) |
|---|---|---|
| `from`, `to` | `yyyy-MM-dd` (calendar date, UTC) | `ParseDatePipe` regex `^\d{4}-\d{2}-\d{2}$` → 400 otherwise. Not ISO timestamps, not unix seconds |
| `bin` | `Nd`, `Nh`, or `Nm` — `1d`, `4h`, `15m` | `ParseTimeSpanPipe` regex `^\d+[dhm]$`; `1w` and `1M` are rejected |
| `spokeId`, `hubId` path params | integers ≥ 1 (spoke 0 and hub 0 do not exist; id 0 is the "create account" sentinel on-chain) | `ExtendedParseIntPipe` |
| `hubId`, `spokeId` query filters | integer; omit or `-1` for "all" | `DefaultValuePipe(-1)` |
| `asset`, `token` | Soroban contract address `C...` (SAC or custom token); URL-encode it | `@Param('asset')` |
| `owner` | Stellar `G...` (or `C...` for contract owners) | string |
| `accountId` | decimal position-NFT token id, as a string | string |
| `skip`, `top` | offset pagination; `top` capped per route (see table) | `ExtendedParseIntPipe(min, max)` |
| `continuationToken` | opaque Cosmos cursor; pass back the previous response's `continuationToken` until `hasMoreResults` is false | `/governance/proposals` only |
| `side` | `deposits` \| `borrows` (holders, distribution) or `deposit` \| `borrow` (asset markets) | `ParseEnumPipe` |
| `scope` | `asset` \| `hub` \| `protocol` | `StellarAnalyticsScope` |

Series endpoints return one point per bin between `from` and `to`. For balance series
(supplied / borrowed / reserve revenue) a `to` that reaches today extends to the bin
containing `now()`, revalued at the live index (`stellar-lending.graphs.ts`); fee series
stop at the last closed bin.

REST responses are indexed, derived views. They can seed balances, charts, candidate
watchlists, or recovery state, but they do not reconstruct canonical raw Soroban events,
their exact ordering, or omitted payloads unless an independent archival source proves that
provenance.

Resolve markets contract-address-first. Start from an asset contract address in the selected
network configuration or an explicit user selection, then choose an exact
`(spokeId, hubId, asset)` reserve. Symbols and names are display metadata only: never choose
the first symbol match or first reserve result.

## Endpoint table

Cache = `s-maxage` seconds. DTO names are the NestJS classes, identical in
`lending-api-types.ts`.

### Lists, context, live state

| Route | Params | DTO | Meaning | Cache |
|---|---|---|---|---|
| `GET /stellar-lending/assets` | — | `StellarAssetListItemDto[]` | every listed asset with cross-hub totals, APY ranges, `hubCount`, `marketCount` | 30 |
| `GET /stellar-lending/hubs` | — | `StellarHubListItemDto[]` | per-hub `tvlUsd`, deposits, borrows, counts | 30 |
| `GET /stellar-lending/spokes` | — | `StellarSpokeListItemDto[]` | per-spoke totals and `connectedHubIds` | 30 |
| `GET /stellar-lending/reserves` | `?hubId&spokeId&asset` (all optional) | `StellarReserveListItemDto[]` | every `spoke × hub × asset` market row with `utilizationRate`, `totalDepositsUsd`, `totalBorrowsUsd`; filterable | 30 |
| `GET /stellar-lending/context` | — | `StellarLendingContextDto` | `assets`, `hubs`, `spokes`, `reserves`, `reserveDetailsByKey` in one call | 30 |
| `GET /stellar-lending/live-state` | — | `StellarLendingLiveStateDto` | live `indexes: StellarMarketIndexByHub[]` (`supplyIndex`/`borrowIndex` raw RAY + `*Short`), `minBorrowCollateralUsdWad` | 10 |
| `GET /stellar-lending/markets/detailed` | — | `StellarDetailedMarketDto[]` | live indexes plus composed oracle price with `stale`, `deviation`, `valid` flags | 30 |

### Entity detail and page views

| Route | Params | DTO | Meaning | Cache |
|---|---|---|---|---|
| `GET /stellar-lending/assets/{asset}` | — | `AssetDto` | header totals, `min/maxSupplyApy`, `min/maxBorrowApy`, `oracleProvider` | 30 |
| `GET /stellar-lending/assets/{asset}/page` | `?from&to&bin&owner?` | `AssetPageDto` | `AssetDto` + `depositMarkets`, `borrowMarkets`, `graphSeries` | 30 (no `owner`) |
| `GET /stellar-lending/assets/{asset}/markets` | `?side=deposit\|borrow` | `AssetMarketDto[]` | per spoke/hub markets for one asset | 30 |
| `GET /stellar-lending/hubs/{hubId}` | — | `HubDto` | totals, `utilization`, `assets: HubAssetDto[]` | 30 |
| `GET /stellar-lending/hubs/{hubId}/page` | `?from&to&bin` | `HubPageDto` | `HubDto` + `graph: MarketGraphDto` + `holders` | 30 |
| `GET /stellar-lending/spokes/{spokeId}` | — | `SpokeDto` | `connectedHubs`, `markets`, liquidation curve (`liquidationTargetHfWad`, `healthFactorForMaxBonusWad`) | 30 |
| `GET /stellar-lending/spokes/{spokeId}/page` | `?from&to&bin` | `SpokePageDto` | `SpokeDto` + `graph: SpokeGraphDto` + `holders` | 30 |
| `GET /stellar-lending/reserves/{spokeId}/{hubId}/{asset}` | — | `ReserveDto` | one market in one spoke: APYs, `utilization`, caps, `irm` curve (RAY strings), `liveSupplyIndexRay`, `liveBorrowIndexRay`, risk BPS. `suppliedShort` / `borrowedShort` are the spoke's slice; `hubPool` and `utilization` are hub-wide | 30 |
| `GET /stellar-lending/reserves/{spokeId}/{hubId}/{asset}/page` | `?from&to&bin` | `ReservePageDto` | `ReserveDto` + `graph: MarketGraphDto` + `holders: HoldersBothSidesDto` | 30 |

### Holders

| Route | Params | DTO | Meaning | Cache |
|---|---|---|---|---|
| `GET /stellar-lending/reserves/{spokeId}/{hubId}/{asset}/holders` | `?side=deposits\|borrows` | `TopHoldersDto` | top accounts: `scaledRay`, `amountShort`, `sharePct`; `totalScaledRay` | 120 |
| `GET /stellar-lending/hubs/{hubId}/holders` | `?side` | `TopHoldersDto` | same, hub-wide | 120 |
| `GET /stellar-lending/spokes/{spokeId}/holders` | `?side` | `TopHoldersDto` | same, spoke-wide | 120 |

### History graphs

| Route | Params | DTO | Meaning | Cache |
|---|---|---|---|---|
| `GET /stellar-lending/reserves/{spokeId}/{hubId}/{asset}/graph` | `?from&to&bin` | `MarketGraphDto` | per-market history: `points[]` with `supplyApy`, `borrowApy`, `utilization`, `totalDepositsUsd`, `totalBorrowsUsd`, `availableLiquidityUsd`, `usdPrice`; optional `fees[]` | 120 |
| `GET /stellar-lending/assets/{asset}/graph` | `?from&to&bin` | `MarketGraphDto` | asset across hubs | 120 |
| `GET /stellar-lending/hubs/{hubId}/graph` | `?from&to&bin` | `MarketGraphDto` | hub | 120 |
| `GET /stellar-lending/spokes/{spokeId}/graph` | `?from&to&bin` | `SpokeGraphDto` | spoke-attributed `suppliedUsd`, `borrowedUsd`, `*Native`, APYs | 120 |
| `GET /stellar-lending/stats/history` | `?from&to&bin` | `StellarStatsHistoryDto` | protocol-wide parallel arrays `timestamps[]`, `supplied[]`, `borrowed[]`, `revenue[]` (USD) | 120 |

### Users, accounts, positions

| Route | Params | DTO | Meaning | Cache |
|---|---|---|---|---|
| `GET /stellar-lending/users/{owner}/positions` | — | `AccountPositionsDto` | all positions for a wallet across spokes/hubs | 5 |
| `GET /stellar-lending/accounts/{accountId}/positions` | — | `AccountPositionsDto` | positions of one account (NFT id) | 5 |
| `GET /stellar-lending/users/{owner}/activity` | `?skip=0&top=50` (top ≤ 200) | `StellarUserActivityItemDto[]` | action feed, newest first: `action`, `side`, `token`, `amountShort`, `usd`, `liquidator`, `seq` | 30 |
| `GET /stellar-lending/users/{accountId}/history` | `?from&to&bin` | `UserHistoryDto` | per-account supplied/borrowed series per token (`points[].side`, `native`, `usd`) | 120 |
| `GET /stellar-lending/pnl` | `?accountId=` | `StellarPositionsPnlDto` | realized + unrealized PnL per asset: `pnlUsd`, `pnlToken`, `interestUsd`, `debtUsd` | 30 |
| `GET /stellar-lending/pnl/scope` | `?scope=asset\|hub\|protocol` | `PnlByScopeDto` | cross-account PnL grouped by scope | 120 |
| `GET /stellar-lending/positions` | `?token&orderBy=Supplied\|Borrowed\|HealthFactor&orderDirection=asc\|desc&skip&top` (top ≤ 100) | `StellarPositionsRankDto` | wallet leaderboard with `healthFactor` | 120 |

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

### Governance and campaign

| Route | Params | DTO | Meaning | Cache |
|---|---|---|---|---|
| `GET /stellar-lending/governance/proposals` | `?top=25&continuationToken?` (top ≤ 100) | `GovernanceProposalsPageDto` | `resources[]`, `hasMoreResults`, `continuationToken` | 60 |
| `GET /stellar-lending/campaign/leaderboard` | `?skip&top` (top ≤ 100) | `StellarCampaignLeaderboardDto` | airdrop campaign rows | 120 |
| `GET /stellar-lending/campaign/me` | `?owner=` | `StellarCampaignMeDto` | one wallet's campaign row | 120 |

`swagger.json` / `lending-api-types.ts` also list `GET /users/{owner}/assets/{asset}/balance` and `GET /users/{owner}/activity/page`; neither exists in `stellar-lending.controller.ts`, and `/reserves/{spokeId}/{hubId}/{asset}/page` is the reverse case.
The controller is authoritative.

## Field scales

`AccountPositionDto` (`/users/{owner}/positions`, `/accounts/{accountId}/positions`) `supplyAmount` / `borrowAmount` are RAY token quantities, not base units: [math.md#api-position-fields-are-ray-quantities](../xoxno-lending/math.md#api-position-fields-are-ray-quantities).
`supplyCap` / `borrowCap` are base-unit strings (`0` = closed); `supplyCapShort` / `borrowCapShort` are human-scaled.
`utilization` and `supplyApy` / `borrowApy` scales: [reads.md#dto-field-semantics](../xoxno-lending-sdk/reads.md#dto-field-semantics).

## Example: DefiLlama-style history in Python (stdlib only)

```python
import json
import urllib.parse
import urllib.request

BASE = "https://api.xoxno.com"  # testnet: https://testnet-api.xoxno.com


def get(path: str, **params: str) -> dict | list:
    url = f"{BASE}{path}"
    if params:
        url += "?" + urllib.parse.urlencode(params)
    with urllib.request.urlopen(urllib.request.Request(url, headers={"Accept": "application/json"})) as r:
        return json.load(r)  # no auth header; respect Cache-Control s-maxage (120 s on history routes)


def reserve_for(asset: str, spoke_id: int, hub_id: int) -> dict:
    """Resolve one explicit market coordinate; never select the first symbol/result."""
    matches = [
        row
        for row in get("/stellar-lending/reserves", asset=asset, spokeId=str(spoke_id), hubId=str(hub_id))
        if row["asset"] == asset and row["spokeId"] == spoke_id and row["hubId"] == hub_id
    ]
    if len(matches) != 1:
        raise LookupError((spoke_id, hub_id, asset))
    return matches[0]


# Load these three coordinates from configs/networks.json / addresses.md or explicit user input.
asset = "C..."  # asset contract address, not a display-symbol lookup
spoke_id = 1
hub_id = 1
reserve = reserve_for(asset, spoke_id, hub_id)

# Per-market borrow APY history: from/to are yyyy-MM-dd, bin is Nd / Nh / Nm.
graph = get(
    f"/stellar-lending/reserves/{spoke_id}/{hub_id}/{urllib.parse.quote(asset, safe='')}/graph",
    **{"from": "2026-01-01", "to": "2026-09-14", "bin": "1d"},
)
borrow_apy = [(p["timestamp"], p["borrowApy"], p["utilization"], p["totalDepositsUsd"]) for p in graph["points"]]

# Protocol-wide TVL / borrowed / revenue (USD per bin).
stats = get("/stellar-lending/stats/history", **{"from": "2026-01-01", "to": "2026-09-14", "bin": "1d"})
tvl = list(zip(stats["timestamps"], stats["supplied"], stats["borrowed"], stats["revenue"]))

# Ready-made DefiLlama dimensions: tvl, borrowed, fees, revenue per bin.
dims = get("/stellar-lending/defillama", **{"from": "2026-01-01", "to": "2026-09-14", "bin": "1d"})["points"]
```

Sources: `xoxno-api-v2/src/endpoints/stellar-lending/stellar-lending.controller.ts` and `dto/*.ts`, `sdk-js/src/sdk/swagger.json`, `sdk-js/src/sdk/stellar/lending-api-types.ts` (`@xoxno/sdk-js` 1.0.214).
