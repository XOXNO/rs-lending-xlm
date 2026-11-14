# Composing controller operations

A contract can chain controller operations atomically. Authorize each token
transfer separately; check account state before each operation using it.

## Token-pull ordering

Finish reads and resolve the stored account ID, authorize the exact transfer,
then immediately call the contract that uses it:

| Call | Authorized nested transfer |
|---|---|
| `supply`, `repay`, `recapitalize` | each hub-asset leg: `self -> pool`, exact submitted amount; duplicate legs are summed into one transfer |
| `liquidate` | each debt leg: `self -> pool`, exact offered amount when the quote is the whole debt, else exact planned amount |
| `multiply` with initial payment | payment asset: `self -> controller`, exact amount |
| router `execute_strategy` | input token: `self -> router`, exact amount |

`borrow`, `withdraw`, and strategy calls without initial payment pull
nothing. Wrapper `open_account`, `deposit`, `supply`, `repay`, and
`liquidate` authorize themselves. Every other pull calls
`xoxno_contract_sdk::lending::helpers::authorize_transfer_as_current(env,
token, from, to, amount)` with the table's `to`; copy the low-level auth tree
only if the downstream ABI differs. No router client ships; declare
`execute_strategy` as in
[`../xoxno-swap-aggregator/payload.md`](../xoxno-swap-aggregator/payload.md).

Orchestrating entrypoints start with the instance `extend_ttl`
([SKILL.md](SKILL.md#storage-ttl)) and, if they read a stored account ID, run
the [renew/reconcile helper](positions.md#canonical-local-account-pointer)
before any auth entry.

## Swap route rules

Route bytes come from the quote service; never build `StrategyPayload`
on-chain.

- `multiply`:
  - `PositionMode::Multiply`: distinct `HubAssetKey` markets; same token
    address passes through with empty swap bytes.
  - `PositionMode::Long`, `PositionMode::Short`: different token addresses;
    identical ones fail `AssetsAreTheSame` (no empty-route passthrough).
- `swap_debt`, `swap_collateral`: distinct markets; same token across hubs
  passes through with an empty route, else non-empty.
- `repay_debt_with_collateral`: same `HubAssetKey` nets directly with empty
  bytes; distinct markets withdraw and swap, empty for same-token cross-hub
  passthrough, else non-empty.

A route that breaks these emptiness rules fails `InvalidPayments`. Enforcement:
[`strategies/multiply.rs`](https://github.com/XOXNO/rs-lending-xlm/blob/main/contracts/controller/src/strategies/multiply.rs),
[`strategies/swap.rs`](https://github.com/XOXNO/rs-lending-xlm/blob/main/contracts/controller/src/strategies/swap.rs),
[`strategies/swap_debt.rs`](https://github.com/XOXNO/rs-lending-xlm/blob/main/contracts/controller/src/strategies/swap_debt.rs),
[`strategies/swap_collateral.rs`](https://github.com/XOXNO/rs-lending-xlm/blob/main/contracts/controller/src/strategies/swap_collateral.rs),
and
[`strategies/repay_debt_with_collateral.rs`](https://github.com/XOXNO/rs-lending-xlm/blob/main/contracts/controller/src/strategies/repay_debt_with_collateral.rs).
Quote sizing and payload checks:
[`../xoxno-swap-aggregator/composition.md`](../xoxno-swap-aggregator/composition.md).

## Contract liquidation

`get_liquidation_estimate` and execution share one planner. Pick the
`SeizeMode`, then apply the [liquidate rules](abi.md#liquidate-rules).

`XoxnoLending::liquidate` does this for one debt market in `Transfer` mode and
returns the planned amount; `Credit` mode or several debt legs need the
generated client. With a whole-debt quote, submitting above the authorized
amount fails, since the controller pulls the full submitted amount.

For `Credit(existing_id)`, verify before submission:

- `account_exists(existing_id)`
- the receiving ID is not the liquidated account
- the liquidator owns the position NFT or is an active delegate
- mode is `Normal`
- spoke matches the liquidated account's

For `Credit(0)`, store and renew the returned ID; never infer it from NFT
enumeration or events.

Plan/apply implementation:
[`positions/liquidation/`](https://github.com/XOXNO/rs-lending-xlm/tree/main/contracts/controller/src/positions/liquidation/).

## Keeper threshold updates

`update_account_threshold(caller, has_risks, account_ids)` loops over
`account_ids` with no explicit bound. Use conservative batches, never a whole
large inventory; submit each exact batch only after it simulates successfully
within footprint and budget. Later batches may see changed prices or
ownership, so judge each simulation alone.

IDs with no `AccountMeta`, no supply positions, or no resolvable NFT owner are
skipped silently, so success does not prove every ID refreshed. A failed call
refreshes none.

## Operation-specific cleanup

"Empty means deleted" does not hold generically:

- full withdrawal requests cleanup after persisting supply changes
- repay never removes an empty account
- liquidation and bad-debt paths clean up on their own conditions
- `repay_debt_with_collateral(close_position = true)` requires zero debt,
  then withdraws remaining collateral to the caller and removes the account

After a possibly deleting path, call `account_exists` and reconcile the
pointer; a surviving ID still needs owner/mode/spoke checks. Via the wrapper,
`Withdrawal::account_closed` from `withdraw`/`withdraw_all` is the signal; see
[positions.md](positions.md#full-exit-with-the-wrapper).

## Submission checklist

Before signing:

1. pointer lookup succeeded and renewed its TTL
2. expected `account_exists` branch observed
3. NFT owner and mode/spoke match
4. route emptiness matches token and mode rules
5. exact simulation succeeds, nested auth and footprint included

Afterward apply the [completion checks](SKILL.md#completion-checks); account
with actual withdrawal/liquidation returns.

Atomic rollback protects on-chain state if a later leg fails, not a stale
quote or an archived pointer.
