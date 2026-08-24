# Endpoint reference

This page is the integrator-facing map of the protocol's callable surface: every
entry point an external caller can reach, what it does, who may call it, what
options it exposes, and the semantics that are easy to get wrong.

[Architecture](architecture.md) explains *why* the surface is shaped this way.
This page states *what it is*.

## Reading these tables

**Auth column.** Every state-changing controller entry point takes an explicit
`caller: Address` and calls `require_auth()` on it. The column describes the
*additional* authority required beyond that signature:

| Value | Meaning |
| --- | --- |
| `signature` | Any address may call; only the caller's own signature is needed |
| `owner` | Caller must currently hold the account's position NFT |
| `owner/delegate` | Caller must be the NFT holder, or an active governance-approved position manager registered as a delegate on that account |
| `owner (contextual)` | Third parties may call, but with a narrowed effect — see the entry |
| `only_owner` | Contract owner (governance) via `stellar_macros::only_owner` |
| `role` | A named governance role |

**Pause column.** `gated` means the entry point carries `#[when_not_paused]` and
reverts while the controller is paused. `open` means it stays callable. The split
is deliberate: pausing must never trap a user's exit or block a liquidation.
See [ADR-0008](../explanation/decisions/0008-halt-flags-gate-liquidation-legs.md).

**Flash-loan reentrancy.** Every monetary controller verb routes through
`require_authorized_caller` (`contracts/controller/src/risk/validation.rs:12`),
which combines `caller.require_auth()` with `require_not_flash_loaning`
(`contracts/controller/src/risk/validation.rs:18`). A call made while a flash
loan is in flight reverts with `FlashLoanOngoing`.

**Units.** RAY is 10^27, WAD is 10^18, BPS is 10,000. "Asset units" means the
token's own decimals.

## Contract topology

| Contract | Reachable by end users | Role |
| --- | --- | --- |
| Controller | Yes — the protocol's front door | Lending state machine: accounts, risk, liquidation, strategies |
| Position NFT | Yes | Account ownership token; transferring it transfers the position |
| Swap aggregator | Yes, standalone | Route execution; also used internally by controller strategies |
| Price aggregator | Read-only | Validated price snapshots |
| XOXNO oracle | Signed writes, open reads | Threshold-signed price submissions |
| DeFindex strategy | Vault integrators | Adapter holding a controller account per vault |
| Pool | No mutators | Custody and market accounting; controller-only mutation |
| Governance | Role-gated, timelocked | Owns the controller |

---

## Controller — core lending

| Endpoint | Auth | Pause | Source |
| --- | --- | --- | --- |
| `supply` | `owner (contextual)` | gated | `contracts/controller/src/lib.rs:99` |
| `borrow` | `owner/delegate` | gated | `contracts/controller/src/lib.rs:113` |
| `withdraw` | `owner/delegate` | open | `contracts/controller/src/lib.rs:127` |
| `repay` | `signature` | open | `contracts/controller/src/lib.rs:139` |

### `supply(caller, account_id, spoke_id, assets) -> u64`

Deposits collateral and returns the account id.

- **Account creation is folded in.** `account_id = 0` mints a fresh position NFT
  to `caller`, binds the account to `spoke_id` in `PositionMode::Normal`, and
  returns the new id (`contracts/controller/src/account.rs`, `create_account`).
  There is no separate account-creation entry point.
- **Batch.** `assets` is `Vec<(HubAssetKey, i128)>`. Repeated legs for the same
  asset are summed and first-appearance order is preserved
  (`contracts/controller/src/payments.rs`, `aggregate_payments`).
- **Zero amounts are rejected** with `AmountMustBePositive`. Negative amounts are
  always fatal, checked before any other rule.
- **Third-party deposits are permitted but narrowed.** A caller who is neither
  owner nor delegate may only add to hub assets the account already holds a
  supply position in (`require_third_party_existing_supply`,
  `contracts/controller/src/positions/supply.rs`). This prevents a third party
  from consuming an account's supply-position slots. New accounts skip the check
  because the caller becomes the owner.
- **Credited amount is measured**, not requested. A token that delivers less than
  it is sent credits the delivered amount.
- Entry gates: spoke listing, `is_collateralizable`, halt flags, supply cap,
  `max_supply_positions`, hub active.

