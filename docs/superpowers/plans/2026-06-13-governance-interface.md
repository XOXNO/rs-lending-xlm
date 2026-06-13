# `interfaces/governance` crate

**Goal:** Add the missing `governance-interface` crate so the governance contract has a published ABI + lightweight `GovernanceClient`, matching the `interfaces/{controller,pool}` convention. The governance split added `interfaces/controller/src/admin.rs` (for gov→controller forwarding) but never gave the *governance* contract its own interface.

**Pattern decision:** **client-only**, mirroring `controller-interface` (not pool's formal `impl`). The `#[contractclient(name = "GovernanceClient")]` trait generates the client; the governance contract matches by ABI name (its 53 entrypoints span 4 `#[contractimpl]` blocks + cfg-gated test methods, so a single formal `impl` would be invasive). Lower risk, consistent with the closest analog (controller).

**Scope:** additive. Governance is the root owner — no contract calls it — so no consumer is *forced* to migrate. Validate the interface matches the real ABI by pointing the test-harness `GovernanceClient` at the new crate and running the governance suite (a signature mismatch then fails loudly).

## Changes
1. **New crate `interfaces/governance`** (package `governance-interface`):
   - `Cargo.toml`: deps `soroban-sdk` (workspace), `common` (path), `controller-interface` (path), `stellar-governance` (workspace). Mirror `interfaces/controller/Cargo.toml`.
   - `src/lib.rs`: `#[contractclient(name = "GovernanceClient")] pub trait GovernanceInterface { … }` mirroring the **production public** entrypoints (read exact signatures from `contracts/governance/src/{deploy,forward,timelock,access}.rs`):
     - deploy: `deploy_controller(wasm_hash) -> Address`, `controller() -> Address`
     - timelock: `execute(executor: Option<Address>, target, function: Symbol, args: Vec<Val>, predecessor, salt) -> Val`, `cancel(canceller, operation_id)`, `update_delay(new_delay)`, `get_min_delay() -> u32`, `get_operation_state(id) -> OperationState`, `get_operation_ledger(id) -> u32`, `hash_operation(...) -> BytesN<32>`, `resolve_market_oracle_config(asset, cfg) -> MarketOracleConfig`, `resolve_oracle_tolerance(first, last) -> OraclePriceFluctuation`
     - 24 `propose_*` (exact arg types from forward.rs)
     - immediate: `pause()`, `unpause()`
     - meta-admin: `upgrade(hash)`, `transfer_ownership(new_owner, live_until_ledger)`, `accept_ownership()`, `grant_role(account, role: Symbol)`, `revoke_role(account, role: Symbol)`
   - **EXCLUDE** the cfg-gated test-only methods (`set_controller`, `has_role`) and `__constructor` (constructors aren't trait methods; clients call via `register`).
   - Use canonical type paths: oracle/config types from `controller_interface::types::*`, `MarketParamsRaw`/`InterestRateModel` from `common::types`, `OperationState` from `stellar_governance::timelock`.
2. **Workspace**: add `"interfaces/governance"` to root `Cargo.toml` members.
3. **Validate** (prove ABI parity): repoint `verification/test-harness` (and the governance contract tests if they import the contract-crate client) to `governance_interface::GovernanceClient`; run `cargo test -p governance` + `cargo test -p test-harness --test governance` green. If a signature is wrong, the generated client won't match the contract at call time → fix the trait.

## Verify
- `cargo check --workspace`; `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test -p governance`; `cargo test -p test-harness --test governance` (+ fuzz governance auth if it uses the client)
- production build unaffected; no contract code behavior change.

## Notes / boundaries
- Client-only means no *compile-time* contract↔interface enforcement (same tradeoff the controller already accepts). A future hardening could make governance formally `impl GovernanceInterface` (pool pattern) — flagged, not done here.
- One commit: `feat(interface): governance-interface crate (GovernanceClient)`.
