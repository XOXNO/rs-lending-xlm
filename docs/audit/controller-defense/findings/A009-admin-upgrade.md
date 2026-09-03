# A009 — ControllerAdmin `only_owner` set_* / upgrade vs governance timelock

- Agent: A009
- Theme: T1
- Severity: medium (deployment / trust-assumption residual; not a missing `#[only_owner]` gate)
- Status: defended (under the documented ownership wiring); partial until Sensitive delay is restored
- Paths:
  - `contracts/controller/src/lib.rs:562-847` (`impl ControllerAdmin`)
  - `contracts/controller/src/governance.rs` (`init`, `upgrade`, `migrate`, pause/unpause, ownership)
  - `contracts/controller/src/config/{registry,asset,spoke}.rs`, `contracts/controller/src/markets.rs`
  - `interfaces/controller/src/admin.rs`, `interfaces/governance/src/lib.rs` (`AdminOperation`)
  - `contracts/governance/src/op.rs` (`resolve_op` / delay tiers)
  - `contracts/governance/src/timelock/immediate.rs` (guardian hot path)
  - `contracts/governance/src/constants.rs` (`TIMELOCK_SENSITIVE_MIN_DELAY_LEDGERS`)
  - `docs/explanation/threat-model.md` Trust roots + Known gaps; `docs/reference/invariants.md` INV-AUTH-01/04/05; `STRIDE.md` I31/I32, Tamper.7, Elevation.1/2
- Defense: Every mutating `ControllerAdmin` entrypoint except `accept_ownership` is `#[only_owner]`. Deploy path sets the controller owner to the governance contract. Typed `AdminOperation` proposals map controller mutators onto Standard or Sensitive delay tiers. Guardian immediate verbs only tighten (pause, flag ratchet, empty hub/spoke). Controller `upgrade` forces pause before Wasm replace.
- Gap: The controller itself has **no** timelock. Delay is entirely a property of **who owns** the controller. Sensitive floor is currently 12 ledgers (~1 min) — threat-model release blocker. Several controller setters rely on governance propose-time validation (nonzero Wasm, contract-address checks) that the controller body does not re-check. `pause` / `set_spoke_asset_flags` are not `AdminOperation` variants (by design: only guardian-immediate under correct ownership).
- Impact: If owner ≠ governance (mis-deploy or completed `TransferCtrlOwnership` to a hot key), every `#[only_owner]` admin verb — including `upgrade`, `upgrade_pool`, `set_price_aggregator`, `force_socialize_bad_debt` — is **immediate**, unbounded protocol control. With correct ownership but unrestored Sensitive floor, the reaction window for Wasm/ownership/oracle-pointer changes collapses to ~1 minute. Funds at risk: entire pool book + all account authority (NFT) + price pointer.
- Evidence: symbols and INV ids below; tests in `contracts/controller/tests/governance/access.rs`, `contracts/governance/tests/{timelock,flows,resolve_op,self_timelock}.rs`, integration `tests/integration/flows/admin.sh` (upgrade pauses by design).
- Opinion: Code matches the threat-model story. The audit question is not “is admin ungated?” — it is “does this deployment keep the owner = governance timelock, and is the Sensitive floor production-sized?” Treat any owner that is not the governance contract as a critical configuration defect, not a residual of the controller crate.

---

## 1. Method

1. Enumerated every `ControllerAdmin` method in `lib.rs:562-847` and recorded `#[only_owner]` / pause macros / body callee.
2. Cross-walked each mutator to `AdminOperation` in `interfaces/governance/src/lib.rs:86-129` and to `resolve_op` delay tier in `contracts/governance/src/op.rs:124-409`.
3. Cross-walked guardian hot paths in `timelock/immediate.rs` against the same controller entrypoints.
4. Compared the resulting immediate-vs-delayed matrix to `docs/explanation/threat-model.md` Trust roots (`:33-75`), Known gaps / Sensitive delay (`:258-268`), upgrade controls (`:193`), INV-AUTH-01/04/05, STRIDE I31/I32.
5. Inspected controller-side validation depth vs governance propose-time validation (defense in depth if owner ≠ governance).

