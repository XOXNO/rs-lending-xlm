# Read positions, assets, and current contract state

Companion to [SKILL.md](SKILL.md). Use the v1 API for ordinary wallet rendering.
Use contract views when current ledger state is required. Neither read path signs
or submits a transaction.

## Read arrays with HTTP or the SDK

Both networks serve these routes without an API key:

| Route | Body |
| --- | --- |
| `GET /stellar-lending/v1/assets` | `LendingAsset[]`; collateral listings |
| `GET /stellar-lending/v1/assets?usage=borrow` | `LendingAsset[]`; borrow listings |
| `GET /stellar-lending/v1/users/{owner}/positions` | `LendingPosition[]`; one item per indexed owned NFT |

Use `https://api.xoxno.com` for mainnet and `https://testnet-api.xoxno.com`
for testnet. Keep API, RPC, contracts, and signing network aligned.

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

A position includes `supplied`, `borrow`, risk estimates, and `nftImage`.
Each token leg includes its contract, labels, logo, decimals, raw and formatted
amounts, APY, price, and USD value. No token-catalog join or balance conversion is
needed. The exact JSON debt key is `borrow`.

The SDK read client collects pages, checks HTTP status and array bodies,
rejects redirects and links to other endpoints, and detects pagination cycles.
It does not validate every field against a schema. Public types and JSON schemas
come from the same read entry; keep them in the SDK.

For direct HTTP positions, follow `Link` with `rel="next"` until absent,
including after an empty page. Resolve relative links against the current URL.
Reject links to another endpoint. Reject repeated links. `limit` is 1–100, default 50;
`cursor` is opaque. An HTTP error is not an empty portfolio.

See the [API reference](https://xoxno.com/docs/stellar-lending/dev/integrator-api)
for fields, nullable values, capacity semantics, and validation rules.

## Units and missing data

- v1 `amountRaw` is an integer token base-unit string. Do not apply RAY conversion.
- `amount` is a decimal whole-token string; keep large ids and amounts as strings.
- APYs are fractions: `0.05` means 5%.
- `priceUsd`, `valueUsd`, and risk values are display estimates.
- `null` means unavailable, except `healthFactor` is also null without debt.
  Use `hasDebt === false` to display “No debt”.
- Names and symbols are labels. Token identity is `sac`; a market also needs
  `hubId`, and account actions need the selected `spokeId`.

Keep known balances visible when another field is missing. Do not replace
missing values with zero or treat an NFT image as a balance authority.

## Freshness and authority

Assets send `Cache-Control: public, s-maxage=10`; positions send `no-store`.
Backend body caches are 5 seconds for assets and 3 seconds for positions;
token metadata is cached for 600 seconds. Indexed NFT inventory is queried
on every wallet request before a position body is reused.

These TTLs do not bound total data age. Indexing, oracle state, market snapshots,
metadata, and HTTP caches have separate freshness limits. Indexed ownership can
lag a transfer. Use `owner_of(accountId)` for current ownership and simulate
mutations before signing.

Refresh when the wallet or network changes. Refresh after ledger `SUCCESS`.
Choose a polling interval for the screen's freshness needs. Discard results
from an old wallet or network selection. After
success, keep a refreshing state until the indexer catches up; do not repeat the
transaction because the API still shows the prior snapshot.

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
