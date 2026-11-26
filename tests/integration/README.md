# E2E integration

E2E tests deploy candidate contracts to Stellar testnet and drive live
transactions. They run only on demand, either locally or with the manual
`E2E Integration` workflow (`.github/workflows/e2e.yml`). Release Build and
pull requests run the offline harness, RPC-fixture, and operator regressions.

## Running the suite

### Prerequisites

You need Stellar CLI 28.0.0, Node 24, Python 3, jq, curl, xxd, and GNU timeout
(`gtimeout` on macOS). Never enable PYTHONOPTIMIZE. Validation uses assertions.

### Commands

```bash
# Production contracts and fixtures have separate output directories.
make integration-wasm candidate-size-check
npm ci --ignore-scripts --prefix tests/integration/sdk
make integration-validate integration-sdk-validate ops-script-check

# Required controlled-ledger evidence for this candidate.
set -o pipefail
cargo test --workspace --no-fail-fast 2>&1 | tee controlled-tests.log
python3 tests/integration/controlled.py controlled-tests.log artifacts/wasm/deploy

# Every release lane, each in a fresh world; default caps: 95m per lane, 150m CI job.
NETWORK=testnet RUN_TS="local-$(date +%Y%m%d-%H%M%S)" \
  bash tests/integration/scenarios/parallel_e2e.sh

# Focused smoke; not a full run.
NETWORK=testnet RUN_TS="liq-$(date +%Y%m%d-%H%M%S)" E2E_LANES="liq-a liq-b liq-c" \
  bash tests/integration/scenarios/parallel_e2e.sh

python3 tests/integration/gate.py tests/integration/runs/<base>-liq-a
python3 tests/integration/release_gate.py collect tests/integration/runs <base>
```

### Environment knobs

| Variable | Default | Effect |
|---|---|---|
| `NETWORK` | `testnet` | Target network |
| `RUN_TS` | required | Unique run name and directory |
| `E2E_LANES` | `RELEASE_LANES` | Lanes `parallel_e2e.sh` starts |
| `E2E_LANE` | per script | Lane of a standalone scenario script |
| `LANE_TIMEOUT` | `95m` | Per-lane timeout in `parallel_e2e.sh` |
| `E2E_RPC_SLOTS` | `12` | Concurrent network processes, all lanes |
| `E2E_RPC_READ_SLOTS` | `6` | Concurrent simulation-only processes |
| `E2E_SLOT_DIR` | `/tmp/rs-lending-e2e-slots-<uid>` | Slot pool, shared by one user's runs on one host |
| `THROTTLE_RETRIES` | `6` (0 to 20) | Retries after 429, 5xx or dropped connection |
| `E2E_LANE_STAGGER` | `60` (0 to 9999) | Seconds before non-critical lanes start |
| `E2E_LANE_GAP` | `10` (0 to 999) | Seconds between lane launches |
| `E2E_RESUME` | `0` | `1` reuses an existing local run ID |

### RPC budget

All lanes share one RPC budget. `env.sh` puts `bin/stellar` first on `PATH`.

- Each network process takes one `E2E_RPC_SLOTS` slot. These processes are
  `configs/script.sh` calls, JSON-RPC `curl`, and SDK RPC processes
  (`sdk/invoke.mjs`, `sdk/balances.mjs`).
- Simulation-only calls first take an `E2E_RPC_READ_SLOTS` slot. These calls are
  `--send=no`, `--build-only`, `tx simulate`, `contract fetch`, `fees stats`,
  `ledger`, read-only `curl`, and `sdk/balances.mjs`.
- These calls take no slot: local verbs (`keys`, `xdr`,
  `tx sign|hash|decode|encode|op`, `contract id|alias`, `version`) and the
  orchestrator's one pre-lane `sdk/limits.mjs` call.

Slots cap processes, not requests. A CLI view makes 2 to 3 requests in ~0.4 s,
and a send makes ~6 in ~4 s. 6 read slots plus 6 sends give ~40 requests/s at
median latency. The gateway allows ~65/s per IP. Measure per-second requests in a live
run before raising either default. Waits of 1 s or more go to
`runs/<run>/rpc-wait.tsv`.

### Throttling and retries

