# Liquidity Pool

The pool holds interest accrual, scaled-share accounting, and tracked cash per
`(hub_id, asset)`. The controller owns risk and policy. The pool owns
arithmetic and liquidity.

| | |
| --- | --- |
| Owner | Controller, fixed at construction. No transfer, accept, or renounce |
| Called by | Controller only. Integrators use [`contracts/controller`](../controller/README.md) |
| Client | `LiquidityPoolInterface` in [`interfaces/pool/src/lib.rs`](../../interfaces/pool/src/lib.rs). It drops `Env` |

## Model

Balances are scaled shares, and value = shares × index. Interest therefore
needs no per-user write. The controller holds the per-account ledger. Positions
arrive as `ScaledPositionRaw` arguments and leave as `PoolPositionMutation` returns.

## Authorization

The controller deploys the pool with itself as constructor argument
(`deploy_v2(wasm_hash, (env.current_contract_address(),))`). Migration uses
`upgrade`. The pool does not re-check these upstream guarantees:

| Guarantee | Enforced in |
| --- | --- |
| `asset_decimals` matches listed decimals (oracle `asset_decimals`, else token `decimals()`), in `[0,18]` | `governance/validate/asset.rs::validate_market_creation` |
| Asset contract is live (`try_decimals`, `try_symbol`) | `governance/validate/asset.rs` |
| Rate-model changes pass the timelock | `governance/op.rs` |
| No flash-loan re-entry | `controller/storage/account.rs::with_flash_guard`, checked by `controller/risk/validation.rs::require_not_flash_loaning` |
| `scaled_amount` is a real position | controller position ledger |
| **Tokens arrived before a cash-crediting call** | controller payment path |

The flash guard wraps every external router and receiver call: `flash_loan`,
`flash_position`, `migrate_from_blend`, `multiply`, `swap_debt`,
`swap_collateral`, `repay_debt_with_collateral`.

The last row matters most. `supply`, `repay`, and `recapitalize` credit `cash`
(a book value) on the controller's word. Only `flash_loan` compares it with
`token.balance()`, three times, with strict equality.

## Entrypoints

