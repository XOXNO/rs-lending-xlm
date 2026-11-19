# E2E release gate

E2E tests deploy candidate contracts to Stellar testnet and drive live
transactions, manually or at release. Pull requests run the offline harness,
RPC-fixture and operator regressions.

## Running the suite

### Prerequisites

Stellar CLI 28.0.0, Node 24, Python 3, jq, curl, xxd, GNU timeout (`gtimeout`
on macOS). Never enable PYTHONOPTIMIZE: validation uses assertions.

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

# Focused smoke; does not satisfy the full release gate.
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

- `E2E_RPC_SLOTS` slot per network process: `configs/script.sh` calls,
  JSON-RPC `curl`, SDK RPC processes (`sdk/invoke.mjs`, `sdk/balances.mjs`).
- Simulation-only calls first take an `E2E_RPC_READ_SLOTS` slot: `--send=no`,
  `--build-only`, `tx simulate`, `contract fetch`, `fees stats`, `ledger`,
  read-only `curl`, `sdk/balances.mjs`.
- No slot: local verbs (`keys`, `xdr`, `tx sign|hash|decode|encode|op`,
  `contract id|alias`, `version`) and the orchestrator's one pre-lane
  `sdk/limits.mjs` call.

Slots cap processes, not requests: a CLI view makes 2 to 3 requests in ~0.4 s,
a send ~6 in ~4 s. 6 read slots plus 6 sends give ~40 requests/s at median
latency; the gateway allows ~65/s per IP. Measure per-second requests in a live
run before raising either default. Waits of 1 s or more go to
`runs/<run>/rpc-wait.tsv`.

### Throttling and retries

- A read hitting HTTP 429 backs off with jitter, up to `THROTTLE_RETRIES`
  times: the gateway's `retry_after` (max 120 s), else 8, 16, 32, then 60 s.
- Same backoff for HTTP 5xx or a dropped connection on read-only JSON-RPC, and
  for a `stellar` command failing with 429, 5xx or lost connection before it
  prints `Signing transaction:`.
- `tx send`, `keys fund` and `--fund` never retry. A signed send hitting 429
  stays `UNKNOWN` and fails.

`parallel_e2e.sh` starts `prod-full`, `prod-caller` and `stress` first, the
rest `E2E_LANE_STAGGER` s later, launches `E2E_LANE_GAP` s apart so lanes never
fund in the same second.

### Run hygiene

- Do not edit scripts during a live run (Bash may read the rest later). Use an
  immutable checkout or copied harness for concurrent work.
- GitHub run IDs include `run_attempt`.
- `E2E_RESUME=1` reuses a local run ID; interrupted cases, unknown submissions
  and completed-case manifest drift cannot resume as fresh work. New run ID
  after a code fix.
- Standalone scenarios use the full lane manifest and gate (omitted cases fail
  coverage) and cover one lane: `full_e2e.sh` defaults to `agg-core`,
  `flash_position.sh` to `flash-a`; `E2E_LANE` overrides.
  `make integration-flash-position` runs `flash-a`, then `flash-b`.

### Reports

| Artifact | Meaning |
|---|---|
| `actions.tsv`, `report.md` | action columns, status, hash, resources, reason |
| `cases.tsv`, `evidence.tsv` | terminal case ranges; transaction/simulation/assertion classification |
| `attempts.jsonl`, `summary.json` | immutable attempt log paths, exit status, hash, receipt state, counts, interruption |
| `metadata.json`, `candidate.json`, `deployed-artifacts.jsonl` | source/config/case hashes, selected cases, network/tools/SDK policy, bytecode identity |
| `network-limits.json` | testnet/mainnet protocol, ledger, version, resource limits |
| `controlled.json`, `controlled-tests.log` | selected controlled-clock proofs; exact hashed test log |
| `logs/` | quotes, XDR, simulations, receipts, attempt stdout/stderr, financial snapshots |
| `before-cleanup.jsonl`, `cleanup-funding.tsv` | pre-teardown state; separately recorded repairs/top-ups |
| `private/` | lane-specific CLI identities; **never upload** |