- A read that gets HTTP 429 backs off with jitter, up to `THROTTLE_RETRIES`
  times. The wait is the gateway's `retry_after` (max 120 s). Without
  `retry_after`, the waits are 8, 16, 32, then 60 s.
- The same backoff applies to HTTP 5xx or a dropped connection on read-only
  JSON-RPC. It also applies to a `stellar` command that fails with 429, 5xx, or
  a lost connection before the command prints `Signing transaction:`.
- `tx send`, `keys fund`, and `--fund` never retry. A signed send that gets 429
  stays `UNKNOWN` and fails.

`parallel_e2e.sh` starts `prod-full`, `prod-caller`, and `stress` first, and the
other lanes `E2E_LANE_STAGGER` seconds later. It launches lanes `E2E_LANE_GAP`
seconds apart, so no two lanes fund in the same second.

### Run hygiene

- Do not edit scripts during a live run (Bash may read the rest later). Use an
  immutable checkout or copied harness for concurrent work.
- GitHub run IDs include `run_attempt`.
- `E2E_RESUME=1` reuses a local run ID. Interrupted cases, unknown submissions,
  and completed-case manifest drift cannot resume as fresh work. Use a new run ID
  after a code fix.
- Standalone scenarios use the full lane manifest and gate (omitted cases fail
  coverage). Each scenario covers one lane. `full_e2e.sh` defaults to
  `agg-core`, and `flash_position.sh` defaults to `flash-a`. `E2E_LANE`
  overrides the default. `make integration-flash-position` runs `flash-a`, then
  `flash-b`.

### Reports

| Artifact | Meaning |
|---|---|
| `actions.tsv`, `report.md` | action columns, status, hash, resources, reason |
| `cases.tsv`, `evidence.tsv` | terminal case ranges. Transaction, simulation, and assertion classification |
| `attempts.jsonl`, `summary.json` | immutable attempt log paths, exit status, hash, receipt state, counts, interruption |
| `metadata.json`, `candidate.json`, `deployed-artifacts.jsonl` | source, config, and case hashes, selected cases, network, tools, and SDK policy, bytecode identity |
| `network-limits.json` | testnet and mainnet protocol, ledger, version, and resource limits |
| `controlled.json`, `controlled-tests.log` | selected controlled-clock proofs. Exact hashed test log |
| `logs/` | quotes, XDR, simulations, receipts, attempt stdout/stderr, financial snapshots |
| `before-cleanup.jsonl`, `cleanup-funding.tsv` | pre-teardown state. Separately recorded repairs and top-ups |
| `private/` | lane-specific CLI identities. **Never upload** |

CI uploads detailed evidence on failure. `state.env` is local execution state.
Reports omit private keys.

### Resource budgets

Live reports carry no budget numbers. Scenario scripts append
[`appendix.md`](appendix.md) to each `report.md`. The appendix names the native
tests that print the budget numbers.

## Release gate

### Release order

1. Build the canonical artifacts.
2. Run the offline and contract checks.
3. Publish those exact files.

Release Build runs no live lanes. Before a deploy, run `E2E Integration` with
`lanes=all` on the release tag. That run builds the same commit again. It does
not test the published files. A passing smoke or mapped ABI is not release
acceptance.

- Builds consume `stellar contract build --optimize --out-dir` output. Cargo's
  target WASM stays raw. Production artifacts are optimized once, then
  stripped. Fixtures use their own output directory.
- `distribution.json` records the hash of every other published file. These
  files are production and SDK WASM, checksums, and SDK and candidate manifests.
- Publication runs `artifacts.py distribution-check` and rejects missing,
  extra, or changed files.
- `release_gate.py` binds a local full run to a packaged bundle. No workflow
  runs it.

### Governance fixture

Candidate `governance.wasm` delays sensitive operations 120960 ledgers (~7
days). This delay is longer than any run, so lanes deploy
`fixtures/governance_e2e.wasm` and prove a same-hash upgrade of it.
`make integration-wasm` builds the fixture with the `short-timelock` feature and
a 12-ledger floor. Only native tests cover the candidate. The fixture's
`timelock` contract-meta stamp fails `artifacts.py check` on any candidate
governance. The failure blocks an accidental deploy or publish.

### Lanes

