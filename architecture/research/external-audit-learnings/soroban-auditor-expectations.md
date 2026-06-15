# Soroban Auditor Expectations & Platform Footguns

What the auditors we'll likely engage actually look for in Soroban contracts, and the platform-specific (class-10) footguns they flag most. Derived from public Blend, Soroswap, Aquarius, Orbit/YieldBlox, and Stellar-core audits by Runtime Verification, Certora, OtterSec, Veridise, and CoinFabrik, plus CoinFabrik's Scout detector set.

## Recurring Soroban-platform footguns (ranked by how often auditors flag them)

1. **Unbounded data in Instance storage** — the single most repeated class-10 finding (OtterSec Soroswap HIGH, Veridise Orbit, RV Aggregator, Scout `dynamic-storage`/`vec-could-be-mapping`/`dos-unexpected-revert-with-vector`). Instance storage is loaded *in full on every invocation* and capped near the 64KB ledger-entry limit. **Use Persistent storage in per-key slots; never a growing `Vec`/`Map` in Instance.**
2. **`require_auth` auth-tree inheritance** — a top-level `require_auth` lets a malicious/upgradeable sub-contract smuggle extra `transfer` auth into the signed tree and drain *any* footprint asset (RV Aggregator A3 HIGH, RV StellarBroker, OtterSec Soroswap, Certora Aquarius M-02 citing Stellar #1092). **Allowlist callees; verify actual balance deltas; never trust returned amounts.**
3. **i128 fixed-point rounding direction + arithmetic safety** — round against the user (HF floor, utilization ceil, debt up); `overflow-checks = true` in `[profile.release]` is *not* sufficient — use `checked_*`/`saturating_*` at money sites; `.pow()` not `^`; multiply-before-divide; division must revert (not return 0) on zero. (OtterSec Blend rounding, Scout `overflow-check`/`divide-before-multiply`/`incorrect-exponentiation`, Blend BLRC-018 unsigned underflow.)
4. **Settle the index before mutating balances/rates** — Blend V2 had *five* findings where a balance/rate changed without first calling `update_emissions`/`accrue_interest`. Certora pushes coupling accrual into a single time-index. (Validates our risk-premium "settle on pre-mutation snapshot" rule.)
5. **First-depositor / share inflation** — hits every share-based subsystem (OtterSec Blend pool + backstop, Blend V2 M-18, Aquarius M-04). Reject donations / seed dead shares / virtual offset / require minted > 0.
6. **Two-step admin + role separation + timelock the upgrade path itself** — Veridise & Certora require splitting the omni-admin into Owner/Pause/Ops/Emergency, requiring the incoming admin to sign, and — critically (Aquarius H-01) — **timelocking `update_current_contract_wasm`**, or any governance delay is a false assurance (Soroban upgrades are instantaneous).
7. **Per-asset linear IO bricks liquidation** — Certora Blend V1 CRITICAL: health-check IO scales with assets held, hits the per-invocation resource limit, makes large positions un-liquidatable. Hard-cap assets/position width; profile against an 80–90% budget high-water mark.
8. **Separate code paths must replay every gate** — Blend V2: flash loans bypassed frozen-pool checks (M-01), re-cache (H-01), and `MAX_POSITIONS` (V2-CERT-M-01); the utilization ceiling was missing from withdraw (H-03). Validate-once-at-boundary only works if *all* entry points share the boundary.
9. **Storage durability / TTL** — extend TTL on config writes (Blend V2-I-01b); allowance `live_until_ledger` ≠ entry TTL (Veridise core); don't equate logical expiry with storage TTL; keeper liveness for footprint TTL.
10. **No Soroban constructors → deploy+init is two txs** — init not enforced before use is front-runnable (RV Aggregator A1); bind deploy salt to admin (Blend ADV-03). Deploy+init atomically via a deployer contract.
11. **Panic surface** — `unwrap`/`expect`/`assert!`/`panic!`/`Map::get` on a missing key all panic → whole-tx revert/DoS. Typed errors + `panic_with_error!` (fuzzable) + safe accessors + existence checks (Scout `unsafe-unwrap`/`unsafe-expect`/`unsafe-map-get`/`avoid-panic-error`/`assert-violation`).
12. **Events & token-interface compliance** — emit events *after* security-relevant storage mutations; SEP-41 token-event compliance; scaled-balance events must match the realized index-scaled delta (indexer correctness). Veridise: `Vec`/`Map` elements may fail `Val` round-trip → panic on retrieval.

## Per-auditor methodology & checklist

### OtterSec
Two-phase: (1) **design audit** — economic/game-theory soundness, flash-loan/large-deposit oracle manipulation treated as a chain-agnostic flaw; (2) **implementation audit** — reentrancy, account/ownership validation, arithmetic overflow, **rounding direction**. Severity keyed to fund-loss × preconditions; DoS = Medium. Cross-checks with other auditors. *(Blend V1, Soroswap, Kamino.)*

### Certora
Two modes: (a) **manual** security assessment (Blend V1, Aquarius — done even without running proofs); (b) **Sunbeam Prover** Wasm-level formal verification with **mutation testing** (Blend V2 FV: 21 seeded mutations across 5 backstop contracts, 229 mutation-catching properties). Pushes: continuous time-coupled indices, explicit `IsInitialized` flag, named scalar constants, **reserve write-back drop-guard**, non-negative index deltas, `checked_sub`/`checked_add` everywhere. **States its FV limits**: the quantitative "HF ≥ floor" was unprovable (too nonlinear) and replaced with a weaker "health is re-checked" surrogate, relying on unproven summaries + ≤2 loop unrolls + abstracted emissions — so "Verified" is conditional. *(Mirror this honesty in our Certora harness docs.)*

### Veridise
Impact × likelihood matrix. Core audit questions: is everything **metered** (DoS)? are errors handled so nodes never crash? is auth per-intent and replay-safe? are TTL lifetimes enforced? unchecked arithmetic? Soroban checklist they publish: validate `Vec`/`Map` ↔ `Val` round-trips, use `panic_with_error!` not `panic!`, manage `contractimport!` stale deps, **never put unbounded data in Instance storage**, document trust assumptions, **TWAP over spot**, two-step admin. *(Orbit CDP, Stellar Soroban core.)*

### Runtime Verification
3-week manual + invariant-driven property-based testing & symbolic execution with their in-house **Komet** tool + **Scout** static analysis. They explicitly gather core invariants *with the team*, build function-call maps, model out-of-scope mocks, and **cross-check an Ethereum-vulnerability list against Soroban**. Findings carry severity *and* execution-difficulty. *(Soroswap Aggregator, StellarBroker.)*

### CoinFabrik
Severity Critical/High/Medium/Minor + status. Checklist: arithmetic errors, race conditions, block-timestamp misuse, DoS, gas, function qualifiers, error handling, **input validation (`Vec` sizes)**, centralization/upgradeability. Maintains the **Scout-soroban** static analyzer (the codified detector set below). Prefers `transfer`+`require_auth` over allowance-based `transfer_from`. *(Aquarius.)*

## CoinFabrik Scout detector set (free pre-audit lint — run before engaging)
`overflow-check` (release profile), `integer-overflow-or-underflow`, `divide-before-multiply`, `incorrect-exponentiation` (`^` vs `.pow`), `unsafe-unwrap`, `unsafe-expect`, `unsafe-map-get`, `avoid-panic-error`, `assert-violation`, `unused-return-enum`, `iterators-over-indexing`, `dos-unbounded-operation`, `dos-unexpected-revert-with-vector`, `dynamic-storage`, `vec-could-be-mapping`, `unprotected-update-current-contract-wasm`, `set-contract-storage`, `unprotected-mapping-operation`, `unnecessary-admin-parameter`, `unrestricted-transfer-from`, `insufficiently-random-values` (ledger time/seq as randomness), `storage-change-events`, `token-interface-events`, `token-interface-inference`, `avoid-unsafe-block`, `avoid-core-mem-forget`, `soroban-version`, `unnecessary-lint-allow`.

## Pre-audit readiness checklist (derived)
- [ ] No unbounded collection in Instance storage; reserve/position registries are Persistent per-key.
- [ ] Every state-mutating entry point `require_auth`s the correct stored role; admin read from storage, never a caller arg.
- [ ] `update_current_contract_wasm` / upgrade is timelocked (not just `require_admin`); timelock delay snapshotted per pending proposal.
- [ ] Two-step ownership; on accept, full role set migrated atomically; revoke fails loud if target lacks the role.
- [ ] `overflow-checks = true` in `[profile.release]` **and** `checked_*`/`saturating_*` at money sites.
- [ ] Rounding direction audited: collateral/HF floor, debt/utilization-divisor ceil, mint divisors round up.
- [ ] Index accrued before validation and before any rate/reserve-factor change; timestamp deltas use `saturating_sub`.
- [ ] Flash-loan/strategy paths replay all normal-path gates (pause, per-asset freeze, position limits, caps, HF); callees allowlisted; output validated by balance delta.
- [ ] Position/feed iteration bounded (10+10) on every path so liquidation can't be resource-DoS'd.
- [ ] No `unwrap`/`expect`/`assert!`/`panic!`/unguarded `Map::get` on reachable contract paths.
- [ ] Events emitted after mutation; scaled-balance event amounts match realized index-scaled deltas; SEP-41 compliant.
- [ ] Storage TTL extended on config writes; logical expiry tracked independently of entry TTL.
- [ ] Markets never go live empty (first-depositor inflation); decimals fetched live and immutable.
- [ ] Certora harness docs state proof scope/limits honestly (reentrancy/accrual/loops out of scope).
- [ ] Run CoinFabrik Scout; resolve or justify every detector hit.
