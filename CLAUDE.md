# CLAUDE.md — rs-lending-xlm

XOXNO Lending: invariant-critical Stellar Soroban contracts, plus formal
proofs, fuzzers, and deployment tooling. Mainnet and testnet deployments are
listed in `configs/networks.json`.

Read [CONTRIBUTING.md](CONTRIBUTING.md) for the human working agreement and the
per-surface evidence table. This file holds only what an agent must know that
the other documents do not say.

## Workspace map

| Path | Contents |
|---|---|
| `contracts/` | Deployable contracts: controller, pool, governance, position-nft, price-aggregator, swap-aggregator, defindex-strategy, xoxno-oracle |
| `interfaces/` | `#[contractclient]` declarations. Six of the eight contracts. `defindex-strategy` has none (it consumes `interfaces/controller`); the `xoxno-oracle` client lives in `common/src/oracle/providers/xoxno.rs` with the other oracle providers |
| `common/` | Shared math, rates, oracle, types, TTL, errors, validation |
| `mock/` | Test doubles: mock-oracle, mock-redstone, flash-loan-receiver, flash-position-receiver, script-runner |
| `tests/test-harness/` | Integration tests. Sub-suites: controller, pool, governance, oracle, strategy, composition, fuzz, meta |
| `tests/fuzz/` | Separate workspace. Excluded from the root workspace |
| `certora/` | Sunbeam specs. Each spec tree is mounted **into** its crate (controller, pool, price-aggregator, common) — see Traps |
| `skills/` | Published integration skill set for downstream consumers; router at `skills/xoxno-lending/SKILL.md`. Not dev tooling |
| `docs/` | Reference, decision records (ADR-0001..0021), threat model, runbooks |
| `vendor/` | Patched `cvlr-soroban` and `cvlr-log`. See the comment in `Cargo.toml` |
| `services/` | Keeper and lending exporter. Separate Cargo workspaces; `make test` does not cover them |

## Commands

`make help` is the index. `make help-build`, `help-verify`, `help-deploy`,
`help-ops`, `help-views`, `help-oracle`, `help-aggregator`, `help-all` are the
topic pages. There are 133 targets — read the index before you invent a
command.

Check ladder, narrowest first:

    make fmt-check                      # cargo fmt --all -- --check
    make access-control-check           # source-only, no build, about 1 second
    make docs-check                     # doc links + doc symbols
    make test-match PATTERN=<substring> # focused harness tests
    make clippy                         # --all-targets -D warnings
    make test                           # whole workspace

Heavier evidence, only for the surface that needs it: `make miri-common`,
`make fuzz FUZZ_TIME=30`, `make proptest PROPTEST_CASES=256`,
`make mutants-<area>`, `make scout`, `make certora-wasm`, `make certora`.

## Traps

**`make test-match` needs `PATTERN=`.** `MATCH=` is not recognised, and the
target fails on an empty `PATTERN`. A target without such a guard ignores a
variable it does not read, so a misnamed filter runs the entire suite.

**This repo is driven from zsh.** Write `--include='*.rs'` with quotes.
Unquoted, zsh fails the command with `no matches found`.

**Controller internal module paths are load-bearing.** `contracts/controller/src/lib.rs`
mounts the Certora spec with `#[path = "../../../certora/controller/spec/mod.rs"]`,
and the spec calls internals by exact path (`crate::positions::process_borrow`,
`crate::positions::supply::process_supply`). Unit tests mount the same way from
`contracts/controller/tests/` and call `pub(crate)` items positionally. A
rename here edits spec files and invalidates tuned prover CI budgets. Verify
the out-of-crate consumers before you move or narrow anything.

**Never write an inline fully-qualified call.** Not
`crate::external::position_nft::nft_mint_call(env, ...)`, not
`common::ttl::renew_instance(e)`. Import at the top of the file, then call bare
or module-qualified. Inline paths hide a file's dependency surface.

