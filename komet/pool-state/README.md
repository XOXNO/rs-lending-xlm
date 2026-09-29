# Actual pool supply transition

This harness links the production `pool` crate and calls
`LiquidityPoolInterface` directly. Production operations are neither copied
nor replaced. Calls execute in the harness contract's frame and storage:
**same-frame/direct-trait**, not an independently deployed pool contract.

`init` calls the real pool constructor and creates a real empty market. Komet
invokes `init` in a separate frame before each test. Keeping creation separate
from supply also matches native authorization behavior: repeated owner
authorization within one frame otherwise fails with `Auth/ExistingValue`.

## Claim

`test_supply_exact(amount)` executes production supply for an empty position
and market, with asset decimals 7, both indexes initially `RAY`, zero interest
rates, and an unchanged ledger timestamp. Its explicit domain is:

```text
0 < amount <= floor(i128::MAX / 10^20)
ledger timestamp <= floor(u64::MAX / 1000)
```

Inputs outside that domain return `true` as an implication premise. Within
it, the expected scaled amount uses ordinary integer arithmetic independently
of production fixed-point helpers:

```text
expected_scaled = amount * 10^20
```

The claim checks the initialized empty state; one returned mutation with the
exact position, actual amount, decimals and indexes; persisted supply equal
to `expected_scaled`; cash equal to `amount`; and unchanged debt, revenue,
indexes and timestamp. `test_supply_one_unit()` fixes `amount = 1` and returns
`false` when the timestamp premise is invalid, so it cannot pass by skipping
an excluded-input branch.

## Assumptions and limits

- Authorization is assumed. Komet's `require_auth` is modeled as a no-op;
  native tests use `mock_all_auths`.
- The controller has already transferred the tokens and reports the empty
  user position truthfully. Production supply performs bookkeeping without
  calling the token contract. The harness uses its own address for the owner
  and dummy asset identity; no token contract is installed. The user-position
  assertion checks the returned mutation, not the controller's user storage.
- The claim starts from production market creation, not arbitrary storage.
  It does not prove arbitrary-state induction, accrual, borrowing, token
  backing, authorization, child-contract isolation, rollback or budgets.
- The successful Komet one-unit witness establishes one concrete transition.
  Native boundary cases and Wasm call inspection supply additional evidence;
  no symbolic pool-state proof is claimed.

## Reproduce

Run from the repository root:

```sh
RUSTC_WRAPPER='' cargo test --locked --manifest-path komet/pool-state/Cargo.toml
cargo fmt --manifest-path komet/pool-state/Cargo.toml -- --check
RUSTC_WRAPPER='' stellar contract build --locked \
  --manifest-path komet/pool-state/Cargo.toml --optimize=false \
  --out-dir /tmp/pool-state-wasm

KPROFILE_TELEMETRY_DISABLED=true \
KOMET_SRC=/tmp/komet-sdk28 \
KOMET_CACHE=/tmp/komet-word-concat-cache \
JAVA_TOOL_OPTIONS='-XX:-MaxFDLimit -XX:ActiveProcessorCount=4' \
/tmp/komet-python -m komet test \
  --wasm /tmp/pool-state-wasm/pool_state_proof.wasm \
  --id test_supply_one_unit --max-examples 1
```

The repaired CLI (`1ec8db3`) passes the one-unit witness using the installed
KRun interface. The deliberately false half-two control fails, and unknown
stopped executions are reported as execution errors. The earlier raw-fuzz
invocation aborted with `Called finish_rewriting with no output file specified`;
that preserved failure was a tool invocation error, not a contract assertion.

Stateful target repair `02f48eb` is accepted and independently reviewed. Its
final target permits changes to the four transaction world-state cells:
`contracts`, `accounts`, `contractData` and `contractCodes`. The concrete
initial state, input constraints, exact `ScBool(true)` assertion, successful
exit and terminal execution cells remain fixed. The harness's exact cash/share
postconditions are unchanged. At that commit, 17 focused cases and all 148
unit tests passed; the actual successful terminal matches the repaired target,
and the actual false-assertion control does not.

