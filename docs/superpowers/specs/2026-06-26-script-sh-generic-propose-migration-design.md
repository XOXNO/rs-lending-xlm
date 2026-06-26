# `configs/script.sh` — migrate the proposer layer to generic `propose(AdminOperation)`

**Date:** 2026-06-26
**Scope:** `configs/script.sh` (deploy/governance tooling) only. No contract, interface, or SDK changes.
**Type:** Repair of a pre-existing breakage. The op-enum migration removed the typed
`propose_*` governance entrypoints; `script.sh` was never updated and is dead for every
admin op on `main`.

## Problem

`script.sh` schedules admin ops through two helpers that call typed proposer entrypoints:

- `schedule_via_proposer`  → `stellar contract invoke --id $gov -- "$propose_fn" --proposer P "$@" --salt …`
- `schedule_via_gov_self_proposer` → same shape for governance-self ops.

`$propose_fn` is `propose_set_aggregator`, `propose_add_asset_to_e_mode`, … — **24+ typed
proposers that no longer exist.** Governance now exposes only the generic
`propose(proposer, op: AdminOperation, salt) -> BytesN<32>`. So every `schedule_*` call in
`script.sh` fails at invoke time. `propose_*` symbols appear **only** in `script.sh`.

## Target

Both schedule helpers call the generic proposer:

```
stellar contract invoke --id $gov -- propose --proposer P --op "$ADMIN_OP_JSON" --salt $salt
```

Execution is unchanged in mechanism but split by target (already the case today):

- **Controller ops** replay through `execute(executor, target, function, args, predecessor, salt)`.
  The stored `args_json` must equal `resolve_op`'s scheduled args (see per-op table).
- **Governance-self ops** replay through `execute_self(executor, op: AdminOperation, salt)` —
  re-passing the same `AdminOperation` JSON (governance re-resolves inline; Soroban self-reentry
  rule).

## AdminOperation JSON encoding

`AdminOperation` is a `#[contracttype]` enum. The sdk-js encoder is the verified reference:
`adminOp(variant, …payload) = scvVec([scvSymbol(variant), …payload])`. The stellar-cli
**friendly-JSON** form for `--op` is:

- Unit variant:            `"DeployPool"`
- Single payload:          `{"SetAggregator": "<C-address>"}`
- Single struct payload:   `{"AddAssetToEModeCategory": { …EModeAssetArgs… }}`
- Multi-field via struct:   every multi-arg op already wraps its fields in an args struct
  (`EModeAssetArgs`, `PoolCapsArgs`, `RoleArgs`, `CreatePoolArgs`, `ConfigureOracleArgs`,
  `EditToleranceArgs`, `TransferOwnershipArgs`, `UpgradePoolParamsArgs`), so payloads are at
  most one value.

**Open item to confirm on first testnet call:** the exact friendly-JSON shape stellar-cli
accepts for a single-tuple-field enum variant (object-keyed `{"Variant": v}` vs `["Variant", v]`).
Resolve by `stellar contract invoke --build-only` + `stellar tx decode` on one op before
converting all 30 (mirrors the TL-5b friendly-vs-ScVal gotcha).

## Per-op mapping (from `op.rs::resolve_op`)

