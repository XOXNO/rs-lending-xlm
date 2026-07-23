# Governance

Timelocked admin of the lending controller and price-aggregator. Role gates,
delays, and Recovery reset are documented on the rustdoc entrypoints.

| | |
| --- | --- |
| Owner | OZ `Ownable` (two-step) |
| Roles | `PROPOSER`, `EXECUTOR`, `CANCELLER`, `GUARDIAN`, `ORACLE` |
| Interface | `interfaces/governance` |
| Design | [ADR 0010](../../architecture/decisions/0010-governance-timelock-for-controller-admin.md) |

## Entrypoints

| Call | Role |
| --- | --- |
| `propose` | `PROPOSER` — schedule `AdminOperation` |
| `execute` / `execute_self` | `EXECUTOR` optional — run ready op |
| `cancel` | `CANCELLER` — veto pending (not Recovery) |
| `pause` / `set_spoke_asset_flags` / `create_hub` / `add_spoke` | `GUARDIAN` — immediate |
| `set_sanity_band` | `ORACLE` — immediate |
| `revoke_role_immediate` | Owner — strip `GUARDIAN`/`ORACLE` |
| `propose_canceller_reset` / `execute_canceller_reset` | Owner / open — Recovery reset |
| `deploy_controller` / `deploy_price_aggregator` | Owner — one-shot |
| `accept_ownership` | Pending owner |
| Views (`get_*`, `hash_operation`, `has_role`, `resolve_*`, addresses) | Public |

## Related

| Doc | Topic |
| --- | --- |
| [INVARIANTS](../../architecture/INVARIANTS.md) | Governance / pause matrix |
| [ADR 0001](../../architecture/decisions/0001-controller-pool-ownership-boundary.md) | Topology |
| [ADR 0010](../../architecture/decisions/0010-governance-timelock-for-controller-admin.md) | Timelock design |
| [ADR 0011](../../architecture/decisions/0011-pause-and-freeze-matrix.md) | Pause / freeze |