---

## 2. Trust assumption (the load-bearing fact)

`#[only_owner]` on the controller does **not** encode a delay. Threat-model table:

| Contract | Owner | Delay on an owner action |
|---|---|---|
| Controller | Governance timelock | Delayed. Typed `AdminOperation` |

Source: `docs/explanation/threat-model.md:39-48`.

Wiring that makes the assumption true:

| Step | Evidence |
|---|---|
| Governance deploys controller with `admin = governance` | `contracts/governance/src/deploy.rs:44` — `deploy_v2(wasm_hash, (env.current_contract_address(),))` |
| Controller constructor sets Ownable owner | `lib.rs:88-90` → `governance::init` → `ownable::set_owner` (`governance.rs:15-16`) |
| Init leaves protocol paused | `governance.rs:33` `pausable::pause` |
| Pool / position-NFT owners are the controller | `markets.rs` deploy passes `env.current_contract_address()`; INV-AUTH-01 |

Consequence: a call that reaches `ControllerAdmin` with a successful owner auth is, in the intended deployment, either:

1. **Delayed** — governance `propose` → wait tier delay → `execute` / `execute_self` → `ControllerAdminClient::*`, or
2. **Immediate (guardian)** — governance `timelock/immediate.rs` role-gated helper → same client, still authenticating as the owner contract.

There is no third public scheduling path: `propose` only accepts `AdminOperation` (`lifecycle.rs:27-60`); raw `execute(target, function, args, …)` can only run an operation id that was scheduled that way.

**If the assumption fails** (EOA/multisig owns the controller, or `TransferCtrlOwnership` completed to a non-timelock address), every row marked “Delayed” below collapses to Immediate. That is Elevation.1 / Tamper.7 with the brake removed.

---

## 3. ControllerAdmin surface inventory

Macros are on the impl in `lib.rs`, not on the trait (`interfaces/controller/src/admin.rs`).
No `ControllerAdmin` mutator carries `#[when_not_paused]` — intentional so pause/unpause/upgrade/config work during halt (A001 expected policy).

