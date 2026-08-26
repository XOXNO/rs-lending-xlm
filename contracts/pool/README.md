# Liquidity Pool

Market engine for the lending protocol: interest accrual, scaled-share
accounting, and tracked cash per `(hub_id, asset)`. The controller owns risk and
policy; this contract owns arithmetic and liquidity.

Fourteen entrypoints are `#[only_owner]`: `create_market`, `update_params`,
`upgrade`, `supply`, `borrow`, `withdraw`, `repay`, `update_indexes`,
`recapitalize`, `flash_loan`, `create_strategy`, `seize_positions`,
`net_settle` and `claim_revenue`. The owner is the controller, so users never
call one directly. Everything else is a public view: the ten `get_*` functions
carry no auth check and anyone may call them. Integrators go through
[`contracts/controller`](../controller); this file is for auditors, for anyone
reading the accounting, and for anyone reading the ABI.

## Model

Two persistent keys per market, and **no per-user storage anywhere**:

```text
PoolKey::Params(HubAssetKey)   # rate curve, asset id, decimals
PoolKey::State(HubAssetKey)    # supplied, borrowed, revenue, indexes, ts, cash
```

Balances are **scaled shares**, not amounts. A share is multiplied by a market
index to get present value, so interest accrues to every holder at once
without touching per-user state:

```text
supply value = supplied * supply_index      debt value = borrowed * borrow_index
```

Positions arrive as arguments (`ScaledPositionRaw`) and leave as return values
(`PoolPositionMutation`). The controller holds the ledger; the pool holds the
aggregates.

## Trust

The controller deploys this contract with itself as constructor argument
(`deploy_v2(wasm_hash, (env.current_contract_address(),))`), so the owner is
fixed at deploy. There is no transfer, accept, or renounce on the ABI —
migration goes through `upgrade`.

Because the controller is the sole caller, the pool does not re-validate what is
already guaranteed upstream:

| Guarantee | Enforced in |
| --- | --- |
| `asset_decimals` matches the token's real `decimals()`, in `[3,18]` | `governance/validate/asset.rs::validate_market_creation` |
| Asset contract is live (`try_decimals` + `try_symbol`) | `governance/validate/asset.rs` |
| Rate-model params are timelocked before reaching the pool | `governance/op.rs` |
| Flash-loan reentrancy | `controller/storage/account.rs::with_flash_guard`, checked by `controller/risk/validation.rs::require_not_flash_loaning` |
| `scaled_amount` maps to a real position | controller position ledger |
| Tokens arrived before any cash-crediting call | controller payment path |

The flash guard wraps every external-router and external-receiver call, so it
covers seven controller entrypoints: `flash_loan`, `flash_position`,
`migrate_from_blend`, and the swap-routed `multiply`, `swap_debt`,
`swap_collateral` and
`repay_debt_with_collateral`. The last row is the load-bearing one: `supply`,
`repay` and `recapitalize` all credit `cash` on the controller's word, without
verifying the transfer. `cash` is a bookkeeping number.
The only reconciliation against a real
`token.balance()` is in `flash_loan`, which checks it three times with strict
equality.

## Surface

| Entrypoint | Role | Tokens |
| --- | --- | --- |
| `create_market` | Verify params, write state, indexes at `RAY` | — |
| `update_params` | Accrue on the **old** curve, then replace the rate model | — |
| `update_indexes` | Accrue each market in the vec; commit only if time elapsed | — |
| `supply` | Mint supply shares, credit cash | in |
| `borrow` | Mint debt shares, debit cash, transfer | out |
| `withdraw` | Burn supply shares, withhold liquidation fee, transfer net | out |
| `repay` | Burn debt shares, credit net, refund overpayment | in/out |
| `net_settle` | Offset a user's own supply against their own debt | — |
| `seize_positions` | Bad-debt write-down, or deposit → revenue | — |
| `claim_revenue` | Burn revenue shares, transfer to owner | out |
| `recapitalize` | Credit cash up to the backing shortfall, refund excess | in/out |
| `flash_loan` | Payout → callback → collect principal + fee | out/in |
| `create_strategy` | Borrow for a strategy, net of fee | out |
| `upgrade` | Replace contract Wasm | — |

Tokens column: `in` means the controller transferred to the pool before the
call and the pool only credits `cash`; `out` means the pool transfers. `in/out`
is an inbound amount with an outbound refund leg — `repay` returns
overpayment, `recapitalize` returns whatever exceeded the shortfall.
`flash_loan` is `out/in`: principal leaves, then principal plus fee returns.

