# External Audit & Incident Learnings — rs-lending-xlm

Research distilled from real lending-protocol incidents and public Soroban DeFi audits, mapped onto our Aave-faithful Soroban lending protocol to pre-empt audit findings before our own engagement.

## Why this exists

rs-lending-xlm is heading toward external audit and mainnet (ADR 0009 launch gates). Rather than wait for auditors to find the well-known lending failure modes, we mined them ourselves: every relevant incident (Aave, Morpho, Spark, Kamino+Solana, Blend) and the public Soroban DeFi audits we could locate and read (Blend, Soroswap, Aquarius, Orbit/YieldBlox, Stellar Asset Contract, + the CoinFabrik Scout detector set; Phoenix/Comet/FxDAO were targeted but no public PDF was found — logged as gaps), distilled what auditors flag and expect, and cross-referenced each lesson against our actual code.

## Artifacts

| File | Contents |
|------|----------|
| `incident-catalog.md` | Realized exploits & near-misses with losses, root cause, fix, source. |
| `vuln-taxonomy.md` | The 10 lending vuln classes, each with real-world examples drawn from the catalog. |
| `soroban-auditor-expectations.md` | Soroban-platform footguns + per-auditor methodology/checklist (RV, Certora, OtterSec, Veridise, CoinFabrik, FYEO, Certik). |
| `self-audit-backlog.md` | **The payload** — each finding mapped to our code (`file:line`) with an exposure verdict (exposed / mitigated / needs-review), priority, recommended action. |
| `variant-analysis.md` | Antipattern grep/Explore hunt across our code for each finding pattern. |

## Method

A 3-tier agent fan-out (per `.claude/plans/...cryptic-journal.md`):
- **Tier 1** — 6 domain agents (Aave / Morpho / Spark / Kamino+Solana / Blend / Soroban-ecosystem), each ran web discovery and fanned out Tier-2 readers.
- **Tier 2** — one reader per audit PDF / post-mortem; findings extracted into a common schema, every finding carrying a working `source_url`.
- **Tier 3** — merge/dedupe → knowledge base (this dir); per-subsystem code-mapping agents read our actual code and produced `self-audit-backlog.md` + `variant-analysis.md`; "mitigated" verdicts adversarially re-checked against our tests/Certora specs; completeness critic loop.

**~324 cited findings** across 6 domains fed the synthesis.

### Severity normalization

| Normalized | Meaning |
|-----------|---------|
| `crit` | Direct fund loss / protocol insolvency, low precondition. |
| `high` | Fund loss or insolvency with a precondition (specific config, race, admin compromise). |
| `med` | Value leakage, griefing/DoS, or accounting drift; or high-impact needing strong preconditions. |
| `low` | Minor leakage, edge-case revert, or quality issue with security flavor. |
| `info` | Design observation, methodology, or non-exploitable hardening note. |

### Exposure verdicts (in `self-audit-backlog.md`)

- `exposed` — our code has the antipattern; `file:line` evidence; survives adversarial refutation.
- `mitigated` — our design/code already defends; evidence + the test/Certora rule that proves it.
- `needs-review` — plausible but unconfirmed; requires a human pass.
- `n/a` — architecturally inapplicable (e.g. we have no backstop/auction module).

## Coverage & caps

- Per-domain document cap ~10–15; readers logged dropped/unreadable docs in each domain report.
- Several audit PDFs are Soroban/Rust but were initially mis-summarized by the WebFetch summarizer as EVM/Solidity (hallucinated "SafeMath", Solidity issue numbers). Readers re-parsed the real PDFs with `pdfminer.six`; **PDF findings in this corpus are from real text, not WebFetch summaries**.
- Known gaps logged: Phoenix/Comet/FxDAO had no public audit PDF located (DEX/CDP coverage retained via Soroswap/Aquarius/Orbit); some Code4rena mitigation-review pages are gated (recovered via the embedded main report).
- **Reflector's own published security model / audit was not mined** — our entire oracle-poison defense depends on Reflector's behavior, yet the corpus only studies it *as the victim's dependency* in the YieldBlox post-mortem. A direct pass over Reflector's own audit is a recommended follow-up. (DeFindex's strategy-side audit was likewise not mined, but the `defindex-strategy` contract itself was code-mapped in round 2.)

A **completeness-critic round 2** (after the initial synthesis) added the Euler/Sonne/UwU/Radiant-2024 incidents and their lessons, mapped the `defindex-strategy` contract and `services/keeper` (both 0 exposed), and qualified the oracle "100x rejected" claim (it means value-extracting flows fail *closed* when the cross-provider anchor is unavailable, not that any single read is always safe).

## Notable headline lessons

1. **Feb 2026 Blend/YieldBlox $10.8M** — a formally-verified HF invariant held *on a poisoned oracle price*. Oracle-source liquidity + on-chain deviation/staleness circuit breakers are first-class security properties, not afterthoughts.
2. **Aave CRV toxic-liquidation spiral** — a flat liquidation bonus creates a frontier above which every liquidation worsens LTV. Our per-account derived bonus ceiling is the defense; verify the math.
3. **Bad-debt socialization is fragile** (Morpho OZ-H01, Blend M-03/M-17) — leave-1-wei to skip socialization; 1-unit round-up underflow reverts cleanup. Maps directly to our known stuck-bad-debt issue.
4. **Soroban-platform class dominates audits** — unbounded instance storage, `require_auth` auth-tree phishing, i128 rounding/overflow, TTL/eviction, instant-upgrade timelock bypass, resource-limit-bricks-liquidation.