CI uploads detailed evidence on failure. `state.env` is local execution state.
Reports omit private keys.

### Resource budgets

Live reports carry no budget numbers. Scenario scripts append
[`appendix.md`](appendix.md), naming the native tests that print them, to each
`report.md`.

## Release gate

### Release order

1. Build the canonical artifacts.
2. Run the offline and contract checks.
3. Run every live lane in `RELEASE_LANES`.
4. Publish those exact files.

A passing smoke or mapped ABI is not release acceptance.

- Builds consume `stellar contract build --optimize --out-dir` output; Cargo's
  target WASM stays raw. Production artifacts: optimized once, then stripped.
  Fixtures: own output directory.
- `distribution.json` and its E2E proof record all 26 published files:
  production/SDK WASM, checksums, SDK/candidate manifests, the distribution
  manifest itself.
- Publication rejects missing, extra or changed files, including a substituted
  manifest. Local proof collection without the packaged bundle cannot
  authorize publication.

### Governance fixture

Candidate `governance.wasm` delays sensitive operations 120960 ledgers (~7
days), longer than any run, so lanes deploy `fixtures/governance_e2e.wasm`
(built by `make integration-wasm`, `short-timelock` feature, 12-ledger floor)
and prove a same-hash upgrade of it. Only native tests cover the candidate. The
fixture's `timelock` contract-meta stamp fails `artifacts.py check` on any
candidate governance, blocking accidental deploy or publish.

### Lanes

| Lane | Required surface |
|---|---|
| `agg-core` | lifecycle, routed strategies, fees, admin upgrade |
| `agg-admin` | same-market settlement, NFT, flash loans, admin, gap-hunt admin, pool surface |
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

Environments: `agg-*` live Reflector and quote routes, fresh candidate
aggregator; `liq-*` isolated SACs and oracle fixtures; `stress` independent
provider fixtures, deterministic shared-account interleaving; `flash-*` live
Reflector, existing receiver fixtures; `blend` and `sdk` a fresh Blend v2 pool per run from the pinned
[`blend_pool.wasm`](fixtures/blend/README.md) with a mock backstop;
`prod-*` disposable policy/wallet roots and explicit provider/LP/token fixtures
for every enabled mainnet market (`prod-caller`: `production_config.py --only
caller`); `sdk` published SDK 1.0.221 and Stellar SDK 16.3.0, fresh contracts.

#### Lane details

Production replay rules:

- Provider registry roles map through the market-policy fixture address map;
  the operator validator keeps its provider checks.
- Fresh setup configures reference and token oracles before market creation;
  dependent oracle execution follows market configuration order.
- A failed operator command keeps its known signing hashes and fetches their
  receipts, preserving the original failure.

### Case and ABI coverage

- `cases.json`: required terminal cases and action predicates by contract role
  and execution type. Its `lanes` field picks each lane's cases; `run_case`
  skips the rest; `test_release_gate.py` pins the map.
- `abi-coverage.json`: every candidate export, including generated NFT methods
  and constructors, mapped to required actions or justified controlled tests.
  Nested-call mappings are source-traced, not host traces. `abi_coverage.py`
  compares actual WASM exports.

A required case cannot pass via `research`, `sim-exceeded`,
`environment-blocked`, empty report, missing actions, unknown status or
duplicate case IDs. Diagnostics cannot erase a failed assertion, even under the
same label. Expected negatives distinguish simulation rejection from a
submitted failed receipt; simulated rollback does not prove committed
rollback.

### Transaction rules

- Mutations `--send=yes`; reads `--send=no`.
- CLI calls use native `--instruction-leeway 20000000` for instruction growth
  from index projection and accrual between simulation and inclusion; no fixed
  margin covers every future state.
- The 10% resource headroom gate still applies. SDK-prepared envelopes are not
  patched.
- Retry only classified transient failures without a signed hash, plus the
  Reflector round race. Once a hash exists, reconcile it; never rebuild a
  mutation with unknown outcome.
- A lost successful CLI response is recovered only after verifying signed
  envelope, host operation and receipt return/event hash. Original CLI status
  and output stay in attempt evidence.

