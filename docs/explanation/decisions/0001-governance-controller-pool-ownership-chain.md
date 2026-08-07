# 0001. Governance → Controller → Pool ownership chain

Status: Accepted

## Context

The protocol is split across several Soroban contracts: a timelocked governance
contract, a controller that holds all account, risk, and oracle logic, and a
liquidity pool that custodies tokens and keeps market accounting. Every
contract boundary is an authorization boundary, and every additional privileged
actor multiplies the attack surface: a transferable admin key on the custody
layer is a single point of catastrophic failure, and risk checks duplicated
across layers invite the two copies to drift apart. The design also needs a
deterministic pool address so that off-chain configuration, indexers, and
integration tests can derive it rather than discover it. Finally, only one
contract should ever face users, so that pausing, halting, and risk
enforcement have exactly one place to live.

## Decision

Authority flows through a single strict chain: Governance owns the Controller,
and the Controller owns the Pool. Each link is enforced with `#[only_owner]`
on every mutating entrypoint — the pool applies it to every mutating
`LiquidityPoolInterface` function in `contracts/pool/src/lib.rs`, so the
Controller is the pool's sole caller.

Governance deploys the Controller itself, passing its own address as the
constructor admin
(`contracts/governance/src/deploy.rs::deploy_controller`, via
`env.deployer().with_current_contract(salt).deploy_v2(wasm_hash, (env.current_contract_address(),))`).
The Controller deploys the Pool the same way
(`contracts/controller/src/markets/mod.rs::deploy_pool`), using the fixed
all-zero 32-byte `contracts/controller/src/markets/mod.rs::POOL_DEPLOY_SALT`
so the pool address is deterministic, and rejecting a second deployment with
`GenericError::PoolAlreadyDeployed`.

The pool's ownership is immutable by construction:
`contracts/pool/src/lib.rs::LiquidityPool::__constructor` calls
`ownable::set_owner` exactly once, and the contract does not implement the
`Ownable` trait under `#[contractimpl]` — unlike
`contracts/swap-aggregator/src/lib.rs::Router` and
`contracts/xoxno-oracle/src/lib.rs::XoxnoOracle`, which do — so it exposes no
`transfer_ownership`/`accept_ownership`. Re-pointing the pool's owner requires
a pool WASM upgrade.

The pool holds no pause flag, no risk parameters, and no oracle reads. Every
guard it applies is arithmetic and backing-based
(`contracts/pool/src/guards.rs::require_utilization_below_max`,
`::require_backed_market`, `::require_solvent_withdraw_state`). All risk,
solvency, oracle, and authorization logic lives one layer up in the
Controller, the only user-facing contract.

## Alternatives

**A transferable pool owner (two-step Ownable, as the swap-aggregator uses).**
The pool would expose `transfer_ownership`/`accept_ownership`, letting
governance re-point the pool to a replacement controller without touching pool
code. That flexibility is precisely the risk: a compromised or mistaken
transfer hands the entire custody layer to an arbitrary address. Because the
Controller is itself upgradeable under governance, controller replacement is
already possible without ever moving the pool's owner, so the transfer surface
buys nothing the chain does not already provide.

**Risk checks duplicated in the pool as defense-in-depth.** The pool would
re-validate health factors, caps, or halt flags before mutating state. This
splits ownership of each invariant across two codebases that must be upgraded
in lockstep; a divergence produces either a bricked market (pool stricter) or
a false sense of coverage (pool laxer). Keeping the pool's guards purely
arithmetic keeps every risk invariant in one auditable place.

**User-facing pool entrypoints with the controller as middleware.** Users
would call the pool directly and the pool would consult the controller for
authorization. This inverts the trust relationship, forces the pool to hold
account context, and makes the pool's ABI the public surface — every risk
change would then leak into custody-layer upgrades.

## Consequences

Authorization review collapses to one question per layer: "is the caller the
owner?" — see ../../reference/invariants.md §INV-AUTH. The pool's threat model
excludes direct user interaction entirely, and a compromised controller is the
worst case the pool must survive (see ../threat-model.md). The deterministic
salt makes the pool address derivable by off-chain tooling from the controller
address alone.

What this makes hard: the pool owner cannot be rotated operationally — any
re-pointing is a WASM upgrade flowing through the governance timelock. All
halt semantics (§INV-HALT) and risk enforcement (§INV-RISK) must be complete
in the Controller, because the pool will execute whatever its owner asks,
subject only to its backing guards (§INV-ACCT). What must stay true: no pool
entrypoint may ever gain a caller other than the Controller, and no
`Ownable` implementation may be added to the pool without revisiting this
decision.
