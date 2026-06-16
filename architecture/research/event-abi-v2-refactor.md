# Event ABI v2 — full breaking refactor plan

Goal: redesign the hot-path contract events for minimum on-chain size, rewrite
the decode pipeline against the new ABI, and reconstruct **byte-identical DB
documents** to today's (account-profile, market-profile, activity rows,
debt-ceiling patches). Breaking changes are free: no mainnet deployment
exists, and the indexer is routinely repointed at fresh testnet controllers
with a reset start ledger, so no dual-decode or migration path is required.

## Why (measured baseline, testnet run 20260611-bulksync)

- Network caps (identical on mainnet): 16,384 B contract events per tx,
  400M instructions per tx, 40 MiB memory.
- A liquidation seizing 10 collaterals emits ~11.1 KB of events before the
  first repay leg; each additional repaid debt adds exactly 1,060 B
  (measured ladder: width 5 = 16,416 B — over by 32 B; width 9 = 20,656 B).
  Single-tx max today: **4 debt repays + 10-collateral seize**.
- 57% of a position delta and 49% of a market snapshot entry is repeated
  ScMap field-name symbols — pure encoding overhead with one consumer
  (our own decode chain in `@xoxno/sdk-js`).

## Wire design

Encoding primitive: `#[contracttype]` on **unnamed-field (tuple) structs**
serializes as `ScVec` (soroban-sdk-macros `derive_type_struct_tuple`),
eliminating all field-name keys. Field order is the ABI; documented in
`common/src/events.rs` and mirrored in the sdk-js decoder tables.

ScVal costs: i128 = 20 B, u64 = 12, u32 = 8, bool = 8, Address = 40,
Symbol(n) = 8 + pad4(n), Vec/Map header = 12.

### `["position", "batch_update"]` — new payload

```rust
// Side is implied by which vec an entry sits in — position_type is GONE.
// Risk params exist only on the deposit side — borrow entries don't pay
// for three zeroed u32s. action becomes a #[repr(u32)] enum.
pub struct PositionBatchV2(
    pub u64,                    // account_id
    pub AccountAttributesV2,    // tuple: owner, e_mode, isolated, mode, isolated_token
    pub Vec<DepositDeltaV2>,
    pub Vec<BorrowDeltaV2>,
);
pub struct DepositDeltaV2(
    pub u32,      // PositionAction discriminant
    pub Address,  // asset
    pub i128,     // scaled_amount_ray (post-mutation)
    pub i128,     // index_ray
    pub i128,     // amount (tx delta, asset-native)
    pub u32,      // liquidation_threshold_bps (entry, e-mode adjusted)
    pub u32,      // liquidation_bonus_bps
    pub u32,      // loan_to_value_bps
);
pub struct BorrowDeltaV2(pub u32, pub Address, pub i128, pub i128, pub i128);
```

Dropped fields and why it's safe:
- `asset_price_wad` — never persisted from position deltas; the activity
  builder's per-tx price map falls back to the market snapshot of the same
  asset in the same tx, which the controller backfills from the same
  `prices_cache` (identical value, guaranteed present for mutated assets).
- `position_type` — implied by the deposits/borrows split.
- `action: Symbol` → `u32` enum — sdk-js decoder maps discriminant back to
  today's action strings (`supply`, `liq_repay`, …) so the activity mapper
  and `mapStellarPositionActivityType` are unchanged.

Sizes: deposit entry 144 B (was 424), borrow entry 120 B (was 424).

### `["market", "batch_state_update"]` — tuple entries, same fields

All nine fields are consumed (market-profile doc + activity price map):
keep every field, change only the encoding.

```rust
// Event-only struct. MarketStateSnapshot stays as-is: it is also the
// return type of update_indexes/flash_loan/add_rewards, which the keeper
// and controller consume — function ABIs do not change.
pub struct MarketStateEntryV2(
    pub Address,        // asset
    pub u64,            // timestamp (ms accrual checkpoint)
    pub i128,           // supply_index_ray
    pub i128,           // borrow_index_ray
    pub i128,           // reserves_ray (live cash, legacy suffix)
    pub i128,           // supplied_ray
    pub i128,           // borrowed_ray
    pub i128,           // revenue_ray
    pub Option<i128>,   // asset_price_wad
);
```

Size: 204 B per entry (was 392).

### `["debt", "ceiling_batch_update"]`

`DebtCeilingEntryV2(pub Address, pub i128)` — 72 B per entry.

### Out of scope

Admin/config events (`market:create`, `config:*`, `market:params_update`,
e-mode, oracle) stay map-encoded: size-irrelevant (admin paths), and
self-describing payloads are worth keeping for explorers. SAC `transfer`
events are the token standard — untouchable, 244 B per leg forever.

## Projected budget (10-collateral seize + 10-debt repay liquidation)

| component | old | new |
|---|---|---|
| 20 × SAC transfer | 4,880 | 4,880 |
| position event (attrs + 10 dep + 10 bor) | ~8,700 | ~2,870 |
| market event (20 entries) | ~7,950 | ~4,180 |
| envelopes/topics | ~500 | ~200 |
| **total** | **~22,000 (rejected)** | **~12.1 KB ✓ (26% headroom)** |