| # | Entrypoint | `lib.rs` | `#[only_owner]` | Body | Controller-side checks (summary) |
|---|---|---:|---|---|---|
| 1 | `set_swap_aggregator` | 566 | yes | `config::registry::set_swap_aggregator` | store + event; **no** contract-address check |
| 2 | `set_price_aggregator` | 573 | yes | `config::registry::set_price_aggregator` | store + event; **no** contract-address check |
| 3 | `set_accumulator` | 580 | yes | `config::registry::set_accumulator` | store + event; **no** contract-address check |
| 4 | `set_position_limits` | 588 | yes | `registry::set_position_limits` | both limits in `1..=POSITION_LIMIT_MAX` |
| 5 | `set_min_borrow_collateral_usd` | 596 | yes | `registry::set_min_borrow_collateral_usd` | `floor_wad >= 0` |
| 6 | `set_position_manager` | 606 | yes | `storage::set_position_manager` | write active flag; **no** address class check |
| 7 | `approve_blend_pool` | 616 | yes | `set_blend_pool_approval(…, true)` | store + event |
| 8 | `revoke_blend_pool` | 627 | yes | `set_blend_pool_approval(…, false)` | store + event |
| 9 | `create_hub` | 637 | yes | `config::spoke::create_hub` | new empty active hub |
| 10 | `add_spoke` | 643 | yes | `config::spoke::add_spoke` | new spoke, default curve |
| 11 | `remove_spoke` | 650 | yes | `remove_spoke` | must not already be deprecated |
| 12 | `set_spoke_liquidation_curve` | 658 | yes | `set_spoke_liquidation_curve` | `validate_liquidation_curve` |
| 13 | `add_asset_to_spoke` | 682 | yes | `config::asset::add_asset_to_spoke` | risk/fees/caps; spoke not deprecated; not already listed |
| 14 | `edit_asset_in_spoke` | 690 | yes | `edit_asset_in_spoke` | same validation; **can clear flags** (full rewrite) |
| 15 | `set_spoke_asset_flags` | 698 | yes | `set_spoke_asset_flags` | `require_flag_ratchet` (false→true only) |
| 16 | `remove_asset_from_spoke` | 718 | yes | `remove_asset_from_spoke` | listed; zero spoke usage |
| 17 | `deploy_pool` | 729 | yes | `markets::deploy_pool` | one-shot; controller becomes pool owner |
| 18 | `deploy_position_nft` | 736 | yes | `markets::deploy_position_nft` | one-shot; controller authorized minter |
| 19 | `create_liquidity_pool` | 753 | yes | `markets::create_liquidity_pool` | hub active; `params.asset_id == asset` |
| 20 | `upgrade_liquidity_pool_params` | 768 | yes | `markets::upgrade_liquidity_pool_params` | accrue then `pool_update_params` |
| 21 | `upgrade_pool` | 778 | yes | `markets::upgrade_pool` | `pool_upgrade_call`; **no** pause of controller |
| 22 | `upgrade_position_nft` | 785 | yes | `markets::upgrade_position_nft` | `nft_upgrade_call`; **no** controller pause |
| 23 | `force_socialize_bad_debt` | 793 | yes | `process_force_socialize_bad_debt` | not flash-loaning; InsolventOnly gate (bypasses dust cap) |
| 24 | `pause` | 802 | yes | `governance::pause` | `pausable::pause` (panics if already paused) |
| 25 | `unpause` | 808 | yes | `governance::unpause` | `pausable::unpause` |
| 26 | `upgrade` | 815 | yes | `governance::upgrade` | pause-if-needed, then `update_current_contract_wasm` |
| 27 | `migrate` | 822 | yes | `governance::migrate` | `new_version > current` |
| 28 | `transfer_ownership` | 835 | yes | `governance::transfer_ownership` | two-step pending owner + TTL |
| 29 | `get_app_version` | 827 | **no** | read instance `AppVersion` | view |
| 30 | `accept_ownership` | 844 | **no** | `ownable::accept_ownership` | pending-owner auth (ownership primitive) |

`renew_then!` renews controller instance TTL before every owner mutator body (`lib.rs:65-69`).

Controller exposes **no** `renounce_ownership` (grep-clean across `contracts/controller` and `interfaces/controller`) — unlike router / XOXNO oracle (threat-model DoS.10). Good.

---

## 4. Immediate vs delayed matrix (intended deployment)

Legend:

- **G-imm** — governance `timelock/immediate.rs`, role-gated, no propose/delay.
- **Std** — `AdminOperation` → `DelayTier::Standard` (`get_min_delay`).
- **Sens** — `AdminOperation` → `DelayTier::Sensitive` (`min.max(TIMELOCK_SENSITIVE_MIN_DELAY_LEDGERS)`).
- **Owner-direct** — callable on controller by whoever holds Ownable; under correct wiring only governance can succeed, and governance has no typed propose for that verb unless listed.

### 4.1 Delayed via typed `AdminOperation` (Standard)

