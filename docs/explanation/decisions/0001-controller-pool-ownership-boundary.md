# ADR 0001: Governance, Controller, Central Pool Boundary

- Status: Accepted
- Date: 2026-05-05
- Deciders: XOXNO Lending contract team

## Context

Three concerns stay separate:

1. **Admin** — ownership, upgrades, listing, oracles, spokes, launch controls  
2. **Risk** — accounts, auth, health factor, liquidation, strategies, flash loans  
3. **Liquidity** — custody, indexes, reserves, revenue, flash-loan settlement, bad debt  

One controller-owned central pool holds all markets. Separation is by
`HubAssetKey { hub_id, asset }`, not one pool contract per asset.

## Decision

Three-contract production topology:

| Contract | Role |
|----------|------|
| **Governance** | Owns the controller. Typed `AdminOperation` + timelock (ADR 0010). Roles (`PROPOSER`, `EXECUTOR`, `CANCELLER`, `ORACLE`, `GUARDIAN`) live here, not on the controller. |
| **Controller** | Sole user-facing surface: accounts, spokes, oracle, risk, liquidation, strategies, flash loans, pause state, pool ownership. Admin surface is `#[only_owner]` (owner = governance after deploy). |
| **Pool** | Owned by the controller. Custody + `Params`/`State` per `HubAssetKey`. All mutators `#[only_owner]`. No `transfer_ownership` in the pool ABI. |

Wiring:

- Governance owner deploys the controller with the governance contract as owner
  (`deploy_controller`, owner-gated one-shot).  
- Controller deploys the pool from the template with itself as owner
  (`deploy_pool`, reached via timelocked admin after the template is set).  
- Market creation creates pool rows for the `HubAssetKey` on that single pool.
  Price activation is separate: token-rooted `AssetOracle(asset)` must exist.

New controllers start **paused**. Resume is timelocked
`AdminOperation::Unpause` (GUARDIAN can pause immediately; see ADR 0010 / 0011).
Controller ownership remains transferable via timelocked
`TransferCtrlOwnership` (new owner must be a contract). Pool ownership is not.

Revenue: pool `claim_revenue` (owner-only) pays the controller; controller
forwards to the configured accumulator.

## Alternatives considered

- **Monolith** — rejected: one upgrade and verification surface for admin, risk, and custody.  
- **One pool per asset** — rejected: multi-asset ops would cross many pool boundaries.  
- **Pool-only** — rejected: cross-asset health needs one risk authority.  
- **External vault + separate accounting** — rejected: extra upgrade surface without better risk model.  

## Consequences

**Positive**

- User flows cross one pool boundary.  
- Risk and oracle policy stay in the controller.  
- Markets stay isolated by `HubAssetKey`.  
- Internal pool `cash` means direct token donations do not raise borrowable liquidity.  
- Admin is primarily timelocked; incident paths are narrow and named.  

**Costs**

- A pool WASM upgrade touches every hub-asset row.  
- One custody address holds all listed token balances.  
- Controller is the single user risk surface and needs focused review.  

## References

- [docs/reference/architecture.md](../../reference/architecture.md)  
- [ADR 0010](./0010-governance-timelock-for-controller-admin.md), [ADR 0011](./0011-pause-and-freeze-matrix.md), [ADR 0009](./0009-mainnet-launch-hardening-and-operational-control.md)  
- `common/src/types/{pool,controller}.rs`  
- `contracts/governance/src/{deploy,timelock,op,access}.rs`  
- `contracts/controller/src/{governance/access,pool_ops,config,setup,external/pool,storage}.rs`  
- `contracts/pool/src/lib.rs`  
- `interfaces/{pool,controller_admin}`  
