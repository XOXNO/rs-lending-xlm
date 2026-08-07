# 0009. Spokes as an immutable per-account risk binding, chosen once at account creation

Status: Accepted

## Context

Risk configuration — LTV, liquidation threshold and bonus, caps, halt flags, the
liquidation curve — must be segmentable: the same market can back a conservative cohort
and an aggressive one, or a collateral-only cohort, without either inheriting the other's
parameters. That segmentation only stays sound if every account resolves to exactly one
risk regime at all times: an account that could hop between regimes mid-life would need
atomic re-validation of its entire position set against the destination's parameters and
caps, and any ambiguity about which regime applies corrupts health-factor math, cap
accounting, and liquidation-curve selection. At the same time, the parameters *inside* a
regime must be able to evolve — governance retunes thresholds — without silently making
existing depositors instantly liquidatable.

## Decision

Every account binds to a `spoke_id` exactly once, at creation.
`contracts/controller/src/account/mod.rs::create_account` asserts `spoke_id >= 1`
(`SpokeError::SpokeNotFound`), requires the spoke non-deprecated via
`contracts/controller/src/context/spoke.rs::Cache::active_spoke`
(`SpokeError::SpokeDeprecated`), and writes `spoke_id` and `mode` into `AccountMeta`
through `contracts/controller/src/storage/account.rs::set_account_meta` — whose sole call
site is account creation, so the binding is never rewritten.

Every subsequent operation must present the matching spoke id:
`contracts/controller/src/account/mod.rs::require_spoke_match` reverts
`SpokeError::SpokeMismatch` on any other id, and
`context/spoke.rs::Cache::ensure_spoke_context` pins a single spoke per transaction,
rejecting a second id inside one flow.

Parameters evolve; the binding does not. Risk parameters are stamped onto positions and
re-stamped from live spoke config on health-reducing verbs
(`contracts/controller/src/risk/params.rs::refresh_supply_risk_params`), with
liquidator-favoring changes gated by
`risk/params.rs::apply_gated_liquidation_params`: a tightened threshold, raised bonus, or
lowered fee is applied to an indebted position only if the account's hypothetical health
factor under the new threshold clears a 1.05-WAD floor
(`risk/params.rs::clears_min_hf`), so a governance retune cannot instantly convert a
healthy position into liquidation prey.

Spokes are deprecated, never deleted.
`contracts/controller/src/config/spoke.rs::remove_spoke` only sets `is_deprecated`, which
blocks new accounts and new entries while leaving existing accounts fully resolvable,
exitable, and liquidatable.

## Alternatives

- **User-migratable spokes.** A `migrate_account(new_spoke_id)` verb would let users
  chase better parameters without unwinding positions. It requires atomically
  re-validating the whole position set against the destination spoke — every listing must
  exist there, every cap must absorb the moved usage, and the health factor must hold
  under the new thresholds — and it moves `SpokeUsage` cap accounting between spokes in
  one step. The immutable binding gets the same end state through exit-and-re-enter,
  which reuses the ordinary entry gates instead of a bespoke migration path.
- **Global per-asset risk config without spokes.** One parameter set per asset is
  simpler, but it forfeits segmentation: no collateral-only cohort next to a borrowable
  one, no conservative/aggressive tiers over shared liquidity, and every retune hits all
  users of the asset at once.
- **Address-keyed positions instead of account ids.** Keying by wallet address would
  remove the account-id indirection but caps each wallet to a single risk profile. With
  numbered accounts, one wallet holds several accounts across spokes and modes, and
  delegation attaches to the account rather than the wallet.

## Consequences

- One account resolves to exactly one risk regime, always: health-factor math,
  liquidation-curve selection (`SpokeLiquidationCurve`), and cap accounting
  (`SpokeUsage`, keyed by spoke id) never face an ambiguous or mid-flight binding — see
  ../../reference/invariants.md §INV-RISK and §INV-ACCT.
- Moving to a different risk regime means exiting positions and re-entering under a new
  account; there is no in-place migration, and any future one would have to be invented
  along with its atomic re-validation.
- Spoke wind-down is slow by construction: deprecation stops growth but existing
  accounts persist until they exit, so operators cannot reclaim a spoke id or force
  closure — funds can never be stranded by deprecation (deprecate-not-delete keeps every
  stored `AccountMeta` resolvable; see ../../reference/invariants.md §INV-STOR).
- Parameter retunes are safe against instant-liquidation griefing by the 1.05-WAD
  hypothetical-HF gate, at the cost that stale liquidator-favoring parameters can linger
  on positions the gate protects until their health recovers (the position keeps its
  stamped values, which bounds the blast radius of a hostile or mistaken retune — see
  ../threat-model.md).
- The single-spoke-per-transaction pin means composite flows (strategies, liquidation)
  cannot mix accounts from different spokes in one call; batch tooling must group work
  by spoke.
