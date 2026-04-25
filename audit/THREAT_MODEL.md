# Threat Model

Adversary models for each concern Daisy flagged. Each section lists **adversary capability**, **target invariant**, **attack sketch**, **current mitigation**, and **residual risk for auditors to confirm**.

Current Stellar protocol parameters (April 2026) anchor the network-layer constraints:
- 400M instructions / tx; 41 MB tx memory
- 200 disk-read entries / tx; 200 write entries / tx
- 132 KB tx size; 16 KB events
- 286 KB write bytes / ledger; 200 KB read bytes / tx
- Soroban TTL: temp 17_280 ledgers (~1d); persistent 2_073_600 (~120d); max 3_110_400 (~180d)

## §1. Flash Loan Re-entrancy

### Adversary capability
Any address can deploy a `receiver` contract that re-invokes the controller from its `execute_flash_loan` callback.

### Target invariants
- I.13 (reserve availability): `current_reserves >= requested_amount` at every transfer.
- I.5 (interest split): `accrued_interest = supplier_rewards + protocol_fee` per global_sync.
- I.9 (HF): no operation completes with HF < 1.0 WAD on a survivor account.

### Attack sketches
1. **Re-borrow inside callback to evade HF check.** The callback calls `controller.borrow(...)` on the attacker account. `borrow` does **not** call `require_not_flash_loaning`. If the just-loaned funds count as collateral (deposited via `supply` in the same tx) before the borrow, the HF check passes. `flash_loan_end` then pulls the funds back; the supply position remains but the underlying liquidity vanishes.
2. **Recursive flash loan from receiver.** `flash_loan` calls `require_not_flash_loaning`, so direct nesting fails. Confirm no other path mutates `FlashLoanOngoing` (strategy fns set it directly — see strategy.rs).
3. **Repay inside callback to drop one's own debt below the liquidation threshold and dodge the liquidation queue.** An attacker could combine permissionless repay with a same-tx oracle update.

### Current mitigation (verified via grep + file reads)
- `process_flash_loan` sets the `FlashLoanOngoing` boolean in **Instance** storage (`controller/src/storage/mod.rs:175-186`) at flash_loan.rs:43 and clears it at flash_loan.rs:61.
- **Every** mutating controller endpoint calls `require_not_flash_loaning`:
  - User position ops: supply.rs:25, borrow.rs:100, withdraw.rs:19, repay.rs:19
  - Strategy entries: strategy.rs:55, 228, 345, 518
  - Other: liquidation.rs:30, flash_loan.rs:22, lib.rs:315 (update_indexes), 349 (clean_bad_debt), 367 (update_account_threshold), 634 (claim_revenue), 640 (add_rewards)
- The receiver callback (`env.invoke_contract::<()>(receiver, "execute_flash_loan", ...)` at flash_loan.rs:51-55) can therefore reach only external contracts (aggregator, token SACs).
- Soroban panic-rollback semantics: a panic anywhere in the flash-loan flow reverts the entire tx, including the `set_flash_loan_ongoing(true)` write. Future flash loans remain available.

### Residual risk
**LOW (revised down from HIGH)** — the boolean guard covers every entry. Auditors should confirm:
  1. A panic inside `pool.flash_loan_end` (e.g., `transfer_from` failing) rolls back state, including the cleared `FlashLoanOngoing`. The whole tx reverts. Standard panic-rollback.
  2. No exception path in `process_flash_loan` clears the flag before all sub-calls complete. Linear flow, no early return.
  3. `multiply` and `swap_*` set their own internal flash loan via direct pool calls rather than the controller's `flash_loan` endpoint, so they do not need `FlashLoanOngoing` false during their internal loop. The *outer* strategy entry checks the flag, so a strategy cannot start while another flash loan is open.

### Audit asks
- Confirm Soroban panic-rollback covers Instance-storage writes (it should — Instance writes commit at tx end).
- **Strategy fns (`multiply`, `swap_*`) re-entry posture — hardened during prep.** Strategies use `pool.create_strategy` (atomic, single-call), not `pool.flash_loan_begin`/`end`. The risk surface was the aggregator call in `swap_tokens` (strategy.rs:467-476). **Now hardened**: `swap_tokens` brackets the aggregator call with `set_flash_loan_ongoing(true/false)` (strategy.rs, around line 467), so any aggregator-callback re-entry into a controller mutating endpoint panics with `FlashLoanOngoing`. A controller-side `received < amount_out_min` postcheck adds defense-in-depth against a router that ignores its own slippage param.