| Controller entrypoint | `AdminOperation` | `resolve_op` notes |
|---|---|---|
| `set_accumulator` | `SetAccumulator` | no propose-time contract check |
| `set_position_limits` | `SetPositionLimits` | validates limits at propose |
| `set_min_borrow_collateral_usd` | `SetMinBorrowCollateralUsd` | `floor_wad >= 0` at propose |
| `create_hub` | `CreateHub` | also G-imm (guardian) |
| `add_spoke` | `AddSpoke` | also G-imm (guardian) |
| `remove_spoke` | `RemoveSpoke` | |
| `add_asset_to_spoke` | `AddAssetToSpoke` | propose-time risk/fee/cap validation |
| `edit_asset_in_spoke` | `EditAssetInSpoke` | same; **reopen / flag-clear path** (INV-AUTH-04) |
| `remove_asset_from_spoke` | `RemoveAssetFromSpoke` | |
| `approve_blend_pool` | `ApproveBlendPool` | require contract address |
| `revoke_blend_pool` | `RevokeBlendPool` | require contract address |
| `create_liquidity_pool` | `CreateLiquidityPool` | token decimals + market creation validation |
| `upgrade_liquidity_pool_params` | `UpgradeLiquidityPoolParams` | `params.verify` |
| `deploy_pool` | `DeployPool` | nonzero Wasm |
| `deploy_position_nft` | `DeployPositionNft` | nonzero Wasm |
| `set_spoke_liquidation_curve` | `SetSpokeLiquidationCurve` | `validate_liquidation_curve` |
| `unpause` | `Unpause` | **only** reopen for global pause |
| `migrate` | `MigrateController` | version bump after upgrade |

Suggested Standard floor in constants: `TIMELOCK_MIN_DELAY_LEDGERS = 34_560` (~2 days at 5s ledgers) — constructor-configured `min_delay` must be verified at deploy (`threat-model.md:266-268`).

### 4.2 Delayed via typed `AdminOperation` (Sensitive)

| Controller entrypoint | `AdminOperation` | Blast radius |
|---|---|---|
| `set_swap_aggregator` | `SetSwapAggregator` | strategy router pointer; propose requires contract address |
| `set_price_aggregator` | `SetPriceAggregator` | dual write: gov storage + controller (`apply_self_op`); Sensitive self_op |
| `set_position_manager` | `SetPositionManager` | who may be granted as account delegates |
| `upgrade_pool` | `UpgradePool` | full pool Wasm (all market accounting) |
| `upgrade_position_nft` | `UpgradePositionNft` | account-ownership token Wasm |
| `upgrade` | `UpgradeController` | controller Wasm; forces pause first |
| `transfer_ownership` | `TransferCtrlOwnership` | owner must propose; new owner must be contract address |
| `force_socialize_bad_debt` | `ForceSocializeBadDebt` | socializes insolvent debt without dust cap |

`operation_delay` (`timelock/mod.rs:41-47`): Sensitive = `min_delay.max(TIMELOCK_SENSITIVE_MIN_DELAY_LEDGERS)`.

**Current floor:** `TIMELOCK_SENSITIVE_MIN_DELAY_LEDGERS = 12` with explicit TEMPORARY comment targeting production `120_960` (`constants.rs:12-19`). Threat-model marks shipping unrestored as a **release blocker** (`threat-model.md:258-264`). Status of this finding: **partial** until that constant is restored via `UpgradeGov`.

### 4.3 Immediate (guardian) — still `#[only_owner]` on controller

Governance authenticates the guardian, then invokes the controller **as owner**:

| Governance API | Role | Controller call | Bound |
|---|---|---|---|
| `pause` | GUARDIAN | `ControllerAdmin::pause` | Tightens only; unpause is Std `Unpause` |
| `set_spoke_asset_flags` | GUARDIAN | `set_spoke_asset_flags` | `require_flag_ratchet` — cannot clear |
| `create_hub` | GUARDIAN | `create_hub` | empty structure; no value move |
| `add_spoke` | GUARDIAN | `add_spoke` | empty structure; default curve |

Source: `timelock/immediate.rs:16-62`; threat-model “Actions with no reaction window” (`:53-75`); INV-AUTH-04.

These four controller entrypoints are `#[only_owner]` but **not** all present as `AdminOperation`:

| Entrypoint | In `AdminOperation`? | Intended reachability under owner=gov |
|---|---|---|
| `pause` | **No** | G-imm only |
| `set_spoke_asset_flags` | **No** | G-imm only (clearing requires Std `EditAssetInSpoke`) |
| `create_hub` | Yes (Std) **and** G-imm | either |
| `add_spoke` | Yes (Std) **and** G-imm | either |

