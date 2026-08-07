# Runtime Invariants

Properties the deployed protocol enforces at runtime. Every entry here is backed
by an in-contract check, a test that pins the behavior, or a Certora rule — not
by intent. If an invariant below breaks, something in
`contracts/`, `common/`, `tests/`, or `certora/` has regressed.

**Enforcement tiers**

- `[code]` — an in-contract assertion or guard on the live execution path; a
  violation reverts the transaction.
- `[test]` — a unit, harness, or fuzz test that fails if the property regresses.
- `[formal*]` — a Certora Sunbeam rule in `certora/**/spec/*_rules.rs`. The
  asterisk is a standing caveat: per `certora/README.md`, controller jobs use
  **trusted cross-contract pool summaries** and a `fetch_prices` harness that
  **always returns a positive feed**, so controller verdicts are conditional on
  those summaries and on oracle success; pool accounting and controller summary
  proofs are deliberately separate. No verdict record is tracked in-repo — rule
  names indicate what is specified, not a proven-on-this-artifact claim.

Domain order matches the threat model: AUTH, ACCT, IDX, ORACLE, RISK, LIQ,
HALT, STOR, FLASH, STRAT.

---

## INV-AUTH — Authorization

### INV-AUTH-01 — Strict ownership chain: Governance → Controller → Pool
- **Statement** — Every mutating pool entrypoint is `#[only_owner]` with the controller as owner, and every controller admin entrypoint is `#[only_owner]` with governance as owner, because each parent deploys its child passing its own address as the constructor admin.
- **Enforced by** — `contracts/pool/src/lib.rs::LiquidityPoolInterface` (fifteen `#[only_owner]` mutators) [code]; `contracts/controller/src/markets/mod.rs::deploy_pool` (controller address as constructor arg) [code]; `contracts/governance/src/deploy.rs::deploy_controller` (governance address as constructor arg) [code]; `contracts/pool/tests/flows.rs::test_flash_loan_rejects_direct_non_owner_pool_call` [test].
- **On violation** — Anyone could mint debt shares, drain cash, or rewrite market params directly against the pool, bypassing every risk check.

### INV-AUTH-02 — Health-reducing verbs require owner-or-delegate
- **Statement** — `borrow` and `withdraw` require the caller to be the account owner, or a delegate that is both listed on the account and registered as an *active* position manager.
- **Enforced by** — `contracts/controller/src/positions/borrow.rs::process_borrow` and `contracts/controller/src/positions/withdraw.rs::process_withdraw` (both call `require_owner_or_delegate`) [code]; `contracts/controller/src/account/mod.rs::is_owner_or_delegate` (active-manager AND delegate-list conjunction) [code]; `certora/controller/spec/market_guard_rules.rs::supply_new_slot_requires_owner_or_delegate` [formal*].
- **On violation** — A stranger could borrow against or drain another account's collateral.

### INV-AUTH-03 — Delegate management and TTL renewal are strict-owner
- **Statement** — `add_delegate`, `remove_delegate`, and `renew_account` require the account owner itself; delegates cannot grant or extend their own authority.
- **Enforced by** — `contracts/controller/src/account/mod.rs::set_account_delegate` and `contracts/controller/src/account/mod.rs::renew_account` (both via `require_account_owner`) [code]; delegate list capped at `contracts/controller/src/constants.rs::MAX_DELEGATES` in `contracts/controller/src/storage/account.rs::add_delegate` [code].
- **On violation** — A delegate (or anyone) could escalate to permanent control of an account by installing more delegates.

### INV-AUTH-04 — Permissionless verbs carry only caller auth and only reduce risk
- **Statement** — `repay`, `liquidate`, `clean_bad_debt`, `recapitalize`, `update_indexes`, `claim_revenue`, and `update_account_threshold` require only the caller's own signature, and third-party `supply` into an existing account is accepted only for hub assets that already have an open supply position on that account.
- **Enforced by** — `contracts/controller/src/positions/repay.rs::process_repay` (no owner check; caller funds the transfer) [code]; `contracts/controller/src/positions/supply.rs::process_supply` (already-open-position gate, `GenericError::NotAuthorized`) [code]; `tests/test-harness/tests/controller/security_audit.rs::poc_permissionless_repay_any_caller` and `tests/test-harness/tests/controller/security_audit.rs::regression_third_party_cannot_open_new_supply_slots` [test].
- **On violation** — Either liveness breaks (nobody can repay/liquidate on behalf of others) or a stranger can open unwanted position slots on a victim account.

### INV-AUTH-05 — Guardian ratchet: immediate powers only tighten
- **Statement** — The GUARDIAN role can immediately `pause` the controller and set per-listing `paused`/`frozen` flags, but the immediate flags path can never clear a set flag, and there is no immediate unpause.
- **Enforced by** — `contracts/governance/src/timelock/immediate.rs::pause` and `contracts/governance/src/timelock/immediate.rs::set_spoke_asset_flags` (GUARDIAN-gated via `begin_immediate`) [code]; `contracts/controller/src/config/asset.rs::require_flag_ratchet` (`SpokeError::SpokeAssetFlagRelaxation`) [code].
- **On violation** — A compromised guardian key could silently re-open a halted market instead of only being able to stop it.

### INV-AUTH-06 — Recovery paths are timelock-only
- **Statement** — Un-halting is always delayed: global unpause exists only as the timelocked `AdminOperation::Unpause` (no `Pause` variant exists — pausing is immediate-only), and clearing per-listing flags exists only as the timelocked `EditAssetInSpoke` full rewrite.
- **Enforced by** — `contracts/governance/src/op.rs::resolve_op` (`AdminOperation::Unpause` → `controller_operation`, `DelayTier::Standard`) [code]; `interfaces/governance/src/lib.rs::AdminOperation` (variant set contains `Unpause`, no `Pause`) [code]; `contracts/controller/src/config/asset.rs::edit_asset_in_spoke` (only unratcheted flag writer, `#[only_owner]`) [code].
- **On violation** — A single hot key could flip protection off without the community reaction window the timelock exists to provide.

### INV-AUTH-07 — Timelock delay only ratchets up
- **Statement** — The governance `min_delay` is non-zero, can never decrease, and is capped at `TIMELOCK_MAX_DELAY_LEDGERS`.
- **Enforced by** — `contracts/governance/src/timelock/mod.rs::require_nonzero_delay` and `contracts/governance/src/timelock/mod.rs::validate_delay_update` (`new_delay >= current && new_delay <= max`) [code].
- **On violation** — Governance could shorten its own delay to zero and convert every timelocked power into an immediate one.

---

## INV-ACCT — Accounting identities

