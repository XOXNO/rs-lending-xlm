# Live Testnet Integration Harness

End-to-end protocol exercise against **live Stellar testnet** using the
`stellar` CLI, friendbot, and the XOXNO swap aggregator. Primitives (`lib/`) →
flows (`flows/`) → scenarios (`scenarios/`). Any subset can run standalone or
in CI.

Before important runs: `make integration-preflight integration-validate`.

## Run

```bash
# Build wasm first (controller, pool, governance, price aggregator, flash
# receiver, DeFindex strategy, swap aggregator, mock oracles):
make integration-wasm   # required: plain `stellar contract build` leaves target/optimized empty

# (Optional) Preflight, plus regenerate the appendix pointer stub
make integration-preflight integration-appendix

# Release e2e — three independent lanes in PARALLEL, then gate each (what CI runs):
RUN_TS=$(date +%Y%m%d-%H%M%S) bash tests/integration/scenarios/parallel_e2e.sh

# Single serial world (debugging / resume): every flow against one deploy:
RUN_TS=$(date +%Y%m%d-%H%M%S) bash tests/integration/scenarios/full_e2e.sh

# Subset of phases, resuming an existing run's contracts/wallets:
PHASES="liquidation stress" RUN_TS=<existing> bash tests/integration/scenarios/full_e2e.sh

# CI green gate for a single RUN_TS (parallel_e2e runs it per lane automatically):
RUN_TS=<ts> bash tests/integration/scenarios/assert_green.sh
```

### Parallel lanes

`parallel_e2e.sh` is network-wait bound, so it splits the suite into independent
self-contained worlds (own controller / pool / governance / wallets / markets,
keyed by `RUN_TS=<base>-<lane>`) and runs them concurrently — wall-clock ≈
slowest lane instead of the sum. The split is along the **aggregator boundary**:

| Lane | Phases | Oracle / venue |
|------|--------|----------------|
| `agg` | lifecycle + strategies + admin + governance | live Reflector + XOXNO aggregator (serial *within* the lane) |
| `liq` | liquidation + defindex | mock oracles, venue-free |
| `stress` | stress | mock oracles, venue-free |

Each lane is gated independently; the run is green only if all three are. The
mock lanes share no state with `agg` or each other. `admin` uses the idle real
EURC market, not liquidation's mocks. The DeFindex strategy flow runs in the
`liq` lane with its own mock collateral.

### CI vs research scenarios

| Tier | Scripts | Gate |
|------|---------|------|
| **Release CI** | `parallel_e2e.sh` (per-lane `full_e2e.sh` → `assert_green.sh`) | All actions must be `ok`, `xfail`, `read`, or `sim-*` (not `sim-error`); no unresolved `FAIL` in any lane. Exercises the full model (see central facts: 3-contract ownership, scaled balances, pause matrix, multi-hub, bad-debt floor, etc.). |
| **Research** | `liq_20feed.sh`, `liq20_v2_walk.sh`, `liq_20feed_*.sh` | Width probes record `research` status (intentional frontier misses); run manually after stress. See `tests/test-harness/tests/fuzz/` for proptest coverage. |

Shared width logic lives in `lib/liq20_width.sh`. `liq_20feed.sh` sets up the
account. `liq20_v2_walk.sh` is the preferred instruction-cap walk;
`liq_20feed_walk.sh`, `liq_20feed_width.sh`, `liq_20feed_bisect.sh`,
`liq_20feed_retry9.sh`, and `liq_20feed_fullrepay.sh` are equally thin wrappers
over the same helpers with different width lists.

Each run writes `runs/<RUN_TS>/`:

| file | content |
|---|---|
| `report.md` | every action with status, tx hash (explorer link), declared CPU instructions / read / write bytes / resource fee |
| `actions.tsv` | the same data, machine-readable |
| `state.env` | deployed contract ids, wallet aliases, completed-block markers (resume support) |
| `logs/` | per-action stdout/stderr, quotes, simulation JSON |

### Interpreting reports

`report.md` (and `combined.md` for parallel lanes) are the primary human-readable artifacts.

