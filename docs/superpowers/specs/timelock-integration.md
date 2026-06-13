# Timelock Integration Spec (OpenZeppelin `stellar-governance` 0.7.1)

Status: **LOCKED** — design ready for Task 2 (wire trait) and Task 3 (route forwarders).

Source of truth: the vendored crate at
`vendor/openzeppelin/stellar-governance/`, copied verbatim from the published
0.7.1 `.crate` (only the two `soroban-sdk` version pins were normalized from
`25.3.0` to `=26.0.0`, mirroring the sibling `stellar-access` /
`stellar-contract-utils` vendoring). All line references below are into that
vendored tree.

The published crate exposes three modules (`src/lib.rs:1-5`): `governor`,
`timelock`, `votes`. We embed ONLY `stellar_governance::timelock`. `governor`
and `votes` are token-weighted on-chain voting and are out of scope.

---

## (a) The `Timelock` trait

Declared `#[contracttrait] pub trait Timelock` at
`src/timelock/mod.rs:72-317`. Embeddable trait (NOT a standalone contract);
implementing it on our `Governance` struct generates a `TimelockClient`.

The crate docs state the host is responsible for "Authorization checks
(who can schedule/execute/cancel)" and "Initialization of minimum delay"
(`mod.rs:26-28`).

| Method | Signature (verbatim, `mod.rs`) | Default impl? | Host must supply |
|---|---|---|---|
| `get_min_delay` | `fn get_min_delay(e: &Env) -> u32` (`84-86`) | YES → `storage::get_min_delay` | no |
| `hash_operation` | `fn hash_operation(e, target: Address, function: Symbol, args: Vec<Val>, predecessor: BytesN<32>, salt: BytesN<32>) -> BytesN<32>` (`98-108`) | YES → builds `Operation`, calls `storage::hash_operation` | no |
| `get_operation_ledger` | `fn get_operation_ledger(e, operation_id: BytesN<32>) -> u32` (`116-118`) | YES → `storage::get_operation_ledger` | no |
| `get_operation_state` | `fn get_operation_state(e, operation_id: BytesN<32>) -> OperationState` (`126-128`) | YES → `storage::get_operation_state` | no |
| `schedule` | `fn schedule(e, target: Address, function: Symbol, args: Vec<Val>, predecessor: BytesN<32>, salt: BytesN<32>, delay: u32, proposer: Address) -> BytesN<32>` (`187-196`) | **NO** | auth + role check + `schedule_operation` |
| `execute` | `fn execute(e, target: Address, function: Symbol, args: Vec<Val>, predecessor: BytesN<32>, salt: BytesN<32>, executor: Option<Address>) -> Val` (`252-260`) | **NO** | optional auth + role + build `Operation` + `execute_operation` |
| `cancel` | `fn cancel(e, operation_id: BytesN<32>, canceller: Address)` (`297`) | **NO** | auth + role check + `cancel_operation` |
| `update_delay` | `fn update_delay(e, new_delay: u32, operator: Address)` (`316`) | **NO** | auth + `set_min_delay` |

The trait docstring at `mod.rs:63-71` lists exactly these four as having no
default impl. The four read-only query methods come for free.

The crate also defines `pub enum TimelockError` (`mod.rs:325-340`,
codes `4000..=4006`) and four contract events: `MinDelayChanged`,
`OperationScheduled`, `OperationExecuted`, `OperationCancelled`
(`mod.rs:362-491`).

---

## (b) `Operation` type and `OperationState` enum

`Operation` — `src/timelock/storage.rs:17-32`:

```rust
#[contracttype]
pub struct Operation {
    pub target: Address,        // contract to call
    pub function: Symbol,       // function to invoke on the target
    pub args: Vec<Val>,         // serialized args
    pub predecessor: BytesN<32>,// dependency op id; [0u8;32] == none
    pub salt: BytesN<32>,       // uniqueness, lets the same op be scheduled twice
}
```

`OperationState` — `src/timelock/storage.rs:38-47`:

```rust
#[contracttype]
#[repr(u32)]
pub enum OperationState { Unset, Waiting, Ready, Done }
```

State is derived (`storage.rs:116-126`) from a single stored `u32` ready-ledger
in `TimelockStorageKey::OperationLedger(id)` (persistent): `0` = `Unset`,
`1` = `Done` (`DONE_LEDGER`), `ready > current_ledger` = `Waiting`, else
`Ready`.

---

## (c) THE LINCHPIN — how execute invokes the target, and self-auth

### What the crate does

`execute_operation` — `src/timelock/storage.rs:292-296` (verbatim):

```rust
pub fn execute_operation(e: &Env, operation: &Operation) -> Val {
    set_execute_operation(e, operation);

    e.invoke_contract::<Val>(&operation.target, &operation.function, operation.args.clone())
}
```

