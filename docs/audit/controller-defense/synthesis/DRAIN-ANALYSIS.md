# Pool-liquidity drain analysis

Adversarial enumeration of every way I can construct to take assets out of the
liquidity pool without an equal-or-greater claim being booked against the
taker. Method: read the pool's accounting layer first (what *must* hold), then
walk every path that moves tokens out and check it against that layer. Source
re-derived at `main@a2afb21`; nothing here is taken from the A001–A110 corpus.

Companion to [FINAL.md](FINAL.md); the independent re-derivation of that report is its §11.

## 1. The two structural facts that close most of the attack surface

**`cash` is stored accounting state, not `token.balance(pool)`.**
`contracts/pool/src/cache/mod.rs` loads `cash` from `PoolStateRaw`; every
mutation goes through `credit_cash` / `debit_cash`
(`contracts/pool/src/cache/cash.rs`). `guards::backing_shortfall` is computed
from `supplied`/`borrowed`/`cash`, never from a live balance. Consequence: a
direct token transfer to the pool address is **inert** — it cannot move the
share price, cannot be borrowed against, and cannot be withdrawn. The entire
ERC-4626 donation / share-inflation / first-depositor family is structurally
absent, not merely guarded.

**Every rounding conversion is directed against the taker.**
`common/src/rates/scaling.rs`: supply mints `div_floor`, borrow mints
`div_ceil`, supplier claims unscale `mul_floor`+`to_asset_floor`, debt
liabilities unscale `mul_ceil`+`to_asset_ceil`. `resolve_withdrawal` pays the
**floor** value on a full close and burns **ceil** shares on a partial;
`resolve_repay` requires the **ceil** debt to close and burns **floor** shares
on a partial; `resolve_net_settle` burns ceil supply against floor debt. Each
leg additionally rejects a nonzero amount that would round to zero shares
(`SupplyRoundsToZeroShares`, `BorrowRoundsToZeroShares`,
`WithdrawRoundsToZeroShares`, `RepayRoundsToZeroShares`,
`NetSettleRoundsToZeroShares`). A dust-farming loop loses money.

`transfer_out` deliberately "does not adjust accounting cash", so each outbound
path must debit separately. All six call sites pair correctly:

| Path | Tokens out | Cash accounting |
|---|---|---|
| `ops/borrow.rs` | `mutation.actual_amount` | `require_reserves` + `debit_cash(amount)` |
| `ops/withdraw.rs` | `net_transfer` | `require_reserves` + `debit_cash(net_transfer)` |
| `ops/strategy.rs` | `amount - fee` | `debit_cash(amount - fee)`; fee stays as revenue |
| `ops/revenue.rs` | `min(cash, floor(revenue))` | `debit_cash(net_transfer)` |
| `ops/repay.rs` | `overpayment` | `credit_cash(net_repay)`; controller moved `net_repay + overpayment` in |
| `ops/recapitalize.rs` | `amount - applied` | `credit_cash(applied)` |

## 2. `recapitalize` — full trace

Permissionless (`caller-auth`: `payer.require_auth()` only), and the only
controller money verb with no listing check on its `hub_asset` argument. It is
nonetheless closed:

1. `contracts/controller/src/lib.rs:401` → `keepers::recapitalize`.
   `payer.require_auth()`, `require_not_flash_loaning`,
   `require_positive_amount`.
2. `payments::transfer_amount_measured(payer → pool, amount)` — credits only
   what arrives, so a fee-on-transfer asset cannot over-credit.
3. `pool_recapitalize_call(hub_asset, payer, received)`. Pool side is
   `#[only_owner]` (owner = controller, set by `deploy_pool` passing
   `env.current_contract_address()` as the constructor admin), so the pool leg
   is unreachable except through step 2.
4. `ops::renewed_market` → `Cache::load` → `storage::read_params` /
   `read_state`, both of which `panic_with_error!(PoolNotInitialized)` on a
   missing market. **This is the gate that makes the absent listing check
   safe**: an attacker-supplied `hub_asset` naming an unknown asset reverts,
   and the whole transaction rolls back including step 2's transfer.
5. `applied = min(amount, guards::backing_shortfall(cache))`;
   `credit_cash(applied)`; `refund = amount - applied` returned to `payer`.

