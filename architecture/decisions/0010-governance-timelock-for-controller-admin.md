# ADR 0010: Governance Timelock for Controller Admin

- Status: Accepted
- Date: 2026-06-13
- Deciders: XOXNO Lending contract team
- Supersedes: none (extends ADR 0009's deferred timelock decision)

## Context

ADR 0009 deferred an on-chain timelock: "No on-chain timelock is required at
launch. Adding one is a future governance decision." Non-emergency owner actions
relied on 48 hours of off-chain notice and operational discipline, which is
weaker than an enforced on-chain delay and leaves the protocol's #1 governance
residual open — a compromised or rushed owner key could change risk
configuration, oracle wiring, or contract code with no enforced warning window.

The protocol's admin surface is split (ADR 0001, governance split): the
`governance` contract owns the `controller`, validates every admin input, and
forwards accepted configuration to the controller's thin owner-gated setters.
This split is what makes an enforced delay tractable — there is a single
choke point (governance) through which all protocol-affecting admin flows.

A timelock must answer four questions:

1. Which admin operations are delayed, and which stay immediate?
2. What enforces the delay on-chain so it cannot be bypassed?
3. How long is the delay, and in what unit?
4. Who may schedule, execute, and cancel operations?

## Decision

Embed the OpenZeppelin `stellar-governance` `timelock` module in the governance
contract and route every controller-targeted admin operation through it. The
timelock's `Operation`/`OperationState` types, the ledger-based state machine,
operation hashing (Keccak256), and the scheduled/executed/cancelled events are
all the vendored OZ module (`vendor/openzeppelin/stellar-governance`); governance
owns only its entrypoint surface and the input validation.

### Realized design (B1): typed validating proposers

The original design scheduled governance-self-targeted operations that would
re-enter governance's own forwarders at execute time. That is **impossible on
Soroban**: empirically verified on `soroban-env-host` 26.1.3, a contract cannot
invoke itself (`invoke_contract` runs in `ContractReentryMode::Prohibited`) and
cannot self-authorize (`current_contract_address().require_auth()` has no
satisfier from its own frame). Scheduled operations must therefore target the
**controller** — a normal governance->controller cross-call, authorized because
governance is the controller's owner (the depth-1 Contract-Invoker rule).

This forces the boundary the design commits to:

- **Controller-targeted operations are timelocked.** Every protocol-affecting
  admin op (asset config, market creation, IRM upgrade, oracle config, position
  limits, e-mode, token approval, controller upgrade/migrate/ownership) is
  reachable only by scheduling it through a typed `propose_<op>` proposer. There
  is no public passthrough to the OZ generic `schedule`, so nothing unvalidated
  can ever be queued.
- **Governance-self-targeted admin is not timelocked** (self-reentry is
  impossible): governance's own `upgrade`, `update_delay`, `grant_role`,
  `revoke_role`, and `transfer_ownership` stay owner-gated and immediate. This is
  a documented limitation, not an oversight (see Consequences).

### The delayed surface (`forward.rs` / `timelock.rs`)

- **`propose_<op>(proposer, args…, salt) -> BytesN<32>`** (24 typed proposers):
  requires PROPOSER (`proposer.require_auth()` + `ensure_role(PROPOSER)`), runs
  the FULL input validation now (the existing `validate::*` bodies), builds an
  `Operation { target = controller, function = <controller thin-setter>, args =
  <validated args>, predecessor = zero, salt }`, and schedules it at the minimum
  delay. Validation runs at schedule time: for pure-input ops this is identical
  to execute time; for `propose_configure_market_oracle` the live feed probe runs
  at schedule, and a feed going stale over the delay is backstopped by the
  controller's fail-closed read path (accepted tradeoff).
- **`execute(executor, target, function, args, predecessor, salt) -> Val`**: one
  generic execute for all ops. When `executor` is `Some`, that address must hold
  EXECUTOR and authorize; `None` leaves execution open so anyone may push a ready
  op through after the delay. Calls OZ `execute_operation`, which does the
  `invoke_contract(controller, function, args)`.
- **`cancel(canceller, operation_id)`**: requires CANCELLER; returns a pending op
  to `Unset`.
- **Queries**: `get_min_delay`, `get_operation_state(id)`,
  `get_operation_ledger(id)`, `hash_operation(...)` — read-only wrappers over the
  OZ storage helpers.

### The immediate surface (owner-gated, not delayed)

- **`pause` / `unpause`**: emergency brakes forwarded to the controller. A 48h
  delay on halting a compromised market is unacceptable; these stay immediate.
- **`deploy_controller(wasm_hash)`**: one-time genesis bootstrap before the
  timelock has anything to govern.
- **`update_delay(new_delay)`** and governance-self admin: owner-immediate
  (self-target can't be timelocked, per the B1 boundary).

### Delay and roles

- **Unit is LEDGERS**, not seconds — the OZ state machine compares a stored
  ready-ledger against `e.ledger().sequence()`. The committed minimum is
  `TIMELOCK_MIN_DELAY_LEDGERS = 34_560` = 48h at the Stellar ~5s/ledger close
  time (= `2 × DAY_IN_LEDGERS`). If network close time drifts, wall-clock delay
  drifts proportionally; 34,560 ledgers is the on-chain invariant.
- **Roles** (`PROPOSER`, `EXECUTOR`, `CANCELLER`) are host-defined `Symbol`s
  gated by `stellar_access::access_control::ensure_role`; the OZ library leaves
  all role logic to the host. The constructor arms the minimum delay and grants
  all three roles to the deployer admin; operators separate them per ADR 0009's
  role policy before public unpause.

## Alternatives Considered

- **Off-chain notice only (ADR 0009 status quo).** Rejected as the long-term
  posture: it relies on operational discipline and provides no on-chain
  guarantee. The timelock makes the warning window unbypassable for
  controller-targeted changes.
- **Self-targeted governance operations re-entering the forwarders.** Rejected
  because it is impossible on Soroban (contract self-reentry and
  self-authorization are both prohibited by the host). This is what forced the
  B1 controller-targeted design.
- **Expose the OZ generic `schedule`.** Rejected: a generic scheduler could queue
  unvalidated operations. Only typed `propose_*` proposers exist, so every queued
  op carries the full input validation.
- **Timelock the emergency pause.** Rejected: delaying a halt of a compromised
  market converts a containable incident into a loss.

## Consequences

Positive:

- Every protocol-affecting controller admin change is delayed by an enforced
  on-chain 48h window; users and integrators get an unbypassable observation
  period. Closes the #1 governance residual (no on-chain timelock).
- Input validation runs at schedule time, so a bad proposal reverts immediately
  instead of sitting queued for 48h.
- Emergency pause stays immediate; the delay never blocks incident response.
- The OZ module carries the state machine, hashing, and events, minimizing
  bespoke timelock logic in governance.

Negative / accepted costs:

- **Governance-self admin (`upgrade`, `update_delay`, ownership, governance role
  grants) is NOT timelocked** — Soroban prohibits self-reentry, so a scheduled op
  cannot target governance itself. These stay owner-gated immediate. A
  governance-contract upgrade or an `update_delay` that shortens the window is
  therefore not itself delayed. Future hardening: a separate timelock-admin
  contract owning governance's upgrade, deferred.
- The production governance wasm carries 24 typed proposers plus the generic
  execute/cancel/queries (governance.wasm 54031 B). The harness keeps immediate
  owner-gated forwarders behind `#[cfg(any(test, feature = "testing"))]` so the
  400+ functional tests configure markets in one frame; the real timelock
  lifecycle is tested separately.
- A 48h delay on routine config changes slows non-emergency operations; this is
  the intended tradeoff.

## References

- `docs/superpowers/specs/timelock-integration.md` ("Revised design (B1)").
- `contracts/governance/src/timelock.rs` (execute/cancel/update_delay/queries).
- `contracts/governance/src/forward.rs` (24 typed proposers + immediate pause).
- `contracts/governance/src/access.rs` (constructor role grants, self-admin).
- `contracts/governance/src/constants.rs` (`TIMELOCK_MIN_DELAY_LEDGERS`).
- `vendor/openzeppelin/stellar-governance/src/timelock/` (OZ state machine).
- `contracts/governance/src/timelock.rs` tests + `tests/test-harness/tests/governance/timelock.rs`.
- ADR 0001 (Controller / Pool Ownership Boundary), ADR 0009 (Mainnet Launch
  Hardening) — this ADR fulfills 0009's deferred timelock decision.
