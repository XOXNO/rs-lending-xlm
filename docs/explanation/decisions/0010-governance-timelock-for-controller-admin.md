# ADR 0010: Governance Timelock For Protocol Admin

- Status: Accepted
- Date: 2026-06-13
- Deciders: XOXNO Lending contract team

## Context

Protocol-admin actions change market risk, oracles, code, pool config,
controller ownership, and governance itself. Multisig custody alone does not
provide an on-chain warning window.

```text
governance owner → governance → controller → central pool
```

Soroban disallows generic self-reentry, so controller-targeted operations and
governance-self operations use separate execution paths.

## Decision

Embed OpenZeppelin `stellar-governance` timelock state in the governance
contract. Schedule only through one typed entrypoint over a closed
`AdminOperation` enum. Callers cannot submit arbitrary target/function/args
triples.

### Controller-targeted operations

```text
Governance::propose(proposer, op: AdminOperation, salt) -> operation_id
```

1. Renew governance TTL; require `PROPOSER` (`begin_proposal`).  
2. `op::resolve_op` validates typed args and returns
   `(target, function, args, DelayTier)`.  
3. Schedule at that tier’s delay.  

`execute(executor, target, function, args, predecessor, salt)` runs ready
controller operations. `Some(executor)` must authorize and hold `EXECUTOR`;
`None` leaves execution open once ready.

Variants cover hubs, spokes, listings, caps, pool deploy/upgrade, controller
upgrade/ownership, position managers, aggregator, accumulator, oracle config,
tolerance, and `Unpause` (`contracts/governance/src/op.rs`).

### Governance-self operations

```text
Governance::execute_self(executor, op: AdminOperation, salt)
```

Same resolve, hash, delay, executor, and expiry rules; the target must be
governance; mutation applies via `apply_self_op`. Covers governance WASM upgrade,
delay update, role grant/revoke, and ownership-transfer initiation.

### Immediate vs timelocked

| Path | Who | What |
|------|-----|------|
| Immediate | **GUARDIAN** | Global `pause`; tighten-only `set_spoke_asset_flags`; `create_hub` / `add_spoke` (new registries stay inert until assets list through the timelock). Hub/spoke create also exist as timelocked `AdminOperation` variants. |
| Immediate | **ORACLE** | `set_sanity_band` (new band must contain the live price and overlap the previous band) |
| Immediate | **Owner** | `deploy_controller`; `revoke_role_immediate` for **GUARDIAN** and **ORACLE** only; accept a scheduled ownership transfer; views |
| Timelocked | PROPOSER → delay → EXECUTOR | Risk-loosening and structural admin, including `AdminOperation::Unpause`, listings, full oracle config, caps, upgrades, role grants, and clearing spoke flags (`EditAssetInSpoke`) |

Resume uses `propose(Unpause)` → wait → `execute`. Governance has no immediate
`unpause` entrypoint. Controller `pause` / `unpause` are owner-only (owner =
governance after execute).

`revoke_role_immediate` does not strip `PROPOSER`, `EXECUTOR`, or `CANCELLER`.
Those roles ride the timelock. A colluding-canceller deadlock uses Recovery-tier
`propose_canceller_reset` (~30 days, non-cancellable).

GUARDIAN may only tighten spoke flags (`false → true` or stay). Clearing a flag
reverts `SpokeAssetFlagRelaxation`; reopening uses timelocked
`EditAssetInSpoke` (including on deprecated spokes).

Roles are granted at construction (fresh deploy) or via timelocked
`GrantGovRole`. Production builds do not ship test-only immediate forwarders
(`#[cfg(any(test, feature = "testing"))]` only).

### Delay and roles

- Delay unit: ledgers. Constructor accepts any non-zero `min_delay`; production
  go-live policy uses `TIMELOCK_MIN_DELAY_LEDGERS = 34_560` (~48 h) as the
  live floor (ADR 0009 / DEPLOYMENT).  
- `DelayTier::Standard` → `get_min_delay()`.  
- `DelayTier::Sensitive` →
  `max(get_min_delay(), TIMELOCK_SENSITIVE_MIN_DELAY_LEDGERS = 120_960)` (~7 d)
  for: `UpgradeGov`, `UpgradeController`, `UpgradePool`,
  `TransferGovOwnership`, `TransferCtrlOwnership`, `SetPriceAggregator` only.  
- `DelayTier::Recovery` →
  `max(get_min_delay(), TIMELOCK_RECOVERY_MIN_DELAY_LEDGERS = 518_400)` (~30 d)
  for `propose_canceller_reset` only (not an `AdminOperation` variant).  
- Max delay: `TIMELOCK_MAX_DELAY_LEDGERS = 241_920` (~14 d); updates are
  non-decreasing.  
- Ready operations expire after `TIMELOCK_OPERATION_GRACE_LEDGERS = 120_960`.  
- Roles: `PROPOSER`, `EXECUTOR`, `CANCELLER`, `ORACLE`, `GUARDIAN`.  
- Delegated `EXECUTOR` and `CANCELLER` are not the same address. The owner may
  hold both.  

### Operation storage

Only pending operations (`Waiting` / `Ready`) keep an `OperationLedger` entry.
Execute and cancel both remove that entry (and local sidecars such as
`RoleRevocationTarget` / `RecoveryOp`). Re-proposing the same payload uses a
fresh `salt` for a new operation id. Typed `propose` paths always set
`predecessor = 0`; predecessor chaining is unsupported.

## Alternatives considered

- Off-chain notice only — not enforceable on-chain.  
- Public generic schedule — unvalidated calls could queue.  
- Generic self-targeted operations — conflicts with Soroban self-reentry rules.  
- One `propose_*` function per admin action — ABI grows with every action;
  `propose(op)` keeps typed variants without a combinatorial surface.  
- Timelocked emergency pause — too slow for halt.  
- Immediate unpause — risk-loosening; requires the delay.  
- Delay reductions — shortening delay is itself a governance risk.  

## Consequences

**Positive:** enforced delay on protocol-affecting and self operations; typed
validation before schedule; longer floor for upgrades and ownership transfer;
cancel before execute; halt is fast; resume is deliberate.

**Costs:** governance owns ABI encoding in `resolve_op`; routine admin is
slower; the immediate surface must stay narrow.

## References

- `contracts/governance/src/op.rs`  
- `contracts/governance/src/timelock.rs`  
- `contracts/governance/src/access.rs`  
- `contracts/governance/src/constants.rs`  
- `interfaces/governance/src/lib.rs`  
- [ADR 0011](./0011-pause-and-freeze-matrix.md)  
- [DEPLOYMENT.md](../../how-to/deploy-and-operate.md)  