### INV-ACCT-01 — Revenue shares are a subset of supplied shares
- **Statement** — `revenue <= supplied` holds after every supply burn and every revenue absorption, and both quantities stay non-negative.
- **Enforced by** — `contracts/pool/src/cache/shares.rs::require_revenue_backed` (asserted in `burn_supply` and `absorb_supply_as_revenue`) [code]; `certora/pool/spec/state_invariant_rules.rs::assert_invariant` (checked by every `invariant_preserved_by_*` rule) [formal*]; `tests/test-harness/tests/fuzz/accounting_conservation.rs::assert_accounting_laws` [test].
- **On violation** — The protocol would claim treasury value backed by no depositor shares, and revenue claims could dilute suppliers.

### INV-ACCT-02 — Cash is an internally tracked counter, not a token balance
- **Statement** — Pool liquidity checks compare against the tracked `cash` field maintained with checked arithmetic; direct token donations to the pool address change no accounting state, and `cash` never goes negative.
- **Enforced by** — `contracts/pool/src/cache/cash.rs::credit_cash` / `contracts/pool/src/cache/cash.rs::debit_cash` (checked, panic on overflow/underflow) [code]; `contracts/pool/src/cache/cash.rs::require_reserves` [code]; `certora/pool/spec/state_invariant_rules.rs::assert_invariant` (`cash >= 0`) [formal*]; `certora/pool/spec/guard_rules.rs::withdraw_never_overdraws_cash` [formal*].
- **On violation** — Donation-based balance inflation could fake solvency, or a withdrawal could pay out tokens the book does not hold.

### INV-ACCT-03 — Inbound value is credited by measured receipt only
- **Statement** — Every inbound transfer on supply, repay, recapitalize, and strategy repay legs credits only the recipient's measured balance delta, never the requested amount, so fee-on-transfer tokens cannot mint uncovered claims.
- **Enforced by** — `contracts/controller/src/payments/transfer.rs::transfer_amount_measured` [code]; call sites `contracts/controller/src/positions/supply.rs::build_supply_entries`, `contracts/controller/src/positions/repay.rs::build_repay_actions`, `contracts/controller/src/strategies/legs.rs::repay_debt_from_controller` [code].
- **On violation** — A non-standard token could credit more supply or burn more debt than tokens actually arrived, draining the pool.

### INV-ACCT-04 — Backing shortfall blocks new deposits, and recapitalization can only fill it
- **Statement** — `supply` reverts while `backing_shortfall > 0` (floor-valued claims minus cash plus ceil-valued debt), and `recapitalize` credits at most the current shortfall, refunding every excess unit to the payer without minting shares.
- **Enforced by** — `contracts/pool/src/guards.rs::backing_shortfall` and `contracts/pool/src/guards.rs::require_backed_market` (asserted in `contracts/pool/src/ops/supply.rs::apply`) [code]; `contracts/pool/src/ops/recapitalize.rs::accounting` (`min(amount, shortfall)` + refund) [code]; `certora/pool/spec/state_invariant_rules.rs::invariant_preserved_by_recapitalize` [formal*].
- **On violation** — New depositors would buy into an underbacked market, or a recapitalizer could overpay into protocol limbo.

### INV-ACCT-05 — Book conservation across operation sequences
- **Statement** — Across arbitrary op sequences, per-asset reserves stay non-negative, per-user debt sums match the pool total, `reserves + borrowed - supplied` stays within a 4-raw-unit rounding tolerance, and cash round-trips exactly through supply/borrow/overpaid-repay/withdraw cycles.
- **Enforced by** — `tests/test-harness/tests/fuzz/accounting_conservation.rs::prop_accounting_conservation` (via `assert_accounting_laws`, `TOLERANCE_UNITS = 4`) [test]; `contracts/pool/tests/flows.rs::test_cash_conservation_across_supply_borrow_overpaid_repay_withdraw` [test]; `contracts/pool/tests/flows.rs::test_two_market_isolation` (conservation is per-market) [test].
- **On violation** — Value would be silently created or destroyed by ordinary sequencing, i.e. an exploitable accounting drift.

### INV-ACCT-06 — Revenue claims never outpay the shares they burn
- **Statement** — A revenue claim pays `min(cash, floor-valued revenue)`, burns shares ceil-proportionally on partial claims, and reverts if a positive payout would burn zero shares; claims also respect the utilization ceiling and the solvent-withdraw guard.
- **Enforced by** — `contracts/pool/src/cache/shares.rs::burn_claimable_revenue` [code]; `contracts/pool/src/ops/revenue.rs::accounting` (utilization + solvency gates) [code]; `certora/pool/spec/fee_strategy_accounting_rules.rs::claim_revenue_burns_equal_shares_and_cash` and `certora/pool/spec/fee_strategy_accounting_rules.rs::positive_revenue_claim_with_zero_share_burn_reverts` [formal*].
- **On violation** — Treasury withdrawals would dilute depositors or extract more cash than the burned entitlement.

### INV-ACCT-07 — No positive value movement rounds to zero shares
- **Statement** — Any positive-amount supply, borrow, withdraw, repay, or net-settle whose share conversion rounds to zero reverts instead of moving tokens against no book entry.
- **Enforced by** — `contracts/pool/src/ops/supply.rs::apply` (`SupplyRoundsToZeroShares`), `contracts/pool/src/ops/withdraw.rs::accounting` (`WithdrawRoundsToZeroShares`), `contracts/pool/src/ops/net_settle.rs::apply` (`NetSettleRoundsToZeroShares`, plus `overpayment == 0`) [code]; `certora/pool/spec/position_accounting_rules.rs::supply_scaled_balance_matches_index` and siblings (exact directed-rounding mint/burn) [formal*].
- **On violation** — Dust-sized operations could move real tokens while leaving positions unchanged — a slow-drain primitive.

---

## INV-IDX — Interest index behavior

### INV-IDX-01 — Borrow index is monotone non-decreasing and capped
- **Statement** — The borrow index only grows under accrual (`old * factor`, factor >= 1) and is clamped at `MAX_BORROW_INDEX_RAY = 1e36`; nothing ever writes it down.
- **Enforced by** — `common/src/rates/index.rs::update_borrow_index` [code]; `certora/common/spec/rate_index_accounting_rules.rs::borrow_index_strictly_grows_below_cap` and `certora/common/spec/rate_index_accounting_rules.rs::borrow_index_cap_is_sticky` [formal*]; `tests/test-harness/tests/fuzz/accounting_conservation.rs::assert_accounting_laws` (borrow-index regression check) [test].
- **On violation** — Debt could shrink without repayment, or grow unboundedly past the i128 domain.

