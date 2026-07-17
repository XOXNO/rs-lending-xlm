# Deployment & Operations Runbook

End-to-end guide for deploying and operating the Stellar lending protocol
(governance + controller + central pool) on testnet and mainnet, and for the
day-to-day management of markets, oracles, spokes, and roles.

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
  | `networks.json` | RPC URL, passphrase, contract addresses, timelock delay, spoke id map (per network) |
  | `<network>/markets.json` | Market list: asset address, risk params, oracle config |
  | `<network>/spokes.json` | Spoke categories and their per-asset risk params |
  | `<network>/hubs.json`, `<network>/oracle_feeds.json`, `<network>/blend.json` | Hub id map, oracle feed catalog, Blend migration inputs |

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

**Bootstrap pattern (mainnet):** deploy + configure with a tiny delay so the whole
setup runs in minutes **while the protocol stays paused**, then raise to the
production delay and only then go live. `make mainnet setup` never auto-unpauses,
and `make mainnet unpause` **refuses** until the on-chain delay reaches
`timelock_min_delay_ledgers`, so mainnet can never be live below the production floor:

```bash
DEPLOY_MIN_DELAY=1 make mainnet setup          # governance deployed with min_delay=1; full setup in ~minutes; LEFT PAUSED
# verify everything (see §7), then lock in the production delay:
make mainnet updateDelay 34560                 # increase-only ratchet → 48h (waits the still-tiny current delay)
make mainnet info                              # confirm min_delay is now 34560
make mainnet unpause                           # go live — gated on delay >= floor
```

`DEPLOY_MIN_DELAY` only affects the governance constructor; `await` logic always
reads the **live** on-chain delay, so it scales correctly before and after the raise.
Leave `timelock_min_delay_ledgers` in `networks.json` at the production value as
the documented target — the unpause gate reads it as the floor.

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
4. `setupAll`: create every market in `configs/<network>/markets.json`, wire its
   oracle, activate it; then create every spoke category in
   `configs/<network>/spokes.json` and add its assets.
5. `unpause` (owner-immediate — no timelock wait). **Mainnet: skipped** — setup
   leaves the protocol paused, and unpause is a separate step gated on the
   timelock delay reaching the production floor (see §3).
6. Print status (`info`, `listMarkets`, `listSpokeCategories`).

New addresses are written back to `networks.json` (`governance`, `controller`,
`pool`, `*_wasm_hash`, `spoke_ids`).

```bash
# testnet — one shot
make testnet setup

# mainnet — bootstrap delay (paused), raise, then go live (see §3)
DEPLOY_MIN_DELAY=1 make mainnet setup
make mainnet updateDelay 34560
make mainnet unpause
```

> A fresh deploy produces brand-new addresses; the previous deployment becomes
> dead (events/positions are not migrated). Update downstream consumers
> (UI / API / indexer) with the new `controller` / `governance` / `pool`.

---

## 5. Markets & oracles

