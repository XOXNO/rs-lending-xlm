# ADR 0001: Governance, Controller, Central Pool Boundary

- Status: Accepted
- Date: 2026-05-05
- Revised: 2026-06-30
- Deciders: XOXNO Lending contract team

## Context

The protocol separates three concerns:

1. Protocol administration: ownership, upgrades, listing, oracle configuration,
   spoke configuration, and launch controls.
2. Account risk: accounts, authorization, spoke risk, oracle policy, health
   factor checks, liquidation, flash loans, and strategy orchestration.
3. Liquidity accounting: custody, indexes, reserves, protocol revenue,
   flash-loan settlement, and bad-debt socialization.

The current protocol uses one controller-owned central pool. Asset separation is
done by `HubAssetKey { hub_id, asset }`, not by deploying one pool contract per
asset.

## Decision

Adopt a three-contract production topology:

- One governance contract owns the controller. It validates admin inputs,
  schedules protocol-affecting changes through typed timelock proposers, and
  executes ready operations after the configured ledger delay (see ADR 0010
  for the full `AdminOperation`, role, and `DelayTier` implementation).
  A narrow set of incident-response operations can execute immediately
  (owner `pause`/`unpause`; GUARDIAN per-listing flags; ORACLE sanity bounds).
- One controller contract is the user-facing protocol contract. It owns accounts,
  spoke configuration, oracle resolution, risk checks, liquidations, strategies,
  flash-loan orchestration, pause state, and pool ownership.
- One central pool contract is owned by the controller. It holds custody and
  stores `PoolKey::Params(HubAssetKey)` and `PoolKey::State(HubAssetKey)`.
  Pool ownership is set once in the pool constructor and the pool ABI exposes
  no `transfer_ownership` surface (contrast with controller and governance).

Market creation requires a token approval and creates pool rows for the supplied
`HubAssetKey`. Price activation is separate: an asset becomes price-active only
when governance configures the token-rooted `AssetOracle(asset)` entry and the
source passes validation.

Controller deployments start paused; the owner must explicitly unpause after
configuration.

Ownership wiring (constructors):
- Governance deploys controller, passing itself as initial owner.
- Controller deploys pool (via template), passing itself as owner.
- Pool ABI has no ownership transfer surface.

## Alternatives Considered

- **Monolithic lending contract.** Rejected because administration, risk, and
  custody would share one upgrade and verification surface.
- **Separate pool per asset.** Rejected because multi-asset operations would cross
  one pool boundary per asset. The central pool keeps per-market accounting
  isolated by storage key while allowing batched controller-pool flows.
- **Pool-only architecture.** Rejected because cross-asset account health still
  needs one risk authority.
- **External vault plus separate accounting contracts.** Rejected because it adds
  upgrade surfaces without improving the risk model.

## Consequences

Positive:

- User flows cross one pool boundary.
- Account risk and oracle policy live in the controller.
- Liquidity rows remain isolated by `HubAssetKey`.
- Internal pool `cash` prevents direct token donations from increasing borrowable
  liquidity.
- Governance provides the primary timelocked admin path above the controller
  (narrow immediate paths exist for incident response — see ADR 0010).
- Revenue claim crosses the boundary cleanly: pool `claim_revenue` (owner-only)
  transfers to the pool's owner (the controller); the controller forwards to the
  configured accumulator.
- Controller and governance use both `stellar_access::ownable` (`#[only_owner]`)
  and access-control admin roles; these are kept in sync on ownership transfer.

Accepted costs:

- A pool WASM upgrade affects all hub-asset rows.
- The central pool custody address holds all listed token balances.
- The controller remains the single user-facing risk authority and needs focused
  audit coverage.

## References

- [SCF_BUILD_ARCHITECTURE.md](../../SCF_BUILD_ARCHITECTURE.md) (topology + sections 1–7)
- ADR 0010 (governance timelock, roles, `DelayTier`, immediate paths)
- ADR 0009 (launch gates using this ownership chain)
- `common/src/types/pool.rs`
- `common/src/types/controller.rs`
- `contracts/governance/src/{deploy.rs, timelock.rs, op.rs, access.rs}`
- `contracts/controller/src/{governance/access.rs, pool_ops/mod.rs, config/mod.rs, setup/mod.rs, external/pool.rs, storage/instance.rs, storage/mod.rs}`
- `contracts/pool/src/lib.rs` (ctor owner, all mutators `#[only_owner]`)
- `interfaces/{pool, controller_admin}`
- Ownership/transfer tests (e.g. test-harness controller ownership tests)
- Central implementation facts: Governance owns Controller (via ownable + access-control); Controller is sole owner of Pool (no transfer surface in pool ABI); revenue flows Pool → Controller → accumulator.