### `borrow(caller, account_id, borrows, to)`

Draws debt against the account's collateral.

- **`owner/delegate` only.**
- **Batch** across assets; positive amounts only.
- **`to: Option<Address>`** separates borrower from recipient. Defaults to
  `caller`. The debt is always booked to `account_id`.
- Entry gates run before the pool call; LTV coverage, health factor, and the
  minimum-borrow-collateral floor are rechecked after
  (`require_post_pool_risk_gates`, `contracts/controller/src/risk/validation.rs:31`).

### `withdraw(caller, account_id, withdrawals, to) -> Vec<(HubAssetKey, i128)>`

Removes collateral and returns the amounts actually paid.

- **`owner/delegate` only.**
- **A zero amount means withdraw the entire position** for that asset
  (`ZeroLeg::MeansAll`; internally `WITHDRAW_ALL_SENTINEL = i128::MAX`,
  `contracts/controller/src/constants.rs`). The zero is sticky: once an asset's
  running total is zero, further legs for that asset stay at "all".
- **The return value is the resolved figure** — the only way a caller learns what
  a withdraw-all actually paid.
- **`to`** redirects the payout; defaults to `caller`.
- **Not pause-gated.** A paused protocol still lets solvent users exit.
- `frozen` blocks entry but not exit, so a frozen asset remains withdrawable.

### `repay(caller, account_id, payments)`

Retires debt.

- **Fully permissionless.** There is no owner or delegate check
  (`process_repay`, `contracts/controller/src/positions/debt.rs:77`). Anyone may
  repay anyone's debt; the caller's signature is needed only to pull the tokens.
- **Batch**, positive amounts only.
- Loads and persists the debt side only.
- **Not pause-gated.**

---

## Controller — liquidation

| Endpoint | Auth | Pause | Source |
| --- | --- | --- | --- |
| `liquidate` | `signature` | open | `contracts/controller/src/lib.rs:155` |
| `clean_bad_debt` | `signature` | open | `contracts/controller/src/lib.rs:174` |

### `liquidate(liquidator, account_id, debt_payments, seize_mode) -> u64`

Repays part of an unhealthy account's debt and seizes collateral at a bonus.

- **Permissionless, including self-liquidation** by the account owner.
- **Batch repayment** across several debt assets in one call.
- **Two delivery modes** (`SeizeMode`, `common/src/types/controller.rs:241`):

| Mode | Token movement | Returns | Use when |
| --- | --- | --- | --- |
| `Transfer` | Pool pays seized collateral in underlying, withholding the protocol fee from the outbound amount | `0` | Normal case |
| `Credit(id)` | Seized **supply shares** move to a controller account; the only tokens that move are the liquidator's repayment | receiving account id | The market has no free cash |

  `Credit(0)` creates the receiving account, owned by the liquidator and bound to
  the **liquidated** account's spoke. A non-zero id must already exist, be owned
  by or delegated to the liquidator, sit in the liquidated account's spoke, and
  be in `PositionMode::Normal`.
  See [ADR-0019](../explanation/decisions/0019-share-credit-liquidation.md).

- **The bonus is not fixed at submission time.** It is a function of the
  account's live health factor against the spoke's liquidation curve
  (`liquidation_target_hf_wad`, `hf_for_max_bonus_wad`,
  `liquidation_bonus_factor_bps`). Only the asset's base bonus is guaranteed as a
  floor. A liquidator must enforce its own profitability bound and revert rather
  than execute below expectation.
- **Seizure is pro-rata across the account's entire collateral set.** That is why
  one `SeizeMode` governs the whole call rather than one per asset, and why
  `no_seize` is a separate flag from `paused`.
- Repayment is credited at the **measured** delivered amount. A debt token that
  delivers less than it is sent scales the seizure down to match.
- Triggers bad-debt socialization if the account remains insolvent and its
  residual collateral is at or below the dust threshold.

### `clean_bad_debt(caller, account_id)`

Socializes an account's remaining debt into the market's supply index and deletes
the account, burning its NFT.

- **Permissionless.**
- Only succeeds when the account is insolvent **and** residual collateral value is
  at or below `BAD_DEBT_USD_THRESHOLD` (`5 * WAD`, i.e. $5 —
  `contracts/controller/src/constants.rs`, `common/src/constants/shared.rs:36`).
  Reverts otherwise.
