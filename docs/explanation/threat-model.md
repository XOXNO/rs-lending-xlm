# Threat model

## Purpose

This model identifies protected assets, actors, trust boundaries, attack
surfaces, and deliberate residual risks. It is a review guide, not a claim that
the listed controls are sufficient in every deployment.

`STRIDE.md` at the repository root is the exhaustive, per-threat version of
this analysis. This document is the narrative summary. Where the two overlap,
`STRIDE.md` carries the finer detail; where `STRIDE.md` cites this document for
a mitigation, the code anchor is stated here.

## Protected assets

- Supplier claims and pool cash.
- Borrower collateral and account authority.
- Correct market indexes, debt totals, and revenue entitlement.
- Price integrity used for solvency and liquidation.
- Governance authority, timelock state, and emergency response.
- Protocol liveness during oracle failure, liquidation, and recovery.
- Fee and referral balances custodied by the swap aggregator
  (`contracts/swap-aggregator/src/fees.rs`,
  `contracts/swap-aggregator/src/vault.rs`).
- XOXNO oracle feed integrity: signer set, submission threshold, and
  freshness windows (`contracts/xoxno-oracle/src/submit.rs`).

## Actors

| Actor | Relevant capability |
|---|---|
| Anonymous caller | May invoke public user and maintenance actions with own authorization |
| Account owner | Controls risk-increasing actions for its account |
| Delegate | Acts only when both account-listed and governance-approved |
| Liquidator | Repays unhealthy debt for discounted collateral |
| Guardian | Immediately pauses or tightens restrictions |
| Governance owner | Sole proposer of ownership transfer and canceller recovery; holds every role at deployment. Recovery is `#[only_owner]`, not a role (`contracts/governance/src/api.rs`, `propose_canceller_reset`) |
| PROPOSER | Schedules typed timelocked operations |
| EXECUTOR | Optional execution gate; execution is permissionless when no executor is named |
| CANCELLER | Cancels pending operations, except recovery operations |
| ORACLE role | Configures price sources and sanity bands |
| Oracle operator | Supplies or configures external price observations |
| Keeper | Calls permissionless maintenance |
| Token contract | May have unusual transfer behavior |
| Router and venues | May be malicious, faulty, or economically poor |
| Flash receiver | Executes arbitrary callback logic |
| External vault | May present arbitrary caller and NAV behavior |

## Trust boundaries

### User to controller

User-controlled parameters, tokens, and routing intent are untrusted. The
controller authenticates the caller and applies account, listing, and price
rules before it touches lending state. Solvency and risk-limit gates run
**after** the mutation, inside the same transaction
(`contracts/controller/src/risk/validation.rs`, `require_post_pool_risk_gates`,
called from `strategy_finalize` in
`contracts/controller/src/strategies/mod.rs`). Their effect depends on
transaction rollback. A path that mutates state and never reaches its
post-operation gate is a critical defect.

### Controller to pool

The controller is the pool’s sole mutator. This prevents direct state changes
from skipping risk checks, but makes controller correctness and ownership
configuration critical.

### Controller to position NFT

The position NFT is the account-ownership authority (one token per account,
token id == account id). Mint and burn require the controller's
authorization; `owner_of` is a public read the controller consults live on
every account access rather than caching, so ownership changes take effect
immediately. Transfer is a standard, controller-independent NFT operation —
the controller does not gate it and cannot prevent it — and lazily revokes
any delegate grant the prior owner had made.

**`approve`/`approve_for_all` hand over the entire position, not a token.**
Because the NFT is the account's ownership authority, approving an address to
transfer the token grants that address the ability to take over full control
of the underlying lending position — collateral, debt, and withdraw rights —
the moment it calls `transfer_from`. This is a materially larger blast radius
than an ordinary collectible approval: a phishing signature that looks like
"approve this marketplace" is, for this NFT, "hand over my entire loan
account." There is no protocol-level mitigation for this beyond user
education. The code confirms it: `contracts/position-nft/src/contract.rs`
implements `NonFungibleToken` with `token_uri` as its only override, so
`approve`, `approve_for_all`, and `transfer_from` are the stock OpenZeppelin
implementations. There is no approval hook, no allowlist, and no controller
callback on transfer. Wallets and front ends integrating position NFTs must
present approval prompts with this risk stated explicitly, not with generic
NFT marketplace copy.

