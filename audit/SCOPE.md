# Audit Scope

**Frozen commit**: `5ee115cfa5097670add106c348b189f01bf3d62b` (`5ee115c`)
**Branch**: `main`
**Date frozen**: 2026-04-16
**Tag (to create)**: `audit-2026-q2`

## In Scope (4 crates, ~16,585 LOC Rust)

### `controller/src/` — protocol entrypoint (~9,386 LOC)

| File | LOC | Purpose |
|---|---|---|
| `lib.rs` | 1,532 | Public ABI, role gating, dispatch |
| `oracle/mod.rs` | 1,620 | Reflector integration, price safety, tolerance bands |
| `oracle/reflector.rs` | 40 | Reflector client wrapper |
| `storage/mod.rs` | 1,032 | All controller storage keys, TTL discipline |
| `config.rs` | 987 | Asset config, e-mode, pool template, oracle wiring |
| `strategy.rs` | 760 | `multiply` / `swap_debt` / `swap_collateral` / `repay_debt_with_collateral` |
| `views.rs` | 716 | Read-only ABI surface |
| `positions/borrow.rs` | 646 | `borrow_batch`, isolation/silo/cap checks |
| `positions/supply.rs` | 638 | `process_supply`, account creation, e-mode entry |
| `positions/liquidation.rs` | 605 | Liquidation cascade, bad-debt socialization |
| `helpers/mod.rs` | 500 | USD math, HF math, valuation helpers |
| `positions/repay.rs` | 446 | Bulk repay, isolated debt decrement, dust rule |
| `router.rs` | 313 | Pool deployment, claim_revenue routing |
| `validation.rs` | 314 | Cross-field invariant checks |
| `positions/emode.rs` | 299 | E-mode threshold overrides |
| `cache/mod.rs` | 248 | Per-tx oracle/config cache |
| `utils.rs` | 164 | Index sync, isolated debt update |
| `positions/account.rs` | 161 | Account meta lifecycle |
| `positions/update.rs` | 153 | Threshold propagation |
| `positions/withdraw.rs` | 143 | Bulk withdraw, dust-lock guard |
| `flash_loan.rs` | 74 | Flash loan orchestration |
| `helpers/testutils.rs` | 52 | Test helpers (compiled in test cfg) |
| `positions/mod.rs` | 8 | Module index |

### `pool/src/` — per-asset liquidity engine (~2,516 LOC)

| File | LOC | Purpose |
|---|---|---|
| `lib.rs` | 1,657 | Pool ABI, supply/borrow/withdraw/repay, flash-loan begin/end, claim_revenue, seize_position, params |
| `interest.rs` | 380 | Index updates, bad-debt socialization, supply-index floor |
| `views.rs` | 258 | Pool read surface |
| `cache.rs` | 221 | Per-call cache |

### `pool-interface/src/` — controller→pool ABI (~76 LOC)

| File | LOC | Purpose |
|---|---|---|
| `lib.rs` | 76 | Client trait the controller uses |

### `common/src/` — shared math, types, errors, events (~4,542 LOC)

| File | LOC | Purpose |
|---|---|---|
| `events.rs` | 807 | Event payload types and emit fns |
| `types.rs` | 540 | All `#[contracttype]` ABI structs |
| `rates.rs` | 426 | Piecewise rate model, supply rate split |
| `fp.rs` | 355 | Typed `Wad`/`Ray`/`Bps` newtypes |
| `fp_core.rs` | 210 | `mul_div_half_up` primitive |
| `errors.rs` | 146 | All `#[contracterror]` enums |
| `constants.rs` | 49 | RAY/WAD/BPS, TTL tiers, tolerance bounds |
| `lib.rs` | 9 | Module index |

## Out of Scope

| Path | Why excluded |
|---|---|
| `fuzz/` | Property tests; results consumable, harness out of scope |
| `test-harness/` | Integration test infrastructure |
| `vendor/` | Vendored CVLR (formal-verification toolchain only) |
| `controller/certora/` | Formal-verification rules; Certora workstream runs separately (see `controller/certora/SPIKES.md`) |
| `configs/` | Operator deployment files; on-chain validation enforces correctness, not file content |
| `target/`, `Cargo.lock` | Build artifacts |
| `Makefile`, `configs/script.sh` | Operator tooling, not on-chain code |

## Trusted Dependencies (out-of-scope, but list for SBOM)

- `soroban-sdk` — Stellar Soroban runtime
- `stellar-access`, `stellar-macros`, `stellar-contract-utils` — OpenZeppelin Stellar contracts (ownable/access-control/pausable/upgradeable). v0.7.0 — young (~79★, project started Dec 2024). Recommend a targeted manual review of the access-control + ownable + pausable + upgradeable code paths exercised by the controller.
- Reflector oracle contracts (external, on-chain) — `configs/<network>_markets.json` sets addresses per network

## Operator policy preconditions (referenced from `DEPLOYMENT.md`)

The protocol's accounting math assumes properties that on-chain validation cannot enforce. `approve_token_wasm` MUST NOT admit:

- **Fee-on-transfer tokens** (finding H-06)
- **Rebasing tokens** (finding H-07)

Approved tokens MUST be standard SAC or audited SEP-41 with strict 1:1 transfer semantics. See `DEPLOYMENT.md` § "Token allowlist policy".

## Build Verification

```bash
git checkout 5ee115cfa5097670add106c348b189f01bf3d62b
make build           # cargo build --target wasm32v1-none --release -p controller -p pool
make optimize        # stellar contract optimize
make test            # cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings   # must be clean
```

Toolchain: see `rust-toolchain.toml`.