`set_execute_operation` (`storage.rs:328-353`) does the state machine only:
asserts `is_operation_ready`, checks the predecessor is `Done`, writes
`DONE_LEDGER`, emits `OperationExecuted`. It performs the invocation NOWHERE
itself.

**The crate calls `e.invoke_contract(target, function, args)` with NO
`authorize_as_current_contract`, NO `require_auth`, and no auth wrapping of any
kind.** Grep of the whole vendored crate confirms `authorize_as_current_contract`
does not appear; the only call sites are the two bare `e.invoke_contract::<Val>`
in `timelock/storage.rs:295` and `governor/storage.rs:614`.

The crate docs explicitly flag the self-target case
(`storage.rs:259-264`): "For self-administration scenarios where the target is
the timelock contract itself, use [`set_execute_operation`] directly instead."
i.e. OZ's own recommendation for a self-targeted op is to bypass the
`invoke_contract` round-trip and dispatch inline within the same frame.

### The decisive Soroban auth rule (grounded, not assumed)

From the Stellar docs, Authorization → Contract Invoker
(`learn/fundamentals/contract-development/authorization.mdx`), verbatim:

> "A Contract Invoker corresponds to `Address::Contract` and is a special case
> where a contract calling another contract is considered authorized. This
> applies only to the **direct invoker** contract `Address`; calls on behalf of
> deeper contracts in the stack are not automatically authorized."

And (`build/guides/auth/contract-authorization.mdx`):

> "These direct calls are implicitly authorized by the invoker and do not
> require explicit authorization."

### Linchpin answer: CAN a self-targeted op re-enter governance's forwarder and pass a self-auth gate?

**YES — with one required change to the forwarder gate.**

When governance's `execute` runs `e.invoke_contract(governance_address, "set_aggregator", args)`,
the governance contract is the **direct invoker** of that sub-call. By the
Contract-Invoker rule, the governance contract's own `Address` is implicitly
authorized for the immediate callee frame. Therefore a
`env.current_contract_address().require_auth()` placed inside the re-entered
forwarder **passes automatically** — no `authorize_as_current_contract` is
needed, because self-invocation is a direct (depth-1) call, not a deeper-stack
delegation.

Likewise, the controller call the re-entered forwarder then makes
(`ControllerAdminClient::set_aggregator`, whose controller-side body asserts
`owner.require_auth()` with `owner == governance`) is ALSO a direct invoke from
governance → controller in that frame, so it too is invoker-authorized. The
existing production chain (governance test `forwarding_passes_controller_owner_auth_via_invoker`,
`contracts/governance/src/tests.rs:160-187`) already relies on exactly this
depth-1 invoker rule.

### Required forwarder-gate change (the mechanism Task 3 must use)

Today every forwarder is `#[only_owner]`, which expands to
`stellar_access::ownable::enforce_owner_auth(e)` →
`owner.require_auth()` (`vendor/openzeppelin/stellar-access/src/ownable/storage.rs:160-164`).
But the governance **owner is the deployer EOA**, set in `__constructor` via
`ownable::set_owner(&env, &admin)` (`contracts/governance/src/access.rs:70-74`).
A timelock self-execute is invoked by the **governance contract**, NOT the admin
EOA, so it would FAIL an `admin.require_auth()` gate.

Therefore the forwarders MUST be re-gated from owner-EOA auth to **self auth**:

```rust
// replaces #[only_owner] on each config/admin forwarder
env.current_contract_address().require_auth();
```

After this change the ONLY way to reach a forwarder body is via the
timelock-`execute` self-invocation (which the Contract-Invoker rule
auto-authorizes), so the delay becomes unbypassable and the forwarder is closed
to every external caller including the admin EOA. The admin EOA's only privileged
surface becomes the timelock entrypoints themselves (`schedule` / `cancel` /
`update_delay`, gated by PROPOSER/CANCELLER auth — see (d)).

This is NOT the OZ "use `set_execute_operation` directly" inline pattern. We
deliberately keep the `invoke_contract` round-trip (default `execute_operation`)
because it produces a clean, observable depth-1 self-call that the invoker rule
authorizes and that the auth tree records — simpler than threading inline
dispatch through a giant match over every forwarder selector.

---

## (d) Role constants and storage helpers the host wires in

### Role constants — NOT provided by the crate

The published 0.7.1 crate does **not** define PROPOSER / EXECUTOR / CANCELLER
constants. The `Timelock` trait leaves all role logic to the host; the role
symbols live only in the OZ *example* `examples/timelock-controller`
(referenced at `mod.rs:31-34`), which is not part of the published library.

We therefore define our own role `Symbol`s in governance and gate with
`stellar_access::access_control::ensure_role`. Proposed constants (Task 2):