## Signatures

Copied from the `LiquidityPoolInterface` trait in
[`interfaces/pool/src/lib.rs`](../../interfaces/pool/src/lib.rs), plus
`__constructor` from `contracts/pool/src/lib.rs`. The trait's `env: Env` is not
part of the wire ABI — a `LiquidityPoolClient` call passes the remaining
arguments only. "Owner" means the address set at construction, normally the
controller.

| Entrypoint | Signature | Who may call | What it does |
| --- | --- | --- | --- |
| `__constructor` | `fn __constructor(env: Env, admin: Address)` | deployer, once | Sets `admin` as the Ownable owner. |
| `create_market` | `fn create_market(env: Env, hub_id: u32, params: MarketParamsRaw)` | owner | Creates the market for `(hub_id, params.asset_id)` with both indexes at `RAY`. |
| `update_params` | `fn update_params(env: Env, hub_asset: HubAssetKey, model: InterestRateModel)` | owner | Accrues on the old curve, then writes the new rate model. |
| `update_indexes` | `fn update_indexes(env: Env, hub_assets: Vec<HubAssetKey>)` | owner | Accrues each market to now, and writes only if time elapsed. |
| `supply` | `fn supply(env: Env, entries: Vec<PoolSupplyEntry>) -> Vec<PoolPositionMutation>` | owner | Mints supply shares and credits cash, one mutation returned per entry. |
| `borrow` | `fn borrow(env: Env, receiver: Address, entries: Vec<PoolBorrowEntry>) -> Vec<PoolPositionMutation>` | owner | Mints debt shares, debits cash, and transfers the asset to `receiver`. |
| `withdraw` | `fn withdraw(env: Env, receiver: Address, is_liquidation: bool, entries: Vec<PoolWithdrawEntry>) -> Vec<PoolPositionMutation>` | owner | Burns supply shares and transfers the net amount to `receiver`. |
| `repay` | `fn repay(env: Env, payer: Address, actions: Vec<PoolAction>) -> Vec<PoolPositionMutation>` | owner | Burns debt shares, credits the net repay, and refunds overpayment to `payer`. |
| `net_settle` | `fn net_settle(env: Env, entry: PoolNetSettleEntry) -> PoolNetSettleResult` | owner | Offsets one user's supply against their own debt. Takes one entry, not a batch. |
| `seize_positions` | `fn seize_positions(env: Env, entries: Vec<PoolSeizeEntry>)` | owner | Writes off bad debt on the borrow side, or books a seized deposit as revenue. Returns nothing. |
| `flash_loan` | `fn flash_loan(env: Env, hub_asset: HubAssetKey, initiator: Address, receiver: Address, amount: i128, data: Bytes) -> i128` | owner | Pays out, calls `execute_flash_loan` on `receiver`, pulls principal plus fee back. Returns the fee. |
| `create_strategy` | `fn create_strategy(env: Env, receiver: Address, action: PoolAction, charge_fee: bool) -> PoolStrategyMutation` | owner | Mints debt, books the optional fee as revenue, and sends `amount - fee` to `receiver`. |
| `recapitalize` | `fn recapitalize(env: Env, hub_asset: HubAssetKey, payer: Address, amount: i128) -> PoolAmountMutation` | owner | Credits cash up to the backing shortfall and refunds the excess to `payer`. |
| `claim_revenue` | `fn claim_revenue(env: Env, hub_asset: HubAssetKey) -> PoolAmountMutation` | owner | Burns revenue shares and transfers the proceeds to the owner. |
| `upgrade` | `fn upgrade(env: Env, new_wasm_hash: BytesN<32>)` | owner | Replaces the contract Wasm. |
| `get_utilisation` | `fn get_utilisation(env: Env, hub_asset: HubAssetKey) -> i128` | anyone | Utilization at the last accrual, as raw `RAY`. |
| `get_reserves` | `fn get_reserves(env: Env, hub_asset: HubAssetKey) -> i128` | anyone | Tracked `cash`, in asset units. |
| `get_deposit_rate` | `fn get_deposit_rate(env: Env, hub_asset: HubAssetKey) -> i128` | anyone | Supplier rate as annual `RAY`. |
| `get_borrow_rate` | `fn get_borrow_rate(env: Env, hub_asset: HubAssetKey) -> i128` | anyone | Borrow rate as annual `RAY`. |
| `get_revenue` | `fn get_revenue(env: Env, hub_asset: HubAssetKey) -> i128` | anyone | Protocol revenue in asset units, floored. |
| `get_supplied_amount` | `fn get_supplied_amount(env: Env, hub_asset: HubAssetKey) -> i128` | anyone | Total supplied underlying in asset units. |
| `get_borrowed_amount` | `fn get_borrowed_amount(env: Env, hub_asset: HubAssetKey) -> i128` | anyone | Total borrowed underlying in asset units. |
| `get_delta_time` | `fn get_delta_time(env: Env, hub_asset: HubAssetKey) -> u64` | anyone | Milliseconds since the market's last accrual. |
| `get_sync_data` | `fn get_sync_data(env: Env, hub_asset: HubAssetKey) -> PoolSyncData` | anyone | Full params plus state for one market. |
| `get_bulk_indexes` | `fn get_bulk_indexes(env: Env, hub_assets: Vec<HubAssetKey>) -> Vec<MarketIndexRaw>` | anyone | Forward-simulated indexes for many markets. Note the plural argument. |

