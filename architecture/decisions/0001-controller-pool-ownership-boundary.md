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
  executes ready operations after the configured ledger delay.
- One controller contract is the user-facing protocol contract. It owns accounts,
  spoke configuration, oracle resolution, risk checks, liquidations, strategies,
  flash-loan orchestration, pause state, and pool ownership.
- One central pool contract is owned by the controller. It holds custody and
  stores `PoolKey::Params(HubAssetKey)` and `PoolKey::State(HubAssetKey)`.

Market creation requires a token approval and creates pool rows for the supplied
`HubAssetKey`. Price activation is separate: an asset becomes price-active only
when governance configures the token-rooted `AssetOracle(asset)` entry and the
source passes validation.

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
- Governance provides one timelocked admin path above the controller.

Accepted costs:

- A pool WASM upgrade affects all hub-asset rows.
- The central pool custody address holds all listed token balances.
- The controller remains the single user-facing risk authority and needs focused
  audit coverage.

## References

- [SCF_BUILD_ARCHITECTURE.md](../../SCF_BUILD_ARCHITECTURE.md)
- `common/src/types/pool.rs`
- `common/src/types/controller.rs`
- `contracts/controller/src/config`
- `contracts/pool/src/lib.rs`