- **Statuses**: `ok` (success), `xfail` (expected revert as designed), `read` (view-only), `sim-ok` / `sim-exceeded` (budget probe results), `research` (intentional wide probes in liquidation/stress research flows — these are expected to have errors in the note and are ignored by green gates), `retry` (transient handled internally).
- **Gates** (`assert_green.sh`): No unresolved `FAIL` or `UNEXPECTED-OK` or `sim-error`. A failing row is forgiven when a later row with the same action label lands `ok` or `xfail` (retry semantics); only unresolved rows fail the gate. All lanes must complete with the "run complete" marker.
- `combined.md` concatenates per-lane reports (used for release attachments). Research scenarios intentionally use `research` rows.
- Full simulation JSON and raw CLI output live under `logs/`. Resource numbers are the ones declared on the signed envelope (see explorer link for full receipt including memory).
- `appendix.md` is copied into each run directory. It holds no numbers: it points at the `meta` budget tests that print them (see `tests/integration/appendix.md`).
- Historical runs are for reference/CI archiving. Do not hand-edit generated `report.md` / `actions.tsv`.

See `tests/integration/lib/report.sh` for the generator and `scenarios/assert_green.sh` for the exact gate.

## Extending the Harness

- Use `inv` / `view` / `xfail` / `sim_probe` / `run_deploy` for all contract work (they record + retry + capture hashes via the centralized helpers).
- For direct `stellar contract deploy/upload` (rare): use `extract_signing_hash "$err_f"` + `sanitize_output "$out_f"` + `is_contract_id`/`is_wasm_hash` + `tail_err_note` + `record` + `save_state`.
- Add new constants to `env.sh` (or document overrides). Prefer `require_var FOO` for load-bearing state.
- Always `phase` + record meaningful statuses (`ok`/`xfail`/`research` etc.).
- Run `make integration-validate integration-preflight` locally.
- Research flows should still record `research` for intentional misses so `assert_green` ignores them.


## Layers

- `env.sh` — network constants, run-dir wiring. Everything overridable by env.
- `lib/core.sh` — run dir, action recording, state persistence (resume).
- `lib/invoke.sh` — `inv` (send + capture tx hash + resources), `xfail`
  (expected revert), `view` (read-only), `sim_probe` (build+simulate budget
  probe, no fees). Tx hash parsed from the CLI's `Signing transaction:` line —
  present only after simulation passes, so it doubles as the success signal.
- `lib/assert.sh` — parsed on-chain assertions (HF, debt, `is_liquidatable`, pool revenue).
- `lib/liq20_width.sh` — 20-feed liquidation width research helpers (`research` status).
- `lib/wallet.sh` — per-run unique friendbot-funded wallets (reused aliases
  run dry across runs; never share wallets between runs).
- `lib/assets.sh` — self-issued SACs, classic trustlines, mint, balances,
  funding via aggregator swap (ONE swap then SAC transfers; rapid repeat
  swaps trip the stale min-out check).
- `lib/aggregator.sh` — quote API; **always `max_splits=1`** or the route
  payload blows the tx budget inside strategy calls.
- `lib/oracle.sh` — deployable mock Reflector / mock RedStone price control.
  Liquidations are only force-able on mock-priced markets (real-feed HF can't
  be pushed underwater); deploy fresh mocks per run, or the feeds go stale and
  the price aggregator rejects reads with `#206 PriceFeedStale`.
- `lib/protocol.sh` — **integration fast-path** deploy (EOA-owned controller,
  immediate admin). Production deploy is `make testnet setup` (governance
  timelock). Also deploys a **governance contract + governance-owned controller**
  (short `INTEG_MIN_DELAY`): `resolve_asset_oracle` overwrites `asset_decimals`
  from the SAC, and the governance-owned controller is the target of the
  timelock e2e. Market bring-up sequence: create pending →
  `resolve_asset_oracle` (governance view) → `set_oracle` on the
  price-aggregator (`PriceKey` + `AssetOracle`) → activate; JSON builders emit
  composable single/dual `AssetOracle` documents.