### INV-IDX-02 — Supply index stays inside [SUPPLY_INDEX_FLOOR_RAW, MAX_SUPPLY_INDEX_RAY]
- **Statement** — On every persisted state, `SUPPLY_INDEX_FLOOR_RAW (= RAY/1_000) <= supply_index <= MAX_SUPPLY_INDEX_RAY`, and accrual itself never lowers it (`update_supply_index` returns at least the old bounded index).
- **Enforced by** — `common/src/rates/index.rs::update_supply_index` (max-with-old clamp) [code]; `contracts/pool/src/interest.rs::apply_bad_debt_to_supply_index` (floor clamp) [code]; `common/src/constants/pool.rs::SUPPLY_INDEX_FLOOR_RAW` [code]; `certora/pool/spec/state_invariant_rules.rs::assert_invariant` [formal*].
- **On violation** — Supplier claims could be wiped to zero (breaking share/asset conversions that divide by the index) or inflated past the representable domain.

### INV-IDX-03 — Supply index is NOT monotone: socialization writes it down
- **Statement** — Bad-debt seizure of a borrow position writes the supply index down proportionally (`floor(index * remaining/total_value)`), floored at `SUPPLY_INDEX_FLOOR_RAW` — so any consumer caching the supply index must tolerate a decrease.
- **Enforced by** — `contracts/pool/src/ops/seize.rs::apply` (Borrow arm) [code]; `contracts/pool/src/interest.rs::apply_bad_debt_to_supply_index` [code]; `certora/pool/spec/seize_settle_accounting_rules.rs::seize_borrow_reduces_debt_and_writes_down_supply` (proportional write-down, floor exception explicit) [formal*].
- **On violation** — Bad debt would be borne by nobody — the loss must land pro-rata on suppliers of that market, and only on them.

### INV-IDX-04 — Accrual is a no-op at zero elapsed time
- **Statement** — When no time has elapsed, `global_sync` returns without touching state, and `update_indexes` emits a snapshot without a storage commit.
- **Enforced by** — `contracts/pool/src/interest.rs::global_sync` (early return via `needs_accrual`) [code]; `contracts/pool/src/ops/market.rs::accrue` (snapshot-not-commit branch) [code]; `certora/controller/spec/index_rules.rs::indexes_unchanged_when_no_time_elapsed` and `certora/common/spec/rates_rules.rs::compound_interest_identity_at_zero_delta` [formal*].
- **On violation** — Same-ledger repeat calls could compound interest from nothing or churn storage writes.

### INV-IDX-05 — Accrued interest splits conservatively into rewards, fee, and shortfall
- **Statement** — Each accrual chunk satisfies `supplier_rewards + protocol_fee == accrued_interest`, and any rewards the floor-rounded supply-index step failed to distribute are booked as protocol revenue instead of destroyed.
- **Enforced by** — `common/src/rates/index.rs::calculate_supplier_rewards` and `common/src/rates/index.rs::supply_index_reward_shortfall` (checked subtraction reverts on over-distribution) [code]; `contracts/pool/src/interest.rs::accrue_chunk` (shortfall added to fee) [code]; `certora/common/spec/rates_rules.rs::supplier_rewards_plus_fee_equals_accrued_interest` and `certora/common/spec/rate_index_accounting_rules.rs::accrued_interest_split_is_conservative` [formal*].
- **On violation** — Interest paid by borrowers would leak — either over-credited to suppliers (insolvency) or silently vanished.

### INV-IDX-06 — Long gaps accrue in bounded chunks and time never runs backwards
- **Statement** — Accrual consumes elapsed time in chunks of at most one year (`MAX_COMPOUND_DELTA_MS`), recomputing the rate per chunk, and a backwards ledger clock yields zero elapsed time rather than a panic.
- **Enforced by** — `contracts/pool/src/interest.rs::global_sync` (chunk loop) [code]; `contracts/pool/src/cache/mod.rs::Cache::elapsed_ms` (`saturating_sub`) [code]; `certora/common/spec/rate_index_accounting_rules.rs::compound_factor_never_below_one` [formal*].
- **On violation** — A long-idle market would either freeze one stale rate across years or brick on a clock anomaly.

---

## INV-ORACLE — Price integrity

### INV-ORACLE-01 — Money paths are fail-closed on prices
- **Statement** — Every state-changing flow prices assets through `prices()`, which reverts the whole call on the first unusable key; the controller panics `OracleNotConfigured` for any un-fetched or omitted asset rather than defaulting to zero.
- **Enforced by** — `contracts/price-aggregator/src/engine.rs::force` [code]; `contracts/controller/src/external/price_aggregator.rs::fetch_prices` (missing-key panic) [code]; `contracts/controller/src/context/oracle.rs::Cache::cached_price` [code]; `certora/price-aggregator/spec/oracle_rules.rs::empty_legs_force_reverts` [formal*]; `tests/test-harness/tests/controller/security_audit.rs::poc_stale_oracle_blocks_liquidation` [test].
- **On violation** — Risk math would run on zero or stale prices, enabling free borrows or wrongful liquidations.

### INV-ORACLE-02 — A final price must clear every quality gate, in fixed precedence
- **Statement** — `force` rejects, in order: explicit resolution error, missing config, staleness, deviation, non-positive price, and sanity-band violation — a price is served only if none trip.
- **Enforced by** — `contracts/price-aggregator/src/engine.rs::Outcome::failure` [code]; `certora/price-aggregator/spec/oracle_rules.rs::single_price_respects_configured_sanity_bounds` [formal*].
- **On violation** — A price that is stale, divergent, or absurd would flow into collateral valuation.

### INV-ORACLE-03 — A half-alive dual oracle is disagreement, not fallback
- **Statement** — When exactly one leg of a two-source config is readable, the outcome is `Partial` with `price_wad = 0` and `deviation = true`, which hard-reverts on the money path — the aggregator never silently degrades to single-source.
- **Enforced by** — `contracts/price-aggregator/src/engine.rs::Outcome::partial` and `contracts/price-aggregator/src/engine.rs::blend` [code]; `certora/price-aggregator/spec/oracle_rules.rs::partial_legs_force_reverts` and `certora/price-aggregator/spec/oracle_rules.rs::partial_legs_soft_deviation` [formal*].
- **On violation** — Killing one feed would suffice to price the asset from the remaining, possibly manipulated, source.