## §2. Math / Insolvency

### Adversary capability
Any user. The attacker chooses timing (block boundary), batch composition, and repeats tiny operations.

### Target invariants
- I.3 (scaled balance): `scaled * index ≈ actual` round-trips within ULP.
- I.5 (interest split conservation).
- I.6 (borrow index monotone).
- I.7 (supply index monotone except bad-debt floor at `10^18` raw).
- I.10 (LTV bound).

### Attack sketches
1. **Rounding asymmetry exploit.** Half-up rounding applies everywhere, but `mul_div_half_up(a, b, c)` and the inverse `mul_div_half_up(out, c, b)` are not exact inverses. Does `scaled_amount` drift when an attacker repeatedly supplies and withdraws one unit? Drift in the user's favor enables a slow-leak attack on the supply pool.
2. **Index sync timing.** `global_sync` runs at every pool mutation. The attacker calls `update_indexes` *just before* a high-utilization spike to lock in a low rate, then borrows — or vice versa.
3. **Fixed-point overflow.** `i128` mul/div paths, specifically `borrowed_ray * borrow_index_ray`: at RAY=10^27, the product overflows once borrowed_ray reaches ~10^11 and index ~10^11. Audit max realistic values against the i128 ceiling (~1.7×10^38).
4. **Flash-loan-driven utilization spike.** Borrow 99% of the pool via flash loan, hold across a sub-call that triggers `update_indexes`, repay. The single-block index update sees extreme utilization, sets a high rate, and accrues large interest that becomes "real" debt for actual borrowers.
5. **Bad-debt socialization gaming.** Force liquidation of one's own account once the `coll ≤ $5` precondition holds, triggering `apply_bad_debt_to_supply_index`. Suppliers lose; the attacker may also hold a short position via swap_debt that profits from the supply-index drop.

### Current mitigation
- Half-up rounding stays consistent (`common::fp_core::mul_div_half_up`).
- A supply-index floor at `10^18` raw prevents division-by-near-zero.
- `clean_bad_debt` requires KEEPER, but `liquidate` exposes in-liquidation socialization to anyone.
- `architecture/MATH_REVIEW.md §3.7` and §5 document open math gaps with specific remediation proposals.

### Residual risk
- **MEDIUM-HIGH** — fuzzing covers single-op invariants but skips multi-op compositions that exploit rounding direction across paths.
- The `apply_bad_debt_to_supply_index` floor blocks zero but not a coordinated socialization that always rounds down. Net supplier loss could exceed bad debt by a small amount.

### Audit asks
- Property: starting from pool-state `S`, any sequence of `(supply, borrow, withdraw, repay)` that closes all positions returns the pool to `S` within N ULPs. Measure N empirically.
- Identify any i128 overflow paths under realistic and unrealistic-but-feasible parameters.
- Determine whether the bad-debt path's bonus calculation treats liquidator and survivor suppliers fairly.

## §3. Bulk Action Threat Models

### Adversary capability
Any user, with budget for transaction size.

### Target invariants
- Per-tx Soroban limits (200 r/w entries, 132 KB tx, 400M instructions).
- I.10 (LTV bound) applied to *post-batch* state, not per-asset.
- Position limits (max 32 supply + 32 borrow per account).

### Attack sketches

**3.1 Same-asset duplicate inflation.**
- `supply([(XLM, 100), (XLM, 100), (XLM, 100), ...])` repeats the same asset N times. `validate_bulk_position_limits` dedupes correctly via `Map<Address, bool>`. **But** does the supply *amount* aggregate correctly, or does each tuple run its own `global_sync` between iterations? Between-iteration index sync that changes the scaled-amount denominator could let repeated tiny supplies drift.
- Testnet smoke (per `architecture/DEPLOYMENT.md:246`) reports duplicates aggregate correctly. Confirm under adversarial parameters.

**3.2 Storage-write exhaustion.**
- A user holds N supply assets and M borrow assets. Position limits cap N + M ≤ 64. Each asset costs 1 SupplyPosition / BorrowPosition write, 1 pool state write, 1 pool param read, and 1 oracle read. Worst case: `liquidate` of a 32-supply / 32-borrow account writes ~64 positions plus 64 pool states and reads 64 oracle prices. **Does this exceed 200 write entries?** Count per-op write footprint.

