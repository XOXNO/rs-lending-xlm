# XOXNO Lending — agent skills

Agent skills for building on XOXNO Lending. Each `SKILL.md` routes a task to
companion references loaded on demand, following the
[Agent Skills specification](https://agentskills.io/specification).

Use directories named `xoxno-*` for integration tasks. Reserve `evals/` for
testing skill behavior.

Start at [xoxno-lending/SKILL.md](xoxno-lending/SKILL.md), select one task
skill, and load only the companion reference named for the task.

| Skill | Load when you are… | Companion files |
|---|---|---|
| [xoxno-lending](xoxno-lending/SKILL.md) | Starting any XOXNO task; need addresses, ids, or formulas | `addresses.md` (generated), `math.md` |
| [xoxno-lending-contracts](xoxno-lending-contracts/SKILL.md) | Writing a Soroban contract that supplies, borrows, holds a position, or receives a flash loan | `abi.md`, `positions.md`, `flash-loans.md`, `composing.md` |
| [xoxno-lending-sdk](xoxno-lending-sdk/SKILL.md) | Building a dApp, backend, or bot in TypeScript on `@xoxno/sdk-js` and `api.xoxno.com` | `reads.md`, `positions.md`, `transactions.md`, `strategies.md`, `frontend.md` |
| [xoxno-swap-aggregator](xoxno-swap-aggregator/SKILL.md) | Quoting or executing swaps, or embedding a swap payload in a lending action or your own contract | `api.md`, `payload.md`, `composition.md` |
| [xoxno-lending-liquidations](xoxno-lending-liquidations/SKILL.md) | Building a liquidation bot, keeper, or risk monitor | — |
| [xoxno-lending-data](xoxno-lending-data/SKILL.md) | Indexing events, building analytics, or calling the REST API from any language | `api.md` |
| [xoxno-lending-troubleshooting](xoxno-lending-troubleshooting/SKILL.md) | Decoding a failed simulation or transaction, or setting up testnet | — |

## Installing

```bash
# Claude Code / any agent that reads ~/.claude/skills
mkdir -p ~/.claude/skills
cp -R path/to/rs-lending-xlm/skills/xoxno-* ~/.claude/skills/

# or, from the published repository
npx skills add https://github.com/XOXNO/rs-lending-xlm
```

Ship the whole set: every skill assumes `xoxno-lending` is available.

## Source of truth and verification

- Contract claims are verified against this repository (`interfaces/`, `common/`,
  `contracts/`, `docs/reference/`). `scripts/check_doc_symbols.py` flags selected
  backticked Rust-looking identifiers absent from its source/config text and
  allowlists; it does not resolve symbols. It skips `xoxno-lending-data/` and
  `xoxno-swap-aggregator/`, which document services in other repositories.
- `xoxno-lending/addresses.md` is generated: `python3 scripts/gen_skill_addresses.py`
  (`--check` fails when it is stale). Never edit it by hand.
- SDK claims are verified against the separate `sdk-js` repository at the version named in
  the skill; API claims against `xoxno-api-v2` (production behind `api.xoxno.com`);
  aggregator claims against `arb-algo`. Re-check them after a release of any of those.
- `evals/scenarios/<skill>/` holds task scenarios in the stellar-dev-skill format, each
  encoding a mistake an agent makes without the skill. Run them before publishing a change.

## Contributing

A skill change is complete when its affected `evals/` scenario is updated,
every `SKILL.md` remains under about 450 lines, and task-specific depth is
disclosed through a companion file from the routing table.