### Controller to router and tokens

The router is untrusted. Token behavior is not assumed to match a requested
transfer amount. The controller measures balances, scopes pull authority, and
rechecks solvency after external work.

**The controller does not bound slippage.** Its only output test is that some
`token_out` arrived: `verify_router_output` in
`contracts/controller/src/strategies/swap/balances.rs` asserts `received > 0`
and nothing more. The real minimum-output bound, `total_min_out`, is carried in
the route payload and enforced **inside the swap aggregator**
(`contracts/swap-aggregator/src/execute/mod.rs`) — the same component this
boundary declares untrusted. This is an assumption the controller does not
enforce. A malicious or maliciously upgraded router can return one unit of
output and keep the rest. The only remaining protection is the post-operation
solvency gate, which permits any loss that still leaves the account healthy.
Treat router compromise and router upgrade as an unbounded-loss path for
in-flight strategies, not a bounded one.

### Controller to price system

The controller trusts only a validated complete price snapshot. Source failure
or ambiguity halts valuation-dependent activity instead of choosing a fallback.

### Governance to protocol

Governance is trusted to set policy, but its ordinary power is delayed. The
guardian is intentionally more available but less powerful: it can tighten,
not reopen.

## Major attack surfaces

### Account operations

Attempted theft: borrow against or withdraw another account.

Controls: owner-or-delegate authorization (ownership resolved live from the
position NFT's `owner_of`, never cached), double/triple-gated delegation (a
grant also lapses the instant its granting owner no longer holds the NFT),
immutable spoke binding, post-operation risk gates, and limits on
third-party supply.

Review: test every account verb with owner, delegate, former delegate,
unrelated caller, permissionless caller, and an account whose position NFT
was transferred mid-session.

### Accounting and non-standard tokens

Attempted theft: claim more value than a fee-on-transfer or unusual token
delivered.

Controls: measured inbound receipt, internal cash accounting, zero-share
rejection, backing-shortfall gate, and conservative rounding.

Review: model partial delivery, direct donation, unexpected callback behavior,
dust values, and all intermediate strategy transfers.

### Oracle integrity

Attempted theft: manipulate, stale, partially fail, or race a price source.

Controls: source admission policy, freshness and sanity checks, dual-source
agreement, one snapshot per mutation, and fail-closed consumption.

Review: include one good leg, one bad leg, both stale, boundary timestamps,
large deviation, and assets with tiny positions that still require a price.

### Liquidation

Attempted theft: liquidate a healthy account, over-seize collateral, or exploit
under-delivered repayment.

Controls: health gate, close bound, bonus and fee limits, measured repayment,
proportional seizure scaling, and explicit bad-debt gates. Liquidation is
permissionless — owners may liquidate their own account; the one remaining
self guard rejects crediting seized collateral back into the liquidated
account.

Review: test close boundaries, dust, partial token delivery, paused debt,
unpriceable collateral, and residual debt.

### Flash loans and reentrancy

Attempted theft: leave the pool underpaid or call protected paths during a
callback.

Controls: contract receiver requirement, exact balance assertions,
allowance-based repayment, transaction rollback, and shared monetary
reentrancy protection.

Review: use receivers that underpay, push rather than approve, reenter every
public path, revert late, or alter token balances.

### Flash position

Attempted theft: treat zero-fee strategy debt as a cash flash loan (return
the borrowed token and close the account in the same call), credit the wrong
account, or skip solvency after the callback.

Controls: no repay/net-settle in `flash_position`, declared collateral with a
strictly positive minimum, measured controller deltas, Wasm receiver that is
neither the controller nor the pool, shared reentrancy guard, and
`strategy_finalize` (INV-STRAT-04, ADR-0020).