- `lib/report.sh` — markdown report. Resource columns are the declared
  Soroban resources decoded from each signed envelope; the explorer link on
  every row shows the full per-tx resource report (incl. memory).

## Flows

`#NNN` is a Soroban contract error discriminant. Codes 1-502 are defined in
[`common/src/errors.rs`](../../common/src/errors.rs) and described in
[`docs/reference/errors.md`](../../docs/reference/errors.md). Three codes come
from the OpenZeppelin Stellar crates instead: `#1000 EnforcedPause` and
`#1001 ExpectedPause` from `stellar-contract-utils`, and `#2000 Unauthorized`
from `stellar-access`.

| flow | covers |
|---|---|
| `lifecycle.sh` | real markets (XLM/USDC/EURC on Reflector), aggregator funding, supply/borrow/repay/withdraw single + bulk, cross-account repay, views, guard reverts (`#14 AmountMustBePositive` on zero, `#100 InsufficientCollateral` over LTV) |
| `strategies.sh` | flash loan success + all 5 failure modes, multiply long/short, swap_debt, swap_collateral, repay_debt_with_collateral (all via aggregator routes) |
| `liquidation.sh` | partial / full / bulk multi-debt liquidation, spoke liquidation, clean_bad_debt socialization, healthy-account guards (`#101 HealthFactorTooHigh`) |
| `defindex.sh` | DeFindex strategy vault lifecycle over its own mock collateral (runs in the `liq` lane) |
| `admin.sh` | pause gates (`#1000 EnforcedPause` / `#1001 ExpectedPause`), position limits, param/config edits with read-back (`#113 InvalidLiqThreshold` bounds), oracle tolerance (resolve→set, owner-auth guard) and the sanity band (`#223 SanityBoundViolated`), `set_min_borrow_collateral_usd` (set/read/`#126 MinBorrowCollateralNotMet` effect/reset/`#116 InvalidBorrowParams`), permissionless keeper/revenue paths, spoke admin lifecycle (`#301 SpokeDeprecated`), upgrade (pauses by design) + migrate + 2-step ownership round-trip |
| `governance.sh` | governance timelock e2e on the governance-owned controller: `deploy_controller` ownership (+`#5 PoolAlreadyDeployed` redeploy), resolver views, propose→cancel (Waiting→Unset), propose→await→`execute` (open executor) lifecycle (Waiting→Ready→Unset), non-PROPOSER guard (`#2000 Unauthorized`), proposal validation (`#36 InvalidPositionLimits`, `#134 InvalidLiquidationCurve`), immediate role revocation refusing the owner's own role (`#44 NotAuthorized`), owner pause + timelocked unpause forwarding |
| `stress.sh` | 20 mock markets; bulk-supply frontier, distinct-feed borrow frontier (single- then dual-source), withdraw probe, repay-1 liquidation seize frontier — all via fee-less simulation probes plus one on-chain proof tx per frontier |
| `swap_aggregator.sh` | Swap-aggregator admin lifecycle |

## Encoding gotchas

- `i128` inside `Vec<(Address,i128)>` JSON must be a **quoted string**;
  scalar `--amount` flags take bare numbers.
- `#[repr(u32)]` enums (`PositionMode`, `AccountPositionType`) pass as bare
  integers.
- Union types use a bare JSON string for a unit variant and `{"Variant": value}`
  for a data variant, e.g. `nature: "Fundamental"` and `read_mode: {"Twap": 3}`
  in the oracle configs, or `"Transfer"` and `{"Credit": 0}` for `SeizeMode`.
  Emit these as JSON rather than a bare word so the CLI parses them as values
  instead of coercing them to strings (`lib/protocol.sh`).
- Mock-primary oracle configs must read **Twap**: `set_oracle` rejects a
  Spot-only single-source config with `#38 SpotOnlyNotProductionSafe`.
  Dual-source anchors must use a different provider kind.
- Flash receiver `data` is the XDR-encoded `FlashLoanRequest{mode}` ScVal.
