# Governance

Timelocked admin of the lending controller and price-aggregator. Role gates,
delays, and Recovery reset are documented on the rustdoc entrypoints.

| | |
| --- | --- |
| Owner | OZ `Ownable` (two-step) |
| Roles | `PROPOSER`, `EXECUTOR`, `CANCELLER`, `GUARDIAN`, `ORACLE` |
| Interface | `interfaces/governance` |

Pending ops only keep `OperationLedger` storage; execute and cancel remove it.
`salt` uniquifies re-proposes; `predecessor` is always `0`.

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

## Halt controls (global + per listing)

| Control | Immediate (GUARDIAN) | Recovery / clear |
| --- | --- | --- |
| Global controller pause | `pause` | Timelocked `AdminOperation::Unpause` |
| Per-spoke-asset `paused` / `frozen` | `set_spoke_asset_flags` (**ratchet**: may only tighten; clearing reverts `SpokeAssetFlagRelaxation`) | Timelocked `AdminOperation::EditAssetInSpoke` with the desired flags |

`EditAssetInSpoke` rewrites the full listing (risk params, caps, **and** halt
flags). Clearing flags there is intentional and delayed — not a bypass of the
immediate-path ratchet. Always pass the intended `paused`/`frozen` on every
edit so a risk-param change does not accidentally re-open a halted listing.