Absence of `Pause` / `SetSpokeAssetFlags` from `AdminOperation` is deliberate asymmetry: proposers cannot schedule a pause delay; guardians act now; reopen is delayed. Matches ADR-0007 / INV-AUTH-04 / Elevation.2.

### 4.4 Not delayed / not owner-gated (by design)

| Entrypoint | Auth | Notes |
|---|---|---|
| `get_app_version` | none | read-only |
| `accept_ownership` | pending owner via `ownable::accept_ownership` | completes Sensitive `TransferCtrlOwnership`; access-control checker treats this as an ownership primitive |

---

## 5. Upgrade surfaces in detail

### 5.1 Controller Wasm — `upgrade` / `UpgradeController`

```38:44:contracts/controller/src/governance.rs
pub(crate) fn upgrade(env: &Env, new_wasm_hash: &BytesN<32>) {
    if !pausable::paused(env) {
        pausable::pause(env);
    }
    env.deployer()
        .update_current_contract_wasm(new_wasm_hash.clone());
}
```

Properties:

- `#[only_owner]` at `lib.rs:814-816`.
- Always ends paused (idempotent if already paused).
- Does **not** bump `AppVersion` — that is a separate `migrate` / `MigrateController` (Std).
- Does **not** validate nonzero hash on the controller; governance `resolve_op` does (`op.rs:344-346`).
- Threat-model / STRIDE cite this force-pause as the upgrade control (INV-AUTH-05 row at threat-model `:193`; STRIDE I32, Tamper.7).
- Integration: `tests/integration/flows/admin.sh` — “upgrade (pauses by design)” then `unpause_after_upgrade`.

### 5.2 Pool Wasm — `upgrade_pool` / `UpgradePool`

- Owner-gated controller → `pool_upgrade_call` (`markets.rs:107-109`).
- Sensitive tier; nonzero hash at propose.
- Does **not** pause the controller or the pool’s user surface by itself. Pool mutators remain `#[only_owner]` (controller-only), so hostile pool Wasm is still only reachable through the (possibly also upgraded) controller — but accounting inside the pool can change arbitrarily after execute.
- Pool has **no** ownership-transfer entrypoint (STRIDE I31); owner changes only via pool Wasm upgrade.

### 5.3 Position NFT Wasm — `upgrade_position_nft` / `UpgradePositionNft`

- Sensitive; nonzero hash at propose.
- Replaces account-ownership authority code. Controller does not force-pause around this call.
- NFT mint/burn remain controller-authorized in the intended build; a malicious NFT Wasm can break `owner_of` / transfer semantics for every account.

### 5.4 Satellite deploy (one-shot)

- `deploy_pool` / `deploy_position_nft` — Std; panic if already set (`PoolAlreadyDeployed` / `PositionNftAlreadyDeployed`).
- Establishing these pointers is a Standard-delay governance act, not Sensitive — acceptable because first deploy is empty, but a wrong Wasm hash at first deploy is still catastrophic. Nonzero-hash check exists at propose.

### 5.5 What is *not* on ControllerAdmin

- Price-aggregator Wasm upgrade is **not** a controller entrypoint; it is Sensitive `UpgradePriceAggregator` targeting the aggregator directly (`op.rs:331-337`). Aggregator has no owner-transfer; replacement of the *pointer* is Sensitive `SetPriceAggregator` (dual write).
- Swap-aggregator upgrade is **not** routed through the controller in current `op.rs` / `markets.rs` (STRIDE Elevation.6 text mentioning `UpgradeSwapAggregator` / `markets.rs:123` is **stale** relative to this tree). Router remains an out-of-governance trust root per threat-model.

---

## 6. set_* trust and validation depth

### 6.1 High-blast pointer setters