The owner is fixed at construction. There is no `transfer_ownership` and no
`accept_ownership` on this contract, so the only way to change the code behind
the address is `upgrade`.

## Errors

Every `#[only_owner]` entrypoint first panics with the Ownable unauthorized
error if the caller is not the owner. Beyond that, each entrypoint panics with
the codes below. Numbers are the raw contract error codes from
`common/src/errors.rs`.

| Entrypoint | Errors |
| --- | --- |
| `create_market` | `AssetAlreadySupported` (2) for a duplicate `(hub_id, asset)`; whatever `MarketParamsRaw::verify` rejects |
| `update_params` | `PoolNotInitialized` (30); whatever `InterestRateModel::verify` rejects |
| `update_indexes`, every `get_*` view | `PoolNotInitialized` (30) |
| `supply` | `AmountMustBePositive` (14) on a negative amount, `PoolInsolvent` (123) when the market is under-backed, `SupplyRoundsToZeroShares` (51) |
| `borrow` | `AmountMustBePositive` (14) — zero is rejected here, `InsufficientLiquidity` (112) from cash or the liquidation buffer, `BorrowRoundsToZeroShares` (47), `UtilizationAboveMax` (127) |
| `withdraw` | `AmountMustBePositive` (14) on a negative amount or fee, `WithdrawRoundsToZeroShares` (49), `WithdrawLessThanFee` (115), `InsufficientLiquidity` (112), `UtilizationAboveMax` (127) on non-liquidation calls, `PoolInsolvent` (123) |
| `repay` | `AmountMustBePositive` (14), `RepayRoundsToZeroShares` (52), `MathOverflow` (33) |
| `net_settle` | `AmountMustBePositive` (14), `NetSettleRoundsToZeroShares` (50), `PoolInsolvent` (123) |
| `seize_positions` | `AmountMustBePositive` (14) |
| `flash_loan` | `AmountMustBePositive` (14), `FlashloanNotEnabled` (401), `InsufficientLiquidity` (112), `InvalidFlashloanReceiver` (412) for a non-Wasm receiver, `InvalidFlashloanRepay` (402) for a short allowance or a balance mismatch |
| `create_strategy` | `AmountMustBePositive` (14) on a negative amount, `StrategyFeeExceeds` (409), plus the whole `borrow` set — it mints debt through the same path |
| `recapitalize` | `AmountMustBePositive` (14) on a negative amount, `MathOverflow` (33) |
| `claim_revenue` | `UtilizationAboveMax` (127), `PoolInsolvent` (123), `OwnerNotSet` (32), `InternalError` (34) |
| `upgrade` | none beyond the owner check |

Every market entrypoint also panics with `PoolNotInitialized` (30) when the
market does not exist, and with `MathOverflow` (33) on a checked-arithmetic
overflow.

## Flow

Every operation is the same five beats:

```text
entrypoint (#[only_owner])
  → Cache::load             # read params + state, bump TTL
  → interest::global_sync   # accrue to now, in ≤1yr chunks
  → mutate                  # cache/shares.rs, cache/cash.rs
  → guards::*               # post-state checks
  → commit → transfer_out → emit
```

Checks-effects-interactions holds everywhere except `flash_loan`, which inverts
by nature and compensates with balance reconciliation.

