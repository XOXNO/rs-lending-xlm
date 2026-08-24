# Runtime invariants

These are the properties an audit should try to falsify. Most are enforced on a
live execution path, covered by tests, or specified formally. A few clauses are
operational consequences of Soroban storage semantics rather than protocol
checks; those are marked as unenforced. A passing test or model is evidence, not
a substitute for reviewing the deployed configuration and integration
assumptions.

Every invariant below carries a **Status** line:

- `ENFORCED` — a runtime check on a live execution path rejects the violation.
  The line names the file and the symbol that does it.
- `VERIFIED` — a test or a Certora rule proves it. Rule names are the `#[rule]`
  functions under `certora/`; test paths point at the file that asserts it.
- `NOT ENFORCED` / `VERIFICATION GAP` — the property is relied upon but nothing
  in this repo checks or proves it. Treat these as open audit surface.

## Authorization

### INV-AUTH-01 — One ownership chain

Governance controls the controller; the controller controls every
state-changing pool action. No user or external component can mutate pool
accounting directly.

**Status:** ENFORCED — `contracts/pool/src/lib.rs` sets the controller as owner
at construction and marks every mutator `#[only_owner]`. VERIFIED —
`tests/test-harness/tests/controller/ownership.rs`;
`contracts/controller/tests/entrypoints.rs`.

### INV-AUTH-02 — Risk-reducing authority is explicit

Borrowing and withdrawing require the account owner or a delegate that is both
listed on the account and active as a position manager. Delegates cannot grant
or renew their own authority.

**Status:** ENFORCED — `contracts/controller/src/account.rs`
(`is_owner_or_delegate`, `require_account_owner`). VERIFIED — rule
`supply_new_slot_requires_owner_or_delegate`;
`tests/test-harness/tests/controller/account.rs`.

### INV-AUTH-03 — Permissionless actions do not create foreign risk

Third parties may repay, liquidate, recapitalize, and perform maintenance.
Third-party supply can only top up an already existing supply position. These
paths must not create an unwanted account slot or increase another user’s risk.

**Status:** ENFORCED — `contracts/controller/src/positions/supply.rs`. VERIFIED
— rule `supply_new_slot_requires_owner_or_delegate`;
`tests/test-harness/tests/controller/liquidation.rs`
(`test_third_party_supply_self_liquidation_allowed`).

### INV-AUTH-04 — Emergency power only tightens

Immediate guardian power can pause and add restrictions. It cannot unpause or
clear a restriction. Reopening is timelocked.

**Status:** ENFORCED — `contracts/controller/src/config/asset.rs`
(`require_flag_ratchet`); `contracts/governance/src/api.rs` limits the guardian
to pause and flag-setting; `unpause` is `#[only_owner]` in
`contracts/controller/src/lib.rs`. VERIFIED —
`contracts/controller/tests/config/asset_flags.rs`
(`set_spoke_asset_flags_rejects_unpause`,
`set_spoke_asset_flags_rejects_clearing_no_seize`);
`tests/test-harness/tests/controller/liquidation_ratchet.rs`.

### INV-AUTH-05 — Governance delay cannot be shortened

The governance delay is non-zero and subsequent updates can only increase it
within the supported domain.

**Status:** ENFORCED — `contracts/governance/src/timelock/mod.rs`;
`contracts/governance/src/access.rs`. VERIFIED —
`contracts/governance/tests/self_timelock.rs`
(`propose_update_delay_rejects_shortening`,
`propose_update_delay_rejects_zero`,
`propose_update_delay_rejects_above_max_cap`).

### INV-AUTH-06 — An account's spoke binding is immutable