A submitted Trapped/ResourceLimitExceeded failure is fatal, except the
Reflector round race: the committed FAILED receipt shows the testnet Reflector
CEX contract reading its own round key outside the footprint. Then `inv` and
`sdk_inv` rebuild the call once and record the failure as `retry` with
`rejected_transaction` evidence. `gate.py` requires the next same-label action
to succeed with:

- the same source, contract, function and arguments;
- an unchanged read-write footprint;
- a read-only footprint differing only by Reflector round entries, including
  the missing one (`receipts.py footprint_drift`).

Every other storage-footprint failure stays fatal.

### Funding swaps

- A checkout-wide file lock spans quote acquisition through receipt
  confirmation.
- Quotes request `platform=aggregator` (Soroban composition); snapshots must
  reach the RPC ledger seen after taking the lock.
- A cancelled or unresolved funding operation leaves
  `runs/.external-funding.<RUN_TS>.pending.json`; later funding with that
  RUN_TS (same lane, or its `E2E_RESUME`) fails before quoting. Other lanes and
  new runs continue under the lock. Reconcile the operation before removing the
  marker.

Unrelated external traders and the deliberate contention tests are outside
this control. SDK evidence includes native simulation responses/resources and
failed wire receipts, observed without altering prepared envelopes.

### Resource gate

- Checks signed envelope declarations, actual transaction/event bytes,
  footprint counts, and both captured network limit sets.
- Declared instructions, including CLI leeway, keep at least 10% headroom.
- Success proves the testnet memory cap; a lower mainnet cap fails closed.
- Mainnet access is read-only limit capture, never submission.

### Evidence arithmetic

- Native XLM deltas include actual committed network fees.
- Raw token/share arithmetic uses integers, including 18-decimal values.
- Exact rounding predicates use committed indexes, not later projected ones.
- Full liquidation closes distinguish ceiling-rounded cash payment from
  floor-valued debt-share credit. Principal floors in WAD before conversion to
  RAY.
- Exact balance and share assertions remain required.

### Controlled time

`controlled-cases.json` labels cases needing a controlled ledger clock:
long-idle interest, HF boundaries, governance expiry and recovery, dust-close
footprint stability, account TTL, host auto-restore. Native tests prove them;
testnet cannot observe them over weeks.

- The long-idle test compares paths sharing the accrual routine, not
  independent reference arithmetic.
- `controlled.py` checks native test names and results and stamps the supplied
  candidate metadata; it does not attest which WASM each test loaded (native
  or raw release image). The curator image is outside the stamp.
- `integration-sdk-validate` tests native RPC restore preparation, refreshed
  source sequence and malformed simulations offline; it claims nothing about
  live archival, restoration or automatic restore by the published SDK.

### Acceptance

- A release run must show every required case on fresh worlds; an ABI map,
  offline regression or partial smoke cannot certify them.
- Production fixture checks preserve enabled policy, caps, fees, rates,
  decimals, hub/spoke relationships and composition, but not real
  external-provider availability.
- Upgrade evidence records identical baseline and candidate controller hashes,
  `executable_differs=false`, and survival of controller, pool, NFT,
  price-aggregator, governance and oracle history. No claim about storage
  migration from a different executable.
- Release dispatch defaults to `dry_run=true`: full gate, every live lane, no
  publication.
- `inject_e2e_failure=true` stops E2E before deployment; publication must be
  skipped. Local publication regressions inject failed lanes and wrong
  artifacts.
- The harness pins published SDK 1.0.221; all four ESM/CJS root and lending
  entrypoints are checked for the native 20M leeway request, returned resources
  and fee, unsigned output, unchanged invocation.
- Error mapping is checked offline and live; local SDK substitutions are
  forbidden.

## Extending

- Use `run_case` and the existing `inv`, `view`, `xfail`, `run_deploy` and
  assertion helpers.
- Propagate prerequisites explicitly; a log message is not a failed case.
- Add required predicates with a new flow; update ABI bindings.
- Add a focused offline fault injection proving a wrong outcome cannot pass.
- Never reduce required stress dimensions after failure.
- Inspect conservation before cleanup; minting or top-ups stay separately
  attributable.
