# Read positions, assets, and current contract state

Companion to [SKILL.md](SKILL.md). Use the v1 API for wallet rendering and
contract views when current ledger state is required. Neither path signs or
submits.

## Read arrays with HTTP or the SDK

Routes, base URLs, query limits, and cache headers are in the
[HTTP reference](../xoxno-lending-data/api.md#integrator-v1-arrays). Keep API,
RPC, contracts, and signing network aligned.

```ts
import {
  createStellarLendingReadClient,
  type LendingPosition,
} from '@xoxno/sdk-js/stellar-lending/read'

const read = createStellarLendingReadClient({ baseUrl: 'https://api.xoxno.com' })

export async function portfolio(owner: string, signal?: AbortSignal) {
  const [positions, assets] = await Promise.all([
    read.positions(owner, { signal }),
    read.assets({ signal }),
  ])
  return { positions, assets }
}
```

A position includes `supplied`, `borrow` (the exact JSON debt key), risk
estimates, and `nftImage`. Each token leg includes its contract, labels, logo,
decimals, raw and formatted amounts, APY, price, and USD value, so no
token-catalog join or balance conversion is needed.

The SDK read client collects pages, checks HTTP status and array bodies,
rejects redirects and links to other endpoints, and detects pagination cycles.
It does not schema-validate every field. Public types and JSON schemas come
from the same read entry; keep them in the SDK. Direct HTTP callers follow the
[pagination rules](../xoxno-lending-data/api.md#integrator-v1-arrays). An HTTP
error is not an empty portfolio.

## Units and missing data

- v1 `amountRaw` is an integer token base-unit string. Do not apply RAY conversion.
- `amount` is a decimal whole-token string; keep large ids and amounts as strings.
- APYs are fractions: `0.05` means 5%.
- `priceUsd`, `valueUsd`, and risk values are display estimates.
- `null` means unavailable, except `healthFactor` is also null without debt.
  Display “No debt” only for `hasDebt === false`.
- Names and symbols are labels. Token identity is `sac`; a market also needs
  `hubId`, and account actions need the selected `spokeId`.

Keep known balances visible when another field is missing. Never replace
missing values with zero or treat an NFT image as a balance authority.

## Freshness and authority

Cache TTLs ([HTTP reference](../xoxno-lending-data/api.md#integrator-v1-arrays))
do not bound total data age: indexing, oracle state, market snapshots,
metadata, and HTTP caches have separate freshness limits. Indexed ownership
can lag a transfer. Use `owner_of(accountId)` for current ownership and
simulate mutations before signing.

Refresh when the wallet or network changes and after ledger `SUCCESS`; poll at
the interval the screen needs. Discard results from an old wallet or network
selection. After success, keep a refreshing state until the indexer catches
up; do not repeat the transaction because the API still shows the prior
snapshot.

## One contract-view simulation helper

Use this helper for all controller and position-NFT views. Callers pass the
contract id, method, and encoded ScVals; view-specific wrappers only encode
arguments and decode `retval`.

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
`get_min_borrow_collateral_usd`. Signature sources and view units:
[../xoxno-lending-contracts/abi.md](../xoxno-lending-contracts/abi.md).
