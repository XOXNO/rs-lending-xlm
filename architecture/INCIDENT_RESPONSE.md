# Incident Response Runbook

Operator-facing event-to-action playbook for the deployed
`rs-lending-xlm` controller and pools. Pairs with
[`architecture/DEPLOYMENT.md`](./DEPLOYMENT.md) (deploy / configure flow)
and [`architecture/ACTORS.md`](./ACTORS.md) (privilege model).

This file is the on-call reference. It assumes:

- a Soroban event indexer is wired to a paging system (PagerDuty,
  Opsgenie, similar);
- the on-call engineer has access to the Owner key (or the multisig
  ceremony to invoke it) and the KEEPER / REVENUE / ORACLE role
  keys;
- the operator can dispatch the verbs in the unified Makefile
  dispatcher (`make mainnet pause`, `make mainnet unpause`, etc — see
  `Makefile`).

For the full event catalogue, see
[`common/src/events.rs`](../common/src/events.rs). For per-fn auth
requirements, see
[`architecture/ENTRYPOINT_AUTH_MATRIX.md`](./ENTRYPOINT_AUTH_MATRIX.md).

## Severity tiers

| Tier | Definition | Response target |
|---|---|---|
| **SEV-1** | User funds at risk OR the protocol is on a path toward bad-debt socialisation in the next ledger or two. | Page on-call within 5 minutes; first action within 15 minutes. |
| **SEV-2** | A single market is impaired (oracle stale, pool TTL near expiry, configuration drift). User funds are not currently at risk but a SEV-1 is plausible if untreated. | Page within 15 minutes; first action within 1 hour. |
| **SEV-3** | An informational event has fired and warrants triage but does not require an immediate operator action. | Triage within one business day. |

## Event-to-action matrix