- Above that threshold the leftover requires the governance-only
  `force_socialize_bad_debt`; see the
  [force-socialize runbook](runbooks/force-socialize-bad-debt.md).

---

## Controller — leverage and strategies

Every entry point here calls `require_authorized_caller` (auth plus the
flash-loan-reentrancy guard) and ends in `strategy_finalize`: restamp listed
collateral LTV, apply post-pool risk gates, persist, emit the position batch
(`contracts/controller/src/strategies/mod.rs`). All are pause-gated.

| Endpoint | Auth | Source |
| --- | --- | --- |
| `multiply` | `owner/delegate` (`AccountGuard::Multiply`) | `contracts/controller/src/lib.rs:233` |
| `flash_position` | `owner/delegate` (`AccountGuard::Multiply`) | `contracts/controller/src/lib.rs:198` |
| `swap_debt` | `owner/delegate` | `contracts/controller/src/lib.rs:267` |
| `swap_collateral` | `owner/delegate` | `contracts/controller/src/lib.rs:293` |
| `repay_debt_with_collateral` | `owner/delegate` | `contracts/controller/src/lib.rs:320` |
| `migrate_from_blend` | `owner/delegate` (`AccountGuard::Migrate`) | `contracts/controller/src/lib.rs:350` |
| `flash_loan` | `signature` | `contracts/controller/src/lib.rs:182` |

### `multiply(...) -> u64`

Opens or extends a leveraged position: borrows `debt_to_flash_loan` of `debt`
into the controller, swaps it into `collateral` through the supplied route, and
deposits the proceeds as collateral.

Options:

- **`account_id = 0` creates the account.** Otherwise `AccountGuard::Multiply`
  requires owner-or-delegate authority, a matching spoke, **and** that the passed
  `mode` equals the account's stored mode, else `AccountModeMismatch`.
- **`mode: PositionMode`** — `Normal | Multiply | Long | Short`
  (`common/src/types/shared.rs:32`). Fixed at account creation and enforced on
  every later `multiply` call against that account.
- **`initial_payment: Option<(HubAssetKey, i128)>`** — user capital contributed
  alongside the borrowed amount, folded into the deposit.
- **`convert_swap: Option<Bytes>`** — a second route that converts the initial
  payment into the collateral asset first. Omit it when the initial payment is
  already denominated in the collateral or the debt asset.

### `flash_position(...) -> u64`

Mints strategy debt onto the account **with no flash fee**, forwards the measured
tokens to `receiver`, invokes its `execute_flash_position` callback, and deposits
measured controller-balance increases of the declared collaterals. It does not
repay. See [ADR-0020](../explanation/decisions/0020-flash-position-callback-multiply.md).

- `receiver` must be a deployed Wasm contract, and must be neither the controller
  nor the pool (`InvalidFlashloanReceiver`).
- Same `AccountGuard::Multiply` account semantics as `multiply`.
- The position must remain open: `require_flash_position_still_open`
  (`contracts/controller/src/strategies/flash_position.rs:371`) is asserted both
  before and after `strategy_finalize`, so the account must end with live debt in
  `debt` and at least one supply position, else `FlashPositionClosed` (505).
- Observer note: the strategy-debt mint is tagged `FlashPos` (16) in the position
  batch; collateral deposited from the callback is tagged `Supply`, identically
  to an ordinary deposit.

#### The two declaration lists

Soroban cannot enumerate an address's token balances — a contract can only ask a
*named* token contract for a balance. The controller therefore cannot discover
what an untrusted receiver sent back; it can only check assets it was told to
check. Both lists are that declaration, and each does double duty: it bounds the
work (every entry costs two cross-contract `balance()` calls, so both lists are
capped at `max_supply_positions`) and it bounds the trust (every entry is
validated against the spoke's listing *before* the callback runs, so
`token::Client` is never invoked on an arbitrary caller-chosen address).

Two balance snapshots are taken **inside** the flash guard, immediately before
the callback (`flash_position.rs:111-113`). Everything afterwards is a diff
against those baselines.

#### `collaterals: Vec<(HubAssetKey, i128)>` — the amount is a minimum

