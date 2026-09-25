# E2E release gate

Live execution is manual or release-only, on Stellar testnet. PRs run offline
harness, RPC-fixture and operator regressions. Release ordering is canonical
build → offline/contract checks → seven live lanes → publication of those exact
files. A passing smoke or a mapped ABI is not release acceptance.

Builds use `stellar contract build --optimize --out-dir` and consume that
optimized output. Cargo's target WASM remains raw. Production artifacts are
optimized once, then stripped; fixtures retain their separate output directory.
Removing the old second optimization pass changes some artifact hashes, so
earlier live proofs do not certify the new build outputs.

Release packaging records all 26 published files in `distribution.json` and its
E2E proof: production/SDK WASM, checksum files, SDK/candidate manifests, and the
distribution manifest itself. Publication rejects missing, extra or changed
files, including a substituted distribution manifest. Standalone local proof
collection without the packaged bundle cannot authorize publication.

## Run

Requires Stellar CLI 28.0.0, Node 24, Python 3, jq, curl, xxd and GNU timeout
(`gtimeout` on macOS). Never enable PYTHONOPTIMIZE: validation uses assertions.

```bash
# Production contracts and fixtures have separate output directories.
make integration-wasm candidate-size-check
npm ci --ignore-scripts --prefix tests/integration/sdk
make integration-validate integration-sdk-validate ops-script-check

# Required controlled-ledger evidence for this candidate.
set -o pipefail
cargo test --workspace --no-fail-fast 2>&1 | tee controlled-tests.log
python3 tests/integration/controlled.py controlled-tests.log artifacts/wasm/deploy

# Seven fresh independent worlds; default caps: 95m per lane, 150m CI job.
NETWORK=testnet RUN_TS="local-$(date +%Y%m%d-%H%M%S)" \
  bash tests/integration/scenarios/parallel_e2e.sh

# Focused smoke; does not satisfy the full release gate.
NETWORK=testnet RUN_TS="liq-$(date +%Y%m%d-%H%M%S)" E2E_LANES=liq \
  bash tests/integration/scenarios/parallel_e2e.sh

python3 tests/integration/gate.py tests/integration/runs/<base>-liq
python3 tests/integration/release_gate.py collect tests/integration/runs <base>
```

Do not edit scripts during a live run: Bash may read their remaining contents
later. Use an immutable checkout or a copied harness for concurrent development.
GitHub run IDs include `run_attempt`. Reusing a local ID requires explicit
`E2E_RESUME=1`; interrupted cases, unknown submissions and completed-case
manifest drift cannot be resumed as fresh work. Use a new ID after fixing code.
Standalone scenarios use the same complete lane manifest and gate; arbitrarily
omitting phases produces incomplete coverage.

## Lanes and evidence

| Lane | Required surface | Environment |
|---|---|---|
| `agg` | lifecycle, NFT, same-market settlement, routed strategies, fees, risk/admin/governance | live Reflector and quote routes; fresh candidate aggregator |
| `liq` | Transfer/Credit liquidation and multi-hub isolation, exact bad debt/recap, two-vault and contract-vault DeFindex | isolated SACs and oracle fixtures |
| `stress` | five collateral plus five debt positions, dual sources, Transfer/Credit maximum liquidation, five-asset exits, composed providers, delayed submission; oracle history/quorum | independent provider fixtures; deterministic shared-account interleaving |
| `flash` | callback success/rejections, protected balances, Long/multiple collateral, delegates and rollback snapshots | live Reflector; existing receiver fixtures |
| `blend` | actual pool allowlist/reserve addresses, six XLM paths plus distinct-token/multiple-liability migration, committed-rate shares/refunds/identity/unrelated balances | real Blend TestnetV2 pool |
| `production` | governance operator setup/replay, enabled mainnet policy readbacks, 7/8/9/18 decimal round trips, XOXNO-backed borrowing, contract caller, same-schema upgrades | disposable policy/wallet roots; explicit provider/LP/token fixtures |
| `sdk` | supply/borrow/repay/withdraw, routed multiply, Blend, events/error mapping/delayed signing | published SDK 1.0.221 and Stellar SDK 16.3.0; fresh contracts |

`cases.json` defines required terminal cases and action predicates, qualified by
contract role and execution type. `abi-coverage.json` maps all 218 candidate
exports, including generated NFT methods and constructors, to required actions
or justified controlled tests. Nested-call mappings are source-traced; they are
not runtime host call traces. `abi_coverage.py` compares actual WASM exports.

Required cases cannot pass via `research`, `sim-exceeded`, `environment-blocked`,
an empty report, missing actions, unknown status, or duplicate case IDs.
Diagnostics cannot erase a failed assertion, even with the same label. Expected
negative cases distinguish simulation rejection from a submitted failed receipt;
a simulation rollback check is not a committed-transaction rollback proof.

Mutations use `--send=yes`; reads use `--send=no`. CLI calls use native
`--instruction-leeway 20000000`. Both earlier 1M and 2M policies produced
submitted Credit 5+5 instruction failures. Controlled replay of the four exact
WASMs from release run 36137339149 measured 40,637,121 instructions at zero
elapsed time and 52,480,186 five seconds later: 11,843,065 additional instructions
for index projection/accrual. The regression uses one fixture provider and
source-account authorization; live dual-provider proofs remain required.
The 20M policy covers that measured delta with additional margin. The original
runs remain failed; maximum dimensions and the 10% resource headroom gate remain
unchanged. A fixed margin does not guarantee every future state transition.
SDK-prepared envelopes are not patched. Retry is
limited to classified transient failures without a signed hash. After a hash
exists, reconcile that hash; never rebuild the mutation. Unexpected submitted
Trapped/ResourceLimitExceeded failures remain fatal.
If the CLI loses a successful response, recovery verifies the signed envelope,
host operation and receipt return/event hash before recovering the result;
the original CLI status and output remain in attempt evidence.

