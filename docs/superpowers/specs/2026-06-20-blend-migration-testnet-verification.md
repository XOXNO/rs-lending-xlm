# Blend V2 → Controller migration — live testnet verification

**Date:** 2026-06-20
**Network:** Stellar testnet (`Test SDF Network ; September 2015`)
**Outcome:** ✅ End-to-end migration proven on-chain. The auth model flagged as the
#1 live-verification risk in the design held under a real wallet signature.

## What was tested

`migrate_from_blend` — the one-click, zero-fee strategy that atomically sweeps a
Blend V2 position into our controller. This run exercised a **collateral-only XLM
migration**: open a real Blend collateral position with the deployer wallet, then
migrate it into a fresh controller account in a single transaction.

The collateral leg is the novel, risky core of the flow: the controller emits a
nested Blend `submit(from=user, spender=controller, to=controller, …)`, the user's
wallet signs the `from` leg through the transaction auth tree, and the controller
authorizes its own spender legs in-contract via `authorize_as_current_contract`.
This had only ever been validated against the `mock_blend.rs` test harness; this is
the first time it ran against the real Blend V2 pool with a real signature.

## Environment / addresses

| Role | Address |
|---|---|
| Deployer / migrating user (EOA) | `GDBBOILYIJBSUQKC3Z3USAW3DGPFHIGVKYA5T4ZUZBO56HBUPHJEN3FV` |
| Our controller | `CCCLDW7WWVPQBYKFWUJTLLAAACBD6MZGAMNOHQ6BVXLXD6VRAGKKDFOT` |
| Our pool | `CA5ZVWRH2KMQ3HAOW32GBFWGMZL2XPGIJ66EC33PYPMBB6IDQHKFGHDZ` |
| Blend V2 pool (`TestnetV2`) | `CCEBVDYM32YNYCVNRXQKDFFPISJJCV557CDZEIRBEE4NCV4KHPQ44HGF` |
| XLM SAC (shared asset) | `CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC` |

`stellar-cli 27.0.0`. Deployer XLM balance at start: ~13,574 XLM.

## Pre-flight findings

1. **Controller is migration-capable and the pool is already whitelisted.**
   `is_blend_pool_approved(TestnetV2)` returned `true`. This single read confirmed
   both that the deployed controller carries the migration wasm (the view exists)
   and that the Blend pool is on the governance allow-list — so no `whitelistBlendPools`
   timelock cycle was needed.

2. **Asset-overlap constraint (the key scoping fact).** Migration can only move an
   asset that is a market in *our* controller (the withdrawn asset is deposited via
   `supply::process_deposit`, and debt is cleared by a zero-fee borrow of that same
   asset from our pool). Comparing both sides:

   - Blend `TestnetV2` reserves: `XLM` (`CDLZFC3SY…`), `CAZAQB3D…`, `CAP5AMC2…`, `CAQCFVLO…` (its USDC/wETH/wBTC)
   - Our markets: `USDC` (`CBIELTK6…`), `XLM` (`CDLZFC3SY…`), `EURC` (`CCUUDM43…`), `BTC` (`CBK3FNAM…`), `ETH` (`CBFNIHC2…`)

   **Only XLM (`CDLZFC3SY…`) is shared.** Blend's USDC is a different token contract
   than ours. Therefore a debt-reconcile leg is not constructible on this pool — a
   debt migration would need a *second* overlapping asset to borrow against. The
   collateral migration is the meaningful, provable end-to-end test here.

3. Deployer started with no Blend position: `{collateral:{},liabilities:{},supply:{}}`.

## Step 1 — Open a Blend collateral position (500 XLM)

```bash
stellar contract invoke --id CCEBVDYM32YNYCVNRXQKDFFPISJJCV557CDZEIRBEE4NCV4KHPQ44HGF \
  --source deployer --network testnet -- submit \
  --from   GDBBOILYIJBSUQKC3Z3USAW3DGPFHIGVKYA5T4ZUZBO56HBUPHJEN3FV \
  --spender GDBBOILYIJBSUQKC3Z3USAW3DGPFHIGVKYA5T4ZUZBO56HBUPHJEN3FV \
  --to     GDBBOILYIJBSUQKC3Z3USAW3DGPFHIGVKYA5T4ZUZBO56HBUPHJEN3FV \
  --requests '[{"address":"CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC","amount":"5000000000","request_type":2}]'
```

