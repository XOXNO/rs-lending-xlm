## Summary

One paragraph: what changed and why. Reference specific INVARIANTS sections or ADRs (e.g. "ADR 0002 / INVARIANTS §5.2 storage", "oracle call-site policy per ADR 0004").

## Risk Surface

**High-priority areas touched (check all that apply and explain):**
- [ ] Storage layout, keys, TTL/renewal, per-side maps (`SupplyPositions`/`BorrowPositions`), `HubAssetKey`, or `PoolKey` changes
- [ ] Oracle config, price resolution, tolerance, providers (Reflector/RedStone/Xoxno), sanity/staleness, or call-site policy
- [ ] Risk params, LTV/liquidation curves, caps, position limits, min-borrow-collateral, or spoke overrides
- [ ] Authorization, delegates, position managers, pause/freeze matrix, or governance/timelock/roles/upgrade
- [ ] Flash-loan or strategy reentrancy, callback surfaces, or balance-delta validation (ADR 0005)
- [ ] Pool accounting, cash, interest split, revenue, bad-debt socialization, or reserves
- [ ] Controller–pool–governance boundaries (ADR 0001) or `HubAssetKey` isolation
- [ ] Common math/rates/types, events (stable ABI), or interface/ABI changes
- [ ] WASM size, Soroban footprint (reads/writes/entries), compute, or resource usage (`make wasm-size-check`)
- [ ] Other (describe):

- Affected contracts/modules:
- Affected invariants or ADRs (cite file:section):
- Specific impact (solvency, oracle, liquidation, flash-loan, storage, etc.):

See `docs/reference/invariants.md`, `docs/reference/architecture.md` §14–15, and `docs/explanation/decisions/README.md`.

## Verification

**Baseline (must pass):**
- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo test --workspace`
- [ ] `make test` (full harness) / `make test-pool`
- [ ] `make wasm-size-check`

**Protocol-sensitive changes — list exactly what was run and attach key results:**
- Certora profile(s) / rules (see `certora/profiles.json` and `certora/*/spec/README.txt`):
- Fuzz target(s) + duration (`make fuzz` / `make fuzz-contract`):
- Proptest / mutants / other (Scout, miri, specific harness tests):

Re-run against the exact tree in this PR. See docs/reference/architecture.md §14 and CONTRIBUTING.md.

## Operations & Downstream

- Deployment, WASM upgrade, or migration required:
- Configuration, role, cap, oracle, spoke, or governance follow-up (including timelock proposals):
- Keeper, indexer, configs, or off-chain impact (TTL, events, feeds):
- Breaking changes for users, liquidators, strategies, or integrators:

## Security

- [ ] This pull request does not disclose a vulnerability.
- [ ] Security-sensitive details, if any, were reported privately through `security@xoxno.com`.