Funding swaps share a checkout-wide file lock from quote acquisition through
receipt confirmation. Quote snapshots must reach the RPC ledger observed after
acquiring the lock. A cancelled or unresolved funding operation leaves
`runs/.external-funding.pending.json`; subsequent funding fails before quoting.
Reconcile the recorded operation before removing that marker. This coordination
does not control unrelated external traders or the deliberate contention tests.
SDK evidence includes native simulation responses/resources and failed wire
receipts; the harness observes them without changing prepared envelopes.

The resource gate checks signed envelope declarations, actual transaction/event
bytes, footprint counts, and both captured network limit sets. Declared
instructions, including CLI leeway, must retain at least 10% headroom. Successful
execution proves the testnet memory cap; a lower mainnet memory cap fails closed.
Mainnet access is read-only limit capture, never transaction submission.

## Reports

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

CI uploads detailed evidence on failure. `state.env` is local execution state;
reports omit private key material. Native XLM financial deltas account for actual
committed network fees. Raw token/share arithmetic uses integers, including
18-decimal values; exact rounding predicates use committed indexes, not later
projected view indexes.

## Controlled time and acceptance limits

`controlled-cases.json` labels long-idle interest, HF boundaries, governance
expiry/recovery, dust-close footprint stability, account TTL and host auto-restore. These are controlled-ledger
proofs, not multiweek testnet observations. The long-idle test compares paths
sharing the accrual routine; it is not independent reference arithmetic.
`integration-sdk-validate` tests native RPC restore preparation, a refreshed
source sequence and malformed simulations offline. It does not claim live
archival/restoration or automatic restore orchestration by the published SDK.

Production fixture checks preserve enabled policy, caps, fees, rates, decimals,
hub/spoke relationships and composition. Fixture-backed checks do not prove
real external-provider availability. Current upgrade evidence explicitly records
identical baseline/candidate controller hashes and `executable_differs=false`; controller/pool/NFT/price-aggregator/governance and oracle history preservation are checked; no v1.0.0
storage migration claim is made.

Final acceptance still requires two fresh complete seven-lane runs on the final
candidate SHA and a release-workflow dry run. Release dispatch defaults to `dry_run=true`; a branch can run the complete gate
without publication. `inject_e2e_failure=true` deliberately stops the E2E job
before deployment and must leave publication skipped. A successful dry run still
requires all seven live lanes. Local publication regressions inject failed lanes
and wrong artifacts. The injected-failure dry run at
[3d4153e0](https://github.com/XOXNO/rs-lending-xlm/actions/runs/36127604894)
passed build/checks, failed E2E deliberately, and skipped publication. A successful
full dry run remains outstanding.
Published SDK 1.0.219 returned null for `AmountMustBePositive` (#14). SDK
1.0.220 fixed that mapping; 1.0.221 adds native preparation instruction leeway.
The harness pins 1.0.221 and checks all four published ESM/CJS root/lending
entrypoints for the native 20M request, returned resources/fee, unsigned output,
and unchanged invocation. Error mapping is checked offline and live. Local SDK substitutions remain forbidden.

Live acceptance remains outstanding for newly added predicates and branches.
Complete liquidation, stress and flash smokes passed their 15, 12 and 11 cases. The SDK
lifecycle, routed strategy and Blend passed financial checks; its earlier
error-mapping and instruction-budget failures require a fresh run with SDK 1.0.221. These smokes used
uncommitted harness snapshots and do
not satisfy final-SHA acceptance. An older smoke hit a submitted Reflector
storage-footprint race, which remains a sticky failure. Fresh complete runs
must demonstrate all required cases; an
ABI map, successful offline regression, or partial smoke cannot certify them.
An aggregation smoke caught a real dust-close footprint failure: accrued
interest made a zero-token native withdrawal become one stroop at inclusion,
but simulation had omitted the recipient account. Preserve that failed receipt
when validating the pool fix; a fresh run must prove the corrected path.
The pool now sends a zero-value SAC transfer when positive shares are fully
burned for a zero-token payout. This reserves writable recipient state; empty
withdrawals and other zero refunds remain no-ops. Dust recipients must have a
valid account/authorized trustline. Custom tokens may implement zero transfers
differently; this fix does not guarantee every state-dependent footprint.
Another live failure exposed a same-ledger `update_indexes` simulation that
recorded pool state as read-only. Inclusion advanced time and interest accrual
needed a write. Index updates now always commit state; a frozen-footprint
regression preserves the original failure and verifies the later-ledger write.
Current-schema upgrades deliberately record same-bytecode baselines and make
no claim about migration from a different executable.

## Extending

Use `run_case` and the existing `inv`, `view`, `xfail`, `run_deploy` and assertion
helpers. Propagate prerequisites explicitly; a log message is not a failed case.
Add required predicates alongside a new flow, update ABI bindings, and add a
focused offline fault injection proving a wrong outcome cannot pass. Never
reduce required stress dimensions after failure. Inspect conservation before
cleanup; minting or top-ups must remain separately attributable.