`ops::run_batch` gives each entry its own `Cache::load`, so two entries hitting
the same market in one batch compose correctly — the second reads the first's
committed state. Indexers: a market touched twice emits two snapshots in one
`PoolMarketStateBatchEvent`; take the last. An empty batch emits nothing.

## State

```text
supplied, borrowed, revenue : Ray, scaled shares
borrow_index                : Ray, monotone non-decreasing
supply_index                : Ray, grows on interest, falls on bad debt
cash                        : i128, token-native, bookkeeping
```

`borrow_index` only ever grows — `update_borrow_index` is its sole writer.
`supply_index` is **not** monotone: `apply_bad_debt_to_supply_index` scales it
down to socialize a loss across suppliers, floored at `SUPPLY_INDEX_FLOOR_RAW`
(`RAY/1000`). Anything caching an index must tolerate a decrease.

**`revenue <= supplied`** — asserted in
`cache/shares.rs::require_revenue_backed`.

Protocol revenue is not a side pot; it is supply shares the protocol owns. Two
paths exist and the distinction is load-bearing:

| Path | Effect | Used by |
| --- | --- | --- |
| `accrue_revenue` | `revenue += s` **and** `supplied += s` — **mints** shares | interest, flash, liquidation and strategy fees |
| `absorb_supply_as_revenue` | `revenue += s` only — **reassigns** existing shares | `seize_positions`, deposit side |

Seizing a deposit moves ownership of shares already counted in `supplied`, so
`supplied` must not change. Minting for interest creates new claims, so it must.
Swapping these corrupts the accounting silently.

Backing health is `guards::backing_shortfall`:

```text
supplied_claim(floor) − (cash + outstanding_debt(ceil)), clamped ≥ 0
```

## Rounding

Rounding direction is a security property: **round against the user, in
favor of the protocol**. Changing a `floor` to a `ceil` is never cosmetic.

| Operation | Direction | Effect |
| --- | --- | --- |
| supply mint | `div_floor` | fewer shares to depositor |
| borrow mint | `div_ceil` | more debt shares to borrower |
| withdraw burn (partial) | `div_ceil` | more shares burned |
| repay burn (partial) | `div_floor` | fewer debt shares forgiven |
| supply readout | `to_asset_floor` | less claimed |
| debt readout | `to_asset_ceil` | more owed |
| revenue claim | `mul_ratio_ceil` | burns more treasury shares than proportional |

Dust defences: the `*RoundsToZeroShares` errors reject amounts that move value
without moving shares, and `Bps::flash_loan_fee_on` floors a fee at `1`.

Rewards that floor rounding keeps out of the supply index are measured by
`supply_index_reward_shortfall` and booked as protocol revenue, so no accrued
value is destroyed. `calculate_deposit_rate` models only `reserve_factor`, so
`get_deposit_rate` overstates realized supplier yield by that rounding
shortfall.

## Interest

`interest::global_sync` accrues from `last_timestamp` to now in chunks of at
most `MAX_COMPOUND_DELTA_MS` (one year), recomputing utilization **per chunk**
so a stale market tracks rate drift instead of freezing one rate across the gap.

```text
util → borrow rate (curve) → e^x (compound) → borrow index
     → supplier rewards / protocol fee (reserve_factor split)
     → supply index → shortfall → revenue
```

Bounds: `MAX_BORROW_INDEX_RAY` and `MAX_SUPPLY_INDEX_RAY` at `1e36`;
`SUPPLY_INDEX_FLOOR_RAW` at `RAY/1000` floors bad-debt write-down.

**`compound_interest` is a fixed 8th-order Taylor series** (`1 + x + x²/2! + … +
x⁸/8!`, nine terms) in `common/src/rates/compound.rs`. The divisors are a constant list;
there is no early-exit. At `x = 2` (`max_borrow_rate` at its `2 × RAY` cap, a
full untouched year) the series gives `7.387302` against `e² = 7.389056` — a
**0.024% under-estimate**, never an over-estimate. Reaching it means suppressing
all activity on a 200%-APR market for a year.

## Guards

Four guards live in `guards.rs`; `require_reserves` is a `Cache` method in
`cache/cash.rs`. `create_strategy` mints debt through `borrow::mint_debt`, so it
inherits every guard that `borrow` runs.

