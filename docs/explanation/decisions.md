# Design decisions

Each record states a protocol rule and its trade-off; exact properties,
arithmetic and risks are in [Invariants](../reference/invariants.md),
[Formulas](../reference/formulas.md) and the [Threat model](threat-model.md).
Read [Architecture](../reference/architecture.md) first. ADR numbers are
stable anchors, grouped by topic, so out of sequence.

## Authority and emergency control

<a id="adr-0001"></a>

### ADR-0001: Governance, controller, and pool authority

Governance owns the controller under the deployment helpers, and the
controller owns the pool. The controller applies account and risk policy; the
pool handles custody and accounting.

**Trade-off:** one shared trust boundary: controller bugs, upgrades, or
authorized ownership changes affect every market.

<a id="adr-0006"></a>

### ADR-0006: Typed proposals and bound execution

Governance validates intent at proposal time and executes the bound payload
only after its delay, within its grace window; targets revalidate at
execution. Reuse needs a fresh proposal and delay. Anyone can execute: no
executor identity, no role check. Cancellation and owner-dependent recovery
have their own rules.

**Trade-off:** security depends on the effective review window and key
custody; see
[governance powers](threat-model.md#governance-windows-and-emergency-powers).

<a id="adr-0007"></a>

### ADR-0007: Emergency ratchet

Guardians pause the controller and tighten listing flags at once; reopening
is delayed:

- A listing edit can keep or tighten flags, never clear one.
- Only the timelocked `relax_spoke_asset_flags`, bound to the listing's flags
  epoch, clears a flag.
- Every flag change advances the epoch, so a relaxation proposed before a
  later guardian action reverts.
- A guardian call naming flags the listing already holds fails with
  `SpokeAssetFlagsUnchanged` and leaves the epoch alone.
- The ORACLE role can narrow sanity bands. Widening needs timelocked oracle
  reconfiguration.

The [listing-freeze runbook](../reference/runbooks/freeze-a-listing.md) gives
the operator steps.

**Trade-off:** a guardian can delay a relaxation, but the epoch moves only on
a real tightening, so it invalidates a pending relaxation at most once per
flag and cannot hold a listing closed by re-asserting flags.

<a id="adr-0008"></a>

### ADR-0008: Independent halt flags

The `frozen`, `paused`, and `no_seize` flags gate different actions: entry
checks `paused` and `frozen`, ordinary exits check `paused`, and collateral
seizure checks `no_seize`. A paused debt leg does not block repayment of
another selected leg, and pausing collateral alone does not block its seizure.
A listing pause applies to one spoke; the market's pool serves the whole hub.

**Trade-off:**

- `no_seize` does not stop new supply, so non-dust `no_seize` collateral can
  block pro-rata liquidation. Inspect usage before setting it; use `frozen` to
  stop entry.
- Interest continues during a listing pause, while its borrowers cannot
  repay; another spoke listing the asset can still move utilization and rate.
- A pause has no liquidation grace period: whoever executes the reopening can
  liquidate next transaction.
- Pair a long pause with a timelocked rate cut or a borrow freeze of the asset
  in every spoke, and reopen only after notice to borrowers.
- Insolvent accounts have the governed
  [force-socialize path](../reference/runbooks/force-socialize-bad-debt.md).

<a id="adr-0009"></a>

### ADR-0009: Immutable spoke binding

An account keeps its spoke for life; no path rebinds it. Governance can
change listings, and anyone can refresh stored risk values with
`update_account_threshold`.

**Trade-off:** listing changes still reach the account, but no caller can move
debt into another risk regime by changing an argument.

<a id="adr-0023"></a>

### ADR-0023: Delegated listing authority through the curator

The curator lets independent registrars run their own hubs and spokes while
governance stays controller owner. It holds the governance `CURATOR`,
`PROPOSER` and `CANCELLER` roles. Each registrar has an admin and members with
role bits:

| Role | Allows |
|---|---|
| `ROLE_MARKET` | Register markets, set cap grants, queue listings, listing edits and rate changes |
| `ROLE_SPOKE` | Create hubs and spokes |
| `ROLE_FLAGS` | Tighten listing flags, queue a flag relaxation |

- Changes that expose no funds are immediate: `create_hub`, `add_spoke`,
  `create_market` and `tighten_flags`. A new market takes no supply or debt
  until a spoke lists it.
- Listings, listing edits, rate changes and flag relaxations go through
  `propose` and wait for the governance timelock.
- A registrar cancels queued operations only for hubs or spokes it owns, so
  a hub owner can stop a listing queued under a grant it later revoked.

The [curator README](../../contracts/curator/README.md) lists each entrypoint.

**Trade-off:** registrars list without their own governance proposal but
cannot skip the timelock.

## Accounting and loss allocation

<a id="adr-0002"></a>

### ADR-0002: Central custody, separate market books

Markets are keyed by hub and asset, each with its own cash, shares, indexes
and revenue. One pool holds custody for all; spokes listing the same hub asset
share its book.

**Trade-off:** one token's markets in different hubs keep distinct books but
share its pool balance and token risks.

<a id="adr-0003"></a>

### ADR-0003: Indexed shares and directed rounding

Supply and debt use shares, with interest applied through indexes. Partial
operations round toward the protocol: supply mints floor shares, withdrawal
burns ceiling shares, borrowing mints ceiling debt, and partial repayment
burns floor debt.

**Trade-off:** accrual touches no account, but close paths and dust handling
do not all follow these rules; see [formulas](../reference/formulas.md).

<a id="adr-0012"></a>

### ADR-0012: Supplier-index loss allocation

Eligible bad debt is removed by lowering the affected market's supply index,
down to a nonzero floor. Supplier claims in that market bear the write-down.

**Trade-off:** the floor protects share conversion but can leave unpaid
backing, so a displayed claim is not a guaranteed withdrawal.

<a id="adr-0013"></a>

### ADR-0013: Credit measured receipts

Supply, repayment, recapitalization, and supported strategy legs credit the
amount received. An under-delivered liquidation repayment reduces the
associated seizure. A liquidation that seizes collateral below 3 decimals
reverts on a short receipt instead.

**Trade-off:** this supports specific receipt-tax behavior, not rebases,
sender surcharges or false balances; see
[token assumptions](threat-model.md#token-assumptions).

<a id="adr-0015"></a>

### ADR-0015: Literal asset-unit caps

Exposure growth converts asset-unit caps to shares at the applicable index. A
zero cap permits no new exposure. Exits do not consume headroom.

**Trade-off:** same-spoke liquidation credit moves existing supply, and its
protocol fee reduces usage, so it needs no new supply-cap headroom.

<a id="adr-0016"></a>

### ADR-0016: Millisecond rates and chunked accrual

Accrual uses a per-millisecond RAY rate derived from the annual rate. It
splits elapsed time into chunks of at most one year (`MAX_COMPOUND_DELTA_MS`);
each chunk uses the preceding chunk's market state.

**Trade-off:** chunking bounds each step, not value overflow, cumulative
work, cadence dependence or rounding error; see
[formulas](../reference/formulas.md).

<a id="adr-0021"></a>

### ADR-0021: Gross-debt cleanup accounting

Cleanup converts remaining account supply to protocol revenue, then
socializes the account's gross debt. It does not net supply and debt first,
even within one market.

**Trade-off:** read treasury gains, supplier losses and recapitalization on
that gross basis; see the
[bad-debt section](threat-model.md#bad-debt-and-liquidation).

<a id="adr-0022"></a>

### ADR-0022: Liquidator payoff at collateral equal to debt

Liquidator profit jumps where collateral `C` falls below debt `D`, and the
protocol keeps this shape (finding R-23, September 2026 upgrade review). With
`x = C / D` and base bonus `b`:

| Region | Liquidator profit | Lender loss |
|---|---|---|
| `1 <= x < 1 + b` (band) | `C - D` | 0 |
| `x < 1` (insolvent) | `C * b / (1 + b)` | `D - C / (1 + b)` |

At `b = 5 %` the profit goes from 0.50 % of `D` at `x = 1.005` to 4.74 % of `D`
at `x = 0.995`, equally for 7-decimal and 0-decimal collateral.

Each alternative is worse:

- A payoff that never rises as `x` falls must pay 0 below `x = 1`, so no
  liquidator closes insolvent accounts and more bad debt stays open.
- A continuous insolvent bonus lets a liquidator earn more by splitting one
  liquidation into many parts. The Certora rule
  `split_liq_chain_bound_holds_when_health_never_recovers` depends on a
  constant insolvent bonus.
- A band quote of `floor(C / (1 + b))` leaves debt with no collateral: 4.29 %
  of `D` at `x = 1.005`.

A liquidator cannot push an account across `x = 1`: the HF-preserving cap
keeps `1 + bonus <= C / D` on a solvent account, so only price or interest
moves `x`.

**Trade-off:** a liquidator can wait for `x < 1`, giving up the band-top
payoff (`b * D`). The gain from waiting is at most `D * b / (1 + b)` per
account; governance lowers that bound on a listing by lowering its base bonus.
The tests `the_liquidator_payoff_jumps_where_collateral_falls_below_debt` and
`zdc_liquidator_payoff_jumps_where_collateral_falls_below_debt` pin the shape;
a payoff change updates both and
[formulas](../reference/formulas.md#bonus-and-target-repayment).

## Price validation

<a id="adr-0004"></a>

### ADR-0004: Dual-source agreement

A dual-source key serves the integer midpoint, rounded down. Both legs must
pass validity, freshness, and tolerance checks, and the result must pass its
sanity band; one healthy leg is not a fallback. Runtime tolerance uses the
half-up rounded larger/smaller ratio; admission validates the reciprocal pair.
Single-source keys have separate admission constraints.

**Trade-off:** price availability needs source availability; see
[price risks](threat-model.md#price-integrity-and-availability).

<a id="adr-0005"></a>

### ADR-0005: Fail-closed valuation

A valuation-dependent operation must obtain every required valid price in its
context; a price failure aborts it.

**Trade-off:** risk decisions stay safe at the cost of withdrawal and
liquidation availability during outages. A borrower can hold a dust leg of a
failing asset to block its own liquidation
([DoS.1](threat-model.md#dos1-detail)); repayment reads no price, so anyone
can repay a dust debt leg and then liquidate, unless the listing is paused.

<a id="adr-0014"></a>

### ADR-0014: Governed source admission

Governance admits a price source only after these checks:

- Admission validates the source's structure, dependencies, provider-address
  overlap, and smoothing policy.
- Smoothing chooses which source to use; it does not transform the midpoint.
  Reflector `Twap` mode takes an equal-weight mean of its accepted
  observations.
- An LP configuration is sole-source. It waives smoothing and tolerance checks
  and uses a separate sanity cap alongside pool and underlying-price
  validation.
- Each changed key gets a provider-specific attestation and a live probe. A
  non-LP probe permits market-condition failures; LP admission requires a
  usable price. Curve attestation is a no-op; its probe enforces the pool
  binding.
- Keys that depend on a changed key, directly or transitively, get structural
  revalidation only, with no new live probe.

**Trade-off:**

- Skipping dependent probes limits repeated provider calls and memory. An LP
  gets live suitability checks on its own admission, not on upstream-key
  edits.
- Provider-address checks do not prove independent operators or upstream
  data. Feed-nature labels remain configuration assertions.

## External execution and settlement

<a id="adr-0010"></a>

### ADR-0010: Cash flash-loan repayment

A cash flash loan needs a contract receiver and an allowance of at least
principal plus fee; the pool pulls exactly that and checks its expected
balance after payout, callback and collection. A transfer to the pool during
the callback fails the post-callback check, so only the pull repays. Excess
allowance is allowed.

**Trade-off:** cash flash loans create no account debt, so they are separate
from account strategy settlement.

<a id="adr-0011"></a>

### ADR-0011: Measure router settlement

The controller authorizes one transfer of exactly the input amount to the
configured router, with no token allowance. Settlement trusts measured
balances, not router return values:

- It ignores the amounts the router returns.
- It rejects a measured spend above the authorized input.
- It requires a positive measured output.
- The router, not the controller, checks the minimum output after fees and
  before payout.
- Unused input that the controller holds returns to the caller. Residuals
  left inside the router follow a capped admin-revenue policy.

**Trade-off:** the final account risk checks do not guarantee route quality.

<a id="adr-0018"></a>

### ADR-0018: Compact route instructions

A route is a compact instruction stream whose instructions index into two
bounded registries, one of addresses and one of amounts.

- Decoding checks the format, every index bound, each Prev link (an input
  taken from the previous instruction's output), and each split weight.
- At runtime, vault accounting checks available balances and venue receipts.
- A split weight applies to the remainder at execution, so weights need not
  sum to one.
- Registry entries need not be unique. LP (Aquarius or Curve) constituents
  come from the pool, not the registry.

**Trade-off:** a valid instruction stream does not establish good economic
execution.

<a id="adr-0019"></a>

### ADR-0019: Liquidation share credit

Liquidation can credit a `Normal`-mode account in the same spoke that the
liquidator owns or is a delegate for, avoiding a collateral cash payout.
Seized shares split into receiver credit and protocol revenue by
reclassification, so the fee mints no unbacked shares.

- Position limits apply to an existing receiver; a `Credit(0)` receiver is
  exempt.
- A newly credited asset needs a listing. Existing positions keep their risk
  tuple; new positions use the listing's values.
- The target's seizure bounds the receiver's supply increase, and the
  receiver's debt does not move.
- The [event reference](../reference/events.md) distinguishes gross LiqSeize
  from net LiqCredit.

**Trade-off:** controller account-isolation proofs exempt the declared
receiver and frame a third account. The receiver-credit bound does not prove
pool fee allocation, because the model summarizes that call; see
[model boundaries](../../certora/README.md#model-boundaries).

<a id="adr-0020"></a>

### ADR-0020: Zero-fee flash-position callback

Flash position creates fee-free debt and forwards it to a caller-chosen
contract callback. It requires nonnegative collateral minima, at least one
positive, an eligible flash-loan debt market, and an open, solvent final
position.

Minima bind measured controller receipts; pool supply remeasures delivery.
Returned debt is never auto-repaid: it becomes declared collateral, is
refunded when refund-listed, or stays uncredited.

**Trade-off:** an authorized borrowing strategy, not a free cash-flash round
trip; a healthy account may back the debt with little added collateral. Its
origination-fee treatment differs from multiply.

## Verification boundaries

<a id="adr-0017"></a>

### ADR-0017: Verification and deployment artifacts

Feature gates keep testing and verification helpers out of deployment code;
release-WASM checks inspect forbidden exports for the artifacts and build
inputs they cover. One gate changes behavior instead of adding an entrypoint:
the governance
`short-timelock` feature lowers the sensitive delay floor from 120960 to 12
ledgers for the end-to-end build. That build stamps a `timelock` contract-meta
entry, and the candidate check fails a governance candidate that carries it.

**Trade-off:**

- Live runs deploy the short-timelock build from the fixtures directory, so
  the released governance Wasm is the one candidate the network run does not
  execute; native tests cover it.
- Formal results depend on their harness and external-call assumptions; see
  [Reading a Sunbeam verdict](../../certora/README.md#reading-a-sunbeam-verdict).