| Lane | Required coverage |
|---|---|
| `agg-core` | lifecycle, routed strategies, fees, admin upgrade |
| `agg-admin` | same-market settlement, NFT, flash loans, admin, gap-hunt admin, pool entrypoints |
| `agg-gov` | risk refresh, swap-aggregator admin, governance |
| `liq-a` | Transfer/Credit liquidation, spoke liquidation, credit rejections, spoke flags and curve |
| `liq-b` | multi-hub isolation, bulk liquidation, exact bad debt/recap |
| `liq-c` | deprecated-spoke credit, two-vault and contract-vault DeFindex; oracle history/quorum |
| `stress` | 5 collateral + 5 debt positions, dual sources, Transfer/Credit max liquidation, five-asset exits, composed providers, delayed submission |
| `flash-a` | callback success, protected balances, Long/multiple collateral, delegates and rollback snapshots, malicious receiver |
| `flash-b` | callback success, create-path and dual-path rejections, strategy-mode and flash-loan gates |
| `blend` | run-deployed Blend pool setup, allowlist and reserve addresses; six XLM paths, distinct-token and multi-liability migration; committed-rate shares, refunds, identity, unrelated balances |
| `prod-full` | governance operator setup and replay; full enabled-mainnet policy readback; 5 same-hash upgrades with a live contract-owned position on a book with a repaid borrow; readback; governance band, flags, pause after upgrade |
| `prod-caller` | same operator, filtered caller config: 8 markets + reference BTC, `configs/mainnet` policies; 7/8/9/18-decimal round trips; Chainlink + RedStone USDT0; XOXNO-backed borrowing; contract caller and authority chain |
| `sdk` | supply/borrow/repay/withdraw, routed multiply, Blend, events/error mapping/delayed signing |

The lanes run in these environments. `agg-*` uses live Reflector and quote
routes and a fresh candidate aggregator. `liq-*` uses isolated SACs and oracle
fixtures. `stress` uses independent provider fixtures and deterministic
shared-account interleaving. `flash-*` uses live Reflector and existing receiver
fixtures. `blend` and `sdk` use a fresh Blend v2 pool per run from the pinned
[`blend_pool.wasm`](fixtures/blend/README.md) with a mock backstop. `prod-*`
uses disposable policy and wallet roots and explicit provider, LP, and token
fixtures for every enabled mainnet market (`prod-caller` uses
`production_config.py --only caller`). `sdk` uses published SDK 1.0.221 and
Stellar SDK 16.3.0, with fresh contracts.

#### Lane details

Production replay rules:

- Provider registry roles map through the market-policy fixture address map.
  The operator validator keeps its provider checks.
- Fresh setup configures reference and token oracles before market creation.
  Dependent oracle execution follows market configuration order.
- A failed operator command keeps its known signing hashes and fetches their
  receipts. The original failure is preserved.

### Case and ABI coverage

- `cases.json` lists required terminal cases and action predicates by contract
  role and execution type. Its `lanes` field picks each lane's cases.
  `run_case` skips the rest. `test_release_gate.py` pins the map.
- `abi-coverage.json` maps every candidate export, including generated NFT
  methods and constructors, to required actions or justified controlled tests.
  Nested-call mappings are source-traced, not host traces. `abi_coverage.py`
  compares actual WASM exports.

A required case cannot pass through `research`, `sim-exceeded`,
`environment-blocked`, empty report, missing actions, unknown status or
duplicate case IDs. Diagnostics cannot erase a failed assertion, even under the
same label. Expected negatives distinguish simulation rejection from a
submitted failed receipt. Simulated rollback does not prove committed
rollback.

### Transaction rules

- Mutations use `--send=yes`. Reads use `--send=no`.
- CLI calls use native `--instruction-leeway 20000000` for instruction growth
  from index projection and accrual between simulation and inclusion. No fixed
  margin covers every future state.
- The 10% resource headroom gate still applies. SDK-prepared envelopes are not
  patched.
- Retry only classified transient failures without a signed hash, plus the
  Reflector round race. Once a hash exists, reconcile it. Never rebuild a
  mutation with unknown outcome.
- The harness recovers a lost successful CLI response only after it verifies
  the signed envelope, the host operation, and the receipt return or event hash.
  The original CLI status and output stay in attempt evidence.