`request_type: 2` = `SupplyCollateral`. Amount `5000000000` stroops = 500 XLM (7 decimals).

- **Tx:** `cbc867b50cecdb96cb86f1c0d9cf236a5447ba8add45859dc2b80c3119180997`
- **Result:** `{"collateral":{"0":"3542563891"},"liabilities":{},"supply":{}}`
- 500 XLM underlying minted **3,542,563,891 bToken shares** (reserve index 0 = XLM;
  bToken rate ≈ 1.4116, i.e. the reserve had accrued interest). Blend stores
  positions as *shares*, keyed by reserve index, not underlying.

## Step 2 — Migrate into the controller (one transaction)

Pre-state snapshot: deployer native XLM `13074.68`, controller XLM SAC balance `0`.

```bash
stellar contract invoke --id CCCLDW7WWVPQBYKFWUJTLLAAACBD6MZGAMNOHQ6BVXLXD6VRAGKKDFOT \
  --source deployer --network testnet -- migrate_from_blend \
  --caller GDBBOILYIJBSUQKC3Z3USAW3DGPFHIGVKYA5T4ZUZBO56HBUPHJEN3FV \
  --account_id 0 \
  --e_mode_category 0 \
  --blend_pool CCEBVDYM32YNYCVNRXQKDFFPISJJCV557CDZEIRBEE4NCV4KHPQ44HGF \
  --collateral_assets '["CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC"]' \
  --supply_assets '[]' \
  --debt_caps '[]'
```

`account_id 0` = create a fresh account. `debt_caps []` = no debt leg.

- **Tx:** `e47c381f346b38fc8384bef64587cdbaa2f8d87a6a7fb304a032be08dc4a174c`
- **Return value:** `1` (the new account id)

### Event trace (single atomic tx)

1. Blend `withdraw_collateral` XLM → `5000010460` underlying, burning all
   `3542563891` shares (the `i128::MAX` withdraw-all clamp). The `+10460` stroops
   over the supplied 500 XLM is interest accrued between Step 1 and Step 2.
2. XLM `transfer` Blend → controller: `5000010460`.
3. XLM `transfer` controller → pool: `5000010460` (the controller passes it straight
   through into our pool via `process_deposit`).
4. Pool `market batch_state_update` for XLM (total supplied bumped by the migrated amount).
5. `UpdatePositionBatchEvent` account_id `1`, deposit XLM underlying `5000010460`,
   risk params `[7000, 700, 6500]` (LTV 70% / reserve factor / liq threshold 65%).
6. `BlendMigrationEvent` account_id `1`, `collateral_count: 1`, `supply_count: 0`,
   `debt_count: 0`.

## Step 3 — Verification (post-state)

| Check | Command (abridged) | Result | Verdict |
|---|---|---|---|
| Blend position emptied | `get_positions --address <deployer>` | `{collateral:{},liabilities:{},supply:{}}` | ✅ |
| Controller acct #1 XLM collateral | `collateral_amount_for_token --account_id 1 --asset XLM` | `5000010460` | ✅ exact match |
| Health factor | `health_factor --account_id 1` | `170141183460469231731687303715884105727` (`i128::MAX`) | ✅ debt-free |
| Total collateral USD (WAD) | `total_collateral_in_usd --account_id 1` | `106616307319238310392` (≈ $106.62) | ✅ 500 XLM × ~$0.213 |
| Controller XLM SAC balance | `balance --id <controller>` | `0` | ✅ passed through |
| Pool XLM SAC balance | `balance --id <pool>` | `5000010460` | ✅ backs the new collateral |

**Conservation:** `5,000,010,460` stroops withdrawn from Blend ==
deposited into our pool == credited to account #1's collateral. No principal lost,
**zero flash-loan fee**, single atomic transaction.

## What this proves and what it does not

**Proven on real testnet infrastructure:**
- The nested-`submit` auth model works end-to-end with a real wallet. The CLI
  simulation discovered the full auth tree (deployer signs the top-level call and the
  nested `submit(from=deployer)`; the controller's spender legs are contract-authorized
  via `authorize_as_current_contract`) and the transaction executed atomically. This
  was the design's stated #1 risk.
