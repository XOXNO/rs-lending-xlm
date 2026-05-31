# ADR 0001: Controller / Pool Ownership Boundary

- Status: Accepted
- Date: 2026-05-05
- Deciders: XOXNO Lending contract team
- Supersedes: none

## Context

A multi-asset lending protocol has to decide where account-level risk logic
lives relative to per-asset liquidity custody. Two natural arrangements exist:

1. A single monolithic contract that owns every market, every account, and all
   token custody.
2. One pool contract per listed asset that owns custody and accounting for
   that asset, plus a controller contract that owns user accounts, oracle
   reads, risk checks, and cross-asset orchestration.

Soroban contracts have per-contract instance storage, per-contract upgrade
state, and explicit `require_auth` boundaries. Each cross-contract call is a
visible host invocation. Risk decisions need a transaction-local view across
multiple assets (collateral in A, debt in B, oracle prices for both).

## Decision

Adopt a controller-and-pool topology:

- One **controller** contract is the only user-facing protocol contract. It
  owns account state, market configuration, oracle resolution, access
  control, risk checks, liquidation, flash loans, and account-bound
  strategy flows (`controller/src/*`).
- One **pool** contract per listed asset. Each pool holds token custody and
  asset-local accounting (supply, debt, indexes, reserves, protocol revenue,
  flash-loan settlement, rate-model updates) for exactly its asset
  (`pool/src/lib.rs`, `pool/src/cache.rs`, `pool/src/interest.rs`).
- The controller-to-pool ABI is the typed Soroban trait
  `LiquidityPoolInterface` (`pool-interface/src/lib.rs`).
- Pools are owner-gated. Mutating accounting and maintenance endpoints enforce
  controller ownership through `verify_admin` (`pool/src/lib.rs`); pool WASM
  upgrade is gated by `#[only_owner]`. Pools never call oracles, routers, or
  other pools.
- Pools are deployed deterministically by the controller (salt derived from
  the asset address) with the controller as owner and asset
  `MarketParams` as constructor input
  (`controller/src/router.rs::create_liquidity_pool`).

## Alternatives Considered

- **Monolithic contract.** Rejected: a single contract concentrates upgrade
  blast radius, mixes asset custody with account logic, and makes per-asset
  WASM upgrades impossible. Storage would also accumulate global state that
  cannot be partitioned by asset, hurting TTL economics.
- **Pool-only architecture (per-asset contracts coordinate directly).**
  Rejected: cross-asset health checks would require either trust between
  pools or a shadow registry, both of which collapse back into a controller.
- **Controller plus a single shared "liquidity vault."** Rejected: single
  vault custody concentrates risk under one upgrade path and erases the
  per-asset upgrade isolation the chosen design provides.

## Consequences

Positive:

- Per-asset upgrade isolation. `upgrade_pool` and `upgrade_pool_params`
  target one asset without touching others
  (`controller/src/router.rs`).
- Custody is partitioned: a bug in one pool cannot drain another asset.
- Account state and risk live in one place
  (`controller/src/positions/*.rs`), so health-factor and oracle checks have
  a coherent view.
- Pools are simple, single-asset, owner-gated state machines whose
  invariants are tractable to verify (`verification/certora/pool/spec/`).
- The `LiquidityPoolInterface` trait gives a typed, audit-friendly surface
  for cross-contract calls.

Negative / accepted costs:

- Every user-visible mutation crosses at least one cross-contract boundary,
  costing one host invocation per touched pool.
- Pool storage and TTL must be maintained per asset
  (`keepalive_pools`).
- The controller becomes a single point of upgrade for all account logic;
  mitigated by `upgrade()` auto-pausing
  (`controller/src/access.rs`) and two-step ownership transfer
  (see ADR 0009).

## References

- `SCF_BUILD_ARCHITECTURE.md` §3 (System Topology), §4 (Contract
  Responsibilities).
- `controller/src/router.rs::create_liquidity_pool`
- `pool/src/lib.rs::verify_admin`
- `pool/src/lib.rs::upgrade`
- `pool-interface/src/lib.rs`