A submitted Trapped/ResourceLimitExceeded failure is fatal, except the
Reflector round race. In that race, the committed FAILED receipt shows the testnet Reflector
CEX contract reading its own round key outside the footprint. `inv` and
`sdk_inv` then rebuild the call once and record the failure as `retry` with
`rejected_transaction` evidence. `gate.py` requires the next same-label action
to succeed with these properties:

- the same source, contract, function, and arguments,
- an unchanged read-write footprint,
- a read-only footprint differing only by Reflector round entries, including
  the missing one (`receipts.py footprint_drift`).

Every other storage-footprint failure stays fatal.

### Funding swaps

- A checkout-wide file lock spans quote acquisition through receipt
  confirmation.
- Quotes request `platform=aggregator` (Soroban composition). Snapshots must
  reach the RPC ledger seen after taking the lock.
- A cancelled or unresolved funding operation leaves
  `runs/.external-funding.<RUN_TS>.pending.json`. Later funding with that
  RUN_TS (same lane, or its `E2E_RESUME`) fails before quoting. Other lanes and
  new runs continue under the lock. Reconcile the operation before removing the
  marker.

Unrelated external traders and the deliberate contention tests are outside
this control. SDK evidence includes native simulation responses and resources and
failed wire receipts. The harness collects this evidence without altering
prepared envelopes.

### Resource gate

- The gate checks signed envelope declarations, actual transaction and event
  bytes, footprint counts, and both captured network limit sets.
- Declared instructions, including CLI leeway, keep at least 10% headroom.
- Success proves the testnet memory cap. A lower mainnet cap fails closed.
- Mainnet access is read-only limit capture, never submission.

### Evidence arithmetic

- Native XLM deltas include actual committed network fees.
- Raw token and share arithmetic uses integers, including 18-decimal values.
- Exact rounding predicates use committed indexes, not later projected ones.
- Full liquidation closes distinguish ceiling-rounded cash payment from
  floor-valued debt-share credit. Principal floors in WAD before conversion to
  RAY.
- Exact balance and share assertions remain required.

### Controlled time

`controlled-cases.json` labels cases that need a controlled ledger clock.
These cases are long-idle interest, HF boundaries, governance expiry and
recovery, dust-close footprint stability, account TTL, and host auto-restore.
Native tests prove them. Testnet cannot observe them over weeks.

- The long-idle test compares paths that share the accrual routine, not
  independent reference arithmetic.
- `controlled.py` checks native test names and results and stamps the supplied
  candidate metadata. It does not attest which WASM each test loaded (native
  or raw release image). The curator image is outside the stamp.
- `integration-sdk-validate` tests native RPC restore preparation, refreshed
  source sequence, and malformed simulations offline. It claims nothing about
  live archival, restoration, or automatic restore by the published SDK.

### Acceptance

- A full `E2E Integration` run must show every required case on fresh worlds.
  An ABI map, offline regression, or partial smoke cannot certify them.
- Production fixture checks preserve enabled policy, caps, fees, rates,
  decimals, hub and spoke relationships, and composition, but not real
  external-provider availability.
- Upgrade evidence records identical baseline and candidate controller hashes,
  `executable_differs=false`, and survival of controller, pool, NFT,
  price-aggregator, governance, and oracle history. The evidence makes no claim
  about storage migration from a different executable.
- Release dispatch defaults to `dry_run=true`. A dry run builds and runs offline
  checks, and it does not publish.
- Local `release_gate.py` regressions inject failed lanes and wrong artifacts.
- The harness pins published SDK 1.0.221. It checks all four ESM and CJS root
  and lending entrypoints for the native 20M leeway request, returned resources
  and fee, unsigned output, and unchanged invocation.
- Error mapping is checked offline and live. Local SDK substitutions are
  forbidden.

## Extending

- Use `run_case` and the existing `inv`, `view`, `xfail`, `run_deploy`, and
  assertion helpers.
- Propagate prerequisites explicitly. A log message is not a failed case.
- Add required predicates with a new flow. Update ABI bindings.
- Add a focused offline fault injection proving a wrong outcome cannot pass.
- Never reduce required stress dimensions after failure.
- Inspect conservation before cleanup. Minting or top-ups stay separately
  attributable.
