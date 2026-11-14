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

## Verified against

The version pins for the SDK, the off-chain services, and the contract crate
are in one place:
[xoxno-lending/SKILL.md](xoxno-lending/SKILL.md#verified-against).

## Source of truth and verification

- `scripts/check_doc_symbols.py` flags backticked Rust-style identifiers that
  are absent from source, configuration, and its allowlists. It does not
  resolve symbols.
- The symbol check skips `xoxno-lending-data/` and `xoxno-swap-aggregator/`.
  Those skills document off-chain services.
- `xoxno-lending/addresses.md` is generated. Regenerate it with
  `python3 scripts/gen_skill_addresses.py`. The `--check` flag fails when the
  file is stale. Never edit it by hand.
- `evals/scenarios/<skill>/` holds task scenarios in the stellar-dev-skill
  format. Each scenario encodes a mistake that an agent makes without the
  skill. Run them before you publish a change.

## Contributing

A skill change is complete when these conditions are true:

- The affected `evals/` scenario is updated.
- Every `SKILL.md` stays under about 450 lines.
- Task-specific depth lives in a companion file named in the routing table.

Apply the [writing rules](../CONTRIBUTING.md#writing-rules) to every skill and
companion reference. The skills use the full editorial profile.
