# XOXNO Lending — Integration Skills

Agent skills for integrating or building on XOXNO Lending (Stellar Soroban).
Each skill is a how-to an agent loads on demand, grounded in contract ABIs and
`@xoxno/sdk-js`. Addresses and RPC endpoints come from configuration — never
from these docs.

Canonical protocol rules: [docs/reference/invariants.md](../docs/reference/invariants.md).
Topology: [docs/reference/architecture.md](../docs/reference/architecture.md).
Shared model: [lending-protocol-fundamentals](./lending-protocol-fundamentals/SKILL.md).

| Layer | Skill | Use it for |
|---|---|---|
| Shared | [lending-protocol-fundamentals](./lending-protocol-fundamentals/SKILL.md) | Hubs/spokes/accounts, units, HF, address discipline |
| On-chain (Rust) | [integrating-lending-from-soroban-contracts](./integrating-lending-from-soroban-contracts/SKILL.md) | Cross-contract supply/borrow/withdraw/repay |
| On-chain (Rust) | [writing-flash-loan-receivers](./writing-flash-loan-receivers/SKILL.md) | `execute_flash_loan` receivers |
| Views | [reading-lending-protocol-state](./reading-lending-protocol-state/SKILL.md) | HF, positions, rates, indexes, caps |
| Off-chain (TS) | [using-lending-sdk](./using-lending-sdk/SKILL.md) | Tx builders, strategies, REST reads |
| Off-chain (TS) | [building-lending-liquidation-bots](./building-lending-liquidation-bots/SKILL.md) | Detection, estimation, execution, bonus curve |
| Off-chain (TS) | [indexing-lending-events](./indexing-lending-events/SKILL.md) | Event decode for indexers |

## Installing

Skills follow the [Agent Skills](https://agentskills.io/specification) format
(`SKILL.md` with YAML frontmatter).

```bash
mkdir -p .claude/skills
cp -R path/to/rs-lending-xlm/skills/*/ .claude/skills/
```

Ship the whole set — layer skills assume `lending-protocol-fundamentals`.
When the ABI or SDK changes, re-verify the affected skill against the docs
above and the code.