### INV-ORACLE-04 — Dual-source blend is a tolerance-gated midpoint inside the leg interval
- **Statement** — With two readable legs, the served price is the arithmetic midpoint, accepted only if `max/min` in bps is within `upper_ratio_bps`; the blended price always lies within `[min(leg), max(leg)]`.
- **Enforced by** — `contracts/price-aggregator/src/tolerance.rs::within_tolerance_band` and `contracts/price-aggregator/src/tolerance.rs::midpoint_price_or_zero` [code]; `certora/price-aggregator/spec/oracle_rules.rs::second_band_price_within_inputs` [formal*]; `tests/test-harness/tests/controller/security_audit_extended.rs::poc_dual_in_band_midpoint_used_on_borrow_path` [test].
- **On violation** — One manipulated feed could drag the served price outside what the honest feed supports.

### INV-ORACLE-05 — Two-layer staleness with exact boundaries; future timestamps bounded
- **Statement** — A leg is stale if either its per-feed window or the asset-wide ceiling trips (`now - ts > max_stale`, boundary inclusive-fresh); in a dual blend staleness is the OR of both legs; timestamps more than 60 s in the future are rejected.
- **Enforced by** — `common/src/oracle/observation.rs::is_stale` and `common/src/oracle/observation.rs::MAX_FUTURE_SKEW_SECONDS` [code]; `contracts/price-aggregator/src/engine.rs::read_source` (component OR asset-ceiling) and `contracts/price-aggregator/src/engine.rs::blend` (leg OR) [code]; `certora/price-aggregator/spec/freshness_rules.rs::exact_staleness_boundary_is_fresh` and `certora/price-aggregator/spec/freshness_rules.rs::one_second_past_staleness_boundary_is_stale` [formal*].
- **On violation** — An attacker could serve old prices past a halted feed, or post future-dated ones that never expire.

### INV-ORACLE-06 — One consistent price snapshot per transaction
- **Statement** — The controller fetches each asset's price at most once per flow and reuses it for every calculation in that transaction, and the aggregator judges all keys in one call against a single clock reading and returns session-cached prices byte-identically.
- **Enforced by** — `contracts/controller/src/context/oracle.rs::Cache::fetch_prices` (fetch-missing-only) and `contracts/controller/src/context/oracle.rs::Cache::cached_price` [code]; `contracts/price-aggregator/src/session.rs::Session::new` (single `now_secs` snapshot) [code]; `certora/price-aggregator/spec/oracle_rules.rs::price_cache_consistency` and `certora/controller/spec/solvency_rules.rs::index_cache_single_snapshot` [formal*].
- **On violation** — Intra-transaction price movement could make debt and collateral legs of one liquidation disagree, opening arbitrage against the protocol.

### INV-ORACLE-07 — Sanity bands are validated and can only move by overlapping steps
- **Statement** — Bands require `0 < min < max <= 1e9 WAD` with a minimum half-width, and the immediate `set_sanity_band` accepts only a new band that overlaps the old one — no single jump to a disjoint range.
- **Enforced by** — `common/src/validation.rs::validate_sanity_bounds` [code]; `contracts/price-aggregator/src/admin.rs::set_sanity_band` (overlap assertion) [code]; `certora/price-aggregator/spec/oracle_rules.rs::invalid_sanity_bounds_revert` [formal*].
- **On violation** — The ORACLE role could teleport the acceptance band to legitimize an arbitrary manipulated price in one move.

---

## INV-RISK — Solvency gates

### INV-RISK-01 — Health-reducing verbs re-prove solvency after the pool call
- **Statement** — `borrow`, `withdraw`, and every strategy finalize by re-running the risk gates on post-call state: `ltv_collateral >= total_debt`, `health_factor >= 1 WAD`, and the min-collateral floor; debt-free accounts skip the gates.
- **Enforced by** — `contracts/controller/src/risk/validation.rs::require_post_pool_risk_gates` [code]; call sites `contracts/controller/src/positions/borrow.rs::process_borrow`, `contracts/controller/src/positions/withdraw.rs::process_withdraw`, `contracts/controller/src/strategies/mod.rs::strategy_finalize` [code]; `certora/controller/spec/solvency_rules.rs::ltv_borrow_bound_enforced`, `certora/controller/spec/health_rules.rs::borrow_safe_or_health_gated`, `certora/controller/spec/health_rules.rs::withdraw_safe_or_health_gated` [formal*].
- **On violation** — An account could exit a borrow or withdraw already underwater, converting user error or oracle noise directly into bad debt.

### INV-RISK-02 — LTV is strictly below the liquidation threshold, and post-bonus seizure fits in 100%
- **Statement** — Every listing satisfies `liquidation_threshold > loan_to_value`, `threshold <= 100%`, and `threshold * (1 + bonus) <= 100%`, so a position at the borrow limit is not yet liquidatable and a threshold-priced liquidation cannot seize more than the collateral.
- **Enforced by** — `common/src/validation.rs::validate_risk_bounds` [code] (applied on listing add/edit).
- **On violation** — Fresh borrows would be instantly liquidatable, or liquidation would be guaranteed to strand unseizable debt.

### INV-RISK-03 — Health factor definition rounds against the borrower
- **Statement** — `health_factor = weighted_collateral / total_debt` where collateral gate values use floor valuation, debt uses ceil valuation at every step, the division floors (saturating), and debt-free accounts read `i128::MAX`.
- **Enforced by** — `contracts/controller/src/risk/totals.rs::calculate_account_risk_totals_body` [code]; `certora/controller/spec/hf_lemma_rules.rs` (valuation monotonicity and rounding direction lemmas) [formal*].
- **On violation** — Rounding would systematically favor borrowers, letting positions hover exploitable hairs above the liquidation line.

### INV-RISK-04 — Risk params restamp live, but tightening is HF-gated
- **Statement** — LTV is always restamped from live listing config before gates run; the liquidation tuple (threshold/bonus/fees) of an indebted account is updated toward the liquidator's favor only if the hypothetical HF at the new threshold still clears 1.05 WAD, and the keeper path asserts the same floor.
- **Enforced by** — `contracts/controller/src/risk/params.rs::restamp_listed_supply_ltv` [code]; `contracts/controller/src/risk/params.rs::apply_gated_liquidation_params` with `contracts/controller/src/risk/params.rs::favors_liquidator` [code]; `contracts/controller/src/keepers/mod.rs::sync_account_thresholds` (`HealthFactorTooLow` below `THRESHOLD_UPDATE_MIN_HF_RAW`) [code].
- **On violation** — A governance parameter tightening would retroactively shove healthy accounts underwater in one block.

