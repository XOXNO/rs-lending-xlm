# Reads: API state and contract views

Companion to [SKILL.md](SKILL.md). `stellarLendingRead` is an HTTP-only reader:
it does not simulate, authorize, or submit transactions. The complete route
inventory belongs in
[../xoxno-lending-data/api.md](../xoxno-lending-data/api.md); this page covers
only the active SDK calls needed by an application.

```ts
import {
  XOXNOClient,
  stellarLendingRead,
} from '@xoxno/sdk-js/stellar-lending'

const read = stellarLendingRead(
  new XOXNOClient({ apiUrl: 'https://api.xoxno.com' }),
)

const [context, live, positions] = await Promise.all([
  read.context(),
  read.liveState(),
  read.userPositions(owner),
])
```

Use:

- `context()` for assets, hubs, spokes, reserves, decimals, caps, flags, and
  the `reserveDetailsByKey` coordinate lookup.
- `liveState()` for current mirrored indexes, WAD prices, and the configurable
  minimum-borrow-collateral floor.
- `reserves(...)` / `reserve(spokeId, hubId, asset)` for market display and
  the exact selected reserve.
- `userPositions(owner)` for the accounts indexed for a wallet and
  `accountPositions(accountId)` for one lending account.
- `marketsDetailed()` when oracle validity, staleness, or deviation flags are
  needed.

The SDK declares `walletBalance(...)` and `userActivityPage(...)`, but the API
does not serve their routes; a deployment without a route returns 404. Do not
make either one a required production path. Read wallet balances from Horizon
or the Soroban token contract. See the endpoint reference for route
availability.

## Units and DTO semantics

The canonical formulas and rounding directions are in
[../xoxno-lending/math.md](../xoxno-lending/math.md).

### DTO field semantics

- `supplyScaledRay` and `borrowScaledRay` are RAY shares.
- `supplyAmount` and `borrowAmount` are RAY token quantities, not builder-ready
  base units.
- `live*IndexRay` falls back to the stored position index when the API has no
  live index. `null` means it has neither, and `*Amount` then repeats the raw
  shares.
- `liveSupplyIndexRay`, `liveBorrowIndexRay`, and market index fields use RAY
  (`1e27`).
- Exact USD prices and risk values use WAD (`1e18`); risk weights and fees use
  BPS (`1e4`).
- `supplyCap` / `borrowCap` are base-unit strings and `0` means closed, not
  unlimited.
- `supplyApy`, `borrowApy`, `utilization`, `*Short`, and `*Usd` are display
  numbers. Never feed them into builders or admission checks.

A display-only RAY conversion is:

```ts
const rayQuantityToBaseFloor = (
  amountRay: string,
  decimals: number,
): bigint => BigInt(amountRay) / 10n ** BigInt(27 - decimals)
```

For exact withdrawal, repayment, valuation, and post-action gates, apply the
contract-specific floor/ceil rules from the math reference or simulate the
contract view. Do not generalize the display conversion into contract
admission math.

## Coordinates and ownership

Reserve identity is the full `(spokeId, hubId, asset)` tuple. The same token
may exist on multiple hubs and spokes. `accountId` is the position-NFT token
id; it does not encode any reserve coordinate. A wallet may own multiple
accounts on one spoke.

Group position rows by `accountId`, let the user select the account, then
verify every row in that account reports the expected `spokeId`. Before a
mutation, confirm the current NFT owner; indexed `owner` is not authoritative
after a transfer. See [positions.md](positions.md).

## Freshness and authority

- Fetch `context()` on page load/navigation.
- Poll one shared `liveState()` query about every 10 seconds.
- Refetch positions only after transaction `SUCCESS`, then retry reconciliation
  if the indexer still serves the prior snapshot.
- Present a price as trustworthy only when its `marketsDetailed()` row has
  `valid: true`. `stale` and `deviation` flag two causes of `valid: false`.
- For liquidation, final sizing, or post-transaction verification, contract
  simulation at the current ledger is authoritative. API data is an indexed
  mirror.

## One contract-view simulation helper

Use this helper for all controller and position-NFT views. Callers provide the
contract id, method, and already-encoded ScVals; view-specific wrappers should
only encode arguments and decode `retval`.

```ts
import {
  Account,
  BASE_FEE,
  Contract,
  Keypair,
  TransactionBuilder,
  rpc,
  xdr,
} from '@stellar/stellar-sdk'
import {
  STELLAR_NETWORK_PASSPHRASE,
  type StellarNetwork,
} from '@xoxno/sdk-js/stellar-lending'

const READ_SOURCE = Keypair.random().publicKey()

export async function simulateView(
  server: rpc.Server,
  network: StellarNetwork,
  contractId: string,
  method: string,
  args: xdr.ScVal[],
): Promise<xdr.ScVal> {
  const tx = new TransactionBuilder(new Account(READ_SOURCE, '0'), {
    fee: BASE_FEE,
    networkPassphrase: STELLAR_NETWORK_PASSPHRASE[network],
  })
    .addOperation(new Contract(contractId).call(method, ...args))
    .setTimeout(30)
    .build()

  const simulation = await server.simulateTransaction(tx)
  if (rpc.Api.isSimulationError(simulation)) {
    throw new Error(`simulate(${method}) failed: ${simulation.error}`)
  }
  if (!simulation.result) {
    throw new Error(`simulate(${method}) returned no value`)
  }
  return simulation.result.retval
}
```

Relevant controller views include `get_account_attributes`,
`get_account_positions`, `get_health_factor`, `get_ltv_collateral_usd`,
`get_collateral_amount`, `get_borrow_amount`, `is_liquidatable`,
`get_spoke_asset`, `get_market_indexes_detailed`, and
`get_min_borrow_collateral_usd`. Exact signatures and ScVal shapes are in
[../xoxno-lending-contracts/abi.md](../xoxno-lending-contracts/abi.md).
