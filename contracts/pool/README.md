# Liquidity Pool

The market engine: interest accrual, scaled-share accounting and tracked cash
per `(hub_id, asset)`. The controller owns risk and policy; the pool owns
arithmetic and liquidity.

| | |
| --- | --- |
| Owner | Controller, fixed at construction. No transfer, accept or renounce |
| Called by | Controller only; users never call the pool. Integrators use [`contracts/controller`](../controller/README.md) |
| Client | [`interfaces/pool`](../../interfaces/pool) |

Full signatures are in the `LiquidityPoolInterface` trait,
[`interfaces/pool/src/lib.rs`](../../interfaces/pool/src/lib.rs); the generated
client drops the `Env` argument.

## Model

Each market has two persistent keys. There is no per-user storage.

```text
PoolKey::Params(HubAssetKey)   # InterestRateModel, asset id, decimals
PoolKey::State(HubAssetKey)    # supplied, borrowed, revenue, indexes, timestamp, cash
```

Balances are scaled shares. Value = shares × index, so interest reaches every
holder without a per-user write:

```text
supply value = supplied * supply_index      debt value = borrowed * borrow_index
```

Positions come in as arguments (`ScaledPositionRaw`) and go out as return
values (`PoolPositionMutation`). The controller holds the per-account ledger.

## Trust

The controller deploys the pool with itself as the constructor argument
(`deploy_v2(wasm_hash, (env.current_contract_address(),))`). There is no
ownership transfer, accept or renounce; migration uses `upgrade`.

The pool does not re-check what the controller already guarantees:

| Guarantee | Enforced in |
| --- | --- |
| `asset_decimals` matches the listed decimals (oracle `asset_decimals`, else token `decimals()`), in `[0,18]` | `governance/validate/asset.rs::validate_market_creation` |
| Asset contract is live (`try_decimals`, `try_symbol`) | `governance/validate/asset.rs` |
| Rate-model changes pass the timelock | `governance/op.rs` |
| No flash-loan re-entry | `controller/storage/account.rs::with_flash_guard`, checked by `controller/risk/validation.rs::require_not_flash_loaning` |
| `scaled_amount` is a real position | controller position ledger |
| **Tokens arrived before a cash-crediting call** | controller payment path |

The flash guard wraps every external router and receiver call: `flash_loan`,
`flash_position`, `migrate_from_blend`, `multiply`, `swap_debt`,
`swap_collateral` and `repay_debt_with_collateral`.

The last row carries the most weight. `supply`, `repay` and `recapitalize`
credit `cash` on the controller's word and do not check the transfer. `cash`
is a book value. Only `flash_loan` compares it with a real `token.balance()`,
three times, with strict equality.

## Entrypoints

Every mutator is `#[only_owner]`; a call without owner auth fails with a host auth
error, not a contract code. Error numbers are from `common/src/errors.rs`.

| Entrypoint | Tokens | Does | Errors (besides 30, 33) |
| --- | --- | --- | --- |
| `__constructor(admin)` | — | Sets the Ownable owner, once | — |
| `create_market(hub_id, params)` | — | Verifies params, writes state with both indexes at `RAY` | `AssetAlreadySupported` (2), `MarketParamsRaw::verify` |
| `update_params(hub_asset, model)` | — | Accrues on the **old** curve, then writes the new model | `InterestRateModel::verify` |
| `update_indexes(hub_assets)` | — | Accrues each market; writes only if time elapsed | — |
| `supply(entries)` | in | Mints supply shares, credits cash | 14, `PoolInsolvent` (123), `SupplyRoundsToZeroShares` (51) |
| `borrow(receiver, entries)` | out | Mints debt shares, debits cash, transfers | 14 (zero too), `InsufficientLiquidity` (112), `BorrowRoundsToZeroShares` (47), `PoolInsolvent` (123), `UtilizationAboveMax` (127) |
| `withdraw(receiver, is_liquidation, entries)` | out | Burns supply shares, keeps the liquidation fee, transfers the net | 14, `WithdrawRoundsToZeroShares` (49), `WithdrawLessThanFee` (115), 112, 127 (non-liquidation), 123, `InternalError` (34) |
| `repay(payer, actions)` | in/out | Burns debt shares, credits the net, refunds overpayment | 14, `RepayRoundsToZeroShares` (52) |
| `net_settle(entry)` | — | Offsets one user's supply against their own debt; one entry, not a batch | 14, `NetSettleRoundsToZeroShares` (50), 123, 34 |
| `seize_positions(entries)` | — | Writes off bad debt, or moves a seized deposit to revenue | 14, 34 |
| `flash_loan(hub_asset, initiator, receiver, amount, data)` | out/in | Pays out, calls `execute_flash_loan`, pulls back principal + fee; returns the fee | 14, `FlashloanNotEnabled` (401), 112, `InvalidFlashloanReceiver` (412), `InvalidFlashloanRepay` (402) |
| `create_strategy(receiver, action, charge_fee)` | out | Mints debt, books the fee as revenue, sends `amount - fee` | 14, `StrategyFeeExceeds` (409), every `borrow` error |
| `recapitalize(hub_asset, payer, amount)` | in/out | Credits cash up to the backing shortfall, refunds the excess | 14 |
| `claim_revenue(hub_asset)` | out | Burns revenue shares, pays the owner | 127, 123, `OwnerNotSet` (32), 34 |
| `upgrade(new_wasm_hash)` | — | Replaces the Wasm | — |