Review: return the debt token after meeting mins and assert debt remains;
empty/all-zero mins; duplicate underlyings; pool/controller as receiver;
dust collateral that fails HF.

### Strategies and route execution

Attempted theft: router overspend, retain input, claim output dishonestly, or
leave the account unhealthy.

Controls: exact pull authorization, balance-delta settlement, residue return,
a strictly-positive output requirement, and the final solvency gate. The
positive-output check is not a slippage bound; see "Controller to router and
tokens" above.

Review: test malicious return data, partial pull, no output, dust output,
callback reentry, and multi-leg routes.

### Governance and emergency response

Attempted theft: use a hot key to reopen risk, bypass delay, replay an
operation, or deadlock cancellation.

Controls: typed scheduling, payload identity, readiness and expiry checks,
role separation, delete-on-execute behavior, recovery handling, and the
guardian ratchet.

Review: examine both the code-level delay floors and the deployment's
configured delay. `TIMELOCK_SENSITIVE_MIN_DELAY_LEDGERS` in
`contracts/governance/src/constants.rs` is currently **12 ledgers, about one
minute**. The comment above it marks the value as temporary: the production
value is 120_960 ledgers (about seven days), and the constant was lowered for
pre-audit iteration. Restoring it requires a governance-executed `UpgradeGov`
operation, because the value is compiled into the shipped artifact. Until that
upgrade lands, every sensitive-tier operation — Wasm upgrade, ownership
transfer, aggregator re-point, role grant, forced bad-debt socialization — is
effectively immediate. Shipping this constant unrestored is a release blocker.
A timelock with a low configured deployment delay is a live operational risk
for the same reason.

### Blend migration

Attempted theft: point a migration at a hostile contract that poses as a Blend
pool, lies about the position it returns, or pulls more repayment than the
caller agreed to.

Controls: only governance-approved pool addresses are accepted
(`is_blend_pool_approved`, checked in
`contracts/controller/src/strategies/migrate_blend.rs`). Each repay pull is
pre-authorized for an exact capped amount rather than an open allowance
(`authorize_repay_pulls` in `contracts/controller/src/external/blend.rs`).
Every external Blend call runs inside the flash-loan guard
(`guarded_submit`), so the pool cannot reenter the controller. Withdrawn
assets must be suppliable in the target spoke, and the flow ends in the
standard post-operation solvency gate.

Review: test an approved-then-compromised pool, a pool that returns no assets,
a pool that repays less than the cap, duplicate debt assets in one request,
and reentry from the pool back into the controller.

Residual risk: approval is a governance decision. An approved pool that is
later upgraded by its own owner is trusted until governance removes it.

### Share-credit liquidation

Attempted theft: use credit-mode delivery to move seized collateral into an
account that should not receive it, to sidestep risk configuration, or to hand
the collateral straight back to the liquidated account.

Controls: `resolve_seize_receiver` in
`contracts/controller/src/positions/liquidation/mod.rs` rejects the liquidated
account itself, requires the liquidator to own or be an active delegate on the
receiving account, requires the receiver to sit in the same spoke, and requires
`PositionMode::Normal`. `Credit(0)` mints a fresh account owned by the
liquidator instead of reusing one.

Review: credit into a full account, into another spoke, into a strategy-mode
account, into the liquidated account, and into an account whose position NFT
transfers mid-transaction.

### Upgrades and code replacement

Attempted theft: replace protocol code with a build that removes a check, or
repoint the price aggregator at a controlled contract.

Controls: controller, pool, position-NFT, and governance Wasm upgrades are
typed timelocked operations (`AdminOperation::UpgradeController`,
`UpgradePool`, `UpgradePositionNft`, `UpgradeGov` in
`contracts/governance/src/op.rs`). A controller upgrade forces the contract
into the paused state before the new code takes effect (`upgrade` in
`contracts/controller/src/governance.rs`). The position NFT's `upgrade`
requires the controller's authorization
(`contracts/position-nft/src/contract.rs`) and is reachable only through the
controller's owner-gated `upgrade_position_nft`. The price aggregator has no
upgrade entrypoint at all; replacing it means a timelocked repoint.