### INV-RISK-05 — Min-borrow collateral floor
- **Statement** — An indebted account must hold at least `min_borrow_collateral_usd` (default 5 WAD USD) of LTV-weighted collateral; a floor of 0 disables the check.
- **Enforced by** — `contracts/controller/src/risk/validation.rs::require_post_pool_risk_gates` (`MinBorrowCollateralNotMet`) [code]; `contracts/controller/src/storage/protocol.rs::get_min_borrow_collateral_usd_wad` (defaults to `common/src/constants/shared.rs::DEFAULT_MIN_BORROW_COLLATERAL_USD_WAD`) [code].
- **On violation** — Dust positions too small to be worth liquidating could accumulate as unliquidatable griefing debt.

### INV-RISK-06 — Position-count limits bound account size
- **Statement** — An account can hold at most the configured number of supply and borrow positions (each limit in `1..=10`), counted over distinct hub assets including the new batch.
- **Enforced by** — `contracts/controller/src/risk/validation.rs::validate_bulk_position_limits` [code]; `contracts/controller/src/config/limits.rs::set_position_limits` (bounds vs `POSITION_LIMIT_MAX`) [code]; `certora/controller/spec/solvency_rules.rs::supply_position_limit_enforced` and `certora/controller/spec/solvency_rules.rs::borrow_position_limit_enforced` [formal*].
- **On violation** — Liquidating a many-position account could exceed the execution budget, making it economically or technically unliquidatable.

---

## INV-LIQ — Liquidation

### INV-LIQ-01 — Eligibility requires HF < 1 with live debt
- **Statement** — Liquidation reverts `HealthFactorTooHigh` unless the account has at least one debt position and its health factor is strictly below 1 WAD.
- **Enforced by** — `contracts/controller/src/positions/liquidation/plan.rs::build_liquidation_plan` [code].
- **On violation** — Healthy accounts could be seized — the protocol's core custody promise fails.

### INV-LIQ-02 — No self-liquidation
- **Statement** — The account owner cannot liquidate their own account.
- **Enforced by** — `contracts/controller/src/positions/liquidation/mod.rs::validate_liquidation_inputs` (`SelfLiquidationNotAllowed`) [code]; `certora/controller/spec/liquidation_rules.rs::self_liquidation_reverts` [formal*]; `tests/test-harness/tests/controller/security_audit_extended.rs::refutation_owner_cannot_self_liquidate` [test].
- **On violation** — Owners would harvest their own liquidation bonus, converting the penalty into a self-payment that socializes the discount onto the pool.

### INV-LIQ-03 — Close amount is capped at the curve's ideal; excess is refunded, not transferred
- **Statement** — Repayment is normalized to `min(paid, ideal)` where ideal targets the spoke's `target_hf`; per-leg payments are capped at that asset's ceil-valued debt; everything above the cap is queued as a refund and never leaves the liquidator, and an under-payment that cannot preserve HF reverts `FullCloseRequired`.
- **Enforced by** — `contracts/controller/src/positions/liquidation/math.rs::normalize_repayment_plan`, `contracts/controller/src/positions/liquidation/math.rs::calculate_repayment_amounts`, `contracts/controller/src/positions/liquidation/math.rs::process_excess_payment` [code]; `certora/controller/spec/liquidation_rules.rs::liquidation_does_not_increase_repaid_debt` and `certora/controller/spec/liquidation_rules.rs::ideal_repayment_targets_curve_hf` [formal*].
- **On violation** — Liquidators could over-close healthy remainders or the protocol could pocket unowed payment.

### INV-LIQ-04 — Bonus is bounded and HF-scaled
- **Statement** — The applied bonus starts at the collateral-weighted base, scales linearly as HF falls below the spoke target, and never exceeds `max_bonus_for_threshold` — the largest bonus for which post-bonus seizure at the effective threshold stays within 100% of collateral.
- **Enforced by** — `contracts/controller/src/positions/liquidation/curve.rs::calculate_linear_bonus_with_target`, `contracts/controller/src/positions/liquidation/curve.rs::max_bonus_for_threshold`, `contracts/controller/src/positions/liquidation/curve.rs::estimate_liquidation_amount` [code]; `certora/controller/spec/liquidation_rules.rs::bonus_bounded`, `certora/controller/spec/liquidation_rules.rs::bonus_monotone_in_hf`, `certora/controller/spec/liquidation_rules.rs::bonus_is_base_at_or_above_target` [formal*].
- **On violation** — Liquidators would be over- or under-incentivized, either looting borrowers or leaving underwater accounts untouched.

### INV-LIQ-05 — Protocol fee is taken on the bonus portion only
- **Statement** — Each seizure splits into `base = seizure / (1 + bonus)` (floor) and `bonus = seizure - base`; the protocol fee is `liquidation_fees` bps of the bonus only, floored with a 1-unit bump when positive-but-zero, capped at the pool gross, and withheld pool-side into revenue rather than paid out.
- **Enforced by** — `contracts/controller/src/positions/liquidation/math.rs::calculate_seized_collateral` [code]; `contracts/pool/src/ops/withdraw.rs::withhold_liquidation_fee` (`WithdrawLessThanFee` guard, fee → revenue) [code]; `certora/controller/spec/liquidation_rules.rs::protocol_fee_bonus_math` and `certora/pool/spec/fee_strategy_accounting_rules.rs::liquidation_withdraw_books_protocol_fee` [formal*].
- **On violation** — The fee would eat into repayment principal, so liquidators would receive less collateral than the debt they extinguished.

### INV-LIQ-06 — Seizure never exceeds the position and shrinks to what was actually received
- **Statement** — Each seizure leg is capped at the position's full current value, and if debt tokens under-delivered (fee-on-transfer), all seizures scale down by `received/planned` with floor rounding, so residue stays with the liquidated account.
- **Enforced by** — `contracts/controller/src/positions/liquidation/math.rs::calculate_seized_collateral` (`capped_ray = min(seizure, actual)`) [code]; `contracts/controller/src/positions/liquidation/math.rs::scale_seizures_to_received` (applied in `contracts/controller/src/positions/liquidation/mod.rs::process_liquidation`) [code]; `certora/controller/spec/liquidation_rules.rs::liquidation_does_not_increase_seized_collateral` [formal*].
- **On violation** — A liquidator paying with a fee-on-transfer token would collect collateral for value that never arrived.

### INV-LIQ-07 — Sub-threshold dust promotes to full close
- **Statement** — If the ideal partial close would strand residual debt strictly between 0 and 5 WAD USD, the plan is promoted to a full close at the same bonus.
- **Enforced by** — `contracts/controller/src/positions/liquidation/curve.rs::estimate_liquidation_amount` (dust rule vs `BAD_DEBT_USD_THRESHOLD`) [code]; `certora/controller/spec/liquidation_rules.rs::estimate_leaves_no_sub_threshold_dust` [formal*].
- **On violation** — Liquidations would systematically manufacture sub-cleanup-threshold debt crumbs nobody can profitably remove.

