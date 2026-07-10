# XOXNO Lending — Integration Skills

Agent skills for developers integrating or building on the XOXNO Lending
protocol (Stellar Soroban). Each skill is a self-contained reference an AI
coding agent loads on demand, grounded in the contract ABIs and the
`@xoxno/sdk-js` SDK. Skills are network-generic: addresses, RPC endpoints,
and network selection always come from configuration, never from the docs.

Start with the shared model, then pick the layer you build at:

| Layer | Skill | Use it for |
|---|---|---|
| Shared | [lending-protocol-fundamentals](./lending-protocol-fundamentals/SKILL.md) | Architecture, hubs/spokes/accounts, units, HF semantics, address/config discipline |
| On-chain (Rust) | [integrating-lending-from-soroban-contracts](./integrating-lending-from-soroban-contracts/SKILL.md) | Your contract supplies/borrows/withdraws/repays via cross-contract calls |
| On-chain (Rust) | [writing-flash-loan-receivers](./writing-flash-loan-receivers/SKILL.md) | `execute_flash_loan` receiver contracts |
| Views (any caller) | [reading-lending-protocol-state](./reading-lending-protocol-state/SKILL.md) | Health factor, positions, rates, indexes, caps — controller and pool views |
| Off-chain (TS) | [using-lending-sdk](./using-lending-sdk/SKILL.md) | Transaction builders, leverage/swap strategies + quote server, REST reads |
| Off-chain (TS) | [building-lending-liquidation-bots](./building-lending-liquidation-bots/SKILL.md) | Liquidation detection, estimation, execution, bonus curve |
| Off-chain (TS) | [indexing-lending-events](./indexing-lending-events/SKILL.md) | Consuming and decoding contract events for indexers and analytics |

## Installing

Skills follow the [Agent Skills](https://agentskills.io/specification) format
(`SKILL.md` with YAML frontmatter) and work with any harness that supports it,
including Claude Code.

- **Claude Code (per project)**: copy the skill directories into your
  project's `.claude/skills/`.
- **Claude Code (global)**: copy them into `~/.claude/skills/`.

```bash
# from your project root
mkdir -p .claude/skills
cp -R path/to/rs-lending-xlm/skills/*/ .claude/skills/
```

Agents discover skills by their frontmatter `description`; no further wiring
is needed. Ship the whole set — the layer skills cross-reference
`lending-protocol-fundamentals` by name.

## Ground truth

Signatures and semantics mirror:

- Contract ABIs: [`interfaces/`](../interfaces) (controller, pool, governance
  client traits) and [`contracts/`](../contracts) sources
- SDK: [`@xoxno/sdk-js`](https://github.com/XOXNO/sdk-js) `stellar-lending`
  subpath
- Deployments: [`configs/networks.json`](../configs/networks.json) (the only
  place addresses live)

When the protocol ABI or SDK changes, re-verify the affected skill against
those sources.
