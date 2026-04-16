# Entrypoint × Auth × Invariant × Pool-Call Matrix

Every public function on `controller` and `pool`, with auth gate, runtime invariants checked (file:line citations from a verification pass), and downstream pool calls.

Legend:
- **Auth**: `Owner` = `#[only_owner]`; `Role(X)` = `#[only_role(caller, "X")]`; `Caller` = `caller.require_auth()`; `AcctOwner` = caller-equals-owner check plus require_auth; `Liquidator` = `liquidator.require_auth()`; `Admin` = `verify_admin` (caller == controller); `View` = no auth, read-only.
- **Reentry**: `P` = `require_not_paused`; `F` = `require_not_flash_loaning`.

## Controller Lifecycle

| Fn | Auth | Reentry | Notes |
|---|---|---|---|
| `__constructor(admin)` | one-shot | — | Sets owner, admin, KEEPER+REVENUE+ORACLE roles, default position limits 10/10 (`lib.rs:117-119`) |
| `upgrade(new_wasm_hash)` | Owner (`lib.rs:128`) | pauses (`lib.rs:130`) | Requires non-zero hash; calls `pausable::pause` then `upgradeable::upgrade` |
| `pause()` | Owner (`lib.rs:138`) | — | Calls `pausable::pause` |
| `unpause()` | Owner (`lib.rs:143`) | — | Calls `pausable::unpause` |

## User Position Operations — verified checks

### `supply(caller, account_id, e_mode, assets)` — `lib.rs:152` → `positions/supply.rs:16`

Checks in execution order:

| # | Check | File:line | Error |
|---|---|---|---|
| 1 | `caller.require_auth()` | supply.rs:23 | host auth |
| 2 | `require_not_paused` | supply.rs:24 | PausableError |
| 3 | `require_not_flash_loaning` | supply.rs:25 | `FlashLoanError::FlashLoanOngoing` |
| 4 | Account-owner check (skipped on first-supply when `account_id==0`) | supply.rs:38 | `GenericError::AccountNotInMarket` |
| 5 | E-mode category not deprecated (per batch) | emode.rs:65-67 | `EModeError::EModeCategoryDeprecated` |
| 6 | `validate_bulk_position_limits` (per batch, atomic) | validation.rs:70 | `CollateralError::PositionLimitExceeded` |
| 7 | Asset supported (per asset) | validation.rs:74 | `GenericError::AssetNotSupported` |
| 8 | Amount > 0 (per asset) | validation.rs:75 | `GenericError::AmountMustBePositive` |
| 9 | E-mode asset membership / `EModeWithIsolated` (per asset) | emode.rs:80-82 | `EModeError` variants / `CollateralError::NotCollateral` |
| 10 | `is_collateralizable` via `can_supply()` | supply.rs:84-85 | `CollateralError::NotCollateral` |
| 11 | Isolation rules: `MixIsolatedCollateral`, etc. | emode.rs:88, 110-124 | `EModeError::MixIsolatedCollateral` |
| 12 | `supply_cap` (when cap > 0) | supply.rs:91 | `CollateralError::SupplyCapReached` |
| 13 | Token transfer caller→pool with balance-delta accounting; received > 0 | supply.rs:210-212 | `GenericError::AmountMustBePositive` |
| 14 | Pool call `pool.supply(position, price_wad, amount)` | supply.rs:218 | (pool admin path) |

**Notes:**
- `token_approved` is checked only at `create_liquidity_pool`, never at supply.
- Price comes from `cache.cached_price(asset)` (`allow_unsafe = true`; supply decreases risk).

### `borrow(caller, account_id, borrows)` — `lib.rs:166` → `positions/borrow.rs:97`

