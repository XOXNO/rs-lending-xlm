# Threat model

## Purpose

This document names what the protocol protects, who can attack it, and where
the protection stops. It is a review guide. It does not claim that the listed
controls are sufficient in every deployment.

Read it with two companion documents:

| Document | Contents |
|---|---|
| [`STRIDE.md`](../../STRIDE.md) | The per-threat matrix, with likelihood and residual ratings |
| [`docs/reference/invariants.md`](../reference/invariants.md) | The `INV-*` properties that the controls must hold |

This document is the narrative. It states the trust roots, the boundaries, and
the gaps. Where a control exists, it points at the `INV-*` identifier instead of
repeating the argument. Where a control does **not** exist, it says so in
[Known gaps](#known-gaps).

## Protected assets

- Supplier claims and pool cash.
- Borrower collateral and account authority.
- Market indexes, debt totals, and revenue entitlement.
- Price integrity used for solvency and liquidation.
- Governance authority, timelock state, and emergency response.
- Protocol liveness during oracle failure, liquidation, and recovery.
- Fee and referral balances held by the swap aggregator.
- XOXNO oracle feed integrity: the signer set, the threshold, and the
  freshness windows.

## Trust roots

`#[only_owner]` does not mean the same thing in every contract. The delay that
applies to an owner action depends on **who the owner is**. This table is the
most important one in the document.

| Contract | Owner | Delay on an owner action |
|---|---|---|
| Controller | Governance timelock | Delayed. Typed `AdminOperation` |
| Pool | Controller | None directly. The controller is the only caller |
| Price aggregator | Governance timelock | Delayed, except the ORACLE role band write |
| Position NFT | Controller | Delayed. Reached through the controller owner gate |
| Governance | Governance owner key | Delayed, except the immediate verbs below |
| **XOXNO oracle** | **A standalone key** | **None. Immediate** |
| **Swap aggregator** | **A standalone key** | **None. Immediate** |
| DeFindex adapter | No owner | Not applicable. It has no admin surface |

The two bold rows are trust roots that governance does not control. Verify the
deployed owner of each before you enable a market that depends on it.

### Actions with no reaction window

Governance has **two** execution paths, not one. The timelock path validates a
typed `AdminOperation`, waits out the delay, then executes. A second path in
`contracts/governance/src/timelock/immediate.rs` skips propose, delay, and
execute entirely. It is role-gated, not owner-gated.

These paths commit immediately. A delay cannot absorb a key compromise here.

| Path | Caller | Bounded by |
|---|---|---|
| `pause` | GUARDIAN | Tightens only. Unpause is timelocked |
| `set_spoke_asset_flags` | GUARDIAN | `require_flag_ratchet`. Flags move false to true only |
| `create_hub`, `add_spoke` | GUARDIAN | Creates empty structures. It does not move value |
| `set_sanity_band` | ORACLE role | Tighten-only ratchet. Widening is timelocked |
| `revoke_role_immediate` | Governance owner | GUARDIAN and ORACLE roles only |
| `upgrade` on the XOXNO oracle | Oracle owner | Nothing |
| `upgrade`, `sweep_balance`, `set_referral_owner` on the swap aggregator | Router owner | Nothing |

A role is granted through the timelock, so this surface is reachable only after
a delay. Once a role is held, every action above is instant. Note that the
delay is the same one that is currently set to about one minute. See
[Deployment gates](#deployment-gates).

## Actors

| Actor | Capability |
|---|---|
| Anonymous caller | Invokes public user and keeper actions with its own authorization |
| Account owner | Holds the position NFT. Controls every risk-increasing action |
| Delegate | Acts only when the account lists it **and** governance approves it |
| Liquidator | Repays unhealthy debt for discounted collateral |
| Guardian | Pauses and tightens. It cannot reopen |
| Governance owner | Proposes ownership transfer and canceller recovery. Holds every role at deployment |
| PROPOSER | Schedules typed timelocked operations |
| EXECUTOR | Optional execution gate. Execution is permissionless when no executor is named |
| CANCELLER | Cancels pending operations, except recovery operations |
| ORACLE role | Narrows sanity bands immediately. Widening is timelocked |
| Oracle signer | Submits signed price observations |
| Keeper | Calls permissionless maintenance |
| Token contract | May transfer an amount other than the requested one |
| Router and venues | May be malicious, faulty, or economically poor |
| Flash receiver | Runs arbitrary callback code |
| External vault | May report arbitrary caller identity and NAV |

The unprivileged mutation surface is declared, not implied.
`make access-control-check` reports 207 entrypoints and 29 declared
permissionless lines. `scripts/permissionless_entrypoints.txt` carries the
justification and the `INV-*` identifier for each one.

## Trust boundaries

### User to controller

User parameters, tokens, and routing intent are untrusted. The controller
authenticates the caller, then applies account, listing, and price rules.

Solvency gates run **after** the mutation, in the same transaction.
`require_post_pool_risk_gates` runs from `strategy_finalize`. The gate depends
on transaction rollback. A path that mutates state and never reaches its gate
is a critical defect (INV-RISK-01).

### Controller to pool

The controller is the only party that mutates the pool. This stops a direct
state change from skipping a risk check. It also makes controller correctness
and the ownership wiring critical (INV-AUTH-01).

### Controller to position NFT

The position NFT is the account-ownership authority. There is one token per
account, and the token id equals the account id. Mint and burn need the
controller's authorization. `owner_of` is a public read. The controller reads
it live on every account access and never caches it, so an ownership change
takes effect immediately (INV-STOR-03).

Transfer is a standard NFT operation. The controller does not gate it and
cannot prevent it. A transfer lazily revokes any delegate grant that the
previous owner made.

**`approve` and `approve_for_all` hand over the whole position.** The NFT is
the account's ownership authority. An address that can transfer the token can
take over the collateral, the debt, and the withdraw rights the moment it calls
`transfer_from`. A phishing signature that reads as "approve this marketplace"
is, for this token, "hand over my whole loan account".

`contracts/position-nft/src/contract.rs` implements `NonFungibleToken` and
overrides `token_uri` only. `approve`, `approve_for_all`, and `transfer_from`
are the stock OpenZeppelin implementations. There is no approval hook, no
allowlist, and no controller callback on transfer. There is no protocol-level
mitigation. Wallets and front ends must state this risk in the approval prompt.

### Controller to router and tokens

The router is untrusted. Token behaviour is not assumed to match a requested
transfer amount. The controller measures balances, scopes pull authority, and
rechecks solvency after external work (INV-STRAT-01, INV-STRAT-02).

**The controller does not bound slippage.** See
[Known gaps](#known-gaps).

### Controller to price system

The controller accepts only a validated, complete price snapshot. A source
failure or an ambiguity stops valuation-dependent activity. The protocol does
not choose a fallback price (INV-ORACLE-01, INV-ORACLE-02, INV-ORACLE-03).

### Governance to protocol

Governance sets policy, and its ordinary power is delayed. The guardian is more
available but less powerful. It can tighten, but it cannot reopen (INV-AUTH-04).

### Vault to adapter

The DeFindex adapter authenticates the calling vault with `from.require_auth()`
in `deposit` and `withdraw`. It keys one controller account per vault address
through `resolve_vault_account`. Positions are supply-only. The adapter exposes
no borrow path.

## Attack surfaces

Each row states what an attacker tries, the control that stops it, and the
invariant that the control must hold.

| Surface | Attacker goal | Control | Invariant |
|---|---|---|---|
| Account operations | Borrow against or withdraw another account | Owner-or-delegate authorization, ownership read live from `owner_of`, immutable spoke binding, post-operation gates | INV-AUTH-02, INV-AUTH-06, INV-RISK-01 |
| Non-standard tokens | Claim more value than the token delivered | Measured receipt on every inbound path, internal cash book, zero-share rejection, backing-shortfall gate | INV-ACCT-02, INV-ACCT-03, INV-ACCT-04 |
| Oracle integrity | Manipulate, stale, or race a price source | Source admission policy, freshness and sanity checks, dual-source agreement, one snapshot per mutation, fail-closed consumption | INV-ORACLE-01..04 |
| Oracle source authority | Fabricate a feed value from a subset of signers | Signer authentication, price bounds, `require_not_future`, `require_fresh_submission`, `require_monotonic_package`, median at threshold, `independence` policy | INV-ORACLE-01, INV-ORACLE-04 |
| Liquidation | Liquidate a healthy account or over-seize collateral | Health gate, close bound, `max_hf_preserving_bonus_bps`, measured repayment, seizure scaled to receipt | INV-LIQ-01, INV-LIQ-02, INV-LIQ-03 |
| Share-credit liquidation | Route seized collateral to an account that must not receive it | `resolve_seize_receiver` rejects the liquidated account and requires owner-or-delegate, the same spoke, and `PositionMode` Normal | INV-AUTH-02, INV-LIQ-02 |
| Flash loans | Leave the pool underpaid, or reenter during a callback | `require_wasm_receiver`, exact balance assertions, allowance-based repayment, `require_not_flash_loaning` | INV-FLASH-01, INV-FLASH-02 |
| Flash position | Treat zero-fee strategy debt as a cash flash loan | No repay or net-settle in the path, declared collateral with a positive minimum, measured deltas, receiver is neither controller nor pool | INV-STRAT-04 |
| Strategies and routes | Overspend, retain input, or claim output dishonestly | Exact pull authorization, balance-delta settlement, residue return, `verify_router_output`, final solvency gate | INV-STRAT-01, INV-STRAT-02 |
| Blend migration | Point a migration at a hostile contract | `is_blend_pool_approved` allowlist, `authorize_repay_pulls` capped per pull, `guarded_submit` blocks reentry | INV-STRAT-03 |
| Governance and emergency | Reopen risk with a hot key, or bypass the delay | Typed scheduling, payload identity, readiness and expiry checks, role separation, guardian ratchet | INV-AUTH-04, INV-AUTH-05 |
| Upgrades | Ship code that removes a check | `UpgradeController`, `UpgradePool`, `UpgradePositionNft`, `UpgradeGov` are typed timelocked operations. Controller `upgrade` forces the paused state. The price aggregator has no upgrade entrypoint | INV-AUTH-05 |
| Permissionless maintenance | Create foreign risk through a keeper verb | Every keeper verb calls `require_not_flash_loaning`. Revenue goes only to the configured accumulator. `require_third_party_existing_supply` limits third-party top-ups | INV-AUTH-03 |
| Storage lifetime | Strand a position by archiving one leg | `renew_account` renews the controller entries and calls the NFT `renew` in the same transaction | INV-STOR-01, INV-STOR-02, INV-STOR-03 |
| Dust griefing | Open positions too small to liquidate, or mint value by rounding | `SupplyRoundsToZeroShares` and its siblings revert. A configured floor rejects unprofitable accounts | INV-ACCT-05, INV-RISK-04 |
| Build-surface leakage | Ship a binary that exports test-only entrypoints | `seed_oracle` and `remove_oracle` compile only under `#[cfg(any(test, feature = "testing"))]`. `make wasm-testing-abi-check` gates the artifact | — |
| Router ownership | Raise fees or edit the fee whitelist | Fee setters are owner-gated and bounded by `FEE_CAP`. Per-strategy loss is bounded by the controller's balance-delta settlement | — |
| Governance recovery | Lose administrative capability permanently | `propose_canceller_reset` on the Recovery tier, which the ordinary cancel path cannot stop. `revoke_role_immediate` for the two hot roles | INV-AUTH-05 |

### Public state and MEV

There are no controls, by design. Positions, health factors, prices, and
pending timelock operations are all public. Liquidation is an open race. The
liquidation curve bounds the bonus and ties repayment size to seizure size, so
liquidators compete on speed and not on extractable excess (INV-LIQ-02).

The protocol makes no confidentiality assumption anywhere. This risk is
accepted.

## Known gaps

This section lists what the protocol does **not** fully protect against. Each
item is either an accepted design decision, a deployment decision, or an open
defect. Nothing here is a theoretical concern.

### Deployment gates

These must close before real value is at stake. They are ownership decisions,
not code defects.

**The swap-aggregator owner is a trust root outside governance unless the
controller owns it.** The router's own `upgrade` is a bare `#[only_owner]` with
no timelock. Governance can reach it only when the controller is the router's
owner: `AdminOperation::UpgradeSwapAggregator` is a Sensitive-tier operation
that calls the controller's `#[only_owner] upgrade_swap_aggregator`, which
invokes the router's `upgrade` (`contracts/governance/src/op.rs:315`,
`contracts/controller/src/markets.rs:123`). `make deploy-aggregator` defaults
the router admin to the deploying signer, so verify the deployed owner: with a
standalone key that timelocked path is unreachable and the router stays a trust
root outside governance. The other owner powers below have no governance path
at all.

The router owner is more powerful than the fee cap suggests. Three further
powers are immediate and uncapped:

- `sweep_balance` moves every token balance above the reserved fee amount to
  any recipient.
- `set_referral_owner` reassigns any referral's fee-claim rights.
- `renounce_ownership` is the stock Ownable method. It permanently disables
  every owner-gated router function, including `upgrade`. There is no recovery
  path.

**The XOXNO oracle owner is a trust root outside governance.** `upgrade`,
`add_signer`, `remove_signer`, and `set_threshold` are all immediate. One key
can replace the price-oracle code. Do not enable a market that depends on a
XOXNO feed while an individual key holds this ownership.

**The sensitive timelock delay is set for pre-audit iteration.**
`TIMELOCK_SENSITIVE_MIN_DELAY_LEDGERS` is 12 ledgers, which is about one
minute. The production value is 120_960 ledgers, which is about seven days. The
comment above the constant marks the value as temporary. The value compiles
into the artifact, so restoring it needs a governance-executed `UpgradeGov`.
Until then, every sensitive operation is effectively immediate. Shipping this
constant unrestored is a release blocker.

Note that the code-level floors are floors only. A deployment that configures a
small delay has the same exposure. Verify the configured delay, not only the
constant.

### The controller does not bound slippage

`verify_router_output` asserts that `received > 0` and nothing more. The real
minimum-output bound, `total_min_out`, is carried in the route payload and
enforced **inside the swap aggregator** — the component this document declares
untrusted.

A malicious or maliciously upgraded router can return one unit of output and
keep the rest. The only remaining protection is the post-operation solvency
gate, which permits any loss that leaves the account healthy.

Treat router compromise and router upgrade as an **unbounded-loss** path for
in-flight strategies. This gap and the router-ownership gate above are the same
risk seen from two sides.

### A delegate has complete economic control of the account

This is a design decision, not a defect, and it is the single most important
fact for a user to understand.

`borrow` and the withdraw path both accept an optional recipient address. Both
are gated only by the owner-or-delegate check. A delegate can therefore borrow
the account's whole credit line to its own address, and withdraw the account's
collateral to its own address. The post-operation health-factor gate is the only
bound.

The controls are real. A delegate must also be an active, governance-approved
position manager, the check is re-read at use time, and an NFT transfer revokes
the grant. But the power granted is complete. User-facing documentation must
state this plainly. "Drawn against the account" is not an adequate description.

### The sanity band tightens only

`set_sanity_band` rejects any widening: the new band must satisfy
`min_wad >= stored min` and `max_wad <= stored max`, otherwise it panics with
`SanityBandMustTighten` (`contracts/price-aggregator/src/admin.rs:197`). A
compromised ORACLE key can therefore only narrow a band — still an instant,
per-asset fail-closed kill switch — and restoring a wider band requires the
timelocked `ConfigureAssetOracle`.

The residual risk is availability, not mispricing. The band is the only
backstop for single-source and LP feeds, and narrowing it is the one pricing
control with no reaction window; every sibling operation —
`ConfigureAssetOracle`, `EditOracleTolerance`, `SetPriceAggregator` — is
timelocked. Treat the ORACLE key as price-critical custody.

### Liquidation has no post-condition check

The health-factor gate is a pre-condition only. There is no
`require_post_pool_risk_gates` anywhere on the liquidation path, and
`contracts/pool/src/ops/seize.rs` commits with no `guards::` assertion.
INV-LIQ-04 records this exact gap: its main property is ENFORCED, but it
carries a separate NOT ENFORCED note for the missing post-condition guard. The
resulting market state is relied on to be solvent. Nothing checks it at
runtime.

Every concrete attack built on this has been refuted. The structural gap
remains: the bonus fallback is safe only because it is meant to force a full
close, and seizure scaling can turn it into a partial close when a debt token
under-delivers. A post-condition assertion would close the class rather than
the case.

### The router input pull is measured (closed)

Previously listed here as an open gap. Closed in commit 9da53261: no residual
risk remains on this path.

`execute::run` credits its vault the `transfer_amount_measured` delta, not the
declared `total_in` (`contracts/swap-aggregator/src/execute/mod.rs:81`), so a
fee-on-transfer input token cannot draw the shortfall out of the accrued fee
backing. The router has no token allowlist by design, so that measurement is
the containment.

### The oracle skew anchor is clamped to ledger time

`recompute_aggregate` takes the maximum of submitted package timestamps and
then clamps it with `newest_ts.min(now * MS_PER_SECOND)`
(`contracts/xoxno-oracle/src/aggregation.rs:125`), so a signer submitting at
the future bound cannot raise the anchor and evict honest submissions from the
cluster.

`set_max_relative_skew_seconds` remains a configuration lever worth reviewing.
A skew window that is too narrow still clusters honest submissions apart and
clears the feed, and a feed outage blocks **liquidation** as well as borrowing.
Treat the window width as a configuration hazard.

### The dust gate and the configured floor can drift apart

`BAD_DEBT_USD_THRESHOLD` is a compile-time copy of the **default** of a
governance-settable floor. Raising the floor desynchronises the two. That opens
a band in which a position has no permissionless cleanup path:
`clean_bad_debt` will not admit it, and `force_socialize_bad_debt` is
owner-only.

The documented lever for a separate listing problem is to raise that same
floor. The two controls therefore pull against each other.

### Governance actions are not observable at the governance contract

`contracts/governance/src/events.rs` defines two events. Both are one-time
deploy events. No event is published for `propose`, `execute`, `execute_self`,
`cancel`, `propose_canceller_reset`, a role grant, a role revoke, or any
immediate-path action.

The controller does emit typed events for the configuration changes it
receives, so an operation that lands on the controller is still visible. Two
things are not:

- **A role grant or revoke produces no event at all.** `GrantGovRole` and
  `RevokeGovRole` execute against governance storage. An address that gains
  GUARDIAN or ORACLE is invisible to event-based monitoring.
- **The two execution paths are indistinguishable.** Nothing on-chain
  separates "executed after the full delay" from "committed through the
  immediate path".

This affects the one contract that holds every other contract's admin key.
Monitoring must read governance storage directly. It cannot rely on events.

### Risk views are not flash-guarded

`get_health_factor` and `is_liquidatable` do not call
`require_not_flash_loaning`. They report mid-transaction state during a
`flash_position` callback. This is a composability risk for any external
protocol that reads these views inside a transaction it does not control. It is
not a risk to this protocol's own accounting, because every mutating path
re-derives its own state.

### The DeFindex adapter has no rescue path

The adapter exposes `asset`, `deposit`, `harvest`, `balance`, and `withdraw`.
It has no owner and no admin surface. There is no route to recover an asset
that becomes stranded in it.

### Flash position pays no origination fee

`multiply` charges a strategy origination fee. `flash_position` reaches the same
end state and charges nothing. The asymmetry is declared in the contract
documentation and violates no invariant. It is recorded because the economic
consequence is not written down elsewhere: if the two endpoints are
interchangeable for a borrower, the origination fee is optional in practice.

### An approved Blend pool can be upgraded by its own owner

Approval is a governance decision. An approved pool that its own owner later
upgrades stays trusted until governance removes it.

### Single-source price keys trust one operator completely

Enumerate every price key that is configured with one source. The independence
policy and the dual-source tolerance check only apply when a second source
exists.

## Availability trade-offs

The protocol chooses safety over availability in these cases. They are
intentional. Operators must monitor them.

- A price outage, or one unusable dual-source leg, blocks every
  valuation-dependent action, **including liquidation**.
- The XOXNO oracle publishes no aggregate below its threshold, so signer
  downtime reads as a price outage.
- A global pause blocks new risk but keeps exits open. `supply` and `borrow`
  carry `#[when_not_paused]`. `withdraw`, `repay`, `liquidate`,
  `clean_bad_debt`, `recapitalize`, and `renew_account` do not (INV-HALT-01).
- A per-listing paused flag **does** close exits for that listing, and can
  block its liquidation leg (INV-HALT-02). The listing flag is the stronger
  control, not the global pause.
- Pause is immediate. Unpause is reachable only through the timelocked
  `Unpause` operation. This asymmetry is deliberate and is the safe direction,
  but it means a mistaken pause costs a full delay to undo.
- `require_utilization_below_max` and `require_liquidation_buffer` can block a
  borrow or a withdrawal even when the account is healthy.
- Bounded position counts and route payload limits can reject a valid large
  action.
- An external token, router, oracle, or vault failure can revert a transaction.
- The flash path asserts an exact balance. An asset that does not deliver
  exactly can never be flash-loaned. Never set `is_flashloanable` on such an
  asset.
- Storage archival of an account entry or the NFT `Owner` entry blocks access
  until the entry is restored.

## Accepted residual risks

- Governance and role keys stay high-value operational trust.
- Route quality is not verified on-chain. A valid but poor route can lose value
  inside the account's permitted risk envelope.
- Bad debt socializes pro-rata over **current** suppliers. A supplier that
  exits before a write-down avoids its share. Value is conserved, the same exit
  is available to every supplier at the same moment, and the exit ceiling is
  bounded by the utilization gate.
- Same-market seized collateral becomes treasury revenue rather than netting
  the socialized debt. Value is conserved. The permissionless `recapitalize`
  path returns treasury value to a market.
- External oracle sources and token implementations are outside protocol
  control.
- Formal models and tests carry explicit assumptions. They do not prove
  arbitrary cross-contract behaviour or a deployed configuration.

## Audit priorities

1. Verify deployment ownership, roles, delay, and active configuration.
   Governance deploys the controller and the price aggregator itself, and the
   controller deploys the pool, so those pointers are wired atomically. The
   standalone deploys are not: check the router owner and the XOXNO oracle
   owner against the intended addresses, and check the DeFindex adapter's
   constructor arguments, because it binds its controller, hub, and spoke at
   construction and has no owner to correct them later.
2. Close the three deployment gates in [Known gaps](#known-gaps) before mainnet.
3. Trace value through every external call to its final accounting entry.
4. Fuzz arithmetic boundaries, share rounding, and liquidation composition.
5. Treat a liveness failure as a security finding when it can trap collateral or
   prevent liquidation under plausible conditions.
6. Check that a cited control still exists. An invariant can hold while its
   citation drifts.
