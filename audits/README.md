# Audits

| Auditor | Report | Scope | Audited commit | Engagement |
|---|---|---|---|---|
| Runtime Verification | [2026-11-runtime-verification.pdf](2026-11-runtime-verification.pdf) | `common/` and eight contracts: pool, controller, price aggregator, swap aggregator, position NFT, XOXNO oracle, governance, DeFindex strategy | `a2486b255b974ef8432ce44de014931f51552f4b` (`v1.1.0`) | 2026-09-30 to 2026-11-13 |

## Runtime Verification findings

| ID | Finding | Severity | Status |
|---|---|---|---|
| S-01 | A withdraw amount of zero closes the market and can leave an account at its borrowing limit | Low | `<status>` |
| S-02 | LTV-only restamp on a loosened listing leaves a position's LTV and LT inconsistent | Low | Fixed in a2b98a97 |
| S-03 | A borrowable market with a 100% utilization cap can be frozen until the pool is upgraded | Medium | Fixed in 127335ec |
| S-04 | A market whose supply value passes about 1.7e11 whole units freezes on the next accrual | Low | Fixed in 127335ec |
| I-01 | Delegate grants survive an NFT transfer and reactivate when it returns | Informative | Acknowledged |
| I-02 | The optional `executor` argument and the EXECUTOR role restrict nothing | Informative | Fixed in d504a4f8 |
| I-03 | Endpoints take key-value vectors where a map would rule out duplicate keys | Informative | `<status>` |
| I-04 | Proposal salts separate pending operations; they do not prevent re-proposal | Informative | `<status>` |
| I-05 | Governance role-change events name the owner as the caller | Informative | `<status>` |
| I-06 | Blend pools and position managers can only be revoked through the timelock | Informative | `<status>` |
| I-07 | In-band sentinel values give ordinary numbers a second meaning | Informative | `<status>` |
| I-08 | The rate-model validation lets the cap flatten the curve below optimal utilization | Informative | Fixed in ba70f9d2 |

Fix commits cite the finding ID in their message. The
[Aeneas proofs](../verification/aeneas/README.md) cover findings S-03, S-04
and I-08.