Every market entrypoint also fails with `PoolNotInitialized` (30) for an
unknown market and `MathOverflow` (33) on checked-arithmetic overflow.
`AmountMustBePositive` (14) rejects a negative amount in `supply`, `withdraw`,
`create_strategy` and `recapitalize`; `borrow` also rejects zero. `InternalError` (34) means a broken invariant:
`revenue > supplied` after a supply burn or deposit seizure, or a revenue claim
that burns zero shares.

**Tokens:** `in` means the controller transferred before the call and the pool
only credits `cash`. `out` means the pool transfers. `in/out` adds an outbound
refund. `flash_loan` sends first and collects after.

### Views

All `get_*` views take a `HubAssetKey`, need no auth, and fail with 30 for an
unknown market.

| View | Returns | Accrued to now |
| --- | --- | --- |
| `get_bulk_indexes(hub_assets)` | Indexes for many markets (used by the controller) | yes, simulated |
| `get_sync_data` | Params + state for one market (used by the controller) | no; caller simulates |
| `get_utilisation` | Utilization, raw `RAY` | no |
| `get_reserves` | Tracked `cash`, asset units | no |
| `get_deposit_rate`, `get_borrow_rate` | Annual rate, `RAY` | no |
| `get_revenue` | Revenue, asset units, floored | no |
| `get_supplied_amount`, `get_borrowed_amount` | Totals, asset units | no |
| `get_delta_time` | Milliseconds since the last accrual | — |

The scalar getters lag by `get_delta_time`, and no controller code reads them.
For live figures use `get_sync_data` with `simulate_update_indexes`.

