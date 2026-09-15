# Positions, ownership, and risk

Companion to [SKILL.md](SKILL.md). The published
`@xoxno/sdk-js@1.0.214` package does **not** export
`computeAccountRisk`, `maxBorrow`, `maxWithdraw`, `maxSupply`, `maxRepay`,
`projectAccountRisk`, `projectReserveApy`, or the alpha math primitives.
Do not import those names in an application pinned to 1.0.214.

Those APIs exist only on an unpublished SDK alpha branch. Documentation or
code that uses them must be explicitly gated by a later package version whose
published `dist/sdk/stellar/index.d.ts` exports the names and whose tarball is
covered by the application's compile test. Until then, implement application
math from [../xoxno-lending/math.md](../xoxno-lending/math.md), or use
authoritative contract views through [reads.md](reads.md).

## Discover accounts without inventing identifiers

`read.userPositions(owner)` returns position rows, not one row per account.
Group by `accountId`:

```ts
import {
  XOXNOClient,
  stellarLendingRead,
  type AccountPositionDto,
} from '@xoxno/sdk-js/stellar-lending'

const read = stellarLendingRead(
  new XOXNOClient({ apiUrl: 'https://api.xoxno.com' }),
)

export async function accountsOf(
  owner: string,
): Promise<Map<string, AccountPositionDto[]>> {
  const { positions } = await read.userPositions(owner)
  const accounts = new Map<string, AccountPositionDto[]>()
  for (const row of positions) {
    accounts.set(row.accountId, [...(accounts.get(row.accountId) ?? []), row])
  }
  return accounts
}
```

`accountId` is the position-NFT token id. It is globally identified by the
position-NFT contract plus token id, but the UI string
`` `${positionNftContract}-${accountId}` `` is only an application cache key,
not a Soroban address and not a reserve identifier.

An account lives on one spoke, but neither its numeric id nor that cache key
encodes the spoke. A wallet can own multiple accounts on the same spoke.
Therefore:

1. Let the user or intent select `accountId`.
2. Fetch `accountPositions(accountId)` or filter the grouped rows.
3. Verify all rows agree on the expected `spokeId`.
4. Match each leg with the complete `(spokeId, hubId, asset)` reserve key.
5. Recheck `owner_of(accountId)` before preparing a mutation.

Do not select `positions[0]`, infer an account from an asset, or treat
`spokeId` as an account identifier. A position-NFT transfer transfers control
of the whole lending account; API ownership is index-time data.

## Exact balances and rounding

The DTO contains:

- `supplyScaledRay` / `borrowScaledRay`: RAY shares.
- `liveSupplyIndexRay` / `liveBorrowIndexRay`: the applied index when
  available.
- `supplyAmount` / `borrowAmount`: RAY token quantities for display, not
  builder amounts.
- entry LTV and liquidation BPS: weights snapshotted when the leg opened.

For builder-ready amounts, use integer math and the protocol's directed
rounding:

- supply shares mint floor; borrow shares mint ceil;
- partial withdrawal burns ceil;
- repayment burns floor;
- full withdrawal pays the floor balance and should be requested with the
  protocol sentinel `amount: '0'`;
- full debt close repays the ceiled debt, with a small accrual buffer; surplus
  is refunded.

The exact formulas and pseudocode are in
[../xoxno-lending/math.md#shares-and-token-amounts](../xoxno-lending/math.md#shares-and-token-amounts).
At a current ledger, `get_collateral_amount` and `get_borrow_amount` are the
authoritative base-unit views.

## Application-side risk

For 1.0.214, risk and max-action helpers are application code, not SDK calls.
Keep the implementation in raw `bigint` WAD/RAY/BPS values:

```text
for each row in one account:
  resolve reserve by (account.spokeId, row.hubId, row.asset)
  resolve an accepted current index and WAD price
  unscale supply with protocol floor; unscale debt with protocol ceil
  value collateral with floor and debt with ceil
  weight collateral by the row's snapshotted entry BPS

borrowLimitWad = sum(ltvWeightedCollateralWad)
liquidationCollateralWad = sum(liquidationWeightedCollateralWad)
healthFactorWad =
  debtWad == 0 ? debtFree : floor(liquidationCollateralWad * WAD / debtWad)
```

Admission also depends on more than health factor: the minimum collateral
floor, reserve pause/freeze flags, borrow/supply caps, hub cash, liquidation
buffer, utilization ceiling, oracle validity, and contract authorization.
Implementing `maxBorrow` as only `(borrowLimit - debt) / price` is unsafe.

For final decisions, simulate the prepared transaction. Contract simulation is
the admission check; a client projection is only a preflight estimate.

## Post-action preview

An application preview may project share deltas using the formulas above, but
must:

- value the route at `amountOutMin`, not optimistic `amountOut`;
- preserve the existing leg's snapshotted entry risk weights;
- use the selected reserve's current settings for a newly opened leg;
- apply share-mint/burn rounding before valuation;
- keep comparisons in raw WAD `bigint`;
- label any formatted number or float-derived leverage as an estimate.

Never promise that a displayed health factor or leverage is admissible.
Preparation can still fail because of price/index movement, stale oracle data,
caps, liquidity, utilization, authorization, trustlines, route slippage,
resource/footprint limits, sequence changes, or expired timebounds.

After transaction `SUCCESS`, refetch positions and live state, reconcile the
created/updated `accountId`, and recheck the position-NFT owner before another
mutation.
