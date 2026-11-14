# Positions, ownership, and risk

Companion to [SKILL.md](SKILL.md). Use v1 position arrays for display, and
current contract state plus prepared-transaction simulation for mutations.

## Discover and select an account

```ts
import {
  createStellarLendingReadClient,
  type LendingPosition,
} from '@xoxno/sdk-js/stellar-lending/read'

const read = createStellarLendingReadClient({ baseUrl: 'https://api.xoxno.com' })

export async function accountsOf(owner: string): Promise<LendingPosition[]> {
  return read.positions(owner)
}
```

Each item is one indexed owned position NFT with `supplied` and `borrow`
arrays, `spokeName`, health, borrow limits, net APY, and an SVG URL. Never
regroup legs from several accounts into one risk calculation.

Key a position by `(network, nftContract, accountId)` and each leg by
`(hubId, sac)`. Neither an account id nor a token symbol encodes a spoke.

Before an owner-wallet action:

1. Select an explicit `accountId` from user intent.
2. Verify its `spokeId` and each selected `(hubId, sac)` market.
3. Recheck `owner_of(accountId)` on the network's position-NFT contract.
4. Pass the selected id as the builder's `accountNonce`.
5. Pass `sac` as the builder's `asset`.
6. Prepare the exact transaction before requesting a signature.

Never choose `positions[0]` or infer an account from a token. An NFT transfer
moves control of the whole lending account, including its debt, and indexed
inventory can lag it.

## Render balances and risk

| Field | Display rule |
| --- | --- |
| `amountRaw` | Integer token base-unit string; supply rounds down, debt rounds up. No RAY conversion. |
| `amount` | Decimal whole-token string; no floating-point conversion needed. |
| `apy`, `netApy` | Fractions; multiply by 100 for percentage display. |
| `hasDebt`, `healthFactor` | Display “No debt” only for `hasDebt === false`; otherwise show health or “Unavailable”. |
| `borrowLimitUsd`, `availableBorrowUsd` | Account estimates before market and other action constraints. |
| `priceUsd`, `valueUsd` | Nullable USD estimates. Missing price can leave raw balances available. |
| `nftImage`, `logoUrl` | Display images; handle failed/missing images and SVG support. |
| `dataStatus` | Inspect individual fields even when the whole item is incomplete. |

Net APY is annual supply income minus debt cost, divided by positive net
equity; it is null when inputs or positive equity are unavailable. A missing
health factor is not proof of safety, and a `ready` item is not an admission
check.

## Transaction sizing and rounding

Raw v1 balances are snapshots; interest can accrue before execution. Never
size a final repay or withdrawal from formatted display numbers.

- Full withdrawal uses `amount: '0'`. Full debt close sends the ceiled current
  debt plus an accrual buffer; excess is refunded.
- Supply shares mint down; debt shares mint up. Partial withdrawal burns up;
  partial repayment burns down.

`get_collateral_amount` and `get_borrow_amount` return current base units
rounded half up; ceiled debt can be one unit above `get_borrow_amount`. See
the [rounding reference](../xoxno-lending/math.md#shares-and-token-amounts).

## Risk checks before an action

Use current controller views for balances, prices, indexes, and stored risk
parameters, and `get_ltv_collateral_usd` for current LTV-weighted collateral
in USD WAD: listed collateral uses the refreshed listing LTV, unlisted keeps
its stored LTV, and each weight is capped by the stored liquidation threshold
([formulas](../xoxno-lending/math.md#health-factor-and-ltv-weighting)). Keep
custom arithmetic in `bigint` WAD/RAY/BPS. For a route preview, apply share
rounding, value collateral output at `amountOutMin`, and label the result an
estimate.

Admission also checks collateral floors, caps, cash, utilization, halt flags,
oracle validity, account rules, authorization, and operation-specific guards.
For collateral with fewer than three decimals, the post-action token minimum
is two base units while debt remains, separate from the USD floor. API flags
do not reproduce every gate, and ledger state can change after preparation,
so preparation does not guarantee execution.

After `SUCCESS`, reconcile the returned account id, API positions, live state,
and current NFT ownership. Follow the
[transaction recovery policy](transactions.md#canonical-lifecycle).