| Setter | Tier | If malicious address sticks | Controller validates address? | Gov propose validates? |
|---|---|---|---|---|
| `set_price_aggregator` | Sens | Every solvency/liquidation price | no | yes (`require_contract_address`) + gov local storage update |
| `set_swap_aggregator` | Sens | Strategy swap routing | no | yes |
| `set_accumulator` | Std | Revenue destination | no | **no** |
| `set_position_manager` | Sens | Delegate eligibility | no | **no** |
| `approve_blend_pool` | Std | Migration target trust | no | yes (contract address) |

Defense-in-depth gap (info under correct ownership; high if owner is hot): controller bodies trust the caller. Governance is the intended filter. `SetAccumulator` lacking a contract-address check at propose is a small gov-side hole (EOA treasury still “works” but is unusual).

### 6.2 Risk-parameter setters

- `edit_asset_in_spoke` / `add_asset_to_spoke`: full risk vector (LTV, threshold, bonus, fees, caps, flags). Std delay. Flag **clearing** only here — guardian cannot (`require_flag_ratchet` in `config/asset.rs:137-148`).
- `set_spoke_liquidation_curve`: Std; curve domain validated both at propose and in controller.
- `set_position_limits` / `set_min_borrow_collateral_usd`: Std; bounded in controller registry.
- `upgrade_liquidity_pool_params`: Std; changes IRM after accrual — rate grief / utilization dynamics, not Wasm.

### 6.3 `force_socialize_bad_debt`

- Sensitive (socializes loss onto suppliers without dust cap).
- Still requires insolvency gate (`BadDebtGate::InsolventOnly`) and `require_not_flash_loaning`.
- Not a silent seizure of healthy accounts; still a Sensitive governance act with supplier P&L impact.

---

## 7. Ownership transfer

| Step | Path | Delay |
|---|---|---|
| Propose new controller owner | `TransferCtrlOwnership` → `transfer_ownership` | Sensitive; proposer must be gov owner (`lifecycle.rs:43-49`); `new_owner` must be contract address |
| Accept | `accept_ownership` | none (pending owner); renews instance TTL |

Two-step design prevents a single mistaken execute from flipping owner without acceptor collusion. After accept, **all** subsequent `#[only_owner]` calls follow the new owner’s delay properties (or lack thereof). Transferring to a non-timelock contract is therefore the highest-leverage Sensitive operation after Wasm upgrades.

Controller init emits `ownership_transfer_completed` so indexers learn genesis owner (`governance.rs:17`; test `init_emits_owner_and_default_limits`).

Controller uses Ownable only — no parallel `access_control` admin (`tests/governance/access.rs:init_sets_owner_not_access_control_admin`).

---

## 8. Threat-model / invariant alignment

| Claim | Live code verdict |
|---|---|
| Controller owner actions are delayed via typed `AdminOperation` | **Holds** iff owner = governance; delay is not in the controller crate |
| Guardian pause / flags / empty hub-spoke are immediate; unpause / flag clear are not | **Holds** — G-imm + ratchet + Std `Unpause` / `EditAssetInSpoke` |
| Upgrades (`UpgradeController`/`Pool`/`PositionNft`) are Sensitive; controller upgrade forces pause | **Holds** for mapping and pause; Sensitive **floor** currently 12 ledgers |
| INV-AUTH-01 one ownership chain | **Holds** in deploy wiring; pool/NFT owned by controller |
| INV-AUTH-04 emergency only tightens | **Holds** at `require_flag_ratchet` + no G-imm unpause |
| INV-AUTH-05 delay cannot be shortened | **Holds** for `UpdateGovDelay` (nondecreasing); does **not** protect the hardcoded Sensitive floor constant (needs `UpgradeGov` to raise) |
| Threat-model Known gap: Sensitive = 12 ledgers | **Confirmed** `constants.rs:19` |
| No controller `renounce_ownership` | **Confirmed** |

---

## 9. Residual gaps and severity