| # | Check | File:line | Error |
|---|---|---|---|
| 1 | `caller.require_auth()` | borrow.rs:98 | host auth |
| 2 | `require_not_paused` | borrow.rs:99 | PausableError |
| 3 | `require_not_flash_loaning` | borrow.rs:100 | `FlashLoanError::FlashLoanOngoing` |
| 4 | `account.owner == caller` | borrow.rs:104 | `GenericError::AccountNotInMarket` |
| 5 | `validate_bulk_position_limits` (atomic batch) | borrow.rs:109 | `CollateralError::PositionLimitExceeded` |
| 6 | LTV-collateral computed once per batch | borrow.rs:114-115 | (feeds all per-asset checks) |
| 7 | Asset supported (per asset) | borrow.rs:387 | `GenericError::AssetNotSupported` |
| 8 | Amount > 0 (per asset) | borrow.rs:388 | `GenericError::AmountMustBePositive` |
| 9 | E-mode validation | borrow.rs:396 | `EModeError` variants |
| 10 | `is_borrowable` flag | borrow.rs:400 | `CollateralError::AssetNotBorrowable` |
| 11 | Isolation: `isolation_borrow_enabled` for isolated accounts | borrow.rs:351 | `EModeError::NotBorrowableIsolation` |
| 12 | Silo: only one borrow position when siloed | borrow.rs:355-356 | `CollateralError::NotBorrowableSiloed` |
| 13 | Silo: existing borrows must match new asset when any are siloed | borrow.rs:361-367 | `CollateralError::NotBorrowableSiloed` |
| 14 | LTV check: total post-borrow debt ≤ LTV-collateral | borrow.rs:404-411 (`validate_ltv_collateral` 311-338) | `CollateralError::InsufficientCollateral` |
| 15 | `borrow_cap` (when cap > 0) | borrow.rs:412-419 (`validate_borrow_cap` 273-290) | `CollateralError::BorrowCapReached` |
| 16 | Isolated-debt ceiling | borrow.rs:421 (`handle_isolated_debt` 204-242, check at 228) | `EModeError::DebtCeilingReached` |

> **CORRECTION from prior doc draft**: the controller never recomputes HF after the batch. The LTV-pre-borrow check (line 14 above) bounds risk — `validate_asset_config` enforces LTV ≤ liquidation threshold, so any LTV-passing state has HF > 1. **Auditors should confirm this suffices under e-mode threshold overrides and compounding interest in same-tx scenarios.**

### `withdraw(caller, account_id, withdrawals)` — `lib.rs:174` → `positions/withdraw.rs`

| # | Check | File:line | Error |
|---|---|---|---|
| 1 | `caller.require_auth()` | withdraw.rs:17 | host auth |
| 2 | `require_not_paused` | withdraw.rs:18 | PausableError |
| 3 | `require_not_flash_loaning` | withdraw.rs:19 | `FlashLoanError::FlashLoanOngoing` |
| 4 | `account.owner == caller` | withdraw.rs:24-26 | `GenericError::AccountNotInMarket` |
| 5 | **`amount == 0` sentinel → `i128::MAX`** (per asset) | withdraw.rs:84 | triggers pool full-withdraw branch |
| 6 | Position must exist (per asset) | withdraw.rs:78-81 | `CollateralError::CollateralPositionNotFound` |
| 7 | Pool call `pool.withdraw(caller, amount, position, ...)` | withdraw.rs:133-140 | — |
| 8 | Pool dust-lock guard: partial `amount` with residual asset = 0 escalates to full | pool/lib.rs:186-197 | behavior change, no panic |
| 9 | Pool reserve check `has_reserves(net_transfer)` | pool/lib.rs:209-212 | `CollateralError::InsufficientLiquidity` |
| 10 | Post-batch HF: when borrows remain, HF must be `>= WAD` | withdraw.rs:44-52 | `CollateralError::InsufficientCollateral` |