Review: confirm which delay tier actually applies to each upgrade operation,
given the timelock constant noted under "Governance and emergency response".

### Router ownership

Attempted theft: the swap aggregator's own owner raises fees, edits the fee
whitelist, or replaces the router code.

Controls: the fee setters are owner-gated and bounded by `FEE_CAP`
(`contracts/swap-aggregator/src/lib.rs`, `fees.rs`). Per-strategy loss is
bounded by the controller's balance-delta settlement, which caps what the
router can spend at the stated input.

Residual risk: the router's `upgrade` is `#[only_owner]` and immediate
(`contracts/swap-aggregator/src/lib.rs`). It has no timelock. An upgraded
router is a standing trust downgrade for every route executed afterwards, and
minimum-output enforcement lives inside the router itself. Treat the router
owner as a trust root separate from protocol governance, and check whether the
deployed router owner is the same key set as protocol governance.

### Oracle source authority

Attempted theft: a subset of oracle signers, or the oracle contract's owner,
fabricates a feed value.

Controls: submissions are signer-authenticated, price-bounded, non-future,
fresh, and per-signer monotonic (`submit_price` in
`contracts/xoxno-oracle/src/submit.rs`). The published value is the median of
fresh, skew-clustered submissions, and falls away entirely when fewer than
`threshold` survive (`contracts/xoxno-oracle/src/aggregation.rs`). Above that,
the price aggregator applies staleness, sanity-band, and dual-source tolerance
checks, plus a source-independence policy
(`independence` in `contracts/price-aggregator/src/validation.rs`).

Review: enumerate every price key configured with a single source. Each one
trusts that source's owner completely; the independence and dual-source checks
only bite when a second source exists.

Residual risk: the XOXNO oracle owner can add or remove signers and change the
threshold immediately (`contracts/xoxno-oracle/src/admin.rs`, all
`#[only_owner]`, no timelock).

### Build-surface leakage

Attempted theft: ship a binary that still exports test-only entrypoints.

Controls: `seed_oracle` and `remove_oracle` write oracle configuration with no
owner check and no validation, and are compiled only under
`#[cfg(any(test, feature = "testing"))]`
(`contracts/price-aggregator/src/lib.rs`).

Review: diff each deployed contract's exported function list against the
expected ABI. Do not rely on feature selection alone — verify the artifact.

### Vault adapter

Attempted theft: a caller poses as a DeFindex vault to reach another vault's
controller account.

Controls: the adapter authenticates the calling vault (`from.require_auth()`
in `deposit` and `withdraw`, `contracts/defindex-strategy/src/lib.rs`) and
keys one controller account per vault address
(`DataKey::VaultAccount`, resolved by `resolve_vault_account`). Positions are
supply-only; the adapter exposes no borrow path. A failed account lookup
panics rather than clearing the mapping, because the mapping is the only route
back to the collateral it points at.

Review: attempt cross-vault access, re-binding an existing account to a new
vault address, and a vault that reports adversarial NAV.

### Permissionless maintenance

Attempted theft: use a permissionless path — index accrual, revenue claim,
threshold refresh, recapitalization, third-party supply — to create foreign
risk or move value to an attacker-chosen destination.

Controls: every keeper verb requires only the caller's own authorization and
rejects calls made while a flash loan is in progress
(`require_not_flash_loaning` at each entry in
`contracts/controller/src/keepers.rs`). Revenue is forwarded only to the
configured accumulator, and the claim panics when none is set.
Recapitalization credits measured receipt only. A third party may top up only
hub assets the account already holds
(`require_third_party_existing_supply` in
`contracts/controller/src/positions/supply.rs`).

Review: call each keeper verb during a flash-loan callback, with an
unconfigured accumulator, and with an account whose NFT was transferred
mid-call.

### Storage lifetime

Attempted theft: none directly. The risk is that archival of an account entry
or of the position NFT's `Owner` entry strands a position, because the
controller resolves ownership live from the NFT and the two legs must stay
alive together.