- `PROPOSER` — may call `schedule`
- `EXECUTOR` — may call `execute` (or `None` → open execution)
- `CANCELLER` — may call `cancel`

Reuse the existing `access_control` admin already wired in governance
(`access.rs:70-74` sets admin = deployer and grants `ORACLE`). Grant
PROPOSER/CANCELLER to the admin EOA at construction; EXECUTOR can be open
(`executor: None`) so anyone may push a ready op through after the delay.

### Storage helpers re-exported from `stellar_governance::timelock`

All re-exported at `src/timelock/mod.rs:48-53`. Host wires these into the four
no-default methods (signatures verbatim from `storage.rs`):

- `schedule_operation(e: &Env, operation: &Operation, delay: u32) -> BytesN<32>` (`225`) — for `schedule`.
- `execute_operation(e: &Env, operation: &Operation) -> Val` (`292`) — for `execute` (does the `invoke_contract`).
- `set_execute_operation(e: &Env, operation: &Operation)` (`328`) — state-only variant for self-target inline dispatch (we do NOT use this; see (c)).
- `cancel_operation(e: &Env, operation_id: &BytesN<32>)` (`376`) — for `cancel`.
- `set_min_delay(e: &Env, min_delay: u32)` (`187`) — for `update_delay` and for init.
- `get_min_delay(e: &Env) -> u32` (`76`) — default `get_min_delay`.
- `hash_operation(e: &Env, operation: &Operation) -> BytesN<32>` (`403`) — Keccak256 over xdr(target)‖xdr(function)‖xdr(args)‖predecessor‖salt.
- query helpers: `get_operation_ledger` (`96`), `get_operation_state` (`116`), `is_operation_ready`/`pending`/`done`, `operation_exists`.

Role helpers from `stellar_access::access_control`
(`vendor/openzeppelin/stellar-access/src/access_control/storage.rs`):

- `ensure_role(e, role: &Symbol, caller: &Address)` (`627`) — throwing role gate (no auth inside; pair with explicit `caller.require_auth()`).
- `has_role(e, account, role) -> Option<u32>` (`43`).
- `grant_role_no_auth(e, account, role, caller)` (`220`) — for construction-time grants.

`schedule_operation` itself enforces `delay >= min_delay`
(`storage.rs:235-237`, `InsufficientDelay`) and `MinDelayNotSet`
(`storage.rs:76-81`), so the host cannot under-delay or schedule before init.

---

## (e) Delay unit and the 48h constant

**Unit is LEDGERS, not seconds.** Confirmed throughout: `delay: u32` is added to
`e.ledger().sequence()` to compute the ready ledger
(`storage.rs:239-243`: `ready_ledger = current_ledger + delay`), and state is
compared against `e.ledger().sequence()` (`storage.rs:117-124`). No timestamp
is used anywhere in the timelock module.

`min_delay` is stored in **instance** storage under
`TimelockStorageKey::MinDelay` (`storage.rs:52-54`, set at `190`, read at
`77-80`). It is `MinDelayNotSet` (panic `4005`) until initialized.

`set_min_delay(e, min_delay)` (`storage.rs:187-192`) is both the init and the
setter; it emits `MinDelayChanged{old, new}` (old defaults to `0`).

The crate's own ledger basis is `DAY_IN_LEDGERS = 17280` (`mod.rs:344`),
i.e. 86400 s/day ÷ **5 s/ledger**. Using that same assumption:

> **48h = 48 × 3600 ÷ 5 = 34,560 ledgers = `2 × DAY_IN_LEDGERS`.**

Locked constant:

```rust
/// Minimum timelock delay in LEDGERS. 48h at the Stellar ~5s/ledger close
/// time (= 2 × OZ DAY_IN_LEDGERS of 17280).
pub const TIMELOCK_MIN_DELAY_LEDGERS: u32 = 34_560;
```

Assumption stated explicitly: **~5 seconds per ledger close** (Stellar
mainnet's nominal close time; identical to the basis OZ uses for its own
`DAY_IN_LEDGERS`). If the network close time drifts, wall-clock delay drifts
proportionally; 34,560 ledgers is the on-chain invariant we commit to.

---

## (f) Constructor / initialization

The crate provides NO constructor — initialization is host-owned (`mod.rs:26-28`).
Governance's existing `__constructor(env, admin)` (`access.rs:70-74`) must be
extended (Task 2) to:

1. `set_min_delay(&env, TIMELOCK_MIN_DELAY_LEDGERS)` — arms the timelock; until
   this runs, `schedule` panics `MinDelayNotSet`.
2. Grant initial timelock roles to `admin`:
   `grant_role_no_auth(&env, &admin, &PROPOSER, &admin)` and the same for
   `CANCELLER`. EXECUTOR stays open (`executor: None`).

