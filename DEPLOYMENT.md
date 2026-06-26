# Deployment & Operations Runbook

End-to-end guide for deploying and operating the Stellar lending protocol
(governance + controller + central pool) on testnet and mainnet, and for the
day-to-day management of markets, oracles, e-modes, and roles.

Everything is driven by `make <network> <action>` and the JSON config under
`configs/`. The same commands work on `testnet` and `mainnet`; only the config
values and the timelock delay differ.

---

## 1. Prerequisites

- **stellar-cli** (pinned version — `make install-stellar-cli`) and **jq**.
- A funded **deployer** key registered with the CLI:
  ```bash
  stellar keys generate deployer --network testnet   # or import an existing key
  stellar keys public-key deployer                   # fund this address
  ```
  Mainnet uses a hardware wallet — see [§9 Ledger signing](#9-ledger-signing).
- Config files (all under `configs/`):
  | File | Purpose |
  |------|---------|
  | `networks.json` | RPC URL, passphrase, contract addresses, timelock delay, e-mode id map (per network) |
  | `<network>_markets.json` | Market list: asset address, risk params, oracle config |
  | `emodes.json` | E-mode categories and their per-asset risk params (per network) |

---

## 2. RPC endpoint (from config)

Every stellar call is pinned to the `rpc_url` + `network_passphrase` in
`networks.json` (exported as `STELLAR_RPC_URL` / `STELLAR_NETWORK_PASSPHRASE`,
which take precedence over the CLI's built-in network RPC while still letting
`--network` resolve contract aliases). To switch providers, edit one line:

```json
"testnet": { "rpc_url": "https://stellar-testnet-gateway.xoxno.com", ... }
```

Use a reliable provider. The public `soroban-testnet.stellar.org` endpoint has
been observed returning transient `TxBadSeq` (stale-sequence reads) and
`Unset`-right-after-schedule (read-after-write lag) during long deploys; the
tooling now retries the safe cases automatically (see [§8](#8-resilience--recovery)),
but a good RPC avoids them in the first place.

---

## 3. The timelock & the bootstrap-delay pattern

All admin operations route through the governance timelock: `schedule → wait
min_delay ledgers → execute`. `min_delay` is fixed at governance construction
from `timelock_min_delay_ledgers` and can later only be **increased**
(`update_delay` is a one-way ratchet — it cannot shorten the delay).

- **testnet**: `timelock_min_delay_ledgers = 12` (~1 min) — fine to deploy with directly.
- **mainnet**: production target is `34560` (~48 h). Deploying with that value
  would make `make mainnet setup` wait ~48 h **per op** (~30 ops). Don't.

**Bootstrap pattern (mainnet):** deploy with a tiny delay so the whole setup runs
in minutes, then raise to the production delay at the end (the raise itself only
waits the still-tiny current delay):

```bash
DEPLOY_MIN_DELAY=1 make mainnet setup          # governance deployed with min_delay=1; full setup in ~minutes
# verify everything (see §7), then lock in the production delay:
make mainnet updateDelay 34560                 # increase-only ratchet → 48h
make mainnet info                              # confirm min_delay is now 34560
```

`DEPLOY_MIN_DELAY` only affects the governance constructor; `await` logic always
reads the **live** on-chain delay, so it scales correctly before and after the raise.
Leave `timelock_min_delay_ledgers` in `networks.json` at the production value as
the documented target.

---

## 4. Fresh deployment

`make <network> setup` runs the full sequence:

1. Build + strip deploy WASM (pool / controller / governance).
2. Upload pool & controller WASM → deploy **governance** (`--admin deployer
   --min_delay <delay>`) → `deploy_controller` via governance → `setPoolTemplate`
   + `deployPool` via the timelock.
3. `configure-controller`: set aggregator + revenue accumulator (both must be in
   `networks.json` — `aggregator`, `accumulator` — or passed via
   `AGGREGATOR_CONTRACT` / `ACCUMULATOR_CONTRACT`).
4. `setupAll`: create every market in `<network>_markets.json`, wire its oracle,
   activate it; then create every e-mode category in `emodes.json` and add its assets.
5. `unpause` (owner-immediate — no timelock wait).
6. Print status (`info`, `listMarkets`, `listEModeCategories`).

New addresses are written back to `networks.json` (`governance`, `controller`,
`pool`, `*_wasm_hash`, `emode_category_ids`).

```bash
# testnet — one shot
make testnet setup

# mainnet — bootstrap delay, then raise (see §3)
DEPLOY_MIN_DELAY=1 make mainnet setup
make mainnet updateDelay 34560
```

> A fresh deploy produces brand-new addresses; the previous deployment becomes
> dead (events/positions are not migrated). Update downstream consumers
> (UI / API / indexer) with the new `controller` / `governance` / `pool`.

---

## 5. Markets & oracles

Markets are defined in `configs/<network>_markets.json`. Each entry: `name`,
`asset_address`, `market_params` (rates, caps, LTV/threshold/bonus, flags), and
`oracle` (strategy + primary/anchor feeds + sanity bounds).

**Bulk (idempotent):**
```bash
make <network> setupAllMarkets        # create + oracle + activate every configured market (skips existing)
```

**One market at a time:**
```bash
make <network> createMarket USDC            # deploy the market (pending/inactive)
make <network> configureMarketOracle USDC   # wire the oracle
make <network> editAssetConfig USDC         # activate + apply risk params
make <network> getMarket USDC               # inspect config
make <network> getPrice USDC                # verify the oracle resolves within tolerance
```

### Oracle rules (must hold or `set_market_oracle_config` reverts)

- **Production strategy requires a non-spot primary.** RedStone is always spot,
  so it can never be the primary; only **Reflector with `read_mode = Twap`**
  qualifies. For `PrimaryWithAnchor`, the primary is Reflector-TWAP and the
  anchor is a **different** provider (RedStone). A spot-only primary fails with
  `SpotOnlyNotProductionSafe (#38)`.
- The oracle proposer **live-probes feeds at schedule time**, so the quote market
  must already exist on-chain before configuring an oracle that references it.
- `max_utilization_ray` is required in `market_params` (optimal < max ≤ RAY).
- Reflector **DEX** sources are USDC-based and fail USD-base validation — they
  cannot back USD-quoted (RWA/bond) markets.

---

## 6. E-modes

E-mode categories live in `configs/emodes.json` per network. Each has a `name`
and per-asset risk params (LTV, threshold, bonus, optional caps).

```bash
make <network> setupAllEModes            # create every category + add its assets (idempotent)
make <network> addEModeCategory 1        # create category 1 from config → records its on-chain id
make <network> addAssetToEMode 1 USDC    # add USDC to category 1
make <network> getEMode 1                # inspect category + assets
make <network> listEModeCategories       # list all
```

The on-chain category id is stored in `networks.json` under `emode_category_ids`
(config-id → on-chain-id). To re-create a category, remove its entry there first
so the idempotent setup re-creates it.

> Each `add_e_mode_category` op derives a category-id-seeded salt, so creating
> several categories in one run produces distinct timelock op ids. (A shared
> salt previously collided on the second category with `#4000`.)

---

## 7. Verify the deployment

```bash
make <network> info                  # governance/controller/aggregator/accumulator + min_delay + paused
make <network> listMarkets           # configured markets
make <network> listEModeCategories   # categories + their assets
make <network> getPrice USDC         # oracle pipeline (price within tolerance)
make <network> getEMode 1            # category params
```

A live, usable deployment shows: governance owns the controller, all markets
active, protocol unpaused, and `getPrice` returning a price `within_first_tolerance`.

---

## 8. Resilience & recovery

**Automatic retries.** Transaction submits retry only on errors that guarantee
the tx never landed (`TxBadSeq`, pre-send connection failures) — never on
ambiguous post-submission timeouts, so nothing is double-submitted. The await
loop tolerates a few `Unset` reads right after a confirmed schedule (RPC lag)
before failing. Tune with `STELLAR_TX_MAX_RETRIES`, `STELLAR_TX_RETRY_DELAY`,
`UNSET_MAX_POLLS`, `AWAIT_MAX_WAIT_SECONDS`.

**Resume an interrupted setup.** If the contracts deployed but a later phase
failed, re-run the idempotent post-deploy phases against the addresses already
in `networks.json` (skips the contract deploy):

```bash
make <network> resume     # configure-controller → markets → oracles → e-modes → unpause
```

**Manual op recovery.** Scheduled ops are recorded under `tmp/ops/<network>/`.
To drive a single op:
```bash
make <network> opState <op-id>      # Unset | Waiting | Ready | Done
make <network> awaitOp <op-id>      # wait until Ready
make <network> executeOp <op-id>    # execute a ready op
make <network> cancelOp <op-id>     # cancel a scheduled op
```
Set `AUTO_EXECUTE=0` on a scheduling command to schedule-only (record the op id
for a later `executeOp`).

---

## 9. Ledger signing

For mainnet, sign with a hardware wallet:
```bash
SIGNER=ledger make mainnet setup
SIGNER=ledger make mainnet updateDelay 34560
```
Each transaction prompts the device; a fresh deploy is ~30 confirmations.

---

## 10. Roles

Governance operational roles are `ORACLE | PROPOSER | EXECUTOR | CANCELLER`
(all timelocked grants). A PROPOSER can schedule ops; an EXECUTOR can execute
ready ops.

```bash
make <network> grantGovRole G...ADDRESS PROPOSER
make <network> grantGovRole G...ADDRESS EXECUTOR
make <network> hasRole       G...ADDRESS PROPOSER     # → true|false
make <network> revokeGovRole G...ADDRESS PROPOSER
```

---

## 11. Upgrades

In-place upgrades via governance (each is timelocked):
```bash
make <network> upgradeController       # upload + upgrade controller
make <network> upgradeGovernance       # upload + upgrade governance (self-timelock)
make <network> upgradePoolTemplate     # upload + set new pool template hash
make <network> upgradePools            # upgrade the central pool to the template
make <network> upgradeAll              # pool template + controller + pool, then unpause
```

---

## 12. Gotchas

- **`NETWORK` env var.** An exported `NETWORK` silently overrides the Makefile
  default (`NETWORK ?= testnet`). The `make <network> ...` form passes it
  explicitly, but prefix one-off shell invocations with `env -u NETWORK` if your
  shell exports it.
- **Interactive `cp`/`mv` aliases.** If your shell aliases these to `-i`, scripted
  overwrites can hang on a prompt — the tooling uses temp files + explicit moves.
- **Increase-only delay.** `updateDelay` cannot lower the timelock. Bootstrap low,
  raise once, deliberately.
- **Aggregator / accumulator are prerequisites.** `setup` and `resume` fail at
  preflight until `networks.json` has non-empty `aggregator` and `accumulator`
  for the network.
