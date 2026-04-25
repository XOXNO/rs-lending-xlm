# STRIDE Threat Model — `rs-lending-xlm`

Submission deliverable matching the [Stellar Developer Foundation STRIDE
template](https://developers.stellar.org/docs/build/security-docs/threat-modeling).
This document is structured around the four template questions:

1. What are we working on?
2. What can go wrong?
3. What are we going to do about it?
4. Did we do a good job?

For deeper risk-tier analysis, blast-radius scenarios, and audit-team asks
beyond the STRIDE surface, see [`audit/THREAT_MODEL.md`](./THREAT_MODEL.md).
For the data-flow visualisation referenced throughout this document, see
[`architecture/DATAFLOW.md`](../architecture/DATAFLOW.md).

---

## 1. What are we working on?

### 1.1 Project description

`rs-lending-xlm` is a multi-asset, two-tier lending and borrowing protocol on
Stellar / Soroban. A single `controller` contract is the protocol entrypoint;
one `pool` child contract per listed asset owns liquidity, interest accrual,
reserves, and revenue accounting. The controller deploys pools from a stored
WASM template and is each pool's admin.

Capabilities: supply, borrow, repay, withdraw across listed assets in normal,
e-mode, or isolation account modes; permissionless liquidations driven by a
deterministic health-factor cascade; flash loans with pool-side reserve
verification and atomic repayment; strategy primitives (leveraged positions,
debt swaps, collateral swaps) routed through an operator-set aggregator;
protocol revenue accruing into the supply index and forwarded to a treasury
accumulator on claim.

### 1.2 Actors and trust tiers

Reproduced from [`architecture/ACTORS.md`](../architecture/ACTORS.md):

```
Owner (single Address, two-step transferable)
├── KEEPER role  — bad-debt cleanup, index maintenance, TTL keepalive
├── REVENUE role — claim_revenue, add_rewards
└── ORACLE role  — configure_market_oracle, edit_oracle_tolerance, disable_token_oracle

Controller contract (each pool's admin)

User (any Address)
├── Account owner       — caller == AccountMeta.owner
├── Liquidator          — pays debt + receives collateral + bonus
└── Flash-loan caller   — signs the outer tx; receiver exports execute_flash_loan

External contracts (trusted-but-validated)
├── Reflector CEX oracle
├── Reflector DEX oracle (optional, DualOracle mode)
├── Aggregator (swap router)
├── Accumulator (revenue sink)
└── Token SACs / SEP-41 contracts on the operator allowlist
```

### 1.3 Data-flow summary

The full data-flow diagram, with explicit trust boundaries between the
controller, each pool, the price surface, the swap aggregator, the accumulator,
and external token SACs, lives in
[`architecture/DATAFLOW.md`](../architecture/DATAFLOW.md). This document
references that diagram by section number when discussing specific flows.

Sequence diagrams for `supply`, `borrow`, `repay`, `withdraw`, `liquidate`,
`flash_loan`, and revenue claim live in
[`architecture/ARCHITECTURE.md §Controller-to-Pool Communication`](../architecture/ARCHITECTURE.md#controller-to-pool-communication).

### 1.4 Trust boundaries

| Boundary | Crossing direction | Validation responsibility |
|---|---|---|
| User ↔ Controller | inbound calls, outbound transfers | Soroban host auth + per-fn `require_auth()` + account-owner check; structural validation in `validation.rs`, `positions/*` |
| Controller ↔ Pool | controller→pool admin call; pool→controller `MarketIndex` return | `verify_admin(&env)` on every pool mutator; controller owns risk decisions |
| Controller ↔ Reflector oracle | outbound read, inbound `PriceData` | `controller/src/oracle/mod.rs` enforces non-positive rejection, hard-staleness, future-timestamp clamp, ≥50% TWAP coverage, two-tier tolerance band, fail-closed on risk-increasing ops |
| Controller ↔ Aggregator | outbound contract call inside `strategy::swap_tokens` | Pre/post `balance()` snapshots; spend ≤ `amount_in`; controller-side `received >= amount_out_min`; `FlashLoanOngoing` set/cleared around the call |
| Controller ↔ Accumulator | outbound transfer | Address-only trust; tokens forwarded blindly per claim |
| Pool ↔ Token SAC | outbound `transfer`/`balance` | "no panic" treated as success (Soroban convention); fee-on-transfer / rebasing tokens excluded by allowlist policy |

---

## 2. What can go wrong?

For each STRIDE category, at least one issue is identified and uniquely
labelled. Items map back to the deeper analyses in
[`audit/THREAT_MODEL.md`](./THREAT_MODEL.md) where applicable.

### 2.1 Spoofing

Is the user who they say they are? Soroban routes every authenticated call
through `Address.require_auth()`, which the host enforces. The risks below
are spoofing-adjacent: identity confusion across actors, callbacks, and
external contracts.

| ID | Issue |
|---|---|
| **Spoof.1** | A flash-loan receiver could attempt to invoke controller mutating endpoints from inside its `execute_flash_loan` callback while masquerading as the original initiator. |
| **Spoof.2** | An aggregator-side callback (during `strategy::swap_tokens`) could re-enter the controller, presenting itself as a privileged caller. |
| **Spoof.3** | A `repay` caller pays down a third party's debt without their consent (anyone-can-repay). Surplus refund must reach the actual repayer (caller), not the account owner. |
| **Spoof.4** | An operator chooses `ReflectorAsset::Other(symbol)` for a Stellar-native asset (or vice versa), mis-resolving Reflector's dispatch and pulling prices for a different token. |
| **Spoof.5** | A pool that is not deployed by the controller's `create_liquidity_pool` template could attempt to register itself with the controller — the canonical Pool↔Asset map must reject this. |
| **Spoof.6** | A token contract on the allowlist could later be upgraded or proxied to malicious WASM and continue to be trusted at runtime. |

### 2.2 Tampering

Has the data or code been modified in some way that violates an invariant?

| ID | Issue |
|---|---|
| **Tamper.1** | Half-up rounding asymmetry in `mul_div_half_up` could let an attacker repeat (supply, withdraw) tuples and accumulate scaled-balance drift in their favour, eventually leaking principal from the supply pool. |
| **Tamper.2** | A flash-loan-driven utilisation spike around an `update_indexes` call locks in extreme rates that become "real" interest debt for actual borrowers post-repay. |
| **Tamper.3** | Bulk `supply([(XLM,100),(XLM,100),…])` repeats the same asset; if amounts do not aggregate consistently or `global_sync` between iterations changes the scaled-amount denominator, repeated tiny supplies could drift. |
| **Tamper.4** | An attacker-deployed `receiver` callback modifies the `flash_loan_end` repayment behaviour by re-routing the SAC `transfer` (e.g. front-running the inner contract's auth declaration). |
| **Tamper.5** | The aggregator returns `received < amount_out_min` (or silently shortchanges the swap), tampering with strategy invariants. |
| **Tamper.6** | A Reflector contract upgrade silently changes `decimals()` or `resolution()`, leaving the cached `cex_decimals` / `dex_decimals` in `MarketConfig` stale and mispricing every operation. |
| **Tamper.7** | Operator misconfiguration (LT≤LTV, liquidation_bonus_bps>15%, isolation_debt_ceiling negative) tampers with on-chain risk parameters. |
| **Tamper.8** | A malicious `add_rewards` call against a fee-on-transfer SAC inflates the supply index relative to actual reserves. |

### 2.3 Repudiation

Is there enough data to "prove" a user took an action if they were to deny it?

| ID | Issue |
|---|---|
| **Repudiate.1** | A user who triggers bad-debt socialization via self-liquidation could later deny the action; suppliers must be able to attribute the supply-index drop unambiguously. |
| **Repudiate.2** | An ORACLE-role holder repoints a market to a manipulated oracle and later denies the change. |
| **Repudiate.3** | A KEEPER selectively withholds `update_account_threshold` propagations for specific accounts, then denies preferential treatment. |
| **Repudiate.4** | A flash-loan caller initiates a complex multi-step strategy that loses funds and later denies authorising the inner aggregator route. |
| **Repudiate.5** | An operator changes `set_position_limits` or `disable_token_oracle` and later denies it (immediate-effect, no two-step today). |

### 2.4 Information Disclosure

Is there anywhere private data is over-shared, or are protections missing for
data that should be confidential?

| ID | Issue |
|---|---|
| **Info.1** | Account positions, e-mode category, isolation flag, liquidation thresholds, and supply/borrow asset lists are all on-chain by design — but operator-supplied free-text fields (e.g. `cex_symbol` strings) could leak unintended metadata. |
| **Info.2** | Liquidation cascades expose the exact failed-target ladder (`1.02 → 1.01 → fallback`) via emitted events, telegraphing protocol thresholds to MEV searchers. |
| **Info.3** | The controller cache deliberately reuses oracle reads inside one tx; a sub-call observer reading prices via cross-contract invocation could infer the controller's risk model state. |
| **Info.4** | `views.rs` exposes `account_data`, `health_factor`, `total_collateral` ABIs that an attacker can use to enumerate every borrower's HF and target the closest-to-liquidation accounts. (Public by design — this is "at-risk" disclosure, not a defect.) |

### 2.5 Denial of Service

Can someone, without authorisation, impact the availability of the service?

| ID | Issue |
|---|---|
| **DoS.1** | Liquidator gas griefing: at maxed `PositionLimits = 32/32`, `liquidate` performs ~64 pool calls. If the call exceeds the 400M instructions / 200 r/w entries / 286 KB write-bytes ledger limits, the account becomes un-liquidatable. |
| **DoS.2** | Account-ID spam: `AccountNonce` is monotonic; an attacker could allocate fresh accounts via cheap operations and inflate state-rent. |
| **DoS.3** | Storage TTL lapse: a pool idle for >120 days could lose `PoolState` to TTL expiry; without `keepalive_pools` cron, the pool becomes unusable. |
| **DoS.4** | Reflector outage or stale feed past `max_price_stale_seconds` (60–86_400 s) freezes risk-increasing ops on the affected market. |
| **DoS.5** | Aggregator-side outage halts every strategy primitive (`multiply`, `swap_*`). |
| **DoS.6** | A bulk operation hitting the 132 KB tx size or 16 KB event size cap reverts at the host boundary, blocking legitimate large operators. |
| **DoS.7** | Flash-loan guard `FlashLoanOngoing` is in **Instance** storage. If somehow set to `true` and not cleared (panic timing), the protocol could reach a stuck state. (Verified via panic-rollback semantics — Instance writes commit at tx end; tx revert clears the write.) |
| **DoS.8** | `update_indexes` dispatched against many markets in one call may hit budget; the keeper must shard. |
| **DoS.9** | `disable_token_oracle` is a single-call kill switch — ORACLE-role compromise freezes withdrawals on the affected market until reconfigured. |

### 2.6 Elevation of Privilege

Are there ways to gain privileges beyond what was granted, through legitimate
or illegitimate means?

| ID | Issue |
|---|---|
| **Elevation.1** | A flash-loan callback or aggregator callback re-enters the controller and triggers a borrow / withdraw / liquidate that bypasses the real HF check by exploiting transient mid-tx state. |
| **Elevation.2** | A user crafts a `supply` / `borrow` batch that bypasses `validate_bulk_position_limits` (e.g. via duplicate-asset tuples that dedupe but separately accumulate). |
| **Elevation.3** | A non-KEEPER caller triggers `clean_bad_debt_standalone` (the standalone path that mutates `supply_index` downward) by exploiting a missing role gate. |
| **Elevation.4** | An ORACLE-role holder repoints a market to a controlled feed and arbitrages liquidations against it. |
| **Elevation.5** | An operator with the Owner key compromises the protocol globally — no on-chain timelock or multisig enforces a delay. |
| **Elevation.6** | An attacker exploits `approve_token_wasm`'s creation-time-only semantics: a token added to the allowlist, then later upgraded to malicious WASM, continues to be trusted at runtime (no `revoke_token_wasm` runtime check). |
| **Elevation.7** | The flash-loan receiver mutates `FlashLoanOngoing` (it should be Instance and out of reach) or any controller storage cell during its callback. |
| **Elevation.8** | A liquidator triggers bad-debt socialization on an account they control to manipulate the supply index for an external short position (e.g. via concurrent `swap_debt`). |

---

## 3. What are we going to do about it?

For each issue above, the protocol either ships an on-chain mitigation or
documents the residual risk and the operator-policy / monitoring response.
Mitigations cite file:line in the production crates so the audit team can
verify each claim against the code.

### 3.1 Spoofing — Mitigations

| ID | Remediation |
|---|---|
| **Spoof.1.R.1** | `process_flash_loan` sets `FlashLoanOngoing = true` in Instance storage at `controller/src/flash_loan.rs:43` and clears at `:61`. **Every** mutating controller endpoint reads the guard via `require_not_flash_loaning` (supply.rs:25, borrow.rs:100, withdraw.rs:19, repay.rs:19, liquidation.rs:30, flash_loan.rs:22, lib.rs:315/349/367/634/640, strategy.rs:55/228/345/518). Any callback re-entry into a mutating endpoint panics with `FlashLoanError::FlashLoanOngoing`, reverting the entire tx. |
| **Spoof.1.R.2** | The receiver callback (`env.invoke_contract::<()>(receiver, "execute_flash_loan", …)` at flash_loan.rs:51-55) cannot reach controller endpoints under the guard. It can only reach external contracts (aggregator, tokens). Soroban panic-rollback semantics ensure a panic anywhere in the flash-loan flow reverts the cleared `FlashLoanOngoing` write — future flash loans remain available. |
| **Spoof.2.R.1** | `strategy::swap_tokens` (`controller/src/strategy.rs:467`) brackets the aggregator call with `set_flash_loan_ongoing(true/false)`. An aggregator-side callback into any controller mutating endpoint panics. Defence-in-depth: controller-side `received >= steps.amount_out_min` post-check (strategy.rs:517) catches a router that ignores its own slippage parameter. |
| **Spoof.3.R.1** | `pool/src/lib.rs:251-268` (line 267) refunds repay overpayment to `caller` (the actual repayer), not the account owner. Verified: `repay` is intentionally permissionless on the target account; refund target is unambiguous. Documented in `architecture/ENTRYPOINT_AUTH_MATRIX.md` and `architecture/ACTORS.md`. |
| **Spoof.4.R.1** | `configure_market_oracle` reads token decimals from the asset contract (`config.rs:321`) and probes the CEX feed via `cex_client.lastprice(&ra).is_none()` (`config.rs:328-331`) — a misconfigured `cex_symbol`/`cex_asset_kind` combination is rejected at config time. `architecture/STELLAR_NOTES.md §3` lists residual asks for the Reflector team about `Stellar` vs `Other` dispatch; flagged for auditor review. |
| **Spoof.5.R.1** | Pool↔Asset binding is one-way: `controller.create_liquidity_pool(asset, …)` deploys a fresh pool from `PoolTemplate` and writes `MarketConfig.pool_address`. There is no controller endpoint to register an externally-deployed pool. |
| **Spoof.6.R.1** | `approve_token_wasm` is creation-time only; `architecture/ACTORS.md` and `architecture/CONFIG_INVARIANTS.md` document that `revoke_token_wasm` does not stop existing pools at runtime. Operator-policy mitigation: monitor approved-token WASM hashes off-chain; on detected token compromise, operator calls `pause()` and migrates users to a new market. Code-level hardening (runtime token-WASM hash check) is tracked as Maturity M-4. |

### 3.2 Tampering — Mitigations

| ID | Remediation |
|---|---|
| **Tamper.1.R.1** | All cross-domain math routes through `common::fp_core::mul_div_half_up` using `I256` intermediates (`common/src/fp_core.rs:13-20`). Half-up is the single rounding convention; `mul_div_floor` is used only where a lower bound is required (liquidation base) and is annotated with the reason. `cargo-fuzz` targets `fp_mul_div`, `fp_div_by_int`, `fp_rescale` exercise the primitives; proptest `fuzz_conservation` covers multi-op compositions. |
| **Tamper.1.R.2** | `architecture/INVARIANTS.md §3` defines the scaled-balance invariant and `MATH_REVIEW.md` tracks Certora rule coverage. Audit ask: closed-sequence pool-state invariant within N ULPs (THREAT_MODEL.md §2 Audit asks). |
| **Tamper.2.R.1** | `compound_interest` uses an 8-term Taylor series with documented accuracy `< 0.01 %` for per-chunk `x ≤ 2 RAY`. `MAX_COMPOUND_DELTA_MS = MS_PER_YEAR` (`pool/src/interest.rs:22-23`) caps per-chunk input. Outstanding hardening: cap `max_borrow_rate_ray ≤ 2 * RAY` in `validate_interest_rate_model` and `pool.update_params` OR make `MAX_COMPOUND_DELTA_MS` adaptive — tracked as Maturity C-2. |
| **Tamper.3.R.1** | `validate_bulk_position_limits` dedupes by asset via `Map<Address, bool>` (`controller/src/validation.rs`). Testnet smoke (`architecture/DEPLOYMENT.md:246`) reports duplicates aggregate correctly. Audit ask: confirm under adversarial parameters. |
| **Tamper.4.R.1** | `pool.flash_loan_end` (`pool/src/lib.rs:353`) calls `tok.transfer(&receiver, &pool, &(amount + fee))`. Soroban-native auth requires the receiver to call `env.authorize_as_current_contract` inside its callback — there is no allowance pattern. Failure to pre-authorise panics with auth-denied, reverting the tx. Documented in `architecture/ACTORS.md`. |
| **Tamper.5.R.1** | `strategy::swap_tokens` snapshots `token_in/out.balance(env.current_contract_address())` BEFORE the aggregator call (strategy.rs:456-457) and re-reads AFTER (strategy.rs:481, 496). The diff runs against the controller's address. Spend bound: `actual_in_spent ≤ amount_in` (strategy.rs:486-488). Controller-side `received >= amount_out_min` postcheck (strategy.rs:517) panics `GenericError::InternalError` on a silent-shortchange aggregator. |
| **Tamper.6.R.1** | `configure_market_oracle` reads decimals once at config time and stores them on `MarketConfig`. Operator runbook (`architecture/DEPLOYMENT.md`) requires re-running `configure_market_oracle` after any Reflector contract upgrade. Reflector-team behaviour spec for upgrades is tracked as an open ask in `architecture/STELLAR_NOTES.md §3`. |
| **Tamper.7.R.1** | `validate_asset_config` rejects: LTV ≥ LT (`validation.rs:122-126`), `liquidation_bonus_bps > MAX_LIQUIDATION_BONUS = 1500` (`:128-130`), negative `isolation_debt_ceiling_usd_wad` (`:143-145`), `flashloan_fee_bps < 0` (`:150-152` `NegativeFlashLoanFee`), `flashloan_fee_bps > MAX_FLASHLOAN_FEE_BPS = 500` (`:153-155` `StrategyFeeExceeds`), LT > 100 % (`:122-124`). `validate_interest_rate_model` enforces monotone slope chain + `max ≥ slope3` (`:90-112`). Per-config-field × validation-site map: `architecture/CONFIG_INVARIANTS.md`. |
| **Tamper.8.R.1** | `supply` and `repay` verify balance delta (`controller/src/positions/supply.rs:210-212`, `repay.rs:62-71`) — fee-on-transfer safe at the controller boundary. `add_rewards` does NOT mirror this pattern today; operator policy MUST restrict `approve_token_wasm` to vanilla SAC / strict 1:1 transfer SEP-41 (documented in `architecture/DEPLOYMENT.md "Token allowlist policy"`). Code-level hardening is tracked as Maturity C-3. |

### 3.3 Repudiation — Mitigations

| ID | Remediation |
|---|---|
| **Repudiate.1.R.1** | Bad-debt socialization emits `PoolInsolventEvent` (`common/src/events.rs`) carrying `old_supply_index_ray`, `new_supply_index_ray`, account ID, and asset. Off-chain indexers reproduce the exact pre/post state; Soroban tx hashes provide non-repudiation at the chain level. |
| **Repudiate.1.R.2** | Liquidations emit a `LiquidationEvent` that carries the liquidator address, target account, debt-payment vector, seized-collateral vector, and bonus/protocol-fee splits. |
| **Repudiate.2.R.1** | Every `configure_market_oracle` and `edit_oracle_tolerance` emits `UpdateAssetOracleEvent` with the full `OracleProviderConfig` payload (`architecture/ORACLE.md §Events`). `disable_token_oracle` emits a corresponding `UpdateAssetOracleEvent` with the disabled state. |
| **Repudiate.3.R.1** | KEEPER actions (`update_indexes`, `update_account_threshold`, `clean_bad_debt`, `keepalive_*`) all emit events naming the caller. `architecture/INCIDENT_RESPONSE.md` enumerates monitoring obligations. |
| **Repudiate.4.R.1** | Strategy entrypoints emit `MultiplyEvent`, `SwapDebtEvent`, `SwapCollateralEvent`, `RepayDebtWithCollateralEvent` carrying the full step list including aggregator route hints. The flash-loan path emits `FlashLoanEvent` with caller, asset, amount, fee. |
| **Repudiate.5.R.1** | All Owner config endpoints emit dedicated events: `UpdateAssetConfigEvent`, `UpdateMarketParamsEvent`, `ApproveTokenWasmEvent`, `UpdateMarketStateEvent`. Operator-policy mitigation: every Owner / role action MUST be off-chain-multisig-gated; events provide post-hoc audit trail. Code-level hardening (on-chain timelock for highest-impact ops) is tracked as Maturity H-1. |

### 3.4 Information Disclosure — Mitigations

| ID | Remediation |
|---|---|
| **Info.1.R.1** | `cex_symbol: Symbol` is constrained by Soroban's `Symbol` type to alphanumeric short names (≤32 chars) — no free-text payload. |
| **Info.2.R.1** | This is by-design. Liquidations are public, permissionless, and the cascade values (`1.02`, `1.01`, fallback) are documented invariants. MEV exposure is the same as for every public lending protocol. Mitigation is liquidator-incentive design (bonus + protocol-fee split), not opacity. |
| **Info.3.R.1** | Oracle reads inside the controller cache (`controller/src/cache/mod.rs`) are not exposed cross-contract. View functions (`views.rs`) are explicit ABI; a sub-call to `view_*` returns the same data any external caller can read — no privilege escalation. |
| **Info.4.R.1** | By design — public lending protocols expose borrower HF for liquidator competition. No remediation required. |

### 3.5 Denial of Service — Mitigations

| ID | Remediation |
|---|---|
| **DoS.1.R.1** | `set_position_limits` clamps `[1, 32]`. Default at controller `__constructor` is `10/10` (`controller/src/lib.rs:117-119`). Empirical max-position liquidate cost benchmark at 32/32 is **outstanding** — tracked in `audit/AUDIT_CHECKLIST.md "Still Outstanding"` and `THREAT_MODEL.md §3.3`. Operator-policy mitigation until measured: keep `PositionLimits` at `10/10` for production deployment. |
| **DoS.2.R.1** | Each `supply` requires an actual collateral transfer + caller auth — cost to attacker meets or exceeds protocol cost. `validation::require_account_owner` ensures an account holding only `AccountMeta` (no positions) cannot exist; positions and meta are atomically created in `supply`. |
| **DoS.3.R.1** | KEEPER-callable `keepalive_shared_state(assets)`, `keepalive_accounts(ids)`, `keepalive_pools(assets)` extend per-key TTL. Operator runbook (`architecture/DEPLOYMENT.md`) requires a periodic keeper cron (cadence < 30-day persistent threshold). Regression coverage: `test-harness/tests/fuzz_ttl_keepalive.rs`. |
| **DoS.4.R.1** | `max_price_stale_seconds` clamps to `[60, 86_400]` (`controller/src/config.rs:381`). Stale-feed reverts are intentional — better to halt risk-increasing ops than trade on bad prices. Supply and repay use `allow_unsafe_price = true` and continue on the safe anchor (`architecture/ORACLE.md`). |
| **DoS.5.R.1** | Aggregator outage affects strategy primitives only; supply / borrow / repay / withdraw / liquidate paths do not depend on the aggregator. `set_aggregator` is Owner-callable; operator can swap to a fallback router. |
| **DoS.6.R.1** | `validate_bulk_position_limits` clamps batch size against `PositionLimits`. Bulk endpoints document Soroban limits in `architecture/STELLAR_NOTES.md`. The 16 KB event-size cap is observed via `UpdatePositionEvent` payload sizing (recent commits a4d2afe / c0ced1a centralised the action discriminator). |
| **DoS.7.R.1** | `FlashLoanOngoing` lives in **Instance** storage (`controller/src/storage/mod.rs:175-186`). Soroban Instance writes commit at tx end — a panic anywhere in the flash-loan flow reverts the `set_flash_loan_ongoing(true)` write. Manual-clear path doesn't exist by design (cannot exist outside the protocol entry, by symmetry). |
| **DoS.8.R.1** | `update_indexes` accepts `Vec<Address>`; KEEPER cron MUST shard against the budget. Operator-policy in `architecture/DEPLOYMENT.md`. |
| **DoS.9.R.1** | `disable_token_oracle` is intentionally a single-call kill switch (`architecture/ACTORS.md §Operator policy notes`). ORACLE-role compromise mitigation: off-chain multisig MUST gate the role; operator policy required. Code-level hardening (timelock + emergency-disable two-step) is tracked as Maturity H-1. |

### 3.6 Elevation of Privilege — Mitigations

| ID | Remediation |
|---|---|
| **Elevation.1.R.1** | See **Spoof.1.R.1** / **Spoof.2.R.1**: the `FlashLoanOngoing` guard blocks every mutating-endpoint re-entry. HF check sites: `withdraw.rs:44-52`, `liquidation.rs:158-160`, `borrow.rs:404-411` (LTV pre-check); aggregator re-entry is bracketed at `strategy.rs:467/487`. |
| **Elevation.2.R.1** | `validate_bulk_position_limits` dedupes by asset (`Map<Address, bool>`). The proptest `fuzz_auth_matrix` exercises the bulk-batch surface against every documented invariant. |
| **Elevation.3.R.1** | `clean_bad_debt(caller, account_id)` is `#[only_role(caller, "KEEPER")]` (`controller/src/lib.rs:347`). The standalone path shares the same `execute_bad_debt_cleanup` math (`controller/src/positions/liquidation.rs:463`) — verified file:line in `audit/AUDIT_PREP.md "Confirmed correct"`. |
| **Elevation.4.R.1** | ORACLE-role configures oracle-feed wiring; `verify_admin` does not gate this. Operator-policy mitigation: ORACLE role MUST be on a separate multisig key from Owner. Tolerance bands are bounded `MIN_FIRST_TOLERANCE` and `MAX_LAST_TOLERANCE` so even role compromise cannot widen unbounded. Code-level hardening (timelock on `configure_market_oracle` for an existing market) is Maturity H-2. |
| **Elevation.5.R.1** | Documented residual risk in `architecture/ACTORS.md §Owner` and `audit/CODE_MATURITY_ASSESSMENT.md §5 Decentralization`. Operator-policy mitigation: Owner key on multisig with off-chain timelock. Code-level hardening (on-chain timelock for `upgrade`, `upgrade_pool`, `edit_asset_config`, `disable_token_oracle`) is Maturity H-1. |
| **Elevation.6.R.1** | See **Spoof.6.R.1**. |
| **Elevation.7.R.1** | Soroban host isolates contract storage by contract ID. A receiver contract cannot mutate the controller's `FlashLoanOngoing` cell — only the controller's own `set_flash_loan_ongoing(…)` helper writes that key. |
| **Elevation.8.R.1** | Bad-debt socialization requires `coll_usd ≤ 5 * WAD` AND `debt_usd > coll_usd` (liquidation.rs:127, 429-430) — both must hold. The supply-index floor at `10^18 raw` (pool/interest.rs:14, 131-135) clamps the maximum index drop. Documented as a "MEDIUM-HIGH residual" pending Certora coverage in `audit/THREAT_MODEL.md §2.5`. |

---

## 4. Did we do a good job?

### 4.1 Is the data-flow diagram referenced beyond creation time?

Yes. `architecture/DATAFLOW.md` is referenced from this STRIDE document, from
`architecture/ARCHITECTURE.md`, from `audit/THREAT_MODEL.md`, and from the
external auditor onboarding flow described in `audit/AUDIT_CHECKLIST.md`. It
is intended to be re-read on every architecture change.

### 4.2 Did STRIDE uncover any new design issues?

Yes — three classes of issue surfaced or were sharpened during this exercise:

1. **Flash-loan re-entry surface across every mutating endpoint** (Spoof.1
   / Elevation.1). The pre-existing controller already had `require_not_flash_loaning`
   on `supply`/`borrow`/`repay`/`withdraw`, but the STRIDE Spoofing review
   confirmed every entry on the public ABI is gated, not just the obvious
   strategy/liquidation/flash-loan endpoints. Documented in
   `architecture/ENTRYPOINT_AUTH_MATRIX.md` (`P` and `F` reentry columns).

2. **Aggregator-callback re-entry was a gap until pre-audit prep**
   (Spoof.2). The `strategy::swap_tokens` aggregator call was not bracketed
   by `set_flash_loan_ongoing(true/false)`. Mitigation shipped during prep
   at `controller/src/strategy.rs:467, 487`, with a controller-side
   `received >= amount_out_min` postcheck added at `:517` for
   defence-in-depth.

3. **Operator-side configuration tampering self-defence** (Tamper.7) drove
   the `architecture/CONFIG_INVARIANTS.md` gap analysis. Eight gaps closed
   during pre-audit prep (LT ≤ 10_000, `isolation_debt_ceiling_usd_wad ≥ 0`,
   `flashloan_fee_bps ≥ 0` with `NegativeFlashLoanFee` error, etc.).

### 4.3 Do the treatments adequately address the issues?

For shipped on-chain mitigations, yes — `audit/AUDIT_PREP.md` "Verified Facts
vs Inferred" pass cited file:line for each. For policy-only mitigations
(Owner-key custody, KEEPER-cron schedule, ORACLE-role multisig), the
treatments are documented but not enforced on-chain. These are the
"Decentralization Weak (1/4)" finding in
`audit/CODE_MATURITY_ASSESSMENT.md §5` and the "Still Outstanding" items
in `audit/AUDIT_CHECKLIST.md` (`max_borrow_rate_ray` cap, max-position
liquidate benchmark, Reflector behaviour spec).

### 4.4 Have additional issues been found after the threat model?

The pre-audit hunt findings (H-01 through H-08, M-01 through M-12, L-01
through L-13, I-01 through I-03) and adversarial-loop findings (N-01 through
N-13) were the input to the prior threat-modelling round. Every finding that
warranted code change has shipped — see `audit/REMEDIATION_PLAN.md` for the
canonical remediation log. Regression gates live in `test-harness/tests/fuzz_*.rs`
(C-01, M-03, M-08, H-03, H-04, L-05, M-09, M-10, M-11, M-14, N-02, NEW-01).

The threat model is a living tool. Updates are required when:

- a new entrypoint is added to the controller or pool ABI,
- a new actor / role is introduced,
- the flash-loan / strategy / liquidation flow changes shape,
- the oracle pipeline changes (additional sources, fallbacks, decimals
  upgrade behaviour),
- the storage durability tier of any key changes.

### 4.5 Process notes

- The STRIDE deliverable is parallel to the depth-first
  `audit/THREAT_MODEL.md`. STRIDE provides per-category breadth and the
  Stellar-template format auditors expect; THREAT_MODEL.md provides
  scenario-level depth, residual-risk classification, and engagement-team
  asks. Both ship.
- The data-flow visual is in `architecture/DATAFLOW.md` to keep STRIDE
  text-only and re-readable.
- Cross-references to file:line in `controller/`, `pool/`, `pool-interface/`,
  `common/` are stable on the frozen audit commit.
