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

Execution is permissionless when the executor identity is omitted. Cancellation
and owner-dependent recovery have distinct rules. Their security depends on
the effective review window and key custody; see
[governance powers](threat-model.md#governance-windows-and-emergency-powers).

<a id="adr-0007"></a>

### ADR-0007: Emergency ratchet

Immediate guardian actions can pause and tighten listing flags. Reopening uses
delayed administration. A full listing rewrite can clear flags, so operators
must explicitly preserve restrictions in those updates. The ORACLE role can
narrow sanity bands; widening requires timelocked oracle reconfiguration.

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

<a id="adr-0009"></a>

### ADR-0009: Immutable spoke binding

An account retains its spoke throughout its lifetime. A caller cannot move
existing debt into another risk regime by changing an argument. Governance can
change listings and refresh applicable stored risk values, but cannot rebind
the account.

## Accounting and loss allocation

<a id="adr-0002"></a>

### ADR-0002: Central custody, separate market books

Markets are keyed by hub and asset, each with its own cash, shares, indexes,
and revenue. One pool provides custody for all markets. Repeated listings of
one token retain distinct books but share its physical balance and token risks.

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

Rates use RAY per millisecond, with accrual divided into bounded chunks. Each
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

## External execution and settlement

<a id="adr-0010"></a>

### ADR-0010: Cash flash-loan repayment

Cash flash loans require a contract receiver and allowance of at least
principal plus fee. The pool pulls exactly that amount and checks its expected balance after
payout, after callback and after collection. Callback pushes do not replace
repayment; excess allowance is permitted. Cash flash loans create no
account debt and are distinct from account strategy settlement.

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
Registries need not be unique; Aquarius LP constituents come from the pool.
A valid instruction stream does not establish good economic execution.

<a id="adr-0019"></a>

### ADR-0019: Liquidation share credit

Liquidation can credit an authorized account in the same spoke, avoiding a
collateral cash payout. Seized shares split into receiver credit and protocol
revenue by reclassification; the fee does not mint unbacked shares.

Receiver position limits still apply, and a newly credited asset needs a
listing. Existing positions retain their risk tuple; new positions use the
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

## Verification boundaries

<a id="adr-0017"></a>

### ADR-0017: Verification and deployment artifacts

Feature gates separate testing and verification helpers from deployment code.
Release-WASM checks inspect forbidden exports for the artifacts and build inputs
they cover. Formal results also depend on their harness and external-call
assumptions. The [proof guide](certora-sunbeam-prover-tuning.md) explains how to
interpret that evidence.