| Event | Default Sev | Trigger | First action | Owner |
|---|---|---|---|---|
| [`PoolInsolventEvent`](#poolinsolventevent) | SEV-1 | Bad-debt socialisation has just lowered `supply_index_ray` toward the `10^18` floor. | Pause the affected pool's mutating endpoints (controller-wide `pause()` if scale warrants); investigate the trigger (oracle? liquidator gas griefing? real bad debt?). | Owner |
| [`CleanBadDebtEvent`](#cleanbaddebtevent) | SEV-2 | KEEPER cleared a stuck bad-debt account that was below the `5 * WAD` threshold for in-liquidation socialisation. | Confirm the account met the precondition; check whether the same KEEPER call triggered a `PoolInsolventEvent` on any market. | KEEPER + Owner |
| [`UpdateDebtCeilingEvent`](#updatedebtceilingevent) | SEV-3 | Isolated-debt aggregate moved (often a routine borrow / repay on an isolated market). Surface only the **near-ceiling** signal; route the rest to the indexer's archive. | If `new_aggregate / ceiling > 0.95`: page ORACLE / Owner to either raise the ceiling or freeze new isolated borrows by editing the asset config. | ORACLE + Owner |
| [`FlashLoanEvent`](#flashloanevent) | SEV-3 (anomaly only) | A flash loan completed. Fire SEV-2 only on **anomaly**: borrow size > 50 % of pool reserves OR fee deviates from the configured `flashloan_fee_bps`. | Confirm the receiver address is known; check post-tx reserves match expected; if anomalous, snapshot the tx hash and trace the receiver's calls. | KEEPER (monitoring), Owner (response) |
| [`UpdateAssetOracleEvent`](#updateassetoracleevent) | SEV-3 (route by status) | Oracle wiring changed (`configure_market_oracle`, `edit_oracle_tolerance`) OR the market transitioned to `Disabled` via `disable_token_oracle`. | If the transition is `Active → Disabled`: SEV-1, page Owner immediately (kill switch fired). If `PendingOracle → Active`: routine; verify the new wiring against the operator runbook. If tolerance bands changed: SEV-2 if last_tolerance widened, SEV-3 otherwise. | ORACLE + Owner |
| [`UpdateMarketStateEvent`](#updatemarketstateevent) | SEV-3 (routine) | Per-market index, reserves, and price update. Continuous emission. | Used for off-chain dashboards (utilisation, supplied/borrowed totals, indexes). Page only if `current_reserves < 0` or `borrow_index_ray < previous` (monotonicity invariant violation). | KEEPER (monitoring) |
| [`UpdateMarketParamsEvent`](#updatemarketparamsevent) | SEV-2 (manual change) | Owner ran `upgrade_pool_params`. | Confirm the change matches the operator-runbook ticket. If unplanned, **assume operator-key compromise**: invoke `pause()`, rotate Owner key, follow `architecture/ACTORS.md §Owner`. | Owner |
| [`UpdateAssetConfigEvent`](#updateassetconfigevent) | SEV-2 (manual change) | Owner ran `edit_asset_config`. | Same as above. | Owner |
| [`ApproveTokenWasmEvent`](#approvetokenwasmevent) | SEV-2 | Owner mutated the token allowlist. | Confirm against ticket; verify the WASM hash matches a vetted SAC / SEP-41 build. Allowlist semantics are creation-time only — see [`architecture/ACTORS.md §Operator policy notes`](./ACTORS.md). | Owner |
| Soroban host: `Error(Budget, ExceededLimit)` | SEV-2 | A user tx hit Soroban's tx-budget cap. | Identify which endpoint; if `liquidate` at maxed positions: page Owner; consider lowering `PositionLimits` per [`audit/THREAT_MODEL.md §3.3`](../audit/THREAT_MODEL.md). | Owner |

## Per-event detail

### `PoolInsolventEvent`

Payload:
- `asset: Address` — pool whose supply index dropped.
- `bad_debt_ratio_bps: i128` — `bad_debt_usd_wad / total_supply_usd_wad`
  in BPS at the moment of socialisation.
- `old_supply_index_ray: i128` — pre-mutation supply index.
- `new_supply_index_ray: i128` — post-mutation supply index. Floor at
  `10^18` raw.

#### Investigation checklist

1. Recover the triggering tx hash from the event topic stream.
2. Determine whether the trigger was:
   - in-liquidation socialisation (the `LiquidationEvent` is one tx
     before this one), or
   - the KEEPER `clean_bad_debt` standalone path
     (`CleanBadDebtEvent` is in the same tx).
3. If in-liquidation: check the target account; was the underwater
   condition driven by oracle drift (compare pre/post `lastprice` on
   the asset against external sources) or organic price action?
4. If KEEPER standalone: confirm the affected account met
   `coll_usd ≤ 5 * WAD && debt_usd > coll_usd`. If not, the KEEPER role
   is compromised — rotate.
5. Compute `new_supply_index_ray / 10^18 raw`. If close to the floor
   (`< 1.5 × 10^18`), pause the pool: future bad debt will clamp at
   the floor and silently lose precision.

#### Response

- **Single-market**: pause via off-chain operator policy (no per-market
  pause exists today; only protocol-wide `pause()`).
- **Cross-market correlation** (multiple `PoolInsolventEvent` in one
  ledger window): SEV-1 escalation — likely oracle compromise.
  Invoke `pause()`. Rotate ORACLE role keys.

### `CleanBadDebtEvent`

Payload:
- `account_id: u64` — account whose debt was cleaned.
- `total_borrow_usd_wad: i128` — debt at the moment of cleanup.
- `total_collateral_usd_wad: i128` — collateral at the moment of cleanup.

#### Investigation checklist

1. Confirm the precondition: `total_collateral_usd_wad ≤ 5 * WAD &&
   total_borrow_usd_wad > total_collateral_usd_wad`.
2. Pair this event with the matching `PoolInsolventEvent` (same tx).
3. Confirm the caller is a known KEEPER address; if not, rotate.

### `UpdateDebtCeilingEvent`

Payload (per asset, on every mutation):
- `asset: Address`
- `new_aggregate_usd_wad: i128` — new isolated-debt total for this
  asset.

#### Filter

Page only when `new_aggregate / ceiling > 0.95`. The 95 % threshold
gives the operator one ledger window to react before isolated borrows
saturate the ceiling.

### `FlashLoanEvent`

Payload:
- `caller: Address`
- `asset: Address`
- `amount: i128`
- `fee: i128`
- `receiver: Address`

#### Anomaly heuristics

Page on **any** of:

- `amount > 0.5 * pre_event_reserves(asset)` (large flash loan).
- `fee != amount * flashloan_fee_bps / BPS` (rounding aside; verify
  half-up).
- The same `receiver` invoking flash loans more than 5 times in one
  ledger window (possible looped attack).

### `UpdateAssetOracleEvent`

Payload mirrors `OracleProviderConfig`. The most consequential transitions
are:

| From → To | Severity | Note |
|---|---|---|
| `PendingOracle` → `Active` | SEV-3 | First-time market wiring. Verify the runbook ticket. |
| `Active` → `Disabled` | SEV-1 | Kill switch fired. Liquidations still proceed; supply / borrow / withdraw freeze. Page Owner. |
| `Active` → `Active` (tolerance widened) | SEV-2 | Wider tolerance allows more drift through `(spot+safe)/2` averaging. Confirm operator intent; if unplanned, ORACLE-role compromise. |
| `Active` → `Active` (tolerance tightened) | SEV-3 | Routine risk-tightening. |

### `UpdateMarketStateEvent`

Continuous emission. Surface only the monotonicity-violation signal:

- `borrow_index_ray < previous` (per asset) — SEV-1, prod invariant
  break (`architecture/INVARIANTS.md §6`).
- `supply_index_ray < previous - 1` outside a `PoolInsolventEvent` tx
  — SEV-1, ditto (`architecture/INVARIANTS.md §7`; bad-debt is the
  only exception).
- `current_reserves < 0` — SEV-1, accounting break.

The remaining signal feeds dashboards (utilisation, total supply,
total borrow per market). No paging.

### `UpdateMarketParamsEvent` / `UpdateAssetConfigEvent`

Manual operator action. Both should match a runbook ticket.

If un-ticketed: **assume operator-key compromise**:
1. Invoke `pause()` (Owner).
2. Snapshot the chain at the affected ledger.
3. Rotate Owner via `transfer_ownership` → `accept_ownership` from a
   fresh hardware wallet.
4. Re-grant KEEPER / REVENUE / ORACLE roles to fresh keys.
5. Investigate the compromise root cause; coordinate disclosure
   per `SECURITY.md`.

### `ApproveTokenWasmEvent`

Mutates the token allowlist (`approve_token_wasm` /
`revoke_token_wasm`). Allowlist semantics are creation-time only:
revoking a token does **not** stop existing pools from using it
(`architecture/ACTORS.md §Operator policy notes`, M-12). If a token
WASM becomes hostile post-listing, the Owner must `pause()` and
migrate users to a new market.

## Standing operations

- **Keeper cron**. KEEPER role MUST run `keepalive_shared_state`,
  `keepalive_accounts`, and `keepalive_pools` at a cadence faster than
  the persistent / instance TTL thresholds (30 d shared / 100 d user /
  120 d instance). Recommendation: run daily; raise to hourly during
  any incident.
- **Update indexes**. KEEPER MUST call `update_indexes([assets])` at
  least every 24 h to keep market state fresh and roll interest
  accrual.
- **Price observation**. The off-chain monitor MUST read Reflector
  spot + TWAP every 60 s and compare to an external reference
  (CoinGecko, CEX REST). Sustained divergence > 10 % feeds the SEV-2
  oracle path.
- **Reserves observation**. Per-pool `reserves()` view; alert when
  utilisation > 95 % for more than 1 h (rate spike imminent).

## Emergency contact tree

(to be filled in by operator team in a private internal doc; this file
intentionally does not pin specific names or pager IDs.)

| Role | Primary | Secondary | Pager group |
|---|---|---|---|
| Owner | TBD | TBD | TBD |
| KEEPER on-call | TBD | TBD | TBD |
| ORACLE on-call | TBD | TBD | TBD |
| REVENUE on-call | TBD | TBD | TBD |
| Audit-firm liaison (during engagement) | TBD | TBD | TBD |

## What this file is NOT

- A replacement for `audit/STRIDE.md` or `audit/THREAT_MODEL.md`.
  Those frame what *can* go wrong; this file frames what to do when
  one of those scenarios materialises.
- A replacement for the audit-engagement `ENGAGEMENT_FINDINGS.md`
  (the auditor opens that fresh at engagement start per
  `audit/AUDIT_CHECKLIST.md`).
- A user-facing security policy. That is `SECURITY.md` at the repo
  root; this file is operator-internal.