An account is bound to one spoke at creation. Every supply, borrow, and exit
re-checks that binding and reverts `SpokeMismatch` (#310) on a mismatch, so risk
parameters cannot be swapped under a live position (ADR-0009).

**Status:** ENFORCED — `contracts/controller/src/account.rs`
(`require_spoke_match`), called from every `AccountGuard` arm. VERIFIED —
`tests/test-harness/tests/controller/spoke.rs`
(`test_supply_rejects_spoke_mismatch_on_existing_account`).

## Accounting

### INV-ACCT-01 — Supply, revenue, and debt shares are non-negative

Revenue shares are a subset of supply shares. No operation may create negative
share totals or a treasury claim with no corresponding supplied shares.

**Status:** ENFORCED — `contracts/pool/src/cache/shares.rs`
(`require_revenue_backed` after every burn or absorb; `Ray::checked_sub` traps
on negative). VERIFIED — rules `withdraw_keeps_revenue_backed`,
`net_settle_keeps_revenue_backed`, `claim_revenue_leaves_no_orphan_debt`.

### INV-ACCT-02 — Cash is the reserve book

Liquidity checks use tracked cash, not an incidental token balance. Donations
do not create lendable cash. Debits cannot make cash negative.

**Status:** ENFORCED — `contracts/pool/src/guards.rs` (`backing_shortfall`,
`require_liquidation_buffer`) reads `cache.cash()`, never a token balance.
VERIFIED — rule `withdraw_never_overdraws_cash`;
`tests/test-harness/tests/controller/supply.rs`.

### INV-ACCT-03 — Credit equals measured receipt

Inbound supply, repayment, recapitalization, and strategy settlement credit
only tokens actually received. Requested transfer amounts are not accounting
evidence.

**Status:** ENFORCED — `common/src/token.rs` (`transfer_amount_measured`), used
by `contracts/controller/src/positions/supply.rs`,
`contracts/controller/src/keepers.rs`,
`contracts/controller/src/positions/liquidation/apply.rs`, and
`contracts/controller/src/strategies/swap/balances.rs`. VERIFIED —
`contracts/controller/tests/events.rs` (fee-on-transfer token fixtures).

### INV-ACCT-04 — Backing shortfall blocks new supply

An underbacked market rejects new supply. Recapitalization can fill no more
than the shortfall and refunds excess without minting shares.

**Status:** ENFORCED — `contracts/pool/src/guards.rs` (`require_backed_market`,
`PoolInsolvent` #123); `contracts/pool/src/ops/recapitalize.rs` caps the fill at
`backing_shortfall`. VERIFIED — rules `supply_sanity`,
`withdraw_keeps_revenue_backed`;
`tests/test-harness/tests/controller/bad_debt_index.rs`.

### INV-ACCT-05 — Positive value must change shares

Any positive operation whose share conversion produces zero shares reverts.
This prevents dust transfers from moving value without a matching book entry.

**Status:** ENFORCED — `contracts/pool/src/ops/` (`supply.rs`, `borrow.rs`,
`repay.rs`, `withdraw.rs`, `net_settle.rs` each raise a `*RoundsToZeroShares`
error). VERIFIED — rules `supply_dust_amount_sanity`,
`positive_revenue_claim_with_zero_share_burn_reverts`,
`net_settle_pivot_never_leaves_zero_scaled_records`.

### INV-ACCT-06 — Revenue claims remain solvent

A revenue claim burns enough shares, respects cash and solvency limits, and
cannot pay a positive amount while burning zero entitlement.

**Status:** ENFORCED — `contracts/pool/src/ops/revenue.rs` calls
`require_utilization_below_max` and `require_solvent_withdraw_state`;
`contracts/pool/src/cache/shares.rs` (`burn_claimable_revenue`) caps at cash.
VERIFIED — rules `claim_revenue_burns_equal_shares_and_cash`,
`positive_revenue_claim_with_zero_share_burn_reverts`,
`claim_revenue_returns_nonnegative_amount`.

### INV-ACCT-07 — Borrow draws leave a liquidation cash buffer

An ordinary borrow draw must leave `LIQUIDATION_BUFFER_BPS` of supplied value in
cash, so a later seizure stays fundable. Exceeding it reverts
`InsufficientLiquidity` (#112). Only borrow draws are gated; exits are not.

**Status:** ENFORCED — `contracts/pool/src/guards.rs`
(`require_liquidation_buffer`), called from `contracts/pool/src/ops/borrow.rs`.
VERIFIED — `contracts/pool/tests/guards.rs`
(`test_require_liquidation_buffer_admits_a_draw_down_to_the_reserve`,
`test_require_liquidation_buffer_rejects_a_draw_one_unit_past_the_reserve`).

### INV-ACCT-08 — Utilization stays below the market ceiling

Borrow, user withdraw, and revenue claim each revert if the market would end
above `params.max_utilization`. The check is skipped only when supply is zero or
the ceiling is at or above RAY 1.0.

**Status:** ENFORCED — `contracts/pool/src/guards.rs`
(`require_utilization_below_max`), called from `contracts/pool/src/ops/borrow.rs`,
`withdraw.rs`, and `revenue.rs`. VERIFIED — rules
`borrow_respects_utilization_cap`, `user_withdraw_respects_utilization_cap`.

### INV-ACCT-09 — A market never ends an operation with debt and no supply

No withdraw, net settlement, or revenue claim may leave a market at zero supply
while debt is outstanding; that state has no index to write against. The
operation reverts `PoolInsolvent` (#123).

**Status:** ENFORCED — `contracts/pool/src/guards.rs`
(`require_solvent_withdraw_state`), called from
`contracts/pool/src/ops/withdraw.rs`, `net_settle.rs`, and `revenue.rs`.
VERIFIED — rule `withdraw_leaves_no_orphan_debt`.

## Interest and indexes

### INV-IDX-01 — Borrow index is monotone and bounded

Accrual cannot decrease debt value. The borrow index remains within its
configured maximum.

**Status:** ENFORCED — `common/src/rates/index.rs` caps at
`MAX_BORROW_INDEX_RAY` (`common/src/constants/pool.rs`). VERIFIED — rules
`update_borrow_index_monotonic`, `update_borrow_index_capped`,
`borrow_index_cap_is_sticky`, `borrow_index_strictly_grows_below_cap`.

### INV-IDX-02 — Supply index is bounded

The supply index stays above a non-zero floor and below its configured maximum.

**Status:** ENFORCED — `contracts/pool/src/interest.rs` floors at
`SUPPLY_INDEX_FLOOR_RAW`; `common/src/rates/index.rs` applies the cap. VERIFIED
— rules `update_supply_index_capped`, `update_supply_index_monotonic`,
`supply_index_cap_is_sticky`.

### INV-IDX-03 — Bad debt may lower the supply index

Supplier value is not monotone. Eligible socialized loss lowers only the
affected market’s supply index.

**Status:** ENFORCED — `contracts/pool/src/ops/seize.rs` syncs and commits one
cache per `hub_asset`, so `apply_bad_debt_to_supply_index`
(`contracts/pool/src/interest.rs`) can only touch that market. VERIFIED — rules
`seize_borrow_reduces_debt_and_writes_down_supply`,
`bad_debt_writedown_is_noop_on_empty_market`;
`tests/test-harness/tests/controller/bad_debt_index.rs`. VERIFICATION GAP — the
scoping holds structurally, but no rule or test asserts that a *different*
market's index is unchanged across a socialization.

### INV-IDX-04 — Accrual is time-consistent

Zero elapsed time changes nothing. Long gaps are processed in bounded forward
chunks; time never moves backward.

**Status:** ENFORCED — `contracts/pool/src/interest.rs`;
`common/src/rates/simulate.rs` chunks at `MAX_COMPOUND_DELTA_MS`
(`common/src/rates/compound.rs`, one year). VERIFIED — rules
`indexes_unchanged_when_no_time_elapsed`, `simulate_indexes_no_time_noop`,
`time_mono_market_index_non_decreasing`,
`compound_interest_identity_at_zero_delta`.

### INV-IDX-05 — Accrued interest is fully assigned

Accrued borrower interest is accounted for as supplier reward or protocol
revenue, including conservative rounding remainders.

**Status:** ENFORCED — `contracts/pool/src/interest.rs` folds
`supply_index_reward_shortfall` into the protocol reward;
`common/src/rates/index.rs`. VERIFIED — rules
`supplier_rewards_plus_fee_equals_accrued_interest`,
`accrued_interest_split_is_conservative`, `supplier_rewards_conservation`.

## Oracle and risk

### INV-ORACLE-01 — Valuation fails closed

Any missing, stale, invalid, or disagreeing required price prevents the
valuation-dependent mutation.

**Status:** ENFORCED — `contracts/price-aggregator/src/engine.rs` (`failure`
classifies the outcome; `force` panics on any failure). VERIFIED — rules
`divergent_prices_revert`, `missing_oracle_config_reverts`, `zero_anchor_reverts`,
`one_second_past_staleness_boundary_is_stale`;
`tests/test-harness/tests/controller/audit_supply_stale_shield.rs`.

### INV-ORACLE-02 — A dual source requires both legs

One functioning leg is not a fallback. Accepted blended prices stay within the
two validated source prices.

**Status:** ENFORCED — `contracts/price-aggregator/src/engine.rs` (`blend`
accepts `Legs::Two` only, marks the outcome stale if either leg is stale, and
returns a value between the two). VERIFIED — rules
`first_band_price_within_inputs`, `second_band_price_within_inputs`,
`beyond_band_price_within_inputs`, `equal_prices_within_symmetric_band`.

### INV-ORACLE-03 — One transaction sees one snapshot

All risk calculations in a mutation use one coherent set of prices.

**Status:** ENFORCED — `contracts/controller/src/context/oracle.rs` fetches once
and caches for the mutation. VERIFIED — rules `price_cache_consistency`,
`index_cache_single_snapshot`.

### INV-ORACLE-04 — Future-dated feeds are not accepted

A feed timestamp more than `MAX_FUTURE_SKEW_SECONDS` (60s,
`common/src/oracle/observation.rs`) past the ledger time is not a valid
observation. The leg is dropped, so a required asset with no other live leg
fails closed under INV-ORACLE-01.

**Status:** ENFORCED — `contracts/price-aggregator/src/observation.rs`
(`OracleObservation::from_multi_feed`, `from_reflector`) returns `None` via
`is_future_at`. VERIFIED — rules `timestamp_beyond_future_skew_reverts`,
`timestamp_at_future_skew_boundary_is_allowed`
(`certora/price-aggregator/spec/freshness_rules.rs`). VERIFICATION GAP — those
rules exercise the panicking helper `check_not_future_at`, which the contracts do
not call; the live path uses the dropping helper `is_future_at`. The bound is the
same constant, but the rules do not cover the live path.

### INV-RISK-01 — Risk-increasing actions re-prove solvency

Caps and listing rules are checked before the pool action. After the pool
action, LTV-gated collateral against total debt, the health factor, and the
minimum-collateral floor are re-proved against fresh totals, and the action
reverts if any fails.

**Status:** ENFORCED — post-action:
`contracts/controller/src/risk/validation.rs` (`require_post_pool_risk_gates`).
Entry-time: `contracts/controller/src/spoke_usage.rs` (`enforce_spoke_cap`) for
caps, `contracts/controller/src/positions/mod.rs`
(`require_listed_unhalted_config`, `require_can_borrow`, `require_can_supply`)
for listing and halt rules. VERIFIED — rules `post_gate_borrow_totals_are_final`,
`post_gate_withdraw_observes_gate_witness`, `borrow_safe_or_health_gated`,
`ltv_borrow_bound_enforced`.

### INV-RISK-02 — Conservative valuation biases safety

Collateral is rounded down, debt is rounded up, and health factor is rounded
down.

**Status:** ENFORCED — `contracts/controller/src/risk/totals.rs`
(`calculate_account_risk_totals`, `sum_debt_usd`, `calculate_ltv_collateral_wad`), mirrored by
`unscale_supply_floor` / `unscale_borrow_ceil` in `contracts/pool/src/guards.rs`.
VERIFIED — rules `hf_division_rounds_against_borrower`,
`position_value_ceil_ge_floor`, `scaled_to_actual_matches_floor_with_rounding`.

### INV-RISK-03 — Risk configuration is coherent

LTV remains strictly below liquidation threshold. Liquidation bonus and fees
cannot consume more collateral than the protocol permits.

**Status:** ENFORCED — `common/src/validation.rs` requires `threshold > ltv`,
`threshold <= BPS`, and `threshold * (BPS + bonus) <= BPS * BPS`. VERIFIED —
rules `add_asset_enforces_valid_bounds`, `edit_asset_enforces_valid_bounds`,
`derived_bonus_respects_threshold`, `bonus_bounded`.

### INV-RISK-04 — Position and delegate counts are bounded

An account cannot exceed the configured maximum supply or borrow position count
(`PositionLimitExceeded`, #109), or hold more than `MAX_DELEGATES` = 16
delegates. This is a liveness constraint as well as a risk one: a liquidation
must fit the transaction budget of the widest admissible account.

**Status:** ENFORCED — `contracts/controller/src/risk/validation.rs`
(`validate_bulk_position_limits`); `contracts/controller/src/storage/account.rs`
enforces `MAX_DELEGATES` (`contracts/controller/src/constants.rs`). VERIFIED —
`contracts/controller/tests/validation.rs`
(`test_validate_bulk_position_limits_deposit_over_cap_panics`,
`test_validate_bulk_position_limits_borrow_over_cap_panics`);
`tests/test-harness/tests/controller/borrow.rs`
(`test_borrow_position_limit_exceeded`).

## Liquidation

### INV-LIQ-01 — Only unhealthy debt can be liquidated

Liquidation requires live debt and health factor below one; it is
permissionless, and an account owner may liquidate its own account. The one
remaining identity guard is receiver-side, not caller-side: in `Credit` seize
mode, the receiving account cannot be the liquidated account itself
(`requested != account_id`, `SelfLiquidationNotAllowed` = error #133) — crediting
seized collateral back to the account it was seized from would undo the
seizure.

**Status:** ENFORCED — `contracts/controller/src/positions/liquidation/plan.rs`
(`build_liquidation_plan` requires non-empty debt and `health_factor <
Wad::ONE`); `contracts/controller/src/positions/liquidation/mod.rs` (the
`Credit`-mode receiver check). VERIFIED —
`tests/test-harness/tests/controller/security_audit_extended.rs`
(`refutation_owner_can_self_liquidate`);
`tests/test-harness/tests/controller/liquidation.rs`;
`tests/test-harness/tests/controller/position_nft.rs`.

### INV-LIQ-02 — Repayment and seizure stay coupled

Repayment is bounded by the close policy, excess is refunded, and seizure
never exceeds the current position.

**Status:** ENFORCED — `contracts/controller/src/positions/liquidation/plan.rs`
(`build_liquidation_plan` → `normalize_repayment_plan`,
`calculate_seized_collateral`, `plan.validate`). VERIFIED — rules
`ideal_repayment_targets_curve_hf`, `full_repay_refunds_overpayment`,
`liquidation_does_not_increase_seized_collateral`,
`split_liq_two_partials_never_out_seize_one_close`.

### INV-LIQ-03 — Under-delivery reduces seizure

When a token transfer delivers less than planned, collateral seizure scales down
with the measured receipt.

**Status:** ENFORCED — `contracts/controller/src/positions/liquidation/apply.rs`
floor-scales each leg's USD to the measured receipt. VERIFIED — rule
`liquidation_does_not_increase_repaid_debt`;
`contracts/controller/tests/events.rs` (1%-skim debt token fixture).

### INV-LIQ-04 — Bad-debt socialization is explicit and total

Residual debt is socialized only after its gates hold. Debt removal and loss
allocation are one atomic result: every remaining position is seized, the loss
is written to the affected markets' supply indexes, and the account entry and
its position NFT are removed in the same invocation.

**Status:** ENFORCED — gates:
`contracts/controller/src/positions/liquidation/mod.rs` (`BadDebtGate`); effect:
`contracts/controller/src/positions/liquidation/bad_debt.rs`
(`execute_bad_debt_cleanup`). VERIFIED — rules `clean_bad_debt_zeros_positions`,
`usage_liq_bad_debt_cleanup_sheds_every_wiped_position`;
`tests/test-harness/tests/controller/bad_debt_index.rs`.

**NOT ENFORCED:** there is no post-condition guard on this path.
`execute_bad_debt_cleanup` and `contracts/pool/src/ops/seize.rs` (`apply`) run no
`guards::` assertion after the writedown — unlike
`contracts/pool/src/ops/revenue.rs`, which calls `require_utilization_below_max`
and `require_solvent_withdraw_state`. The resulting market state is relied upon
to be solvent, but nothing checks it at runtime.

## Halts, caps, and storage

### INV-HALT-01 — Global pause blocks new risk

Global pause blocks risk-increasing actions while preserving safe exits where
the listing permits them.

**Status:** ENFORCED — `contracts/controller/src/lib.rs` marks supply, borrow,
flash loan, multiply, swaps, migrate, and delegate grants `#[when_not_paused]`;
withdraw, repay, and liquidate carry no such attribute. VERIFIED —
`tests/test-harness/tests/controller/admin.rs`.

### INV-HALT-02 — Frozen, paused, and no_seize gate different legs

The three listing flags are disjoint by design, and each leg honours exactly one
policy (`FreezePolicy` in `contracts/controller/src/positions/mod.rs`):

- `BlockOnEntry` — new exposure (supply, borrow). Rejects `paused` and `frozen`.
- `AllowOnExit` — user-initiated exits (withdraw, repay, strategy legs) and the
  liquidation **repay** leg. Rejects `paused`, tolerates `frozen`.
- `SeizureLeg` — the liquidation **seizure** leg. Rejects `no_seize` only
  (`SpokeAssetSeizureHalted` = #318), and tolerates `paused` and `frozen`.

So frozen prevents new exposure but permits exits, and paused blocks entry and
every user-initiated exit including a liquidation's repay leg. Paused does not
reach the seizure leg: seizure is pro-rata over an account's whole collateral
set, so gating it on `paused` would turn a per-listing halt into a protocol-wide
liquidation halt (ADR-0008).

**Status:** ENFORCED — `contracts/controller/src/positions/mod.rs`
(`FreezePolicy`, `enforce_spoke_asset_flags`); the seizure call sites are in
`contracts/controller/src/positions/liquidation/plan.rs` and `apply.rs`.
VERIFIED — `tests/test-harness/tests/controller/spoke.rs`;
`tests/test-harness/tests/controller/spoke_liquidation_combo.rs`.

### INV-HALT-03 — Caps are literal and exit-safe

Zero cap admits nothing. Entry paths enforce usage at the live index; exits do
not consume a cap or underflow its usage.

**Status:** ENFORCED — `contracts/controller/src/spoke_usage.rs`
(`enforce_spoke_cap` scales the cap by the live index, so `cap = 0` gives
`cap_scaled = 0`; `apply_exit` never touches the cap). VERIFIED — rules
`usage_exit_without_usage_row_is_a_noop`, `usage_withdraw_tracks_scaled_delta`;
`tests/test-harness/tests/controller/spoke_caps.rs`; ADR-0015.

### INV-STOR-01 — Persistent state has lifecycle discipline

Account and market records use their intended persistence lifetime, renew when
read or written, and remove empty account state without leaving reachable
orphaned authority.

**Status:** ENFORCED — `common/src/constants/shared.rs` (TTL classes);
`contracts/controller/src/positions/mod.rs` (`renew_user_account` on write);
`contracts/controller/src/account.rs` (`remove_account_and_burn_nft`,
`cleanup_account_if_empty`). VERIFIED —
`tests/test-harness/tests/controller/position_nft.rs`;
`tests/test-harness/tests/zz_storage_sizing.rs`.

### INV-STOR-02 — NFT TTL renewal is asymmetric with account renewal

This is the parent id for four separately checkable statements, INV-STOR-02a to
INV-STOR-02d. Together they say: the NFT instance renews on the account
lifecycle, two explicit paths renew the per-token `Owner` entry to the
controller's window, passive reads renew it only to OpenZeppelin's shorter
window, and the residual gap is an operational hazard nothing in this repo
checks.

#### INV-STOR-02a — The NFT instance renews with account lifecycle

The position NFT's instance entry (controller address, collection metadata,
sequential id counter) renews to the protocol's instance TTL on every `mint` and
`burn` — that is, on every controller account create and delete.

**Status:** ENFORCED — `contracts/position-nft/src/contract.rs` (`mint` and
`burn` both call `renew_instance`). VERIFIED —
`contracts/position-nft/src/test.rs` (`burn_extends_instance_ttl`).

#### INV-STOR-02b — Ownership entries renew on two explicit paths

`renew_account` on the controller extends the account's NFT `Owner` entry to the
controller's 120-day per-user window (`TTL_BUMP_USER`), and
`position-nft::renew(token_id)` is permissionless: anyone, including a keeper or
a liquidation bot, may extend any live token's `Owner` entry. A TTL extension
moves no state, cannot reassign the token, and cannot shorten a lifetime.

**Status:** ENFORCED — `contracts/controller/src/account.rs` (`renew_account`
calls the NFT's `renew`); `contracts/position-nft/src/contract.rs` (`renew`, no
`require_auth`). Window constants: `common/src/constants/shared.rs`
(`TTL_THRESHOLD_USER` = 30 days, `TTL_BUMP_USER` = 120 days). VERIFIED —
`contracts/position-nft/src/test.rs`
(`renew_extends_owner_entry_ttl_to_user_window`, `renew_nonexistent_token_fails`).

#### INV-STOR-02c — Passive ownership reads renew on OZ's shorter window

OpenZeppelin's `owner_of` refreshes the `Owner(token_id)` entry back **to** its
own 30-day default (`OWNER_EXTEND_AMOUNT`), and only once the entry's remaining
life has fallen below OZ's 29-day threshold (`OWNER_TTL_THRESHOLD`). Touches do
not stack, and a touch on a fresh entry adds nothing. An account whose owner only
ever touches `owner_of` passively — more than ~30 days between renewals, and
within the controller's own 120-day window — can therefore let its `Owner` entry
archive while controller state is still live.

**Status:** ENFORCED — `stellar-tokens` 0.7.2,
`src/non_fungible/storage.rs` (`owner_of`) and `src/non_fungible/mod.rs`
(`OWNER_EXTEND_AMOUNT`, `OWNER_TTL_THRESHOLD`). This is dependency behaviour, not
a protocol check. VERIFICATION GAP — no repo test exercises the passive-renewal
window.

#### INV-STOR-02d — An archived `Owner` entry must be restored before use

Once the `Owner(token_id)` entry archives, any controller operation that reads or
burns the NFT traps until the entry is restored — by a `RestoreFootprint`
operation on that entry, or by calling the permissionless
`position-nft::renew(token_id)`. Bots should renew proactively on positions they
monitor, and must handle restore-then-liquidate as the fallback.

**Status:** NOT ENFORCED and VERIFICATION GAP — this is Soroban platform
behaviour, not a protocol check. `grep -rn RestoreFootprint contracts/ tests/
common/` returns no hits; no repo test exercises restore-then-liquidate. The
dependency is real: `contracts/position-nft/src/contract.rs` (`burn`) reads
`Base::owner_of` before updating, and `renew` reads it before extending. See
`docs/explanation/threat-model.md` ("Controller to position NFT") and the
`building-lending-liquidation-bots` skill.

### INV-STOR-03 — Account existence and NFT existence are paired

A controller account id and its position NFT are created and destroyed together.
Every account deletion goes through `remove_account_and_burn_nft`, so a live
account can never lack its token and a burned token can never leave a reachable
account entry.

**Status:** ENFORCED — `contracts/controller/src/account.rs`
(`remove_account_and_burn_nft`, the only remover, called by
`cleanup_account_if_empty` and by the bad-debt path). VERIFIED —
`tests/test-harness/tests/controller/position_nft.rs`
(`supply_mints_nft_with_token_id_equal_account_id`,
`emptying_account_burns_nft_and_resupply_mints_fresh_id`,
`clean_bad_debt_burns_nft`, `force_socialize_bad_debt_burns_nft`).

## Flash loans and strategies

### INV-FLASH-01 — Flash repayment is exact

Pool balances are checked around the callback. Repayment is allowance-pulled
and includes the exact required fee.

**Status:** ENFORCED — `contracts/pool/src/ops/flash.rs` requires
`allowance >= total_repayment`, pulls with `transfer_from`, and re-checks the
balance. VERIFIED — rules `flash_repayment_terms_recover_principal_and_fee`,
`flash_fee_booking_is_exact`,
`flash_apply_accounting_books_fee_without_principal_cash`;
`tests/test-harness/tests/controller/flash_loan.rs`.

### INV-FLASH-02 — Monetary reentrancy is blocked

The flash callback, router call, pool-transfer legs, and external strategy paths
share protection against entering protected monetary flows recursively.

Every window that hands control to an untrusted contract — a receiver callback,
a router, a Blend pool, or a listed token whose `transfer` may run a hook — is
wrapped in `with_flash_guard`. All six production setters:

- `strategies/flash_loan.rs:35` (`process_flash_loan`) — the flash-loan callback.
- `strategies/flash_position.rs:108` (`process_flash_position`) — the debt-token
  forward *and* the `execute_flash_position` receiver callback.
- `strategies/swap/route.rs:26` (`call_router_with_reentrancy_guard`) — the
  swap-aggregator router call.
- `strategies/legs.rs:104` (`withdraw_collateral_to_controller`) — the pool
  withdraw leg that moves collateral into the controller.
- `positions/debt.rs:272` (`borrow_into_controller`) — the pool transfer to the
  controller, held so a listed token's transfer hook cannot reenter before the
  strategy swap guard is taken.
- `external/blend.rs:91` (`guarded_submit`) — the Blend cross-contract submit.

The guard nests: `with_flash_guard` records the previous flag and only clears it
when it was not already set (`storage/account.rs:304-312`), so an inner window
inside an outer one (for example `borrow_into_controller` reached from
`process_flash_position`) cannot clear the flag early.

**Status:** ENFORCED — `contracts/controller/src/storage/account.rs`
(`with_flash_guard`), set by the six sites listed above; checked by
`contracts/controller/src/risk/validation.rs` (`require_not_flash_loaning`).
VERIFIED — rules `flash_loan_guard_blocks_callers`,
`flash_loan_guard_blocks_supply_entrypoint`,
`flash_loan_guard_blocks_liquidation_entrypoint`,
`flash_loan_guard_cleared_after_summarized_pool_return`.
Per-window tests: `tests/test-harness/tests/poc_multiply_reentrancy.rs` covers
the `multiply`/router window;
`tests/test-harness/tests/strategy/flash_position_adversarial.rs` covers the
`flash_position` callback window; `tests/test-harness/tests/meta/reentrancy_matrix.rs`
sweeps entry points against held guards.

When adding a `with_flash_guard` call site, add it to this list — the setter set
is the invariant's enforcement surface, and an unlisted window is an unreviewed
one. Check with:
`grep -rn "with_flash_guard" --include='*.rs' contracts/ | grep -v tests/`
(expect the six sites above, plus the definition in `storage/account.rs` and the
re-export in `storage/mod.rs`).

### INV-STRAT-01 — Router authority is narrowly scoped

The router cannot pull more input than approved. Its return values are not
trusted.

**Status:** ENFORCED — `contracts/controller/src/strategies/swap/auth.rs`
pre-authorizes exactly one `token_in.transfer` of `amount_in` to the router;
`contracts/controller/src/strategies/swap/route.rs` discards the router's return
value. VERIFIED — rules `swap_collateral_preserves_directional_bounds`,
`swap_debt_preserves_directional_bounds`, `multiply_sanity`; ADR-0011.

### INV-STRAT-02 — Strategy settlement is measured and solvent

Swaps must produce measured output, return residue to the rightful caller, and
finish behind the same risk gates as ordinary account operations.

**Status:** ENFORCED — `contracts/controller/src/strategies/swap/balances.rs`
(`NoSwapOutput` when nothing is received; leftover `token_in` returned to
`refund_to`); `contracts/controller/src/strategies/legs.rs` and
`contracts/controller/src/risk/validation.rs` apply the same post-action gate.
VERIFIED — rules `net_settle_keeps_revenue_backed`,
`net_settle_never_persists_supply_drained_with_debt`,
`post_gate_swap_collateral_totals_are_final`,
`post_gate_multiply_observes_gate_witness`.

### INV-STRAT-03 — External integrations run against an allowlist

Blend migration accepts only a governance-approved Blend pool; anything else
reverts `BlendPoolNotApproved`.

**Status:** ENFORCED — `contracts/controller/src/strategies/migrate_blend.rs`
(`validate_migration_request`) checks
`contracts/controller/src/storage/protocol.rs` (`is_blend_pool_approved`).
VERIFIED — `tests/test-harness/tests/strategy/migrate_blend.rs`
(`test_migrate_unapproved_blend_pool_reverts`).

### INV-STRAT-04 — Flash position cannot round-trip to a closed account

`flash_position` mints strategy debt with no flash fee and never repays that
debt in the same call. The receiver's only protocol-side settlement is a
measured collateral deposit onto the same account, followed by ordinary
solvency gates. After a successful call the account must still hold that
debt **and** at least one supply position (`FlashPositionClosed` otherwise).
It must not become a free cash flash loan.

**Status:** ENFORCED — `contracts/controller/src/strategies/flash_position.rs`
(`FlashPositionClosed`, `common/src/errors.rs` 505) and the shared
`strategy_finalize` gate in `contracts/controller/src/strategies/mod.rs`.
