# 0015. Caps are declared in asset units, converted at the live index per call, and zero means zero

Status: Accepted

## Context

Per-spoke-asset supply and borrow caps bound how much exposure a listing may
accumulate. Three forces shape the design. Operators think in token amounts
("cap USDC supply at 10M"), while positions are stored as RAY-scaled shares
whose real value grows with the interest index — so the cap must be stated in
one unit and enforced in the other. Interest accrual can push existing usage
above any fixed ceiling, so the design must decide whether a breached cap
traps users inside the market. And configuration mistakes are inevitable: the
semantics of an absent or zero cap decide whether a fat-fingered listing fails
open (unbounded market) or fails closed (no entry).

## Decision

Caps are stored in asset base units on the spoke-asset config
(`SpokeAssetConfig::supply_cap` / `::borrow_cap`, selected by
`contracts/controller/src/spoke/caps.rs::UsageSide::cap`) and converted to
scaled-share space at every enforcement, using the market index current at
that call: `contracts/controller/src/spoke/caps.rs::cap_to_scaled` computes
`floor(from_asset(cap) * RAY / index)` with saturating math, so the ceiling
applies to present balances including all accrued interest.

Only entries are checked. `contracts/controller/src/spoke/caps.rs::enforce_spoke_cap`
asserts `usage + delta <= cap_scaled` and reverts with the side-specific error
(`SpokeSupplyCapReached` / `SpokeBorrowCapReached`); it is reached from every
entry leg via `contracts/controller/src/context/spoke.rs::apply_spoke_entry`.
Exits never consult the cap: `contracts/controller/src/spoke/caps.rs::SpokeUsageContext::apply_exit`
subtracts usage with overflow and non-negativity checks only, and a missing
usage row on exit is a deliberate no-op (`MissingUsage::Absent`), so
withdrawal, repayment, and liquidation stay open regardless of cap state.

There is no unlimited sentinel. `cap == 0` yields `cap_scaled == 0`, so any
positive entry reverts — a zero cap is a soft wind-down in which the market
only shrinks. Both sides are pinned by
`contracts/controller/tests/spoke.rs::zero_supply_cap_rejects_entry` and
`::zero_borrow_cap_rejects_entry`, and production config relies on it:
collateral-only listings in `configs/mainnet/spokes.json` carry
`"borrow_cap": "0"`. At listing time,
`common/src/validation.rs::require_cap_within_asset_domain` (called from
`contracts/controller/src/config/asset.rs`) rejects caps that do not fit the
asset's RAY-upscaled domain, so the conversion in `cap_to_scaled` cannot
overflow into a silently smaller ceiling.

## Alternatives

- **Zero means unlimited (the Aave-style sentinel).** Operators would express
  "no cap" as `0` and real caps explicitly, with something like `i128::MAX`
  never needed. But then the dangerous default and the omitted-field default
  coincide: a forgotten cap field lists an unbounded market. Under
  zero-means-zero, the same omission lists a closed one — the failure mode is
  an inconvenience, not an exposure. Wind-down also falls out for free rather
  than needing a dedicated flag.
- **Caps denominated in scaled shares.** Enforcement would be a plain integer
  comparison with no per-call index conversion, and the cap would never be
  breached by accrual alone. But the cap's real-token meaning would silently
  shrink as the index grows, so an operator-set "10M USDC" ceiling would drift
  upward in token terms forever. Asset-unit caps keep the operator's mental
  model and the enforced quantity identical.
- **Checking caps on exits too.** Symmetric enforcement is simpler to state,
  but interest accrual alone can push usage above cap, and blocking exits in
  that state would trap suppliers and borrowers inside an over-cap market —
  inverting the cap's purpose from bounding exposure to preserving it.

## Consequences

Easy: wind-down of a listing is one governance write (set caps to zero) with
no new mechanism; collateral-only listings are just `borrow_cap: 0`;
misconfiguration fails closed, so a review miss costs availability, not
solvency. The cap tracks true economic exposure because conversion happens at
the live index.

Hard: usage can legitimately sit above cap after accrual, so operators and
indexers must treat "over cap" as a valid steady state that only blocks
further entry. Every entry leg pays the index conversion, and cap semantics
are coupled to index correctness (see ../../reference/invariants.md
§INV-IDX).

Must stay true: exits remain cap-exempt so liquidation and repayment can
always shrink a market (see ../../reference/invariants.md §INV-RISK and
§INV-LIQ); listing-time domain validation keeps `cap_to_scaled` overflow-free;
and the zero-cap meaning stays "closed", because flipping it to "unlimited"
inverts the fail-closed posture the configuration workflow assumes (see
../threat-model.md).
