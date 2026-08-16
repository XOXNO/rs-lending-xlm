# 0008. Halt flags gate liquidation legs

**Status:** Accepted

## Decision

A global pause blocks risk-increasing activity while preserving exits where
possible. At listing level there are three independent flags:

- `frozen` blocks new exposure but permits exits.
- `paused` blocks every *user* verb on the listing, including the debt leg of a
  liquidation that names that asset.
- `no_seize` blocks the liquidation *seizure* leg, and nothing else.

Pausing represents an unsafe asset state; freezing represents a closed-to-new-risk
market; halting seizure represents collateral the protocol does not want handed to
liquidators.

### Why seizure has its own flag

Pause is a *user-activity* halt. Seizure is not user activity, and gating it on
`paused` scales badly: seizure is pro-rata across **all** of an account's
collateral, so pausing one listing would halt liquidation of every account that
holds any of it — not just accounts a liquidator would have targeted there. A
listing-level halt would become a protocol-wide liquidation halt. Aave's narrower
version of this was a Medium finding (CS-AAVE4-002), fixed the same way: let
liquidation-critical calls through on paused reserves.

The debt side needs no equivalent change. The pause check on the repayment leg
iterates only the payments the liquidator chose, so a paused debt asset is
opt-in and can never block a liquidation the liquidator wants to perform.

### Ratchet

`no_seize` follows the same one-way ratchet as `paused` and `frozen`: the
guardian's immediate `set_spoke_asset_flags` may only set it, never clear it.
Clearing stays timelocked through `edit_asset_in_spoke`. This preserves ADR-0007.

### Scope: `no_seize` gates the leg, it does not define it

`no_seize` decides *whether* a seizure may happen. How a seizure settles — in
underlying or as credited shares — is a separate decision recorded in
[ADR-0019](0019-share-credit-liquidation.md), along with the account binding it
requires, its cap-usage arithmetic, and the `LiqSeize`/`LiqCredit` event
contract. The flag applies identically to both settlement modes.

### Operating a pause: interest keeps accruing

A paused listing keeps accruing interest while repayment of it is disabled, so a
borrower accumulates debt they cannot pay down for as long as the pause lasts.
This is a direct consequence of the halt semantics above, not a defect, and Aave
V4 carries the identical behaviour (ChainSecurity note 8.2).

Operator guidance:

- Treat a pause as short-lived. The interest accrued over minutes or hours is
  immaterial; over weeks it is not.
- If a pause must persist, drop the listing's rate curve via
  `upgrade_liquidity_pool_params` so the debt stops growing meaningfully. This
  is the same lever Aave points at, and it is timelocked.
- Prefer `frozen` when the intent is only "no new exposure". Frozen permits
  exits, so borrowers can still repay and no debt is trapped.
- Reserve `paused` for a genuinely unsafe asset, and `no_seize` for the narrower
  case where the asset must not be handed to liquidators.

## Guarantees

- Operators can stop new risk without necessarily trapping users.
- A paused debt asset cannot be repaid or liquidated until governance resolves
  the condition.
- Pausing a collateral listing never makes its holders unliquidatable.
- A delisted asset (no listing config at all) remains both exitable and seizable.
- The distinction is visible and testable rather than an implicit convention.

## Auditor focus

Model each verb against global pause, frozen, paused, `no_seize`, delisted, and
liquidation entry and exit legs. Liveness consequences are security-relevant, in
both directions: a halt that is too broad strands solvent liquidations, and one
that is too narrow leaves an unsafe asset reachable.

## Proposed amendment (2026-08-16) — not shipped

**Status:** Draft. No production change in this revision.

`no_seize` does not block supply (`require_can_supply` uses `BlockOnEntry`:
`paused` and `frozen` only). Seizure is pro-rata and every surviving seize
leg is gated by `SeizureLeg`. So after the guardian sets `no_seize` on listing
X, any account that *then supplies* X cannot be liquidated — including its
other collateral. Existing holders of a just-flagged X are already stuck.
A planned seize that floors to zero units is dropped *before* the flag check,
so a tiny no-seize position neither shields nor is protected.

This ADR's guarantees only claim that *pausing* a collateral listing never
makes holders unliquidatable. The `no_seize` shield is therefore in-scope for
the flag as written, and is a governance footgun rather than an accidental
bypass.

### Options (do not land A or B as a silent patch)

| Option | Change | Why not (or when) |
|---|---|---|
| **A** — `require_can_supply` rejects `no_seize` | Stops *new* shields | Contradicts “`no_seize` blocks the seizure leg, **and nothing else**” and `no_seize_does_not_block_entry_or_exit`. Needs this ADR rewritten. Does not unstick existing holders. |
| **B** — skip `no_seize` legs and seize the rest | Existing holders stay liquidatable | Breaks `proportion_seized = weighted / total`, which is the scalar `max_hf_preserving_bonus_bps` uses. Re-allocate or under-seize both invalidate V-6 / CS-AAVE4-009. Not a small change. |
| **C — recommended** — couple at the setter: `no_seize` implies `frozen` | `require_flag_ratchet` / `set_spoke_asset_flags` require `frozen` whenever `no_seize` is set. Frozen already blocks new supply. | One assert. No seizure-math change. Existing holders still need `force_socialize_bad_debt` (owner). Rewrite `set_spoke_asset_flags_tightens_no_seize_independently`. |

**Open question this amendment must answer before any code:** is `no_seize` a
*live* flag on a book with active borrowers, or a *wind-down* flag that must
travel with `frozen`? Option C is the wind-down reading.

Until that is decided, `force_socialize_bad_debt` is the hatch for insolvent
accounts that `no_seize` has made unliquidatable. See
[force-socialize-bad-debt](../../reference/runbooks/force-socialize-bad-debt.md).