The `i128` is a **slippage floor**, not a cap and not an exact expectation. The
measured delta is what gets deposited, never the declared figure
(`collect_collateral_deposits`, `flash_position.rs:335`).

| Receiver delivers | Result |
| --- | --- |
| More than declared | All of it becomes collateral |
| Less than declared | Revert `CollateralMinimumNotMet` (504) |
| Zero, where `min_amount` is 0 | Leg silently skipped (`delta > 0` filter) |
| Zero on every declared asset | Revert `CollateralMinimumNotMet` — `deposits` is empty |

Validation, all before the callback (`validate_collaterals`,
`flash_position.rs:152`): non-empty; `len() <= max_supply_positions`; each
`min_amount >= 0`; no duplicate `HubAssetKey` **and** no duplicate underlying
`Address` (the same token may be listed in two hubs); `require_can_supply` for
spoke listing, collateralizable flag, and halt flags; **at least one
`min_amount > 0`** or `CollateralRequired` (503); then the standard supply-entry
gates for caps and position-count limits.

Pre-validating the whole set before control passes to the receiver is what makes
the post-callback deposit safe: every asset `process_deposit` can touch has
already cleared listing, caps, and flags. A token the receiver pushes that is not
in the declared set has no snapshot, so no delta is ever computed for it and it
cannot be credited as collateral.

#### `refund_assets: Vec<Address>` — delta only, paid to the caller

`refund_controller_balance_delta`
(`contracts/controller/src/strategies/legs.rs:226`) transfers
`balance - baseline`, and only when that difference is positive. It is **never**
the controller's gross balance. The recipient is **`caller`**, not the receiver
and not the account owner. The leg runs after `process_deposit` and outside the
flash guard.

Constraints (`validate_refund_assets`, `flash_position.rs:200`):

| Rule | Error |
| --- | --- |
| `len() <= max_supply_positions` | `InvalidPayments` |
| No duplicates | `InvalidPayments` |
| Must be listed and active in the account's spoke | listing error |
| Must not overlap any declared collateral asset | `InvalidPayments` |

The listing requirement exists because the refund leg hands a caller-supplied
address to `token::Client` *after the flash guard has closed*; requiring it to be
listed keeps that call on a governance-approved contract.

Note what is not forbidden: **the debt asset may appear in `refund_assets`**, and
the overlap check is only against `collaterals`.

#### Returning the debt token does not repay

`flash_position` has no repay leg. Debt tokens handed back by the receiver are
refunded to the caller and the debt stands at its full minted amount. Asserted by
`test_flash_position_returning_debt_token_does_not_repay`
(`tests/test-harness/tests/strategy/flash_position.rs`).

#### Refunds are silent, and there is no dry run

No view or return value tells a caller in advance whether a refund will occur;
`flash_position` returns only the account id, and there is no simulation
counterpart to `get_liquidation_estimate`. No refund event exists anywhere in
`contracts/controller/src/events/`, and `FlashPositionEvent` carries no refund
field. The only on-chain trace is the token contract's own `transfer` event,
controller to caller.

This is not a prediction problem in practice: `receiver` is a contract the caller
deployed, so the caller already knows what its own callback hands back.
`refund_assets` is a declaration of intent, not a forecast. Refunds become
non-zero through swap slippage leaving unspent debt tokens, partial fills,
over-delivery of a non-collateral asset, rounding dust from multi-hop routes, or
a deliberate return of the debt token.

Listing an asset that turns out to have a zero delta costs two `balance()` calls
and one cached spoke-config lookup, with no transfer and no event. Over-listing
costs a little budget; under-listing is irreversible.

#### Unlisted leftovers are stranded, and unstealable

An asset that is neither a declared collateral nor in `refund_assets` stays in
the controller permanently.

No later caller can sweep it. `refund_before` is captured inside the guard,
before the callback, so any pre-existing controller balance is part of the
**baseline**. A subsequent user listing that asset refunds only what their own
callback pushed; the earlier leftover sits inside their baseline and is
untouchable. The original caller cannot recover it either, for the same reason.