- `i128::MAX` withdraw-all semantics against a Blend reserve that had accrued interest
  (share→underlying conversion correct).
- Collateral hand-off: Blend → controller → our pool → credited as the user's
  collateral, with the end-state health gate (`strategy_finalize`) passing for a
  debt-free position.
- Fresh-account creation path (`account_id = 0` → account `1`).

**Not exercised here (and why):**
- The **debt-reconcile leg** (zero-fee borrow → `Repay(cap)` → refund reconcile).
  This requires a Blend debt asset that is *also* one of our markets; on `TestnetV2`,
  XLM is the only overlapping asset, so no second asset exists to borrow against.
  Covered by the `mock_blend.rs` harness tests (`test_migrate_debt_and_collateral`,
  `test_migrate_debt_cap_too_low_reverts`) but pending a real-pool run.
- The **non-collateral supply leg** (`supply_assets`, `Withdraw=1`). Same single-asset
  constraint; harness-covered (`test_migrate_supply_only`).

To exercise the debt path on real infrastructure, either (a) deploy/whitelist a Blend
pool whose reserves include a second token that is also one of our markets, or (b) add
a market to our controller matching a second `TestnetV2` reserve. Both are out of scope
for this verification run.

## Explorer links

- Blend supply: https://stellar.expert/explorer/testnet/tx/cbc867b50cecdb96cb86f1c0d9cf236a5447ba8add45859dc2b80c3119180997
- Migration: https://stellar.expert/explorer/testnet/tx/e47c381f346b38fc8384bef64587cdbaa2f8d87a6a7fb304a032be08dc4a174c

---

## Addendum — same-asset (looped) debt migration (2026-06-20)

After implementing same-asset looping support (`docs/.../2026-06-20-blend-migration-same-asset-looping-design.md`),
the **debt path was verified live for the first time**. The controller was upgraded
in-place to the looping-capable wasm via governance (timelocked `upgrade`, which
pauses the controller — followed by `unpause`).

**Setup:** the deployer opened a real XLM loop on Blend — one `submit` with
`SupplyCollateral(XLM, 500)` + `Borrow(XLM, 200)` (tx
`886d266e813229df32fb13cd2b9b8993d138ba0a2206050b3d72f561c011b91c`), yielding 500
XLM collateral + 200 XLM debt in the same reserve.

**Auth bug found and fixed.** The first migration attempt failed at simulation with
`Error(Auth, InvalidAction)`: the repay pull `transfer(controller → blend_pool, cap)`
was authorized as a sub-invocation **nested under** a `submit` auth entry. But Blend's
`submit` `spender.require_auth()` (spender = controller) is satisfied *implicitly*
because the controller is `submit`'s direct invoker, so the submit frame collapses out
of the controller's auth tree and the host expects the deeper transfer at the **top
level**. Fix (commit `74a7e2d`): emit the repay pulls as top-level
`authorize_as_current_contract` entries (matching `swap::pre_authorize_router_pull`).
This path is invisible to the harness (`mock_all_auths` bypasses real auth), so the bug
was latent until this live debt-path run.

**Migration (tx `8b9dd2198230d11ffd284c4cd4fbab9b732337fd266bed6ec6d3e1b7a43a46a0`):**
`migrate_from_blend(collateral_assets=[XLM], debt_caps=[(XLM, 250)])` into a fresh
account. Trace: phase-1 borrow 250 XLM → Blend repay (cleared 200.008, refunded 49.99)
→ reconcile to 200 debt; phase-2 withdraw 500.016 XLM → deposit as collateral. Returned
account id `3`.

**Verification (post-state):**

| Check | Result |
|---|---|
| Blend position | `{collateral:{},liabilities:{},supply:{}}` — cleared ✅ |
| Account #3 XLM collateral | `5000158201` (~500.02 XLM) ✅ |
| Account #3 XLM debt | `2000084822` (~200.01 XLM) — reconciled, not the 250 cap ✅ |
| Health factor | `1749981152471011145` (~1.75) ✅ |
| can_be_liquidated | `false` ✅ |

The same token (XLM) is both collateral and debt on account #3 — the looped position,
preserved 1:1. The two-phase submit and the dual nested-`submit` auth (the design's #1
risk) are proven on real infrastructure.