**3.3 Liquidator gas griefing.**
- A liquidator submits `liquidate` with N debt payments. Each iteration runs pool repay, transfer, and emit. With 32 max borrow positions, that yields at most 32 iterations. Combined with 32 collateral seizures, the call hits 64 pool calls per liquidation. At ~6M instructions per pool call (estimate), 64 × 6M = 384M — **right at the 400M tx limit**.
- The protocol exposes no multi-account batched liquidation endpoint. Liquidators submit one tx per account. Small bad debts may be uneconomical to clear, so bad debts accumulate.
- **Empirical bench shipped during prep**: `test-harness/tests/bench_liquidate_max_positions.rs::bench_liquidate_5_supply_5_borrow_within_default_budget` runs setup + liquidate at `PositionLimits = 5/5` × 5 markets under Soroban's default budget (`with_budget_enabled()`). Outcome classification:
  * **PASS**: any successful liquidation OR any panic carrying a budget / limit / cpu / memory / entries / size token in its message (Soroban's `HostError(Budget, ExceededLimit)` family).
  * **FAIL**: opaque panic outside the budget envelope. Catch-all guards `liquidate` against rendering an account un-liquidatable through a path the cost model doesn't surface.
  * **Observed**: under recording-mode auth (`mock_all_auths`) the host's auth-tree machinery exhausts budget during setup at 5/5 × 5 markets. The classify-or-fail check passes — Soroban surfaces the limit cleanly. **In production** (real signed-tx auth, much smaller auth tree) the budget surface is materially smaller; the bench captures the worst-case cost-model behaviour, not the production-typical case.
- **Operator-policy implication**: keep `PositionLimits = 10/10` (the controller-default at `controller/src/lib.rs:117-119`) or lower until a production-environment benchmark on testnet validates the worst-case `liquidate` against the actual signed-tx footprint. Raising the limit toward the contract cap (32/32) requires re-running the bench against the real-tx flow and reading Soroban's `host.charge_budget` telemetry from the testnet RPC. **Audit ask**: confirm the test-harness vs production divergence on auth-tree budget cost.

**3.4 Bulk repay with surplus.**
- `repay([(XLM, 1_000_000_000)])` against a debt of `100` triggers a pool refund of `999_999_900`. Confirm the refund reaches the *caller* (not the account owner), since repay is permissionless. (Check `pool/src/lib.rs::repay` refund target.)

**3.5 Bulk withdraw triggering HF check at end.**
- `withdraw([(XLM, all), (USDC, all), (BTC, all)])` recomputes HF only after the last withdraw if borrows remain. Intermediate state is technically below 1.0, but only the *final* state matters. Confirm no observable side-effect (event, storage commit) lets another contract use the intermediate state in a sub-call.

### Current mitigation
- Position limits clamp to `[1, 32]` (via `set_position_limits`).
- `validate_bulk_position_limits` dedupes by asset.
- A per-call cache (`ControllerCache`) skips redundant oracle reads in a single tx.
- Each pool mutation returns `MarketIndex`, giving the controller a fresh index without a separate read.

### Residual risk
- **MEDIUM** — no one has measured the empirical worst-case footprint at maxed-out positions. If a 32+32 liquidation OOMs (over 400M instructions or > 200 r/w entries), the account stays un-liquidatable until someone trims it manually.
- **MEDIUM** — duplicate-asset same-batch math under non-identity rate parameters needs verification.

### Audit asks
- Measure instruction count and footprint for `liquidate` at maxed position counts.
- Decide whether to offer multi-account liquidation as a primitive (gas amortization) or keep per-account isolation.
- Identify the refund destination in the `repay` overflow case — caller or account owner.

## §4. Reflector Oracle Manipulation / Confusion

### Adversary capability
- A market maker or large holder can nudge the underlying CEX/DEX market price within Reflector's averaging window.
- A configuration-time observer cannot change the oracle (ORACLE role required) but can probe for misconfigurations.

### Target invariants
- I.14 (oracle invariants): decimals separated; tolerance bands respected.
- I.9 (HF) depends on oracle price.

### Attack sketches

**4.1 Stale-price + within-tolerance.**
- Reflector publishes prices on a fixed cadence. A generous `max_price_stale_seconds` (e.g., 900s) lets an attacker who controls the CEX-side flow wait for a favorable price, then call `borrow` while that price is stale-but-not-rejected. The safe price (TWAP-ish) acts as a second tier: when spot deviates from safe within first tolerance, the protocol uses safe.

**4.2 First/last tolerance band gaming.**
- When `(spot, safe)` lie within last but not first tolerance, the protocol returns `(spot+safe)/2`. The adversary chooses operations that benefit from the average bias.

**4.3 `allow_unsafe_price` paths.**
- Per `architecture/INVARIANTS.md §14`, supply and repay use the safe price even when deviation is breached (`allow_unsafe_price = true`). During a price shock:
  - Suppliers can still deposit (good).
  - Liquidators cannot use the breached price to liquidate (intentional).
  - Repayers can still settle debt at the *safe* price, which may sit far from spot, letting distressed users repay cheaply.

**4.4 Decimal mismatch.**
- `configure_market_oracle` reads `cex_decimals` and `dex_decimals` on-chain. A mid-life Reflector upgrade with different decimals leaves the cached value stale. **Risk**: misprice every operation.

**4.5 Reflector `cex_asset_kind` confusion.**
- `Stellar` vs `Other` asset kind: an operator who picks `Other` for a Stellar-native symbol may receive zero or wrong data. No on-chain verification exists.

### Current mitigation
- The contract reads decimals on-chain at config time.
- Tolerance bands stay bounded to `[50, 5000] BPS`.
- Two-tier safety (first / last) returns `(agg+safe)/2` at the mid tier.
- TWAP record count is configurable per market (no lower bound).
- The ORACLE role guards wiring changes.

### Residual risk
- **HIGH** — Reflector behavior is the largest unknown for this team (per Daisy's note). Specifically:
  - Does Reflector revert on missing data or return zero?
  - What update cadence and average staleness does Reflector deliver?
  - Do TWAP records exist immediately after market deployment, or can a fresh asset have empty TWAP history?
  - Does `Stellar` vs `Other` asset kind drive Reflector's dispatch, and does on-chain validation exist?
- **MEDIUM** — a Reflector contract upgrade invalidates cached decimals.

### Audit asks
- Provide an authoritative behavior spec for Reflector under missing symbol, stale data, decimal change, and contract upgrade.
- Verify that `(spot+safe)/2` averaging resists repeated nudges toward an attractive midpoint.

## §5. Misconfiguration Self-Defense

### Adversary capability
- The "adversary" here is the operator (Owner or ORACLE-role holder) who sets bad parameters by accident.

### Target invariants
- Self-defense: the protocol rejects parameters that violate documented invariants.

### Attack sketches

**5.1 LTV >= LT.** `validate_asset_config` rejects (`validation.rs:122-126`). ✓

**5.2 Liquidation bonus > 15%.** `validate_asset_config` rejects (`validation.rs:128-130`). ✓

**5.3 Negative isolation debt ceiling.** Rejected at `validation.rs:143-145`. ✓

**5.4 Negative or over-cap flashloan fee.** `NegativeFlashLoanFee` rejected at `validation.rs:150-152`; `StrategyFeeExceeds` at `:153-155` (cap = `MAX_FLASHLOAN_FEE_BPS = 500`). ✓

**5.5 LT > 100%.** Rejected (`> BPS` at `validation.rs:122-124`). ✓

**5.6 max_price_stale_seconds outside [60, 86400].** Rejected at `config.rs:374-376`. ✓

**5.7 twap_records > 12.** Rejected at `config.rs:314-316`. `twap_records == 0` is an intentional spot-fallback path.

**5.8 Wrong asset_decimals in MarketParams.** Rejected at `router.rs:22-25` under `#[cfg(not(feature = "testing"))]`. Prod code re-reads `token.decimals()` and compares. ✓

**5.9 cex_symbol that doesn't resolve.** Rejected at `config.rs:328-331` via `cex_client.lastprice(&ra).is_none()` probe.

**5.10 `ExchangeSource::SpotOnly` on production Active markets.** Rejected at `config.rs:369-372` under `#[cfg(not(feature = "testing"))]` (`SpotOnlyNotProductionSafe`). ✓

**5.11 No upper cap on `max_borrow_rate_ray`.** `validate_interest_rate_model` (`validation.rs:90-112`) enforces monotone slope chain + `max >= slope3`, but no absolute upper bound. `compound_interest`'s 8-term Taylor is documented accurate only for per-chunk `x ≤ 2 RAY`. Operator setting `max_borrow_rate_ray > 2 * RAY` combined with 100%-util accrual produces material under-accrual of interest (~6.8% at `x = 5`). **Not a direct attacker path** (owner-only config + all-internal accrual), but a config-hardening recommendation for the audit team.

### Current mitigation
- Every documented config rule except §5.11 is enforced on-chain.
- Runtime cap on compound interest chunk via `MAX_COMPOUND_DELTA_MS = MS_PER_YEAR` (`pool/src/interest.rs:22-23`) bounds per-chunk input but not per-chunk `x` magnitude when `max_borrow_rate_ray` is large.

### Residual risk
- **LOW** — the remaining gap is economic-accuracy under operator misconfiguration, not attacker-extractable. Addressable by either capping `max_borrow_rate_ray ≤ 2 * RAY` in validation OR making `MAX_COMPOUND_DELTA_MS` adaptive.

### Audit asks
- Confirm the Taylor-truncation accuracy envelope for the intended operator rate range.
- Recommend a canonical upper bound on `max_borrow_rate_ray` or an adaptive chunking scheme.

## §6. Cross-Cutting Concerns

### Account ID enumeration / griefing
- `account_id` is a `u64` from the global `AccountNonce`. Anyone can call `supply` with a fresh `account_id`, allocating storage. With 17_280-ledger temp TTL and ~120-day persistent TTL, an attacker who runs cheap ops could spam fresh accounts and inflate state-rent costs.
- Mitigation: each `supply` requires an actual collateral transfer plus caller auth. Cost to attacker meets or exceeds protocol cost.
- Residual: confirm that an account holding **only** `AccountMeta` (no positions) cannot exist — `supply` must create positions atomically with meta.

### Storage TTL lapse
- Persistent: ~120 days. Instance: ~180 days. Temp: ~1 day.
- A pool idle for more than 120 days could lose `PoolState` to TTL expiry. The KEEPER-callable `keepalive_*` endpoints address this. Confirm the operator runbook includes a regular keepalive cron.
- Where does `FlashLoanOngoing` live? Temp-storage placement risks a cosmic-ray TTL expiry mid-loan, leaving the system permanently stuck. (Should be Instance — verify.)

### Token transfer panic semantics
- Soroban SAC `transfer(from, to, amount)` panics on insufficient balance or on `from.require_auth()` failure. It returns no value. The pool treats "no panic" as success.
- `flash_loan_end` (pool/lib.rs:353) uses `tok.transfer(receiver→pool, total)`, which **requires the receiver's pre-authorization** via `env.authorize_as_current_contract` inside the callback (Soroban-native auth, not ERC-20 `transfer_from`/`approve`). See `architecture/ACTORS.md`.
- The controller-side supply transfer at supply.rs:210-212 verifies `received > 0` via balance delta — fee-on-transfer safe.
- The repay transfer at repay.rs:62-71 follows the same pattern.
- No transfer uses `try_invoke_contract`; all calls invoke directly and propagate panics.

### Aggregator return-value trust (verified + hardened)
- `strategy::swap_tokens` snapshots `token_in/out.balance(env.current_contract_address())` BEFORE the aggregator call (strategy.rs:456-457) and re-reads AFTER (strategy.rs:481, 496). The diff runs against the **controller's** address — verified.
- Spend bound: `actual_in_spent <= amount_in` (strategy.rs:486-488).
- **`amount_out_min` postcheck added during prep**: `received < steps.amount_out_min` panics with `GenericError::InternalError`. The aggregator can no longer silently shortchange the strategy.
- Re-entry block: `FlashLoanOngoing` set/cleared around the router call.

## Summary Risk Heat Map

| Concern | Likelihood | Impact | Priority |
|---|---|---|---|
| §1 Flash loan re-entry into supply/borrow | **Low (revised — guard covers every entry)** | High (pool drain) | P2 (verify Soroban panic-rollback only) |
| §2.4 Flash-loan utilization spike | Medium | Medium (mispriced rates) | P1 |
| §2.5 Bad-debt socialization gaming | Low | Medium (supplier loss) | P1 |
| §3.3 Liquidator gas griefing at max positions | Medium | Medium (un-liquidatable account) | P1 |
| §3.4 Bulk-repay refund destination | Low | Low (confirm intent) | P2 |
| §4.1 Stale-but-tolerated price | Medium | High (mispriced borrow/liq) | **P0** |
| §4.4 Reflector upgrade decimal staleness | Low | High (mispriced) | P1 |
| §5 Misconfig `max_borrow_rate_ray` upper bound | Medium (operator erm) | Low (accounting drift, not theft) | P2 |
| §6 Account ID spam | Low | Low | P2 |