| ID | Finding | Severity | Status |
|---|---|---|---|
| G1 | Controller has no native delay; trust = owner identity | medium (config) | defended in code; operational gate |
| G2 | `TIMELOCK_SENSITIVE_MIN_DELAY_LEDGERS = 12` | medium (release blocker per threat-model) | partial |
| G3 | Controller setters omit address/Wasm checks that gov propose applies | low / info | accepted under owner=gov; becomes high if G1 fails |
| G4 | `SetAccumulator` / `SetPositionManager` lack propose-time contract checks | info | minor gov validation gap |
| G5 | `upgrade_pool` / `upgrade_position_nft` do not force controller pause | info | accepted; pool has no user entry surface; NFT reads are live |
| G6 | `pause` / `set_spoke_asset_flags` omitted from `AdminOperation` | none (design) | defended — matches INV-AUTH-04 |
| G7 | Stale STRIDE Elevation.6 reference to controller-routed swap-aggregator upgrade | info (docs drift) | out of controller scope; note for A020 |

No missing `#[only_owner]` on a mutating `ControllerAdmin` entrypoint. No ungated upgrade. No guardian path that unpauses or clears flags.

---

## 10. Impact quantification (conditional)

| Scenario | Blast radius |
|---|---|
| Correct owner + production delays | Reaction window = Standard/Sensitive floors; canceller can drop pending ops; guardian can pause/tighten immediately |
| Correct owner + Sensitive=12 | Wasm / oracle pointer / ownership / force-socialize effectively ~1 minute delay — treat as near-immediate for MEV and key-compromise response |
| Owner is hot key / multisig | Immediate `upgrade` (paused but new code), `upgrade_pool`, `upgrade_position_nft`, price/swap pointer swaps, unpause, flag clears via `edit_asset_in_spoke`, force socialize — **protocol-total** loss possible |
| Guardian key compromise only | Immediate pause + tighten flags + create empty hubs/spokes; cannot unpause, clear flags, upgrade, or move pointers (INV-AUTH-04) |
| PROPOSER compromise only | Can schedule any `AdminOperation` after role grant (itself Sensitive); cannot skip delay; canceller / expiry remain brakes |

---

## 11. Tests / evidence pointers

| Area | Location |
|---|---|
| Owner init / accept ownership | `contracts/controller/tests/governance/access.rs` |
| Sensitive delay for upgrades / aggregator | `contracts/governance/tests/timelock.rs` (`propose_upgrade_pool_uses_sensitive_delay`, price aggregator) |
| Zero-hash reject on upgrades | `contracts/governance/tests/flows.rs` |
| Immediate unpause rejected | `flows.rs` / `timelock.rs` (`execute_immediate` Unpause fails outside testing feature) |
| Resolve tiers for NFT / force socialize | `contracts/governance/tests/resolve_op.rs` |
| Delay nondecreasing | `contracts/governance/tests/self_timelock.rs` |
| Flag ratchet | `contracts/controller/tests/config/asset_flags.rs` |
| Live upgrade pauses | `tests/integration/flows/admin.sh` |

---

## 12. Opinion

The `ControllerAdmin` surface is correctly owner-gated and correctly **without** pause gates. The security property “admin changes are delayed” is **not** a property of `lib.rs` / `governance.rs` alone — it is the composition of (1) Ownable owner = governance, (2) typed `AdminOperation` scheduling, (3) Standard/Sensitive floors, (4) guardian ratchet for the hot subset.

Audit sign-off for this theme should verify deployment facts, not invent a controller-local timelock:

1. On-chain controller owner == governance contract.
2. Configured `min_delay` and `TIMELOCK_SENSITIVE_MIN_DELAY_LEDGERS` meet production policy (restore 120_960).
3. No completed `TransferCtrlOwnership` to a non-timelock.
4. Guardian/ORACLE role holders are known and rotatable via `revoke_role_immediate`.

Code review of `#[only_owner]` placement: **pass**. Timelock assumption vs threat-model: **aligned**, with the documented Sensitive-floor partial.
