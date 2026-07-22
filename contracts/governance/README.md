# Governance

Timelocked admin of the lending controller and price-aggregator. Ownable +
access-control roles; delay via `stellar-governance` timelock. Guardian/oracle
incident paths bypass delay.

| | |
| --- | --- |
| Owner | OZ `Ownable` (two-step) |
| Roles | `PROPOSER`, `EXECUTOR`, `CANCELLER`, `GUARDIAN`, `ORACLE` |
| Interface | `interfaces/governance` |
| Targets | controller, price-aggregator, self |

## Role

```text
PROPOSER ──propose──► Timelock ──execute──► Controller / PriceAggregator / Gov
GUARDIAN ──pause / flags (immediate)
ORACLE   ──sanity band / freeze (immediate)
```

Controller admin is not open: production ownership is this contract
([ADR 0010](../../architecture/decisions/0010-governance-timelock-for-controller-admin.md)).

## Surface

| Area | Entrypoints |
| --- | --- |
| Lifecycle | `deploy_controller`, `deploy_price_aggregator` |
| Timelock | `propose`, `execute`, `cancel`, `execute_self`, hash/state views |
| Immediate | `pause`, `set_spoke_asset_flags`, `set_sanity_band` (role-gated) |
| Recovery | `propose_canceller_reset`, `execute_canceller_reset` |
| Roles | grant/revoke (timelocked); `revoke_role_immediate` where allowed |
| Ops | Typed `AdminOperation` (assets, spokes, pool, oracle, upgrade, …) |

Delay tiers: standard min delay; sensitive (upgrades, ownership) and recovery
(council reset) raise the floor.

## Layout

```text
src/
  lib.rs       Contract shell
  timelock.rs  Propose / execute / cancel + delay tiers
  op.rs        AdminOperation → (target, fn, args, tier)
  access.rs    Roles
  validate/    Asset, spoke, oracle config/probe, tolerance
  deploy.rs    Controller + price-aggregator deploy
  storage.rs   Wired addresses, TTL
  events.rs    Governance events
```

## Related

| Doc | Topic |
| --- | --- |
| [ADR 0001](../../architecture/decisions/0001-controller-pool-ownership-boundary.md) | Topology |
| [ADR 0010](../../architecture/decisions/0010-governance-timelock-for-controller-admin.md) | Timelock design |
| [ADR 0011](../../architecture/decisions/0011-pause-and-freeze-matrix.md) | Pause / freeze |
