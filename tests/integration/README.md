# E2E release gate

End-to-end tests that deploy the candidate contracts to Stellar testnet and
drive them through live transactions. Live runs are manual or release-only.
Pull requests run the offline harness, RPC-fixture and operator regressions.

## Running the suite

### Prerequisites

- Stellar CLI 28.0.0, Node 24, Python 3, jq, curl, xxd and GNU timeout
  (`gtimeout` on macOS).
- Never enable PYTHONOPTIMIZE: validation uses assertions.

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
| `RUN_TS` | required | Unique run name; names the run directory |
| `E2E_LANES` | `RELEASE_LANES` | Lanes `parallel_e2e.sh` starts |
| `E2E_LANE` | per script | Lane of a standalone scenario script |
| `LANE_TIMEOUT` | `95m` | Per-lane timeout in `parallel_e2e.sh` |
| `E2E_RPC_SLOTS` | `12` | Network processes that may run at once, across all lanes |
| `E2E_RPC_READ_SLOTS` | `6` | Simulation-only processes that may run at once |
| `E2E_SLOT_DIR` | `/tmp/rs-lending-e2e-slots-<uid>` | Slot pool directory, shared by every run of one user on one host |
| `THROTTLE_RETRIES` | `6` (0 to 20) | Retries after HTTP 429, 5xx or a dropped connection |
| `E2E_LANE_STAGGER` | `60` (0 to 9999) | Seconds before the non-critical lanes start |
| `E2E_LANE_GAP` | `10` (0 to 999) | Seconds between two lane launches |
| `E2E_RESUME` | `0` | `1` allows reuse of an existing local run ID |

### RPC budget

All lanes share one RPC budget. `env.sh` puts `bin/stellar` first on `PATH`.

- Each network CLI call holds one `E2E_RPC_SLOTS` slot. This includes
  `configs/script.sh` calls, each JSON-RPC `curl`, and each SDK RPC process
  (`sdk/invoke.mjs`, `sdk/balances.mjs`).
- Simulation-only calls first take one `E2E_RPC_READ_SLOTS` slot. These are
  `--send=no`, `--build-only`, `tx simulate`, `contract fetch`, `fees stats`,
  `ledger`, read-only `curl` and `sdk/balances.mjs`.
- Local verbs take no slot: `keys`, `xdr`, `tx sign|hash|decode|encode|op`,
  `contract id|alias`, `version`.
- The orchestrator's single `sdk/limits.mjs` call runs before any lane and
  takes no slot.

A slot caps processes, not requests:

- A CLI view makes 2 to 3 requests in about 0.4 s.
- A CLI send makes about 6 requests in about 4 s.
- 6 read slots and 6 sends give about 40 requests per second at median
  latency. The gateway limit is about 65 per second from one IP.

Before you raise either slot default, record per-second request counts in a
live run. Each wait of 1 s or more is logged to `runs/<run>/rpc-wait.tsv`.

### Throttling and retries

- A read that gets HTTP 429 backs off with jitter. It waits the gateway's
  `retry_after` (at most 120 s), or 8, 16, 32 and then 60 s.
- It retries at most `THROTTLE_RETRIES` times.
- The same backoff covers HTTP 5xx and a dropped connection on a read-only
  JSON-RPC call.
- It also covers a `stellar` command that fails with 429, 5xx or a lost
  connection before it prints `Signing transaction:`.
- `tx send`, `keys fund` and `--fund` are never retried. A signed send that
  gets 429 stays `UNKNOWN` and fails.

`parallel_e2e.sh` starts `prod-full`, `prod-caller` and `stress` first. The
other lanes start `E2E_LANE_STAGGER` seconds later. Every other launch waits
`E2E_LANE_GAP` seconds after the previous one, so lanes do not fund in the same
second.

### Run hygiene

- Do not edit scripts during a live run: Bash may read their remaining contents
  later. Use an immutable checkout or a copied harness for concurrent work.
- GitHub run IDs include `run_attempt`.
- To reuse a local run ID, set `E2E_RESUME=1`. Interrupted cases, unknown
  submissions and completed-case manifest drift cannot resume as fresh work.
- After you fix code, use a new run ID.
- Standalone scenarios use the same complete lane manifest and gate. Omitting
  cases produces incomplete coverage.