Three properties worth stating explicitly, because A016 (677 bytes) does not:

- **It cannot be used as a donation attack.** The clamp at `backing_shortfall`
  means cash can only be restored to backing parity, never pushed above it. No
  share is minted, so the injection is a pure gift to existing suppliers up to
  the shortfall and refunded beyond it.
- **The asset transferred and the market credited are the same key.**
  `hub_asset.asset` is both the transfer asset in step 2 and part of the market
  key in step 4, so there is no confused-deputy variant (pay in asset X,
  credit market Y).
- **It is reachable while the controller is paused.** `lib.rs:401` carries no
  `#[when_not_paused]`. Intentional and safe — the verb only adds backing —
  but it means `recapitalize` and `renew_account` are the controller's only
  pause-independent mutators, which is worth stating in the pause matrix.

The unmeasured refund hop is already pinned by
`tests/test-harness/tests/controller/outbound_transfer_measurement.rs::recapitalize_refund_is_unmeasured_but_strands_nothing`.

## 3. Enumeration

`✓` = closed by a mechanism I re-derived from source. `!` = open.
Vector 27 was pursued as a live finding and retracted — see §4.

| # | Attack | Mechanism | What closes it |
|---|---|---|---|
| 1 | Donate tokens to inflate share price, then withdraw | ERC-4626 inflation | ✓ `cash` is stored, never balance-derived |
| 2 | First-depositor share-price manipulation | Tiny initial supply, then donate | ✓ indexes start at RAY and move only on time-accrual, revenue mint, and bad-debt socialization — never on supply/withdraw volume |
| 3 | Dust-farm rounding across many small ops | Asymmetric floor/ceil | ✓ all conversions directed against the taker + five RoundsToZeroShares asserts |
| 4 | Withdraw more than held | Oversized `amount` | ✓ `resolve_withdrawal` caps at `pos_scaled`; full close pays floor |
| 5 | Repay less than owed, close position | Undersized `amount` | ✓ `resolve_repay` requires `amount >= ceil(debt)` to burn the position |
| 6 | Borrow past available liquidity | Oversized borrow | ✓ `require_reserves` + `require_liquidation_buffer` + `require_utilization_below_max` |
| 7 | Withdraw leaving debt unbacked | Pull collateral out from under debt | ✓ pool `require_solvent_withdraw_state`; controller `require_post_pool_risk_gates` (LTV ≥ debt, HF ≥ 1 WAD, min-borrow floor) |
| 8 | Supply into an insolvent book to dilute | Enter after a shortfall | ✓ `guards::require_backed_market` on supply |
| 9 | Flash loan and never repay | Skip repayment | ✓ `require_wasm_receiver`; **balance equality** (`==`, not `>=`) asserted after payout, again after the callback, and after collection; allowance pre-checked; repayment pulled by the pool via `transfer_from`, never pushed |
| 10 | Flash loan, reenter to observe/mutate mid-state | Callback reentry | ✓ `with_flash_guard` around the callback; `require_not_flash_loaning` at every monetary entrypoint |
| 11 | Router keeps the swap proceeds | Malicious/compromised route | **!** controller asserts only `received > 0` (`strategies/swap.rs::verify_router_output`). Known: A048/A056, ranked P0 in the FINAL report |
| 12 | Call the pool directly, bypassing controller risk gates | Skip HF checks | ✓ every pool mutator is `#[only_owner]`, owner = controller |
| 13 | Name a fake or unlisted `hub_asset` | Confused market key | ✓ `read_params`/`read_state` panic `PoolNotInitialized` |
| 14 | Recapitalize as an attack | Over-credit, or credit market Y paying asset X | ✓ see §2 |
| 15 | Liquidate a healthy account | Force a bonus payout | ✓ HF gate before planning; `require_not_flash_loaning` |
| 16 | Seize more collateral than the account holds | Oversized seizure | ✓ per-leg clamp at `actual_ray`; `LiquidationPlan::validate` bounds every entry |
| 17 | Extract an outsized liquidation bonus | Push bonus above the collateral buffer | ✓ `max_bonus_for_threshold` caps at `(1−thr)/thr`; `max_hf_preserving_bonus_bps` caps at `hf/proportion − 1`; `FullCloseRequired` when no partial is priceable |
| 18 | Credit-mode seize mints unbacked supplier claims | Mint instead of reclassify | ✓ `absorb_supply_as_revenue` raises `revenue` only; `require_revenue_backed` asserts `revenue ≤ supplied`; `split_seized_shares` asserts `fee + liquidator == seized` exactly, fee rounded up |
| 19 | Under-deliver the liquidation repayment, keep the full seizure | Fee-on-transfer repay leg | ✓ `transfer_amount_measured` per leg + `scale_seizures_to_received` floors every seizure field by `received/planned` |
| 20 | Claim protocol revenue beyond what accrued | Drain via revenue | ✓ `burn_claimable_revenue` = `min(cash, floor(revenue))`, burns shares from both `revenue` and `supplied`; pays the pool's Ownable owner (the controller), which forwards a measured delta to the accumulator |
| 21 | Socialize bad debt to move the index adversarially | Force supplier loss | ✓ auto path capped at total supplied value and floored at `SUPPLY_INDEX_FLOOR_RAW`; `force_socialize_bad_debt` is owner-only, Sensitive tier |
| 22 | Self-liquidate to extract value | Own-account liquidation | ✓ net wealth change is `−protocol_fee`; requires HF < 1. (Does skip `require_utilization_below_max`, which is the buffer's purpose) |
| 23 | Grief accrual to inflate interest | Spam `update_indexes` | ✓ accrual is purely time-driven; `needs_accrual()` no-ops within a ledger; long gaps chunked at `MAX_COMPOUND_DELTA_MS` |
| 24 | Over-admit past a spoke cap | Missing usage row | contingent — `apply_exit` no-ops on a missing row (A080). Reachability closed today: `remove_asset_from_spoke` asserts usage is zero (`SpokeAssetInUse`), and shared keys renew TTL on read *and* write |
| 25 | Exhaust the pool via unbounded input | Uncapped keeper/mutator `Vec`s | ✓ as a loss vector — costs the attacker their own fees only |
| 26 | Governance-speed parameter attack | List a bad asset, re-point a feed, then borrow | **!** `timelock_min_delay_ledgers: 12` on mainnet + Standard tier = `min` ⇒ ~72 s on `AddAssetToSpoke`, `EditAssetInSpoke`, `ConfigureAssetOracle`, `EditOracleTolerance`. See FINAL.md §11.1 |
| 27 | Token transfer hook reenters an unguarded payout leg | Listed callback-capable token | ✓ **the Soroban host refuses it** — see §4 |

## 4. Retracted: token-hook reentrancy into the unguarded payout legs

I pursued this as the headline finding and it does not stand. Recording it in
full, because the reasoning is the kind that looks airtight from source alone
and the corpus contains the same error.

**The observation that is true.** INV-FLASH-02
(`docs/reference/invariants.md:644`) states that "every window that hands
control to an untrusted contract — a receiver callback, a router, a Blend pool,
**or a listed token whose `transfer` may run a hook** — is wrapped in
`with_flash_guard`", and lists six setters: `flash_loan.rs:35`,
`flash_position.rs:110`, `swap.rs:89`, `legs.rs:103`, `debt.rs:273`,
`blend.rs:91`. Every one is either an explicit external invocation or a pool
leg that moves funds **into the controller**. The user-facing payout legs are
genuinely unguarded, and their recipient is genuinely attacker-chosen
(`require_external_recipient`, `positions/mod.rs:43`, rejects only the
controller and pool addresses):

| Unguarded window | Token moves to |
|---|---|
| `settle_debt` → `pool_borrow_call` | caller-supplied `to` |
| `apply_withdraw_batch` → `pool_withdraw_call` | caller-supplied `to`, or the liquidator |
| `apply_repay_batch` → `pool_repay_call` → refund | `payer` |
| `keepers::recapitalize` → refund | `payer` |
| `settle_supply` → `transfer_amount_measured(caller → pool)` | hook fires sender-side |
| `legs.rs:83` `refund_controller_balance_delta` | `caller` |

The three ordering facts are also true: the pool commits before it transfers
(`accounting()` then `transfer_out`, inside `run_batch`);
`require_post_pool_risk_gates` reads the **in-memory** `Account`, not storage;
and `finalize_position_flow` is **last-write-wins per side**
(`PositionSides::Debt` on borrow, `Supply` on withdraw), with per-account
positions living only in the controller. On any chain that permits reentrancy,
that composes into debt erasure: outer `borrow` pays out → hook reenters
`borrow` → inner passes solvency against stale debt and persists `Debt` →
outer overwrites it from its own snapshot, cash already gone.

**Why it does not stand.** The Soroban host refuses to invoke a contract
already on the call stack. `tests/test-harness/tests/poc_host_reentrancy.rs`
pins it, with a control to isolate the variable:

```
A -> B -> C : Ok(Ok(7))                              // same 3-hop shape, no repeat
A -> B -> A : Err(Ok(Error(Context, InvalidAction))) // host refuses the reentry
```

When a listed token's `transfer` runs, the stack is
controller → pool → token. The controller *and* the pool are both on it, so the
hook cannot reach either, whatever the flag says. Multi-operation transactions
do not help: `with_flash_guard` clears on the way out, and a failed operation
rolls back. The precondition for every shape above is unreachable.

**What survives.**

1. **The guard is defense-in-depth over a host guarantee, not the barrier.**
   Worth saying plainly in INV-FLASH-02, which currently reads as though guard
   placement is safety-critical — the reading that sent me hunting for
   unguarded windows, and that would send an external auditor the same way.
   `STRIDE.md:425` already states the model correctly ("no EVM-style mid-state
   external observation"); the invariant and the finding files do not carry it.

2. **No test distinguishes the guard from the host.** The four adversarial
   hook/router tests (`strategy/adversarial.rs` ×2,
   `strategy/flash_position_adversarial.rs`,
   `controller/flash_loan_adversarial.rs`) assert only `result.is_err()`. The
   host error arrives before any controller code runs, so those tests pass
   identically with `with_flash_guard` deleted — they pin nothing about the
   guard. `meta/reentrancy_matrix.rs` force-sets the flag and tests the
   *checker* in isolation, never a real nesting. So INV-FLASH-02's
   "VERIFIED" rests on assertions that cannot fail for the stated reason. This
   is the report's A108 evidence-density residual landing on its most
   safety-critical invariant.

3. **A048's Gap (2) is false and the FINAL report inherits it.** A048 records
   "Deposit leg runs with flash flag clear — listed-token transfer hooks can
   reenter monetary entrypoints mid-settlement (A007 residual)". The host makes
   that unreachable. It should be struck, not carried as a residual.

4. **A055 stays where the report put it.** With reentrancy off the table, a
   non-SAC listed token is back to inexact delivery only, and the FINAL
   report's `C / M ≤ that market's TVL` band and P1 ranking are correct. My
   earlier re-banding to cross-market, and the promotion of SAC-only listing to
   P0, are both withdrawn.

## 5. Actions

| # | Action | Consequence if not done |
|---|---|---|
| 1 | State the host non-reentrancy guarantee in INV-FLASH-02 and in the threat model's flash row, with `with_flash_guard` described as the second barrier | The invariant reads as though guard placement is the barrier. Two independent readers (this analysis, and A048) built a false residual out of it; an external auditor will spend the same time |
| 2 | Make the four adversarial hook/router tests assert the *specific* rejection, and add a case that fails if `with_flash_guard` is removed — or, if the host preempts it in every reachable shape, assert the host error and say so | INV-FLASH-02's "VERIFIED" currently rests on `is_err()` assertions that cannot fail for the stated reason |
| 3 | Strike A048's Gap (2) from the corpus and from anything downstream of it | The report carries a residual that the platform makes unreachable |
| 4 | Keep `poc_host_reentrancy.rs` as the pin for the guarantee everything else leans on | Nothing in the repo records *why* the unguarded payout legs are safe |
| 5 | Add `#[when_not_paused]` consideration for `recapitalize`, or document it as intentionally pause-independent in the pause matrix | It and `renew_account` are the only pause-independent controller mutators, which the matrix does not say |

Items 1-3 in the earlier draft of this file (wrapping the payout legs, promoting
SAC-only listing to P0, re-banding A055) are **withdrawn** — see §4.