Every controller read of its own token balance is a baseline capture paired with
a `checked_sub` — `strategies/legs.rs:69,102,119`,
`strategies/flash_position.rs:256,269,346`, `strategies/swap/balances.rs:18,33`,
`strategies/migrate_blend.rs:302,325,361`. **No path transfers the controller's
gross balance.** `ControllerAdmin` exposes no sweep or rescue entry point, so
stranded balances are inaccessible to everyone, including governance.

That is a deliberate trade. A rescue sweep would be a function that moves the
controller's gross balance somewhere — exactly the primitive the measured-delta
discipline exists to eliminate, and it would reintroduce that vector across every
strategy in order to recover dust that only accumulates when a caller's own
receiver misbehaves. The protocol chose unrecoverable over sweepable.

#### Receiver author checklist

1. Set each `min_amount` to a real slippage floor — it is the only post-callback
   protection against a bad route.
2. At least one `min_amount` must be positive.
3. List in `refund_assets` every asset the callback might return, including the
   debt asset if it might not be fully spent.
4. `refund_assets` may not overlap `collaterals`, may not duplicate, and every
   entry must be listed in the account's spoke.
5. Do not expect a returned debt token to repay anything.
6. End the callback with the position still open: live debt in `debt` and at
   least one supply position.

### `swap_debt(caller, account_id, existing_debt, amount, new_debt, swap)`

Refinances one debt asset into another: borrows `amount` of `new_debt`, swaps to
`existing_debt`, repays the existing position. The two assets must differ and
`existing_debt`'s hub must be active.

Indexer note: **both** legs carry `SwDebtR` — the new borrow and the old repay.

### `swap_collateral(caller, account_id, current, amount, new, swap)`

Rotates collateral without unwinding debt: withdraws `amount` of `current`, swaps
to `new`, redeposits. The two assets must differ and `current`'s hub must be
active.

### `repay_debt_with_collateral(caller, account_id, collateral, collateral_amount, debt, swap, close_position)`

Deleverages using collateral already in the account.

- **When collateral and debt are the same market the position is netted directly
  on the pool** — no swap, no router trust boundary.
- **`close_position: bool`** — when set and no debt remains after the repayment,
  also withdraws all remaining collateral to `caller`. A one-call full exit.

### `migrate_from_blend(...) -> u64`

Moves a position out of an approved Blend pool: borrows up to `debt_caps` to
clear the caller's Blend debt, sweeps `collateral_assets` and `supply_assets`
into this pool as collateral, repays any leftover borrowed amount.

- `blend_pool` must be approved (`is_blend_pool_approved`); approval is a
  governance action.
- Reverts if the request carries no assets.

### `flash_loan(caller, asset, amount, receiver, data)`

Standard flash loan through the pool.

- `receiver` must be a Wasm contract.
- Sets the flash-loan flag for the duration of the pool call, which blocks every
  monetary controller verb via `require_not_flash_loaning`.
- Principal plus fee is pulled back before the call returns; repayment is
  verified against exact pool balances.
- Emits `FlashLoanEvent` with the fee charged.

---

## Controller — account and delegation

| Endpoint | Auth | Pause | Source |
| --- | --- | --- | --- |
| `add_delegate` | `owner` | gated | `contracts/controller/src/lib.rs:417` |
| `remove_delegate` | `owner` | open | `contracts/controller/src/lib.rs:423` |
| `renew_account` | `owner` | open | `contracts/controller/src/lib.rs:409` |

- **`add_delegate` requires the delegate to be an active, governance-approved
  position manager at the moment of the grant.** A dormant grant to an address
  governance has not yet approved would arm on later activation, so it is
  rejected outright (`contracts/controller/src/account.rs`, `set_account_delegate`).
- **`remove_delegate` is deliberately not pause-gated** — revoking authority must
  never be blocked.
- Delegates are capped at `MAX_DELEGATES = 16`.
- **A delegate grant is bound to the owner who made it.** `DelegateGrant` stores
  `granted_by`; transferring the NFT deactivates the previous owner's grants
  immediately, because `get_delegates` reads as empty for anyone else. The stale
  entry is purged on the new owner's next delegate write. No explicit cleanup
  call exists or is needed.
- **`renew_account` extends both the account's storage TTL and the NFT `Owner`
  entry**, closing the 30d/120d renewal asymmetry with OZ's `owner_of`
  (INV-STOR-02).

---

## Controller — permissionless maintenance

All four require only the caller's signature. None carries a privileged role.