### INV-LIQ-08 — Bad-debt socialization is gated, rechecked, and total
- **Statement** — Permissionless `clean_bad_debt` requires `total_debt > total_collateral` AND `total_collateral <= 5 WAD` (owner-only `force_socialize_bad_debt` drops only the dust cap); every liquidation re-runs the dust-capped gate on post-liquidation totals; admitted cleanup seizes every position, drains spoke usage, and deletes the account entry, so bad debt is assigned exactly once.
- **Enforced by** — `contracts/controller/src/positions/liquidation/curve.rs::is_socializable_bad_debt` and `contracts/controller/src/positions/liquidation/mod.rs::BadDebtGate` [code]; `contracts/controller/src/positions/liquidation/apply.rs::check_bad_debt_after_liquidation` [code]; `contracts/controller/src/positions/liquidation/bad_debt.rs::execute_bad_debt_cleanup` [code].
- **On violation** — Live accounts could be socialized away, or the same insolvency could be written off twice against suppliers.

---

## INV-HALT — Pause, freeze, and caps

### INV-HALT-01 — Global pause blocks risk-increasing verbs; exits and liquidations stay open
- **Statement** — `#[when_not_paused]` gates exactly `supply`, `borrow`, `flash_loan`, the five strategies, `migrate_from_blend`, `update_indexes`, `claim_revenue`, `update_account_threshold`, and `add_delegate`; `withdraw`, `repay`, `liquidate`, `clean_bad_debt`, `recapitalize`, `renew_account`, `remove_delegate`, and all views stay callable while paused.
- **Enforced by** — `contracts/controller/src/lib.rs::Controller` (`ControllerInterface` impl, attribute placement) [code]; `tests/test-harness/tests/controller/security_audit_extended.rs::poc_global_pause_blocks_risk_increasing_allows_exit_and_liq` [test].
- **On violation** — Either pause becomes a fund trap (exits blocked) or it stops protecting (new risk admitted during an incident).

### INV-HALT-02 — Fresh deployments and upgrades land paused
- **Statement** — `init` pauses the controller at construction, and `upgrade` self-pauses before the WASM swap if not already paused, so going live always requires a timelocked `Unpause`.
- **Enforced by** — `contracts/controller/src/governance/access.rs::init` and `contracts/controller/src/governance/access.rs::upgrade` [code].
- **On violation** — Unconfigured or freshly-swapped code would immediately face user traffic.

### INV-HALT-03 — Spoke `paused` blocks every verb on that listing, including exits ("tainted debt")
- **Statement** — A `paused` listing reverts supply, borrow, withdraw, AND repay for that asset; because liquidation's repay leg routes through the same check, an account whose debt listing is paused is also unliquidatable until the flag clears.
- **Enforced by** — `contracts/controller/src/positions/mod.rs::enforce_spoke_asset_flags` (paused asserted under both freeze policies) [code]; `contracts/controller/src/positions/repay.rs::build_repay_actions` (`AllowOnExit` still checks paused) [code]; `contracts/controller/tests/positions/flags.rs::paused_blocks_withdraw_repay` and `tests/test-harness/tests/controller/security_audit.rs::poc_paused_debt_blocks_liquidation_repay` [test].
- **On violation** — The strongest halt would stop being total — or, unexpectedly, exits through a "fully halted" market would move funds.

### INV-HALT-04 — Spoke `frozen` blocks only entry; exits remain
- **Statement** — A `frozen` listing rejects new supply and borrow (`BlockOnEntry`) while withdraw, repay, net-settle, and both liquidation legs proceed (`AllowOnExit`), so governance can wind a listing down without trapping funds.
- **Enforced by** — `contracts/controller/src/positions/mod.rs::FreezePolicy` and `contracts/controller/src/positions/mod.rs::enforce_spoke_asset_flags` [code]; `contracts/controller/tests/positions/flags.rs::frozen_allows_withdraw_repay` and `tests/test-harness/tests/controller/security_audit_extended.rs::poc_spoke_pause_blocks_withdraw_freeze_allows` [test].
- **On violation** — Freeze would either trap deposits or fail to stop new exposure.

### INV-HALT-05 — Delisted assets stay exitable
- **Statement** — A hub asset no longer listed on the spoke makes the flag check a no-op, so existing positions in delisted assets can always be withdrawn and repaid; delisting itself requires both usage counters at zero.
- **Enforced by** — `contracts/controller/src/positions/mod.rs::enforce_spoke_asset_flags` (missing listing → no-op) [code]; `contracts/controller/src/config/asset.rs::remove_asset_from_spoke` (`SpokeAssetInUse`) [code]; `contracts/controller/tests/positions/flags.rs::missing_spoke_asset_is_noop` [test].
- **On violation** — Governance delisting would orphan user funds behind a check that can never pass again.

### INV-HALT-06 — Caps are zero-means-zero, converted at the live index
- **Statement** — Supply and borrow caps are asset-unit ceilings converted to scaled shares at the current index per call; there is no unlimited sentinel — a cap of 0 rejects every positive entry on that side.
- **Enforced by** — `contracts/controller/src/spoke/caps.rs::cap_to_scaled` and `contracts/controller/src/spoke/caps.rs::enforce_spoke_cap` [code]; `contracts/controller/tests/spoke.rs::zero_supply_cap_rejects_entry` and `contracts/controller/tests/spoke.rs::zero_borrow_cap_rejects_entry` [test].
- **On violation** — An operator setting 0 expecting "closed" would instead have opened the side unlimited — the exact inversion the sentinel-free design prevents.

### INV-HALT-07 — Only entries enforce caps; exits are uncapped and never underflow
- **Statement** — Cap checks run only on the entry direction; exits subtract usage without any cap comparison, no-op when no usage row exists, and panic `InternalError` if usage would go negative.
- **Enforced by** — `contracts/controller/src/positions/mod.rs::apply_leg_usage` (direction split) [code]; `contracts/controller/src/spoke/caps.rs::SpokeUsageContext::apply_exit` (`next >= 0` assert) [code].
- **On violation** — Interest accrual pushing usage past a cap would block withdrawals, or usage corruption would silently free cap headroom.

### INV-HALT-08 — Halt flags ratchet on the immediate path
- **Statement** — `set_spoke_asset_flags` may flip `paused`/`frozen` false→true or leave them set, never true→false; the only clearing path is the timelocked full-rewrite `edit_asset_in_spoke` (see INV-AUTH-05/06 for the role split).
- **Enforced by** — `contracts/controller/src/config/asset.rs::require_flag_ratchet` [code]; `contracts/controller/src/config/asset.rs::set_spoke_asset_flags` [code].
- **On violation** — The incident-response path would double as an incident-cause path.

