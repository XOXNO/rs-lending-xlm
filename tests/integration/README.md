# Live Testnet Integration Harness

End-to-end protocol exercise against **live Stellar testnet** using the
`stellar` CLI, friendbot, and the XOXNO swap aggregator. Designed as lego
bricks: primitives (`lib/`) → flows (`flows/`) → scenarios (`scenarios/`),
so any subset can run standalone or inside a release workflow.

## Run

```bash
# Build wasm first (controller, pool, governance, flash receiver, mock oracles):
make integration-wasm   # or: stellar contract build

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
| `liq` | liquidation + caps | mock oracles, venue-free |
| `stress` | stress | mock oracles, venue-free |

Each lane is gated independently; the run is green only if all three are. The
mock lanes share no state with `agg` or each other (`admin` uses the idle real
EURC market, not liquidation's mocks; `caps` uses its own mock collateral).

### CI vs research scenarios

| Tier | Scripts | Gate |
|------|---------|------|
| **Release CI** | `parallel_e2e.sh` (per-lane `full_e2e.sh` → `assert_green.sh`) | All actions must be `ok`, `xfail`, `read`, or `sim-*` (not `sim-error`); no unresolved `FAIL` in any lane |
| **Research** | `liq_20feed.sh`, `liq20_v2_walk.sh`, `liq_20feed_*.sh` | Width probes record `research` status (intentional frontier misses); run manually after stress |

Shared width logic lives in `lib/liq20_width.sh`. **`liq20_v2_walk.sh`** is the canonical instruction-cap walk; `liq_20feed_walk.sh`, `width.sh`, `bisect.sh`, and `retry9.sh` are thin wrappers.

Each run writes `runs/<RUN_TS>/`:

| file | content |
|---|---|
| `report.md` | every action with status, tx hash (explorer link), declared CPU instructions / read / write bytes / resource fee |
| `actions.tsv` | the same data, machine-readable |
| `state.env` | deployed contract ids, wallet aliases, completed-block markers (resume support) |
| `logs/` | per-action stdout/stderr, quotes, simulation JSON |

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
  be pushed underwater); deploy fresh mocks per run or feeds go stale (#206).
- `lib/protocol.sh` — **integration fast-path** deploy (EOA-owned controller,
  immediate admin). Production deploy is `make testnet setup` (governance
  timelock). Also deploys a **governance contract + governance-owned controller**
  (short `INTEG_MIN_DELAY`): the resolver views turn input oracle configs into
  the resolved `MarketOracleConfig` the controller setter stores, and the
  governance-owned controller is the target of the timelock e2e. Market bring-up
  sequence: approve_token → create pending → `resolve_market_oracle_config`
  (governance view) → `set_market_oracle_config` → activate; JSON builders for
  params / asset config / single + dual oracle configs.
- `lib/report.sh` — markdown report. Resource columns are the declared
  Soroban resources decoded from each signed envelope; the explorer link on
  every row shows the full per-tx resource report (incl. memory).

## Flows

| flow | covers |
|---|---|
| `lifecycle.sh` | real markets (XLM/USDC/EURC on Reflector), aggregator funding, supply/borrow/repay/withdraw single + bulk, cross-account repay, views, guard reverts (#14 zero, #100 over-LTV) |
| `strategies.sh` | flash loan success + all 5 failure modes, multiply long/short, swap_debt, swap_collateral, repay_debt_with_collateral (all via aggregator routes) |
| `liquidation.sh` | partial / full / bulk multi-debt liquidation, spoke liquidation, clean_bad_debt socialization, healthy-account guards (#101); `flow_caps` — supply-cap (#105) + borrow-cap (#106) reverts on a dedicated capped market |
| `admin.sh` | pause gates (#1000/#1001), position limits (#36), param/config edits with read-back (#113 bounds), oracle tolerance (resolve→set, owner-auth guard), `disable_token_oracle` (#12/#2000), `set_min_borrow_collateral_usd` (set/read/#126 effect/reset/#116), keeper ops (+non-KEEPER #2000), revenue (+non-REVENUE #2000), spoke admin lifecycle (#301), upgrade (pauses by design) + migrate + 2-step ownership round-trip |
| `governance.sh` | governance timelock e2e on the governance-owned controller: `deploy_controller` ownership (+#5 redeploy), resolver views, propose→cancel (Waiting→Unset), propose→await→`execute` (open executor) lifecycle (Waiting→Ready→Done), non-PROPOSER guard (#2000), owner-immediate pause/unpause forwarding |
| `stress.sh` | 20 mock markets; bulk-supply frontier, distinct-feed borrow frontier (single- then dual-source), withdraw probe, repay-1 liquidation seize frontier — all via fee-less simulation probes plus one on-chain proof tx per frontier |

## Encoding gotchas (hard-won)

- `i128` inside `Vec<(Address,i128)>` JSON must be a **quoted string**;
  scalar `--amount` flags take bare numbers.
- `#[repr(u32)]` enums (PositionMode, OracleStrategy) pass as bare integers.
- Union types use `{"Variant": value}` / `"Variant"` for unit variants
  (e.g. `anchor: "None"`, `read_mode: {"Twap": 3}`).
- Mock-primary oracle configs must read **Twap** (Spot-only single-source is
  rejected, #38); dual-source anchors must be a different provider kind.
- Flash receiver `data` is the XDR-encoded `FlashLoanRequest{mode}` ScVal.
