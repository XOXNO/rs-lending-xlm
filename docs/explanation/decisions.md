# Design decisions

Each decision record states a protocol rule and its trade-off. Read
[Architecture](../reference/architecture.md) first for the component model.
[Invariants](../reference/invariants.md), [Formulas](../reference/formulas.md)
and the [Threat model](threat-model.md) define the exact properties, arithmetic
and risks.

ADR numbers are stable anchors. The records are grouped by topic, so the
numbers are not in sequence.

| ADR | Decision | Topic |
|---|---|---|
| [0001](#adr-0001) | Governance, controller, and pool authority | Authority |
| [0002](#adr-0002) | Central custody, separate market books | Accounting |
| [0003](#adr-0003) | Indexed shares and directed rounding | Accounting |
| [0004](#adr-0004) | Dual-source agreement | Prices |
| [0005](#adr-0005) | Fail-closed valuation | Prices |
| [0006](#adr-0006) | Typed proposals and bound execution | Authority |
| [0007](#adr-0007) | Emergency ratchet | Authority |
| [0008](#adr-0008) | Independent halt flags | Authority |
| [0009](#adr-0009) | Immutable spoke binding | Authority |
| [0010](#adr-0010) | Cash flash-loan repayment | Execution |
| [0011](#adr-0011) | Measure router settlement | Execution |
| [0012](#adr-0012) | Supplier-index loss allocation | Accounting |
| [0013](#adr-0013) | Credit measured receipts | Accounting |
| [0014](#adr-0014) | Governed source admission | Prices |
| [0015](#adr-0015) | Literal asset-unit caps | Accounting |
| [0016](#adr-0016) | Millisecond rates and chunked accrual | Accounting |
| [0017](#adr-0017) | Verification and deployment artifacts | Verification |
| [0018](#adr-0018) | Compact route instructions | Execution |
| [0019](#adr-0019) | Liquidation share credit | Execution |
| [0020](#adr-0020) | Zero-fee flash-position callback | Execution |
| [0021](#adr-0021) | Gross-debt cleanup accounting | Accounting |
| [0022](#adr-0022) | Liquidator payoff at collateral equal to debt | Accounting |
| [0023](#adr-0023) | Delegated listing authority through the curator | Authority |

## Authority and emergency control

<a id="adr-0001"></a>

### ADR-0001: Governance, controller, and pool authority

Governance owns the controller under the deployment helpers, and the controller
owns the pool. The controller applies account and risk policy while the pool
handles custody and accounting. This creates a shared trust boundary:
controller bugs, upgrades, or authorized ownership changes can affect every market.

<a id="adr-0006"></a>

### ADR-0006: Typed proposals and bound execution

Governance validates administrative intent at proposal time and executes the
bound payload only after its delay and within its grace window. Target
contracts also validate at execution. Completed operations require a fresh
proposal and delay before reuse.

Execution is permissionless: it takes no executor identity and checks no role. Cancellation
and owner-dependent recovery have distinct rules. Their security depends on
the effective review window and key custody; see
[governance powers](threat-model.md#governance-windows-and-emergency-powers).

<a id="adr-0007"></a>

### ADR-0007: Emergency ratchet

Immediate guardian actions can pause the controller and tighten listing flags.
Reopening uses delayed administration. A listing edit can keep or tighten flags
but never clears one. Clearing is a separate timelocked operation,
`relax_spoke_asset_flags`, bound to the listing's flags epoch. Every flag
change advances the epoch, so a relaxation proposed before a later guardian
action reverts when executed. A guardian call that names the flags the listing
already holds is rejected (`SpokeAssetFlagsUnchanged`) and leaves the epoch
alone: the epoch moves only on a genuine tightening, so a guardian can
invalidate a pending relaxation at most once per flag and cannot hold a listing
closed against governance by re-asserting the same flags. The ORACLE role can
narrow sanity bands; widening requires timelocked oracle reconfiguration.
The [listing-freeze runbook](../reference/runbooks/freeze-a-listing.md) gives the
operator steps.

<a id="adr-0008"></a>

### ADR-0008: Independent halt flags

The `frozen`, `paused`, and `no_seize` flags control different actions. Entry
checks paused/frozen, ordinary exits check paused, and collateral seizure
checks no_seize. A paused debt leg does not block repayment of another selected
leg; pausing collateral alone does not block its seizure.

A seizure restriction does not stop new supply. Non-dust no_seize collateral
can therefore block an account's pro-rata liquidation. Operators should inspect
existing usage before setting no_seize and use frozen to stop new entry.
Interest continues during listing pauses; a prolonged pause may also require
a timelocked rate reduction. Insolvent accounts have the governed
[force-socialize path](../reference/runbooks/force-socialize-bad-debt.md).

A listing pause applies to one spoke, while the market's pool serves its whole
hub. Borrowers in the paused listing cannot repay, but another spoke that lists
the same asset can still move its utilization and so its rate. A pause has no
liquidation grace period: whoever executes the reopening can liquidate in the
next transaction. Operators should pair a long pause with a rate reduction or a
borrow freeze of that asset in every spoke, and should reopen only when
borrowers have had notice.

<a id="adr-0009"></a>

### ADR-0009: Immutable spoke binding

An account retains its spoke throughout its lifetime. A caller cannot move
existing debt into another risk regime by changing an argument. Governance can
change listings, and any caller can refresh an account's stored risk values
with `update_account_threshold`. No path rebinds the account.

<a id="adr-0023"></a>

### ADR-0023: Delegated listing authority through the curator

The curator contract lets independent registrars run their own hubs and
spokes. Governance stays the controller owner. The curator holds the
governance `CURATOR`, `PROPOSER` and `CANCELLER` roles.

Each registrar has an admin and members with role bits:

| Role | Allows |
|---|---|
| `ROLE_MARKET` | Register markets, set cap grants, queue listings, listing edits and rate changes |
| `ROLE_SPOKE` | Create hubs and spokes |
| `ROLE_FLAGS` | Tighten listing flags, queue a flag relaxation |

Changes that expose no funds are immediate: `create_hub`, `add_spoke`,
`create_market` and `tighten_flags`. A new market takes no supply or debt until
a spoke lists it. Listings, listing edits, rate changes and flag relaxations go
through `propose`, so they wait for the governance timelock. A registrar can cancel a queued operation only for a hub or spoke
that it owns. A hub owner can thus stop a listing queued under a grant that
it later revoked.

The trade-off: registrars add listings without a governance proposal of their
own, but they cannot skip the timelock. The
[curator README](../../contracts/curator/README.md) lists each entrypoint and
its boundary.

## Accounting and loss allocation

<a id="adr-0002"></a>

### ADR-0002: Central custody, separate market books

Markets are keyed by hub and asset, each with its own cash, shares, indexes,
and revenue. One pool provides custody for all markets. Markets for one token
in different hubs keep distinct books but share its pool balance and token
risks. Spokes that list the same hub asset share its single market book.

<a id="adr-0003"></a>

### ADR-0003: Indexed shares and directed rounding

Supply and debt use shares, with interest applied through indexes. This avoids
updating every account on accrual. Supply mints floor shares, withdrawals burn
ceiling shares, borrowing mints ceiling debt, and partial repayment burns floor
debt. The [formula reference](../reference/formulas.md) defines close paths and dust handling,
which do not all follow the partial-operation rules.

<a id="adr-0012"></a>

### ADR-0012: Supplier-index loss allocation

Eligible bad debt is removed by reducing the affected market's supply index,
subject to a nonzero floor. Supplier claims in that market bear the write-down.
The floor protects share conversions but can leave unpaid backing; a displayed
claim does not guarantee that the supplier can withdraw that amount.

<a id="adr-0013"></a>

### ADR-0013: Credit measured receipts

Supply, repayment, recapitalization, and supported strategy legs credit the
amount received. Under-delivered liquidation repayment reduces the associated
seizure, except that a liquidation seizing collateral below 3 decimals reverts
on a short receipt. This supports specific receipt-tax behavior; it does not make rebases,
sender surcharges, or false balances safe. See [token assumptions](threat-model.md#token-assumptions).

<a id="adr-0015"></a>

### ADR-0015: Literal asset-unit caps

Exposure growth converts asset-unit caps to shares at the applicable index.
A zero cap permits no new exposure; exits do not consume headroom. Same-spoke
liquidation credit moves existing supply and its protocol fee reduces usage.
It therefore does not require new supply-cap headroom.

<a id="adr-0016"></a>

### ADR-0016: Millisecond rates and chunked accrual

Accrual uses a per-millisecond RAY rate derived from the annual rate. It splits
elapsed time into chunks of at most one year (`MAX_COMPOUND_DELTA_MS`). Each
chunk uses the preceding chunk's market state. This bounds individual time
steps without eliminating value overflow, cumulative work, cadence dependence,
or rounding error. The [formula reference](../reference/formulas.md) defines these limits.

<a id="adr-0021"></a>

### ADR-0021: Gross-debt cleanup accounting

Cleanup converts remaining account supply to protocol revenue, then socializes
its gross debt. This applies even when the supply and debt belong to the same
market; cleanup does not net them first. Treasury gains, supplier losses, and
recapitalization must be interpreted on that basis.

The [bad-debt section](threat-model.md#bad-debt-and-liquidation) explains the financial
consequences.

<a id="adr-0022"></a>

### ADR-0022: Liquidator payoff at collateral equal to debt

Audit finding R-23 noted that the liquidator profit jumps where collateral `C`
falls below debt `D`. The protocol keeps this payoff shape.

Let `x = C / D` and let `b` be the base bonus:

| Region | Liquidator profit | Lender loss |
|---|---|---|
| `1 <= x < 1 + b` (band) | `C - D` | 0 |
| `x < 1` (insolvent) | `C * b / (1 + b)` | `D - C / (1 + b)` |

At `b = 5 %` the profit goes from 0.50 % of `D` at `x = 1.005` to 4.74 % of `D`
at `x = 0.995`. The jump is the same for 7-decimal and 0-decimal collateral.

Each alternative is worse:

- A payoff that never rises as `x` falls must pay 0 below `x = 1`. Then no
  liquidator closes an insolvent account, and more bad debt stays open.
- A continuous insolvent bonus lets a liquidator earn more by splitting one
  liquidation into many parts. The Certora rule
  `split_liq_chain_bound_holds_when_health_never_recovers` depends on a
  constant insolvent bonus.
- A band quote of `floor(C / (1 + b))` leaves debt with no collateral. At
  `x = 1.005` that is 4.29 % of `D`.

A liquidator cannot push an account across `x = 1`. The HF-preserving cap keeps
`1 + bonus <= C / D` on a solvent account, so only a price move or interest
moves `x`. A liquidator that waits for `x < 1` gives up the larger payoff at the
band top (`b * D`). The gain from waiting is at most `D * b / (1 + b)` per
account. To reduce that bound on a listing, governance lowers its base bonus.

Two tests pin the payoff shape:
`the_liquidator_payoff_jumps_where_collateral_falls_below_debt` and
`zdc_liquidator_payoff_jumps_where_collateral_falls_below_debt`. A change to the
payoff must update both tests and
[formulas](../reference/formulas.md#bonus-and-target-repayment).

## Price validation

<a id="adr-0004"></a>

### ADR-0004: Dual-source agreement

A dual-source key serves the integer midpoint, rounded down. Both legs must
pass validity, freshness, and tolerance checks, and the result must pass its
sanity band. One healthy leg is not a fallback. Runtime tolerance uses the half-up
rounded larger/smaller ratio; the reciprocal pair is validated at admission.
Single-source keys have separate admission constraints. This makes source
availability a condition of price availability; see [price risks](threat-model.md#price-integrity-and-availability).

<a id="adr-0005"></a>

### ADR-0005: Fail-closed valuation

A valuation-dependent operation must obtain every required valid price in its
context. Price failure aborts the operation. This protects risk decisions at
the cost of withdrawal and liquidation availability during outages.

A borrower can hold a dust leg of a failing asset to block its own
liquidation ([DoS.1](threat-model.md#stride-register)). Repayment reads no
price, so anyone can repay a dust debt leg during the outage and then
liquidate, unless that listing is paused.

<a id="adr-0014"></a>

### ADR-0014: Governed source admission

Admission validates source structure, dependencies, provider-address overlap,
and smoothing policy. Smoothing governs source selection rather than
transforming the midpoint. Reflector `Twap` mode takes an
equal-weight mean of its accepted observations. LP configurations are
sole-source, waive smoothing and tolerance checks, and use a separate sanity
cap alongside pool and underlying-price validation.

Changed keys receive provider-specific attestation and a probe. Non-LP probes
permit market-condition failures; LP admission requires a usable price. Curve
attestation is a no-op; that probe enforces the pool binding.
Transitive dependents receive structural revalidation without new live probes.
This limits repeated provider calls and their memory cost; an LP receives live
suitability checks on its own admission, not on each upstream-key edit.
Provider-address checks do not establish independent operators or upstream
data, and feed-nature labels remain configuration assertions.

## External execution and settlement

<a id="adr-0010"></a>

### ADR-0010: Cash flash-loan repayment

Cash flash loans require a contract receiver and allowance of at least
principal plus fee. The pool pulls exactly that amount and checks its expected balance after
payout, after callback and after collection. A transfer to the pool during the
callback fails the post-callback check, so only the pull repays. Excess
allowance is permitted. Cash flash loans create no account debt and are
distinct from account strategy settlement.

<a id="adr-0011"></a>

### ADR-0011: Measure router settlement

The controller authorizes one exact input-token transfer invocation to the
configured router, with no token allowance. Settlement ignores returned amounts,
rejects excess measured spend, and requires positive measured output. Router minimum
output is checked after fees but before payout, not independently by the
controller. Controller-held unused input returns to the caller; router-internal
residuals follow a capped admin-revenue policy. Final account risk checks
do not guarantee route quality.

<a id="adr-0018"></a>

### ADR-0018: Compact route instructions

Routes use bounded indexed address and amount registries.
Decoding checks format, index bounds, Prev links, and individual split weights;
runtime vault accounting checks available balances and venue receipts.
Split weights apply to the shrinking remainder and need not sum to one.
Registries need not be unique; LP (Aquarius or Curve) constituents come from the pool.
A valid instruction stream does not establish good economic execution.

<a id="adr-0019"></a>

### ADR-0019: Liquidation share credit

Liquidation can credit a `Normal`-mode account in the same spoke that the
liquidator owns or is a delegate for, avoiding a collateral cash payout.
Seized shares split into receiver credit and protocol revenue by
reclassification; the fee does not mint unbacked shares.

Position limits still apply to an existing receiver (a `Credit(0)` receiver
is exempt), and a newly credited asset needs a
listing. Existing positions retain their risk tuple; new positions use the
listing's values. The receiver's supply increase is bounded by the target's
seizure and its debt does not move. The [event reference](../reference/events.md) distinguishes
gross LiqSeize from net LiqCredit.

Controller account-isolation proofs exempt the declared receiver and frame a
third account. The receiver-credit bound does not prove pool fee allocation
because the model summarizes that call; see
[model boundaries](../../certora/README.md#model-boundaries).

<a id="adr-0020"></a>

### ADR-0020: Zero-fee flash-position callback

Flash position creates debt without an origination fee and forwards it to a
caller-chosen contract callback. It requires nonnegative collateral minima
with at least one positive, an eligible flash-loan debt market, and an open
solvent final position. Minima bind measured controller receipts; pool supply
remeasures delivery. Returned debt is never auto-repaid: it can become
declared collateral, be refunded when refund-listed, or remain uncredited.

This is an authorized borrowing strategy, not a free cash-flash round trip.
An already healthy account may support the new debt with little added
collateral. Its origination-fee treatment differs from multiply.

## Verification boundaries

<a id="adr-0017"></a>

### ADR-0017: Verification and deployment artifacts

Feature gates separate testing and verification helpers from deployment code.
Release-WASM checks inspect forbidden exports for the artifacts and build inputs
they cover. One gate changes behavior instead of adding an entrypoint: the
governance `short-timelock` feature lowers the sensitive delay floor from 120960
to 12 ledgers for the end-to-end build. Live runs deploy that build from the
fixtures directory, so the released governance Wasm is the one candidate the
network run does not execute; native tests cover it. The build stamps a
`timelock` contract-meta entry, and the candidate check fails on a governance
candidate that carries it. Formal results also depend on their harness and external-call
assumptions. [Reading a Sunbeam verdict](../../certora/README.md#reading-a-sunbeam-verdict)
explains how to interpret that evidence.