Markets are defined in `configs/<network>/markets.json`. Each entry: `name`,
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
make <network> addAssetToSpoke 1 USDC       # activate: list in a spoke with risk params
make <network> getMarket USDC               # inspect config
make <network> getPrice USDC                # verify the oracle resolves within tolerance
```

Run `make <network> validateConfigs` first — it cross-checks the markets,
spokes, and networks JSON (hub ids, risk bounds, oracle wiring, spoke/market
parity) and is also run automatically before any `setup`/`resume`/`setupAll*`.

### Oracle rules (must hold or `set_market_oracle_config` reverts)

- **Anchored markets require a non-spot primary.** RedStone and the XOXNO
  adapter are always spot, so neither can be the primary; only **Reflector
  with `read_mode = Twap`** qualifies. For `PrimaryWithAnchor`, the primary is
  Reflector-TWAP and the anchor is a **different** provider (RedStone or
  XOXNO) on a **different contract**. A spot primary on an anchored market
  fails with `SpotOnlyNotProductionSafe (#38)`. `Single` markets may read
  spot, but their sanity band is capped at ±10%
  (`SanityBandTooWideForSingleSource`).
- The oracle proposer **live-probes feeds at schedule time**, so the quote market
  must already exist on-chain before configuring an oracle that references it.
- `max_utilization_ray` is required in `market_params` (optimal < max ≤ RAY).
- Reflector **DEX** sources are quoted in USDC and reprice through the quote
  asset's own oracle (`ReflectorBase::Quoted`): the quote market (USDC) must
  appear **earlier in the markets file** than any DEX-priced market, because
  setup configures oracles in file order. `validateConfigs` checks this.

---

## 6. Spokes

Spoke categories live in `configs/<network>/spokes.json`. Each has a `name`
and per-asset risk params (LTV, threshold, bonus, optional caps).

```bash
make <network> setupAllSpokes            # create every category + add its assets (idempotent)
make <network> addSpoke 1        # create category 1 from config → records its on-chain id
make <network> addAssetToSpoke 1 USDC    # add USDC to category 1
make <network> getSpoke 1                # inspect category + assets
make <network> listSpokes                # list all
```

The on-chain category id is stored in `networks.json` under `spoke_ids`
(config-id → on-chain-id). To re-create a category, remove its entry there first
so the idempotent setup re-creates it.

> Each `add_spoke` op derives a category-id-seeded salt, so creating
> several categories in one run produces distinct timelock op ids. (A shared
> salt previously collided on the second category with `#4000`.)

---

## 7. Verify the deployment

```bash
make <network> info                  # governance/controller/aggregator/accumulator + min_delay + paused (see INVARIANTS.md §5.4 pause/freeze matrix)
make <network> checkDelay            # live timelock delay vs configured target (bootstrap guard)
make <network> listMarkets           # configured markets
make <network> listSpokes            # categories + their assets
make <network> listOps               # every recorded governance op + live state
make <network> getPrice USDC         # oracle pipeline (price within tolerance)
make <network> getSpoke 1            # category params
```

A live, usable deployment shows: governance owns the controller, all markets
active, protocol unpaused, and `getPrice` resolving a live price for each market.

**Keeper (TTL maintenance):** Deploy and run the separate `services/keeper`
workspace (see its README). It discovers and extends TTL for controller
instance/persistent keys (including `AssetOracle`, `Spoke`, account maps,
`Params`/`State`), pool entries, governance, and WASM code. The keeper
self-authorizes `update_indexes` (no controller `KEEPER` role). Without it,
storage can archive after TTL windows. Config uses the same `networks.json`
and `contracts.markets` list.

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
make <network> resume     # configure-controller → markets → oracles → spokes → unpause
```

**Manual op recovery.** Scheduled ops are recorded under `configs/ops/<network>/`
(tracked in git — commit them so a mainnet op waiting out its delay survives the
machine). To inspect and drive ops:
```bash
make <network> listOps              # every recorded op + live state
make <network> executeReady         # execute all Ready ops
make <network> opState <op-id>      # Unset | Waiting | Ready | Done
make <network> awaitOp <op-id>      # wait until Ready
make <network> executeOp <op-id>    # execute a ready op
make <network> cancelOp <op-id>     # cancel a scheduled op
```
Set `AUTO_EXECUTE=0` on a scheduling command to schedule-only (record the op id
for a later `executeOp`).

Scheduling is **idempotent**: every schedule pre-computes its deterministic op
id (`hash_operation`) and reuses an op that is already Waiting/Ready — or skips
one that is Done — instead of re-proposing, so `make <network> resume` and
re-running `setupAll*` are safe after a partial failure.

**Re-applying a previous setting (toggle A → B → back to A):** the timelock
marks an executed op id Done forever, so identical args cannot reuse their old
id. The tooling handles this automatically with **salt generations** (a hash
chain off the deterministic base salt):

- **Direct verbs** (`editAssetInSpoke`, `configureMarketOracle`,
  `approveToken`, role grants, …) detect the Done op and re-apply at the next
  free generation — toggling back just works. `REAPPLY_ON_DONE=0` disables
  this (skip instead).
- **Bulk flows** (`setupAll*`, `resume`) run in converge mode: Done ops are
  treated as already applied, EXCEPT where an on-chain probe proves drift
  (spoke assets), which forces a re-apply. So resume never schedules
  redundant ops, and a config toggle still converges.
- **Creators** (`addSpoke`, `createHub`, `deployPool`) never auto-re-apply —
  re-executing one would mint a duplicate entity.
- `SALT_NONCE=<n>` remains a manual override that mints a fresh id for any
  verb; `MAX_SALT_GENERATIONS` (default 16) caps automatic probing.

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

Governance operational roles are `PROPOSER | EXECUTOR | CANCELLER | ORACLE | GUARDIAN`.
Most grants are timelocked; `GUARDIAN` is intended for immediate per-listing
incident actions (e.g. set flags) and is not timelocked for those. The controller
itself defines no `KEEPER`, `REVENUE`, or `ORACLE` roles — those live on governance.

A PROPOSER can schedule ops; an EXECUTOR can execute ready ops.

```bash
make <network> grantGovRole G...ADDRESS PROPOSER
make <network> grantGovRole G...ADDRESS EXECUTOR
make <network> grantGovRole G...ADDRESS GUARDIAN
make <network> hasRole       G...ADDRESS PROPOSER     # → true|false
make <network> revokeGovRole G...ADDRESS PROPOSER
```

---

## 11. Governance keys & recovery

**Owner is a native Stellar multisig, not a contract.** Governance's `owner`
is a single `Address`. Set it to a Stellar account with multiple signers by
adding signers and weights and raising the `medium`/`high` thresholds via
that account's `SetOptions` operation (M-of-N). No Safe-style multisig
*contract* is needed or supported — the governance contract only ever sees
one owner `Address` and calls `require_auth` on it; the account's
signature-weight rules decide whether that succeeds.

**Canceller council is independent of the owner.** Grant `CANCELLER` to each
council member through the timelocked `grantGovRole` (same path as any other
role):
```bash
make <network> grantGovRole G...ADDRESS CANCELLER
```
Any single canceller can veto a pending op via `cancelOp` — a 1-of-N veto,
not a quorum. The one exception: a canceller cannot veto the op that revokes
its own `CANCELLER` role (no one blocks their own removal); every other
pending op, including a revocation targeting a *different* canceller, stays
vetoable.

**The owner's immediate-revoke path cannot touch cancellers.**
`revoke_role_immediate` — the owner's bypass-the-timelock emergency
de-authorization path — only accepts `GUARDIAN`/`ORACLE`.
`PROPOSER`/`EXECUTOR`/`CANCELLER` revocations always ride the timelock and
the single-veto rule above, so a compromised owner key cannot instantly
strip the council.

**Recovery tier breaks a colluding-canceller deadlock.** If enough cancellers
collude to veto every attempt to remove them, the owner-only, non-vetoable
Recovery tier is the only way out. `propose_canceller_reset(new_cancellers,
salt)` schedules a full council reset at the Recovery delay
(`TIMELOCK_RECOVERY_MIN_DELAY_LEDGERS = 518_400` ledgers, ~30 days); once
matured, `execute_canceller_reset(executor, new_cancellers, salt)` revokes
every non-owner `CANCELLER` holder and grants `CANCELLER` to each address in
`new_cancellers` (the owner keeps its own `CANCELLER`). Recovery ops are
marked non-cancellable, so no canceller — captured or not — can block them.
The long public delay is deliberate: it is slow enough that Recovery cannot
serve as a quiet theft path even for a compromised owner multisig, while
still being the only mechanism that can outlast a captured council.

These are dedicated entrypoints, not config-driven `make` verbs — a
`Vec<Address>` argument doesn't fit the JSON-config action dispatcher (see
`make help`). Invoke them directly:
```bash
make invoke FN=propose_canceller_reset \
  ARGS='--new_cancellers ["G...","G...","G..."] --salt <64-hex>' NETWORK=<network>
# ... wait out the ~30-day Recovery delay, then:
make invoke FN=execute_canceller_reset \
  ARGS='--executor null --new_cancellers ["G...","G...","G..."] --salt <64-hex>' NETWORK=<network>
```

---

## 12. Upgrades

In-place upgrades via governance (each is timelocked):
```bash
make <network> upgradeController       # upload + upgrade controller
make <network> upgradeGovernance       # upload + upgrade governance (self-timelock)
make <network> upgradePoolTemplate     # upload + set new pool template hash
make <network> upgradePools            # upgrade the central pool to the template
make <network> upgradeAll              # pool template + controller + pool, then unpause
```

---

## 13. Gotchas

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