---

## INV-STOR — Storage and TTL

### INV-STOR-01 — Storage class discipline
- **Statement** — Controller state lives in its designated class — instance for protocol registry/config, shared-persistent for hubs/spokes/listings/usage/managers, user-persistent for the four per-account keys — and the flash-loan session flag is the controller's only temporary entry.
- **Enforced by** — `contracts/controller/src/storage/ttl.rs::get_shared` / `contracts/controller/src/storage/ttl.rs::get_user` (class-specific accessors) [code]; `contracts/controller/src/storage/session.rs::SessionKey` (private temporary enum) [code]; `common/src/types/pool.rs::PoolKey` (pool: two persistent variants only) [code].
- **On violation** — Critical state in the wrong class would expire on the wrong schedule (e.g. an auth flag surviving, or an account record dying young).

### INV-STOR-02 — TTL renews on every access
- **Statement** — Controller persistent reads and writes both extend the key's TTL; pool market keys renew on every cache load (reads included) and every mutating entrypoint renews the instance.
- **Enforced by** — `contracts/controller/src/storage/ttl.rs::get_persistent` / `contracts/controller/src/storage/ttl.rs::set_persistent` [code]; `contracts/pool/src/storage.rs::renew_market` and `contracts/pool/src/storage.rs::load_sync_data` [code]; `contracts/pool/tests/flows.rs::test_pool_mutation_renews_instance_and_market_ttls` [test].
- **On violation** — Actively-used markets or accounts could silently archive, bricking accounting mid-flight.

### INV-STOR-03 — Account keys are co-renewed after position writes
- **Statement** — Position-map writers deliberately skip TTL renewal; every position flow ends with `renew_user_account`, which extends all live account keys (meta, both sides, delegates) together, and owners can force the same via `renew_account`.
- **Enforced by** — `contracts/controller/src/positions/mod.rs::persist_account_positions` (calls `renew_user_account`) [code]; `contracts/controller/src/storage/account.rs::renew_user_account` (`has`-guarded co-renewal) [code]; `contracts/controller/src/account/mod.rs::renew_account` [code].
- **On violation** — An account's sides could expire on different ledgers, leaving debt alive with its collateral archived.

### INV-STOR-04 — Empty state prunes its key
- **Statement** — Empty position maps, empty delegate lists, zero-both-sides spoke usage rows, deactivated position managers, and revoked Blend pools remove their storage keys instead of writing empty values.
- **Enforced by** — `contracts/controller/src/storage/account.rs::write_side_map`, `contracts/controller/src/storage/account.rs::set_delegates`, `contracts/controller/src/storage/spoke.rs::set_spoke_usage`, `contracts/controller/src/storage/protocol.rs::set_position_manager`, `contracts/controller/src/storage/protocol.rs::set_blend_pool_approved` [code].
- **On violation** — Zombie keys would accrue rent and make "revoked" states indistinguishable from "empty but present" ones.

### INV-STOR-05 — Account deletion is total and reachable only through empty or socialized states
- **Statement** — `remove_account_entry` deletes all four account keys atomically, and it is invoked only when both position maps are empty (full-exit withdraw, post-liquidation cleanup) or by bad-debt socialization after every position was seized.
- **Enforced by** — `contracts/controller/src/storage/account.rs::remove_account_entry` [code]; `contracts/controller/src/account/mod.rs::cleanup_account_if_empty` [code]; `contracts/controller/src/positions/liquidation/bad_debt.rs::execute_bad_debt_cleanup` [code].
- **On violation** — Deleting a live account would erase debt without settlement; partial deletion would strand orphan keys claiming positions for a dead account.

### INV-STOR-06 — Market keys are never deleted
- **Statement** — The pool exposes no path that removes `PoolKey::Params` or `PoolKey::State`; once created, a market's storage exists forever, and a listing can only leave a spoke when both usage counters are zero.
- **Enforced by** — `contracts/pool/src/storage.rs` (write/renew helpers only — no `remove` on market keys) [code]; `contracts/controller/src/config/asset.rs::remove_asset_from_spoke` (`SpokeAssetInUse` gate) [code].
- **On violation** — Deleting market state under live scaled positions would sever every holder's claim at once.

---

## INV-FLASH — Flash loans

### INV-FLASH-01 — Exact balance assertions bracket the callback
- **Statement** — The pool asserts its token balance equals `pre - amount` immediately after payout AND again after the receiver callback returns, and equals `pre + fee` after repayment collection — a receiver pushing tokens back directly mid-callback fails the reconciliation.
- **Enforced by** — `contracts/pool/src/ops/flash.rs::apply` with `contracts/pool/src/ops/flash.rs::require_balance` [code]; `contracts/pool/src/ops/flash.rs::terms` [code]; `certora/pool/spec/flash_loan_accounting_rules.rs::flash_repayment_terms_recover_principal_and_fee` [formal*]; `contracts/pool/tests/flows.rs::test_flash_loan` [test].
- **On violation** — A receiver could satisfy repayment by illusion — double-counting a push as both repayment and retained balance.

### INV-FLASH-02 — Repayment is allowance-pull only
- **Statement** — The pool collects `amount + fee` exclusively via `transfer_from` against the receiver's allowance (checked `>= total` first); there is no push-based repayment path.
- **Enforced by** — `contracts/pool/src/ops/flash.rs::collect_repayment` (`InvalidFlashloanRepay`) [code].
- **On violation** — Ambiguous repayment accounting — the balance assertions of INV-FLASH-01 stop being sufficient to prove the pool was made whole.

### INV-FLASH-03 — Fee is exact, floored at one unit, and booked as revenue plus cash
- **Statement** — The fee is `flashloan_fee` bps of the amount, bumped to 1 unit when the rate is non-zero but rounds to zero, and after repayment it is minted as protocol revenue shares and credited to cash — the principal never touches tracked cash.
- **Enforced by** — `common/src/math/fp.rs::Bps::flash_loan_fee_on` [code]; `contracts/pool/src/ops/flash.rs::book_fee` [code]; `certora/pool/spec/flash_loan_accounting_rules.rs::flash_fee_booking_is_exact` [formal*].
- **On violation** — Free flash loans at small sizes, or fee value bypassing the revenue book.