Events stop binding entirely. The 400M CPU cap becomes the wall: measured
400.5M at width 10 / 391.7M at width 9 on the OLD encoding; v2 sheds some
serialization cost, putting width 10 on the boundary and width 9 safely in.

## Per-repo changes (dependency order)

### 1. rs-lending-xlm

- `common/src/events.rs`: add the v2 tuple structs above; delete
  `EventPositionDelta`, `UpdatePositionBatchEvent`'s old payload,
  `MarketStateSnapshot`-in-event usage, old debt-ceiling batch payload.
  Add `PositionAction` `#[repr(u32)]` enum (one discriminant per current
  action symbol — authoritative table in the docstring).
- `contracts/controller/src/cache/mod.rs`: `record_position_update` /
  `record_debt_position_update` take the enum; buffer deposit and borrow
  deltas in separate vecs; `emit_position_batch` / `emit_market_batch`
  build the v2 payloads; price backfill applies only to market entries.
- Call sites: replace action `Symbol` literals with enum variants
  (grep `record_position_update|record_debt_position_update`).
- Tests: `tests/test-harness/tests/controller/events.rs` asserts
  event shapes — rewrite against v2; add a size-regression test asserting
  a 10+10 liquidation's serialized events stay under ~13 KB.
- Certora: events are not modeled in specs; verify the harness mirrors
  compile (known-broken emode issue is pre-existing and separate).

### 2. @xoxno/types (origin/main)

- `src/requests/lending/stellar-lending-events.dto.ts`: decoded shapes are
  wire-agnostic and mostly survive. Deltas: remove `assetPriceWad` from
  `StellarEventPositionDelta`; keep `positionType` and `action: string`
  (sdk-js reconstructs both — downstream consumers unchanged). Publish.

### 3. @xoxno/sdk-js (branch `alpha` — the active branch)

- `src/sdk/stellar/events/decode.ts`: for the three hot topics, replace the
  keyed-`Raw` access with positional reads (`scValToNative` yields arrays
  for tuple structs). Reconstruct today's decoded objects exactly:
  `action` string from the enum table, `positionType` from which vec the
  entry came from, risk params only on deposit entries (borrow side stays
  `undefined`, as today). Hard cutover — no Map/Vec sniffing needed since
  the indexer repoints to a fresh controller.
- `__tests__/decode.test.ts`: regenerate fixtures from test-harness XDR
  dumps (add a small dump helper to the harness if needed); add a parity
  test pinning decoded-output equality for equivalent old/new payloads.
- Publish; bump `@xoxno/types`.

### 4. xoxno-az-functions (origin/main)

- Bump `@xoxno/sdk-js` + `@xoxno/types`.
- `stellar-position-update.event.ts`: decoded shape is unchanged except
  `assetPriceWad` is now always `undefined` → field already optional;
  delete the dead plumbing. Activity builder unchanged (price-map fallback
  covers position rows). All doc builders (`toDoc`s) untouched —
  **DB documents are byte-identical by construction**.
- Repoint indexer at the freshly deployed controller + reset start ledger
  (established pattern, see commits `45e46fc`, `0f8d118`).

## DB-doc parity matrix (proof obligation)

| doc field | v1 source | v2 source |
|---|---|---|
| account: supply/borrowAmountScaled, supply/borrowIndex | delta.scaled/index + position_type | same values; side from vec membership |
| account: entry{Threshold,Bonus,Ltv} | delta risk params (deposit side) | DepositDeltaV2 fields 5–7 |
| account: entryLiquidationFee | always `undefined` (no on-chain source) | unchanged |
| account: address/isolated/eMode/mode/isolatedToken | account_attributes | AccountAttributesV2 |
| market: all fields incl. usdPrice | snapshot 9 fields | MarketStateEntryV2 9 fields |
| activity: activityType | action symbol | enum → same strings in decoder |
| activity: transactionAmount | delta.amount | field 4 |
| activity: usdPrice | delta.asset_price_wad ∥ price map | price map (identical value) |
| debt-ceiling patch | entry.asset/total | DebtCeilingEntryV2 |

## Verification

1. Contract: full per-crate test bar + harness suite (events tests rewritten),
   plus the events-size regression test.
2. sdk-js: decode fixtures + parity snapshot tests.
3. az-functions: existing processor tests against new sdk version; one
   end-to-end fixture (raw XDR → docs) compared field-by-field with a
   pre-refactor golden file.
4. Testnet e2e: redeploy, run `flows/stress.sh` + the `liq_20feed*` scenarios;
   expected: 9-repay + 10-seize lands (CPU-bound), 10-repay borderline;
   bulk-supply/borrow frontiers unchanged.
5. Point a staging indexer at the new controller; diff Cosmos docs for a
   scripted day of activity against the golden expectations.

## Risks / notes

- Field order IS the ABI: any reorder is silent corruption. Mitigate with
  the docstring table + sdk-js fixtures generated from contract XDR, not
  hand-written.
- `Option<i128>` in a tuple struct must encode as Void/i128 — covered by the
  generated fixtures.
- Simulation enforces neither the events cap nor the instruction cap:
  liquidation bots keep a client-side width cap (9 after this refactor).
- If 10-repay single-tx ever becomes a hard requirement, the remaining
  lever is CPU (~0.1%), not events.