The [endpoint reference](../../docs/reference/endpoints.md#pool) lists
signatures and view units. The [error reference](../../docs/reference/errors.md)
lists error conditions. Every mutator is `#[only_owner]`. Missing owner auth is
a host auth error, not a contract code.

| Entrypoint | Tokens | Does |
| --- | --- | --- |
| `create_market` | — | Verifies params. Writes state, both indexes `RAY` |
| `update_params` | — | Accrues on the **old** curve, then writes the new one |
| `update_indexes` | — | Accrues each market. Always commits, so the write is in the footprint |
| `supply` | in | Mints supply shares, credits cash |
| `borrow` | out | Mints debt shares, debits cash, transfers |
| `withdraw` | out | Burns supply shares, keeps liquidation fee, transfers net |
| `repay` | in/out | Burns debt shares, credits net, refunds overpayment |
| `net_settle` | — | Offsets one user's supply against own debt. One entry, not a batch |
| `seize_positions` | — | Writes off bad debt, or moves a seized deposit to revenue |
| `flash_loan` | out/in | Pays out, calls `execute_flash_loan`, pulls principal plus fee. Returns fee |
| `create_strategy` | out | Mints debt, books fee as revenue, sends `amount - fee` |
| `recapitalize` | in/out | Credits cash up to backing shortfall, refunds excess |
| `claim_revenue` | out | Burns revenue shares, pays owner |
| `upgrade` | — | Replaces the Wasm |

`in` means the controller transferred first, and the pool only credits `cash`.
`out` means the pool transfers. `in/out` adds an outbound refund. `flash_loan`
sends first and collects after. `InternalError` (34) is a broken invariant. It
fires when `revenue > supplied` after a supply burn or deposit seizure, or when
a revenue claim burns zero shares.

### Views

`get_*` views take a `HubAssetKey`, need no auth, and fail with 30 for an
unknown market. Only `get_bulk_indexes` accrues (simulated). The controller
reads it and `get_sync_data`, never the scalar getters. The scalar getters lag
by `get_delta_time`. For live figures, use `get_sync_data` with
`simulate_update_indexes`. `get_deposit_rate` models only `reserve_factor`, so
it overstates realized supplier yield by the rounding shortfall
([Rounding](#rounding)). **Views are not read-only.** Each view renews the market
TTL, so on-chain polling costs writes.

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

Checks-effects-interactions holds except in `flash_loan`, which pays out
first, then reconciles. `ops::run_batch` loads a fresh `Cache` per entry.
Supply, borrow, withdraw, and repay reject a repeated market (`InvalidPayments`,
16), since each leg carries a position read before the batch. Only
`seize_positions` composes same-market entries. The second reads the first's
committed state, and one `PoolMarketStateBatchEvent` carries both snapshots.
Indexers take the last.

## State

```text
supplied, borrowed, revenue : Ray, scaled shares
borrow_index                : Ray, never decreases
supply_index                : Ray, rises with interest, falls on bad debt
cash                        : i128, asset units, book value
```

- `update_borrow_index` is the only writer of `borrow_index`.
- `apply_bad_debt_to_supply_index` scales `supply_index` down to spread a loss.
  The floor is `SUPPLY_INDEX_FLOOR_RAW` (`RAY/1000`). **Code that caches an
  index must accept a decrease.**
- `cache/shares.rs::require_revenue_backed` checks that `revenue <= supplied`.

Revenue is protocol-owned supply shares. Two paths add to it. Never swap them:

| Path | Effect | Used by |
| --- | --- | --- |
| `accrue_revenue` | `revenue += s` and `supplied += s` (mints) | interest, flash, liquidation, and strategy fees |
| `absorb_supply_as_revenue` | `revenue += s` only (reassigns) | `seize_positions`, deposit side |

A seized deposit is already in `supplied`. Interest creates new claims. The
[formulas](../../docs/reference/formulas.md#backing-and-cash-constraints) define
`guards::backing_shortfall`.

## Rounding

Round against the user. Changing `floor` to `ceil` is never cosmetic.
The [formulas](../../docs/reference/formulas.md#shares-and-token-amounts) list
the directions for each boundary.
The revenue claim burns with `mul_ratio_ceil`. `Bps::flash_loan_fee_on`
charges at least 1 at a positive rate. `supply_index_reward_shortfall` books
the supplier reward lost to floor rounding as revenue.

## Interest

`interest::global_sync` accrues in chunks of at most `MAX_COMPOUND_DELTA_MS`
(one year) and recomputes utilization per chunk
([formulas](../../docs/reference/formulas.md#rates-and-accrual)).
`MAX_BORROW_INDEX_RAY` and `MAX_SUPPLY_INDEX_RAY` are `1e36`. At the market
value ceiling a chunk accrues nothing and emits one `MarketValueCeilingEvent`
([INV-IDX-06](../../docs/reference/invariants.md#inv-idx-06)).
`compound_interest` (`common/src/rates/compound.rs`) is a fixed 9-term Taylor
series with no early exit. At `x = 2` (`2 × RAY` max rate idle for a year), it
gives `7.387302` against `e² = 7.389056`. That result is 0.024% low. Smaller `x`
can round slightly high.

## Guards

`require_reserves` is on `Cache` in `cache/cash.rs`. The rest are in
`guards.rs`. `create_strategy` mints through `borrow::mint_debt`, so it runs every
`borrow` guard.

| Guard | Runs on | Skipped on | Error |
| --- | --- | --- | --- |
| `require_backed_market` | `supply` | everything else | 123 |
| `require_reserves` | `borrow`, `create_strategy`, `withdraw`, `flash_loan`, `claim_revenue` | — | 112 |
| `require_liquidation_buffer` | `borrow`, `create_strategy` | `withdraw`, `flash_loan` | 112 |
| `require_utilization_below_max` | `borrow`, `create_strategy` | zero supply shares, zero debt value | 127 |
| `require_exit_utilization_below_max` | non-liquidation `withdraw`, `claim_revenue` | a 100% ceiling, `net_settle`, `seize_positions`, liquidation | 127 |
| `require_within_entry_ceiling` | `supply`, `borrow`, `create_strategy` (after the mint and again after the fee), `flash_loan` (after the fee) | every exit | 136 |
| `require_supply_for_debt` | `withdraw`, `net_settle`, `claim_revenue` | — | 123 |

The liquidation buffer (`LIQUIDATION_BUFFER_BPS`, [INV-ACCT-07](../../docs/reference/invariants.md#inv-acct-07))
is flat, so a borrow can fail with 112 while cash exceeds the draw. [INV-ACCT-11](../../docs/reference/invariants.md#inv-acct-11) covers the entry
ceiling. A backing shortfall blocks supply, not withdrawal. `recapitalize`
restores the backing.

**Exit is not always open.** The utilization cap is a post-state check, and a
withdrawal raises utilization. Once accrual alone reaches the cap, no
non-liquidation withdrawal passes until repayment or liquidation. This
`1 - max_utilization` headroom is separate from the 2% buffer.

## Per-entrypoint notes

**`update_params`** keeps `asset_id` and `asset_decimals` unchanged after
creation.

**`withdraw`** returns `actual_amount = gross`. The receiver gets
`gross − protocol_fee`, minted back as protocol shares with the cash kept. A
zero-token payout that burns the last shares attempts a zero-value transfer,
which keeps the receiver's account or trustline in the footprint. Other zero
payouts move no tokens.

**`net_settle`** moves no tokens and is not `withdraw` plus `repay`. If `ceil(debt)`
is one unit above `floor(supply)`, supply closes and one debt unit stays. If the
two are equal, both close. No utilization gate is needed, because in exact
arithmetic

```text
(B−x)/(S−x) − B/S  =  x·(B−S) / [S·(S−x)]
```

is negative whenever `B < S`. Rounding can still move one share.

**`create_strategy`** computes the fee before it mints. `amount == 0` with
`charge_fee` and positive `flashloan_fee` fails `StrategyFeeExceeds` (minimum
fee 1). Otherwise, `mint_debt` rejects zero with 14.

**`repay`** and **`recapitalize`** refund from cash without a debit. The refund is correct
only because the controller transferred the full amount first.

**`claim_revenue`** on a fully lent-out market pays less than `get_revenue`, or
zero. The rest stays as revenue shares and keeps earning. Guards run even with
nothing claimable. If they pass, it returns `actual_amount = 0`, moves no
tokens, and emits a snapshot. Always read `actual_amount` from the returned
`PoolAmountMutation`.

## Storage and TTL

| Key | Tier | Holds |
| --- | --- | --- |
| `PoolKey::Params(HubAssetKey)` | persistent | `InterestRateModel`, asset id, decimals |
| `PoolKey::State(HubAssetKey)` | persistent | Supplied, borrowed, revenue, indexes, timestamp, cash |
| OpenZeppelin `Ownable` owner | instance | Controller |

The pool has no per-user storage. Persistent entries renew on every read,
views included (`TTL_THRESHOLD_SHARED` 30 days, `TTL_BUMP_SHARED` 180 days).
The instance renews at the start of each mutator.

## Events

Read [pool events](../../docs/reference/events.md#pool-events) and
[row layouts](../../docs/reference/events.md#market-and-configuration-records)
before writing a decoder.

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

The shared math is in [`common`](../../common): `math/fp.rs` (`Ray`, `Wad`, `Bps`),
`math/fp_core.rs` (`i128` mul-div, `I256` only on overflow), `rates/` (curve,
compound, index, scaling, simulate), and `types/pool.rs` (ABI types, `verify`).

## References

- Formal proofs: [`certora/pool/spec`](../../certora/pool/spec/README.md)
- Tests: `make test-pool`. After a `common/src/math` change, run `make miri-common`.
