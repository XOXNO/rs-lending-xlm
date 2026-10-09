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
| [xoxno-lending-contracts](xoxno-lending-contracts/SKILL.md) | Writing or testing a Soroban contract that supplies, borrows, holds a position, or receives a flash loan | `abi.md`, `positions.md`, `flash-loans.md`, `composing.md`, `testing.md` |
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
- `xoxno-lending-contracts` claims about the `xoxno-contract-sdk` crate are verified against
  its public repository at the crate version named in that skill.
- Verify SDK claims against the named `sdk-js` release. Verify API claims
  against `xoxno-api-v2`. Verify aggregator claims against `arb-algo`. Re-check them after a release of any of those.
- `evals/scenarios/<skill>/` holds task scenarios in the stellar-dev-skill format, each
  encoding a mistake an agent makes without the skill. Run them before publishing a change.

## Contributing

A skill change is complete when its affected `evals/` scenario is updated,
every `SKILL.md` remains under about 450 lines, and task-specific depth is
disclosed through a companion file from the routing table.

## Writing rules

Apply the ASD-STE100 writing principles throughout each skill and companion
reference. The skills use the full editorial profile. The public Mintlify
docs use a separate, selective profile.

1. Use one term for one technical meaning.
2. Use active voice and direct instructions.
3. Give one action per procedure step.
4. Put conditions before the action they control.
5. Keep procedural sentences at 20 words or fewer where possible.
6. Keep descriptive sentences at 25 words or fewer where possible.
7. Split long explanations into short paragraphs, lists, or tables.
8. Preserve exact code names, types, units, and error namespaces.
9. Describe the current interface only. Remove superseded examples and paths.
10. Verify safety conditions against committed source and published exports.

Technical names are necessary vocabulary. Code and formulas keep their exact
syntax. These rules do not certify compliance with the official ASD-STE100
dictionary; that dictionary was not available for this review.