### INV-FLASH-04 — One reentrancy guard covers flash loans, the router, and Blend
- **Statement** — A temporary-storage flag is raised around the pool flash-loan call, every swap-router call, and every Blend `submit`; while set, all position verbs, liquidation, bad-debt cleanup, and keeper entrypoints revert `FlashLoanOngoing`.
- **Enforced by** — `contracts/controller/src/storage/session.rs::with_flash_guard` [code]; `contracts/controller/src/risk/validation.rs::require_not_flash_loaning` via `contracts/controller/src/positions/mod.rs::require_position_caller`, `contracts/controller/src/positions/liquidation/mod.rs::process_liquidation`, `contracts/controller/src/positions/liquidation/mod.rs::process_clean_bad_debt` [code]; `contracts/controller/src/strategies/swap/route.rs::call_router_with_reentrancy_guard`, `contracts/controller/src/external/blend.rs::guarded_submit` [code]; `tests/test-harness/tests/meta/reentrancy_matrix.rs::test_all_state_changing_entries_reject_under_flash_loan_ongoing` [test]; `certora/controller/spec/flash_loan_rules.rs::flash_loan_guard_blocks_supply_entrypoint` [formal*].
- **On violation** — A callback could reenter position verbs against the uncommitted state of INV-FLASH-06 and trade on inconsistent books.

### INV-FLASH-05 — Guard exceptions are exactly the non-monetary account verbs
- **Statement** — `renew_account`, `add_delegate`, and `remove_delegate` are the only state-changing user entrypoints that skip the flash-loan guard — none of them can move value or change positions.
- **Enforced by** — `contracts/controller/src/account/mod.rs::renew_account` and `contracts/controller/src/account/mod.rs::set_account_delegate` (no `require_not_flash_loaning` on their paths) [code]; `certora/controller/spec/flash_loan_rules.rs::flash_loan_guard_allows_when_clear` (complement direction) [formal*].
- **On violation** — Widening the exception set to any monetary verb reopens the reentrancy surface the guard exists to close.

### INV-FLASH-06 — Pool state is uncommitted during the callback
- **Statement** — The pool cache commits only after repayment succeeds, so a failed or dishonest callback rolls back to the pre-call persisted state, and the fee is the only state delta a successful flash loan leaves.
- **Enforced by** — `contracts/pool/src/ops/flash.rs::apply` (commit ordered after `collect_repayment`) [code]; `certora/pool/spec/state_invariant_rules.rs::invariant_preserved_by_flash_fee_booking` [formal*].
- **On violation** — A reverted flash loan would leave partially-updated market state — the classic flash-attack foothold.

### INV-FLASH-07 — Receivers must be Wasm contracts on an active hub
- **Statement** — Both controller and pool reject non-Wasm receivers, the controller additionally requires a positive amount, an active hub, and no nested flash loan, and the pool requires the market flag `is_flashloanable`.
- **Enforced by** — `contracts/controller/src/strategies/flash_loan.rs::process_flash_loan` [code]; `contracts/pool/src/ops/flash.rs::apply` (`FlashloanNotEnabled`, `require_wasm_receiver`) [code].
- **On violation** — Loans to wallets or dead addresses could never repay, converting flash liquidity into theft.

---

## INV-STRAT — Strategy and router distrust

### INV-STRAT-01 — The router can neither overspend nor over-pull
- **Statement** — After every router call the controller asserts its `token_in` balance did not increase and that the measured spend does not exceed the authorized `amount_in`; any unspent remainder is refunded to the designated refund address.
- **Enforced by** — `contracts/controller/src/strategies/swap/balances.rs::settle_router_input` (`RouterOverspend`) [code]; `tests/test-harness/tests/fuzz/strategy_router_invariants.rs::prop_swap_collateral_conserves_position_delta` (zero residual allowance) [test].
- **On violation** — A malicious router could drain controller-held balances beyond the single authorized input.

### INV-STRAT-02 — Swaps must produce measured output
- **Statement** — The swap result is the measured `token_out` balance delta — the router's return value is discarded — and a non-positive delta reverts `NoSwapOutput`.
- **Enforced by** — `contracts/controller/src/strategies/swap/balances.rs::verify_router_output` [code]; `contracts/controller/src/strategies/swap/route.rs::call_router_with_reentrancy_guard` (return value ignored) [code].
- **On violation** — A router reporting fictional output would let strategies book collateral that never arrived.

### INV-STRAT-03 — External pulls are pre-authorized with exact scope only
- **Statement** — The controller grants invoker auth in exactly two places, each scoped to specific `transfer` calls with exact arguments — one `transfer(controller, router, amount_in)` before a swap, and one `transfer(controller, blend_pool, max)` per debt asset before Blend repay — never a blanket approval.
- **Enforced by** — `contracts/controller/src/strategies/swap/auth.rs::pre_authorize_router_pull` [code]; `contracts/controller/src/external/blend.rs::authorize_repay_pulls` [code].
- **On violation** — An external contract could pull arbitrary controller balances under a standing approval.

### INV-STRAT-04 — Strategy refunds return residue to the caller
- **Statement** — Unspent swap input and post-repay controller balance deltas are transferred back to the caller (or designated refund address); the controller retains no strategy residue.
- **Enforced by** — `contracts/controller/src/strategies/swap/balances.rs::settle_router_input` (leftover transfer) [code]; `contracts/controller/src/strategies/legs.rs::refund_controller_balance_delta` [code].
- **On violation** — User value would silently accumulate on the controller, unaccounted by any book.

### INV-STRAT-05 — Every strategy ends behind the full solvency gate
- **Statement** — All strategies finalize through `strategy_finalize`: live LTV restamped, post-pool risk gates re-run (LTV bound, HF >= 1, min floor), both position sides persisted, and empty accounts removed — no strategy path exits without re-proving account health.
- **Enforced by** — `contracts/controller/src/strategies/mod.rs::strategy_finalize` [code]; `tests/test-harness/tests/fuzz/strategy_router_invariants.rs::prop_multiply_succeeds_with_safe_hf_and_clean_router` (post-multiply HF >= 1 WAD, guard cleared) [test].
- **On violation** — A multi-leg strategy could assemble an underwater position no single verb would have permitted.

### INV-STRAT-06 — Strategy entry legs re-use the verb gates
- **Statement** — Strategy deposits preflight `require_can_supply` (hub active, listed, pause/freeze, collateralizable) before any external leg, and strategy borrows run the full borrow entry gates including caps and position limits — strategies bypass no per-verb protection.
- **Enforced by** — `contracts/controller/src/positions/mod.rs::require_can_supply` (called from strategy preflights) [code]; `contracts/controller/src/positions/mod.rs::validate_position_entry_gates` [code]; `certora/controller/spec/strategy_rules.rs` (same-token rejections, multiply requires collateralizable target) [formal*].
- **On violation** — Strategies would be a side door around freezes, caps, and listing gates.
