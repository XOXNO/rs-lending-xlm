# Audit records and dispositions

This archive records security-review scope, finding dispositions, and retained
evidence. Findings are reconciled against source at
`d26b93ebb48d718b69571ec737f0097af3379916`. Test counts belong to the reviewed
revisions; they do not attest current code, hosted proofs, or deployment state.

For the source behavior and trust assumptions that guide a fresh review, start
with the [Threat model](../explanation/threat-model.md).

## Source revisions and scope

### Controller defensive protections, September 2026

The re-derivation reviews source at
`a2afb21cc826f79679d7f421b89eca046ff09e2c`. Its scope covers controller
authorization, entry gates, storage, measured custody, spoke usage, and
operation context. It reports no confirmed controller-boundary extraction under
the stated ownership and token assumptions.

Pool arithmetic and oracle aggregation are outside the 110-scope coverage
claim. A separate drain analysis inspects supporting paths. The A001–A110 wave
notes were assembled separately; use the
[final synthesis and corrections](https://github.com/XOXNO/rs-lending-xlm/blob/d26b93ebb48d718b69571ec737f0097af3379916/docs/audit/controller-defense/synthesis/FINAL.md),
[residual revalidation](https://github.com/XOXNO/rs-lending-xlm/blob/d26b93ebb48d718b69571ec737f0097af3379916/docs/audit/controller-defense/synthesis/RESIDUAL_REVALIDATION.md),
and [drain analysis](https://github.com/XOXNO/rs-lending-xlm/blob/d26b93ebb48d718b69571ec737f0097af3379916/docs/audit/controller-defense/synthesis/DRAIN-ANALYSIS.md).

### Astra public-mutation review, 2026-09-05

The [review](https://github.com/XOXNO/rs-lending-xlm/blob/d26b93ebb48d718b69571ec737f0097af3379916/docs/audit/astra-2026-09-05/report.md)
covers controller public mutations through the pool, price aggregation, and
shared math at source `99613335b410f70ff42dd99d13ff530f6adaee67`.
Its [source manifest](astra-2026-09-05/evidence/source-sha256.txt) contains
132 files.

The report records no newly confirmed vulnerability and 2,120 passing native
tests, including two composition probes. The five dispatched independent reviewers
did not return completed final reviews. See the
[reviewed source](https://github.com/XOXNO/rs-lending-xlm/tree/99613335b410f70ff42dd99d13ff530f6adaee67).

### Pool accounting and arithmetic review, 2026-09-05

The [review](https://github.com/XOXNO/rs-lending-xlm/blob/d26b93ebb48d718b69571ec737f0097af3379916/docs/audit/pool-accounting-2026-09-05/report.md)
covers all 34 production files in pool, common math, and common rates at source
`99613335b410f70ff42dd99d13ff530f6adaee67`. It includes three completed
independent reviews and reports no newly confirmed ordinary-user extraction path.

Recorded validation includes 397 distinct passing native tests, 83,792
actual-source arithmetic assertions, and 105 independent equation-replay
combinations. The [coverage and hashes](https://github.com/XOXNO/rs-lending-xlm/blob/d26b93ebb48d718b69571ec737f0097af3379916/docs/audit/pool-accounting-2026-09-05/coverage.md)
and [validation record](https://github.com/XOXNO/rs-lending-xlm/blob/d26b93ebb48d718b69571ec737f0097af3379916/docs/audit/pool-accounting-2026-09-05/validation.md)
bind those results to their review scope.

The counts overlap across reviews and must not be added into an assurance
total. A passing test can intentionally reproduce a known limitation.

For the controller review, final corrections take precedence over the A101–A110
syntheses' missing-file claims and rankings. The A001–A100 primaries remain on
the audit branch associated with PR #134, outside the retained local corpus.

## Disposition ledger

The states below describe the reconciliation revision:

- **Active:** a behavior or trust assumption remains relevant; exploitability is
  not established by that label.
- **Source-remediated:** an implementation change addresses the reported behavior;
  the label does not claim a fresh test or proof.
- **Withdrawn or rejected:** the report lacks a reachable production example or
  conflicts with the inspected source.

| Concern | Disposition at the reconciliation revision | Evidence and boundary |
|---|---|---|
| A009: short timelock and ownership composition | Active deployment/configuration risk | Sensitive floor and both configured standard delays remain 12 ledgers. Controller ownership, deployed delays, router owner, and XOXNO oracle owner require independent attestation; repository config cannot prove them. |
| A048/A056: strategy output floor | Active design residual | Controller validates positive measured swap output; the quantitative route minimum remains in the aggregator payload. Loss can approach authorized swap notional, constrained by final risk checks while debt remains. This is not a demonstrated protocol-wide mint. |
| A055: non-exact or dishonest tokens | Active listing assumption | Receiver tax alone does not establish a pool custody deficit when sender debit is exact. Sender surcharges, rebases, clawbacks, and lying balances are separate risks; measured receipts cannot establish arbitrary token honesty. Do not reuse the old market-TVL ceiling as a universal bound for a hostile collateral listing. |
| A064: `no_seize` independent of entry freeze | Active design/availability residual | A seizure halt does not itself block supply. ADR-0008 keeps the flags independent; coupling them was a proposal, not a shipped audit fix. |
| A040: threshold-update batch failure | Active keeper limitation | With `has_risks=true`, a failed per-account 1.05 HF check aborts the whole batch and retains the prior risk snapshot. LTV-only refresh does not impose this final gate. This is availability behavior, not a new authorization bypass. |
| A062/A015: raw mutation and keeper vectors | Active hygiene item | Position counts are bounded and duplicate payments are aggregated; raw vector processing remains subject to transaction resource limits. No inventory extraction was established. |
| Stale collateral price; cleanup dust versus configured floor; delegate A→B→A grant reuse | Active documented limitations | Fail-closed valuation can block recovery; the cleanup threshold is compile-time while the borrow floor is configurable; a grant binds its granting owner rather than an ownership epoch. The Astra grant observation lacked a completed independent final review. |
| Same-market collateral/debt cleanup | Active loss-allocation design | Cleanup reclassifies remaining collateral as revenue and socializes gross debt. Same-market netting is not implemented; see the [proposal record](#design-proposal-records). |
| Large-value accrual overflow | Active numeric-domain limit | Historical local whale sequence reaches `MathOverflow` before the borrow-index cap and then blocks sync-first repayment/withdrawal. Raised caps, disabled utilization cap, high initial utilization, and elapsed years are part of that reproduction; current mainnet exposure was not established. |
| Supply-index loss floor | Active recovery-policy limit | The floor can preserve unpaid claims. Supply entry checks backing; recap restores the current shortfall and refunds excess without resetting the index. |
| Pool liquidation fee clipped against pre-burn supply | Source-remediated | Withdrawal at reconciliation burns shares before minting retained-fee revenue. The retained fee-headroom test asserts the complete fee entitlement. The old report's tiny revenue value describes the historical ordering. |
| FP-EDGE-01 and FP-EDGE-02 signed rounding underflows | Source-remediated | Downscaling delegates to quotient/remainder integer division; neither biases `i128::MIN` before dividing. The same change removes the historical positive `MAX/2` bias overflow and rejects negative divisors in release code. Historical pool reachability of the signed extremes was not established. |
| A080: archived/missing spoke usage permits over-admission | Withdrawn as a live production finding | Persistent archive requires restoration or fails the transaction; it does not turn an existing row into `None`. First positive entry creates usage, and current monetary merges update usage with positions. Planted missing-row tests establish tolerance only. Missing-row exits intentionally remain no-ops. |
| A007/A048 token-hook reentry into controller/pool | Withdrawn as a live production finding | Soroban rejects indirect entry into a contract already on the call stack. The historical drain narrative reports A→B→C success and A→B→A rejection; its named host probe is absent from the current checkout. Generic `is_err()` hook tests alone do not prove the controller flash flag supplied the rejection. |
| A094: missing index refresh in a future merge | Review/process item | Historical live merges refreshed mutation indexes. It was a hypothetical future regression, not a demonstrated omission. Current per-call context lives in `context.rs`; old `Cache` paths are historical. |
| Listing decimals never checked | Superseded overstatement | Governance listing validates token metadata equality and the 3–18 decimal domain. Direct owner-authorized pool construction trusts its input and accepts 0–18 decimals; that lower-level trust boundary is not proof of an ordinary-user bypass. |
| Credit fee creates unbacked supply; refund sweeps preexisting controller funds; stale LTV on gated borrow/withdraw | Rejected in the Astra review | Share-credit fees reclassify existing shares; refunds use balance differences; final risk gates refresh listed LTVs. Reopen only with a reachable current counterexample. |

## Design proposal records

ADR-0008's proposal to couple `no_seize` with `frozen` was closed without
adoption on 2026-09-05. The implemented flags remain independent; the
[halt rationale](../explanation/decisions.md#adr-0008) explains their operating cost.

ADR-0021, same-market bad-debt netting, remains proposed and deferred as of
2026-09-02. It would change cleanup accounting and event semantics. The
[source behavior](../explanation/decisions.md#adr-0021) reclassifies remaining
supply as revenue and socializes gross debt.

The [historical STRIDE matrix](https://github.com/XOXNO/rs-lending-xlm/blob/d26b93ebb48d718b69571ec737f0097af3379916/STRIDE.md)
retains its original risk ratings. The maintained threat model keeps the IDs
but does not reuse those ratings as a fresh assessment.

## Retained evidence

The following artifacts identify what a reviewer can reproduce. Immutable test
versions distinguish audit-time evidence from tests that changed afterward.

| Evidence | Availability and use |
|---|---|
| Astra [unit log](astra-2026-09-05/evidence/unit-tests.log), [integration log](astra-2026-09-05/evidence/integration-tests.log), [probe log](astra-2026-09-05/evidence/probes-final.log), [source hashes](astra-2026-09-05/evidence/source-sha256.txt) | Tracked and available in a fresh checkout. The old report incorrectly described these three logs as excluded. Hashes bind the audited source, not current HEAD. |
| [Pool arithmetic runner](pool-accounting-2026-09-05/evidence/math/run.py), [generator](pool-accounting-2026-09-05/evidence/math/generate.py), [probe](pool-accounting-2026-09-05/evidence/math/probe.rs), [commands](pool-accounting-2026-09-05/evidence/math/commands.json) | Tracked historical reproducer. The probe expects three old panics; it is **not a passing current-source regression check** after the rounding fixes. Use a scratch copy with the reviewed source revision to reproduce that record. Generated vectors and binaries were not retained. |
| [Rates equation replay](pool-accounting-2026-09-05/evidence/rates-reference.py) | Tracked Python standard-library reference. It evaluates equations independently; it does not execute the contract. |
| Pool `.log` files mentioned in the old validation record | Ignored local artifacts, present in the review checkout but unavailable in a fresh clone. They remain untouched; their original names and commands are in the immutable validation record. |
| [Astra probes](../../tests/test-harness/tests/astra_audit.rs), [pool accounting tests](../../tests/test-harness/tests/pool_money_flow_audit.rs) | Retained runnable tests. The pool fee test was updated after the audit. [Baseline Astra test](https://github.com/XOXNO/rs-lending-xlm/blob/d26b93ebb48d718b69571ec737f0097af3379916/tests/test-harness/tests/astra_audit.rs), [baseline pool test](https://github.com/XOXNO/rs-lending-xlm/blob/d26b93ebb48d718b69571ec737f0097af3379916/tests/test-harness/tests/pool_money_flow_audit.rs). These files were added after the audited source revision; the baseline pool test already includes the fee fix. |
| Controller usage and historical host reentry evidence | [Usage composition](../../tests/test-harness/tests/controller/spoke_usage_tracks_positions.rs) remains present. The [immutable drain narrative](https://github.com/XOXNO/rs-lending-xlm/blob/d26b93ebb48d718b69571ec737f0097af3379916/docs/audit/controller-defense/synthesis/DRAIN-ANALYSIS.md) reports host reentry checks, but its named `poc_host_reentrancy.rs` probe is absent from the current checkout; that check is retained as a historical report, not a currently runnable test. |

## Limits and next verification

These reviews use native Rust/Soroban host tests, often with mocked authorization
and relaxed resource budgets. They do not establish compiled-WASM budget fit,
live ownership or configuration, deployed source equality, complete dependency
security, exhaustive transaction sequences, or full protocol formal verification.
The pool review inspects controller context only to assess pool-call reachability.

At the reconciliation revision, static Certora inventory and path checks passed.
Artifact provenance failed because source hashes and the controller file count
had changed. That reconciliation included no rebuilt artifacts, prover submission,
or full replay of the historical executions. See
[Certora tuning and proof limits](../explanation/certora-sunbeam-prover-tuning.md).

Before relying on these records for a release:

1. Verify the deployment, active market parameters, and token semantics.
2. Reproduce the relevant tests against the release source.
3. Measure maximum-position liquidation with its real oracle composition.
4. Bind each formal verdict to its rebuilt artifact, configuration, assumptions,
   and source fingerprint.
