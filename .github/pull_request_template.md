## Summary

One paragraph: what changed and why, citing modules or entrypoints (e.g.
"controller withdraw restamp", "pool interest index floor").

## Risk Surface

**Areas touched (check and explain each):**
- [ ] Storage layout, keys, TTL/renewal, per-side maps (`SupplyPositions`/`BorrowPositions`), `HubAssetKey` or `PoolKey`
- [ ] Oracle config, price resolution, tolerance, providers (Reflector/RedStone/Xoxno/Aquarius), sanity/staleness, or call-site policy
- [ ] Risk params, LTV/liquidation curves, caps, position limits, min-borrow-collateral, or spoke overrides
- [ ] Authorization, delegates, position managers, pause/freeze matrix, or governance/timelock/roles/upgrade
- [ ] Flash-loan or strategy reentrancy, callback surfaces, or balance-delta validation
- [ ] Pool accounting, cash, interest split, revenue, bad-debt socialization, or reserves
- [ ] Controller–pool–governance boundaries or `HubAssetKey` isolation
- [ ] Common math/rates/types, events (stable ABI), or interface/ABI changes
- [ ] WASM size, Soroban footprint (reads/writes/entries), compute or resources (`make wasm-size-check`)
- [ ] Other (describe):

- Affected contracts/modules:
- Affected invariants (cite file:function or test):
- Impact (solvency, oracle, liquidation, flash-loan, storage, …):

See [CONTRIBUTING.md](../CONTRIBUTING.md), the contract README and rustdoc, and
the [invariants](../docs/reference/invariants.md).

## Verification

**Baseline (must pass):**
- [ ] `make fmt-check`
- [ ] `make clippy`
- [ ] `make build`, then `make test` (whole workspace)
- [ ] `make wasm-size-check`
- [ ] `make docs-check` / `make access-control-check`

**Protocol-sensitive changes — list what ran, with key results:**
- Certora profile(s) / rules (see `certora/profiles.json` and `certora/*/spec/README.*`):
- Fuzz target(s) + duration (`make fuzz` / `make fuzz-contract`):
- Proptest / mutants / other (Scout, miri, harness tests):

Run against the exact tree in this PR.

## Operations & Downstream

- Deployment, WASM upgrade, or migration required:
- Config, role, cap, oracle, spoke or governance follow-up (incl. timelock proposals):
- Keeper, indexer, configs, or off-chain impact (TTL, events, feeds):
- Breaking changes for users, liquidators, strategies, or integrators:

## Security

- [ ] This pull request does not disclose a vulnerability.
- [ ] Security-sensitive details, if any, were reported privately through `security@xoxno.com`.
