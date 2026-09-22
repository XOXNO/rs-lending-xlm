# Composing controller operations

One contract invocation can sequence controller verbs atomically. Each token
pull still needs an exact nested authorization, and account state must be
revalidated at the branch where it is used.

## Token-pull ordering

Perform reads and local pointer reconciliation first. Then authorize, then
immediately invoke the consuming contract:

| Call | Authorized nested transfer |
|---|---|
| `supply`, `repay`, `recapitalize` | each asset: `self -> pool`, exact submitted amount |
| `liquidate` | each debt asset: `self -> pool`, exact offered amount when the quote is the whole debt, else exact planned amount |
| `multiply` with initial payment | payment asset: `self -> controller`, exact amount |
| router `execute_strategy` | input token: `self -> router`, exact amount |

`borrow`, `withdraw`, and controller strategy calls without an initial payment
do not pull tokens from the caller contract. Use
`common::token::authorize_transfer_as_current`; do not copy the low-level auth
tree unless the downstream ABI differs.

Every public entrypoint that owns this orchestration should start with
`common::ttl::renew_instance(&env)`. If it reads a persisted account ID, use
the renew/reconcile helper in
[positions.md](positions.md#canonical-local-account-pointer) before creating
any auth entry.

## Swap route rules

Route bytes come from the quote service; do not construct
`StrategyPayload` on-chain.

- `multiply`:
  - `PositionMode::Multiply` requires distinct `HubAssetKey` markets. If those
    markets use the same token address, empty swap bytes select passthrough.
  - `PositionMode::Long` and `PositionMode::Short` require different token
    addresses. Identical token addresses fail `AssetsAreTheSame`; empty-route
    passthrough is not available.
- `swap_debt` and `swap_collateral` require distinct markets. For two hubs
  using the same token, an empty route passes the token through; otherwise a
  non-empty route is required.
- `repay_debt_with_collateral` nets directly only when collateral and debt are
  the same `HubAssetKey`. Distinct markets use the swap path, where same-token
  cross-hub passthrough can use empty bytes.

These branches are enforced by
[`strategies/multiply.rs`](../../contracts/controller/src/strategies/multiply.rs),
[`strategies/swap.rs`](../../contracts/controller/src/strategies/swap.rs), and
[`strategies/repay_debt_with_collateral.rs`](../../contracts/controller/src/strategies/repay_debt_with_collateral.rs).
Quote sizing and payload checks are in
[`../xoxno-swap-aggregator/composition.md`](../xoxno-swap-aggregator/composition.md).

## Contract liquidation

`get_liquidation_estimate` and execution share the liquidation planner. Build
the intended `SeizeMode`, simulate the estimate, subtract per-asset refunds,
and authorize only the resulting planned debt payments. Then invoke
`liquidate` with those planned amounts.

For `Credit(existing_id)`, verify before submission:

- `account_exists(existing_id)`
- position NFT owner is the liquidator contract
- account mode is `Normal`
- receiving account spoke matches the victim

For `Credit(0)`, store and renew the returned account ID locally. Do not infer
the receiving ID from NFT enumeration or events.

The plan/apply implementation is under
[`positions/liquidation/`](../../contracts/controller/src/positions/liquidation/).

## Keeper threshold updates

`update_account_threshold` loops over `account_ids` without an explicit
contract-side input bound. Do not submit an arbitrarily large owner inventory.
Use conservative batches and require successful Soroban simulation for each
exact batch before signing/submitting it. A later batch can observe changed
prices or ownership, so treat each simulation result independently.

Unknown IDs are skipped, but that is not proof that known IDs were fully
updated if the transaction exceeded budget and failed.

## Operation-specific cleanup

Do not apply a generic "empty after any verb means deleted" rule:

- a full withdrawal requests cleanup after persisting supply changes
- repay does not automatically remove an empty account
- liquidation and bad-debt paths perform cleanup under their own conditions
- `repay_debt_with_collateral(close_position = true)` first requires debt to
  be gone, then withdraws remaining collateral

After a path that can delete, call `account_exists` and reconcile the local
pointer. Remember that this view checks only `AccountMeta`; a surviving ID
still needs NFT owner/mode/spoke checks before reuse.

## Submission checklist

Before signing:

1. local account pointer lookup succeeded and renewed its persistent TTL
2. expected `account_exists` branch was observed
3. NFT owner and account mode/spoke match the branch
4. route emptiness matches token and mode rules
5. exact transaction simulation succeeds, including nested auth and footprint

After execution:

1. returned account IDs are stored with a fresh local TTL
2. actual withdrawal/liquidation return values are used for accounting
3. potentially deleted accounts are reconciled
4. flash flows are considered complete only after callback settlement, not
   merely after callback return

Atomic rollback protects on-chain state when a later leg fails, but it does not
make a stale off-chain quote or an archived local pointer safe.