The earlier formal run incorrectly required persistent storage to equal its
initial state. It was interrupted and invalidated; its checkpoint is preserved
under `overconstrained-proof/`, with exact target comparisons under
`rejected-stateful-target/`.

**Formal verification remains incomplete.** Two fresh runs with the repaired
target were deliberately interrupted for performance: the original full Wasm
after approximately 23.3 minutes, and an independently reviewed smaller Wasm
after 26.1 minutes. Both final checkpoints contain only initial and target
nodes, with no execution edges. The smaller run's sampled physical footprint
reached 23.7 GB; no symbolic speedup was demonstrated. These interruptions are
not property failures or completed proofs. Concrete witness and target-repair
checks remain separate evidence.

## Recorded evidence

Generated artifacts are kept outside the source tree at:

```text
/Users/mihaieremia/.codex/visualizations/2026/09/26/01a0ded9-99d4-7d02-bbb7-d1cb3fcfd966/komet-pool-math-proof/pool-state
```

Replay the recorded Wasm with its pinned hash:

```sh
pool_state_evidence=/Users/mihaieremia/.codex/visualizations/2026/09/26/01a0ded9-99d4-7d02-bbb7-d1cb3fcfd966/komet-pool-math-proof/pool-state
KPROFILE_TELEMETRY_DISABLED=true \
KOMET_SRC=/tmp/komet-sdk28 \
KOMET_CACHE=/tmp/komet-word-concat-cache \
JAVA_TOOL_OPTIONS='-XX:-MaxFDLimit -XX:ActiveProcessorCount=4' \
/tmp/komet-python "$pool_state_evidence/replay.py" \
  "$pool_state_evidence/pool_state_proof.wasm" /tmp/pool-state-replay
```

The replay prints semantic exit and terminal-cell summaries; success requires
exit 0 and empty program, instructions and stacks as in the recorded result.

- Native boundary test: four fresh markets; amounts `1`, `10_000_000`,
  `MAX_SUPPLY - 1`, `MAX_SUPPLY`. See `native-tests.txt`.
- `pool_state_proof.wasm` is built without `certora` features;
  `pool_state_proof.wat` retains the wrapper call to the linked
  production `<pool::LiquidityPool as pool_interface::LiquidityPoolInterface>::supply`.
- `source-manifest.json` pins compiler, Wasm hash and source hashes.
- Komet's frontend filters selected test bindings before decoding unrelated
  pool endpoint types. No production Wasm/spec sections were removed.
- `direct-diagnostic/result.json` and `direct-diagnostic/terminal-summary.json`
  record successful production initialization and the one-unit witness:
  process exit 0, semantic exit 0, empty program/instruction/call stacks after
  a call expecting `ScBool(true)`. `direct-diagnostic/terminal.kore` retains the
  terminal configuration.
- `komet-one-unit-cli.txt` and `fuzz-diagnostic/` preserve the CLI interpreter
  invocation failure described above. `komet-one-unit-binding-failure.txt`
  preserves the earlier ABI-discovery blocker; neither is a contract failure.
- `stateful-target-repair/REPORT.md` records accepted commit `02f48eb`, source
  review, 148 passing unit tests, exact old/new target comparisons, actual
  terminal matching, and rejection of an old proof checkpoint after the target
  changed. These checks do not establish a backend symbolic proof.
- `full-wasm-performance-interruption/` and `rooted-proof/` retain both repaired
  target runs, including final checkpoints, logs, exact recorder exits, input
  fingerprints and interruption records. `rooted-proof/` also retains the
  resource profile. No pool proof remains running from these attempts.
- The smaller candidate's derivation and independent original-instruction
  identity review are in sibling `booster-native-stack/state-export-dce/`.
  It preserves selected reachable production/harness operations and assertions;
  it is a separate proof-performance artifact, not a deployment replacement.

The accepted frontend repair's CLI logs, source, commands and independent
controls are retained in the sibling
`booster-native-stack/frontend-binding-and-fuzz/` evidence directory.
