# Composing controller operations

A contract can chain controller operations atomically. Authorize each token
transfer separately. Check account state before each operation that uses it.

## Token-pull ordering

Finish reads and resolve the stored account ID. Then authorize the exact
transfer. Then call the contract that uses it immediately. The table shows the
nested transfer for each call:

| Call | Authorized nested transfer |
|---|---|
| `supply`, `repay`, `recapitalize` | each hub-asset leg: `self -> pool`, exact submitted amount. Duplicate legs are summed into one transfer |
| `liquidate` | each debt leg: `self -> pool`, exact offered amount when the quote is the whole debt, else exact planned amount |
| `multiply` with initial payment | payment asset: `self -> controller`, exact amount |
| router `execute_strategy` | input token: `self -> router`, exact amount |

`borrow`, `withdraw`, and strategy calls without initial payment pull
nothing. Wrapper `open_account`, `deposit`, `supply`, `repay`, and
`liquidate` authorize themselves. Every other pull calls
`xoxno_contract_sdk::lending::helpers::authorize_transfer_as_current(env,
token, from, to, amount)` with the table's `to`. Copy the low-level auth tree
only if the downstream ABI differs. No router client ships. Declare
`execute_strategy` as in
[`../xoxno-swap-aggregator/payload.md`](../xoxno-swap-aggregator/payload.md).

Orchestrating entrypoints start with the instance `extend_ttl`
([SKILL.md](SKILL.md#storage-ttl)). If they read a stored account ID, they then
run the [renew and reconcile helper](positions.md#canonical-local-account-pointer)
before any auth entry.

## Swap route rules

Route bytes come from the quote service. Never build `StrategyPayload`
on-chain. The route rules differ by operation:

- `multiply`:
  - `PositionMode::Multiply` needs distinct `HubAssetKey` markets. The same
    token address passes through with empty swap bytes.
  - `PositionMode::Long` and `PositionMode::Short` need different token
    addresses. Identical addresses fail with `AssetsAreTheSame`, with no
    empty-route passthrough.
- `swap_debt` and `swap_collateral` need distinct markets. The same token
  across hubs passes through with an empty route. Otherwise the route must be
  non-empty.
- `repay_debt_with_collateral`: the same `HubAssetKey` nets directly with empty
  bytes. Distinct markets withdraw and swap. The route is empty for a same-token
  cross-hub passthrough and non-empty otherwise.

A route that breaks these emptiness rules fails `InvalidPayments`. These files enforce the rules:
[`strategies/multiply.rs`](https://github.com/XOXNO/rs-lending-xlm/blob/main/contracts/controller/src/strategies/multiply.rs),
[`strategies/swap.rs`](https://github.com/XOXNO/rs-lending-xlm/blob/main/contracts/controller/src/strategies/swap.rs),
[`strategies/swap_debt.rs`](https://github.com/XOXNO/rs-lending-xlm/blob/main/contracts/controller/src/strategies/swap_debt.rs),
[`strategies/swap_collateral.rs`](https://github.com/XOXNO/rs-lending-xlm/blob/main/contracts/controller/src/strategies/swap_collateral.rs),
and
[`strategies/repay_debt_with_collateral.rs`](https://github.com/XOXNO/rs-lending-xlm/blob/main/contracts/controller/src/strategies/repay_debt_with_collateral.rs).
For quote sizing and payload checks, see
[`../xoxno-swap-aggregator/composition.md`](../xoxno-swap-aggregator/composition.md).

## Contract liquidation

`get_liquidation_estimate` and execution share one planner. Pick the
`SeizeMode`, then apply the [liquidate rules](abi.md#liquidate-rules).

`XoxnoLending::liquidate` applies the liquidate rules for one debt market in
`Transfer` mode and returns the planned amount. `Credit` mode or several debt
legs need the generated client. With a whole-debt quote, submitting above the
authorized amount fails, because the controller pulls the full submitted
amount.

For `Credit(existing_id)`, verify these conditions before submission:

- `account_exists(existing_id)` returns true
- the receiving ID is not the liquidated account
- the liquidator owns the position NFT or is an active delegate
- the mode is `Normal`
- the spoke matches the liquidated account's spoke

For `Credit(0)`, store and renew the returned ID. Never infer it from NFT
enumeration or events.

The plan and apply implementation is in
[`positions/liquidation/`](https://github.com/XOXNO/rs-lending-xlm/tree/main/contracts/controller/src/positions/liquidation/).

## Keeper threshold updates

`update_account_threshold(caller, has_risks, account_ids)` loops over
`account_ids` with no explicit bound. Use conservative batches, never a whole
large inventory. Submit each exact batch only after it simulates successfully
within footprint and budget. Later batches may see changed prices or
ownership, so judge each simulation alone.

IDs with no `AccountMeta`, no supply positions, or no resolvable NFT owner are
skipped silently, so success does not prove every ID refreshed. A failed call
refreshes none.

## Operation-specific cleanup

The rule "Empty means deleted" does not hold in general:

- full withdrawal requests cleanup after persisting supply changes
- repay never removes an empty account
- liquidation and bad-debt paths clean up on their own conditions
- `repay_debt_with_collateral(close_position = true)` requires zero debt,
  then withdraws remaining collateral to the caller and removes the account

After a path that may delete the account, call `account_exists` and reconcile
the pointer. A surviving ID still needs owner, mode, and spoke checks. Through
the wrapper, `Withdrawal::account_closed` from `withdraw` and `withdraw_all` is
the signal. See
[positions.md](positions.md#full-exit-with-the-wrapper).

## Submission checklist

Before signing, confirm these conditions:

1. pointer lookup succeeded and renewed its TTL
2. expected `account_exists` branch observed
3. NFT owner, mode, and spoke match
4. route emptiness matches token and mode rules
5. the exact simulation succeeds, including nested authorization and footprint

Afterward, apply the [completion checks](SKILL.md#completion-checks). Use the
actual withdrawal and liquidation returns for accounting.

Atomic rollback protects on-chain state if a later leg fails. It does not
protect against a stale quote or an archived pointer.