**The codebase-memory graph stops at every contract boundary.** Its `CALLS`
edges do not cross Soroban contracts, and the 3 `Route` nodes are git
dependency URLs, not endpoints. `trace_path(mode='cross_service')` and
`cross-repo-intelligence` return empty here — that means "wrong tool", not "no
cross-contract calls". Cypher also rejects `NOT f.is_test`; write
`f.is_test = false`. Use the `endpoint-tracer` agent instead; it encodes the
boundary dictionary and the verified query set.

**Certora `cvlr-soroban` is vendored on purpose.** Upstream still pins
`soroban-sdk` 26.1.0, which puts two majors in the graph and breaks the
`certora` feature. Do not remove the `[patch]` block until upstream moves
to 27.

## Units

Keep the unit boundary explicit. Never mix these in one expression:

| Unit | Used for |
|---|---|
| token amount | transfers, balances, caps |
| WAD | USD values, health factor |
| RAY | shares, indexes, rates |
| BPS | ratios, fees, bonuses |

Rounding direction must favour the protocol invariant, not whichever operation
is convenient. See [ADR-0003](docs/explanation/decisions.md#adr-0003).

## Gates that block a PR

- `make access-control-check` — every `#[contractimpl]` entrypoint is gated or
  declared in `scripts/permissionless_entrypoints.txt`. A stale or over-broad
  declared line fails as loudly as a missing one. New permissionless entrypoint
  means a new justified line in that file.
- `make fmt-check`, `make docs-check`, `make integration-validate` — the
  `static-gates` job in `.github/workflows/tests.yml`.
- `make ops-script-check` — same job. `configs/script.sh` must map a config
  spoke id to its on-chain id (`onchain:N` is the raw escape) and must carry
  the live `paused`/`frozen`/`no_seize` flags through a listing edit. Offline,
  about 1 second.
- `make wasm-size-check`, `make wasm-testing-abi-check` — testing-only
  entrypoints must not exist in a deployable artifact ([ADR-0017](docs/explanation/decisions.md#adr-0017)).

`tests.yml` and `security.yml` start only when their `paths` filter matches. A
change limited to `docs/`, `skills/`, `configs/` or a root Markdown file starts
neither, so run these gates locally.

## Where to read before changing behaviour

| Change | Document |
|---|---|
| Accounting, risk, liquidation arithmetic | [docs/reference/formulas.md](docs/reference/formulas.md) |
| Properties that must hold | [docs/reference/invariants.md](docs/reference/invariants.md) |
| Trust boundaries and threats | [docs/explanation/threat-model.md](docs/explanation/threat-model.md) |
| Why a design is the way it is | [docs/explanation/decisions.md](docs/explanation/decisions.md) |
| Numeric domains and limits | [docs/reference/formulas.md#numeric-limits](docs/reference/formulas.md#numeric-limits) |
| Errors and events | [docs/reference/errors.md](docs/reference/errors.md), [docs/reference/events.md](docs/reference/events.md) |
| Formal verification | [certora/README.md](certora/README.md) |

## Agents and skills in this repo

| Name | Use it for |
|---|---|
| `endpoint-tracer` | Ground-truth cross-contract trace of an endpoint. Call it **before** any bug hunt |
| `soroban-invariant-auditor` | Judge a traced path against protocol invariants |
| `certora-spec-writer` | Write or repair a CVL rule |
| `test-reviewer` | Review test quality after writing tests |
| `/verify` | Run the check ladder against the current diff |
| `/deploy-preflight` | Read-only pre-deploy audit. Never run a deploy without it |

## Do not

- Do not claim a check passed without running it. Report the exact command and
  its output.
- Do not weaken a lint, test, or gate to make verification pass.
- Do not commit hunks you did not write. Other sessions share this checkout —
  run `git diff` and stage by path.
- Do not touch `configs/`, run any `deploy-*` or `upgrade-*` target, or spend
  a signer without the user asking in this session.