> **DOC DRIFT**: `INVARIANTS.md §A` and `ARCHITECTURE.md §Withdraw` claim "no `amount == 0` sentinel". **The code has one** at withdraw.rs:84 (`amount == 0` → `i128::MAX` → triggers pool's full-withdraw branch). Update the upstream docs.

### `repay(caller, account_id, payments)` — `lib.rs:182` → `positions/repay.rs`

| # | Check | File:line | Error |
|---|---|---|---|
| 1 | `caller.require_auth()` | repay.rs:17 | host auth |
| 2 | `require_not_paused` | repay.rs:18 | PausableError |
| 3 | `require_not_flash_loaning` | repay.rs:19 | `FlashLoanError::FlashLoanOngoing` |
| 4 | **NO account-owner check** (anyone may repay anyone) | repay.rs:20 (comment) | by design |
| 5 | Amount > 0 (per asset) | repay.rs:50 | `GenericError::AmountMustBePositive` |
| 6 | Borrow position must exist (per asset) | repay.rs:53-56 | `CollateralError::DebtPositionNotFound` |
| 7 | Token transfer caller→pool, balance-delta verify > 0 | repay.rs:62-71 | `GenericError::AmountMustBePositive` |
| 8 | Pool call; overpayment refund | pool/lib.rs:251-268 | refunds `caller` (the actual repayer) |
| 9 | Isolated-debt decrement with sub-$1 dust erasure | utils.rs:85-88 (invoked from repay.rs:131-141) | `cache.flush_isolated_debts()` flushes (repay.rs:39) |

### `liquidate(liquidator, account_id, debt_payments)` — `lib.rs:190` → `positions/liquidation.rs`

| # | Check | File:line | Error |
|---|---|---|---|
| 1 | `liquidator.require_auth()` | liquidation.rs:28 | host auth |
| 2 | `require_not_paused` | liquidation.rs:29 | PausableError |
| 3 | `require_not_flash_loaning` | liquidation.rs:30 | `FlashLoanError::FlashLoanOngoing` |
| 4 | HF must start `< WAD` (= 1.0) | liquidation.rs:158-160 | `CollateralError::HealthFactorTooHigh` (101) |
| 5 | Three-tier HF target cascade: 1.02 → 1.01 → fallback `d_max = total_coll / (1+base_bonus)` | helpers/mod.rs:216, 231 (1.02); 261 (1.01); 284 (d_max) | — |
| 6 | Regression guard in fallback rejects when `base_new_hf < WAD::ONE && base_new_hf < hf` | helpers/mod.rs:295 | — |
| 7 | Per-asset seizure split: `base = capped_amount / (1 + bonus)` floored, `bonus = capped_amount - base`, `protocol_fee = bonus * liquidation_fees_bps` | liquidation.rs:361-366 | conservation by construction |
| 8 | Process payments loop | liquidation.rs:57-89 | per-asset `pool.repay` |
| 9 | Process seizures loop | liquidation.rs:92-122 | per-asset `pool.seize_position` |
| 10 | Post-liquidation bad-debt check: `total_debt_usd > total_collateral_usd && total_collateral_usd <= 5 * WAD` | liquidation.rs:127, 429-430 | triggers `apply_bad_debt_to_supply_index` |
| 11 | Supply-index floor `SUPPLY_INDEX_FLOOR_RAW = 10^18 raw` | pool/interest.rs:14, 131-135 | clamped, never below |

`clean_bad_debt(caller, account_id)` (`KEEPER`-gated, `lib.rs:347`) calls `clean_bad_debt_standalone` (liquidation.rs:442-471), which shares the `execute_bad_debt_cleanup` math path (line 463) — no separate code path.

### `flash_loan(caller, asset, amount, receiver, data)` — `lib.rs:203` → `flash_loan.rs:9`

| # | Check | File:line | Error |
|---|---|---|---|
| 1 | `caller.require_auth()` | flash_loan.rs:18 | host auth |
| 2 | `require_not_paused` | flash_loan.rs:21 | PausableError |
| 3 | `require_not_flash_loaning` | flash_loan.rs:22 | `FlashLoanError::FlashLoanOngoing` |
| 4 | Amount > 0 | flash_loan.rs:25 | `GenericError::AmountMustBePositive` |
| 5 | Market active | flash_loan.rs:26 | `GenericError::PairNotActive` |
| 6 | `is_flashloanable` | flash_loan.rs:32-34 | `FlashLoanError::FlashloanNotEnabled` |
| 7 | Fee = `Bps::from_raw(flashloan_fee_bps).apply_to(amount)` (half-up via `mul_div_half_up`) | flash_loan.rs:37, common/fp.rs:189-191 | — |
| 8 | Set guard `set_flash_loan_ongoing(true)` | flash_loan.rs:43 | — |
| 9 | `pool.flash_loan_begin(asset, amount, receiver)` | flash_loan.rs:46-47 | pool transfers `amount` to receiver |
| 10 | Receiver callback `execute_flash_loan(initiator, asset, amount, fee, data)` | flash_loan.rs:51-55 | `env.invoke_contract::<()>(...)` |
| 11 | `pool.flash_loan_end(asset, amount, fee, receiver)` | flash_loan.rs:58 | pool calls `tok.transfer(receiver→pool, amount+fee)` (Soroban-native auth) at pool/lib.rs:353 |
| 12 | Clear guard | flash_loan.rs:61 | — |

> **Soroban auth model for repayment**: pool/lib.rs:353 calls plain `tok.transfer(&receiver, &env.current_contract_address(), &total)`, which triggers `receiver.require_auth()` inside the SAC. The receiver contract MUST call `env.authorize_as_current_contract(...)` (or equivalent host-auth pre-authorization) inside its `execute_flash_loan` callback to satisfy this auth. This is **NOT** an ERC-20 `approve`/`transfer_from` allowance pattern.

> **Panic-rollback**: a panic anywhere in steps 8-12 reverts the entire tx, including the `set_flash_loan_ongoing(true)` write. Future flash loans remain unblocked.

## Strategy Operations — `controller/src/strategy.rs`

| Fn | Auth | Reentry guard at entry | Notable per-fn check |
|---|---|---|---|
| `multiply` | Caller (strategy.rs:53) | strategy.rs:55 | mode ∈ {Multiply, Long, Short} (rejects Normal at strategy.rs:63); Long/Short with third-token initial_payment requires convert_steps (strategy.rs:94-96) |
| `swap_debt` | Caller | strategy.rs:228 | — |
| `swap_collateral` | Caller | strategy.rs:345 | rejects isolated accounts at strategy.rs:359 (`FlashLoanError::SwapCollateralNoIso`) |
| `repay_debt_with_collateral` | Caller | strategy.rs:518 | `close_position == true` requires zero remaining borrows; panics `CannotCloseWithRemainingDebt` at strategy.rs:638-641 |

**Strategy aggregator interaction (`swap_tokens`, strategy.rs:456-499)**:
- Snapshots controller's `balance_in_before` / `balance_out_before` (strategy.rs:456-457).
- Calls aggregator with `steps.amount_out_min` (strategy.rs:472).
- Re-reads controller's `balance_in_after` / `balance_out_after` (strategy.rs:481, 496).
- Verifies spend ≤ `amount_in` (strategy.rs:482-488); records received amount (strategy.rs:497-499).
- **The AGGREGATOR enforces `amount_out_min` (DEX-level); the controller does not re-verify.** This trusts the operator-set aggregator.

> **Strategy flash-loan guard gap (already noted in THREAT_MODEL §1)**: strategy fns never call `set_flash_loan_ongoing(true)` themselves — they go directly through `pool.flash_loan_begin`/`pool.flash_loan_end`. During a strategy's internal pool-flash-loan window, the controller-level boolean stays false. Aggregator callbacks (operator-trusted) are the only sub-calls in that window.

## Keeper / Revenue / Oracle Roles

(Unchanged from previous version — see git history. All gates are `#[only_role(caller, "KEEPER" / "REVENUE" / "ORACLE")]` and verified.)

## Configuration Endpoints

(All `#[only_owner]` per `lib.rs:451-627`. Field-level validation per `audit/CONFIG_INVARIANTS.md`.)

## Pool Public ABI (`pool/src/lib.rs`)

Every mutating pool fn calls `verify_admin(&env)` → `ownable::enforce_owner_auth(env)` (pool/lib.rs:37-39). Construction sets admin to the controller.

| Fn | File:line | Notable |
|---|---|---|
| `__constructor(admin, params)` | pool/lib.rs:77 | Sets owner; inits indexes to RAY; `last_timestamp = ledger.timestamp() * 1000` |
| `supply(position, price_wad, amount)` | pool/lib.rs:100, verify_admin at 106 | `global_sync` then `scaled = calculate_scaled_supply(amount)`; `supplied_ray += scaled` |
| `borrow(caller, amount, position, price_wad)` | pool/lib.rs:128, verify_admin at 135 | `global_sync`; panics unless `has_reserves(amount)`; transfers pool→caller |
| `withdraw(caller, amount, position, ...)` | pool/lib.rs:165, verify_admin at 174 | Full vs partial branch; dust-lock at lines 186-197; reserve check at 209-212 |
| `repay(caller, amount, position, price_wad)` | pool/lib.rs:237, verify_admin at 244 | `global_sync`; refunds excess to `caller` (pool/lib.rs:251-268, line 267) |
| `update_indexes(price_wad)` | pool/lib.rs:285, verify_admin at 286 | `global_sync` |
| `add_rewards(price_wad, amount)` | pool/lib.rs:301, verify_admin at 302 | Bumps supply index |
| `flash_loan_begin(asset, amount, receiver)` | pool/lib.rs:317, verify_admin at 318 | pool→receiver transfer |
| `flash_loan_end(asset, amount, fee, receiver)` | pool/lib.rs:340, verify_admin at 341 | `tok.transfer(receiver→pool, amount+fee)` at line 353 (Soroban-native auth) |
| `create_strategy(...)` | pool/lib.rs:377, verify_admin at 385 | Per-strategy accounting |
| `seize_position(...)` | pool/lib.rs:424, verify_admin at 429 | base+bonus+fee split; bad-debt path may call `apply_bad_debt_to_supply_index` |
| `claim_revenue(caller, price_wad)` | pool/lib.rs:453, verify_admin at 454 | Proportional burn (full or partial); transfer = `min(reserves, treasury_actual)` |
| `update_params(params)` | pool/lib.rs:509, verify_admin at 520 | Controller pre-validates |
| `upgrade(new_wasm_hash)` | pool/lib.rs:590, verify_admin at 595 | Non-zero hash |
| `keepalive()` | pool/lib.rs:594, verify_admin at 595 | Bumps Instance TTL |