| Endpoint | Pause | Source |
| --- | --- | --- |
| `update_indexes(caller, assets)` | gated | `contracts/controller/src/lib.rs:379` |
| `claim_revenue(caller, assets) -> Vec<i128>` | gated | `contracts/controller/src/lib.rs:387` |
| `update_account_threshold(caller, has_risks, account_ids)` | gated | `contracts/controller/src/lib.rs:396` |
| `recapitalize(payer, hub_asset, amount) -> i128` | open | `contracts/controller/src/lib.rs:403` |

- **`claim_revenue` is callable by anyone, but the proceeds go only to the
  configured accumulator.** The caller cannot name a recipient. Reverts if no
  accumulator has been configured. Returns the claimed amount per asset, in
  input order.
- **`update_account_threshold` refreshes cached risk parameters** after a listing
  change. With `has_risks = true` it also reloads the debt side and reverts if
  the account's health factor falls below `THRESHOLD_UPDATE_MIN_HF_RAW`
  (1.05 WAD) — so a keeper cannot use a threshold tightening to push accounts
  into liquidation. Accounts with no stored metadata or no supply positions are
  skipped rather than reverting.
- **`recapitalize` applies only up to the actual shortfall and refunds the
  excess**, returning the amount actually applied. Credit is measured, not
  requested.

---

## Controller — views

Read-only. None mutates state. Views that accept a list are bounded by
`MAX_VIEW_INPUTS = 256` via `require_view_inputs_bound`
(`contracts/controller/src/views.rs:18`).

**Account risk**

| View | Returns |
| --- | --- |
| `is_liquidatable(account_id) -> bool` | Health factor below 1 WAD |
| `get_health_factor(account_id) -> i128` | WAD ratio of liquidation-weighted collateral to debt; `i128::MAX` when debt-free or the account does not exist |
| `get_total_collateral_usd(account_id) -> i128` | WAD, unweighted |
| `get_total_borrow_usd(account_id) -> i128` | WAD |
| `get_liquidation_collateral(account_id) -> i128` | WAD, threshold-weighted — the ceiling on seizable collateral |
| `get_ltv_collateral_usd(account_id) -> i128` | WAD, LTV-weighted — the ceiling on borrowing power |

**Account state**

| View | Returns |
| --- | --- |
| `get_account_positions(account_id)` | `(Map<HubAssetKey, AccountPositionRaw>, Map<HubAssetKey, DebtPositionRaw>)` |
| `get_account_attributes(account_id) -> AccountAttributes` | Spoke id and position mode |
| `account_exists(account_id) -> bool` | Whether the account has been created |
| `get_collateral_amount(account_id, hub_asset) -> i128` | Asset units; zero if no such position |
| `get_borrow_amount(account_id, hub_asset) -> i128` | Asset units; zero if no such position |

**Simulation**

`get_liquidation_estimate(account_id, debt_payments, seize_mode) -> LiquidationEstimate`
returns `seized_collaterals`, `protocol_fees`, `refunds`, `max_payment_wad`, and
`bonus_rate_bps`.

Two traps: `seized_collaterals` is **gross** of `protocol_fees` (the liquidator
ends up with the difference), and **the units follow the mode** — asset units for
`Transfer`, RAY-scaled supply shares for `Credit`.

**Market and configuration**

| View | Returns |
| --- | --- |
| `get_market_index(hub_asset) -> MarketIndexRaw` | Supply and borrow indexes, RAY |
| `get_market_indexes_detailed(hub_assets) -> Vec<MarketIndexView>` | Indexes plus resolved price, primary and anchor legs, `stale` / `deviation` / `valid` flags |
| `get_spoke(spoke_id) -> SpokeConfig` | Deprecation flag and liquidation-curve parameters |
| `get_spoke_asset(spoke_id, hub_asset) -> SpokeAssetConfig` | Per-spoke risk config; panics with `AssetNotInSpoke` if unlisted |
| `get_spoke_usage(spoke_id, hub_asset) -> SpokeUsageRaw` | RAY-scaled supply and borrow usage against caps; zeroed default if none recorded |
| `get_pool_address() -> Address` | Deployed pool |
| `price_aggregator() -> Address` | Configured price aggregator |
| `get_min_borrow_collateral_usd() -> i128` | WAD floor for opening a borrow |
| `is_blend_pool_approved(pool) -> bool` | Blend migration allowlist |
| `get_app_version() -> u32` | Migration version |