Controls: `renew_account` (`contracts/controller/src/account.rs`) renews the
controller's account entries and calls the NFT's `renew` in the same
transaction, putting both on the same user window. The NFT's `renew`
(`contracts/position-nft/src/contract.rs`) checks existence first and extends
the `Owner` key; the instance TTL is renewed on mint, burn, and upgrade.

Review: confirm no account can reach a state where one leg is archived and the
other is live. Note that `renew_account` requires the owner's authorization, so
a third party cannot pay rent through the controller; only the NFT's own
`renew` is permissionless.

### Governance recovery

Attempted theft: none. The risk is permanent loss of administrative
capability.

Controls: the owner can propose a canceller-set reset
(`propose_canceller_reset`, `#[only_owner]`), which runs on the `Recovery`
delay tier and is marked so the ordinary cancellation path cannot stop it
(`contracts/governance/src/timelock/recovery.rs`). The recovery floor is
518_400 ledgers (`TIMELOCK_RECOVERY_MIN_DELAY_LEDGERS`). The owner can revoke
roles immediately (`revoke_role_immediate`). Execution is permissionless when
no executor is named.

Review: verify at least one live key exists per role, and that the recovery
delay is acceptable as a worst-case outage.

Residual risk: loss of the owner key is unrecoverable. No path resets the
owner.

### Public state and MEV

Attempted theft: front-run a liquidation, or act ahead of a scheduled
governance operation.

Controls: none, by design. All positions, health factors, prices, and pending
timelock operations are public. Liquidation is an open race. The liquidation
curve bounds the bonus and ties repayment size to seizure size
(`contracts/controller/src/positions/liquidation/curve.rs`,
`max_hf_preserving_bonus_bps`), so competing liquidators compete on speed
rather than on extractable excess.

Residual risk: accepted. The protocol makes no confidentiality assumption
anywhere.

### Dust griefing

Attempted theft: open positions too small to be worth liquidating, or find a
rounding path that mints value.

Controls: share conversions that round a positive amount to zero shares revert
(`SupplyRoundsToZeroShares` and siblings in `contracts/pool/src/ops/`). A
configurable minimum borrow-collateral floor rejects accounts that would be
unprofitable to liquidate (`require_post_pool_risk_gates` in
`contracts/controller/src/risk/validation.rs`, `MinBorrowCollateralNotMet`).

Review: sweep values around each rounding boundary on supply, withdraw,
borrow, repay, and net settle.

Residual risk: the floor is a configured value. It is inactive when set to
zero.

## Availability risks

The protocol chooses safety over availability in several cases:

- A price outage or one unusable dual-source leg blocks valuation-dependent
  actions, including liquidation.
- A paused debt listing can block its liquidation leg.
- Bounded position counts and route payload limits constrain resource use but
  may reject otherwise valid large actions.
- External token, router, oracle, and vault failures can revert a transaction.
- A market's utilization ceiling and liquidation buffer can block a borrow or
  a withdrawal even when the account itself is healthy
  (`require_utilization_below_max` and `require_liquidation_buffer` in
  `contracts/pool/src/guards.rs`).
- The XOXNO oracle publishes no aggregate at all when fewer than `threshold`
  signers submit fresh prices, so signer downtime reads as a price outage.
- Storage archival of an account or of its position NFT `Owner` entry blocks
  access until the entry is restored.

These are intentional trade-offs that operators must monitor and govern.

## Accepted residual risks

- Governance and configured role keys remain high-value operational trust.
- Route quality is not fully optimized on-chain; a valid but poor route can
  lose value within the account’s permitted risk envelope.
- External oracle sources and token implementations are outside protocol
  control.
- Formal models and tests have explicit assumptions; they do not prove
  arbitrary cross-contract behavior or deployed configuration.

## Audit priorities

1. Verify deployment ownership, roles, delay, and active configuration.
2. Trace value through every external call to its final accounting entry.
3. Fuzz arithmetic boundaries, share rounding, and liquidation composition.
4. Treat liveness failures as security findings when they can trap collateral
   or prevent liquidation under plausible conditions.
