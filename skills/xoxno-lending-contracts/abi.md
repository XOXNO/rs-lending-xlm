# ABI integration notes

This file is a companion to [SKILL.md](SKILL.md).

## Authoritative sources

These sources are authoritative:

- Integrator clients and types: the `xoxno_contract_sdk::lending` modules,
  generated from the embedded WASM (see [SKILL.md](SKILL.md#dependencies))
- Controller signatures:
  [`interfaces/controller/src/lib.rs`](https://github.com/XOXNO/rs-lending-xlm/blob/main/interfaces/controller/src/lib.rs).
  Admin signatures:
  [`interfaces/controller/src/admin.rs`](https://github.com/XOXNO/rs-lending-xlm/blob/main/interfaces/controller/src/admin.rs)
- Pool, NFT, router, and price signatures:
  [`interfaces/`](https://github.com/XOXNO/rs-lending-xlm/tree/main/interfaces/)
- Shared contract types:
  [`common/src/types`](https://github.com/XOXNO/rs-lending-xlm/tree/main/common/src/types/)
- Error names, namespaces, causes, and remedies:
  [`docs/reference/errors.md`](https://github.com/XOXNO/rs-lending-xlm/blob/main/docs/reference/errors.md)

## Integration interpretation

The controller takes `HubAssetKey { hub_id, asset }`, not a bare token.
Amounts are token base units unless the type or source says WAD, RAY, or BPS.
Account IDs are `u64`. NFT token IDs are `u32`.

The controller has four authorization groups:

- caller-only: direct invoker authorization for
  `caller = current_contract_address()`
- owner or delegate: the caller owns the position NFT or holds a current grant
  as an active governance-approved position manager
- owner-only: `renew_account`, `add_delegate`, and `remove_delegate` need the
  current NFT owner. A keeper cannot substitute itself.
- controller-only or admin: never call pool mutators, NFT mint or burn, or
  governance entrypoints from an integration

Token pulls need a separate nested authorization. See
[composing.md](composing.md#token-pull-ordering).

## Operation rules

| Operation | Integration behavior to preserve |
|---|---|
| `supply` | ID `0` creates and returns a `Normal` account. Persist and renew that account. A third party can top up only an existing supply market. |
| `borrow` / `withdraw` | `to = None` pays the caller. `to` is never the pool or the controller. `i128::MAX` withdraws everything. A zero leg fails with `AmountMustBePositive`. Use the returned actual amounts. Wrapper `withdraw` and `withdraw_all` return `Withdrawal { amount, account_closed }`. |
| `repay` | See [repay rules](#repay-rules). |
| `liquidate` | See [liquidate rules](#liquidate-rules). |
| `flash_loan` | The receiver approves `amount + fee` to the pool. A direct repayment transfer is rejected. |
| `flash_position` | Debt stays on the account. The callback pushes declared collateral to the controller. Undeclared balances are not credited. |
| `multiply`, `swap_debt`, `swap_collateral`, `repay_debt_with_collateral` | For distinctness and route emptiness rules, see [composing.md](composing.md#swap-route-rules). `close_position` withdraws remaining collateral to the caller. Any remaining debt fails with `CannotCloseWithRemainingDebt`. |
| `renew_account` | Renews the controller account entries and the NFT state. Never renews the caller's pointer or instance. |

### Repay rules

- Anyone may repay. Excess is refunded to the caller.
- Wrapper `repay` returns the amount repaid, excluding the refund. When you
  repay with a user's tokens, send `amount - repaid` back to that user.
- `repay` never removes the account, even after the last debt. Reconcile only
  after operations that can remove it.

### Liquidate rules

To liquidate, follow these steps:

1. Simulate `get_liquidation_estimate`.
2. Subtract its per-asset `refunds` from the quoted amounts.
3. Omit a market whose planned amount is zero.
4. Submit and authorize exactly those planned amounts.

When the quote covers the whole debt, the controller pulls each submitted
amount in full and the pool refunds the excess. `Credit(0)` creates a receiving
`Normal` account in the victim's spoke. Details are in
[composing.md](composing.md#contract-liquidation).

## Views that need interpretation

Read these views as follows:

- `account_exists(id)` checks and extends only `AccountMeta(id)`. It proves
  nothing about the supply and borrow maps, the delegates, or the NFT storage.
  Before a privileged action, also validate NFT ownership and
  `get_account_attributes`.
- `get_health_factor(id)` returns a WAD value. It returns `i128::MAX` for no
  debt or a saturated positive ratio. A missing account fails with
  `AccountNotFound` (`controller #24`). `is_liquidatable(id)` fails the same way.
- `get_collateral_amount` and `get_borrow_amount` return accrued token base
  units, rounded half up. `get_account_positions` returns raw RAY-scaled
  shares.
- `get_market_index` returns accrued RAY indexes and reads no oracle.
- `get_spoke_usage` returns RAY-scaled usage, zero when unset. Caps are token
  base units. Zero is not an unlimited sentinel.

## Batch semantics

Payment vectors of `supply`, `borrow`, `withdraw`, `repay`, and `liquidate`
must be non-empty. Negative and zero amounts fail. Duplicate market keys
aggregate in first-seen order. A sum past `i128::MAX` fails with
`MathOverflow`. New supply and debt markets count against position limits.

`update_account_threshold` has no explicit input bound
(see [`risk/params.rs`](https://github.com/XOXNO/rs-lending-xlm/blob/main/contracts/controller/src/risk/params.rs)).
Batch it as in [composing.md](composing.md#keeper-threshold-updates).

`get_liquidation_estimate` and `get_market_indexes_detailed` reject more
than `MAX_VIEW_INPUTS` (256) entries with `InvalidPayments`. Other views lack
that bound, so simulate their batches.

## Failure handling

Do not keep a local per-entrypoint error list. Identify the failing contract
first, because controller codes collide with router codes and oracle codes
collide with NFT codes. Then use
[`docs/reference/errors.md`](https://github.com/XOXNO/rs-lending-xlm/blob/main/docs/reference/errors.md).

A code-only lookup misses these failures:

- A token `Error(Auth, InvalidAction)` has no contract code. It means a
  missing or mismatched `authorize_as_current_contract`, often because a read
  consumed the next invocation.
- A host failure from a `try_` client has no contract code. It does not mean
  `account_exists == false`.
- An archived local pointer means the account may exist, but your contract
  lost its ID.
- If the callback succeeds but the transaction fails, the post-callback
  allowance, minimum collateral, open-position, or solvency settlement failed.
- Decode a router error that arrives through a controller strategy in the
  router namespace.

For `try_` calls, only `Ok(Ok(value))` is success. Contract, conversion, and
host invocation errors are distinct branches. The
[pointer helper](positions.md#canonical-local-account-pointer) clears state
only on an explicit successful `false`.