| Guard | Fires on | Not on | Error |
| --- | --- | --- | --- |
| `require_backed_market` | `supply` | everything else | `PoolInsolvent` (123) |
| `require_reserves` | `borrow`, `create_strategy`, `withdraw`, `flash_loan` | — | `InsufficientLiquidity` (112) |
| `require_liquidation_buffer` | `borrow`, `create_strategy` | `withdraw`, `flash_loan` | `InsufficientLiquidity` (112) |
| `require_utilization_below_max` | `borrow`, `create_strategy`, `withdraw` (non-liq), `claim_revenue` | `net_settle`, `seize`, liquidation | `UtilizationAboveMax` (127) |
| `require_solvent_withdraw_state` | `withdraw`, `net_settle`, `claim_revenue` | — | `PoolInsolvent` (123) |

`require_liquidation_buffer` reserves a flat `LIQUIDATION_BUFFER_BPS` of the
floored supplied amount, 200 bps (2%), from
`common/src/constants/pool.rs`. It requires `cash - draw >= reserved`. The rate
is flat: it does not read `max_utilization` and no market parameter changes it.
Integrators should expect this: a borrow can fail with `InsufficientLiquidity`
even though the pool holds more cash than the borrow asks for, because the last
2% of supply is held back for seizures. `require_reserves` runs first and checks
only `cash >= draw`, so both checks return the same error code.

Two asymmetries are policy, not oversight:

1. **Backing shortfall blocks entry, not exit.** `require_backed_market` gates
   only `supply`, so you cannot supply into an under-backed market but you can
   withdraw from one. `recapitalize` is the way back.
2. **Liquidation withdraws skip the utilization cap.** Liquidations must proceed
   at the ceiling.

Exit is *not* unconditionally available. `require_utilization_below_max` still
applies to non-liquidation withdrawals, and it is an absolute post-state test —
burning supply shares raises utilization, so once utilization reaches the cap no
withdrawal of any size passes, and interest accrual alone pushes it there. That
cap leftover (`1 - max_utilization`) is not the 2% cash buffer above. It is
released by repayment, or consumed by liquidation, which bypasses the guard.

`require_utilization_below_max` early-returns when `max_utilization >= RAY`, so
setting it to exactly `RAY` disables the cap for that market.

## Notes

**`update_params`** accrues and commits on the old curve before swapping, so the
new rate is never applied retroactively. `asset_id` and `asset_decimals` are
immutable after creation.

**`withdraw`** returns `actual_amount = gross`, while the receiver gets
`gross − protocol_fee`. The fee is re-minted as protocol shares and the cash
stays. Do not read `actual_amount` as tokens received.

**`net_settle`** moves no tokens. It does not compose withdraw+repay. The
settle size is the conservative overlap
`min(requested, floor(supply), ceil(debt))`. A side is fully closed only when
that overlap exhausts that side — a half-up display that is one native unit
above the floor cannot wipe supply and leave a stroop of debt, and matching
conservative values close both books.

In exact arithmetic it cannot raise utilization in a healthy market:

```text
(B−x)/(S−x) − B/S  =  x·(B−S) / [S·(S−x)]
```

The denominator is positive, so the sign follows `(B−S)`. With `B < S`
(utilization below 1) settling strictly lowers it; it rises only when `B > S`,
a market already past every cap. Directed rounding can still move one share
at the token boundary. Hence no utilization gate here.

**`create_strategy`** computes its fee before minting debt. `amount == 0` with
`charge_fee` and a positive `flashloan_fee` fails `StrategyFeeExceeds` (min fee
is 1). Otherwise `mint_debt` rejects zero with `AmountMustBePositive`.

**`repay`** and **`recapitalize`** refund from pool cash without debiting it,
correct only because the controller transferred the full amount in first.

**`claim_revenue`** is capped by cash. `Cache::burn_claimable_revenue` claims
`min(cash, floor(revenue_value))`, so a fully lent-out market pays out less than
`get_revenue` reports, and pays out zero when `cash` is zero. A partial claim
burns revenue shares with `mul_ratio_ceil`, which burns slightly more shares
than the proportional amount. Nothing is lost: the unclaimed remainder stays as
revenue shares and keeps earning. When nothing is claimable the call still
succeeds, returns `actual_amount = 0`, moves no tokens, and still emits a market
state snapshot. Always read `actual_amount` from the returned
`PoolAmountMutation`; never assume the full revenue was paid.

## Views

| View | Consumer | Interest-synced |
| --- | --- | --- |
| `get_bulk_indexes` | controller (`context/market_index.rs`) | **yes**, forward-simulated |
| `get_sync_data` | controller (`context/pool.rs`) | raw; caller simulates |

`get_bulk_indexes` exists so the controller can get forward-accurate indexes
for an unsynced market without paying for a state write. That is its whole
purpose.