`update_delay` later lets governance change the delay — but, per the locked
design, `update_delay` itself should be reachable ONLY through the timelock
(self-auth), so changing the delay is itself delayed. (Decision for Task 3:
gate `update_delay`'s body on `current_contract_address().require_auth()` just
like the forwarders, and schedule delay changes as ordinary operations.)

---

## LOCKED INTEGRATION DESIGN

### Forwarder-auth mechanism (Task 3)

Replace `#[only_owner]` on every config/admin forwarder in
`contracts/governance/src/forward.rs`, `deploy.rs`, and the controller-admin
methods in `access.rs` with an explicit self-auth gate:

```rust
env.current_contract_address().require_auth();
```

Reachable only via timelock `execute` self-invocation (depth-1 Contract-Invoker
rule auto-authorizes it). Validation logic in `validate::*` stays exactly where
it is — but note it now runs at **execute** time. If we want validation to
revert at **schedule** time (better UX, fail fast), wrap each forwarder selector
in a typed schedule-time helper that re-runs `validate::*` before building the
`Operation`; this is optional and can land in Task 3.

### Public entrypoint surface governance exposes

Delayed (go through the timelock; `execute` re-enters the self-auth forwarders):

- `schedule(target, function, args, predecessor, salt, delay, proposer)` — `proposer.require_auth()` + `ensure_role(PROPOSER)` + `schedule_operation`.
- `execute(target, function, args, predecessor, salt, executor)` — if `Some(exec)`: `exec.require_auth()` + `ensure_role(EXECUTOR)`; then `execute_operation` (does the self-`invoke_contract`).
- `cancel(operation_id, canceller)` — `canceller.require_auth()` + `ensure_role(CANCELLER)` + `cancel_operation`.
- `update_delay(new_delay, operator)` — self-auth gate (delayed like the forwarders) + `set_min_delay`.
- `get_operation_state(operation_id)` / `get_min_delay()` / `hash_operation(...)` / `get_operation_ledger(...)` — free read-only views.

Immediate (NOT delayed — emergency / one-time, keep their current direct gate):

- `pause()` / `unpause()` — keep direct owner/admin auth. These are emergency
  brakes; a 48h delay on pausing a compromised market is unacceptable. Decision:
  gate on the admin EOA (a dedicated `GUARDIAN` role is the cleaner long-term
  shape, but admin-EOA is the minimal Task-3 change).
- `deploy_controller(wasm_hash)` — one-time bootstrap before the timelock has
  anything to govern; keep its current owner gate.

### Locked constant

```rust
pub const TIMELOCK_MIN_DELAY_LEDGERS: u32 = 34_560; // 48h @ 5s/ledger
```

### Testing-gated harness fast-path (preview for Task 4)

Unit/integration tests cannot wait 34,560 real ledgers. Two compatible levers,
both `#[cfg(any(test, feature = "testing"))]` only, so production WASM is
unaffected:

1. **Advance the ledger** — `e.ledger().set_sequence_number(seq + delay)` jumps
   past the ready ledger (the timelock is purely ledger-sequence based, so this
   is exact). This is the preferred path: it exercises the REAL delay logic.
2. **Execute-now shim** — a `#[cfg(feature = "testing")]` governance method that
   schedules with `delay = get_min_delay()` then advances and executes in one
   call, OR a testing-only constructor variant that sets
   `min_delay = 0` so ops are `Ready` immediately. Use sparingly; prefer (1) so
   the delay invariant stays under test.

The harness (`verification/test-harness`) should route admin actions through
`schedule` → ledger-advance → `execute`, mirroring the production path, with a
single helper that wraps the advance so existing admin-routing tests change
minimally.

---

## Version-compatibility record

- Published `stellar-governance` 0.7.1 normalized `Cargo.toml` requests
  `soroban-sdk = "25.3.0"` (caret) — IDENTICAL to the published
  `stellar-access` 0.7.1, which the repo already vendors and runs against
  `soroban-sdk = "=26.0.0"`. We applied the same one-line pin edit
  (`25.3.0` → `=26.0.0`, two occurrences). No other dependency. No
  `stellar-access` / `stellar-contract-utils` / `stellar-macros` dependency in
  the governance crate, so no transitive OZ-version conflict. **No genuine
  version conflict; the audit-window stack is unchanged.**
- Vendored layout mirrors the siblings exactly: `Cargo.toml`, `README.md`,
  `src/` only (stripped `.cargo_vcs_info.json`, `Cargo.lock`, `Cargo.toml.orig`).
- Patched into the root workspace via `[workspace.dependencies]
  stellar-governance = "=0.7.1"` and `[patch.crates-io] stellar-governance =
  { path = "vendor/openzeppelin/stellar-governance" }`. `cargo check
  --workspace` is green; cargo emits the expected
  "patch ... was not used in the crate graph" warning because no member depends
  on it yet (Task 2 adds the dependency).