- A standalone run covers one lane: `full_e2e.sh` defaults to `agg-core` and
  `flash_position.sh` to `flash-a`. Set `E2E_LANE` to pick another.
- `make integration-flash-position` runs `flash-a`, then `flash-b`.

### Reports

| Artifact | Meaning |
|---|---|
| `actions.tsv`, `report.md` | original action columns, status, hash, resources and reason |
| `cases.tsv`, `evidence.tsv` | terminal case ranges and transaction/simulation/assertion classification |
| `attempts.jsonl`, `summary.json` | immutable attempt log paths, exit status, hash, receipt state, counts and interruption |
| `metadata.json`, `candidate.json`, `deployed-artifacts.jsonl` | source/config/case hashes, selected cases, network/tools/SDK policy and bytecode identity |
| `network-limits.json` | testnet/mainnet protocol, ledger, version and resource limits |
| `controlled.json`, `controlled-tests.log` | selected controlled-clock proofs and exact hashed test log |
| `logs/` | quotes, XDR, simulations, receipts, attempt stdout/stderr and financial snapshots |
| `before-cleanup.jsonl`, `cleanup-funding.tsv` | state before teardown and separately recorded repairs/top-ups |
| `private/` | lane-specific CLI identities; **never upload** |

CI uploads detailed evidence on failure. `state.env` is local execution state.
Reports omit private key material.

### Resource budgets

Live reports carry no budget numbers. `tests/integration/appendix.md` points
at the native tests that print them, and the scenario scripts append it to
each `report.md`.

```bash
# Per-call CPU and memory, with CPU split by host cost type.
cargo test -p test-harness --test meta budget_breakdown -- --nocapture
# Ledger footprint sizes against the mainnet limits.
cargo test -p test-harness --test meta footprints_fit_mainnet_limits -- --nocapture
```

## Release gate

### Release order

1. Build the canonical artifacts.
2. Run the offline and contract checks.
3. Run every live lane in `RELEASE_LANES`.
4. Publish those exact files.

A passing smoke or a mapped ABI is not release acceptance.

- Builds use `stellar contract build --optimize --out-dir` and consume that
  optimized output. Cargo's target WASM stays raw.
- Production artifacts are optimized once, then stripped. Fixtures keep their
  separate output directory.
- Release packaging records all 26 published files in `distribution.json` and
  its E2E proof: production/SDK WASM, checksum files, SDK/candidate manifests,
  and the distribution manifest itself.
- Publication rejects missing, extra or changed files, including a substituted
  distribution manifest.
- Standalone local proof collection without the packaged bundle cannot
  authorize publication.

### Governance fixture

Governance is the one contract a live run does not take from the candidate set.

- The candidate `governance.wasm` holds every sensitive operation for 120960
  ledgers (about seven days). No run can wait that out.
- `make integration-wasm` also builds `fixtures/governance_e2e.wasm` with the
  governance `short-timelock` feature. It lowers that floor to 12 ledgers, and
  every lane deploys it.
- The governance upgrade proof is a same-hash upgrade of that build.
- The candidate `governance.wasm` is covered by the native tests only.
- The fixture carries a `timelock` contract-meta stamp. `artifacts.py check`
  fails on a candidate governance that carries it, so it cannot be deployed or
  published by mistake.

### Lanes

