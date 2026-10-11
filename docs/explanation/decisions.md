# Protocol design rationale

These explanations connect protocol rules to their trade-offs for maintainers
and auditors. Read [Architecture](../reference/architecture.md) first for the
component model. [Invariants](../reference/invariants.md),
[Formulas](../reference/formulas.md), and the [Threat model](threat-model.md)
define exact properties, arithmetic, and risks.

The numbered anchors provide stable references across documentation. Sections
describe implemented behavior.

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

Every proposal also records a guard: the governance state the operation is
bound to ([INV-AUTH-07](../reference/invariants.md#inv-auth-07)). Each
counter is as narrow as the action that must void the operation. Every
proposal records its proposer's epoch, which advances when that address
loses `PROPOSER`. The owner always holds the role and loses it only at a
handover to another address, so the handover voids the former owner's
queue, including the proposer-open operations it held PROPOSER for, while
independent proposers' work survives; a revoked proposer's queue dies with
its key ([ADR-0023](#adr-0023)).
`Unpause` records the emergency epoch, which a guardian pause and an executed
controller upgrade advance. A role grant records the revocation epoch of its
account and role, so a later revocation of that pair, immediate or
timelocked, voids it without voiding unrelated grants or reopenings. A
listing records its token's flag epoch, an oracle reconfiguration its
key's band epoch and an aggregator switch the band epoch of every key, so the
guardian's and oracle role's immediate writes void what was queued before
them. Counters only increase, so a voided operation
never revives when a value is later restored. The guard stores values only:
the key of each counter is read back from the scheduled function and
arguments at execution, as it was at proposal. An operation scheduled before
guards existed has no record and is held to the owner epoch and the counter
its kind binds at 0, the value they all had at that upgrade; every new
proposal therefore writes a record, even one that binds nothing. Operations
carry no predecessor; [ADR-0022](#adr-0022) orders the two pairs that need
it.

Execution is permissionless when the executor identity is omitted. Cancellation
and owner-dependent recovery have distinct rules. Their security depends on
the effective review window and key custody; see
[governance powers](threat-model.md#governance-windows-and-emergency-powers).

<a id="adr-0007"></a>

### ADR-0007: Emergency ratchet

Immediate guardian actions can pause the controller and tighten listing flags.
Reopening uses delayed administration; every guardian pause advances the
emergency epoch, so an `Unpause` proposed before a later pause reverts when
executed. A listing edit can keep or tighten flags but never clears one. An
empty flagged listing can be removed directly, because an absent listing is
at least as restrictive; the re-listing that could clear the flags is voided
in governance, where every guardian flag write advances the token's flag
epoch and an `AddAssetToSpoke` of that token proposed before it, in any spoke
or hub, reverts. Clearing is a separate timelocked operation,
`relax_spoke_asset_flags`, bound to the listing's flags epoch. Every guardian
flag call and every flag change advances the epoch, so a relaxation proposed
before a later guardian action reverts when executed. The ORACLE role can
narrow sanity bands; widening requires timelocked oracle reconfiguration
proposed after the last narrowing, since a `ConfigureAssetOracle` reverts once
the key's band was written or changed after its proposal.
The [listing-freeze runbook](../reference/runbooks/freeze-a-listing.md) gives the
operator steps.

<a id="adr-0022"></a>

### ADR-0022: Ordering without predecessors

Ready operations run in any order, and anyone may execute them. Two orders
matter. A reopening must not run ahead of the fix it waits for, and an older
build must not land after a newer one of the same contract. Governance
enforces both without an interface change and without keeping executed
operations' timelock markers
([INV-AUTH-08](../reference/invariants.md#inv-auth-08)). It counts pending
`UpgradeController` operations: a proposal adds one, and execution,
cancellation and the clearing of an expired operation each remove one, which a
per-operation marker makes exact. An `Unpause` reverts while the count is
above zero. Each upgrade also records the upgrade epoch of the contract it
replaces, keyed by the target and upgrade function, and every executed upgrade
of that contract advances it.

The alternative was an optional `predecessor` on `propose` and
`execute_self`, with OpenZeppelin's Done markers kept after execution. It
binds any pair the operator chooses but is opt-in, changes the ABI and the
operator tooling, and leaves every executed tuple's salt spent. The pending
count is not opt-in, but it is coarse. While any controller upgrade is
pending, a voided or expired one included, no reopening can run until the
upgrade executes or a canceller cancels it. An emergency reopening that does
not want the queued upgrade therefore needs a cancel first. Two upgrades of
one contract cannot be queued to land in sequence either: the first to
execute voids the other, which is proposed again. An `UpgradeController`
pending at the governance upgrade that installs this rule is not counted, so
the upgrade sweep cancels and re-proposes it.

<a id="adr-0023"></a>

### ADR-0023: Owner-held liquidation terms and proposer containment

A non-owner `PROPOSER` keeps routine listing power: listings, caps, limits,
the interest-rate model, edits that keep or improve a listing's terms,
manager deactivation and spoke removal. Liquidation terms decide when an
account becomes liquidatable and what it loses, so they belong to the owner:
the spoke liquidation curve, forced socialization, position-manager
activation, and a listing edit that lowers the threshold, raises the bonus or
lowers the fees of the live listing, read from the controller at proposal
([INV-AUTH-09](../reference/invariants.md#inv-auth-09)). A proposal-time
check alone lets a proposer queue a kept edit and a raise, execute the raise
and then the kept edit, and walk the live terms back. A non-owner edit is
therefore a compare-and-swap: its guard records the live threshold, bonus and
fees, and execution reverts unless the controller still holds them. The
owner's edits are not bound. A per-operation cap on each cut was rejected: it
slows the attack rather than removing it and binds the owner's emergency
cuts too.

The owner can revoke `PROPOSER` and `EXECUTOR` immediately, as it can
`GUARDIAN` and `ORACLE`. A stolen proposer key is worth its queue, so every
proposal records its proposer's epoch, which every loss of the role
advances, a handover away from the owner included: one call removes the key
and voids everything it queued, and any canceller clears the dead
operations at leisure. Routine key rotation therefore voids the old key's
pending work, which the new key proposes again. `CANCELLER` keeps its
timelocked removal, which is what lets cancellers veto a compromised owner
key.

The canceller council holds at most 24 members, the owner included. A reset
revokes every non-owner member and seats the new list in one transaction:
replacing N members writes 3N + 6 ledger entries, and each classic-account
member adds about 470 bytes of role events. Against soroban-sdk 28's mainnet
profile (`soroban-sdk-28.0.0/src/testutils/cost_estimate.rs:146-151`: 200
write entries, 400 ledger entries, 16,384 event bytes, 400M instructions per
transaction), 24 seats write 75 entries and keep more than 5 KB of event
headroom; 32 seats kept less than the 2 KB margin the earlier cap claimed.

<a id="adr-0008"></a>

### ADR-0008: Independent halt flags

The `frozen`, `paused`, and `no_seize` flags control different actions. Entry
checks paused/frozen, ordinary exits check paused, and collateral seizure
checks no_seize. A paused debt leg does not block repayment of another selected
leg; pausing collateral alone does not block its seizure. One limitation is
accepted: a collateral leg below 3 decimals seizes whole units, so when the
unpaused debt legs together back less than one unit at `1 + bonus`, every
offer reverts until the pause lifts, the price moves, or accrual ends that
state. Such an account is solvent, so bad-debt cleanup does not apply either,
and no liquidation path exists while the state lasts. Before pausing a debt
listing in such a spoke, enumerate the accounts whose only collateral is a leg
below 3 decimals and whose unpaused debt is below the value of one unit
divided by `1 + bonus`. A unit here is one base unit of the collateral token
([unit definition](../reference/formulas.md#bonus-and-target-repayment)). No
mainnet listing is below 3 decimals; prefer `frozen` over `paused` for debt
legs in a spoke that lists such collateral.

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

<a id="adr-0027"></a>

### ADR-0027: Per-account valuation exclusion for a feed outage

A price outage halts every valuation of an account that holds the affected leg
(INV-ORACLE-01). Supply needs no price, so an indebted borrower can add a dust
leg of a collateral whose feed it controls, such as an Aquarius LP it can take
below `min_pool_value_wad`, and delay its own liquidation (threat model
DoS.1). The configuration answers come first. Keep LP collateral out of spokes
that share accounts with borrowable majors, and size `min_pool_value_wad` far
below plausible pool values. When an outage happens anyway, the guardian
can exclude the shielding leg of a named account from that account's
valuation.

The exclusion is per account, not per listing. A listing-wide exclusion would
let a decoy force the guardian's hand: to unblock one dust leg, every holder
of the listing in the spoke would lose its collateral value and become
liquidatable on its other legs at the bonus, during an outage they did not
cause, and would stay so after the feed recovered until a delayed restore. A
per-account exclusion unblocks the shielding account alone. Honest holders
stay frozen during the outage, as without the exclusion, and are valued in
full once the price is back. The guardian picks the accounts off-chain: those
that are underwater on their other legs, or at the last good price.

The exclusion is a side map keyed by account, not a field of the stored
position or listing: no stored type changes shape. An excluded supply leg
counts as zero collateral in the health factor, the post-pool gates, the views
and the liquidation plan of that account, and is never priced or seized there.
Dropping a non-negative term can only lower collateral, so the exclusion is
conservative for lenders. It makes the account liquidatable on its other legs,
never less liquidatable. The per-leg 1.05 check that decides whether a
liquidator-favoring tuple applies values the same book: the flow names the
account in its context when it loads it, so the merge paths keep their
signatures. Valuing the full book there would let a third party's top-up
stamp a tuple only the hidden leg carries once its feed recovers, and would
revert the owner's flows on the dead feed while it is out.

Four rules keep it from hiding collateral that a liquidation should seize or
from serving another role:

- The controller admits the exclusion only while the asset's strict price
  read fails, and only for a feed outage: stale, no last price, empty or short
  TWAP history, invalid price, source disagreement, or an Aquarius pool below
  its floor. A sanity-band violation is refused, because the oracle role can
  narrow a band immediately and make an honest price fail. A guardian cannot
  zero a priced leg, alone or with the oracle role.
- Only the timelocked `restore_account_asset_valuation` clears it. It names
  the exclusion's ledger stamp, so a restore prepared for an earlier exclusion
  cannot clear a later one, and it executes only once the price is usable, so
  a restore queued during the outage cannot re-arm the shield. Like every
  proposal it carries the proposer's guard, so revoking that proposer voids
  it ([INV-AUTH-07](../reference/invariants.md#inv-auth-07)).
- Debt is never written off against an excluded leg. Standalone and forced
  cleanup price every leg strictly, and the residual cleanup after a
  liquidation skips an account that still holds one.
- The exclusion never touches the listing's flags epoch. A restore and a
  `relax_spoke_asset_flags` of the same listing are independent and can run
  in the same timelock round.

The owner may withdraw an excluded leg whenever the remaining legs keep the
account solvent. The protocol counted it at zero, so lenders lose nothing, and
it is the borrower's way out. From an unhealthy account it cannot leave.

There is no in-contract grace period. The exclusion is admitted only during
an outage that already freezes the account, so a grace period would only
lengthen the delay the attacker wanted. The account's owner gets notice from
`AccountValuationEvent` and from `get_account_valuation_exclusions`. A debt leg
of the outage asset is still priced, so the exclusion does not unblock an
account whose unusable feed is a debt leg.

<a id="adr-0009"></a>

### ADR-0009: Immutable spoke binding

An account retains its spoke throughout its lifetime. A caller cannot move
existing debt into another risk regime by changing an argument. Governance can
change listings, and any caller can refresh an account's stored risk values
with `update_account_threshold`. No path rebinds the account.

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
seizure. This supports specific receipt-tax behavior; it does not make rebases,
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
chunk uses the preceding chunk's market state and caps its interest at the
market value ceiling. This bounds individual time steps without eliminating
cumulative work, cadence dependence, or rounding error. The [formula reference](../reference/formulas.md) defines these limits.

<a id="adr-0021"></a>

### ADR-0021: Gross-debt cleanup accounting

Cleanup converts remaining account supply to protocol revenue, then socializes
its gross debt. This applies even when the supply and debt belong to the same
market; cleanup does not net them first. Treasury gains, supplier losses, and
recapitalization must be interpreted on that basis.

The [bad-debt section](threat-model.md#bad-debt-and-liquidation) explains the financial
consequences.

<a id="adr-0024"></a>

### ADR-0024: Bounded revenue claims

Revenue accrues on every unit of interest, including interest an uncleaned
insolvent account will never pay, and the pool cannot tell the two apart. A
revenue claim is a permissionless draw on cash, so while a market carries debt
it draws only the cash above the 2% liquidation buffer, and pays only if the
whole payout keeps utilization at or below `min(max_utilization, 95%)`
(`MAX_CLAIM_UTILIZATION_RAY`); otherwise it pays nothing. The bound does not
depend on the market's own ceiling, so a market with its ceiling disabled
keeps the cash its liquidations and cleanup need.

The claim is all or nothing rather than a partial payout down to the line:
measured on an uncleaned 94% book over 30 days, a partial payout paid out more
revenue on uncollectible interest than the refusal, while the refusal
reproduces exactly what a 95% market ceiling already did. A claim that cannot
pay returns 0 instead of reverting, and a zero claim skips the utilization and
supply-for-debt guards, so a keeper's multi-market claim batch never fails over
one market. Claims small enough to fit under the bound still pay revenue on
uncollectible interest; prompt cleanup is the remedy. A ledger of interest
actually repaid would separate the two but needs new storage. See the
[payout formula](../reference/formulas.md#revenue-payout) and
[INV-ACCT-06](../reference/invariants.md#inv-acct-06).

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

<a id="adr-0014"></a>

### ADR-0014: Governed source admission

Admission validates source structure, dependencies, provider-address overlap,
and smoothing policy. Smoothing governs source selection rather than
transforming the midpoint. Reflector `Twap` mode takes an
equal-weight mean of its accepted observations. LP configurations are
sole-source, waive smoothing and tolerance checks, and use a separate sanity
cap alongside pool and underlying-price validation.

Changed keys receive provider-specific attestation and a probe. Non-LP probes
permit market-condition failures; LP admission requires a usable price.
Transitive dependents receive structural revalidation without new live probes.
This limits repeated provider calls and their memory cost; an LP receives live
suitability checks on its own admission, not on each upstream-key edit.
Provider-address checks do not establish independent operators or upstream
data, and feed-nature labels remain configuration assertions.

<a id="adr-0026"></a>

### ADR-0026: Admission bounds what a read can reach

Admission holds each configuration to what its reads can reach, so a key
that is admitted can price while its providers are live, and an exemption
holds for every input a shared provider controls. Four rules follow.

- A Reflector budget covers `(records + 2) x resolution`, two rounds for a
  spot read: the history window, the current period, and the round Reflector
  keeps serving while the next one is late. A `(records + 1)` budget goes
  stale for the publication delay of every round.
- A pair keeps its wide band only while every contract its legs share moves
  the blend by at most `11/9`, counting the factor range, the contract's
  reach into the leg's quote key one level down, and the pair's tolerance.
  The quote reach is the quote's own tolerance when the contract serves one
  leg of a two-leg quote, and unbounded when it serves every leg. Deeper
  reach is not walked: it is either inside one leg of the quote, which the
  quote's tolerance already bounds, or it makes the reach unbounded. One
  extra basis point on each tolerance covers the read's half-up comparison.
- A feed reports at least eight decimals, so a TWAP mean that floors to one
  quantum misvalues a token by at most 1e-8 USD.
- The registry admits at most 112 keys. Revalidating dependents scans the
  whole registry, resolving each key's dependency once, and walks each
  dependent's composition; the instruction limit binds before the 400-entry
  footprint. A full registry of dependents with two `Scaled` legs at the
  resolution depth limit costs about 357M of the 400M instructions per
  transaction, shared pairs two quotes deep about 246M. Branching above the
  edited key beyond one pair of legs is outside the measured bound.

The rules only refuse more. Keys admitted before an upgrade are not
re-attested, but every dependent revalidation runs the new structural rules,
so the deploy preflight must enumerate the registry and the pending
`ConfigureAssetOracle` and `EditOracleTolerance` operations. Mainnet passes
all four: every Reflector leg is `Twap(3)` at 3,600 s or 7,200 s, every feed
reports 8 or 14 decimals, the registry holds about 32 keys, and SolvBTC sits
at about 1.213 of the 1.222 cap.

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
do not guarantee route quality. The router's fee base, rounding and residual
allowance are in [Router fees and residuals](../reference/formulas.md#router-fees-and-residuals).

<a id="adr-0018"></a>

### ADR-0018: Compact route instructions

Routes use bounded indexed address and amount registries.
Decoding checks format, index bounds, Prev links, and individual split weights;
runtime vault accounting checks available balances and venue receipts.
Split weights apply to the shrinking remainder and need not sum to one.
Registries need not be unique; Aquarius LP constituents come from the pool.
Token identity is address identity: the same-token checks on the strategy
endpoints and on every swap compare registry addresses, so a duplicate entry
cannot make the input token its own declared output.
A valid instruction stream does not establish good economic execution.

<a id="adr-0019"></a>

### ADR-0019: Liquidation share credit

Liquidation can credit a `Normal`-mode account in the same spoke that the
liquidator owns or is a delegate for, avoiding a collateral cash payout.
Seized shares split into receiver credit and protocol revenue by
reclassification; the fee does not mint unbacked shares.

Receiver position limits still apply to a receiver with positions; an empty
receiver takes every seized leg, so a lowered limit cannot block `Credit(0)`.
No credit opens a supply slot past `POSITION_LIMIT_MAX`, which the
per-account resource budget is sized for. A newly credited asset needs a
listing. Existing positions retain their risk
tuple; new positions use the
listing's values. The receiver's supply increase is bounded by the target's
seizure and its debt does not move. The [event reference](../reference/events.md) distinguishes
gross LiqSeize from net LiqCredit.

Controller account-isolation proofs exempt the declared receiver and frame a
third account. The receiver-credit bound does not prove pool fee allocation
because the model summarizes that call; see
[model boundaries](certora-sunbeam-prover-tuning.md#model-boundaries).

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

<a id="adr-0025"></a>

### ADR-0025: Liquidation records the pool's close and quotes what executes

A partial seizure whose floored token amount reaches the pool's half-up
balance is recorded as the full close the pool performs: Transfer requests that
amount and the pool burns every share, and Credit takes the exact held shares,
so the two modes agree, and the sub-half-unit remainder goes to the Credit
receiver or stays as pool cash in Transfer mode. Whole-unit legs follow the
same rule. The rejected variant lowered a would-close request by one unit: it
broke Credit/Transfer parity by up to a unit, charged the liquidator for a unit
it did not receive (a whole coin on a whole-unit leg), skipped a leg whose
repayment it kept, and left a fractional whole-unit holding that no offer could
liquidate.

The Credit estimate applies the receiver gates execution applies, except the
owner-or-delegate check, which needs a caller; the interface keeps no caller
parameter. Each liquidation leg's feed decimals must equal the pool's, checked
on the pool mutation at execution (on the listing for a Credit seizure and in
the estimate). The check is not in market loading because valuation never
reads decimals and every other flow would pay a pool read per asset. The base
bonus rounds the USD-weighted average once over exact products widened to 256
bits when needed, bounding its gap to the exact average at half a BPS for any
leg count. The [formula reference](../reference/formulas.md#liquidation-sizing-and-fees)
states each bound.

## Verification boundaries

<a id="adr-0017"></a>

### ADR-0017: Verification and deployment artifacts

Feature gates separate testing and verification helpers from deployment code.
Release-WASM checks inspect forbidden exports for the artifacts and build inputs
they cover. Formal results also depend on their harness and external-call
assumptions. The [proof guide](certora-sunbeam-prover-tuning.md) explains how to
interpret that evidence.
