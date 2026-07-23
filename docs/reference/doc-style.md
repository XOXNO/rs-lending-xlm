# Protocol documentation style

Code is the source of truth. Rewrite or delete comments that disagree with
behavior. Do not change runtime logic to match prose in a docs-only PR.

## Surfaces

| Surface | Shape |
|---------|--------|
| **Crate / module `//!`** | 2–6 lines: what it owns, trust/auth boundary, pointer to [invariants](./invariants.md) or an ADR. No file inventories. |
| **Public endpoint `///`** | (1) one-line effect + who may call, (2) `# Arguments` only if non-obvious, (3) `# Errors` with **named** variants, (4) `# Events` if emitted, (5) `# Security Warning` only when the caller must enforce solvency/auth this contract does not. |
| **Interface traits** | Same semantics as the matching `contractimpl`. Money-path: full docs on **both** trait and impl. Admin/view: full docs on the trait; impl may use a matching one-line effect. |
| **Types (`common`)** | Type: purpose + units. Fields: native / BPS / WAD / RAY, raw vs typed, ABI-stable misnomers called out. |
| **Errors** | One sentence: observable failure meaning. |
| **Internal helpers** | Doc only when non-obvious (rounding, CEI, index floors, trust). Delete restatements of the code. |
| **Contract README** | Index only: entrypoint name + one-phrase role + link to rustdoc / invariants. No duplicated full semantics. |

## Voice

- Present tense, active voice.
- No marketing. No “this function will…”.
- Prefer ≤8 lines per endpoint unless the Errors list is long.

## Anti-patterns

- Truncated openings (`/// position mutations…`).
- Stale constants in comments (wrong HF targets, renamed crates).
- Path names that do not match the tree (`aggregator/`, `xoxno-oracle-adapter/`).
- Describing intended-but-unimplemented behavior.

## Exemplar (pool-style public mutator)

```rust
/// Supplies `amount` into the market and mints scaled shares. Owner (controller) only.
/// The controller must pre-transfer the tokens.
///
/// # Errors
/// * `PoolNotInitialized` — no stored state for the market.
/// * `PoolInsolvent` — aggregate claims exceed cash plus debt.
/// * `SupplyRoundsToZeroShares` — positive amount mints zero shares at the current index.
///
/// # Events
/// * topics — `["market", "batch_state_update"]`
///
/// # Security Warning
/// * Performs no account health check; the controller must gate the supply.
```

## When behavior changes

Update rustdoc **and** any cited invariants / ADR section in the same PR.
See [CONTRIBUTING.md](../../CONTRIBUTING.md).
