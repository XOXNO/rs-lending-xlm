# Pool bulk-first endpoints — redesign plan

Goal: the pool's position verbs accept arrays and loop internally, so the
controller pays ONE cross-contract frame per verb per transaction instead of
one per asset. Measured motivation (10×10 liquidation, tx 3f4b6cc6…): 20 of
the 62 invocations are sequential `repay`/`withdraw` calls into the same pool
contract — the largest controller-controlled frame block (~60–110M of 372M
instructions, ~10–18MB of 29.3MB memory). Returns stay `PoolPositionMutation`,
one per entry, **in input order**. Breaking ABI is fine: pre-mainnet, and the
pool ABI is internal (controller-owned); no off-chain consumer touches it.

## Type changes (`common/src/types/pool.rs`)

```rust
/// Per-asset mutation payload. The funds counterparty is NOT here — it is
/// hoisted to the endpoint (identical for every entry of a call).
pub struct PoolAction {
    pub asset: Address,
    pub position: ScaledPositionRaw,
    pub amount: i128,
}

pub struct PoolSupplyEntry  { pub action: PoolAction, pub supply_cap: i128 }
pub struct PoolBorrowEntry  { pub action: PoolAction, pub borrow_cap: i128 }
pub struct PoolWithdrawEntry { pub action: PoolAction, pub protocol_fee: i128 }
```

`caller` is removed from `PoolAction`. Each endpoint names its counterparty
by its actual role:

## New pool ABI (`interfaces/pool/src/lib.rs`)

```rust
/// No counterparty: the controller pre-transfers tokens into the pool, and
/// the old `action.caller` was carried but never read on this path.
fn supply(env: Env, entries: Vec<PoolSupplyEntry>) -> Vec<PoolPositionMutation>;

/// Tokens sent TO `receiver` (`cache.transfer_out`).
fn borrow(env: Env, receiver: Address, entries: Vec<PoolBorrowEntry>) -> Vec<PoolPositionMutation>;

/// `is_liquidation` hoisted: one flag per call, per-asset protocol_fee stays
/// per entry (fee scales with each asset's seized value). Net transfer goes
/// to `receiver`.
fn withdraw(env: Env, receiver: Address, is_liquidation: bool,
            entries: Vec<PoolWithdrawEntry>) -> Vec<PoolPositionMutation>;

/// Tokens already transferred to the pool by the controller. `payer` is the
/// OVERPAYMENT REFUND destination (`transfer_out(&caller, overpayment)` in
/// today's body) — the one repay-side use of the old caller field.
fn repay(env: Env, payer: Address, actions: Vec<PoolAction>) -> Vec<PoolPositionMutation>;
```

Unchanged: `create_strategy` (single-asset by nature), `seize_position`,
`flash_loan`, `update_indexes`, `claim_revenue`, `get_sync_data`,
`bulk_get_sync_data`, admin fns. (`seize_position` bulk for clean_bad_debt is
a possible follow-up, not in scope.)

Semantics:
- All-or-nothing: any entry panicking reverts the whole call (same atomicity
  as today's sequential calls inside one tx).
- Empty vec → empty vec (controller never sends one; no panic site added).
- Duplicate assets allowed: entries process sequentially, the second sees the
  first's post-state — identical to two calls today. The controller dedupes
  plans anyway (`aggregate_positive_payments`).
- `#[only_owner]` checked ONCE per call (today: once per asset) — small bonus.
- `renew_pool_instance` hoisted out of the loop.

## Pool implementation (`contracts/pool/src/lib.rs`)

Each verb body factors into an internal `fn supply_one(env, payer, entry) ->
PoolPositionMutation` (the existing body, `caller` replaced by the hoisted
address); the endpoint is `entries.iter().map(supply_one).collect()`. Per-entry
`load_synced_cache` stays (different markets, each accrues independently).

## Controller changes

- `cross_contract/pool.rs`: wrappers take the hoisted address + entry vecs.
- `positions/supply.rs` / `borrow.rs` / `withdraw.rs` / `repay.rs`: the
  per-asset loops invert — build the entry vec first, ONE pool call, then
  iterate returned mutations (input-ordered) doing exactly today's per-asset
  bookkeeping: `record_market_update`, position upsert, event delta record,
  isolated-debt adjustment. Event ORDER is preserved because results are
  input-ordered — the indexer sees identical event content.
- `positions/liquidation.rs`: repay leg = one bulk `repay`; seizure leg = one
  bulk `withdraw(receiver=liquidator, is_liquidation=true)`. 20 frames → 2.
- Strategies (`multiply`, `swap_*`, `rp_col_*`, close): bulk-of-one through
  the same endpoints — no special-casing, identical frame cost to today.
- Token transfers: unchanged ownership — controller still pre-transfers repay
  funds; pool still does outbound/inbound SAC transfers inside its (now
  single) frame. SAC frame count is unchanged (one per distinct token —
  the standard's floor).

## What does NOT change

- `PoolPositionMutation` shape, indexes math, caps enforcement, dust gates,
  HF checks, events (shape AND order) — the indexer/sdk/types/api/ui need
  **zero changes**.
- Keeper: `update_indexes`/`get_sync_data` untouched.

## Verification

1. Pool unit tests: mechanical update of every client call to vec form
   (helper `t.action(...)` grows `t.entry(...)` builders); add bulk-specific
   tests: input-order of returns, multi-asset supply/repay equivalence vs
   sequential singles, duplicate-asset sequencing, empty-vec, per-entry cap
   enforcement (entry 2 violating cap reverts entry 1's effect).
2. Controller: full harness suite (events tests already pin order/shape);
   fuzz reference + `pool_native` target updated.
3. Certora: pool-side mirror re-sync (types changed); controller summaries
   for pool calls re-typed (controller certora is pre-broken — keep pool
   green, note controller mirror in the known-broken pile).
4. e2e: in-place upgrade path — pause controller → upgrade pool wasm →
   upgrade controller wasm → unpause (the admin flow's documented sequence);
   alternatively fresh deploy + fleet repoint. Then stress + `liq_20feed`
   width walk.

## Measured result on the 10×10 liquidation (post-implementation)

62 → 44 invocations. MEASURED on-chain (core_metrics, identical scenario,
controller CA5O…DPZV, tx 0d313caa… vs pre-bulk 3f4b6cc6…):
CPU 372.29M → 366.61M (−1.5%); memory 29.33MB → 25.75MB (−12%); declared
budget headroom at width 10: 1.6M → 7.0M instructions.

The original estimate (~60–110M CPU) was wrong: a same-contract
re-invocation costs ~0.3M instructions + ~200KB memory (the marginal-call
slope), NOT the ~1.28MB first-instantiation cost — the host reuses the
parsed module within a transaction. The refactor's durable value is the
memory headroom (61% of cap at width 10), one auth check per verb, smaller
call envelopes, and the bulk-first ABI itself; the CPU wall is governed by
intra-contract logic and the 20 per-asset Reflector reads (a DIFFERENT
contract, so those frames do pay instantiation) — that investigation is the
next lever.
