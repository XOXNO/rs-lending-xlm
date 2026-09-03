# Shared seed knowledge (read first)

## Code map

- Entrypoints: `contracts/controller/src/lib.rs`
- Cache: `contracts/controller/src/context/` (`Cache` in `mod.rs`)
- Spoke usage: `contracts/controller/src/spoke_usage.rs`
- Storage: `contracts/controller/src/storage/`
- Positions: `contracts/controller/src/positions/{supply,debt,mod,liquidation}/`
- Risk gates: `contracts/controller/src/risk/{validation,totals,params}.rs`
- Strategies: `contracts/controller/src/strategies/`
- Pool FFI: `contracts/controller/src/external/pool.rs`
- Permissionless surface: `scripts/permissionless_entrypoints.txt`

## Docs of record

- `docs/reference/invariants.md`
- `docs/reference/formulas.md`
- `docs/explanation/threat-model.md`
- `STRIDE.md`
- `docs/reference/numeric-bounds.md`

## Hard unit rules

Never mix token amounts, WAD, RAY, BPS in one expression without explicit conversion.
Rounding must favour protocol invariants (ADR-0003).

## Cache facts (verified from source)

`Cache` memoizes: token prices, market indexes, pool address, pool sync data,
spoke usage context, spoke config, spoke assets, verified hubs; buffers supply
and debt event deltas. `Cache::new` renews instance TTL; `new_view` does not.

## Spoke usage facts (verified from source)

`SpokeUsageContext` loads rows lazily, applies entry with cap check via
`calculate_scaled_cap`, apply_exit no-ops on missing row / zero delta, and
`persist()` writes every cached row. Cap errors are spoke supply/borrow cap
errors.

## Coordination path

Write findings to `docs/audit/controller-defense/findings/AXXX-*.md`.
Do not edit other agents' files; create `disagreements/AXXX-vs-AYYY.md` if needed.
