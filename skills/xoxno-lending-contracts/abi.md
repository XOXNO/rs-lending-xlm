# ABI integration notes

Companion to [SKILL.md](SKILL.md). This page is deliberately not an ABI or
error catalogue.

## Canonical owners

- Integrator clients and types: the `xoxno_contract_sdk::lending` modules,
  generated from the WASM the crate embeds (see
  [SKILL.md](SKILL.md#dependencies))
- Controller signatures:
  [`interfaces/controller/src/lib.rs`](../../interfaces/controller/src/lib.rs)
- Controller admin signatures:
  [`interfaces/controller/src/admin.rs`](../../interfaces/controller/src/admin.rs)
- Pool, NFT, router, and price signatures:
  [`interfaces/`](../../interfaces/)
- Shared contract types:
  [`common/src/types`](../../common/src/types/)
- Canonical error names, namespaces, causes, and remedies:
  [`docs/reference/errors.md`](../../docs/reference/errors.md)

Generated clients pass arguments by reference. Use the crate's types as
described in [SKILL.md](SKILL.md#dependencies); duplicating a
`#[contracttype]` locally is unnecessary and can drift.

## Integration interpretation

The controller accepts `HubAssetKey { hub_id, asset }`, not a bare token
address. Amounts are token base units unless the interface type or source says
WAD, RAY, or BPS. Account IDs are `u64` in the controller and NFT token IDs are
`u32`.

Authorization groups relevant to a contract caller:

- caller-only: direct invoker auth for `caller = current_contract_address()`
- owner/delegate: the caller must own the position NFT or have a current grant
  as an active governance-approved position manager
- owner-only: `renew_account`, `add_delegate`, and `remove_delegate` require
  the current NFT owner; a keeper cannot substitute its own address
- controller-only/admin: do not call pool mutators, NFT mint/burn, or
  governance entrypoints from an integration contract

Token pulls are separate nested auth. `supply`, `repay`, `liquidate`, and
`recapitalize` pull from the caller to the pool. `multiply` pulls an optional
initial payment to the controller. See
[composing.md](composing.md#token-pull-ordering).

## High-value behavior by operation

| Operation | Integration behavior to preserve |
|---|---|
| `supply` | ID `0` creates a `Normal` account and returns its ID. Existing-account third-party top-up is limited to an existing supply market. Persist and renew the returned ID locally. |
| `borrow` / `withdraw` | `to = None` pays the caller. `to` cannot be the pool or controller. Zero in a withdrawal leg means withdraw all. Use the returned actual withdrawal amounts. The wrapper's `withdraw` and `withdraw_all` return `Withdrawal { amount, account_closed }`. |
| `repay` | Anyone may repay. Excess is refunded to the caller. The wrapper's `repay` returns the amount repaid, which excludes that refund; a contract that repays with a user's tokens sends `amount - repaid` back to that user. `repay` never removes the account, even when it repays the last debt; reconcile only after an operation that can remove the account. |
| `liquidate` | Simulate `get_liquidation_estimate`, subtract its per-asset `refunds`, then submit and authorize exactly those planned amounts. Omit a market whose planned amount is zero. When the quote covers the whole debt, the controller pulls each submitted amount in full and the pool refunds the excess. `Credit(0)` creates a receiving `Normal` account in the victim's spoke. |
| `flash_loan` | Receiver approves `amount + fee` to the pool. A direct repayment transfer is rejected. |
| `flash_position` | Debt remains on the account. The callback pushes declared collateral to the controller; undeclared controller balances are not credited. |
| `multiply` | `Multiply` requires distinct markets. The swap bytes must be empty when both markets use the same token address, and non-empty otherwise. `Long` and `Short` reject identical token addresses. |
| `swap_debt` / `swap_collateral` | Distinct markets are required. The route must be empty when both markets use the same token address, and non-empty otherwise. |
| `repay_debt_with_collateral` | The same market nets with an empty route. Different markets withdraw and swap: the route must be empty when both markets use the same token address, and non-empty otherwise. `close_position` withdraws all remaining collateral to the caller and fails with `CannotCloseWithRemainingDebt` if any debt remains. |
| `renew_account` | Renews controller account entries and NFT state. It never renews the caller contract's persistent pointer or instance. |

## Views that need interpretation

- `account_exists(id)` checks and extends only `AccountMeta(id)`. It does not
  prove that supply maps, borrow maps, delegates, or NFT storage are live.
  Before a privileged action, also validate NFT ownership and
  `get_account_attributes`.
- `get_health_factor(id)` returns WAD and uses `i128::MAX` when there is no
  debt or no account.
- `get_collateral_amount` and `get_borrow_amount` return accrued token base
  units. `get_account_positions` returns raw RAY-scaled shares.
- `get_market_index` returns accrued RAY indexes without reading an oracle.
- `get_spoke_usage` returns RAY-scaled usage and a zeroed value when no usage
  is stored. Caps are token base units; zero is not an unlimited sentinel.

## Batch semantics

The payment vectors of `supply`, `borrow`, `withdraw`, `repay`, and
`liquidate` are non-empty. Negative amounts fail, and zero fails except in a
withdrawal leg. Duplicate market keys are aggregated in first-seen
order; for withdrawal, any zero for a market wins and means the whole
position. New supply/debt markets count against position limits.

`update_account_threshold(caller, has_risks, account_ids)` iterates the input
without an explicit length bound in
[`risk/params.rs`](../../contracts/controller/src/risk/params.rs). A keeper
must use conservative batches, simulate every batch successfully, and only
submit a batch whose footprint and budget fit.

`get_liquidation_estimate` and `get_market_indexes_detailed` reject more than
`MAX_VIEW_INPUTS` (256) entries with `InvalidPayments`. Do not apply that
bound to other views; simulate their batches.

## Failure handling

Do not maintain a local per-entrypoint error list. Decode the failing contract
first because controller/router and oracle/NFT codes collide numerically, then
use [`docs/reference/errors.md`](../../docs/reference/errors.md).

Integration failures often missed by a code-only error lookup:

- token `Error(Auth, InvalidAction)` with no contract code: missing or
  mismatched `authorize_as_current_contract`, commonly because a read consumed
  the next invocation
- host failure from a `try_` client: no contract error code exists; do not
  interpret it as `account_exists == false`
- archived local pointer: controller account may still exist, but the caller
  contract no longer knows its ID
- successful callback followed by transaction failure: post-callback
  allowance, minimum collateral, open-position, or solvency settlement failed
- router error surfaced through a controller strategy: decode it in the
  router namespace

For a `try_` client call, only `Ok(Ok(value))` is success. A contract error,
conversion error, and host invocation error are distinct branches. The
canonical pointer helper in [positions.md](positions.md#canonical-local-account-pointer)
clears state only on an explicit successful `false`.
