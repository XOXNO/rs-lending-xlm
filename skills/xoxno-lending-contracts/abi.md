# ABI integration notes

Companion to [SKILL.md](SKILL.md).

## Authoritative sources

- Integrator clients and types: the `xoxno_contract_sdk::lending` modules,
  generated from the embedded WASM (see [SKILL.md](SKILL.md#dependencies))
- Controller signatures:
  [`interfaces/controller/src/lib.rs`](https://github.com/XOXNO/rs-lending-xlm/blob/main/interfaces/controller/src/lib.rs);
  admin:
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
Account IDs are `u64`; NFT token IDs `u32`.

Authorization groups:

- caller-only: direct invoker auth for `caller = current_contract_address()`
- owner/delegate: caller owns the position NFT or holds a current grant as an
  active governance-approved position manager
- owner-only: `renew_account`, `add_delegate`, `remove_delegate` need the
  current NFT owner; a keeper cannot substitute itself
- controller-only/admin: never call pool mutators, NFT mint/burn, or
  governance entrypoints from an integration

Token pulls are separate nested auth; see
[composing.md](composing.md#token-pull-ordering).

## Operation rules

| Operation | Integration behavior to preserve |
|---|---|
| `supply` | ID `0` creates and returns a `Normal` account; persist and renew it. Third-party top-up only into an existing supply market. |
| `borrow` / `withdraw` | `to = None` pays caller; `to` never the pool or controller. `i128::MAX` withdraws all; a zero leg also does, but is deprecated. Use returned actual amounts. Wrapper `withdraw`/`withdraw_all` return `Withdrawal { amount, account_closed }`. |
| `repay` | See [repay rules](#repay-rules). |
| `liquidate` | See [liquidate rules](#liquidate-rules). |
| `flash_loan` | Receiver approves `amount + fee` to the pool; direct repayment transfer rejected. |
| `flash_position` | Debt stays on the account. Callback pushes declared collateral to the controller; undeclared balances not credited. |
| `multiply`, `swap_debt`, `swap_collateral`, `repay_debt_with_collateral` | Distinctness and route emptiness: [composing.md](composing.md#swap-route-rules). `close_position` withdraws remaining collateral to caller; any remaining debt fails `CannotCloseWithRemainingDebt`. |
| `renew_account` | Controller account entries and NFT state; never the caller's pointer or instance. |

### Repay rules

- Anyone may repay. Excess is refunded to the caller.
- Wrapper `repay` returns the amount repaid, excluding the refund. Repaying
  with a user's tokens, send `amount - repaid` back to that user.
- `repay` never removes the account, even after the last debt; reconcile only
  after operations that can.

### Liquidate rules

1. Simulate `get_liquidation_estimate`.
2. Subtract its per-asset `refunds` from the quoted amounts.
3. Omit a market whose planned amount is zero.
4. Submit and authorize exactly those planned amounts.

When the quote covers the whole debt, the controller pulls each submitted
amount in full and the pool refunds excess. `Credit(0)` creates a receiving
`Normal` account in the victim's spoke. Details:
[composing.md](composing.md#contract-liquidation).

## Views that need interpretation

- `account_exists(id)`: checks and extends only `AccountMeta(id)`; proves
  nothing about supply/borrow maps, delegates, or NFT storage. Before a
  privileged action, also validate NFT ownership and `get_account_attributes`.
- `get_health_factor(id)`: WAD; `i128::MAX` for no debt, no account, or a
  saturated positive ratio.
- `get_collateral_amount`, `get_borrow_amount`: accrued token base units,
  half up. `get_account_positions`: raw RAY-scaled shares.
- `get_market_index`: accrued RAY indexes, no oracle read.
- `get_spoke_usage`: RAY-scaled usage, zeroed when unset. Caps are token base
  units; zero is not an unlimited sentinel.

## Batch semantics

Payment vectors of `supply`, `borrow`, `withdraw`, `repay`, and `liquidate`
must be non-empty. Negative amounts fail; zero fails except in a withdrawal
leg. Duplicate market keys aggregate in first-seen order. In withdrawal, a
zero leg mixed with a positive leg for one market fails with
`InvalidPayments`. New supply/debt markets count against position limits.

`update_account_threshold` has no explicit input bound
([`risk/params.rs`](https://github.com/XOXNO/rs-lending-xlm/blob/main/contracts/controller/src/risk/params.rs));
batch it as in [composing.md](composing.md#keeper-threshold-updates).

`get_liquidation_estimate` and `get_market_indexes_detailed` reject over
`MAX_VIEW_INPUTS` (256) entries with `InvalidPayments`; other views lack that
bound, so simulate their batches.

## Failure handling

Keep no local per-entrypoint error list. Identify the failing contract first
(controller/router and oracle/NFT codes collide), then use
[`docs/reference/errors.md`](https://github.com/XOXNO/rs-lending-xlm/blob/main/docs/reference/errors.md).

Failures a code-only lookup misses:

- token `Error(Auth, InvalidAction)`, no contract code: missing or mismatched
  `authorize_as_current_contract`, often because a read consumed the next
  invocation
- host failure from a `try_` client: no contract code; not
  `account_exists == false`
- archived local pointer: the account may exist, but your contract lost its ID
- callback succeeded, transaction failed: post-callback allowance, minimum
  collateral, open-position, or solvency settlement failed
- router error via a controller strategy: decode in the router namespace

For `try_` calls only `Ok(Ok(value))` is success; contract, conversion, and
host invocation errors are distinct branches. The
[pointer helper](positions.md#canonical-local-account-pointer) clears state
only on an explicit successful `false`.