---

## Controller — administration

Not end-user surface. Every entry point in `ControllerAdmin`
(`interfaces/controller/src/admin.rs`) carries `#[only_owner]`, and the owner is
the governance contract. Reaching them requires a governance proposal, the
applicable timelock tier, and execution. `accept_ownership` is the sole
unguarded entry, callable only by the pending owner.

The surface covers: aggregator and accumulator wiring, position limits, the
minimum-borrow floor, position-manager registration, Blend allowlisting,
hub/spoke lifecycle, spoke liquidation curves, asset listing and flags, pool and
NFT deployment and upgrade, market creation and rate-model updates,
`force_socialize_bad_debt`, pause/unpause, controller upgrade and migration, and
ownership transfer.

Emergency asymmetry: the guardian can pause and tighten listing flags
immediately, but **reopening requires timelocked governance**. See
[Architecture](architecture.md#emergency-and-governance-model).

---

## Position NFT

`contracts/position-nft` implements OpenZeppelin `NonFungibleToken` plus
`NonFungibleEnumerable`, so holders get the standard surface: `transfer`,
`transfer_from`, `approve`, `approve_for_all`, `balance`, `owner_of`,
`get_owner_token_id`, `get_token_id`, and metadata.

The consequential property: **`account_id` is the `token_id`, and the controller
stores no owner address.** It calls `owner_of(account_id)` on every authority
check (`storage::account_owner`). Transferring the token therefore transfers the
whole lending position — collateral and debt together, atomically, with no
protocol-side handover step.

`mint`, `burn`, and `renew` are controller-only. `upgrade` is governance-routed.
Account deletion always goes through `remove_account_and_burn_nft`
(`contracts/controller/src/account.rs`) so that a deleted account can never leave
a live token behind.

---

## Swap aggregator

`execute_strategy(sender, total_in, swap_xdr) -> i128` is the user-facing
mutator. It decodes `swap_xdr` as a `StrategyPayload`, pulls `total_in` from
`sender`, runs optional LP burn, swap paths, and LP mint, applies fees, enforces
`total_min_out`, and returns the delivered output. It is usable standalone, not
only through controller strategies.

`claim_referral_fees(id, tokens)` pays a referral's accrued balances to its
configured owner — the only claim path not restricted to the contract owner.

Open reads: `admin`, `static_fee_bps`, `referral`, `referral_counter`,
`is_whitelisted`, `whitelisted_tokens`, `admin_fee_balance`,
`referral_fee_balance`.

Owner-only: static fee, whitelist management, referral lifecycle,
`claim_admin_fees`, `sweep_balance` (which leaves fee buckets intact), `upgrade`,
and the `Ownable` two-step ownership transfer.

The controller treats this contract as **untrusted**: it grants authority for a
stated input, ignores returned claims, and settles on measured balance deltas.
See [ADR-0011](../explanation/decisions/0011-untrusted-swap-router-balance-deltas.md)
and [ADR-0013](../explanation/decisions/0013-token-custody-split-measured-deltas.md).

---

## Price aggregator

Open reads:

| View | Returns |
| --- | --- |
| `prices(keys) -> Map<PriceKey, PriceFeedRaw>` | Resolved prices |
| `quotes(keys) -> Map<PriceKey, PriceStatus>` | Per-key status including validity |
| `price_spread(key) -> (i128, i128)` | Conservative low/high pair |
| `oracle(key) -> Option<AssetOracle>` | Configured source definition |
| `get_owner() -> Option<Address>` | Owner |

`set_oracle`, `set_sanity_band`, and `set_tolerance` are governance-routed.
`set_sanity_band` additionally has a guardian fast path through the governance
contract.

Every valuation-dependent mutation consumes a complete snapshot. Source failure,
staleness, disagreement, or a failed sanity rule reverts the operation: the
protocol halts risk-taking rather than acting on a questionable price.

---

## XOXNO oracle

Writes are threshold-signed: `submit_price` and `submit_prices` verify signer
set membership and the configured threshold, plus submission age and relative
skew bounds.

Reads are open and Reflector-compatible: `lastprice`, `price`, `prices`,
`read_price_data`, `read_price_data_for_feed`, `read_price_history`, `assets`,
`feeds`, `base`, `decimals`, `resolution`, `max_stale_seconds`,
`max_submission_age_seconds`, `max_relative_skew_seconds`.

Signer set, threshold, feed registration, and the staleness bounds are
governance-routed.

`set_threshold`, `set_max_submission_age_seconds`, and
`set_max_relative_skew_seconds` store the new bound only; they do not re-derive
aggregates that already exist. Follow a change with `recompute_feeds(feed_ids)`
to apply it to feeds that already hold an aggregate, in batches small enough to
stay inside the transaction footprint limit (about one ledger entry per signer
plus three, per feed). `feeds()` enumerates the registered ids. Sweeping every
feed inside the setter would make its footprint grow with the feed count and
eventually make those settings permanently unchangeable.

---

## DeFindex strategy adapter

`contracts/defindex-strategy` implements `DeFindexStrategyTrait` for vault
integrators: `asset()`, `deposit(amount, from)`, `withdraw(...)`,
`harvest(from, data)`, `balance(from)`.

The adapter holds one controller account per vault and is subject to the ordinary
account, spoke, and solvency rules. It has no privileged path into the pool.

---

## Pool

`interfaces/pool/src/lib.rs` declares a full ABI, but **every mutator is
controller-only**. The publicly meaningful surface is the view set:
`get_utilisation`, `get_reserves`, `get_deposit_rate`, `get_borrow_rate`,
`get_revenue`, `get_supplied_amount`, `get_borrowed_amount`, `get_delta_time`,
`get_sync_data`, `get_bulk_indexes`.

`get_reserves` reports the market's cash figure. Reserves are not a separate
quantity from cash.

The pool does not infer risk from token balances. It keeps its own cash book and
market share totals; inbound transfers are credited at the amount actually
received.

---

## Cross-cutting semantics

**1. `account_id = 0` means "create".** It applies to `supply`, `multiply`,
`flash_position`, `migrate_from_blend`, and as `SeizeMode::Credit(0)` in
`liquidate`. Each of these returns the resolved account id.

**2. Spoke binding is permanent.** An account is bound to one spoke at creation
and every later call rechecks it (`SpokeMismatch`). The spoke supplies LTV,
liquidation threshold, bonus, fees, caps, and halt flags for every position in
that account. The binding never changes.

**3. Position mode is fixed at creation.** `multiply` and `flash_position` assert
the passed mode equals the stored mode (`AccountModeMismatch`).

**4. Three independent halt flags** (`SpokeAssetConfig`,
`common/src/types/controller.rs`):

| Flag | Blocks |
| --- | --- |
| `paused` | Every user verb for that asset |
| `frozen` | Entry only; exit remains open |
| `no_seize` | The liquidation seizure leg only |

Seizure is deliberately not gated by `paused`, because seizure is pro-rata across
an account's whole collateral set — pausing one collateral would otherwise halt
liquidation of every account holding it.

**5. Zero is overloaded by direction.** On entry legs (`supply`, `borrow`,
`repay`) a zero amount is rejected. On `withdraw` it means "the entire position".

**6. Batching is uniform.** Multi-asset calls sum duplicate legs per asset and
preserve first-appearance ordering. Overflow in the sum reverts with
`MathOverflow`.

**7. Amounts are measured, never assumed.** Fee-on-transfer and short-delivering
tokens are first-class supported. `LiquidationEvent.repaid_usd_wad` reports the
delivered repayment, never the planned one.

**8. `LiqSeize` is gross, `LiqCredit` is net.** The protocol fee is taken between
the two legs of a share-credit liquidation. Summing both tags as the same
quantity double-counts. `SeizeMode::Transfer` emits only `LiqSeize`, also gross.

**9. A `Credit(0)` account has no creation event.** It is announced only through
the second position batch's `account_attributes`, and returned to the caller.
An indexer that discovers accounts from a creation event alone will miss it.

**10. Pause protects exits.** `withdraw`, `repay`, `liquidate`,
`clean_bad_debt`, `recapitalize`, `renew_account`, and `remove_delegate` stay
callable while the protocol is paused. Entry and leverage do not.
