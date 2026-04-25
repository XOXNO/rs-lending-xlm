# SDF Audit Bank — Submission Package

Single-document index for the Soroban Security Audit Bank intake form.
Maps each application requirement from
[Stellar Audit Bank Official Rules](https://stellar.gitbook.io/scf-handbook/supporting-programs/audit-bank/official-rules)
and the
[Audit Readiness Checklist](https://stellar.gitbook.io/scf-handbook/supporting-programs/audit-bank/audit-readiness-checklist)
to deliverables in this repository.

## 1. Project description and purpose

**Name**: XOXNO Lending (`rs-lending-xlm`)
**Network**: Stellar / Soroban
**Description**: Multi-asset lending and borrowing protocol. Suppliers
deposit collateral, borrowers draw against it, liquidators settle
underwater positions. Strategy primitives (`multiply` / `swap_debt` /
`swap_collateral` / `repay_debt_with_collateral`) and atomic flash loans
extend the core flows.
**Source of truth**: [`README.md`](../README.md),
[`architecture/ARCHITECTURE.md`](../architecture/ARCHITECTURE.md).

## 2. Smart-contract and technical-architecture details

| Document | Coverage |
|---|---|
| [`architecture/ARCHITECTURE.md`](../architecture/ARCHITECTURE.md) | System topology, controller↔pool boundary, sequence diagrams (supply / borrow / repay / withdraw / revenue). |
| [`architecture/DATAFLOW.md`](../architecture/DATAFLOW.md) | External entities, processes, data stores, trust boundaries, numbered data flows. STRIDE-template "What are we working on?" deliverable. |
| [`architecture/STORAGE.md`](../architecture/STORAGE.md) | Per-key durability tier, TTL strategy, cache layer. |
| [`architecture/ORACLE.md`](../architecture/ORACLE.md) | Reflector integration, two-tier tolerance bands, staleness, decimals discovery. |
| [`architecture/INVARIANTS.md`](../architecture/INVARIANTS.md) | 18 protocol invariants with algebra and worked examples. |
| [`architecture/ENTRYPOINT_AUTH_MATRIX.md`](../architecture/ENTRYPOINT_AUTH_MATRIX.md) | Per-fn auth × invariants × downstream pool calls. |
| [`architecture/ACTORS.md`](../architecture/ACTORS.md) | Privilege model, trust boundaries, operator-policy notes. |
| [`architecture/CONFIG_INVARIANTS.md`](../architecture/CONFIG_INVARIANTS.md) | Per-config-field validation site map. |
| [`architecture/STELLAR_NOTES.md`](../architecture/STELLAR_NOTES.md) | Soroban platform assumptions and open questions. |
| [`architecture/MATH_REVIEW.md`](../architecture/MATH_REVIEW.md) | Certora rule-coverage audit, doc-vs-code drift. |
| [`architecture/DEPLOYMENT.md`](../architecture/DEPLOYMENT.md) | Operator runbook (build / deploy / configure). |
| [`architecture/INCIDENT_RESPONSE.md`](../architecture/INCIDENT_RESPONSE.md) | Event-to-action playbook for the deployed protocol. |
| [`architecture/GLOSSARY.md`](../architecture/GLOSSARY.md) | Domain vocabulary (e-mode, isolation, scaled, indexes, ray / wad / bps, …). |

## 3. Development status and GitHub repositories

- Status: **pre-audit freeze**. Internal review and remediation complete
  (see §6). External engagement scheduled with Runtime Verification
  (implementation) and Certora (formal verification) in parallel.
- Frozen audit tag: `audit-2026-q2` on the `audit/2026-q2` branch.
- Toolchain pin: `rust-toolchain.toml` → `1.93.1`.
- Build verification: `make build` + `make optimize` (artefacts under
  `target/wasm32v1-none/release/`).
- 685 / 685 tests passing, 95.43 % line coverage on the in-scope crates
  (see [`audit/TOOLING_SCAN.md`](./TOOLING_SCAN.md)).

## 4. Previous security practices and audits

- Internal hunt + adversarial-loop rounds shipped (H-/M-/L-/N-/I-/C-/NEW-
  series). Canonical summary:
  [`audit/REMEDIATION_PLAN.md`](./REMEDIATION_PLAN.md).
- 95.43 % line coverage; 209 Certora rule fns across 13 confs; 6
  cargo-fuzz targets; 7 proptest harnesses; Miri on `common`.
- Static analysis clean: `cargo audit` 0 vulns (3 transitive
  informational advisories on `derivative`, `paste`, `rand` —
  documented); `cargo clippy -D warnings` clean; `soroban-scanner` 0
  findings on the deployed crate surface.
- No prior external audits.

## 5. Audit Readiness Checklist (Stellar SDF)

| Item | Status | Evidence |
|---|---|---|
| **Funding**: SCF-funded? | ✅ | XOXNO is an SCF-funded project. |
| **Repo Hygiene**: code well organised? | ✅ | Workspace structure (`controller/`, `pool/`, `pool-interface/`, `common/`, `test-harness/`, `fuzz/`), clear module boundaries, no top-level clutter. |
| **Integration Tests**: tests present + executed? | ✅ | `cargo test --workspace` → 685 passed. Suite covers controller, pool, integration flows, fuzz, proptest. |
| **Threat Model**: STRIDE-style threat model? | ✅ | [`audit/STRIDE.md`](./STRIDE.md) (Stellar SDF template format) plus deeper [`audit/THREAT_MODEL.md`](./THREAT_MODEL.md) (adversary-capability per concern class). |
| **Dataflow Diagram**: trust boundaries identified? | ✅ | [`architecture/DATAFLOW.md`](../architecture/DATAFLOW.md) — 7 trust boundaries, 10 numbered flows, Mermaid diagram. |
| **Tooling Scan** (optional/bonus): scanner report? | ✅ | [`audit/TOOLING_SCAN.md`](./TOOLING_SCAN.md) — `cargo audit` / `cargo clippy` / `soroban-scanner` / `cargo-fuzz` / `proptest` / Miri / Certora compile-check. |
| **Remediation Plan** (optional/bonus): plan for findings? | ✅ | [`audit/REMEDIATION_PLAN.md`](./REMEDIATION_PLAN.md) — every shipped fix with severity, root cause, fix commit, regression-test file. |

## 6. Eligibility

| Criterion | Status |
|---|---|
| SCF funding | Yes (XOXNO via SCF). |
| KYC / sanction checks | Pending — entity used for SCF funding to be re-verified at intake. Operator action: confirm with the SDF intake team that the KYC pack from the original SCF round still applies. |
| **Eligible category** | ✅ **Priority — Financial Protocol** (lending: managing on-chain user value). Also a **Yield-Bearing Token Protocol** through scaled supply / borrow indexes. |
| Code complete + audit needed within 4–6 weeks | ✅ Pre-audit freeze; Runtime Verification + Certora target as the next milestone. |
| Extensive tests + testnet | ✅ 685 tests, 95.43 % coverage; `make setup-testnet` deploys end-to-end and runs smoke validation per `architecture/DEPLOYMENT.md §Smoke-Test Runbook`. |
| Self-service tooling report + remediation plan in submission | ✅ [`audit/TOOLING_SCAN.md`](./TOOLING_SCAN.md) + [`audit/REMEDIATION_PLAN.md`](./REMEDIATION_PLAN.md). |
| STRIDE threat model in submission | ✅ [`audit/STRIDE.md`](./STRIDE.md). |
| Responsive during audit | Operator commitment. Slack / Discord channel to be opened at engagement start; daily async standups; weekly sync calls (per [`audit/AUDIT_CHECKLIST.md`](./AUDIT_CHECKLIST.md) "During Audit"). |
| Ability to cover co-pay | See §8 below. |

## 7. Audit-firm preference

The SDF intake form asks for an audit-firm preference. XOXNO's preference,
documented to inform SDF scheduling:

| Track | Preference | Rationale |
|---|---|---|
| **Formal verification** | **Certora** | Existing investment: 209 spec rule fns, 13 confs, vendored CVLR toolchain, 7 backfilled solvency rules. Compile-step gate passes (`cargo check --features certora --no-default-features`). Engagement-team handoff documented at [`controller/certora/HANDOFF.md`](../controller/certora/HANDOFF.md). |
| **Implementation review** | **Runtime Verification** OR **Spearbit + Cantina** | Runtime Verification: proven Soroban-domain expertise, complementary to Certora's FV track. Spearbit + Cantina: distributed-researcher network, breadth across DeFi-lending attack patterns. |

Both tracks can run in parallel per `audit/AUDIT_CHECKLIST.md`.

## 8. Co-payment and operational readiness

Per the [Co-Payment System](https://stellar.gitbook.io/scf-handbook/supporting-programs/audit-bank/official-rules#audit-co-payment-system):

| Audit | Co-pay | Refundable | Status |
|---|---|---|---|
| Initial Audit | 5 % | Yes — if all C/H/M findings closed within 20 business days. | Treasury earmarked. Operator commitment to the 20-business-day fix window: see "Operational readiness" below. |
| Growth Audit (>$10M TVL) | 0 % | n/a | Future. |
| Scale Audit (>$100M TVL) | 0 % | n/a | Future. |
| Pre-Traction Follow-Up #1 | 20 % | n/a | Treasury reserve scoped. |
| Pre-Traction Follow-Up #2 | 50 % | n/a | Treasury reserve scoped. |

### Operational readiness for the 20-business-day refund window

To qualify for the 5 % Initial Audit refund, every C/H/M finding must be
closed and verified within 20 business days. XOXNO's operational
commitments:

1. On-call engineer assigned to the audit-engagement window; dedicated
   capacity for fix shipping.
2. CI gates remain strict on the `audit/2026-q2` branch — every fix
   tagged with the engagement finding ID, regression test added under
   `test-harness/tests/`.
3. Async daily updates to the engagement Slack/Discord channel.
4. Weekly sync call with each audit firm.
5. Re-verification handoff back to the audit firm within 1 business day
   per shipped fix.

## 9. Hand-off artefacts

The audit team receives:

```
rs-lending-xlm @ tag audit-2026-q2

architecture/                        # protocol design + invariants + glossary + incident
audit/                               # this submission package + STRIDE + scope + threat model + remediation plan + tooling scan + maturity
controller/                          # protocol entrypoint
controller/certora/HANDOFF.md        # Certora-specific engagement notes
controller/certora/spec/             # 16 spec modules / 209 rule fns
controller/confs/                    # 13 prover confs
pool/                                # per-asset liquidity contracts
pool-interface/                      # controller -> pool ABI
common/                              # shared math, types, errors, events
test-harness/                        # 685-test integration + property + benchmark harness
fuzz/                                # 6 cargo-fuzz targets
vendor/cvlr/                         # vendored CVLR (Certora spec runtime), no_std-patched
configs/                             # operator deploy configs (testnet, mainnet)
.github/workflows/                   # CI: ci.yml + fuzz.yml + release.yml
```

## 10. Outstanding items at submission

Items deferred to engagement-team handling per
[`audit/AUDIT_CHECKLIST.md "Still Outstanding"`](./AUDIT_CHECKLIST.md#still-outstanding):

- Production-tx budget benchmark for `liquidate` at the contract cap
  `PositionLimits = 32/32`. Harness-only bench at 5/5 × 5 markets ships
  in `test-harness/tests/bench_liquidate_max_positions.rs`. Operator-policy
  fallback: keep `PositionLimits = 10/10` until measured on testnet.
- Reflector behaviour spec (`architecture/STELLAR_NOTES.md §Reflector`
  Q6–Q10). External Reflector-team contact required; if unanswered by
  kickoff, routed directly to auditors as an open ask.
- Empirical Certora prover run. Local toolchain validates spec compile
  and orphan-free conf↔rule mapping. Engagement-team dispatches the
  cloud prover. See `controller/certora/HANDOFF.md`.

Items resolved during this audit-prep cycle:

- ✅ STRIDE threat model deliverable.
- ✅ Dataflow diagram with explicit trust boundaries.
- ✅ Consolidated tooling scan report.
- ✅ Remediation plan canonicalised (replaces removed pre-remediation
  finding logs).
- ✅ `max_borrow_rate_ray ≤ 2 * RAY` cap in
  `validate_interest_rate_model` and `pool::update_params`. Regression
  tests in `test-harness/tests/admin_config_tests.rs`.
- ✅ Certora spec compile clean; orphan-check clean (`13 confs / 190
  source rules / 0 orphans`).
- ✅ Incident-response runbook.
- ✅ Domain glossary.

## 11. Submission checklist (before sending the SDF intake form)

- [ ] Confirm KYC / sanction-check pack is current (operator action).
- [ ] Push tag `audit-2026-q2` on the `audit/2026-q2` branch from
  current HEAD.
- [ ] Confirm Treasury earmark for 5 % Initial Audit co-pay.
- [ ] Submit SDF intake form with this document linked.
- [ ] Stand up engagement Slack/Discord channel.
- [ ] Notify Certora intake (engagement-track preference signalled).
- [ ] Notify Runtime Verification intake (engagement-track preference
  signalled).