| Lane | Required surface | Environment |
|---|---|---|
| `agg-core` | lifecycle, routed strategies, fees, admin upgrade | live Reflector and quote routes; fresh candidate aggregator |
| `agg-admin` | same-market settlement, NFT, flash loans, admin, gap-hunt admin, pool surface | live Reflector and quote routes; fresh candidate aggregator |
| `agg-gov` | risk refresh, swap-aggregator admin, governance | live Reflector and quote routes; fresh candidate aggregator |
| `liq-a` | Transfer/Credit liquidation, spoke liquidation, credit rejections, spoke flags and curve | isolated SACs and oracle fixtures |
| `liq-b` | multi-hub isolation, bulk liquidation, exact bad debt/recap | isolated SACs and oracle fixtures |
| `liq-c` | deprecated-spoke credit, two-vault and contract-vault DeFindex; oracle history/quorum | isolated SACs and oracle fixtures |
| `stress` | five collateral plus five debt positions, dual sources, Transfer/Credit maximum liquidation, five-asset exits, composed providers, delayed submission | independent provider fixtures; deterministic shared-account interleaving |
| `flash-a` | callback success, protected balances, Long/multiple collateral, delegates and rollback snapshots, malicious receiver | live Reflector; existing receiver fixtures |
| `flash-b` | callback success, create-path and dual-path rejections, strategy-mode and flash-loan gates | live Reflector; existing receiver fixtures |
| `blend` | real pool addresses, XLM and multi-liability migration, committed-rate shares and refunds; see [Lane details](#lane-details) | real Blend TestnetV2 pool |
| `prod-full` | governance operator setup and replay, mainnet policy readbacks, same-hash upgrades; see [Lane details](#lane-details) | disposable policy/wallet roots; explicit provider/LP/token fixtures for every enabled mainnet market |
| `prod-caller` | the same operator on the filtered caller config; see [Lane details](#lane-details) | disposable policy/wallet roots; `production_config.py --only caller` fixtures |
| `sdk` | supply/borrow/repay/withdraw, routed multiply, Blend, events/error mapping/delayed signing | published SDK 1.0.221 and Stellar SDK 16.3.0; fresh contracts |

#### Lane details

- `blend`: actual pool allowlist and reserve addresses; six XLM paths plus
  distinct-token and multiple-liability migration; committed-rate shares,
  refunds, identity and unrelated balances.
- `prod-full`: governance operator setup and replay; full enabled mainnet
  policy readbacks; 5 same-hash upgrades with a live contract-owned position on
  a book with a repaid borrow; the full policy readback; then governance band,
  flags and pause on the upgraded contracts.
- `prod-caller`: 8 markets and reference BTC, with policies from
  `configs/mainnet`; 7/8/9/18 decimal round trips; Chainlink + RedStone USDT0;
  XOXNO-backed borrowing; contract caller and authority chain.

Production replay rules:

- Provider registry roles map through the same fixture address map as market
  policies. The operator validator keeps its provider checks.
- Fresh setup configures the reference and token oracles before market
  creation. Dependent oracle execution follows the market configuration order.
- A failed operator command keeps its known signing hashes and fetches their
  receipts. The original command failure is preserved.

### Case and ABI coverage

- `cases.json` defines required terminal cases and action predicates,
  qualified by contract role and execution type.
- Its `lanes` field selects the cases each lane runs. `run_case` skips the
  others, and `test_release_gate.py` pins the map.
- `abi-coverage.json` maps all candidate exports, including generated NFT
  methods and constructors, to required actions or justified controlled tests.
- Nested-call mappings are source-traced, not runtime host call traces.
  `abi_coverage.py` compares actual WASM exports.

A required case cannot pass via:

- `research`, `sim-exceeded` or `environment-blocked`;
- an empty report, missing actions or unknown status;
- duplicate case IDs.

Diagnostics cannot erase a failed assertion, even with the same label. Expected
negative cases distinguish simulation rejection from a submitted failed
receipt. A simulation rollback check is not a committed-transaction rollback
proof.

### Transaction rules

- Mutations use `--send=yes`; reads use `--send=no`.
- CLI calls use native `--instruction-leeway 20000000`. This covers the
  instruction growth from index projection and accrual between simulation and
  inclusion, with margin. A fixed margin does not cover every future state
  transition.
- The 10% resource headroom gate still applies. SDK-prepared envelopes are not
  patched.
- Retry only classified transient failures without a signed hash, plus the
  Reflector round race below.
- After a hash exists, reconcile that hash. Never rebuild the mutation while
  its outcome is unknown.
- If the CLI loses a successful response, recovery verifies the signed
  envelope, host operation and receipt return/event hash before it recovers the
  result. The original CLI status and output stay in attempt evidence.

A submitted Trapped/ResourceLimitExceeded failure is fatal, except for the
Reflector round race. In that race, the committed FAILED receipt shows the
testnet Reflector CEX contract reading one of its own round keys outside the
footprint.

For that case only, `inv` and `sdk_inv` rebuild the same call once. They record
the failed transaction as `retry` with `rejected_transaction` evidence.
`gate.py` then requires the next action with the same label to be a successful
transaction with:

- the same source, contract, function and arguments;
- an unchanged read-write footprint;
- a read-only footprint that differs only by Reflector round entries,
  including the missing one (`receipts.py footprint_drift`).

Every other storage-footprint failure stays fatal.

### Funding swaps

- Funding swaps share a checkout-wide file lock from quote acquisition through
  receipt confirmation.
- Quotes request `platform=aggregator` for Soroban composition.
- Quote snapshots must reach the RPC ledger observed after the lock is taken.
- A cancelled or unresolved funding operation leaves
  `runs/.external-funding.<RUN_TS>.pending.json`. Later funding with that
  RUN_TS (the same lane, or an `E2E_RESUME` of it) fails before quoting.
- Other lanes and new runs continue. The checkout-wide lock still serializes
  their quotes.
- Reconcile the recorded operation before you remove that marker.

This coordination does not control unrelated external traders or the
deliberate contention tests. SDK evidence includes native simulation
responses/resources and failed wire receipts. The harness observes them without
changing prepared envelopes.

### Resource gate

- The gate checks signed envelope declarations, actual transaction/event bytes,
  footprint counts, and both captured network limit sets.
- Declared instructions, including CLI leeway, must keep at least 10% headroom.
- Successful execution proves the testnet memory cap. A lower mainnet memory
  cap fails closed.
- Mainnet access is read-only limit capture, never transaction submission.

### Evidence arithmetic

- Native XLM financial deltas account for actual committed network fees.
- Raw token/share arithmetic uses integers, including 18-decimal values.
- Exact rounding predicates use committed indexes, not later projected view
  indexes.
- Full liquidation closes distinguish ceiling-rounded cash payment from
  floor-valued debt-share credit. Principal floors in WAD before conversion to
  RAY.
- Exact balance and share assertions remain required.

### Controlled time

`controlled-cases.json` labels the cases that need a controlled ledger clock:
long-idle interest, HF boundaries, governance expiry and recovery, dust-close
footprint stability, account TTL and host auto-restore.

- Native tests prove them; testnet does not observe them over weeks.
- The long-idle test compares paths that share the accrual routine. It is not
  independent reference arithmetic.
- `controlled.py` checks native test names and results and stamps the supplied
  candidate metadata. It does not attest which WASM each test loaded.
- Loaders may use native contracts or raw release images. The curator image is
  outside this stamp.
- `integration-sdk-validate` tests native RPC restore preparation, a refreshed
  source sequence and malformed simulations offline. It makes no claim about
  live archival, restoration, or automatic restore orchestration by the
  published SDK.

### Acceptance

- A release run must demonstrate every required case on fresh worlds. An ABI
  map, an offline regression, or a partial smoke cannot certify them.
- Production fixture checks preserve enabled policy, caps, fees, rates,
  decimals, hub/spoke relationships and composition. Fixture-backed checks do
  not prove real external-provider availability.
- Upgrade evidence records identical baseline and candidate controller hashes
  and `executable_differs=false`. It checks that controller, pool, NFT,
  price-aggregator, governance and oracle history survive the upgrade. It makes
  no claim about storage migration from a different executable.
- Release dispatch defaults to `dry_run=true`, so a branch can run the complete
  gate without publication. A dry run still requires every live lane.
- `inject_e2e_failure=true` stops the E2E job before deployment and must leave
  publication skipped. Local publication regressions inject failed lanes and
  wrong artifacts.
- The harness pins published SDK 1.0.221. It checks all four published ESM/CJS
  root and lending entrypoints for the native 20M leeway request, returned
  resources and fee, unsigned output, and unchanged invocation.
- Error mapping is checked offline and live. Local SDK substitutions are
  forbidden.

## Extending

- Use `run_case` and the existing `inv`, `view`, `xfail`, `run_deploy` and
  assertion helpers.
- Propagate prerequisites explicitly; a log message is not a failed case.
- Add required predicates alongside a new flow.
- Update ABI bindings.
- Add a focused offline fault injection proving a wrong outcome cannot pass.
- Never reduce required stress dimensions after failure.
- Inspect conservation before cleanup; minting or top-ups must remain
  separately attributable.
