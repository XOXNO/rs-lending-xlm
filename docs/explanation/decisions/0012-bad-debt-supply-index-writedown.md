# 0012. Bad debt is socialized by writing the supply index down, with a hard floor

Status: Accepted

## Context

When an account's debt exceeds its collateral, the shortfall is real: some
supplier claims are no longer backed by anything. Leaving the position on the
books poisons the market — utilization and rates are computed against debt that
will never repay, and the last suppliers to withdraw absorb the entire hole.
The protocol needs a mechanism that removes insolvent positions, assigns the
loss deterministically and immediately, and cannot itself be weaponized: a
permissionless cleanup path invites griefing (socializing accounts that still
hold meaningful collateral worth liquidating), and an unbounded write-down could
drive the supply index toward zero, where share arithmetic degenerates.

## Decision

Socialization is a supply-index write-down, gated by insolvency plus a dust cap
on the permissionless path. The controller admits an account through
`contracts/controller/src/positions/liquidation/mod.rs::BadDebtGate`:
the permissionless `process_clean_bad_debt` uses `DustCapped`, i.e.
`contracts/controller/src/positions/liquidation/curve.rs::is_socializable_bad_debt`
(`total_debt > total_collateral` and `total_collateral <= BAD_DEBT_USD_THRESHOLD`,
5 WAD in `contracts/controller/src/constants.rs`); the owner-only
`process_force_socialize_bad_debt` uses `Insolvent`, which drops the dust cap.
An admitted account has every position sent to the pool's `seize_positions`
entrypoint (`contracts/pool/src/lib.rs`).

Per entry, `contracts/pool/src/ops/seize.rs::apply` branches on side:

- **Borrow:** the position is valued as bad debt at ceil
  (`unscale_borrow_ceil_ray`), `contracts/pool/src/interest.rs::apply_bad_debt_to_supply_index`
  scales the supply index by `remaining / total_supplied_value` with floor
  rounding, clamps the result at `SUPPLY_INDEX_FLOOR_RAW = RAY / 1_000`
  (`common/src/constants/pool.rs`), and the debt shares are burned.
- **Deposit:** `absorb_supply_as_revenue` reassigns the shares to protocol
  revenue without changing `supplied` — remaining collateral offsets the loss
  rather than vanishing.

Losses hit all suppliers of that market pro-rata and instantly, priced into the
index the moment the seize commits. This makes the supply index the one
non-monotone quantity in the system; the borrow index only ever grows. The
write-down arithmetic is proved by
`certora/pool/spec/seize_settle_accounting_rules.rs::seize_borrow_reduces_debt_and_writes_down_supply`,
including the floor exception.

## Alternatives

**A first-loss reserve or insurance fund.** Protocol revenue would be drawn down
before supplier claims are touched. This softens supplier UX but adds a second
accounting domain (fund sizing, replenishment policy, ordering against
concurrent seizes) and, critically, only defers the same write-down once the
fund is empty. The implemented design keeps the loss path single-step and
always-defined; governance can still route revenue toward recapitalization
out-of-band.

**Tracking bad debt as a standing liability repaid by future revenue.** No
immediate index cut; instead a ledger of unbacked debt slowly amortized. This
keeps the index monotone but makes the market's stated exchange rate a lie in
the interim — early withdrawers exit at the pre-loss rate, late ones eat the
whole hole, which is precisely the bank-run dynamic socialization exists to
remove.

**Unbounded write-down with no floor.** Simpler, but a catastrophic loss could
push the index to zero (or near it), where scaled-share conversions divide by
vanishing values and new deposits mint absurd share counts. The `RAY/1_000`
floor keeps share arithmetic in a sane domain; any loss the floor absorbs
surfaces instead as a backing shortfall
(`contracts/pool/src/guards.rs::backing_shortfall`) repayable through
`contracts/pool/src/ops/recapitalize.rs`.

## Consequences

Solvency accounting closes immediately: after a seize, remaining supplier claims
are fully backed (up to the floor case), which is what keeps the ACCT and LIQ
domains of ../../reference/invariants.md provable per-operation. The dust cap
bounds the permissionless path — an account worth liquidating cannot be
socialized by a griefer, only swept once its collateral is economically inert
(see ../threat-model.md for the griefing analysis).

What it makes hard: every integrator, exporter, and off-chain accounting system
must tolerate a supply index that can decrease — IDX-domain monotonicity holds
for the borrow index only. Suppliers in a market carry tail risk with no notice
period; that is the explicit price of run-free exits.

What must stay true: the write-down happens only through the seize path, floor
included; deposit-side seizure keeps `supplied` unchanged (revenue absorbs the
shares, so the index is untouched); and any residual shortfall the floor leaves
behind remains visible via `backing_shortfall` until recapitalized. Changing the
threshold constant or the floor re-opens both the Certora proofs and the griefing
analysis.