The scalar getters — `get_utilisation`, `get_reserves`, `get_deposit_rate`,
`get_borrow_rate`, `get_revenue`, `get_supplied_amount`, `get_borrowed_amount`,
`get_delta_time` — are consumed by no controller code. They return checkpoint
values as of `last_timestamp` and lag by the accrual gap. For live figures use
`get_sync_data` plus `simulate_update_indexes`.

`get_borrow_rate` and `get_deposit_rate` return **annual** RAY (the capped
curve APR). Divide by `RAY` for a unit fraction. Accrual still compounds the
per-millisecond form internally.

**Views are not read-only** — each renews market TTL, so on-chain polling incurs
write cost.

## Layout

```text
lib.rs        # ABI; every entrypoint delegates to ops/
ops/          # one module per entrypoint, end to end
cache/        # Cache: load a market, mutate by named transition, commit
  scale.rs    #   share ⇄ asset conversion
  shares.rs   #   mint/burn supply and debt, revenue mechanics
  cash.rs     #   credit/debit, require_reserves, transfer_out
  report.rs   #   index setters, snapshot, controller-facing mutations
interest.rs   # lands accrual (via common::rates::accrue_step), revenue, bad debt
guards.rs     # utilization, backing, solvency
storage.rs    # the only place PoolKey is built, read, written, renewed
views.rs      # checkpoint reads behind the view ABI
events.rs     # batched market-state and params events
time.rs       # ledger clock in milliseconds
```

Shared math lives in [`common`](../../common): `math/fp.rs` (`Ray`, `Wad`,
`Bps`), `math/fp_core.rs` (`i128` mul-div, widening to `I256` only on overflow),
`rates/` (curve, compound,
index, scaling, simulate), `types/pool.rs` (ABI types and `verify`).

Persistent entries renew on every read; the instance renews at the top of each
mutator. `TTL_THRESHOLD_SHARED` is 5 days, `TTL_BUMP_SHARED` 180 days.

Events are batched: `PoolMarketStateBatchEvent` on state change,
`PoolMarketParamsBatchEvent` on create and rate-model replace, and
`StrategyFeeEvent` only when a strategy fee is non-zero. Topics, payload shapes
and field order are in
[`docs/reference/events.md`](../../docs/reference/events.md) under "Pool
events". Read it before writing a decoder: the two batch events use
single-value data, `PoolMarketStateEvent` rows are 9-entry vectors in a fixed
order, and `PoolMarketParamsEvent` rows are maps.

## Verification

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

make test-pool        # cargo test -p pool, the pool unit tests
make test             # cargo test -p test-harness, the full harness
make miri-common      # for changes under common/src/math or common/src/rates
```

`make test` runs in parallel. `TEST_THREADS` is empty by default, so libtest
uses one thread per core. Only `make test-verbose` pins `TEST_THREADS := 1`.
Pass `TEST_THREADS=1 make test` to serialize by hand.

Formal verification runs profiles, not features. Do not confuse the two.

A **profile** is a named list of `.conf` files in
[`certora/profiles.json`](../../certora/profiles.json). It is what you pass to
`make certora`. There are seven:

```bash
make certora-wasm                          # build the Certora Wasm first
make certora CERTORA_PROFILE=sanity        # the default when unset
make certora CERTORA_PROFILE=core
make certora CERTORA_PROFILE=fast
make certora CERTORA_PROFILE=heavy
make certora CERTORA_PROFILE=flash-position
make certora CERTORA_PROFILE=manual
make certora CERTORA_PROFILE=all
make certora-list                          # print the confs in each profile
```

`make certora` needs `CERTORAKEY` in the environment and `certoraSorobanProver`
on `PATH`.

A **feature** is a Cargo feature in `contracts/pool/Cargo.toml`. `certora-wasm`
selects one to compile a rule set into the verification Wasm; you never pass one
to `CERTORA_PROFILE`. The pool declares eleven:

```text
certora                                  # base: pulls in the cvlr crates
certora-focused                          # narrows common/ to the focused build
certora-position-accounting-rules
certora-seize-settle-accounting-rules
certora-fee-strategy-accounting-rules
certora-flash-loan-accounting-rules
certora-core-sanity-rules
certora-guard-rules
certora-isomorphism-rules
certora-lifecycle-rules
certora-state-invariant-rules
```

Each of the nine rule-set features implies `certora` and `certora-focused`.