| AdminOperation variant | `--op` payload | target | scheduled fn (replay) | replay args_json |
|---|---|---|---|---|
| `DeployPool` / `AddEModeCategory` | unit | controller | `deploy_pool` / `add_e_mode_category` | `[]` |
| `SetAggregator`/`SetAccumulator`/`ApproveToken`/`RevokeToken`/`ApproveBlendPool`/`RevokeBlendPool`/`DisableTokenOracle` | `<address>` | controller | matching setter | `[{address}]` |
| `SetLiquidityPoolTemplate`/`UpgradePool`/`UpgradeController` | `<wasm-hash>` | controller | `set_liquidity_pool_template`/`upgrade_pool`/`upgrade` | `[{bytes}]` |
| `SetMinBorrowCollateralUsd` | `<i128>` | controller | `set_min_borrow_collateral_usd` | `[{i128}]` |
| `RemoveEModeCategory`/`MigrateController` | `<u32>` | controller | `remove_e_mode_category`/`migrate` | `[{u32}]` |
| `SetPositionLimits(PositionLimits)` | struct | controller | `set_position_limits` | `[<PositionLimits scval>]` |
| `EditAssetConfig(addr, AssetConfigRaw)` | `[addr, cfg]` | controller | `edit_asset_config` | `[{address}, <cfg scval>]` |
| `AddAssetToEModeCategory`/`EditAssetInEModeCategory(EModeAssetArgs)` | struct | controller | matching fn | `[<EModeAssetArgs scval>]` |
| `UpdatePoolCaps(PoolCapsArgs)` | struct | controller | `update_pool_caps` | `[{address},{i128},{i128}]` (asset, supply, borrow) |
| `RemoveAssetFromEMode(RemoveAssetFromEModeArgs)` | struct | controller | `remove_asset_from_e_mode` | `[{address},{u32}]` |
| `CreateLiquidityPool(CreatePoolArgs)` | struct | controller | `create_liquidity_pool` | `[{address}, <params scval>, <config scval>]` |
| `UpgradeLiquidityPoolParams(UpgradePoolParamsArgs)` | struct | controller | `upgrade_liquidity_pool_params` | `[{address}, <IRM scval>]` |
| **`ConfigureMarketOracle(ConfigureOracleArgs)`** | struct | controller | `set_market_oracle_config` | `[{address}, <RESOLVED MarketOracleConfig>]` — **resolved via `resolve_market_oracle_config` view** |
| **`EditOracleTolerance(EditToleranceArgs)`** | struct | controller | `set_oracle_tolerance` | `[{address}, <RESOLVED OraclePriceFluctuation>]` — **resolved via `resolve_oracle_tolerance` view** |
| `UpgradeGov(hash)` | `<wasm-hash>` | **gov-self** | `execute_self` (re-pass op) | n/a |
| `UpdateGovDelay(u32)` | `<u32>` | gov-self | `execute_self` | n/a |
| `GrantGovRole`/`RevokeGovRole(RoleArgs)` | struct | gov-self | `execute_self` | n/a |
| `TransferGovOwnership`/`TransferCtrlOwnership(TransferOwnershipArgs)` | struct | gov-self / controller | `execute_self` / `transfer_ownership` | n/a / `[{address},{u32}]` |

(The `scval_*` helpers already exist for `AssetConfigRaw`, `MarketParamsRaw`/IRM,
`PositionLimits`, oracle config — reuse them for both the `--op` struct payload and the replay
`args_json`.)

## The two oracle ops (the only non-passthrough)

Governance does live oracle probing at propose time and bakes the **resolved** config into the
scheduled args. So for `ConfigureMarketOracle` / `EditOracleTolerance` the replay `args_json`
must hold the resolved value, fetched from the governance views:

```
resolved=$(stellar contract invoke --id $gov -- resolve_market_oracle_config --asset A --cfg '<input>')
resolve d=$(stellar contract invoke --id $gov -- resolve_oracle_tolerance --first_tolerance F --last_tolerance L)
```

These views are read-only and already exist (TL-5b). Bake their output into `args_json`.

## Helper rewrite

1. `schedule_via_proposer(controller_fn, admin_op_json, args_json, cli_executable, salt, …)`:
   call `-- propose --proposer P --op "$admin_op_json" --salt $salt`; keep
   `write_op_record(op_id, controller_fn, args_json, salt, cli_executable)` unchanged (the
   executeOp path is unaffected — it already calls generic `execute`).
2. `schedule_via_gov_self_proposer(admin_op_json, salt, …)`: call `-- propose … --op …`; record
   the `admin_op_json` for `execute_self` replay.
3. Each `propose_*` call site builds its `admin_op_json` (variant + struct payload via the
   existing `scval_*`/friendly builders) instead of passing typed `--flags`.
4. `gen_salt` stays (`sha256(NETWORK|fn|args_json)`); pass the same salt to `propose`. The op-id
   governance returns is the hash over `resolve_op`'s output + salt — consistent as long as
   `args_json` equals the scheduled args.

## Verification

- **Local (shape):** run each op's `admin_op_json` builder through `jq` and, where feasible,
  `stellar contract invoke … --build-only` + `stellar tx decode` to confirm the XDR matches the
  sdk-js byte-parity fixtures. Confirm the enum friendly-JSON shape on ONE op before fanning out.
- **Real gate (testnet):** a full `schedule → awaitOp → executeOp` dry-run on testnet for a
  representative op of each shape (unit, address, struct, oracle-resolved, gov-self). This is the
  only true validation; flagged because it cannot run locally.

## Non-goals

- No contract / interface / SDK changes — `propose(AdminOperation)`, `execute`, `execute_self`,
  and the views already exist.
- Not changing the op-record/`executeOp`/`cancelOp` storage format beyond what the per-op
  `args_json` shapes require.
- The integration-test flows (`tests/integration/flows/*.sh`) are a separate, already-fixed
  surface (direct controller calls, PR #24) and are out of scope here.
