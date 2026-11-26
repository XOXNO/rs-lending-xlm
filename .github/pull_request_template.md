## Summary

Write one paragraph that states what changed and why. Cite modules or
entrypoints, for example "controller withdraw restamp" or "pool interest index
floor".

## Risk areas

**Areas touched (check and explain each):**
- [ ] Storage layout, keys, TTL or renewal, per-side maps (`SupplyPositions`/`BorrowPositions`), `HubAssetKey` or `PoolKey`
- [ ] Oracle config, price resolution, tolerance, providers (Reflector, RedStone, Xoxno, or Aquarius), sanity or staleness checks, or call-site policy
- [ ] Risk params, LTV or liquidation curves, caps, position limits, min-borrow-collateral, or spoke overrides
- [ ] Authorization, delegates, position managers, pause and freeze matrix, or governance, timelock, roles, or upgrade
- [ ] Flash-loan or strategy reentrancy, callbacks, or balance-delta validation
- [ ] Pool accounting, cash, interest split, revenue, bad-debt socialization, or reserves
- [ ] Controller, pool, and governance boundaries or `HubAssetKey` isolation
- [ ] Common math, rates, types, events (stable ABI), or interface or ABI changes
- [ ] WASM size, Soroban footprint (reads, writes, and entries), compute or resources (`make wasm-size-check`)
- [ ] Other (describe):

- Affected contracts or modules:
- Affected invariants (cite file:function or test):
- Impact (for example solvency, oracle, liquidation, flash-loan, or storage):

See [CONTRIBUTING.md](../CONTRIBUTING.md), the contract README and rustdoc, and
the [invariants](../docs/reference/invariants.md).

## Verification

**Baseline (must pass):**
- [ ] `make fmt-check`
- [ ] `make clippy`
- [ ] `make build`, then `make test` (whole workspace)
- [ ] `make wasm-size-check`
- [ ] `make docs-check` and `make access-control-check`

**Protocol-sensitive changes. List what ran, with key results:**
- Certora profiles and rules (see `certora/profiles.json` and `certora/*/spec/README.*`):
- Fuzz targets and duration (`make fuzz` / `make fuzz-contract`):
- Proptest, mutants, or other checks (Scout, miri, harness tests):

Run the checks against the exact tree in this pull request.

## Operations and downstream

- Deployment, WASM upgrade, or migration required:
- Config, role, cap, oracle, spoke, or governance follow-up (including timelock proposals):
- Keeper, indexer, configs, or off-chain impact (TTL, events, feeds):
- Breaking changes for users, liquidators, strategies, or integrators:

## Security

- [ ] This pull request does not disclose a vulnerability.
- [ ] Security-sensitive details, if any, were reported privately through `security@xoxno.com`.
