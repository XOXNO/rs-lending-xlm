# Deployment & Operations Runbook

Deploy and operate governance + controller + pool on testnet or mainnet via
`make <network> <action>` and JSON under `configs/`. Commands are the same on
both networks; only config values and timelock delay differ.

Pause and unpause: §3 (GUARDIAN pause immediate; unpause is always timelocked).

---

## 1. Prerequisites

- **stellar-cli** (`make install-stellar-cli`) and **jq**
- Funded **deployer** key in the CLI:

```bash
stellar keys generate deployer --network testnet   # or import
stellar keys public-key deployer                   # fund this address
```

Mainnet: hardware wallet — [§9](#9-ledger-signing).

| Config | Purpose |
|--------|---------|
| `configs/networks.json` | RPC, passphrase, contract addresses, timelock delay, spoke id map |
| `configs/<network>/markets.json` | Assets, risk params, oracle config |
| `configs/<network>/spokes.json` | Spoke categories + per-asset risk |
| `configs/<network>/hubs.json`, `oracle_feeds.json`, `blend.json` | Hub map, feed catalog, Blend migration |

---

## 2. RPC endpoint

Every invoke uses `rpc_url` + `network_passphrase` from `networks.json`
(`STELLAR_RPC_URL` / `STELLAR_NETWORK_PASSPHRASE` override the CLI default while
`--network` still resolves aliases). Edit one line to switch providers:

```json
"testnet": { "rpc_url": "https://stellar-testnet-gateway.xoxno.com", ... }
```

Public `soroban-testnet.stellar.org` can lag (`TxBadSeq`, brief `Unset` after
schedule). Tooling retries safe cases ([§8](#8-resilience--recovery)); a solid
RPC still helps long deploys.

---

## 3. Pause, unpause, and the bootstrap delay

### Protocol behavior (authoritative)

| Action | Who | Path |
|--------|-----|------|
| **Pause** controller | `GUARDIAN` | Immediate governance entrypoint `pause(caller)` → controller `pause` |
| **Unpause** controller | Timelock | `propose(AdminOperation::Unpause)` → wait delay → `execute` (no governance `unpause` entrypoint) |
| Controller `pause` / `unpause` | Owner only | Owner = governance contract |
| Spoke `paused` / `frozen` tighten | `GUARDIAN` | Immediate `set_spoke_asset_flags` (clearing a flag is timelocked edit) |

Global pause still allows repay, withdraw, liquidate, and `clean_bad_debt`.
Spoke flags: [ADR 0011](../explanation/decisions/0011-pause-and-freeze-matrix.md).

### Tooling

```bash
make <network> pause      # GUARDIAN-immediate (signer is --caller)
make <network> unpause    # propose AdminOperation::Unpause → await → execute
```

`unpause` keeps a mainnet floor check: refuses while live `min_delay` is below
`timelock_min_delay_ledgers` in `networks.json`. Use `listOps` / `awaitOp` /
`executeOp` if you schedule with `AUTO_EXECUTE=0`.

### Timelock delay

All admin ops: `schedule → wait min_delay ledgers → execute`.
`min_delay` is set at governance construction; `update_delay` can only
**increase** it (one-way ratchet).

| Network | Typical delay |
|---------|----------------|
| testnet | `12` ledgers (~1 min) — fine for full setup |
| mainnet production | `34560` (~48 h) — do **not** deploy with this for the ~30-op setup |

**Mainnet bootstrap:** short delay while **paused** → configure → raise delay
to floor → only then unpause via timelock.

```bash
DEPLOY_MIN_DELAY=1 make mainnet setup   # full setup; stays paused
make mainnet updateDelay 34560          # increase-only ratchet
make mainnet info                       # confirm min_delay
# Then unpause via AdminOperation::Unpause (propose → await → execute)
```

`DEPLOY_MIN_DELAY` only affects the governance constructor. Await logic always
reads the **live** on-chain delay. Leave `timelock_min_delay_ledgers` in
`networks.json` at the production floor; use it as the go-live gate.

`make mainnet setup` never auto-unpauses. `make testnet setup` unpauses after
markets via the timelocked `Unpause` path (short testnet delay).

---

## 4. Fresh deployment

`make <network> setup`:

1. Build + strip WASM (pool / controller / governance / price-aggregator).
2. Upload pool & controller WASM → deploy governance → `deploy_controller` →
   `deploy_price_aggregator` (governance-owned oracle authority) →
   `deployPool(hash)` via timelock (single op; no template step).
3. `configure-controller`: swap aggregator + revenue accumulator (must be in
   `networks.json` as `aggregator` / `accumulator`, or
   `AGGREGATOR_CONTRACT` / `ACCUMULATOR_CONTRACT`), then the timelocked
   `SetPriceAggregator` self-op wiring the controller to the deployed
   price-aggregator.
4. `setupAll`: every market in `markets.json` (create, oracle, activate) then
   every spoke in `spokes.json`.
5. Unpause attempt (testnet only in Makefile; mainnet left paused — see §3).
6. Status: `info`, `listMarkets`, `listSpokeCategories`.

The **price aggregator** (oracle authority) is deployed and wired by `setup`
itself — no pre-set address needed. The **swap aggregator** (external DEX
router) and **accumulator** (revenue treasury wallet) are prerequisites: set
them in `networks.json`, or deploy the bundled swap aggregator first with
`make <network> deployAggregator` (writes back `networks.json.aggregator`).

Addresses write back to `networks.json` (`governance`, `controller`, `pool`,
`price_aggregator`, `*_wasm_hash`, `spoke_ids`).

```bash
make testnet setup

DEPLOY_MIN_DELAY=1 make mainnet setup
make mainnet updateDelay 34560
# unpause via AdminOperation::Unpause when ready
```

A fresh deploy creates **new** addresses; the old stack is dead (no position
migration). Point UI / API / indexer at the new ids.

---

## 5. Markets & oracles

Defined in `configs/<network>/markets.json`: `name`, `asset_address`,
`market_params`, `oracle`. `market_params` includes the rate curve plus
`is_flashloanable` (bool) and `flashloan_fee` (u32 bps, ≤ 500).

```bash
make <network> validateConfigs          # before any setup/resume
make <network> setupAllMarkets          # create + oracle + activate (idempotent)

make <network> createMarket USDC
make <network> configureMarketOracle USDC
make <network> addAssetToSpoke 1 USDC   # activate with spoke risk params
make <network> getMarket USDC
make <network> getPrice USDC
```

### Oracle rules (else `set_oracle_config` reverts)

- **Anchored** markets need a non-spot primary. RedStone and `xoxno-oracle` are
  always spot → cannot be primary. Production shape: Reflector `Twap` primary +
  different provider/contract as anchor. Spot primary on anchored market →
  `SpotOnlyNotProductionSafe`. `Single` may use spot; sanity band capped at ±10%.
- Proposer **live-probes** feeds at schedule time; quote market must already
  exist when an oracle references it.
- `max_utilization_ray` required (optimal < max ≤ RAY).
- Reflector **DEX** sources reprice through the quote asset oracle: put the
  quote market (e.g. USDC) **earlier** in `markets.json` than DEX-priced
  markets. `validateConfigs` checks this.

---

## 6. Spokes

```bash
make <network> setupAllSpokes
make <network> addSpoke 1
make <network> addAssetToSpoke 1 USDC
make <network> getSpoke 1
make <network> listSpokes
```

On-chain category ids land in `networks.json` → `spoke_ids`. To re-create a
category, remove its map entry so idempotent setup recreates it.

Each `add_spoke` uses a category-seeded salt so multi-category runs do not
collide on op ids.

---

## 7. Verify the deployment

```bash
make <network> info           # contracts, min_delay, pause-related status
make <network> checkDelay     # live delay vs configured floor
make <network> listMarkets
make <network> listSpokes
make <network> listOps        # recorded ops + live state
make <network> getPrice USDC
make <network> getSpoke 1
```

Live system: governance owns controller, markets active, protocol **unpaused**,
`getPrice` resolves for each market.

**Keeper (TTL):** separate workspace `services/keeper` (see its README). Extends
TTL for controller instance/persistent keys (oracles, spokes, accounts, etc.),
pool, governance, WASM. Self-authorizes `update_indexes` (no controller
`KEEPER` role). Without it, storage can archive. Uses the same `networks.json`.

---

## 8. Resilience & recovery

**Retries.** Submit retries only when the tx never landed (`TxBadSeq`, pre-send
connection loss) — never on ambiguous post-submit timeouts. Await tolerates a
few `Unset` reads after schedule. Env: `STELLAR_TX_MAX_RETRIES`,
`STELLAR_TX_RETRY_DELAY`, `UNSET_MAX_POLLS`, `AWAIT_MAX_WAIT_SECONDS`.

**Resume** after partial setup (addresses already in `networks.json`):

```bash
make <network> resume     # configure-controller → markets → oracles → spokes → unpause path
```

**Manual ops** (under `configs/ops/<network>/` — commit these on mainnet):

```bash
make <network> listOps
make <network> executeReady
make <network> opState <op-id>    # Unset | Waiting | Ready | Done
make <network> awaitOp <op-id>
make <network> executeOp <op-id>
make <network> cancelOp <op-id>
```

`AUTO_EXECUTE=0` schedules only. Scheduling is idempotent via deterministic
`hash_operation` ids.

**Re-apply after A→B→A:** executed op ids stay Done forever. Tooling uses salt
generations (hash chain off base salt):

- Direct verbs (`editAssetInSpoke`, `configureMarketOracle`, role grants, …)
  bump generation when Done. `REAPPLY_ON_DONE=0` skips instead.
- Bulk (`setupAll*`, `resume`): converge mode; re-apply only when on-chain
  probe shows drift (e.g. spoke assets).
- Creators (`addSpoke`, `createHub`, `deployPool`) never auto-re-apply.
- `SALT_NONCE=<n>` force-fresh id; `MAX_SALT_GENERATIONS` (default 16) caps probing.

---

## 9. Ledger signing

```bash
SIGNER=ledger make mainnet setup
SIGNER=ledger make mainnet updateDelay 34560
```

Each tx prompts the device; a full deploy is on the order of ~30 confirmations.

---

## 10. Roles

Governance roles: `PROPOSER` | `EXECUTOR` | `CANCELLER` | `ORACLE` | `GUARDIAN`.

| Role | Power |
|------|--------|
| **PROPOSER** | Schedule `AdminOperation` |
| **EXECUTOR** | Execute ready ops (or open execute when `executor = None`) |
| **CANCELLER** | Cancel pending ops (role revocations are non-cancellable) |
| **GUARDIAN** | Immediate: global `pause`, tighten spoke flags, create hub/spoke |
| **ORACLE** | Immediate: move sanity band (must contain live price) |

Controller has **no** `KEEPER` / `REVENUE` / `ORACLE` roles — only owner +
pausable. Keeper self-authorizes where the contract allows.

```bash
make <network> grantGovRole G...ADDRESS PROPOSER
make <network> grantGovRole G...ADDRESS GUARDIAN
make <network> hasRole       G...ADDRESS PROPOSER
make <network> revokeGovRole G...ADDRESS PROPOSER
```

Most grants ride the timelock. Immediate incident paths are GUARDIAN/ORACLE as
above.

---

## 11. Governance keys & recovery

**Owner is a native Stellar multisig account, not a Safe-style contract.**
Governance stores one owner `Address` and `require_auth`s it; M-of-N is
account `SetOptions` weights/thresholds.

**Canceller council** is independent. Grant `CANCELLER` via timelocked
`grantGovRole`. Any single canceller can veto a pending op (`cancelOp`) —
1-of-N, not quorum. Exception: a canceller cannot cancel the op that revokes
**its own** role.

**Immediate revoke** (`revoke_role_immediate`) only accepts `GUARDIAN` /
`ORACLE`. Stripping `PROPOSER` / `EXECUTOR` / `CANCELLER` always uses the
timelock + single-veto rule, so a compromised owner cannot instantly gut the
council.

**Recovery tier** breaks a colluding-canceller deadlock: owner-only,
non-vetoable `propose_canceller_reset` / `execute_canceller_reset` at
`TIMELOCK_RECOVERY_MIN_DELAY_LEDGERS` (~30 days). Replaces the non-owner
canceller set; owner keeps its own `CANCELLER`. Dedicated entrypoints — use
`invoke-id` against governance (not the controller dispatcher):

```bash
make invoke-id CONTRACT_ID=<governance> FN=propose_canceller_reset \
  ARGS='--new_cancellers ["G...","G..."] --salt <64-hex>' NETWORK=<network>
# wait Recovery delay...
make invoke-id CONTRACT_ID=<governance> FN=execute_canceller_reset \
  ARGS='--executor null --new_cancellers ["G...","G..."] --salt <64-hex>' NETWORK=<network>
```

---

## 12. Upgrades

All timelocked. Pool bootstrap and day-2 upgrades both pass the Wasm hash in
the op — nothing is stored on the controller as a template.

```bash
make <network> upgradeController       # upload + UpgradeController
make <network> upgradeGovernance       # upload + UpgradeGov
make <network> upgradePool             # upload + UpgradePool
make <network> upgradeAll              # upgradePool + upgradeController + unpause path

# Fresh deploy path (also used by setup): upload pool wasm, then
# DeployPool(hash) via the timelock — single op, no separate template step.
```

Price-aggregator has no in-place WASM upgrade: deploy a new instance owned by
governance, then timelocked `SetPriceAggregator` (Sensitive).

---

## 13. Gotchas

- **`NETWORK` env.** An exported `NETWORK` overrides Makefile default. Prefer
  `make <network> ...`, or `env -u NETWORK` for one-offs.
- **Interactive `cp`/`mv`.** Shell `-i` aliases hang scripts; tooling uses temps.
- **Increase-only delay.** Bootstrap low, raise once.
- **Swap aggregator / accumulator required.** `configure-controller` fails if
  the swap `aggregator` / `accumulator` are missing from `networks.json`
  (override with `ALLOW_MISSING_AGGREGATOR=1` / `ALLOW_MISSING_ACCUMULATOR=1`).
  The price aggregator is deployed + wired by `setup`, not a prerequisite.
- **No on-chain `is_paused` view** in current tooling notes — infer from
  behavior / events / off-chain index.