`get_deposit_rate` models only `reserve_factor`, so it overstates realized
supplier yield by the rounding shortfall (see [Rounding](#rounding)).

**Views are not read-only.** Each renews the market TTL, so on-chain polling
costs writes.

## Flow

Each mutation of an existing market runs:

```text
entrypoint (#[only_owner])
  → Cache::load             # read params + state, bump TTL
  → interest::global_sync   # accrue to now, in chunks of at most 1 year
  → mutate                  # cache/shares.rs, cache/cash.rs
  → guards::*               # reserve, utilization, backing
  → commit → transfer_out → emit
```

Checks-effects-interactions holds everywhere except `flash_loan`, which pays
out first and then reconciles balances.

`ops::run_batch` loads a fresh `Cache` per entry, so two entries on the same
market compose: the second reads the first's committed state. A market touched
twice emits two snapshots in one `PoolMarketStateBatchEvent`; indexers take the
last. An empty batch emits nothing.

## State

```text
supplied, borrowed, revenue : Ray, scaled shares
borrow_index                : Ray, never decreases
supply_index                : Ray, rises with interest, falls on bad debt
cash                        : i128, asset units, book value
```

- `update_borrow_index` is the only writer of `borrow_index`.
- `apply_bad_debt_to_supply_index` scales `supply_index` down to spread a loss
  over suppliers, with a floor of `SUPPLY_INDEX_FLOOR_RAW` (`RAY/1000`).
  **Code that caches an index must accept a decrease.**
- `revenue <= supplied`, checked by `cache/shares.rs::require_revenue_backed`.

Revenue is supply shares the protocol owns, not a separate pot. Two paths add
to it, and they must not be swapped:

| Path | Effect | Used by |
| --- | --- | --- |
| `accrue_revenue` | `revenue += s` and `supplied += s` (mints) | interest, flash, liquidation and strategy fees |
| `absorb_supply_as_revenue` | `revenue += s` only (reassigns) | `seize_positions`, deposit side |

A seized deposit is already in `supplied`; interest creates new claims.

Backing shortfall (`guards::backing_shortfall`):

```text
supplied_claim(floor) − (cash + outstanding_debt(ceil)), clamped ≥ 0
```

## Rounding

Round against the user. A `floor` to `ceil` change is never cosmetic.

| Operation | Direction | Effect |
| --- | --- | --- |
| supply mint | `div_floor` | fewer shares to the depositor |
| borrow mint | `div_ceil` | more debt shares |
| withdraw burn (partial) | `div_ceil` | more shares burned |
| repay burn (partial) | `div_floor` | less debt forgiven |
| supply readout | `to_asset_floor` | less claimed |
| debt readout | `to_asset_ceil` | more owed |
| revenue claim | `mul_ratio_ceil` | more treasury shares burned |

- The `*RoundsToZeroShares` errors reject amounts that move value but no
  shares.
- `Bps::flash_loan_fee_on` charges at least 1 when the fee rate is positive.
- Supplier reward lost to floor rounding is measured by
  `supply_index_reward_shortfall` and booked as revenue, so no accrued value
  is lost.

## Interest

`interest::global_sync` accrues from `last_timestamp` to now in chunks of at
most `MAX_COMPOUND_DELTA_MS` (one year). It recomputes utilization per chunk,
so a stale market follows the rate change instead of keeping one rate for the
whole gap.

```text
utilization → borrow rate (curve) → compound → borrow index
            → supplier reward / protocol fee (reserve_factor split)
            → supply index → shortfall → revenue
```

`MAX_BORROW_INDEX_RAY` and `MAX_SUPPLY_INDEX_RAY` are `1e36`.

`compound_interest` (`common/src/rates/compound.rs`) is a fixed 9-term Taylor
series, `1 + x + … + x⁸/8!`, with no early exit. At `x = 2` (a `2 × RAY`
max rate untouched for a full year) it gives `7.387302` against
`e² = 7.389056`, 0.024% low. At smaller `x`, rounding can give a tiny
overestimate.

## Guards

`guards.rs` holds four guards; `require_reserves` is on `Cache` in
`cache/cash.rs`. `create_strategy` mints through `borrow::mint_debt`, so it
runs every `borrow` guard.

| Guard | Runs on | Skipped on | Error |
| --- | --- | --- | --- |
| `require_backed_market` | `supply` | everything else | 123 |
| `require_reserves` | `borrow`, `create_strategy`, `withdraw`, `flash_loan`, `claim_revenue` | — | 112 |
| `require_liquidation_buffer` | `borrow`, `create_strategy` | `withdraw`, `flash_loan` | 112 |
| `require_utilization_below_max` | `borrow`, `create_strategy`, non-liquidation `withdraw`, `claim_revenue` | `net_settle`, `seize_positions`, liquidation | 127 |
| `require_supply_for_debt` | `borrow`, `create_strategy`, `withdraw`, `net_settle`, `claim_revenue` | — | 123 |

**Liquidation buffer.** `require_liquidation_buffer` keeps a flat
`LIQUIDATION_BUFFER_BPS` (200 bps) of the floored supplied amount for
seizures: it requires `cash - draw >= reserved`. No market parameter changes
it. So a borrow can fail with 112 while the pool holds more cash than the
borrow asks for.

**Deliberate asymmetries.**

1. A backing shortfall blocks supply, not withdrawal. `recapitalize` restores
   backing.
2. Liquidation withdrawals skip the utilization cap, so liquidations work at
   the ceiling.

**Exit is not always open.** The utilization cap is a post-state check, and a
withdrawal raises utilization. Once accrual alone pushes utilization to the
cap, no non-liquidation withdrawal passes. Repayment or liquidation frees it.
This `1 - max_utilization` headroom is separate from the 2% buffer.

`max_utilization >= RAY` turns the cap off for that market.

## Per-entrypoint notes

**`update_params`** commits accrual on the old curve before the swap, so a new
rate never applies to the past. `asset_id` and `asset_decimals` never change
after creation.

**`withdraw`** returns `actual_amount = gross`; the receiver gets
`gross − protocol_fee`. The fee is minted back as protocol shares and the cash
stays. `actual_amount` is not the amount received.

**`net_settle`** moves no tokens and is not withdraw + repay. It settles
`min(requested, floor(supply), ceil(debt))`. When `ceil(debt)` is one unit
above `floor(supply)`, supply closes and one unit of debt stays; when they are
equal, both close. It needs no utilization gate: in exact arithmetic

```text
(B−x)/(S−x) − B/S  =  x·(B−S) / [S·(S−x)]
```

is negative whenever `B < S`. Rounding can still move one share at the token
boundary.

**`create_strategy`** computes the fee before it mints debt. `amount == 0` with
`charge_fee` and a positive `flashloan_fee` fails with `StrategyFeeExceeds`
(the minimum fee is 1); otherwise `mint_debt` rejects zero with 14.

**`repay`** and **`recapitalize`** refund from pool cash without a debit. This
is correct only because the controller transferred the full amount first.

**`claim_revenue`** pays `min(cash, floor(revenue_value))`, so a fully lent-out
market pays less than `get_revenue` shows, or zero. The unclaimed rest stays as
revenue shares and keeps earning. The guards run even when nothing is
claimable; if they pass, the call returns `actual_amount = 0`, moves no tokens
and emits a snapshot. Always read `actual_amount` from the returned
`PoolAmountMutation`.

## Layout

```text
lib.rs        # ABI and owner gates; mutators delegate to ops/
ops/          # one module per mutator; market.rs has create, params, accrual
cache/        # Cache: load a market, mutate by named transition, commit
  scale.rs    #   share ⇄ asset conversion
  shares.rs   #   mint/burn supply and debt, revenue
  cash.rs     #   credit/debit, require_reserves, transfer_out
  report.rs   #   index setters, snapshot, controller-facing mutations
interest.rs   # accrual (via common::rates::accrue_step), revenue, bad debt
guards.rs     # utilization, backing, solvency
storage.rs    # the only place PoolKey is built, read, written, renewed
views.rs      # reads behind the view ABI
events.rs     # batched market-state and params events
time.rs       # ledger clock in milliseconds
```

Shared math is in [`common`](../../common): `math/fp.rs` (`Ray`, `Wad`, `Bps`),
`math/fp_core.rs` (`i128` mul-div, `I256` only on overflow), `rates/` (curve,
compound, index, scaling, simulate) and `types/pool.rs` (ABI types, `verify`).

**TTL.** Persistent entries renew on every read; the instance renews at the
start of each mutator. `TTL_THRESHOLD_SHARED` is 30 days, `TTL_BUMP_SHARED`
180 days.

**Events.** `PoolMarketStateBatchEvent` on state change,
`PoolMarketParamsBatchEvent` on create and model change, `StrategyFeeEvent`
only for a non-zero strategy fee. Read "Pool events" in
[`docs/reference/events.md`](../../docs/reference/events.md) before you write a
decoder: batch events carry single-value data, `PoolMarketStateEvent` rows are
9-entry vectors in fixed order, and `PoolMarketParamsEvent` rows are maps.

## Verification

```bash
make test-pool        # pool unit tests
make test             # whole workspace, harness included
make miri-common      # fp_core Miri tests, for common/src/math changes
make clippy
```

## References

- Errors: [`docs/reference/errors.md`](../../docs/reference/errors.md)
- Events: [`docs/reference/events.md`](../../docs/reference/events.md)
- Formulas: [`docs/reference/formulas.md`](../../docs/reference/formulas.md)
- Formal proofs: [`certora/pool/spec`](../../certora/pool/spec/README.md)
