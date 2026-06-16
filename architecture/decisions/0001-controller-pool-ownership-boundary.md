# ADR 0001: Governance, Controller, and Central Pool Boundary

- Status: Accepted
- Date: 2026-05-05
- Revised: 2026-06-16
- Deciders: XOXNO Lending contract team
- Supersedes: original separate-pool topology recorded in this ADR

## Context

A multi-asset lending protocol has to separate three jobs:

1. Protocol administration: listing assets, changing risk config, upgrading
   contracts, and managing privileged roles.
2. Account risk: user accounts, oracle reads, health-factor checks,
   liquidations, strategies, and pause gates.
3. Liquidity accounting: token custody, supplied and borrowed totals, indexes,
   reserves, protocol revenue, flash loans, and interest-rate parameters.

The current protocol uses one controller-owned central pool. Asset separation
is done by storage key:
`PoolKey::Params(asset)` and `PoolKey::State(asset)`.

Soroban makes each cross-contract call visible and budgeted. Batching
multi-asset operations into one pool call lowers invocation count while keeping
per-asset accounting rows independent.

## Decision

Adopt a three-contract production topology:

- One **governance** contract owns the controller. It validates admin inputs,
  schedules protocol-affecting changes through typed timelock proposers, and
  executes ready operations after the ledger delay
  (`contracts/governance/src/*`).
- One **controller** contract is the user-facing protocol contract. It owns
  account state, market configuration, oracle resolution, risk checks,
  liquidation, strategy orchestration, flash-loan orchestration, controller
  roles, and pause state (`contracts/controller/src/*`).
- One **central pool** contract is owned by the controller. It holds custody for
  every listed asset and stores per-asset accounting rows:
  `PoolKey::Params(asset)` and `PoolKey::State(asset)`
  (`contracts/pool/src/lib.rs`, `contracts/pool/src/cache.rs`).

The controller-to-pool ABI is the typed `LiquidityPoolInterface`
(`interfaces/pool/src/lib.rs`). The pool ABI carries the market asset in each
`PoolAction` or endpoint argument, so one pool call can process a batch of
asset-scoped entries.

Pool mutating endpoints, maintenance endpoints, and WASM upgrade are
`#[only_owner]`; the owner is the controller. The pool never calls oracles,
routers, governance, or another pool. It only performs asset-scoped accounting
and token transfers for requests authorized by the controller.

Market listing is split:

- `deploy_pool()` deploys the central pool once with deterministic
  `POOL_DEPLOY_SALT` and stores it in `ControllerKey::Pool`.
- `create_liquidity_pool(asset, params, config)` is a legacy ABI name. It
  creates an asset market inside the central pool, stores controller
  `Market(asset)` as `PendingOracle`, adds `asset` to `PoolsList`, and consumes
  the single-use `ApprovedToken(asset)` allow-list entry.
- Oracle activation is separate. A market becomes usable only after governance
  schedules and executes `set_market_oracle_config`.

## Alternatives Considered

- **Monolithic lending contract.** Rejected: it mixes administration, account
  risk, and custody in one upgrade surface. It also makes accounting and
  verification harder because every concern shares one state machine.
- **Separate pool contracts.** Superseded: they separate custody by contract,
  but every multi-asset operation crosses one pool boundary per asset. The
  current central pool keeps asset-scoped accounting rows in storage while
  allowing batched pool calls.
- **Pool-only architecture.** Rejected: cross-asset health checks still require
  a central risk authority. Letting pools coordinate directly would recreate a
  controller through ad hoc trust.
- **External shared vault plus separate accounting contracts.** Rejected:
  custody and accounting would split across more upgrade surfaces without
  improving the user-facing risk model.

## Consequences

Positive:

- User flows cross one pool contract boundary, even when a batch touches
  multiple assets.
- Account risk and oracle policy live in one place, the controller.
- Liquidity accounting remains per asset because every pool row is keyed by the
  token address.
- Pool reserve accounting uses internal `cash`, so direct token donations cannot
  inflate borrowable liquidity.
- Governance provides a single validated, timelocked admin path above the
  controller.

Negative / accepted costs:

- Pool WASM upgrade now affects all market rows at once. Asset-level accounting
  is separated by storage key, not by separate contract code.
- The central pool custody address holds all listed token balances. A pool-code
  bug has wider custody impact than in a separate-pool deployment.
- `PoolsList` is a legacy name. In current code it is the listed-asset registry,
  not a list of pool contract addresses.
- The controller is still the single user-facing risk authority. Governance
  timelock and immediate pause reduce admin risk, but do not remove the need to
  audit controller logic carefully.

## References

- `SCF_BUILD_ARCHITECTURE.md` §3 (System Topology), §4 (Contract
  Responsibilities), §6 (Market Lifecycle).
- `contracts/governance/src/{deploy.rs,forward.rs,timelock.rs,self_timelock.rs}`
- `contracts/controller/src/router.rs::{deploy_pool,create_liquidity_pool}`
- `contracts/controller/src/storage/instance.rs::{get_pool,set_pool}`
- `contracts/controller/src/storage/pools.rs`
- `contracts/pool/src/lib.rs`
- `common/src/types/pool.rs` (`PoolKey`, `PoolAction`, `Pool*Entry`)
- `interfaces/pool/src/lib.rs`
